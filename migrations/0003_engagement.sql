-- Engagement / motivation / compensation v0.1 footprint.
-- See docs/motivation-and-compensation.md for design rationale.
-- Account tier promotion (T1 email, T2 passkey) and payout machinery are v0.3+.

-- Account / engagement state on the observer
ALTER TABLE observers ADD COLUMN account_tier INTEGER NOT NULL DEFAULT 0;     -- 0=anon, 1=email, 2=passkey, 3=researcher
ALTER TABLE observers ADD COLUMN email TEXT;                                  -- T1+
ALTER TABLE observers ADD COLUMN email_verified_at INTEGER;
ALTER TABLE observers ADD COLUMN display_name TEXT;                           -- pseudonymous, opt-in
ALTER TABLE observers ADD COLUMN streak_days INTEGER NOT NULL DEFAULT 0;
ALTER TABLE observers ADD COLUMN streak_last_date TEXT;                       -- ISO YYYY-MM-DD (TZ: observer's, see notes)
ALTER TABLE observers ADD COLUMN freezes_remaining INTEGER NOT NULL DEFAULT 1;
ALTER TABLE observers ADD COLUMN skill_score REAL;                            -- Bayesian, 0..1; null until calibrated
ALTER TABLE observers ADD COLUMN total_trials INTEGER NOT NULL DEFAULT 0;     -- denormalised, server-maintained
ALTER TABLE observers ADD COLUMN compensation_mode TEXT NOT NULL DEFAULT 'volunteer'
    CHECK (compensation_mode IN ('volunteer','charity','paid'));
ALTER TABLE observers ADD COLUMN charity_choice TEXT;                         -- nullable; matches a key in compensation config
ALTER TABLE observers ADD COLUMN weekly_digest_optin INTEGER NOT NULL DEFAULT 0;
ALTER TABLE observers ADD COLUMN named_credit_optin INTEGER NOT NULL DEFAULT 0;
ALTER TABLE observers ADD COLUMN gdpr_consent_at INTEGER;
ALTER TABLE observers ADD COLUMN gdpr_consent_version INTEGER;
ALTER TABLE observers ADD COLUMN data_region TEXT;                            -- 'eu','us','other' — observer-self-reported or geoip-bin

-- Themes (corpus partitioning + autonomy support)
CREATE TABLE corpus_themes (
    slug         TEXT PRIMARY KEY,        -- 'nature', 'art', 'in_the_wild', ...
    display_name TEXT NOT NULL,
    description  TEXT,
    is_default   INTEGER NOT NULL DEFAULT 0,
    is_wow       INTEGER NOT NULL DEFAULT 0,    -- "wow" surfacing for variable-reward
    enabled      INTEGER NOT NULL DEFAULT 1
);

-- Map an existing source_hash to its theme(s). Themes are tags, not partitions —
-- one source can carry multiple. v0.2 ingestion populates this.
CREATE TABLE corpus_image_themes (
    source_hash TEXT NOT NULL,
    theme_slug  TEXT NOT NULL REFERENCES corpus_themes(slug),
    PRIMARY KEY (source_hash, theme_slug)
);

-- Sessions: which theme the observer asked for; soft-cap markers
ALTER TABLE sessions ADD COLUMN theme_slug TEXT REFERENCES corpus_themes(slug);
ALTER TABLE sessions ADD COLUMN counted_trials INTEGER NOT NULL DEFAULT 0;     -- post-warmup, post-grading
ALTER TABLE sessions ADD COLUMN soft_capped_at_trial INTEGER;                  -- when the soft cap fired

-- Badges (milestone celebrations + named credit)
CREATE TABLE badges (
    slug         TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    description  TEXT,
    icon_emoji   TEXT
);

CREATE TABLE observer_badges (
    observer_id TEXT NOT NULL REFERENCES observers(id),
    badge_slug  TEXT NOT NULL REFERENCES badges(slug),
    awarded_at  INTEGER NOT NULL,
    PRIMARY KEY (observer_id, badge_slug)
);

-- Seed the default themes
INSERT INTO corpus_themes (slug, display_name, description, is_default) VALUES
    ('nature',       'Nature',         'Landscapes, wildlife, plants', 1),
    ('art',          'Art',            'Paintings, drawings, prints',  0),
    ('in_the_wild',  'In the wild',    'Mixed real-world photos',      0);

-- Seed the milestone badges
INSERT INTO badges (slug, display_name, description, icon_emoji) VALUES
    ('first_trial',     'First rating',     'Submitted your first rating',                NULL),
    ('first_10',        'Warming up',       '10 trials contributed',                      NULL),
    ('first_50',        'Calibrated',       '50 trials contributed',                      NULL),
    ('first_100',       'Centurion',        '100 trials contributed',                     NULL),
    ('first_250',       'Quartermaster',    '250 trials contributed',                     NULL),
    ('first_500',       'Full deck',        '500 trials contributed',                     NULL),
    ('first_1000',      'Kilo',             '1,000 trials contributed',                   NULL),
    ('streak_3',        '3-day streak',     'Returned three days in a row',               NULL),
    ('streak_7',        'Week-long',        'Seven-day streak',                           NULL),
    ('streak_30',       'Monthly habit',    'Thirty-day streak',                          NULL),
    ('skill_calibrated','Sharp eye',        'Skill score above 0.85 on goldens',          NULL);
