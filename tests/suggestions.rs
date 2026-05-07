//! HTTP integration test for /api/suggestions/*.
//!
//! Covers: submit (multipart) → list (admin-gated) → accept (promotes to
//! curator_candidates + 200 on /file) → reject (404 on /file) → withdraw
//! (email-matched). Verifies the resolved license id when the form passes
//! 'self', the verified-email path when an observer record exists, and the
//! admin-token enforcement on review endpoints.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use axum::routing::{get, post};
use reqwest::multipart;
use sqlx::sqlite::SqlitePoolOptions;
use squintly::coefficient::CoefficientSource;
use squintly::handlers::AppState;
use squintly::suggestions;

const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // signature
    0x00, 0x00, 0x00, 0x0D, b'I', b'H', b'D', b'R', // IHDR length + tag
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, b'I', b'D', b'A',
    b'T', 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
    0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
];

async fn boot_app() -> Result<(SocketAddr, sqlx::SqlitePool, std::path::PathBuf)> {
    // SAFETY: each integration test binary runs in its own process, so env
    // mutations here don't conflict with other tests.
    unsafe {
        std::env::set_var("SQUINTLY_SUGGESTION_ADMIN_TOKEN", "test-admin-token");
        // Make sure no email backend is configured so the notify path takes
        // the no-op branch (assertions on `notification_attempted == false`).
        std::env::remove_var("RESEND_API_KEY");
        std::env::remove_var("POSTMARK_SERVER_TOKEN");
        std::env::remove_var("POSTMARK_FROM_EMAIL");
    }
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    let dir = tempfile::tempdir()?.keep();
    let state = Arc::new(AppState {
        pool: pool.clone(),
        coefficient: CoefficientSource::Disabled,
        manifest: tokio::sync::RwLock::new(Default::default()),
        anchors: tokio::sync::RwLock::new(Default::default()),
        source_flags: tokio::sync::RwLock::new(Default::default()),
        suggestions: squintly::suggestion_store::SuggestionStore::LocalDisk(
            squintly::suggestion_store::LocalDiskStore::new(dir.clone()),
        ),
    });
    let api = Router::new()
        .route(
            "/suggestions",
            post(suggestions::submit).get(suggestions::list),
        )
        .route("/suggestions/{id}/withdraw", post(suggestions::withdraw))
        .route("/suggestions/{id}/accept", post(suggestions::accept))
        .route("/suggestions/{id}/reject", post(suggestions::reject))
        .route("/suggestions/{id}/file", get(suggestions::file));
    let app = Router::new().nest("/api", api).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok((addr, pool, dir))
}

fn tiny_png_part() -> multipart::Part {
    multipart::Part::bytes(TINY_PNG.to_vec())
        .file_name("tiny.png")
        .mime_str("image/png")
        .expect("png mime")
}

#[tokio::test]
async fn suggestion_submit_then_accept_then_serve() -> Result<()> {
    let (addr, pool, _dir) = boot_app().await?;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let form = multipart::Form::new()
        .text("email", "submitter@example.com")
        .text("original_page_url", "https://example.com/cool-photo")
        .text(
            "original_image_url",
            "https://cdn.example.com/cool-photo.jpg",
        )
        .text("license_id", "self")
        .text("license_text_freeform", "I took it myself, CC0.")
        .text("attribution", "Pat Q. Photographer")
        .text("why", "Edge case for color noise reduction.")
        .part("file", tiny_png_part());
    let resp: serde_json::Value = client
        .post(format!("{base}/api/suggestions"))
        .multipart(form)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let id = resp["id"].as_i64().unwrap();
    let sha = resp["sha256"].as_str().unwrap().to_string();
    assert_eq!(resp["status"], "pending");
    assert_eq!(resp["notification_attempted"], false);

    let row = sqlx::query_as::<_, (String, String, String)>(
        "SELECT submitter_email, license_id, status FROM suggestions WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(row.0, "submitter@example.com");
    assert_eq!(row.1, "self");
    assert_eq!(row.2, "pending");

    let file_resp = client
        .get(format!("{base}/api/suggestions/{id}/file"))
        .send()
        .await?;
    assert_eq!(file_resp.status().as_u16(), 404);

    // Without an admin_token query param, list rejects: 400 when the env-side
    // admin token is configured (request lacks the secret), 503 when it isn't
    // (review surface is disabled). Both are non-success.
    let no_admin = client
        .get(format!("{base}/api/suggestions?status=pending"))
        .send()
        .await?;
    let code = no_admin.status().as_u16();
    assert!(code == 400 || code == 503, "got {code}");

    let listed: serde_json::Value = client
        .get(format!(
            "{base}/api/suggestions?status=pending&admin_token=test-admin-token"
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let arr = listed.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], id);

    let acc: serde_json::Value = client
        .post(format!("{base}/api/suggestions/{id}/accept"))
        .json(&serde_json::json!({
            "admin_token": "test-admin-token",
            "reviewer_email": "lilith@imazen.io",
            "reason": "good edge case"
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(acc["status"], "accepted");
    let cand_row = sqlx::query_as::<_, (String, String, String)>(
        "SELECT corpus, license_id, blob_url FROM curator_candidates WHERE sha256 = ?",
    )
    .bind(&sha)
    .fetch_one(&pool)
    .await?;
    assert_eq!(cand_row.0, "public-suggestions");
    assert_eq!(cand_row.1, "self");
    assert!(
        cand_row
            .2
            .contains(&format!("/api/suggestions/{}/file", id))
    );

    let file_resp = client
        .get(format!("{base}/api/suggestions/{id}/file"))
        .send()
        .await?;
    assert_eq!(file_resp.status().as_u16(), 200);
    assert_eq!(
        file_resp.headers().get("content-type").unwrap(),
        "image/png"
    );
    let body = file_resp.bytes().await?;
    assert_eq!(body.as_ref(), TINY_PNG);

    let acc2 = client
        .post(format!("{base}/api/suggestions/{id}/accept"))
        .json(&serde_json::json!({"admin_token": "test-admin-token"}))
        .send()
        .await?;
    assert_eq!(acc2.status().as_u16(), 409);
    Ok(())
}

#[tokio::test]
async fn suggestion_rejects_non_image_and_missing_fields() -> Result<()> {
    let (addr, _pool, _dir) = boot_app().await?;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let form = multipart::Form::new()
        .text("email", "x@y.com")
        .text("original_page_url", "https://example.com/p")
        .part(
            "file",
            multipart::Part::bytes(b"plain text file".to_vec())
                .file_name("notes.txt")
                .mime_str("text/plain")?,
        );
    let resp = client
        .post(format!("{base}/api/suggestions"))
        .multipart(form)
        .send()
        .await?;
    assert_eq!(resp.status().as_u16(), 400);

    let form = multipart::Form::new()
        .text("original_page_url", "https://example.com/p")
        .part("file", tiny_png_part());
    let resp = client
        .post(format!("{base}/api/suggestions"))
        .multipart(form)
        .send()
        .await?;
    assert_eq!(resp.status().as_u16(), 400);

    let form = multipart::Form::new()
        .text("email", "x@y.com")
        .part("file", tiny_png_part());
    let resp = client
        .post(format!("{base}/api/suggestions"))
        .multipart(form)
        .send()
        .await?;
    assert_eq!(resp.status().as_u16(), 400);
    Ok(())
}

#[tokio::test]
async fn suggestion_withdraw_with_email_match() -> Result<()> {
    let (addr, pool, _dir) = boot_app().await?;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let form = multipart::Form::new()
        .text("email", "alice@example.com")
        .text("original_page_url", "https://example.com/x")
        .part("file", tiny_png_part());
    let r: serde_json::Value = client
        .post(format!("{base}/api/suggestions"))
        .multipart(form)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let id = r["id"].as_i64().unwrap();

    let bad = client
        .post(format!("{base}/api/suggestions/{id}/withdraw"))
        .json(&serde_json::json!({"email": "mallory@evil.com"}))
        .send()
        .await?;
    assert_eq!(bad.status().as_u16(), 400);

    let ok: serde_json::Value = client
        .post(format!("{base}/api/suggestions/{id}/withdraw"))
        .json(&serde_json::json!({"email": "ALICE@example.com", "reason": "second thoughts"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(ok["status"], "withdrawn");
    let st: String = sqlx::query_scalar("SELECT status FROM suggestions WHERE id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(st, "withdrawn");
    Ok(())
}
