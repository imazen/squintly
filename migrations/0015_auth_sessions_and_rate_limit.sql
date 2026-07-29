-- Rate limiting for magic-link requests, and real signed-in sessions.
--
-- Two changes, both consequences of opening sign-in to any address:
--
-- 1. `/api/auth/start` mails a link to whatever address the caller names, so
--    with no allowlist in front of it the only thing keeping it from being a
--    mail cannon is a rate limit. We count recent rows in `auth_tokens` rather
--    than keep a separate counter table: a token row is already written for
--    every accepted request, so it *is* the request log. The added indexes are
--    what make that count cheap.
--
-- 2. `requester_ip_hash` is a salted BLAKE3 bucket, never a raw address —
--    limiting per-address alone is trivially defeated by cycling addresses, and
--    the project's stated posture is "no IP logging beyond a hashed bucket".
ALTER TABLE auth_tokens ADD COLUMN requester_ip_hash TEXT;
CREATE INDEX idx_auth_tokens_email_created ON auth_tokens(email, created_at);
CREATE INDEX idx_auth_tokens_ip_created
    ON auth_tokens(requester_ip_hash, created_at)
    WHERE requester_ip_hash IS NOT NULL;

-- Signed-in sessions. Until now `auth_verify` handed the browser an observer id
-- and nothing else, so "signed in" was a client-side claim the server never
-- checked — which is why admin actions could only be gated by a shared token.
-- A session is a second 32-byte secret, stored hashed exactly like a magic-link
-- token, carried in an HttpOnly cookie.
--
-- `email` is stored so admin status can be resolved from the *current*
-- allowlist on every request. Deliberately no `is_admin` column: a grant
-- snapshotted at sign-in time would outlive the operator's removal from the
-- allowlist, which is the wrong direction to fail in.
CREATE TABLE auth_sessions (
    token_hash  TEXT PRIMARY KEY,          -- BLAKE3 hex of the cookie value
    observer_id TEXT NOT NULL REFERENCES observers(id),
    email       TEXT NOT NULL,
    created_at  INTEGER NOT NULL,          -- unix ms
    expires_at  INTEGER NOT NULL,          -- unix ms
    revoked_at  INTEGER                    -- NULL until sign-out
);
CREATE INDEX idx_auth_sessions_observer ON auth_sessions(observer_id);
CREATE INDEX idx_auth_sessions_expires  ON auth_sessions(expires_at);
