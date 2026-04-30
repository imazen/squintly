-- Participant grading & outlier management.
-- See docs/participant-grading.md for design rationale.

ALTER TABLE observers ADD COLUMN qualifier_passed INTEGER;
ALTER TABLE observers ADD COLUMN qualifier_score INTEGER;
ALTER TABLE observers ADD COLUMN trusted_pool INTEGER NOT NULL DEFAULT 0;

ALTER TABLE sessions ADD COLUMN session_grade TEXT;             -- 'A'..'F'
ALTER TABLE sessions ADD COLUMN session_weight REAL NOT NULL DEFAULT 1.0;
ALTER TABLE sessions ADD COLUMN flagged_terminate INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN golden_pass_rate REAL;
ALTER TABLE sessions ADD COLUMN straight_line_max INTEGER;
ALTER TABLE sessions ADD COLUMN straight_line_ratio REAL;       -- KonIQ line-clicker ratio
ALTER TABLE sessions ADD COLUMN rt_below_floor_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN no_reveal_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN even_odd_r REAL;
ALTER TABLE sessions ADD COLUMN n_trials INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN n_pair_trials INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN graded_at INTEGER;

ALTER TABLE trials ADD COLUMN expected_choice TEXT;             -- for goldens

ALTER TABLE responses ADD COLUMN response_flags TEXT;           -- comma-separated

CREATE TABLE observer_grades (
    observer_id          TEXT PRIMARY KEY REFERENCES observers(id),
    computed_at          INTEGER NOT NULL,
    n_trials             INTEGER NOT NULL,
    n_sessions           INTEGER NOT NULL,
    quality_grade        TEXT NOT NULL,
    weight               REAL NOT NULL,
    pwcmp_log_lik        REAL,
    pwcmp_dist_l         REAL,
    cid22_mean_norm_diff REAL,
    cid22_sd_norm_diff   REAL,
    even_odd_r           REAL,
    sigma_acr            REAL,
    delta_acr            REAL,
    golden_pass_rate     REAL,
    notes                TEXT
);
CREATE INDEX idx_observer_grades_grade ON observer_grades(quality_grade);
CREATE INDEX idx_responses_flags ON responses(response_flags) WHERE response_flags IS NOT NULL;
