//! Sign-in is open to any address; a rate limit is what protects the mailer,
//! and admin is the thing that is allowlisted.
//!
//! Both properties are easy to get backwards, and both fail quietly if you do:
//! an over-tight limit locks real participants out of their own data, and an
//! admin check that trusts a client-held identifier grants privilege to anyone
//! who can set a cookie. So this exercises the real HTTP surface — a magic-link
//! round trip against a local Postmark stub, then the session it mints against
//! an admin-gated route.

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
        .max_connections(4)
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
        metric_scores: Default::default(),
    });

    let api = Router::new()
        .route("/auth/start", post(handlers::auth_start))
        .route("/auth/verify", get(handlers::auth_verify))
        .route("/auth/whoami", get(handlers::auth_whoami))
        .route("/auth/signout", post(handlers::auth_signout))
        .route(
            "/curator/backfill-dims",
            post(squintly::curator::backfill_dims),
        );
    let app = Router::new().nest("/api", api).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok(addr)
}

/// `xff` lets one test simulate distinct clients; without it every request
/// would share a bucket and only the per-address limit would be under test.
async fn start_login(addr: SocketAddr, email: &str, xff: &str) -> Result<(u16, String)> {
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/api/auth/start"))
        .header("x-forwarded-for", xff)
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

/// Pull the verify token out of what the stub was asked to send.
fn link_token(outbox: &Outbox, idx: usize) -> String {
    let sent = outbox.lock().unwrap();
    let text = sent[idx]["TextBody"].as_str().unwrap();
    text.split("token=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

/// One test, not several: the config is read from the process environment and
/// `cargo test` runs a file's tests on parallel threads, so splitting these
/// would race on `set_var`. The env is set once here and never mutated again.
#[tokio::test]
async fn sign_in_is_open_rate_limited_and_only_grants_admin_to_the_roster() -> Result<()> {
    let (postmark, outbox) = postmark_stub().await?;

    // SAFETY: set before any request is served, and never mutated afterwards,
    // so no thread can observe a torn value mid-read.
    unsafe {
        std::env::set_var("POSTMARK_API_BASE", format!("http://{postmark}"));
        std::env::set_var("POSTMARK_SERVER_TOKEN", "stub-token");
        std::env::set_var("POSTMARK_FROM_EMAIL", "noreply@squintly.example");
        std::env::set_var("SQUINTLY_ADMIN_EMAILS", "boss@imazen.io");
        std::env::set_var("SQUINTLY_IP_HASH_SALT", "test-salt");
        // Exercise the hourly caps without waiting an hour, and keep the
        // cooldown out of the way so the per-hour rule is what's under test.
        std::env::set_var("SQUINTLY_AUTH_COOLDOWN_MS", "0");
        std::env::set_var("SQUINTLY_AUTH_PER_EMAIL_HOURLY", "3");
        std::env::set_var("SQUINTLY_AUTH_PER_IP_HOURLY", "5");
        std::env::set_var("SQUINTLY_INSECURE_COOKIES", "1");
    }

    let addr = squintly_server().await?;

    // --- open to anyone: no allowlist stands between a participant and a link
    let (status, body) = start_login(addr, "a-stranger@example.com", "203.0.113.1").await?;
    assert_eq!(status, 200, "sign-in must be open to any address: {body}");
    assert_eq!(outbox.lock().unwrap().len(), 1);

    // --- per-address hourly cap --------------------------------------------
    for i in 2..=3 {
        let (status, _) = start_login(addr, "a-stranger@example.com", "203.0.113.1").await?;
        assert_eq!(status, 200, "request {i} should still be under the cap");
    }
    let (status, body) = start_login(addr, "a-stranger@example.com", "203.0.113.1").await?;
    assert_eq!(status, 429, "a 4th request for one address must be refused");
    assert!(
        body.contains("Slow down"),
        "the refusal should be legible: {body}"
    );
    assert_eq!(
        outbox.lock().unwrap().len(),
        3,
        "a rate-limited request must not send mail"
    );

    // A different address from the same network is still allowed — the address
    // cap must not act as a network ban.
    let (status, _) = start_login(addr, "someone-else@example.com", "203.0.113.1").await?;
    assert_eq!(status, 200);

    // --- per-network hourly cap --------------------------------------------
    // Cycling addresses is exactly what a per-address limit cannot see. This
    // network has now had 4 of its 5.
    let (status, _) = start_login(addr, "third@example.com", "203.0.113.1").await?;
    assert_eq!(status, 200);
    let (status, body) = start_login(addr, "fourth@example.com", "203.0.113.1").await?;
    assert_eq!(
        status, 429,
        "cycling addresses must not sidestep the limit: {body}"
    );

    // A different network is unaffected.
    let (status, _) = start_login(addr, "elsewhere@example.com", "198.51.100.9").await?;
    assert_eq!(status, 200);

    // --- signing in mints a session; a non-roster address is not admin -----
    let client = reqwest::Client::builder().cookie_store(true).build()?;
    let token = link_token(&outbox, 0);
    let r = client
        .get(format!("http://{addr}/api/auth/verify?token={token}"))
        .send()
        .await?;
    assert!(r.status().is_success());
    assert!(
        r.headers()
            .get_all("set-cookie")
            .iter()
            .any(|v| v.to_str().unwrap_or("").contains("squintly_session=")),
        "verify must mint a session cookie"
    );

    let who: serde_json::Value = client
        .get(format!("http://{addr}/api/auth/whoami"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(who["signed_in"], true);
    assert_eq!(who["email"], "a-stranger@example.com");
    assert_eq!(
        who["is_admin"], false,
        "an ordinary participant must never be admin"
    );

    // ...and that session cannot drive an admin route.
    let r = client
        .post(format!("http://{addr}/api/curator/backfill-dims"))
        .json(&json!({"admin_token": null}))
        .send()
        .await?;
    assert_eq!(
        r.status().as_u16(),
        403,
        "a signed-in non-admin must be refused"
    );

    // --- an address on the roster does get admin ---------------------------
    let admin = reqwest::Client::builder().cookie_store(true).build()?;
    let (status, _) = start_login(addr, "boss@imazen.io", "198.51.100.20").await?;
    assert_eq!(status, 200);
    let idx = outbox.lock().unwrap().len() - 1;
    let token = link_token(&outbox, idx);
    admin
        .get(format!("http://{addr}/api/auth/verify?token={token}"))
        .send()
        .await?;
    let who: serde_json::Value = admin
        .get(format!("http://{addr}/api/auth/whoami"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(who["is_admin"], true, "the roster address must be admin");

    // Signing out revokes it — a stale cookie must stop working.
    admin
        .post(format!("http://{addr}/api/auth/signout"))
        .send()
        .await?;
    let who: serde_json::Value = admin
        .get(format!("http://{addr}/api/auth/whoami"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(who["signed_in"], false, "sign-out must revoke the session");
    assert_eq!(who["is_admin"], false);

    Ok(())
}
