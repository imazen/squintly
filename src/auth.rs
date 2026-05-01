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
//! - Send is via Resend (reqwest POST); without `RESEND_API_KEY`, the start
//!   endpoint returns a 503 with a clear hint — no silent dev-mode that would
//!   make production failures look successful.
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

#[derive(Debug)]
pub struct ResendConfig {
    pub api_key: String,
    pub from: String,
}

impl ResendConfig {
    /// Reads `RESEND_API_KEY` and `RESEND_FROM_EMAIL`. Returns `None` if the
    /// API key isn't set; callers turn that into a 503 with a hint, never a
    /// silent success.
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("RESEND_API_KEY").ok()?;
        if api_key.is_empty() {
            return None;
        }
        let from = std::env::var("RESEND_FROM_EMAIL")
            .unwrap_or_else(|_| "Squintly <onboarding@resend.dev>".to_string());
        Some(Self { api_key, from })
    }
}

pub struct EmailMessage<'a> {
    pub to: &'a str,
    pub link_url: &'a str,
}

pub async fn send_magic_link(cfg: &ResendConfig, msg: EmailMessage<'_>) -> Result<()> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "from": cfg.from,
        "to": [msg.to],
        "subject": "Sign in to Squintly",
        "text": format!(
            "Click to sign in to Squintly: {}\n\nThis link expires in 15 minutes. \
             If you didn't request it, ignore this email.",
            msg.link_url
        ),
        "html": format!(
            "<p>Click to sign in to Squintly:</p>\
             <p><a href=\"{url}\">{url}</a></p>\
             <p style=\"color:#888;font-size:0.9em;\">This link expires in 15 minutes. \
             If you didn't request it, ignore this email.</p>",
            url = msg.link_url
        ),
    });
    let resp = client
        .post("https://api.resend.com/emails")
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .await
        .context("calling Resend")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Resend rejected the send ({status}): {text}");
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
