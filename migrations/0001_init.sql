-- Squintly schema v0.1 — see SPEC.md for design rationale.

CREATE TABLE observers (
    id            TEXT PRIMARY KEY,
    created_at    INTEGER NOT NULL,
    user_agent    TEXT,
    age_bracket   TEXT,
    vision_corrected TEXT
);

CREATE TABLE sessions (
    id                   TEXT PRIMARY KEY,
    observer_id          TEXT NOT NULL REFERENCES observers(id),
    started_at           INTEGER NOT NULL,
    ended_at             INTEGER,
    device_pixel_ratio   REAL NOT NULL,
    screen_width_css     INTEGER NOT NULL,
    screen_height_css    INTEGER NOT NULL,
    color_gamut          TEXT,
    dynamic_range_high   INTEGER,
    prefers_dark         INTEGER,
    pointer_type         TEXT,
    timezone             TEXT,
    -- self-report
    viewing_distance_cm  INTEGER,
    ambient_light        TEXT,
    -- calibration: CSS px per millimeter on this physical screen (Li 2020 chinrest)
    css_px_per_mm        REAL,
    -- tagging
    notes                TEXT
);

CREATE TABLE trials (
    id               TEXT PRIMARY KEY,
    session_id       TEXT NOT NULL REFERENCES sessions(id),
    kind             TEXT NOT NULL,        -- 'single' | 'pair'
    source_hash      TEXT NOT NULL,
    a_encoding_id    TEXT NOT NULL,
    a_codec          TEXT NOT NULL,
    a_quality        REAL,
    a_bytes          INTEGER,
    b_encoding_id    TEXT,
    b_codec          TEXT,
    b_quality        REAL,
    b_bytes          INTEGER,
    intrinsic_w      INTEGER NOT NULL,
    intrinsic_h      INTEGER NOT NULL,
    staircase_id     TEXT,
    staircase_target TEXT,                 -- 'notice' | 'dislike' | 'hate'
    staircase_step   INTEGER,
    is_golden        INTEGER NOT NULL DEFAULT 0,
    served_at        INTEGER NOT NULL
);

CREATE TABLE responses (
    trial_id              TEXT PRIMARY KEY REFERENCES trials(id),
    choice                TEXT NOT NULL,    -- '1'..'4' for single; 'a'/'b'/'tie'/'flag_broken' for pair
    dwell_ms              INTEGER NOT NULL,
    reveal_count          INTEGER NOT NULL,
    reveal_ms_total       INTEGER NOT NULL,
    zoom_used             INTEGER NOT NULL,
    viewport_w_css        INTEGER NOT NULL,
    viewport_h_css        INTEGER NOT NULL,
    orientation           TEXT NOT NULL,    -- 'portrait' | 'landscape'
    image_displayed_w_css REAL NOT NULL,
    image_displayed_h_css REAL NOT NULL,
    intrinsic_to_device_ratio REAL NOT NULL,
    pixels_per_degree     REAL,
    responded_at          INTEGER NOT NULL
);

CREATE TABLE staircases (
    id                 TEXT PRIMARY KEY,
    session_id         TEXT NOT NULL REFERENCES sessions(id),
    source_hash        TEXT NOT NULL,
    codec              TEXT NOT NULL,
    target             TEXT NOT NULL,    -- 'notice' | 'dislike' | 'hate'
    rule               TEXT NOT NULL,    -- '3down1up' | '2down1up' | '1down1up'
    started_at         INTEGER NOT NULL,
    converged          INTEGER NOT NULL DEFAULT 0,
    converged_quality  REAL,
    reversals          INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_trials_session  ON trials(session_id);
CREATE INDEX idx_trials_source   ON trials(source_hash);
CREATE INDEX idx_trials_staircase ON trials(staircase_id);
CREATE INDEX idx_responses_trial ON responses(trial_id);
CREATE INDEX idx_staircases_session ON staircases(session_id);
