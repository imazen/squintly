-- Manifest snapshot pinning. Closes the last data-sprawl exit gate from
-- README §7 ("Curator R2 snapshot tag pinned in each session's
-- study_commit so a re-run is reproducible"). Without this, six months
-- after the pilot we cannot answer "what curator manifest was loaded
-- when observer X took session Y?" — the curator's `load_r2_public`
-- previously fetched + upserted but kept no trace of the source.

CREATE TABLE manifest_snapshots (
    id              TEXT PRIMARY KEY,
    loaded_at       INTEGER NOT NULL,
    r2_public_base  TEXT NOT NULL,
    manifest_path   TEXT NOT NULL,           -- e.g. "manifest.jsonl"
    manifest_sha256 TEXT NOT NULL,           -- sha256 of the raw fetched body
    body_bytes      INTEGER NOT NULL,
    n_candidates    INTEGER NOT NULL,        -- post-license-filter count
    UNIQUE (r2_public_base, manifest_path, manifest_sha256)
);

CREATE INDEX idx_manifest_snapshots_loaded ON manifest_snapshots(loaded_at DESC);

-- Sessions started after the snapshot landed get pinned. NULL means
-- either pre-snapshot or an FS-coefficient run with no R2 manifest.
ALTER TABLE sessions ADD COLUMN manifest_snapshot_id TEXT
    REFERENCES manifest_snapshots(id);

CREATE INDEX idx_sessions_manifest_snapshot
    ON sessions(manifest_snapshot_id)
    WHERE manifest_snapshot_id IS NOT NULL;
