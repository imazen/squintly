-- 0008_suggestions.sql — public corpus suggestions / uploads.
--
-- Anyone can POST a candidate image to /api/suggestions with license info,
-- original page URL, and (mandatory) submitter email. Squintly stores the
-- bytes under SQUINTLY_SUGGESTIONS_DIR/{sha[:2]}/{sha[2:4]}/{sha}.{ext}.
-- A reviewer can later promote the row into curator_candidates.
--
-- Per project decision: suggestions live forever — no expiry, no cleanup.
-- Status flips between 'pending' / 'accepted' / 'rejected' / 'withdrawn',
-- but rows (and files) stay.

CREATE TABLE IF NOT EXISTS suggestions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    sha256          TEXT NOT NULL,
    submitted_at    INTEGER NOT NULL,
    -- Submitter contact. submitter_observer_id, when present, points at
    -- observers.id (i.e. the uploader was logged in via the magic-link flow).
    -- submitter_email is the captured email at submit time, even if it's
    -- different from the observer's verified one (we don't second-guess).
    submitter_email           TEXT NOT NULL,
    submitter_observer_id     TEXT,
    submitter_email_verified  INTEGER NOT NULL DEFAULT 0,
    -- Provenance
    original_page_url   TEXT NOT NULL,
    original_image_url  TEXT,
    -- License declaration
    license_id              TEXT NOT NULL,            -- key from licensing registry or 'self' / 'other'
    license_text_freeform   TEXT,
    attribution             TEXT,
    why                     TEXT,
    -- Stored file
    file_path       TEXT NOT NULL,                    -- absolute path on disk
    file_size_bytes INTEGER NOT NULL,
    mime_type       TEXT,
    width           INTEGER,
    height          INTEGER,
    -- Lifecycle
    status          TEXT NOT NULL DEFAULT 'pending',  -- 'pending' | 'accepted' | 'rejected' | 'withdrawn'
    status_reason   TEXT,
    reviewed_at     INTEGER,
    reviewer_email  TEXT,
    accepted_candidate_sha256 TEXT,                   -- when promoted to curator_candidates
    -- Notification ledger (best-effort)
    notification_sent_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_suggestions_status ON suggestions(status, submitted_at);
CREATE INDEX IF NOT EXISTS idx_suggestions_sha    ON suggestions(sha256);
CREATE INDEX IF NOT EXISTS idx_suggestions_email  ON suggestions(submitter_email);
