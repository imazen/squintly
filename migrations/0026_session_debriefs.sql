-- What an observer says about a stretch of work they already did.
--
-- # Why this is not keyed on `sessions`
--
-- A `sessions` row is created when the app boots and is never closed by most
-- people — they answer some trials and shut the tab. So a session's wall-clock
-- span says almost nothing about a sitting, and "how was your session?" keyed
-- on it would be asking about a container the observer never experienced as a
-- unit.
--
-- What they DID experience is a bout: a contiguous run of answers with no long
-- gap. So a debrief is stored against a TIME RANGE for an observer, computed
-- from `responses.responded_at`, and joins back to whatever responses fall
-- inside it. The same arithmetic the leaderboard's `active_seconds` already
-- uses, with a longer gap — see `debrief.rs::BOUT_GAP_MS`.
--
-- # A skip is recorded, not absent
--
-- `skipped` exists so a declined prompt is a fact rather than a gap. Without it
-- the only way to know somebody had been asked would be the absence of a row,
-- which is indistinguishable from never having been asked — and a returning
-- observer would then face the same question about the same evening forever.
-- Nagging somebody at two participants is expensive.
--
-- # Reasons are a fixed list, stored as a sorted comma-joined set
--
-- Not free text, and not a numeric self-rating. Each reason names a specific
-- CIRCUMSTANCE ("I didn't realise I could answer can't-tell"), which is a fact
-- the observer actually knows and which maps to a concrete analysis. A global
-- "rate your attention 1-5" would be an outcome self-judgement: poorly
-- calibrated, and it invites saying whatever seems safest. See
-- docs/OBSERVER-FEEDBACK.md §7.
--
-- `note` is optional free text on top, for the thing the list did not cover.
CREATE TABLE session_debriefs (
    id            TEXT    PRIMARY KEY,
    observer_id   TEXT    NOT NULL,
    -- The bout this is about, as a closed range over `responses.responded_at`.
    bout_start_ms INTEGER NOT NULL,
    bout_end_ms   INTEGER NOT NULL,
    -- How many responses fell inside it when the debrief was taken. Stored
    -- rather than recomputed so the report says what the observer was actually
    -- shown, even if the bout arithmetic is later tuned.
    responses     INTEGER NOT NULL,
    -- Sorted, comma-joined reason keys. Empty string = none selected.
    reasons       TEXT    NOT NULL DEFAULT '',
    note          TEXT,
    -- 1 when the observer dismissed the prompt without answering.
    skipped       INTEGER NOT NULL DEFAULT 0,
    -- 'return' (asked on their next visit) or 'end' (they clicked End session).
    -- Recorded because the two are different measurement conditions: one is
    -- recall of an earlier evening, the other is immediate.
    prompted_at   TEXT    NOT NULL,
    submitted_at  INTEGER NOT NULL
) STRICT;

-- `pending` asks "does this observer have a bout with no debrief row", which
-- without this is a scan per page load.
CREATE INDEX idx_session_debriefs_observer
    ON session_debriefs (observer_id, bout_end_ms);
