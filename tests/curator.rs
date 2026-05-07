//! Integration test for the curator backend. Runs the full HTTP loop:
//!
//!   POST /api/curator/manifest    → load TSV + JSONL fixtures
//!   GET  /api/curator/licenses    → license registry surfaces unsplash, etc.
//!   GET  /api/curator/stream/next → returns first candidate + suggestion
//!   POST /api/curator/decision    → take with groups + sizes
//!   POST /api/curator/threshold   → save q_imperceptible
//!   GET  /api/curator/progress    → counts match
//!   GET  /api/curator/export.tsv  → header + license_id present + rows joined
//!
//! The fixture is a tiny TSV (matches corpus-builder shape) plus a tiny
//! JSONL (matches `upload_all.py`'s manifest line). No network — purely
//! in-process through a SqliteMemory pool. Verifies the same wire format
//! the e2e Playwright suite consumes.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use axum::routing::{get, post};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use squintly::coefficient::CoefficientSource;
use squintly::curator;
use squintly::handlers::AppState;

const TSV_FIXTURE: &str = "# fixture for curator integration test\n\
                           # generated 2026-04-30\n\
                           corpus\trelative_path\twidth\theight\tsize_bytes\tsuspected_category\n\
                           unsplash-webp\twebp/unsplash/first.webp\t2400\t1758\t819512\tunsplash_photo\n\
                           source_jpegs\tsource_jpegs/second.jpg\t4000\t3000\t1500000\tphoto_natural\n\
                           wikimedia-webshapes\tweb/third.svg\t800\t600\t40000\tlogo\n";

const JSONL_FIXTURE: &str = r#"{"sha256":"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789","format":"webp","source":"unsplash","source_label":"unsplash-webp","width":2400,"height":1800,"file_size":820000}
{"sha256":"fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210","format":"jpeg","source":"github-issues","source_label":"github-issues","width":1024,"height":768,"file_size":250000}
"#;

async fn boot_app() -> Result<(SocketAddr, sqlx::SqlitePool)> {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    let state = Arc::new(AppState {
        pool: pool.clone(),
        coefficient: CoefficientSource::Disabled,
        manifest: tokio::sync::RwLock::new(Default::default()),
        anchors: tokio::sync::RwLock::new(Default::default()),
        source_flags: tokio::sync::RwLock::new(Default::default()),
    });
    let api = Router::new()
        .route("/curator/stream/next", get(curator::stream_next))
        .route("/curator/decision", post(curator::decision))
        .route("/curator/threshold", post(curator::threshold))
        .route("/curator/progress", get(curator::progress))
        .route("/curator/manifest", post(curator::load_manifest))
        .route("/curator/licenses", get(curator::license_registry))
        .route("/curator/export.tsv", get(curator::export_tsv));
    let app = Router::new().nest("/api", api).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok((addr, pool))
}

#[tokio::test]
async fn curator_full_loop_with_tsv_fixture() -> Result<()> {
    let (addr, _pool) = boot_app().await?;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // 1. license registry — must include unsplash and mixed-research.
    let licenses: serde_json::Value = client
        .get(format!("{base}/api/curator/licenses"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let arr = licenses.as_array().expect("licenses array");
    let ids: Vec<&str> = arr.iter().map(|p| p["id"].as_str().unwrap_or("")).collect();
    assert!(ids.contains(&"unsplash"), "license registry has unsplash");
    assert!(ids.contains(&"mixed-research"));
    assert!(ids.contains(&"github-issues"));

    // 2. load TSV manifest.
    let load: serde_json::Value = client
        .post(format!("{base}/api/curator/manifest"))
        .json(&json!({
            "kind": "tsv",
            "body": TSV_FIXTURE,
            "blob_url_base": "https://r2.example/blobs"
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(load["inserted"], 3);
    assert_eq!(load["total"], 3);

    // 3. GET stream/next without a curator_id returns the first candidate.
    let curator_id = "test-curator-uuid-1234";
    let next: serde_json::Value = client
        .get(format!(
            "{base}/api/curator/stream/next?curator_id={curator_id}"
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let cand = &next["candidate"];
    assert_eq!(cand["corpus"], "unsplash-webp");
    assert_eq!(next["license"]["id"], "unsplash");
    assert_eq!(next["license"]["redistribute_bytes"], true);
    assert_eq!(next["total"], 3);
    let sha = cand["sha256"].as_str().unwrap().to_string();
    let suggestion = &next["suggestion"];
    assert!(
        !suggestion["sizes"].as_array().unwrap().is_empty(),
        "expected size chips suggested"
    );

    // 4. POST decision → take with groups.
    let decision: serde_json::Value = client
        .post(format!("{base}/api/curator/decision"))
        .json(&json!({
            "source_sha256": sha,
            "curator_id": curator_id,
            "decision": "take",
            "groups": {
                "core_zensim": true,
                "core_encoding": true
            },
            "sizes": [256, 512, 1024],
            "source_q_detected": 95.0,
            "recommended_max_dim": 2400,
            "decision_dpr": 3.0
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let decision_id = decision["decision_id"].as_i64().unwrap();
    assert_eq!(decision["took"], true);

    // 5. POST threshold for the largest selected size.
    let thr: serde_json::Value = client
        .post(format!("{base}/api/curator/threshold"))
        .json(&json!({
            "decision_id": decision_id,
            "target_max_dim": 1024,
            "q_imperceptible": 76.5,
            "measurement_dpr": 3.0,
            "measurement_distance_cm": 30.0,
            "encoder_label": "browser-canvas-jpeg"
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(thr["ok"], true);

    // 6. progress — 1 take, 0 reject, 1 threshold.
    let prog: serde_json::Value = client
        .get(format!(
            "{base}/api/curator/progress?curator_id={curator_id}"
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(prog["total_candidates"], 3);
    assert_eq!(prog["decisions"], 1);
    assert_eq!(prog["takes"], 1);
    assert_eq!(prog["thresholds"], 1);

    // 7. stream/next again — should now skip the first source and return the
    //    next undecided candidate.
    let next2: serde_json::Value = client
        .get(format!(
            "{base}/api/curator/stream/next?curator_id={curator_id}"
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_ne!(next2["candidate"]["sha256"], cand["sha256"]);
    assert_eq!(next2["candidate"]["corpus"], "source_jpegs");

    // 8. export.tsv contains license columns + threshold row.
    let tsv = client
        .get(format!(
            "{base}/api/curator/export.tsv?curator_id={curator_id}"
        ))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let lines: Vec<&str> = tsv.lines().collect();
    assert!(
        lines[0].contains("license_id"),
        "header has license_id: {}",
        lines[0]
    );
    assert!(lines[0].contains("license_label"));
    assert!(lines[0].contains("license_redistribute"));
    assert!(lines[0].contains("q_imperceptible"));
    assert!(
        lines.len() >= 2,
        "at least one data row, got {}",
        lines.len()
    );
    let row0 = lines[1];
    assert!(
        row0.contains("Unsplash License"),
        "row carries license label: {row0}"
    );
    assert!(
        row0.contains("76.50"),
        "row carries threshold value: {row0}"
    );
    Ok(())
}

#[tokio::test]
async fn curator_jsonl_fixture_resolves_r2_url() -> Result<()> {
    let (addr, _pool) = boot_app().await?;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let resp: serde_json::Value = client
        .post(format!("{base}/api/curator/manifest"))
        .json(&json!({
            "kind": "jsonl",
            "body": JSONL_FIXTURE,
            "blob_url_base": "https://pub-test.r2.dev"
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(resp["inserted"], 2);

    let next: serde_json::Value = client
        .get(format!("{base}/api/curator/stream/next?curator_id=anon"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let blob_url = next["candidate"]["blob_url"].as_str().unwrap();
    assert!(
        blob_url.starts_with("https://pub-test.r2.dev/blobs/"),
        "blob URL is R2-shaped: {blob_url}"
    );
    // First entry sorted by sha256 starts with 'a'…
    let sha = next["candidate"]["sha256"].as_str().unwrap();
    assert!(blob_url.contains(&format!("/{}/{}/", &sha[0..2], &sha[2..4])));
    Ok(())
}

#[tokio::test]
async fn curator_rejects_invalid_decision() -> Result<()> {
    let (addr, _pool) = boot_app().await?;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // Empty manifest — POST decision fails with 400 because the source isn't a candidate.
    let r = client
        .post(format!("{base}/api/curator/decision"))
        .json(&json!({
            "source_sha256": "deadbeef".repeat(8),
            "curator_id": "anon",
            "decision": "take"
        }))
        .send()
        .await?;
    assert_eq!(r.status().as_u16(), 404);

    // Bad decision string.
    client
        .post(format!("{base}/api/curator/manifest"))
        .json(&json!({
            "kind": "tsv",
            "body": TSV_FIXTURE,
            "blob_url_base": "https://r2.example"
        }))
        .send()
        .await?
        .error_for_status()?;
    let r = client
        .post(format!("{base}/api/curator/decision"))
        .json(&json!({
            "source_sha256": "deadbeef".repeat(8),
            "curator_id": "anon",
            "decision": "explode"
        }))
        .send()
        .await?;
    assert_eq!(r.status().as_u16(), 400);
    Ok(())
}
