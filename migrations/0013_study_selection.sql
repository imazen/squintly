-- Named studies, selected at runtime (src/studies.rs).
--
-- One deployment hosts several studies whose trial streams differ because what
-- they measure differs: the pre-registered crowd study interleaves
-- single-stimulus ratings with pairwise, while a rank-agreement study
-- (SSIMULACRA2 as the non-photo oracle, imazen/squintly#4) is forced-choice
-- only. Their responses must never be pooled — an ACR rating and a 2AFC
-- judgement are different quantities on different scales.
--
-- Without this column that separation is impossible after the fact: two
-- studies' rows are indistinguishable in `responses`. Defaulting to 'main'
-- keeps every pre-existing session attributed to the study it was actually
-- collected under.

ALTER TABLE sessions ADD COLUMN study_id TEXT NOT NULL DEFAULT 'main';

CREATE INDEX IF NOT EXISTS idx_sessions_study ON sessions(study_id);
