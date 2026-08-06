-- Objective metric scores per encoding, so the human ranking has something to
-- be correlated against.
--
-- The default study exists to ask whether SSIMULACRA2 ranks non-photo encodings
-- as well as it ranks photographs (imazen/squintly#4). Answering that needs a
-- score per encoding, and until now squintly held none at all: `EncodingMeta`
-- carries codec/quality/effort/bytes, the corpus builder computes no metric,
-- and nothing in the schema stored one. Every number the app could produce was
-- one side of a correlation.
--
-- # Long, not wide
--
-- One row per (encoding, metric) rather than a column per metric. zenmetrics
-- emits fourteen columns today across six families, and several of them carry
-- an implementation version IN THE NAME — `cvvdp_imazen_v0_0_1`,
-- `iwssim_cpu_imazen_v0_1_2`, `ssim2_imazen_iir_v3`. Those namespaces are
-- deliberately disjoint so sidecars from different backends do not collide on
-- join. A wide table would need a migration per metric AND per version bump,
-- which is a migration every time a GPU kernel is retuned. Long costs one join
-- and accepts anything.
--
-- # Provenance is per row, because a metric can be re-ingested
--
-- `source` names the file the value came from and `ingested_at` when. Two
-- ingests of the same metric for the same encoding is an UPDATE, not a second
-- row — the primary key enforces that — so a corrected score replaces a wrong
-- one and the provenance columns say which run it came from.
--
-- # Direction is NOT stored here
--
-- Whether higher is better belongs to the metric, not to the measurement, so it
-- lives in code (`src/metrics.rs::direction_of`) where it can be reasoned about
-- and tested. Storing it per row would let two rows disagree about the same
-- metric, and the one that got it backwards would silently invert a rank
-- correlation — an error that looks exactly like the finding the study is
-- trying to make.
CREATE TABLE encoding_metrics (
    encoding_id TEXT    NOT NULL,
    metric      TEXT    NOT NULL,
    value       REAL    NOT NULL,
    -- Where it came from: a filename, a sweep id, whatever the operator passed.
    -- Nullable because an ingest without one is still better than no scores.
    source      TEXT,
    ingested_at INTEGER NOT NULL,
    PRIMARY KEY (encoding_id, metric)
) STRICT;

-- The report reads "every encoding's score for ONE metric" to build a ranking,
-- which without this is a full scan of a table that grows as
-- encodings × metrics.
CREATE INDEX idx_encoding_metrics_metric ON encoding_metrics (metric);
