-- Participant exclusion disposition: the recorded decision about whether an
-- observer's data counts, and the statistics behind it.
--
-- Deliberately a record, not a delete. The IQA literature is consistent that
-- hard-reject screening is the weaker tool — BT.500's procedure "loses all data
-- from rejected subjects" and draws a sharp boundary where a slightly-noisy
-- subject is treated like a careful one (zenpapers ch3-5 §4.2.2, tracing the
-- LIVE-Meta avatar paper §III-E). So every rating stays in `responses`; this
-- table only says what a screen concluded, and consumers decide whether to
-- honour it. That is what lets one dataset report both screened and unscreened
-- numbers.
--
-- Rebuilt wholesale by the nightly batch, like `observer_grades`.
CREATE TABLE observer_dispositions (
    observer_id      TEXT PRIMARY KEY REFERENCES observers(id),
    -- Which study's ratings were screened. Screens compare an observer against
    -- their peers, and two studies measure different quantities, so pooling
    -- them would build the reference distribution out of incomparable scores.
    study_id         TEXT,
    n_ratings        INTEGER NOT NULL,
    -- Ratings on stimuli that had enough *other* observers to compare against.
    n_comparable     INTEGER NOT NULL,
    mean_rating      REAL,
    sd_rating        REAL,
    -- BT.500 §A.1 kurtosis β₂ = m₄/m₂² over this observer's own scores, and
    -- the rejection band it selected (2σ if 2 ≤ β₂ ≤ 4, else √20 σ).
    beta2            REAL,
    band_sigma       REAL,
    outlier_count    INTEGER NOT NULL,
    outlier_rate     REAL,
    -- ch3-5 §4.4: Pearson correlation with the per-stimulus mean over others.
    r_s              REAL,
    -- 'included' | 'excluded' | 'insufficient_data'. The third is not a
    -- synonym for the first: "we checked and they're fine" and "we could not
    -- check" must stay distinguishable in an audit.
    disposition      TEXT NOT NULL,
    reason           TEXT,
    -- Whether the policy was in force when this was computed, so an export can
    -- be reproduced knowing which way the switch was set.
    policy_enabled   INTEGER NOT NULL,
    computed_at      INTEGER NOT NULL
);
CREATE INDEX idx_observer_dispositions_disposition ON observer_dispositions(disposition);
