-- Let an observer correct a misclick, without letting them launder their data.
--
-- There was no way back. One stray tap — and the trial screen is now driven by
-- thumb-sized buttons, held mouse buttons and single keystrokes, so stray taps
-- are *more* likely than they were — recorded a judgement the observer knew was
-- wrong, permanently. A known-wrong response is worse than a missing one: it
-- enters the fit as a real opinion.
--
-- The correction is RECORDED, not applied destructively. Deleting the first
-- answer would let someone retroactively tidy their own data, and "they
-- answered A, then changed to B" is a fact about the session that analysis may
-- legitimately want — a high revision rate is itself a signal about attention
-- or about the pair being hard.
--
--   choice           the answer that counts (the latest one)
--   original_choice  the FIRST answer, kept once a revision happens; NULL means
--                    never revised
--   revised_at       when it was last changed
--   revision_count   how many times
--
-- Attention checks deliberately score `COALESCE(original_choice, choice)` — the
-- first answer. Otherwise undo would defeat the honeypot: fail it, notice, take
-- it back. See `grading.rs`.
ALTER TABLE responses ADD COLUMN original_choice TEXT;
ALTER TABLE responses ADD COLUMN revised_at INTEGER;
ALTER TABLE responses ADD COLUMN revision_count INTEGER NOT NULL DEFAULT 0;
