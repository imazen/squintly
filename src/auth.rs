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
//! - Sign-in is open to **any** address. Linking an email is how a participant
//!   carries their observer ID to a second device; gating it would lock real
//!   participants out of their own data. What keeps the start endpoint from
//!   being a mail cannon is [`RateLimit`], not an allowlist.
//! - [`EmailAllowlist`] gates **admin** instead (`SQUINTLY_ADMIN_EMAILS`), and
//!   there an unset variable grants nobody — a privilege nobody needs in order
//!   to take part should fail closed.
//! - Signing in mints a session (see [`SESSION_COOKIE`]) so the server can tell
//!   who someone is on later requests. Before this, "signed in" was a
//!   client-side claim the server never checked.
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

/// A set of email addresses, used to grant a capability to named people.
///
/// This gates **admin**, not sign-in. Sign-in itself is open to any address:
/// an observer linking an email is how they carry an existing observer ID to a
/// second device, which is a participant feature and not a privilege. Closing
/// it would lock real participants out of their own data to solve a problem
/// that a rate limit solves better (see [`RateLimit`]).
///
/// Admin is the opposite case — it is a privilege, nobody needs it to take
/// part, and the safe direction on a public deployment is that an unset
/// variable grants it to **nobody**. Empty therefore means "no admins", never
/// "everyone".
///
/// Entries are separated by commas or whitespace and are case-insensitive:
///   * `someone@example.com` — that one address;
///   * `@example.com` — any address at that domain (not its sub-domains).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EmailAllowlist {
    addresses: BTreeSet<String>,
    domains: BTreeSet<String>,
}

/// Addresses that get admin once signed in.
pub const ADMIN_EMAILS_ENV: &str = "SQUINTLY_ADMIN_EMAILS";

impl EmailAllowlist {
    /// The admin roster, re-read per request so that removing someone from the
    /// variable takes effect at once rather than at their next sign-in.
    pub fn admins() -> Self {
        Self::parse(&std::env::var(ADMIN_EMAILS_ENV).unwrap_or_default())
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
                    "ignoring allowlist entry: a domain rule must look like '@example.com'"
                ),
                None if looks_like_email(&entry) => {
                    addresses.insert(entry);
                }
                None => tracing::warn!(
                    entry = %entry,
                    "ignoring allowlist entry: not an email address or an '@domain' rule"
                ),
            }
        }
        Self { addresses, domains }
    }

    pub fn is_empty(&self) -> bool {
        self.addresses.is_empty() && self.domains.is_empty()
    }

    /// `email` must already be trimmed and lowercased, as the auth handlers do
    /// before they validate its shape.
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
            return "empty (no admins)".to_string();
        }
        let mut parts: Vec<String> = self.addresses.iter().cloned().collect();
        parts.extend(self.domains.iter().map(|d| format!("@{d}")));
        parts.join(", ")
    }
}

/// How often one address, and one client, may ask for a magic link.
///
/// With sign-in open to any address this is the whole defence on
/// `/api/auth/start`. Two limits, because either alone is trivially defeated:
/// per-address stops one inbox being buried, and per-client stops the same
/// caller cycling through a list of victims' addresses to sidestep it.
///
/// Both windows are enforced against `auth_tokens`, which already gets a row
/// per accepted request — the request log and the token store are the same
/// table, so there is no counter to drift out of sync with reality.
#[derive(Debug, Clone, Copy)]
pub struct RateLimit {
    /// Minimum gap between two links to the same address.
    pub per_email_cooldown_ms: i64,
    /// Links per address per hour.
    pub per_email_hourly: i64,
    /// Links per client IP bucket per hour, across all addresses.
    pub per_ip_hourly: i64,
}

impl Default for RateLimit {
    fn default() -> Self {
        Self {
            // Long enough that a bored attacker cannot fill an inbox, short
            // enough that a real person who mistyped their address, or whose
            // first mail went to spam, can just try again.
            per_email_cooldown_ms: 60_000,
            per_email_hourly: 5,
            // Generous for shared egress (an office, a university, carrier-grade
            // NAT) while still bounding a single source's total send volume.
            per_ip_hourly: 20,
        }
    }
}

impl RateLimit {
    pub fn from_env() -> Self {
        let d = Self::default();
        let num = |key: &str, default: i64| -> i64 {
            match std::env::var(key) {
                Ok(v) => match v.trim().parse::<i64>() {
                    Ok(n) if n >= 0 => n,
                    _ => {
                        tracing::warn!(key, value = %v, "unparseable rate limit; using default");
                        default
                    }
                },
                Err(_) => default,
            }
        };
        Self {
            per_email_cooldown_ms: num("SQUINTLY_AUTH_COOLDOWN_MS", d.per_email_cooldown_ms),
            per_email_hourly: num("SQUINTLY_AUTH_PER_EMAIL_HOURLY", d.per_email_hourly),
            per_ip_hourly: num("SQUINTLY_AUTH_PER_IP_HOURLY", d.per_ip_hourly),
        }
    }
}

/// What a rate-limit check decided, and how long to wait if it refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateVerdict {
    Allow,
    Deny { retry_after_s: i64, reason: String },
}

/// Decide from counts already fetched, so the policy is testable without a
/// database and the SQL stays in the handler.
pub fn rate_verdict(
    limit: &RateLimit,
    now_ms: i64,
    last_for_email_ms: Option<i64>,
    email_count_last_hour: i64,
    ip_count_last_hour: i64,
) -> RateVerdict {
    if let Some(last) = last_for_email_ms {
        let since = now_ms.saturating_sub(last);
        if limit.per_email_cooldown_ms > 0 && since < limit.per_email_cooldown_ms {
            let remaining = limit.per_email_cooldown_ms - since;
            let wait = ((remaining + 999) / 1000).max(1);
            return RateVerdict::Deny {
                retry_after_s: wait,
                reason: format!("a sign-in link was just sent to this address; wait {wait}s"),
            };
        }
    }
    if limit.per_email_hourly > 0 && email_count_last_hour >= limit.per_email_hourly {
        return RateVerdict::Deny {
            retry_after_s: 3600,
            reason: format!(
                "this address has requested {} sign-in links in the last hour",
                email_count_last_hour
            ),
        };
    }
    if limit.per_ip_hourly > 0 && ip_count_last_hour >= limit.per_ip_hourly {
        return RateVerdict::Deny {
            retry_after_s: 3600,
            reason: "too many sign-in links requested from this network in the last hour".into(),
        };
    }
    RateVerdict::Allow
}

/// Bucket a client address into an opaque, salted hash.
///
/// Never stores or logs the address itself: the project's posture is no IP
/// logging beyond a hashed bucket, and a bare hash of an IPv4 address is
/// reversible by brute force in seconds (there are only 2^32 of them). The
/// salt is what makes the bucket opaque, so a deployment that wants that
/// property must set `SQUINTLY_IP_HASH_SALT`; without it we say so once and
/// fall back to a process-lifetime random salt, which still defeats offline
/// reversal but resets the counters on restart.
pub fn hash_ip(ip: &str, salt: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(salt.as_bytes());
    h.update(b"\x00");
    h.update(ip.as_bytes());
    h.finalize().to_hex()[..32].to_string()
}

/// Client address for rate limiting, from the proxy header when present.
///
/// Railway (like any PaaS) terminates TLS in front of the app, so the socket
/// peer is the proxy and `X-Forwarded-For` carries the client. We take the
/// **first** entry, which is what the closest trusted proxy appended. A client
/// can forge additional entries; that only lets them *split* their own bucket,
/// which the per-address limit still bounds.
pub fn client_ip(forwarded_for: Option<&str>, peer: Option<&str>) -> Option<String> {
    if let Some(xff) = forwarded_for {
        if let Some(first) = xff.split(',').map(str::trim).find(|s| !s.is_empty()) {
            return Some(first.to_string());
        }
    }
    peer.map(|s| s.to_string())
}

/// Name of the cookie carrying a signed-in session.
pub const SESSION_COOKIE: &str = "squintly_session";

/// Build the `Set-Cookie` value for a freshly minted session.
///
/// `HttpOnly` so script cannot read it (the observer ID stays in localStorage
/// for anonymous use; this is a different, higher-value secret). `SameSite=Lax`
/// because the cookie has to survive the top-level navigation *from the email
/// client* into `/api/auth/verify` — `Strict` would drop it on exactly that
/// hop and sign-in would silently never take. `Secure` unless we're on plain
/// HTTP for local dev, since a Secure cookie is discarded outright over
/// `http://localhost` in some browsers.
pub fn session_cookie(token: &str, secure: bool, max_age_s: i64) -> String {
    let mut c =
        format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_s}");
    if secure {
        c.push_str("; Secure");
    }
    c
}

/// Read our session cookie out of a `Cookie:` header.
pub fn session_from_cookie_header(header: &str) -> Option<String> {
    header.split(';').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        (k.trim() == SESSION_COOKIE).then(|| v.trim().to_string())
    })
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

    /// Empty must mean "nobody is an admin", never "everybody".
    #[test]
    fn an_unset_allowlist_admits_nobody() {
        let empty = EmailAllowlist::parse("");
        assert!(empty.is_empty());
        assert!(!empty.allows("lilith@imazen.io"));
        assert!(!empty.allows("anyone@example.com"));
    }

    #[test]
    fn exact_addresses_match_case_insensitively() {
        let a = EmailAllowlist::parse("Lilith@Imazen.IO");
        assert!(a.allows("lilith@imazen.io"));
        assert!(!a.allows("someone-else@imazen.io"));
        assert!(!a.allows("lilith@example.com"));
        // Substring near-misses must not slip through.
        assert!(!a.allows("evil-lilith@imazen.io"));
        assert!(!a.allows("lilith@imazen.io.evil.test"));
    }

    #[test]
    fn domain_rules_match_the_whole_domain_only() {
        let a = EmailAllowlist::parse("@imazen.io");
        assert!(a.allows("lilith@imazen.io"));
        assert!(a.allows("anyone@imazen.io"));
        assert!(!a.allows("lilith@notimazen.io"));
        assert!(!a.allows("lilith@imazen.io.evil.test"));
        // Sub-domains are a different domain; list them explicitly if wanted.
        assert!(!a.allows("lilith@mail.imazen.io"));
    }

    #[test]
    fn separators_and_blank_entries_are_tolerated() {
        let a = EmailAllowlist::parse(" a@x.test ,, b@y.test\n@z.test\t;  ");
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
            let a = EmailAllowlist::parse(junk);
            assert!(a.is_empty(), "{junk:?} should have parsed to nothing");
            assert!(!a.allows("lilith@imazen.io"));
            assert!(!a.allows("root@localhost"));
        }
        // Junk alongside a good entry keeps the good one and only the good one.
        let a = EmailAllowlist::parse("@, lilith@imazen.io, nonsense");
        assert!(a.allows("lilith@imazen.io"));
        assert!(!a.allows("someone@elsewhere.test"));
    }

    #[test]
    fn describe_reports_what_was_parsed() {
        assert_eq!(EmailAllowlist::parse("").describe(), "empty (no admins)");
        assert_eq!(
            EmailAllowlist::parse("b@x.test, a@x.test, @y.test").describe(),
            "a@x.test, b@x.test, @y.test"
        );
    }

    // ---------- rate limiting ----------

    const RL: RateLimit = RateLimit {
        per_email_cooldown_ms: 60_000,
        per_email_hourly: 5,
        per_ip_hourly: 20,
    };

    #[test]
    fn a_first_request_is_always_allowed() {
        assert_eq!(rate_verdict(&RL, 1_000_000, None, 0, 0), RateVerdict::Allow);
    }

    #[test]
    fn the_same_address_must_wait_out_the_cooldown() {
        let now = 1_000_000;
        match rate_verdict(&RL, now, Some(now - 10_000), 1, 1) {
            RateVerdict::Deny { retry_after_s, .. } => assert_eq!(retry_after_s, 50),
            v => panic!("expected a denial, got {v:?}"),
        }
        // One millisecond past the window is allowed — the boundary must not be
        // off by a whole cooldown.
        assert_eq!(
            rate_verdict(&RL, now, Some(now - 60_001), 1, 1),
            RateVerdict::Allow
        );
    }

    #[test]
    fn hourly_caps_apply_per_address_and_per_network() {
        let now = 1_000_000;
        // Cooldown satisfied, but the address has had its five.
        assert!(matches!(
            rate_verdict(&RL, now, Some(now - 120_000), 5, 0),
            RateVerdict::Deny { .. }
        ));
        // Fresh address, but this network has been cycling through addresses —
        // the case a per-address limit alone cannot see.
        assert!(matches!(
            rate_verdict(&RL, now, None, 0, 20),
            RateVerdict::Deny { .. }
        ));
        assert_eq!(rate_verdict(&RL, now, None, 0, 19), RateVerdict::Allow);
    }

    #[test]
    fn a_zero_limit_disables_that_rule_rather_than_blocking_everything() {
        let off = RateLimit {
            per_email_cooldown_ms: 0,
            per_email_hourly: 0,
            per_ip_hourly: 0,
        };
        let now = 1_000_000;
        assert_eq!(
            rate_verdict(&off, now, Some(now - 1), 9_999, 9_999),
            RateVerdict::Allow
        );
    }

    // ---------- client address handling ----------

    #[test]
    fn the_client_ip_comes_from_the_first_forwarded_entry() {
        // Railway terminates TLS in front of us, so the socket peer is the
        // proxy; the leftmost XFF entry is the client the proxy saw.
        assert_eq!(
            client_ip(Some("203.0.113.7, 10.0.0.1"), Some("10.0.0.1")).as_deref(),
            Some("203.0.113.7")
        );
        assert_eq!(
            client_ip(None, Some("198.51.100.4")).as_deref(),
            Some("198.51.100.4")
        );
        assert_eq!(
            client_ip(Some("  "), Some("198.51.100.4")).as_deref(),
            Some("198.51.100.4")
        );
        assert_eq!(client_ip(None, None), None);
    }

    #[test]
    fn the_ip_bucket_is_salted_so_it_cannot_be_brute_forced_back() {
        let a = hash_ip("203.0.113.7", "salt-one");
        let b = hash_ip("203.0.113.7", "salt-two");
        assert_ne!(a, b, "a different salt must give a different bucket");
        assert_eq!(a, hash_ip("203.0.113.7", "salt-one"), "must be stable");
        assert_ne!(a, hash_ip("203.0.113.8", "salt-one"));
        assert!(
            !a.contains("203.0.113"),
            "the address must not survive in the bucket"
        );
    }

    // ---------- session cookie ----------

    #[test]
    fn the_session_cookie_is_httponly_and_lax() {
        let c = session_cookie("abc", true, 100);
        assert!(c.contains("HttpOnly"), "script must not be able to read it");
        // Lax, not Strict: the cookie has to survive the top-level navigation
        // out of a mail client into /api/auth/verify, which Strict would drop.
        assert!(c.contains("SameSite=Lax"));
        assert!(c.contains("Secure"));
        assert!(c.contains("Max-Age=100"));
        assert!(!session_cookie("abc", false, 100).contains("Secure"));
    }

    #[test]
    fn the_session_cookie_is_found_among_others() {
        assert_eq!(
            session_from_cookie_header("foo=1; squintly_session=deadbeef; bar=2").as_deref(),
            Some("deadbeef")
        );
        assert_eq!(
            session_from_cookie_header("squintly_session=xyz").as_deref(),
            Some("xyz")
        );
        assert_eq!(session_from_cookie_header("foo=1; bar=2"), None);
        // Must not match a cookie that merely ends with our name.
        assert_eq!(session_from_cookie_header("not_squintly_session=xyz"), None);
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
