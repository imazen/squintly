-- How the observer drove the trial UI.
--
-- Not cosmetic — it changes what the other columns mean. In `tap` mode the
-- encoding is on screen by default and the observer presses and holds to see
-- the reference, so `reveal_ms_total` is time spent *away* from the stimulus
-- being judged. In `hold` mode the resting state is inverted: the reference is
-- shown and holding a mouse button flicks to A or B, so the same column
-- measures the default state and is naturally large.
--
-- Both are legitimate ways to compare, and both record "time the reference was
-- on screen" — but pooling them without knowing which is which would put two
-- different quantities in one column. Same reason `study_id` exists.
--
-- A third mode, 'buttons', shares `hold`'s inverted resting state but picks the
-- side with the mouse button instead of the half of the frame pressed.
--
-- 'tap' is the historical behaviour, so existing rows are correctly labelled by
-- the default.
ALTER TABLE responses ADD COLUMN input_mode TEXT NOT NULL DEFAULT 'tap';

-- Whether a keyboard was used at any point in the trial. Keyboard-driven
-- responses have a different reaction-time floor than pointer-driven ones (no
-- travel-to-target time), which `grading.rs` thresholds against.
ALTER TABLE responses ADD COLUMN keyboard_used INTEGER NOT NULL DEFAULT 0;

-- Milliseconds from trial render to the judged image being decoded and painted.
-- Before variant preloading, switching A/B/original re-decoded through a single
-- <img>, so this was paid repeatedly and invisibly inside `dwell_ms`. Recording
-- it keeps that separable: a slow first paint is not deliberation.
ALTER TABLE responses ADD COLUMN ui_ready_ms INTEGER;
