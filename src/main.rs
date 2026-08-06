use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use axum::routing::{get, post};
use clap::Parser;
use rust_embed::RustEmbed;
use sqlx::sqlite::SqlitePoolOptions;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use squintly::coefficient::{CoefficientSource, FsCoefficient, HttpCoefficient};
use squintly::curator;
use squintly::handlers::{self, AppState};
use squintly::metrics_api;
use squintly::suggestions;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/web/dist/"]
struct WebAssets;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// HTTP base URL of a running coefficient viewer (e.g. http://localhost:8081)
    #[arg(long, env = "SQUINTLY_COEFFICIENT_HTTP")]
    coefficient_http: Option<String>,

    /// Filesystem path to a coefficient SplitStore (`meta/` + `blobs/`)
    #[arg(long, env = "SQUINTLY_COEFFICIENT_PATH")]
    coefficient_path: Option<PathBuf>,

    /// SQLite database path
    #[arg(long, env = "SQUINTLY_DB", default_value = "squintly.db")]
    db: PathBuf,

    /// Filesystem root for public corpus suggestions. Defaults to
    /// `<db_parent>/suggestions`. Must be writable; created on startup.
    #[arg(long, env = "SQUINTLY_SUGGESTIONS_DIR")]
    suggestions_dir: Option<PathBuf>,

    /// Bind address (CLAUDE.md bans port 8080; default is 3030).
    /// On Railway, the runtime sets PORT — we honour it automatically below.
    #[arg(long, env = "SQUINTLY_BIND", default_value = "127.0.0.1:3030")]
    bind: SocketAddr,
}

/// Resolve the bind address: if the `PORT` env var is set (Railway, Fly, Heroku,
/// other PaaS conventions), bind to 0.0.0.0 on that port. Otherwise honour `--bind`.
fn resolve_bind(cli_bind: SocketAddr) -> SocketAddr {
    if let Ok(p) = std::env::var("PORT") {
        if let Ok(port) = p.parse::<u16>() {
            return SocketAddr::from(([0, 0, 0, 0], port));
        }
    }
    cli_bind
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,squintly=debug")),
        )
        .init();

    let cli = Cli::parse();

    let coeff: CoefficientSource = match (
        cli.coefficient_http.as_deref(),
        cli.coefficient_path.as_deref(),
    ) {
        (Some(url), _) => CoefficientSource::Http(HttpCoefficient::new(url)?),
        (None, Some(path)) => CoefficientSource::Fs(FsCoefficient::new(path.to_path_buf())),
        (None, None) => {
            tracing::warn!(
                "no coefficient source configured; running with an empty manifest. \
                 Set SQUINTLY_COEFFICIENT_HTTP or SQUINTLY_COEFFICIENT_PATH and \
                 POST /api/manifest/refresh to wire one in."
            );
            CoefficientSource::Disabled
        }
    };

    let db_url = format!("sqlite://{}?mode=rwc", cli.db.display());
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect(&db_url)
        .await
        .with_context(|| format!("opening sqlite db at {}", cli.db.display()))?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    // Don't bail on startup if coefficient is unreachable. Railway's healthcheck
    // would otherwise mark every deploy unhealthy until coefficient is up. We
    // start with an empty manifest; `POST /api/manifest/refresh` retries it,
    // and `GET /api/trial/next` returns a clean 409 until real data is loaded.
    let manifest = match coeff.refresh_manifest().await {
        Ok(m) => {
            tracing::info!(
                sources = m.sources.len(),
                encodings = m.encodings.len(),
                "loaded coefficient manifest"
            );
            m
        }
        Err(e) => {
            tracing::warn!(error = %e, "coefficient manifest fetch failed; starting with empty manifest");
            squintly::coefficient::Manifest::default()
        }
    };

    // Load anchors + source-flags from the v0.2 schema. Empty until
    // operators populate them; the sampler degrades to plain manifest mode.
    let anchors = squintly::handlers::load_anchor_pool(&pool)
        .await
        .unwrap_or_default();
    let source_flags = squintly::handlers::load_source_flags(&pool)
        .await
        .unwrap_or_default();
    tracing::info!(
        anchors = anchors.anchors.len(),
        honeypots = anchors.honeypots.len(),
        held_out = source_flags.held_out.len(),
        "loaded anchor pool + source flags"
    );

    let suggestions_local_default = cli.suggestions_dir.clone().unwrap_or_else(|| {
        cli.db
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("suggestions")
    });
    if let Err(e) = std::fs::create_dir_all(&suggestions_local_default) {
        tracing::warn!(
            path = %suggestions_local_default.display(),
            error = %e,
            "could not create local suggestions dir (R2 will still work if configured)"
        );
    }
    let suggestions =
        squintly::suggestion_store::SuggestionStore::from_env(suggestions_local_default);

    // Which study new sessions join unless the client names one. Logged because
    // a forced-choice study looks identical from the outside until you read the
    // responses table.
    let default_study = squintly::studies::default_study();
    tracing::info!(
        default_study = default_study.id,
        pairwise_only = default_study.sampler.pairwise_only,
        available = ?squintly::studies::STUDIES.iter().map(|s| s.id).collect::<Vec<_>>(),
        "studies"
    );

    let state = Arc::new(AppState {
        pool,
        coefficient: coeff,
        manifest: tokio::sync::RwLock::new(manifest),
        anchors: tokio::sync::RwLock::new(anchors),
        source_flags: tokio::sync::RwLock::new(source_flags),
        suggestions,
    });

    // Spawn the nightly observer_grades batch. Fires once on startup so a
    // fresh deploy has a populated table, then every 24h. We don't block
    // startup on the first run — failures bubble through the log layer
    // and the request path keeps working off whatever's in observer_grades
    // from the previous run.
    {
        let pool = state.pool.clone();
        tokio::spawn(async move {
            loop {
                match squintly::grading::rebuild_observer_grades(&pool).await {
                    Ok(n) => tracing::info!(observers = n, "rebuilt observer_grades"),
                    Err(e) => tracing::warn!(error = %e, "observer_grades rebuild failed"),
                }
                // Participant exclusion runs beside the soft grade, not instead
                // of it: the screens compare each observer against their peers,
                // so the verdict can change as other people rate the same
                // stimuli even when this observer has done nothing new.
                match squintly::exclusion::rebuild_dispositions(&pool, |study| {
                    squintly::exclusion::ExclusionPolicy::for_study(
                        squintly::studies::by_id(study)
                            .map(|s| s.exclusion_default)
                            .unwrap_or(false),
                    )
                })
                .await
                {
                    Ok(n) => tracing::info!(observers = n, "rebuilt observer_dispositions"),
                    Err(e) => {
                        tracing::warn!(error = %e, "observer_dispositions rebuild failed")
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
            }
        });
    }

    // Hourly db_health snapshot. Tracks per-table row counts + DB file size
    // so we notice drift between snapshots. Failures are non-fatal — a
    // missed hour just leaves the prior hour's snapshot as the latest.
    {
        let pool = state.pool.clone();
        tokio::spawn(async move {
            loop {
                match squintly::db_health::refresh(&pool).await {
                    Ok(n) => tracing::debug!(tables = n, "db_health refreshed"),
                    Err(e) => tracing::warn!(error = %e, "db_health refresh failed"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
        });
    }

    // Tower-mirror background task. Auto-detects /mnt/tower at startup;
    // when present (the user's local dev mount), runs nightly
    // `VACUUM INTO` snapshots into `/mnt/tower/output/squintly-archive/`.
    // When absent (Railway, CI, vast.ai, anywhere not on Lilith's LAN),
    // silently no-ops with a single info log — Tower-mirror is a
    // local-dev convenience, not a production safety requirement.
    {
        let db_path = cli.db.clone();
        let tower_root = std::path::Path::new("/mnt/tower/output/squintly-archive");
        // Kill-switch for throwaway instances (e2e harness, ad-hoc dev
        // servers): their DBs are wiped per run and must not accumulate
        // snapshots on the NAS.
        let mirror_disabled = std::env::var("SQUINTLY_DISABLE_TOWER_MIRROR")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false);
        if mirror_disabled {
            tracing::info!("Tower mirror disabled via SQUINTLY_DISABLE_TOWER_MIRROR");
        } else if tower_root.parent().map(|p| p.exists()).unwrap_or(false) {
            tracing::info!(
                path = %tower_root.display(),
                "Tower mount detected; scheduling nightly VACUUM INTO snapshots"
            );
            // Ensure the archive dir exists; non-fatal if it can't be made.
            if let Err(e) = std::fs::create_dir_all(tower_root) {
                tracing::warn!(error = %e, "could not create Tower archive dir; mirror disabled");
            } else {
                let pool = state.pool.clone();
                let tower_root = tower_root.to_path_buf();
                tokio::spawn(async move {
                    loop {
                        let stamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%SZ");
                        let dest = tower_root.join(format!("squintly-{stamp}.db"));
                        let dest_str = dest.to_string_lossy().to_string();
                        // SQLite VACUUM INTO produces an atomic consistent
                        // snapshot from a running database; safer than copying
                        // the live .db file.
                        let sql = format!("VACUUM INTO '{}'", dest_str.replace('\'', "''"));
                        match sqlx::query(&sql).execute(&pool).await {
                            Ok(_) => tracing::info!(path = %dest_str, "Tower snapshot written"),
                            Err(e) => tracing::warn!(error = %e, "Tower VACUUM INTO failed"),
                        }
                        // Once-per-day cadence; first run fires immediately
                        // (so dev sees evidence the mirror is working).
                        tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
                    }
                });
            }
        } else {
            tracing::info!(
                "Tower mount /mnt/tower not present; nightly Tower snapshots disabled \
                 (this is expected on Railway / CI / remote deploys). \
                 source DB: {}",
                db_path.display()
            );
        }
    }

    let api = Router::new()
        .route("/session", post(handlers::create_session))
        .route("/session/{id}/end", post(handlers::end_session))
        .route("/trial/next", get(handlers::next_trial))
        .route("/trial/{id}/response", post(handlers::record_response))
        .route("/proxy/source/{hash}", get(handlers::proxy_source))
        .route("/proxy/encoding/{id}", get(handlers::proxy_encoding))
        .route("/observer/{id}/profile", get(handlers::observer_profile))
        .route("/auth/start", post(handlers::auth_start))
        .route("/auth/verify", get(handlers::auth_verify))
        .route("/auth/whoami", get(handlers::auth_whoami))
        .route("/auth/signout", post(handlers::auth_signout))
        .route("/calibration", get(handlers::calibration_list))
        .route(
            "/calibration/response",
            post(handlers::calibration_response),
        )
        .route(
            "/calibration/finalize",
            post(handlers::calibration_finalize),
        )
        .route("/export/pareto.tsv", get(handlers::export_pareto))
        .route("/export/thresholds.tsv", get(handlers::export_thresholds))
        .route("/export/responses.tsv", get(handlers::export_responses))
        .route("/export/unified.tsv", get(handlers::export_unified))
        .route(
            "/export/pareto.manifest.json",
            get(handlers::export_pareto_manifest),
        )
        .route(
            "/export/thresholds.manifest.json",
            get(handlers::export_thresholds_manifest),
        )
        .route(
            "/export/responses.manifest.json",
            get(handlers::export_responses_manifest),
        )
        .route(
            "/export/unified.manifest.json",
            get(handlers::export_unified_manifest),
        )
        .route("/stats", get(handlers::stats))
        .route("/studies", get(handlers::list_studies))
        .route("/studies/progress", get(handlers::study_progress))
        .route("/leaderboard", get(handlers::leaderboard))
        .route("/manifest/refresh", post(handlers::refresh_manifest))
        // Curator mode (corpus development).
        .route("/curator/stream/next", get(curator::stream_next))
        .route("/curator/decision", post(curator::decision))
        .route("/curator/decision/undo", post(curator::undo_decision))
        .route("/curator/generate-variant", post(curator::generate_variant))
        .route("/curator/threshold", post(curator::threshold))
        .route("/curator/progress", get(curator::progress))
        .route("/curator/manifest", post(curator::load_manifest))
        .route("/curator/load-r2-public", post(curator::load_r2_public))
        // Objective metric scores. All three are admin-only: the values say how
        // well the metric under test agrees with the observers, and an observer
        // who reads that has been told something about the answer to the
        // question they are being asked.
        .route(
            "/admin/metrics",
            post(metrics_api::ingest).get(metrics_api::catalog),
        )
        .route("/admin/disposition", get(metrics_api::disposition))
        .route("/curator/backfill-dims", post(curator::backfill_dims))
        .route("/curator/blob/{sha256}", get(curator::blob_proxy))
        .route(
            "/curator/candidates/delete",
            post(curator::delete_candidate),
        )
        .route("/curator/licenses", get(curator::license_registry))
        .route("/curator/export.tsv", get(curator::export_tsv))
        // Public corpus suggestions / uploads.
        .route(
            "/suggestions",
            post(suggestions::submit).get(suggestions::list),
        )
        .route("/suggestions/{id}/withdraw", post(suggestions::withdraw))
        .route("/suggestions/{id}/accept", post(suggestions::accept))
        .route("/suggestions/{id}/reject", post(suggestions::reject))
        .route("/suggestions/{id}/file", get(suggestions::file));

    let app = Router::new()
        .nest("/api", api)
        .fallback(handlers::serve_static::<WebAssets>)
        .with_state(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());

    let bind = resolve_bind(cli.bind);
    // Provenance is only useful if it's real. A build that lost its build
    // script still runs, but every export it writes is unattributable — say so
    // loudly at boot rather than letting "unknown" reach a TSV unnoticed.
    if handlers::BUILD_COMMIT == "unknown" {
        tracing::warn!(
            "build_commit is \"unknown\" — build.rs did not run (check the Dockerfile \
             COPYs build.rs, or pass SQUINTLY_BUILD_COMMIT). Exports from this build \
             cannot be traced to a source revision."
        );
    } else {
        tracing::info!(build_commit = handlers::BUILD_COMMIT, "build provenance");
    }
    // Sign-in itself is open to any address; what is gated is admin. An empty
    // roster is a legitimate configuration (a deployment with no operators) but
    // is silently indistinguishable from "I set the variable and fat-fingered
    // it", which is the mistake worth catching at boot rather than in a support
    // thread. Log what the process actually parsed either way.
    let admins = squintly::auth::EmailAllowlist::admins();
    if admins.is_empty() {
        tracing::warn!(
            "{} is empty — nobody can hold admin on this deployment. Sign-in and \
             anonymous use are unaffected.",
            squintly::auth::ADMIN_EMAILS_ENV
        );
    } else {
        tracing::info!(admins = %admins.describe(), "admin roster");
    }
    // Whether an `excluded` disposition is acted on. Logged per study because
    // it changes every aggregate downstream, and because the env override is
    // silent by design once it has been applied.
    for st in squintly::studies::STUDIES {
        let p = squintly::exclusion::ExclusionPolicy::for_study(st.exclusion_default);
        tracing::info!(
            study = st.id,
            enforced = p.enabled,
            study_default = st.exclusion_default,
            "participant exclusion policy"
        );
    }
    let rl = squintly::auth::RateLimit::from_env();
    tracing::info!(
        per_email_cooldown_ms = rl.per_email_cooldown_ms,
        per_email_hourly = rl.per_email_hourly,
        per_ip_hourly = rl.per_ip_hourly,
        "sign-in rate limits"
    );
    tracing::info!(addr = %bind, "squintly listening");
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
