-- Optional email magic-link auth. Pattern adapted from Weaver
-- (/home/lilith/fun/weaver/convex/auth.ts):
-- - Tokens are 32 bytes of crypto random, hex-encoded (64 chars).
-- - Only the BLAKE3 hash is stored — the plaintext lives only in the email URL.
-- - 15-minute TTL; single-use.
-- - On verify, observer rows are merged so the email always resolves to the
--   "canonical" observer for that user across devices.

CREATE TABLE auth_tokens (
    token_hash    TEXT PRIMARY KEY,         -- BLAKE3 hex digest of the plaintext token
    email         TEXT NOT NULL,
    -- Observer that requested the link. On verify we either:
    --   1. find an existing observer with this email and merge requesting_observer_id into it,
    --   2. or set observers.email = email on requesting_observer_id (first sign-in).
    requesting_observer_id TEXT REFERENCES observers(id),
    expires_at    INTEGER NOT NULL,         -- unix ms
    consumed_at   INTEGER,                  -- NULL until single-use
    created_at    INTEGER NOT NULL
);
CREATE INDEX idx_auth_tokens_email      ON auth_tokens(email);
CREATE INDEX idx_auth_tokens_expires_at ON auth_tokens(expires_at);

-- Index on observers.email so the verify path can quickly find an existing
-- canonical observer for cross-device sign-in. Email is not unique here (a
-- prior unverified record could exist) — verify resolves the canonical one.
CREATE INDEX idx_observers_email ON observers(email) WHERE email IS NOT NULL;

-- When two observer rows are merged, the loser row's trials/sessions point at
-- the winner via this redirect table. The frontend's localStorage observer_id
-- gets rewritten on verify, but old in-flight trials may still reference the
-- merged-away id; we keep the redirect rather than rewrite history.
CREATE TABLE observer_aliases (
    alias_id      TEXT PRIMARY KEY REFERENCES observers(id),
    canonical_id  TEXT NOT NULL REFERENCES observers(id),
    merged_at     INTEGER NOT NULL
);
