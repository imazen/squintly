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
//! - Who may request a link is restricted by `SQUINTLY_LOGIN_ALLOWLIST`
//!   (see [`LoginAllowlist`]); an unset variable denies everyone, because an
//!   open start endpoint lets any caller send mail from the operator's domain.
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

use std::collections::BTreeSet;

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

/// At least two non-empty labels of `[a-z0-9-]`, so that `.`, `localhost` and
/// `imazen.io.` are all rejected rather than becoming a rule that matches more
/// than it looks like it does.
fn is_plausible_domain(d: &str) -> bool {
    let labels: Vec<&str> = d.split('.').collect();
    labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// Who is allowed to request a magic link.
///
/// `/api/auth/start` is unauthenticated by construction — it has to be, since
/// the whole point is to reach someone who can't prove who they are yet — and
/// it mails a link to whatever address the caller names. On a public
/// deployment that makes it an email-amplification vector: anyone can make
/// this deployment's Postmark account send mail, from the operator's verified
/// From address, to a recipient they chose. What that costs is sender
/// reputation, which is far more expensive to get back than the quota.
///
/// So the allowlist is mandatory, and an **unset variable denies everyone**
/// rather than admitting everyone. The asymmetry is the point: closed-by-
/// default fails as a 403 that names the variable to set, while open-by-default
/// fails as a public endpoint quietly sending mail on demand for as long as
/// nobody looks. This module already takes that position on the mailer itself
/// (missing Postmark config is a loud 503, not a dev-mode that no-ops), and
/// nothing here is load-bearing for anonymous use — sign-in exists only to
/// carry an existing observer ID to a second device.
///
/// Entries are separated by commas or whitespace and are case-insensitive:
///   * `someone@example.com` — that one address;
///   * `@example.com` — any address at that domain.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LoginAllowlist {
    addresses: BTreeSet<String>,
    domains: BTreeSet<String>,
}

impl LoginAllowlist {
    pub const ENV: &'static str = "SQUINTLY_LOGIN_ALLOWLIST";

    pub fn from_env() -> Self {
        Self::parse(&std::env::var(Self::ENV).unwrap_or_default())
    }

    pub fn parse(raw: &str) -> Self {
        let mut addresses = BTreeSet::new();
        let mut domains = BTreeSet::new();
        for entry in raw.split([',', ';', ' ', '\t', '\n', '\r']) {
            let entry = entry.trim().to_ascii_lowercase();
            if entry.is_empty() {
                continue;
            }
            match entry.strip_prefix('@') {
                Some(domain) if is_plausible_domain(domain) => {
                    domains.insert(domain.to_string());
                }
                // A bare `@`, or `@localhost`, or `@.`, would admit far more
                // than the operator meant. Drop it loudly instead of guessing —
                // a silently-widened allowlist is the failure this type exists
                // to prevent.
                Some(_) => tracing::warn!(
                    entry = %entry,
                    "ignoring {} entry: a domain rule must look like '@example.com'",
                    Self::ENV
                ),
                None if looks_like_email(&entry) => {
                    addresses.insert(entry);
                }
                None => tracing::warn!(
                    entry = %entry,
                    "ignoring {} entry: not an email address or an '@domain' rule",
                    Self::ENV
                ),
            }
        }
        Self { addresses, domains }
    }

    pub fn is_empty(&self) -> bool {
        self.addresses.is_empty() && self.domains.is_empty()
    }

    /// `email` must already be trimmed and lowercased, as `auth_start` does
    /// before it validates the shape.
    pub fn allows(&self, email: &str) -> bool {
        debug_assert_eq!(email, email.trim().to_ascii_lowercase());
        if self.addresses.contains(email) {
            return true;
        }
        match email.rsplit_once('@') {
            Some((_, domain)) => self.domains.contains(domain),
            None => false,
        }
    }

    /// One line for the startup log, so an operator can see what the running
    /// process actually parsed rather than what they think they typed.
    pub fn describe(&self) -> String {
        if self.is_empty() {
            return "empty (email sign-in disabled)".to_string();
        }
        let mut parts: Vec<String> = self.addresses.iter().cloned().collect();
        parts.extend(self.domains.iter().map(|d| format!("@{d}")));
        parts.join(", ")
    }
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
    /// Postmark API origin. Overridable via `POSTMARK_API_BASE` so tests can
    /// point the send at a local stub — an allowlist whose "this address is
    /// admitted" direction is unverifiable would only be half-tested, and
    /// nothing else about this flow can be exercised without a mail sink.
    pub api_base: String,
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
        let api_base = std::env::var("POSTMARK_API_BASE")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "https://api.postmarkapp.com".to_string());
        Some(Self {
            server_token,
            from,
            message_stream,
            api_base,
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
        .post(format!("{}/email", cfg.api_base.trim_end_matches('/')))
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

    /// The whole reason this type exists: no configuration must never mean
    /// "anyone may make this deployment send mail".
    #[test]
    fn an_unset_allowlist_admits_nobody() {
        let empty = LoginAllowlist::parse("");
        assert!(empty.is_empty());
        assert!(!empty.allows("lilith@imazen.io"));
        assert!(!empty.allows("anyone@example.com"));
    }

    #[test]
    fn exact_addresses_match_case_insensitively() {
        let a = LoginAllowlist::parse("Lilith@Imazen.IO");
        assert!(a.allows("lilith@imazen.io"));
        assert!(!a.allows("someone-else@imazen.io"));
        assert!(!a.allows("lilith@example.com"));
        // Substring near-misses must not slip through.
        assert!(!a.allows("evil-lilith@imazen.io"));
        assert!(!a.allows("lilith@imazen.io.evil.test"));
    }

    #[test]
    fn domain_rules_match_the_whole_domain_only() {
        let a = LoginAllowlist::parse("@imazen.io");
        assert!(a.allows("lilith@imazen.io"));
        assert!(a.allows("anyone@imazen.io"));
        assert!(!a.allows("lilith@notimazen.io"));
        assert!(!a.allows("lilith@imazen.io.evil.test"));
        // Sub-domains are a different domain; list them explicitly if wanted.
        assert!(!a.allows("lilith@mail.imazen.io"));
    }

    #[test]
    fn separators_and_blank_entries_are_tolerated() {
        let a = LoginAllowlist::parse(" a@x.test ,, b@y.test\n@z.test\t;  ");
        assert!(a.allows("a@x.test"));
        assert!(a.allows("b@y.test"));
        assert!(a.allows("anyone@z.test"));
        assert!(!a.allows("c@w.test"));
    }

    /// A malformed entry must be dropped, never widened into something that
    /// matches more than the operator wrote.
    #[test]
    fn junk_entries_are_dropped_rather_than_widening_the_list() {
        for junk in [
            "@",
            "@localhost",
            "not-an-email",
            "@.",
            "  @  ",
            "@imazen.io.",
            "@.imazen.io",
        ] {
            let a = LoginAllowlist::parse(junk);
            assert!(a.is_empty(), "{junk:?} should have parsed to nothing");
            assert!(!a.allows("lilith@imazen.io"));
            assert!(!a.allows("root@localhost"));
        }
        // Junk alongside a good entry keeps the good one and only the good one.
        let a = LoginAllowlist::parse("@, lilith@imazen.io, nonsense");
        assert!(a.allows("lilith@imazen.io"));
        assert!(!a.allows("someone@elsewhere.test"));
    }

    #[test]
    fn describe_reports_what_was_parsed() {
        assert_eq!(
            LoginAllowlist::parse("").describe(),
            "empty (email sign-in disabled)"
        );
        assert_eq!(
            LoginAllowlist::parse("b@x.test, a@x.test, @y.test").describe(),
            "a@x.test, b@x.test, @y.test"
        );
    }

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
