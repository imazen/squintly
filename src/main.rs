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

    let state = Arc::new(AppState {
        pool,
        coefficient: coeff,
        manifest: tokio::sync::RwLock::new(manifest),
        anchors: tokio::sync::RwLock::new(anchors),
        source_flags: tokio::sync::RwLock::new(source_flags),
    });

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
        .route("/stats", get(handlers::stats))
        .route("/manifest/refresh", post(handlers::refresh_manifest))
        // Curator mode (corpus development).
        .route("/curator/stream/next", get(curator::stream_next))
        .route("/curator/decision", post(curator::decision))
        .route("/curator/threshold", post(curator::threshold))
        .route("/curator/progress", get(curator::progress))
        .route("/curator/manifest", post(curator::load_manifest))
        .route("/curator/licenses", get(curator::license_registry))
        .route("/curator/export.tsv", get(curator::export_tsv));

    let app = Router::new()
        .nest("/api", api)
        .fallback(handlers::serve_static::<WebAssets>)
        .with_state(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());

    let bind = resolve_bind(cli.bind);
    tracing::info!(addr = %bind, "squintly listening");
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
