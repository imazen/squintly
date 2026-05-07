-- 0007_curator.sql — corpus curator mode.
--
-- Adds three tables behind /api/curator/*:
--   - curator_candidates: in-DB cache of the candidate manifest the curator
--     streams over (TSV from corpus-builder OR JSONL from R2). Optional —
--     curator can also stream from an in-memory loaded manifest. Persisting
--     the candidate set lets us enumerate progress (i/N) and keeps "next"
--     stable across reloads.
--   - curator_decisions: take/reject/flag with group memberships and the
--     detected source profile.
--   - curator_size_variants: the size-variant chip selections (target max dims).
--   - curator_thresholds: per-(decision, size) q_imperceptible from the slider.
--
-- All anonymous (curator_id is a localStorage UUID, same shape as observers.id).
-- License columns surface per-source license metadata so the curator UI can
-- render attribution and the export TSVs can carry it downstream.

CREATE TABLE IF NOT EXISTS curator_candidates (
    sha256          TEXT PRIMARY KEY,         -- 64 hex chars, matches manifest
    corpus          TEXT NOT NULL,            -- e.g. 'unsplash-webp', 'source_jpegs'
    relative_path   TEXT,                     -- file-source manifest path (informational)
    width           INTEGER,
    height          INTEGER,
    size_bytes      INTEGER,
    format          TEXT,                     -- 'jpeg' | 'png' | 'webp' | 'avif' | 'jxl' | …
    suspected_category TEXT,                  -- e.g. 'photo_natural_or_detailed'
    has_alpha       INTEGER NOT NULL DEFAULT 0,
    has_animation   INTEGER NOT NULL DEFAULT 0,
    license_id      TEXT,                     -- key into licensing.rs LICENSE_REGISTRY
    license_url     TEXT,                     -- per-image attribution URL when known
    blob_url        TEXT NOT NULL,            -- where to GET the bytes (R2 blob URL or local proxy)
    added_at        INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    order_hint      INTEGER NOT NULL DEFAULT 0  -- lower = earlier in stream
);

CREATE INDEX IF NOT EXISTS idx_curator_candidates_order
    ON curator_candidates(order_hint, sha256);

CREATE TABLE IF NOT EXISTS curator_decisions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    source_sha256   TEXT NOT NULL,
    curator_id      TEXT NOT NULL,
    decided_at      INTEGER NOT NULL,
    decision        TEXT NOT NULL,            -- 'take' | 'reject' | 'flag'
    reject_reason   TEXT,
    -- Group membership; NULL/0 means "not in this group"
    in_core_zensim       INTEGER NOT NULL DEFAULT 0,
    in_medium_zensim     INTEGER NOT NULL DEFAULT 0,
    in_full_zensim       INTEGER NOT NULL DEFAULT 0,
    in_core_encoding     INTEGER NOT NULL DEFAULT 0,
    in_medium_encoding   INTEGER NOT NULL DEFAULT 0,
    in_full_encoding     INTEGER NOT NULL DEFAULT 0,
    -- Source profile
    source_codec         TEXT,
    source_q_detected    REAL,
    source_w             INTEGER NOT NULL,
    source_h             INTEGER NOT NULL,
    recommended_max_dim  INTEGER,
    -- Viewing context that produced the decision (for reproducibility)
    decision_dpr         REAL,
    decision_viewport_w  INTEGER,
    decision_viewport_h  INTEGER,
    UNIQUE (source_sha256, curator_id)
);

CREATE INDEX IF NOT EXISTS idx_curator_decisions_curator
    ON curator_decisions(curator_id, decided_at);

CREATE TABLE IF NOT EXISTS curator_size_variants (
    decision_id     INTEGER NOT NULL REFERENCES curator_decisions(id) ON DELETE CASCADE,
    target_max_dim  INTEGER NOT NULL,
    generated_sha256 TEXT,
    generated_path   TEXT,
    PRIMARY KEY (decision_id, target_max_dim)
);

CREATE TABLE IF NOT EXISTS curator_thresholds (
    decision_id     INTEGER NOT NULL REFERENCES curator_decisions(id) ON DELETE CASCADE,
    target_max_dim  INTEGER NOT NULL,
    q_imperceptible REAL NOT NULL,
    measured_at     INTEGER NOT NULL,
    measurement_dpr REAL NOT NULL,
    measurement_distance_cm REAL,
    -- Encoder identity, so versions don't get conflated downstream.
    encoder_label   TEXT NOT NULL DEFAULT 'browser-canvas-jpeg',
    PRIMARY KEY (decision_id, target_max_dim)
);
