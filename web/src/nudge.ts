// Process nudges: what to say to an observer who is answering without looking.
//
// # Why this exists
//
// The screens in `src/exclusion.rs` all measure an observer against other
// observers, and all of them work by spending data — you find the careless one
// and you drop them. The live studies expect **two** participants, so there is
// nobody to be an outlier against and dropping either costs half the dataset.
//
// The literature's answer for a small panel is to calibrate rather than screen
// (zenpapers ch10 §10.2.3, training-as-tuning). This is the in-session half of
// that: when the recorded effort on a trial says nobody actually compared the
// two images, say so — gently, rarely, and in terms of what to do next.
//
// # The line this must not cross
//
// Feedback during a scored session is about PROCESS, never OUTCOME. See
// `docs/OBSERVER-FEEDBACK.md`. Concretely, nothing here may:
//
//   - reveal that a trial was a repeat (`Study::p_repeat`) — an observer who
//     knows would recall their previous answer instead of judging again, which
//     measures memory and reports it as the noise ceiling;
//   - say whether an answer was right, including on a golden pair — that both
//     identifies the attention checks and trains the observer toward the metric
//     under test.
//
// So every message below is answer-neutral: it says how to look, and it would
// read identically whichever side the observer picked. That is also why these
// do not need to be recorded as a bias on `choice` the way the can't-tell hint
// does — but they DO change `switch_count` and `dwell_ms` on later trials, so
// `process_nudges_seen` records how many the observer had seen before each
// response (migration 0024) and effort can be read before-and-after.

/// Which view an answer was given from — enough to phrase the nudge for the
/// trial that actually happened.
export type NudgeTrialKind = 'pair' | 'single';

/// The recorded effort for one answer, in the units the response row carries.
export interface TrialEffort {
  kind: NudgeTrialKind;
  /// Time from the judged image painting to the answer. Not from trial start:
  /// waiting for a decode is not deliberation.
  dwellMs: number;
  /// How many times the observer changed which image was on screen.
  switchCount: number;
}

/// Under this, an answer is too quick to have involved a comparison.
///
/// Deliberately well below the observed median (~14s live, tail to 163s —
/// CLAUDE.md) rather than near it: this is meant to catch the clearly-careless,
/// not to second-guess somebody who found an easy pair easy. A threshold that
/// fires on ordinary fast answers would train observers to perform
/// deliberation, which is worse data than the problem it set out to fix.
export const FAST_ANSWER_MS = 3500;

/// At or below this many view changes, the observer did not go back and forth.
///
/// Satisfying the seen-both gate already forces one change per arm, so this is
/// "looked once at each and answered" — no comparison, just two impressions.
export const MIN_COMPARE_SWITCHES = 2;

/// Answers that must land before the first nudge can fire.
///
/// One fast answer is noise; a habit takes a few. And the first answer of a
/// session is when somebody is still working out where the controls are, having
/// just read the instructions — correcting them there reads as "you already got
/// it wrong", which at two participants is a cost we cannot pay. It also keeps
/// the first nudge clear of the milestone at 2, so the earliest thing an
/// observer sees is the encouraging one.
export const MIN_ANSWERS_BEFORE_NUDGE = 3;

/// Answers that must pass between nudges.
///
/// Back-to-back nudges read as nagging, and a nag invites performance rather
/// than reporting. Spacing them also means the observer has had a real chance
/// to act on the last one before being told again.
export const NUDGE_COOLDOWN = 8;

/// Most nudges in one session.
///
/// Someone who has seen this three times and is still answering the same way
/// has made a choice; saying it a fourth time gains nothing and costs goodwill
/// we cannot spare at two participants.
export const MAX_NUDGES_PER_SESSION = 3;

/// Mutable per-session nudge bookkeeping. Lives in memory only — a nudge budget
/// that survived a reload would be indistinguishable from one that never fired.
export interface NudgeState {
  shown: number;
  answers: number;
  answersSinceLast: number;
}

/// The cooldown starts already elapsed, so `MIN_ANSWERS_BEFORE_NUDGE` alone
/// decides when the first one may fire rather than the two gates stacking into
/// a wait long enough that the advice arrives after the habit has set.
export function newNudgeState(): NudgeState {
  return { shown: 0, answers: 0, answersSinceLast: NUDGE_COOLDOWN };
}

/// What to say. Both are instructions, and neither depends on the answer given.
const TEXT: Record<NudgeTrialKind, string> = {
  pair: 'Flick between A and B a few times — differences show in the change, not the picture.',
  single: 'Flick to the original and back — differences show in the change, not the picture.',
};

export interface Nudge {
  text: string;
  /// `info`, never `warn`: this is a suggestion about technique, and colouring
  /// it as a warning turns it into a verdict on the observer.
  tone: 'info';
}

/**
 * Decide whether to nudge after an answer, and mutate `state` accordingly.
 *
 * Call once per committed response. Returns `null` far more often than not —
 * the budget above is the point, not an afterthought.
 *
 * `suppress` is for the caller that already has something to show (a
 * milestone): a reward and a correction competing for the same two seconds
 * means neither is read, and interrupting an earned milestone to say "look
 * harder" turns it into a scolding. The cooldown still advances, so a
 * suppressed nudge is skipped rather than queued.
 */
export function considerNudge(
  effort: TrialEffort,
  state: NudgeState,
  suppress = false,
): Nudge | null {
  state.answers += 1;
  state.answersSinceLast += 1;
  if (suppress) return null;
  if (state.answers <= MIN_ANSWERS_BEFORE_NUDGE) return null;
  if (state.shown >= MAX_NUDGES_PER_SESSION) return null;
  if (state.answersSinceLast < NUDGE_COOLDOWN) return null;

  const careless =
    effort.dwellMs > 0 &&
    effort.dwellMs < FAST_ANSWER_MS &&
    effort.switchCount <= MIN_COMPARE_SWITCHES;
  if (!careless) return null;

  state.shown += 1;
  state.answersSinceLast = 0;
  return { text: TEXT[effort.kind], tone: 'info' };
}
