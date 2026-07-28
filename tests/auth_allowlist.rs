//! `/api/auth/start` must mail a link only to allowlisted addresses.
//!
//! The endpoint is unauthenticated by construction — it exists to reach someone
//! who cannot prove who they are yet — so without an allowlist it lets any
//! caller send mail from the operator's verified From address to a recipient of
//! their choosing. This pins both directions: a listed address gets a link, an
//! unlisted one gets a 403 and no send.
//!
//! Everything runs against a local Postmark stub via `POSTMARK_API_BASE`, so
//! the suite never touches a third party and never sends real mail.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::routing::{get, post};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use squintly::coefficient::{CoefficientSource, HttpCoefficient};
use squintly::handlers::{self, AppState};

type Outbox = Arc<Mutex<Vec<serde_json::Value>>>;

async fn accept_email(
    State(outbox): State<Outbox>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    outbox.lock().unwrap().push(body);
    Json(json!({"MessageID": "stub", "ErrorCode": 0}))
}

/// A stand-in for `api.postmarkapp.com` that records what it was asked to send.
async fn postmark_stub() -> Result<(SocketAddr, Outbox)> {
    let outbox: Outbox = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/email", post(accept_email))
        .with_state(outbox.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok((addr, outbox))
}

async fn squintly_server() -> Result<SocketAddr> {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let state = Arc::new(AppState {
        pool,
        // Never consulted: the auth routes don't read the image store.
        coefficient: CoefficientSource::Http(HttpCoefficient::new("http://127.0.0.1:1")?),
        manifest: tokio::sync::RwLock::new(Default::default()),
        anchors: tokio::sync::RwLock::new(Default::default()),
        source_flags: tokio::sync::RwLock::new(Default::default()),
        suggestions: squintly::suggestion_store::SuggestionStore::LocalDisk(
            squintly::suggestion_store::LocalDiskStore::new(tempfile::tempdir()?.keep()),
        ),
    });

    let api = Router::new()
        .route("/auth/start", post(handlers::auth_start))
        .route("/auth/verify", get(handlers::auth_verify));
    let app = Router::new().nest("/api", api).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok(addr)
}

async fn start_login(addr: SocketAddr, email: &str) -> Result<(u16, String)> {
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/api/auth/start"))
        .json(&json!({
            "email": email,
            "observer_id": null,
            "origin": "https://squintly.example",
        }))
        .send()
        .await?;
    let status = r.status().as_u16();
    Ok((status, r.text().await?))
}

/// One test, not several: the allowlist is read from the process environment,
/// and `cargo test` runs a file's tests on parallel threads, so splitting these
/// would race on `set_var`. The env is set once here and never mutated again.
#[tokio::test]
async fn only_allowlisted_addresses_receive_a_magic_link() -> Result<()> {
    let (postmark, outbox) = postmark_stub().await?;

    // SAFETY: set before any request is served, and never mutated afterwards,
    // so no thread can observe a torn value mid-read.
    unsafe {
        std::env::set_var("POSTMARK_API_BASE", format!("http://{postmark}"));
        std::env::set_var("POSTMARK_SERVER_TOKEN", "stub-token");
        std::env::set_var("POSTMARK_FROM_EMAIL", "noreply@squintly.example");
        std::env::set_var(
            "SQUINTLY_LOGIN_ALLOWLIST",
            "lilith@imazen.io, @allowed.test",
        );
    }

    let addr = squintly_server().await?;

    // --- refused: not on the list -------------------------------------------
    let (status, body) = start_login(addr, "stranger@example.com").await?;
    assert_eq!(status, 403, "unlisted address must be refused: {body}");
    assert!(
        body.contains("allowlist"),
        "the refusal should say why, got: {body}"
    );
    assert!(
        outbox.lock().unwrap().is_empty(),
        "a refused request must not send mail"
    );

    // A near-miss on an allowlisted domain is still a different domain.
    let (status, _) = start_login(addr, "stranger@allowed.test.evil.example").await?;
    assert_eq!(status, 403, "domain rules must not match by suffix");
    assert!(outbox.lock().unwrap().is_empty());

    // --- admitted: exact address --------------------------------------------
    let (status, body) = start_login(addr, "lilith@imazen.io").await?;
    assert_eq!(status, 200, "listed address must be admitted: {body}");
    {
        let sent = outbox.lock().unwrap();
        assert_eq!(sent.len(), 1, "exactly one mail");
        assert_eq!(sent[0]["To"], "lilith@imazen.io");
        let text = sent[0]["TextBody"].as_str().unwrap_or_default();
        assert!(
            text.contains("https://squintly.example/api/auth/verify?token="),
            "the mail should carry a verify link, got: {text}"
        );
    }

    // --- admitted: domain rule, and case is normalised before matching ------
    let (status, body) = start_login(addr, "  ANYONE@Allowed.Test  ").await?;
    assert_eq!(status, 200, "domain rule must admit, case-folded: {body}");
    assert_eq!(outbox.lock().unwrap()[1]["To"], "anyone@allowed.test");

    Ok(())
}
