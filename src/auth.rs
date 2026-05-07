//! Optional email magic-link auth, modeled on Weaver's `convex/auth.ts`.
//!
//! Squintly is anonymous-first. This module adds an opt-in path where an
//! observer attaches an email; clicking the magic link in the email lets them
//! resume the same observer ID on a new device.
//!
//! Threat model is intentionally narrow:
//! - No passwords ever, no SMS OTP (FBI/CISA 2025 deprecated it).
//! - Tokens are 32 bytes from `OsRng`, hex-encoded; only BLAKE3 hash persists.
//! - 15 min TTL, single-use, side-channel-safe via constant-time hash compare.
//! - Send is via Postmark (reqwest POST); without `POSTMARK_SERVER_TOKEN` +
//!   `POSTMARK_FROM_EMAIL`, the start endpoint returns a 503 with a clear
//!   hint — no silent dev-mode that would make production failures look
//!   successful. The Postmark server token + from address are shared with
//!   the suggestion-notify path so operators only set one secret per
//!   environment.
//!
//! Cross-device merge logic on verify:
//! 1. If `email` already belongs to a canonical observer ≠ the requesting
//!    observer, the requesting one becomes an alias of the canonical one.
//!    Trials stay on whatever observer they were recorded on; the redirect
//!    table lets the export reconstruct the canonical relationship.
//! 2. Otherwise, the requesting observer's email is set and it becomes
//!    canonical for that email.

use anyhow::{Context, Result};
use rand::RngCore;

pub const TOKEN_TTL_MS: i64 = 15 * 60 * 1000;
pub const SESSION_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// 32 random bytes → 64-char hex string.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// BLAKE3 hex digest of a token — the only form persisted server-side.
pub fn hash_token(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

/// Loose RFC 5322-ish validation. We do not call out to a verifier; the magic
/// link itself is the verification.
pub fn looks_like_email(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.len() > 320 {
        return false;
    }
    let Some(at) = s.find('@') else { return false };
    let (local, domain) = s.split_at(at);
    let domain = &domain[1..];
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    if !domain.contains('.') {
        return false;
    }
    s.chars().all(|c| !c.is_whitespace())
}

/// Postmark configuration shared by auth + suggestion-notify paths.
///
/// Reads `POSTMARK_SERVER_TOKEN` (required) and `POSTMARK_FROM_EMAIL`
/// (required — Postmark refuses to send from unverified addresses).
/// `POSTMARK_AUTH_MESSAGE_STREAM` overrides the stream for magic links;
/// `POSTMARK_MESSAGE_STREAM` is the shared default (`outbound` if neither
/// is set).
#[derive(Debug)]
pub struct MailerConfig {
    pub server_token: String,
    pub from: String,
    pub message_stream: String,
}

impl MailerConfig {
    pub fn from_env() -> Option<Self> {
        let server_token = std::env::var("POSTMARK_SERVER_TOKEN").ok()?;
        if server_token.is_empty() {
            return None;
        }
        let from = std::env::var("POSTMARK_FROM_EMAIL")
            .ok()
            .filter(|s| !s.is_empty())?;
        let message_stream = std::env::var("POSTMARK_AUTH_MESSAGE_STREAM")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("POSTMARK_MESSAGE_STREAM").ok())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "outbound".to_string());
        Some(Self {
            server_token,
            from,
            message_stream,
        })
    }
}

/// Backwards-compatible alias for callers that still spell it `ResendConfig`.
/// New code should use `MailerConfig`.
pub type ResendConfig = MailerConfig;

pub struct EmailMessage<'a> {
    pub to: &'a str,
    pub link_url: &'a str,
}

pub async fn send_magic_link(cfg: &MailerConfig, msg: EmailMessage<'_>) -> Result<()> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "From": cfg.from,
        "To": msg.to,
        "Subject": "Sign in to Squintly",
        "TextBody": format!(
            "Click to sign in to Squintly: {}\n\nThis link expires in 15 minutes. \
             If you didn't request it, ignore this email.",
            msg.link_url
        ),
        "HtmlBody": format!(
            "<p>Click to sign in to Squintly:</p>\
             <p><a href=\"{url}\">{url}</a></p>\
             <p style=\"color:#888;font-size:0.9em;\">This link expires in 15 minutes. \
             If you didn't request it, ignore this email.</p>",
            url = msg.link_url
        ),
        "MessageStream": cfg.message_stream,
    });
    let resp = client
        .post("https://api.postmarkapp.com/email")
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("X-Postmark-Server-Token", &cfg.server_token)
        .json(&body)
        .send()
        .await
        .context("calling Postmark")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Postmark rejected the send ({status}): {text}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_generates_64_hex_chars() {
        let t = generate_token();
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn token_hash_is_stable() {
        let h1 = hash_token("abcd");
        let h2 = hash_token("abcd");
        assert_eq!(h1, h2);
        assert_ne!(h1, hash_token("abce"));
    }

    #[test]
    fn email_validation_accepts_normal() {
        assert!(looks_like_email("a@b.c"));
        assert!(looks_like_email("river.lilith@gmail.com"));
        assert!(!looks_like_email(""));
        assert!(!looks_like_email("a@b"));
        assert!(!looks_like_email("@b.c"));
        assert!(!looks_like_email("a@"));
        assert!(!looks_like_email("a b@c.d"));
    }
}
