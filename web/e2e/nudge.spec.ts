// Pure-logic tests for the process-nudge budget.
//
// No browser: `considerNudge` is a decision over recorded effort, and the
// interesting cases are the ones it must NOT fire on. Driving those through a
// real session would mean staging a careless answer eight times to watch
// nothing happen, which tests the harness rather than the rule.

import { test, expect } from '@playwright/test';
import {
  considerNudge,
  newNudgeState,
  FAST_ANSWER_MS,
  MIN_COMPARE_SWITCHES,
  NUDGE_COOLDOWN,
  MAX_NUDGES_PER_SESSION,
  MIN_ANSWERS_BEFORE_NUDGE,
  type TrialEffort,
} from '../src/nudge';

const careless: TrialEffort = {
  kind: 'pair',
  dwellMs: FAST_ANSWER_MS - 500,
  switchCount: MIN_COMPARE_SWITCHES,
};
const careful: TrialEffort = { kind: 'pair', dwellMs: 14_000, switchCount: 6 };

/// Advance past the settling-in answers so a test can reach the rule it means
/// to check. Uses careful answers so it cannot itself consume the budget.
function settle(s: ReturnType<typeof newNudgeState>) {
  for (let i = 0; i < MIN_ANSWERS_BEFORE_NUDGE; i++) considerNudge(careful, s);
  return s;
}

test('nudges a fast answer with no back-and-forth', () => {
  const s = settle(newNudgeState());
  const n = considerNudge(careless, s);
  expect(n).not.toBeNull();
  expect(n!.text).toContain('A and B');
  expect(s.shown).toBe(1);
});

test('says nothing about a considered answer', () => {
  const s = newNudgeState();
  expect(considerNudge(careful, s)).toBeNull();
  expect(s.shown).toBe(0);
});

test('a slow answer with few switches is left alone', () => {
  // Somebody who looked once at each and thought about it for twenty seconds
  // did the task. Only the combination of fast AND unswitched is the signal.
  const s = settle(newNudgeState());
  expect(considerNudge({ kind: 'pair', dwellMs: 20_000, switchCount: 1 }, s)).toBeNull();
});

test('a fast answer after real comparison is left alone', () => {
  const s = settle(newNudgeState());
  expect(considerNudge({ kind: 'pair', dwellMs: 1_000, switchCount: 8 }, s)).toBeNull();
});

test('holds off for the cooldown, then fires again', () => {
  const s = settle(newNudgeState());
  expect(considerNudge(careless, s)).not.toBeNull();
  // Every answer in between is careless and every one of them stays quiet:
  // repeating the advice before it can have been acted on is nagging.
  for (let i = 1; i < NUDGE_COOLDOWN; i++) {
    expect(considerNudge(careless, s), `answer ${i} inside the cooldown`).toBeNull();
  }
  expect(considerNudge(careless, s)).not.toBeNull();
  expect(s.shown).toBe(2);
});

test('stops after the session cap even if the pattern continues', () => {
  const s = settle(newNudgeState());
  let fired = 0;
  for (let i = 0; i < NUDGE_COOLDOWN * (MAX_NUDGES_PER_SESSION + 3); i++) {
    if (considerNudge(careless, s)) fired++;
  }
  expect(fired).toBe(MAX_NUDGES_PER_SESSION);
});

test('a suppressed nudge is skipped, not queued', () => {
  // A milestone owns the notice layer for those two seconds. The nudge does not
  // wait its turn and land on the next answer — that would put a correction
  // immediately after a reward, which reads as taking the reward back.
  const s = settle(newNudgeState());
  expect(considerNudge(careless, s, true)).toBeNull();
  expect(s.shown).toBe(0);
  expect(considerNudge(careless, s)).not.toBeNull();
});

test('phrases a single-stimulus trial as the original, not B', () => {
  // Under `hold` a single-stimulus trial rests on the reference and there is no
  // B at all; naming one would point at a control that is not on screen.
  const s = settle(newNudgeState());
  const n = considerNudge({ ...careless, kind: 'single' }, s);
  expect(n!.text).toContain('original');
  expect(n!.text).not.toContain('A and B');
});

test('a missing dwell is not treated as instant', () => {
  // `dwell_ms` is 0 when the judged image never painted, which is a decode
  // problem rather than a careless observer. Nudging there would blame somebody
  // for the network.
  const s = settle(newNudgeState());
  expect(considerNudge({ kind: 'pair', dwellMs: 0, switchCount: 0 }, s)).toBeNull();
});

test('says nothing on the first answers of a session', () => {
  // Somebody who has just read the instructions and is finding the controls has
  // not formed a habit yet, and "you already got it wrong" is a cost we cannot
  // pay at two participants. One fast answer is noise; a few is a pattern.
  const s = newNudgeState();
  for (let i = 0; i < MIN_ANSWERS_BEFORE_NUDGE; i++) {
    expect(considerNudge(careless, s), `answer ${i + 1}`).toBeNull();
  }
  expect(considerNudge(careless, s)).not.toBeNull();
});
