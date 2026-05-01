-- v0.2 methodology rigor schema. Adds:
-- - corpus_anchors: per-source canonical (codec, quality) tuples used for
--   scale calibration and interpolation, plus is_honeypot flag for inline
--   verification (CID22 §Anchors and §Participant screening).
-- - calibration_pool: small fixed set of test trials used for the 5-trial
--   onboarding pass at session start (CID22 §4 training images,
--   adapted with answer feedback per Foldit / KonIQ qualifier).
-- - calibration_responses: the answers observers give during onboarding.
-- - observers.calibrated: 0/1 soft-flag indicating they passed onboarding.
-- - sources.held_out: 0/1, marks the held-out validation set per
--   docs/methodology.md §10.
-- - sources.codec_version: blob with provenance metadata (when coefficient
--   provides it).

CREATE TABLE corpus_anchors (
    source_hash    TEXT NOT NULL,
    encoding_id    TEXT NOT NULL,
    codec          TEXT NOT NULL,
    quality        REAL NOT NULL,
    role           TEXT NOT NULL,           -- 'anchor' | 'honeypot' | 'reference'
    expected_choice TEXT,                   -- for honeypots: '1'..'4' (single) or 'a'/'b'/'tie' (pair)
    notes          TEXT,
    PRIMARY KEY (source_hash, encoding_id)
);
CREATE INDEX idx_anchors_role  ON corpus_anchors(role);
CREATE INDEX idx_anchors_codec ON corpus_anchors(codec);

-- Held-out validation set discipline. 20% of sources will be marked
-- held_out=1; their data is collected but never feeds training, parameter
-- selection, or threshold calibration — only end-of-pipeline metric
-- evaluation. Plumbed through to the export TSVs as a column.
CREATE TABLE source_flags (
    source_hash TEXT PRIMARY KEY,
    held_out    INTEGER NOT NULL DEFAULT 0,
    codec_version_blob TEXT,                -- arbitrary JSON of (codec_name -> version)
    notes       TEXT
);

-- Calibration pool: a small, fixed set of pre-baked test trials used at
-- session start. Each entry is its own self-contained trial with a known
-- expected answer; they don't reference the corpus_anchors table because
-- they may use synthetic or out-of-corpus stimuli for IMC purposes.
CREATE TABLE calibration_pool (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL,            -- 'single' | 'pair' | 'imc'
    description   TEXT NOT NULL,
    -- For 'single' / 'pair': URLs and codec metadata.
    source_url    TEXT,
    a_url         TEXT,
    b_url         TEXT,
    a_codec       TEXT,
    b_codec       TEXT,
    a_quality     REAL,
    b_quality     REAL,
    -- Intrinsic dimensions for the displayed image (single or pair-A).
    intrinsic_w   INTEGER,
    intrinsic_h   INTEGER,
    -- Expected answer.
    expected_choice TEXT NOT NULL,           -- '1','2','3','4' / 'a','b','tie'
    -- Optional caption shown after the response (the "answer" feedback).
    feedback_text TEXT,
    -- IMC trials use a one-line instruction in feedback_text and ignore
    -- source_url/a_url/b_url; the frontend renders a text-only "tap tie"
    -- gate.
    -- Optional ordering hint; ascending = more important to ask early.
    order_hint    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE calibration_responses (
    id            TEXT PRIMARY KEY,
    observer_id   TEXT NOT NULL REFERENCES observers(id),
    session_id    TEXT NOT NULL REFERENCES sessions(id),
    pool_id       TEXT NOT NULL REFERENCES calibration_pool(id),
    choice        TEXT NOT NULL,
    correct       INTEGER NOT NULL,         -- 0/1
    dwell_ms      INTEGER NOT NULL,
    served_at     INTEGER NOT NULL,
    responded_at  INTEGER NOT NULL
);
CREATE INDEX idx_calibration_responses_obs ON calibration_responses(observer_id);

ALTER TABLE observers ADD COLUMN calibrated INTEGER NOT NULL DEFAULT 0;
ALTER TABLE observers ADD COLUMN calibration_score REAL;
ALTER TABLE observers ADD COLUMN calibrated_at INTEGER;

-- Trial-level held-out flag (denormalised so exports don't need to join).
ALTER TABLE trials ADD COLUMN held_out INTEGER NOT NULL DEFAULT 0;
