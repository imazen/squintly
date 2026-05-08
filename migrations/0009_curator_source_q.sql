-- 0009_curator_source_q.sql — per-candidate detected JPEG quality.
--
-- Stores the libjpeg-style quality estimate (1-100) computed from the first
-- DQT marker by `src/jpeg_q.rs::estimate_quality`. NULL means "not a JPEG"
-- or "couldn't parse." When set, `suggest()` uses it to truncate the
-- size-chip set per the spec §6.1 auto-downscale rule.

ALTER TABLE curator_candidates ADD COLUMN source_q_detected REAL;
