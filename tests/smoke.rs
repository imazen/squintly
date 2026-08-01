//! End-to-end smoke test: spin a fake coefficient over HTTP, run squintly against it,
//! exercise session → trial → response → export, and assert the export TSV has the
//! expected schema.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::routing::get;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use squintly::coefficient::{CoefficientSource, HttpCoefficient};
use squintly::handlers::{self, AppState};

async fn fake_coefficient() -> Result<SocketAddr> {
    let app = Router::new()
        .route("/api/manifest", get(manifest))
        .route("/api/sources/{hash}/image", get(source_image))
        .route("/api/encodings/{id}/image", get(encoding_image));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok(addr)
}

async fn manifest() -> Json<serde_json::Value> {
    Json(json!({
        "sources": [
            {"hash": "deadbeef00000001", "width": 256, "height": 256, "size_bytes": 12345, "corpus": "test", "filename": "a.png"},
            {"hash": "deadbeef00000002", "width": 1024, "height": 1024, "size_bytes": 67890, "corpus": "test", "filename": "b.png"}
        ],
        "encodings": [
            {"id": "e1", "source_hash": "deadbeef00000001", "codec_name": "mozjpeg", "quality": 30.0, "encoded_size": 5000},
            {"id": "e2", "source_hash": "deadbeef00000001", "codec_name": "mozjpeg", "quality": 60.0, "encoded_size": 8000},
            {"id": "e3", "source_hash": "deadbeef00000001", "codec_name": "mozjpeg", "quality": 90.0, "encoded_size": 18000},
            {"id": "e4", "source_hash": "deadbeef00000002", "codec_name": "mozjpeg", "quality": 50.0, "encoded_size": 22000},
            {"id": "e5", "source_hash": "deadbeef00000002", "codec_name": "mozjpeg", "quality": 80.0, "encoded_size": 45000}
        ]
    }))
}

async fn source_image(Path(_hash): Path<String>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let png = b"\x89PNG\r\n\x1a\nfake".to_vec();
    let mut h = HeaderMap::new();
    h.insert("content-type", "image/png".parse().unwrap());
    (StatusCode::OK, h, png)
}

async fn encoding_image(Path(_id): Path<String>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let bytes = b"\xff\xd8\xff\xe0fake".to_vec();
    let mut h = HeaderMap::new();
    h.insert("content-type", "image/jpeg".parse().unwrap());
    (StatusCode::OK, h, bytes)
}

#[tokio::test]
async fn smoke_full_loop() -> Result<()> {
    let coeff_addr = fake_coefficient().await?;
    let coeff = HttpCoefficient::new(&format!("http://{coeff_addr}"))?;
    let manifest = squintly::coefficient::Coefficient::refresh_manifest(&coeff).await?;
    assert_eq!(manifest.sources.len(), 2);
    assert_eq!(manifest.encodings.len(), 5);

    // In-memory SQLite for the test
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    // Pre-seed a manifest snapshot row so we can assert the session pins it.
    let snapshot_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO manifest_snapshots (id, loaded_at, r2_public_base, manifest_path, \
         manifest_sha256, body_bytes, n_candidates) \
         VALUES (?, 1, 'https://r2.example', 'manifest.jsonl', \
         'deadbeef0000000000000000000000000000000000000000000000000000beef', 42, 7)",
    )
    .bind(&snapshot_id)
    .execute(&pool)
    .await?;

    let state = Arc::new(AppState {
        pool: pool.clone(),
        coefficient: CoefficientSource::Http(coeff),
        manifest: tokio::sync::RwLock::new(manifest),
        anchors: tokio::sync::RwLock::new(Default::default()),
        source_flags: tokio::sync::RwLock::new(Default::default()),
        suggestions: squintly::suggestion_store::SuggestionStore::LocalDisk(
            squintly::suggestion_store::LocalDiskStore::new(tempfile::tempdir()?.keep()),
        ),
    });

    // Build the same router as main, then exercise it via reqwest in a spawned server.
    let api = Router::new()
        .route("/session", axum::routing::post(handlers::create_session))
        .route(
            "/session/{id}/end",
            axum::routing::post(handlers::end_session),
        )
        .route("/trial/next", get(handlers::next_trial))
        .route(
            "/trial/{id}/response",
            axum::routing::post(handlers::record_response),
        )
        .route("/observer/{id}/profile", get(handlers::observer_profile))
        .route("/export/pareto.tsv", get(handlers::export_pareto))
        .route("/export/thresholds.tsv", get(handlers::export_thresholds))
        .route("/export/responses.tsv", get(handlers::export_responses))
        .route(
            "/export/responses.manifest.json",
            get(handlers::export_responses_manifest),
        )
        .route("/stats", get(handlers::stats));
    let app = Router::new().nest("/api", api).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // 1. create session
    let s = client
        .post(format!("{base}/api/session"))
        .json(&json!({
            "observer_id": null,
            "user_agent": "smoke-test",
            "device_pixel_ratio": 3.0,
            "screen_width_css": 390,
            "screen_height_css": 844,
            "color_gamut": "p3",
            "viewing_distance_cm": 30,
            "ambient_light": "room",
            "css_px_per_mm": 4.7,
            "local_date": "2026-04-30",
            "theme_slug": "nature",
            // Pinned: the smoke test exercises the general loop, including
            // single-stimulus ratings, so it wants the unrestricted study
            // rather than whichever one is currently the deployment default.
            "study_id": "main"
        }))
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    let session_id = s["session_id"].as_str().unwrap().to_string();
    let observer_id = s["observer_id"].as_str().unwrap().to_string();
    assert_eq!(s["streak_days"], 1);
    assert_eq!(s["streak_outcome"], "advanced");

    // The freshly-created session must pin the manifest snapshot we
    // seeded above (the latest snapshot in manifest_snapshots).
    let (pinned,): (Option<String>,) =
        sqlx::query_as("SELECT manifest_snapshot_id FROM sessions WHERE id = ?")
            .bind(&session_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        pinned.as_deref(),
        Some(snapshot_id.as_str()),
        "session should pin the latest manifest snapshot"
    );

    // 2. fetch a few trials and record responses
    let mut single_seen = false;
    let mut pair_seen = false;
    for _ in 0..30 {
        let trial: serde_json::Value = client
            .get(format!("{base}/api/trial/next?session_id={session_id}"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let trial_id = trial["trial_id"].as_str().unwrap().to_string();
        let kind = trial["kind"].as_str().unwrap();
        let choice = if kind == "single" {
            single_seen = true;
            "2"
        } else {
            pair_seen = true;
            "a"
        };
        let ack: serde_json::Value = client
            .post(format!("{base}/api/trial/{trial_id}/response"))
            .json(&json!({
                "choice": choice,
                "dwell_ms": 1500,
                "reveal_count": 1,
                "reveal_ms_total": 400,
                "zoom_used": false,
                "viewport_w_css": 390,
                "viewport_h_css": 700,
                "orientation": "portrait",
                "image_displayed_w_css": 360.0,
                "image_displayed_h_css": 360.0,
                "intrinsic_to_device_ratio": 1.0,
                "pixels_per_degree": 60.0
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert!(ack.get("total_trials").and_then(|v| v.as_u64()).is_some());
    }
    assert!(single_seen, "should see single-stimulus trials");
    assert!(pair_seen, "should see pair trials");

    // 3. stats
    let stats: serde_json::Value = client
        .get(format!("{base}/api/stats"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(stats["sessions"], 1);
    assert_eq!(stats["responses"], 30);

    // 3b. observer profile shows the streak, total_trials, and at least one milestone badge
    let profile: serde_json::Value = client
        .get(format!("{base}/api/observer/{observer_id}/profile"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(profile["streak_days"], 1);
    assert_eq!(profile["total_trials"], 30);
    let badges = profile["badges"].as_array().expect("badges array");
    assert!(
        badges.iter().any(|b| b["slug"] == "first_10"),
        "should award first_10"
    );
    let themes = profile["themes"].as_array().expect("themes array");
    assert!(themes.iter().any(|t| t["slug"] == "nature"));

    // 4. exports
    let pareto = client
        .get(format!("{base}/api/export/pareto.tsv"))
        .send()
        .await?
        .text()
        .await?;
    assert!(
        pareto.starts_with("image_id\tsize\tconfig_name"),
        "pareto header: {pareto}"
    );

    let thresholds = client
        .get(format!("{base}/api/export/thresholds.tsv"))
        .send()
        .await?
        .text()
        .await?;
    assert!(thresholds.starts_with("image_id\tsize\tcodec\tconditions_bucket\tq_notice"));

    let responses = client
        .get(format!("{base}/api/export/responses.tsv"))
        .send()
        .await?
        .text()
        .await?;
    assert!(responses.starts_with("trial_id\tsession_id\tobserver_id"));
    let response_lines = responses.lines().count();
    assert!(
        response_lines >= 31,
        "expected header + 30 rows, got {response_lines}"
    );

    // The TSV must come with a Link rel=describedby pointing at the manifest;
    // the manifest itself must carry build_commit + sha256 + row_count.
    let resp_full = client
        .get(format!("{base}/api/export/responses.tsv"))
        .send()
        .await?;
    let link = resp_full
        .headers()
        .get("link")
        .map(|v| v.to_str().unwrap_or("").to_string())
        .unwrap_or_default();
    assert!(
        link.contains("responses.manifest.json"),
        "missing describedby Link header: {link}"
    );
    let manifest_json: serde_json::Value = client
        .get(format!("{base}/api/export/responses.manifest.json"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(manifest_json["kind"], "responses");
    assert!(manifest_json["build_commit"].is_string());
    assert_eq!(
        manifest_json["sha256"].as_str().map(|s| s.len()),
        Some(64),
        "sha256 should be 64 hex chars"
    );
    assert!(
        manifest_json["row_count"].as_u64().unwrap_or(0) >= 30,
        "manifest row_count = {}",
        manifest_json["row_count"]
    );

    Ok(())
}
