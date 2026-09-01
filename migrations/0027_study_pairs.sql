-- A pre-mined, pre-registered pair list that a study serves in a planned order.
--
-- Every pairing rule squintly had until now *derives* the pair at serve time
-- from the manifest: `AdjacentQuality` picks two rungs of one codec,
-- `RestorationVsBaseline` matches a restored encode to its input. Both are the
-- right shape when the question is about the corpus. They are the wrong shape
-- when the question is about two *metrics*.
--
-- An adjudication study asks: on the pairs where model M and SSIMULACRA2 order
-- two encodes DIFFERENTLY, which one does a human agree with? That set cannot
-- be drawn at serve time — deciding membership needs both scorers' opinions
-- over the whole ladder of every reference, which is an offline mining pass.
-- And it must not be re-drawn per session: the stimulus set is part of the
-- pre-registration, so it is fixed before the first judgment and the same list
-- is served in the same planned order.
--
-- # Why the plan lives in the database rather than in the sampler
--
-- The sampler is deliberately a pure function of the manifest, with no DB
-- access (see `sampling::pick_trial`). A planned sequence is state: which rows
-- have been served, to whom, in what order, and which are planned repeats of
-- which. That belongs beside the trials it produces, not in a pure sampler.
--
-- # Repeats are ROWS, not a probability
--
-- `Study::p_repeat` re-serves a random already-answered pair at some rate.
-- That is right for a crowd study where the ceiling is a population statistic.
-- For a single observer the repeat set IS the instrument — the intra-observer
-- consistency rate is what every claim gets divided by — so it is planned:
-- a repeat is an ordinary row with `repeat_of_pair` set, placed at a chosen
-- distance from its original. `p_repeat` is 0 for a manifest study and the two
-- mechanisms never both fire.
--
-- # `meta_json` is opaque here on purpose
--
-- It carries what the mining pass recorded about the pair: each scorer's
-- value on each side, the margins, the quality zone, the codec pair, the
-- stratum's own fields. The server does not interpret any of it — it round
-- trips to the export so the analysis can join a judgment back to the exact
-- disagreement it was chosen to settle. Interpreting it here would put the
-- analysis's schema in the serving path, where a change to one breaks the
-- other.
CREATE TABLE study_pairs (
    pair_id         TEXT    NOT NULL PRIMARY KEY,
    study_id        TEXT    NOT NULL,
    -- Planned serve order within the study. Not unique: the ingest may leave
    -- gaps or ties, and ties break on pair_id so the order is still total.
    seq             INTEGER NOT NULL,
    source_hash     TEXT    NOT NULL,
    a_encoding_id   TEXT    NOT NULL,
    b_encoding_id   TEXT    NOT NULL,
    -- Which stratum of the pre-registered design this row belongs to. Free
    -- text so a new study can name its own; the analysis groups on it.
    stratum         TEXT    NOT NULL,
    -- Non-null => this row is a planned exact repeat of another row, for the
    -- intra-observer consistency measurement.
    repeat_of_pair  TEXT REFERENCES study_pairs(pair_id),
    -- Non-null => the answer is known (a calibration / attention row). Uses the
    -- same 'a'/'b' vocabulary as `responses.choice`, in UNCOUNTERBALANCED
    -- terms: it names an ENCODING SIDE as ingested, and `counterbalance_pair`
    -- flips it with the slots exactly as it does for a golden pair.
    expected_choice TEXT,
    meta_json       TEXT    NOT NULL DEFAULT '{}',
    ingested_at     INTEGER NOT NULL
) STRICT;

-- The serving query: "next unserved row for this study, in planned order".
CREATE INDEX idx_study_pairs_order ON study_pairs(study_id, seq, pair_id);
CREATE INDEX idx_study_pairs_repeat ON study_pairs(repeat_of_pair)
    WHERE repeat_of_pair IS NOT NULL;

-- Which planned row a trial came from.
--
-- NULL for every sampler-drawn trial, which is every trial that existed before
-- this migration. Without it a judgment cannot be joined back to the
-- disagreement it was chosen to settle: `(source_hash, a_encoding_id,
-- b_encoding_id)` looks like it would be enough, but counterbalancing swaps the
-- slots and a planned repeat is a second row over the identical encodings, so
-- that triple is neither stable nor unique.
ALTER TABLE trials ADD COLUMN study_pair_id TEXT REFERENCES study_pairs(pair_id);
CREATE INDEX idx_trials_study_pair ON trials(study_pair_id)
    WHERE study_pair_id IS NOT NULL;
