-- How long each variant was actually looked at, how often the observer swapped,
-- and the hooks for study controls.
--
-- `reveal_count` / `reveal_ms_total` only ever measured time on the REFERENCE,
-- and under `hold`/`buttons` the reference is the resting view — so that column
-- is dominated by "not currently pressing anything" and says nothing about
-- effort. The informative quantity is the inverse: time spent holding a variant
-- up against the original, and how many times the observer went back and forth
-- before committing.
--
-- That is a difficulty signal. A pair the observer flips between six times over
-- twenty seconds is near their discrimination threshold; one answered in two
-- seconds with a single look is not. Both are correct answers and BT treats
-- them identically, but only the first tells you the pair was hard — which is
-- exactly what a rank-agreement study needs, because a metric disagreeing on
-- pairs humans find hard is a different finding from one disagreeing on pairs
-- humans find easy.
--
-- Stored RAW, per view, in milliseconds. Not normalised at write time: the
-- interesting form is relative to that observer's other trials in the same
-- session, and the session is not finished when this row is written. Baking in
-- a z-score against a partial session would be unrecoverable; analysis can
-- always normalise afterwards, and cannot un-normalise.
ALTER TABLE responses ADD COLUMN switch_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE responses ADD COLUMN ms_on_a INTEGER NOT NULL DEFAULT 0;
ALTER TABLE responses ADD COLUMN ms_on_b INTEGER NOT NULL DEFAULT 0;
ALTER TABLE responses ADD COLUMN ms_on_ref INTEGER NOT NULL DEFAULT 0;

-- Test–retest: this trial re-serves a pair the observer already answered.
--
-- The control the rank-agreement study was missing. Human-vs-ssim2 SROCC is
-- uninterpretable on its own — if an observer agrees with *themselves* only 80%
-- of the time on repeated pairs, then ssim2 cannot exceed roughly that, and
-- "ssim2 scored 0.7" means something completely different depending on whether
-- the ceiling is 0.95 or 0.72. Repeats measure that ceiling directly.
--
-- NULL for an ordinary trial.
ALTER TABLE trials ADD COLUMN repeat_of_trial_id TEXT REFERENCES trials(id);
CREATE INDEX idx_trials_repeat_of ON trials(repeat_of_trial_id)
    WHERE repeat_of_trial_id IS NOT NULL;
