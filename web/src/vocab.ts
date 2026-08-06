// The words the interface uses, in one place.
//
// Six terms were in circulation for four concepts — trial, comparison, rating,
// response, observer, reviewer — mixed freely between the front page, the trial
// chrome, the menu, the end screen and the board. "Trial 7" above the picture,
// "21 comparisons" in a milestone, "your ratings count" in the debrief and
// "reviewers" on the board all described overlapping things, so somebody
// reading two screens could not tell whether they were the same count.
//
// The split kept here is between the INTERNAL vocabulary and the USER-FACING
// one. Internally `trial`, `response` and `observer` are correct and stay —
// they are the table names, the column names, and the words the methodology
// literature uses. This module is the translation layer, so renaming a screen's
// wording never means renaming a column.
//
// | concept                        | internal   | user-facing |
// |--------------------------------|------------|-------------|
// | the person judging             | observer   | reviewer    |
// | one pair judgement             | trial/pair | comparison  |
// | one single-stimulus judgement  | trial      | rating      |
// | either, in aggregate           | response   | rating      |
// | one sitting                    | bout       | session     |
//
// "Rating" doubles as the aggregate on purpose: it is the word people already
// use for "I rated some images", and a count that mixes pairs and singles has
// no better name. Where a count is pairs ONLY — the 20-comparison reliability
// mark, the board's headline — say **comparison**, because that distinction is
// load-bearing (a 4-tier rating does not feed Crowd-BT η).

export type TrialKind = 'single' | 'pair';

/// What to call one judgement of this kind, to the person making it.
export function unitNoun(kind: TrialKind, plural = false): string {
  const word = kind === 'pair' ? 'comparison' : 'rating';
  return plural ? `${word}s` : word;
}

/// The counter above the stimulus. Just the ordinal and the noun — "Trial 7"
/// was jargon for the one screen where space is scarcest, and it named the
/// internal concept rather than what the person is doing.
export function trialCounterLabel(kind: TrialKind, n: number): string {
  return `${unitNoun(kind)} ${n}`;
}

/// The person, to themselves and to each other. Never "observer" on screen:
/// it is the methodology's word, and it describes somebody being watched
/// rather than somebody doing the work.
export const PERSON = 'reviewer';
export const PEOPLE = 'reviewers';

/// One short sentence explaining what is being asked, for anywhere somebody
/// might arrive without having read the instructions.
export const TASK_BLURB =
  'You are shown two compressed versions of the same picture and asked which is ' +
  'closer to the original. There is no right answer to know in advance — the ' +
  'answer is what you see.';
