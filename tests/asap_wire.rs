//! Integration test for the ASAP active-sampling wire in
//! `handlers::next_trial`. Seeds the responses table with a deliberate
//! pattern that makes one adjacent pair the least-decided (highest EIG),
//! then drives the handler enough times to count which pair it returns.

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
use squintly::db::now_ms;
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

/// One source with five same-codec encodings at q ∈ {20, 40, 60, 80, 95}.
/// Bytes scale monotonically; same-codec, so no pair is trivial. Only one
/// source so the sampler can't route around the ASAP path.
async fn manifest() -> Json<serde_json::Value> {
    Json(json!({
        "sources": [
            {"hash": "asapsource0000001", "width": 256, "height": 256, "size_bytes": 12345, "corpus": "test", "filename": "a.png"}
        ],
        "encodings": [
            {"id": "q20", "source_hash": "asapsource0000001", "codec_name": "mozjpeg", "quality": 20.0, "encoded_size": 4000},
            {"id": "q40", "source_hash": "asapsource0000001", "codec_name": "mozjpeg", "quality": 40.0, "encoded_size": 8000},
            {"id": "q60", "source_hash": "asapsource0000001", "codec_name": "mozjpeg", "quality": 60.0, "encoded_size": 14000},
            {"id": "q80", "source_hash": "asapsource0000001", "codec_name": "mozjpeg", "quality": 80.0, "encoded_size": 22000},
            {"id": "q95", "source_hash": "asapsource0000001", "codec_name": "mozjpeg", "quality": 95.0, "encoded_size": 40000}
        ]
    }))
}

async fn source_image(Path(_hash): Path<String>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut h = HeaderMap::new();
    h.insert("content-type", "image/png".parse().unwrap());
    (StatusCode::OK, h, b"\x89PNG\r\n\x1a\nfake".to_vec())
}

async fn encoding_image(Path(_id): Path<String>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut h = HeaderMap::new();
    h.insert("content-type", "image/jpeg".parse().unwrap());
    (StatusCode::OK, h, b"\xff\xd8\xff\xe0fake".to_vec())
}

/// Insert a synthetic (trial, response) row representing one prior pair
/// observation between encodings `a` and `b` with outcome encoded as
/// 'a' / 'b' / 'tie'.
async fn insert_synthetic_response(
    pool: &sqlx::SqlitePool,
    session_id: &str,
    a_id: &str,
    b_id: &str,
    choice: &str,
) -> Result<()> {
    let trial_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO trials (id, session_id, kind, source_hash, a_encoding_id, a_codec, \
         a_quality, a_bytes, b_encoding_id, b_codec, b_quality, b_bytes, intrinsic_w, \
         intrinsic_h, is_golden, held_out, served_at) \
         VALUES (?, ?, 'pair', 'asapsource0000001', ?, 'mozjpeg', NULL, NULL, ?, 'mozjpeg', \
         NULL, NULL, 256, 256, 0, 0, ?)",
    )
    .bind(&trial_id)
    .bind(session_id)
    .bind(a_id)
    .bind(b_id)
    .bind(now_ms())
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO responses (trial_id, choice, dwell_ms, reveal_count, reveal_ms_total, \
         zoom_used, viewport_w_css, viewport_h_css, orientation, image_displayed_w_css, \
         image_displayed_h_css, intrinsic_to_device_ratio, pixels_per_degree, responded_at) \
         VALUES (?, ?, 1000, 1, 200, 0, 390, 700, 'portrait', 360.0, 360.0, 1.0, 60.0, ?)",
    )
    .bind(&trial_id)
    .bind(choice)
    .bind(now_ms())
    .execute(pool)
    .await?;
    Ok(())
}

#[tokio::test]
async fn asap_wire_targets_least_decided_pair() -> Result<()> {
    let coeff_addr = fake_coefficient().await?;
    let coeff = HttpCoefficient::new(&format!("http://{coeff_addr}"))?;
    let manifest = squintly::coefficient::Coefficient::refresh_manifest(&coeff).await?;
    assert_eq!(manifest.encodings.len(), 5);

    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let state = Arc::new(AppState {
        pool: pool.clone(),
        coefficient: CoefficientSource::Http(coeff),
        manifest: tokio::sync::RwLock::new(manifest),
        anchors: tokio::sync::RwLock::new(Default::default()),
        source_flags: tokio::sync::RwLock::new(Default::default()),
        suggestions: squintly::suggestion_store::SuggestionStore::LocalDisk(
            squintly::suggestion_store::LocalDiskStore::new(tempfile::tempdir()?.keep()),
        ),
        sampler: Default::default(),
    });

    // We need a real session row to satisfy the trial → session FK and the
    // supported_codecs lookup in `next_trial`. We make one directly via SQL
    // so we don't run the session-creation streak logic.
    sqlx::query(
        "INSERT INTO observers (id, created_at, user_agent) VALUES ('obs-prior', ?, 'seed')",
    )
    .bind(now_ms())
    .execute(&pool)
    .await?;
    let prior_session = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO sessions (id, observer_id, started_at, device_pixel_ratio, \
         screen_width_css, screen_height_css, color_gamut, viewing_distance_cm, ambient_light, \
         supported_codecs) \
         VALUES (?, 'obs-prior', ?, 3.0, 390, 844, 'p3', 30, 'room', 'jpeg,png')",
    )
    .bind(&prior_session)
    .bind(now_ms())
    .execute(&pool)
    .await?;

    // Seed history. The pair we want ASAP to target is the q95-anchored edge
    // (q80, q95): we feed it mixed evidence (β_80 ≈ β_95 = 0), while every
    // other adjacent pair gets 40 decisive observations (higher q wins).
    // The anchor at q95 stops the chain propagation from pulling β_80 down,
    // so the EIG ranking lines up with intuition.
    for _ in 0..40 {
        insert_synthetic_response(&pool, &prior_session, "q20", "q40", "b").await?;
        insert_synthetic_response(&pool, &prior_session, "q40", "q60", "b").await?;
        insert_synthetic_response(&pool, &prior_session, "q60", "q80", "b").await?;
    }
    for _ in 0..10 {
        insert_synthetic_response(&pool, &prior_session, "q80", "q95", "a").await?;
        insert_synthetic_response(&pool, &prior_session, "q80", "q95", "b").await?;
        insert_synthetic_response(&pool, &prior_session, "q80", "q95", "tie").await?;
    }

    // Wire just the next_trial route — we don't need the session-create or
    // export bits for this test.
    let api = Router::new()
        .route("/trial/next", get(handlers::next_trial))
        .route(
            "/session",
            axum::routing::post(handlers::create_session),
        );
    let app = Router::new().nest("/api", api).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // Create a fresh session via the normal handler (so the FK is satisfied
    // for the trials this run generates).
    let s = client
        .post(format!("{base}/api/session"))
        .json(&json!({
            "observer_id": null,
            "user_agent": "asap-wire-test",
            "device_pixel_ratio": 3.0,
            "screen_width_css": 390,
            "screen_height_css": 844,
            "color_gamut": "p3",
            "viewing_distance_cm": 30,
            "ambient_light": "room",
            "css_px_per_mm": 4.7,
            "local_date": "2026-04-30",
            "theme_slug": "nature"
        }))
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    let session_id = s["session_id"].as_str().unwrap().to_string();

    // Drive next_trial many times; count which adjacent pair the handler
    // picks when it returns a Pair trial. ASAP should overwhelmingly favour
    // (q40, q60) since it has the highest EIG.
    let mut pair_counts: std::collections::HashMap<(String, String), u32> =
        std::collections::HashMap::new();
    let mut pair_trials = 0;
    for _ in 0..200 {
        let trial: serde_json::Value = client
            .get(format!("{base}/api/trial/next?session_id={session_id}"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if trial["kind"] != "pair" {
            continue;
        }
        pair_trials += 1;
        let a = trial["a"]["encoding_id"].as_str().unwrap().to_string();
        let b = trial["b"]["encoding_id"].as_str().unwrap().to_string();
        let mut pair = [a, b];
        pair.sort();
        let [a, b] = pair;
        *pair_counts.entry((a, b)).or_insert(0) += 1;
    }

    assert!(pair_trials >= 20, "should see many pair trials");
    let target = ("q80".to_string(), "q95".to_string());
    let target_count = pair_counts.get(&target).copied().unwrap_or(0);
    let other_total: u32 = pair_counts
        .iter()
        .filter(|(k, _)| **k != target)
        .map(|(_, v)| *v)
        .sum();
    assert!(
        target_count > other_total,
        "expected ASAP to dominate with (q80, q95) — the mixed-evidence anchor-edge pair; \
         got target={target_count}, other={other_total}, counts={pair_counts:?}"
    );
    Ok(())
}
