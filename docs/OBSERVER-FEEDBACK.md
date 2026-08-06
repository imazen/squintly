# Steering observers toward usable data, when you cannot exclude any of them

Design note. Written 2026-08-05, when the expected participant count is **two**.

## 1. Why the usual answer does not apply

Every screening instrument squintly implements assumes you can afford to drop
somebody:

| screen | needs | at N=2 |
|---|---|---|
| §4.4 peer-mean correlation (`exclusion.rs`) | other observers rating the same images | **useless** — one peer, and "outlier vs the mean of one other person" is not a measurement |
| §4.2.1 BT.500 kurtosis-2 | per-stimulus scores | undefined on pairwise data anyway (see `crowd_bt.rs` header) |
| Crowd-BT η (`crowd_bt.rs`) | enough comparisons to separate observer noise from item difficulty | **weakly identified** — with two observers, if both drift the same way the fit cannot tell that from the items genuinely being that way |

And even where a screen *works*, acting on it costs 50% of the dataset. So the
posture has to change: the instruments stay (they are recorded, never enforced —
see `ExclusionPolicy::enabled`), but they become **diagnostics for the operator**,
not gates on the participant.

The thing that actually improves the data at N=2 is making each observer's
answers better while they are giving them.

## 2. What the literature says to do instead

`zenpapers/docs/iqa-methods/reference-book/ch10_human_eval_collection.md` §10.2.3
is explicit, and it is the pattern we should follow:

> The training round is **not** scored against the participant; it teaches the
> response mapping and acclimates the eye to the stimulus type.

KonJND-1k's MTurk study is the worked example: 10 training trials with
**immediate range feedback**, workers can only proceed once in range, and
failures do not reject anybody. The chapter calls this *training-as-tuning* —
"a teach-by-feedback loop, not a pass/fail gate."

Its recommended pattern for a new study:

- 4–6 training trials with **large, unambiguous quality gaps**
- **Immediate visual feedback** ("✓ this is the more distorted side")
- **No scoring; failures do not reject**
- Cannot reach the main session without completing them
- A skip cookie for someone who has done it before

That is the whole answer to "we only have two participants": you do not screen
them, you calibrate them.

## 3. The line that must not be crossed

Feedback is only safe when it cannot teach the observer the answer to a *scored*
trial. Two specific prohibitions, both of which would destroy an instrument we
depend on:

**Never reveal which trials are repeats.** `Study::p_repeat` re-serves pairs the
observer already answered, and their agreement with themselves is the noise
ceiling — the number that licenses any statement about the metric at all (see
CLAUDE.md). An observer who knows a pair is a repeat will try to *remember* their
previous answer instead of judging it again, which measures memory and reports it
as reliability. The ceiling would inflate toward 1.0 and every ρ/ceiling
conclusion built on it would be wrong.

**Never give correctness feedback on scored trials.** Golden pairs have a known
answer, so telling somebody they got one wrong is possible — and doing it would
(a) identify which trials are checks, defeating them, and (b) train the observer
toward whatever produced the "correct" answer, which is the metric. A study that
trains its observers on the metric it is trying to evaluate has measured nothing.

So: **feedback on process during the session, feedback on outcome only in
training and only in aggregate afterwards.**

## 4. What we can safely say, and when

### 4.1 Training (before the first session) — outcome feedback allowed

Unambiguous pairs, ✓/✗ shown, nothing recorded against the observer. This is the
one place correctness feedback is safe, because the gaps are large enough that
the answer is not a judgement call and there is nothing to bias.

Status: **not built**. This is the highest-value unbuilt item in this note.

### 4.2 During a session — process feedback only

All of these are derivable from data we already record, and none of them mentions
whether an answer was right:

| signal | recorded as | what it means | what to say |
|---|---|---|---|
| answered very fast with no view switches | `dwell_ms`, `switch_count` | almost certainly not compared | "Flick between A and B before answering — that is where the difference shows" |
| long held time, no tie used | `ms_on_a/b/ref`, `choice` | grinding at threshold | already handled by the can't-tell nudge (`cant_tell_hint_ms`) |
| never magnified on a large source | `zoom_factor`, `pan_count` | judging at a size where the artefact is invisible | "Pinch to magnify — some differences only show at 1:1" |
| long gap since last answer | `responded_at` | came back after a break; criterion may have drifted | "Welcome back — remember, closer to the original, not nicer" |

The wording rule: describe **what to do**, never **how they did**. "Flick between
them before answering" is process. "You are answering too fast to be accurate" is
a judgement, and a judgement invites the observer to perform rather than report.

Status: **built** — `web/src/nudge.ts`, wired into `trial.ts::submit`. It fires
only on the combination (`dwell < FAST_ANSWER_MS` **and**
`switch_count <= MIN_COMPARE_SWITCHES`), because either alone is innocent: a
slow answer with one look each is deliberation, and a fast answer after eight
flicks is an easy pair. It yields to a milestone, waits `NUDGE_COOLDOWN`
answers between firings, and stops after `MAX_NUDGES_PER_SESSION` — someone who
has heard it three times has made a choice, and a fourth costs goodwill we
cannot spare at two participants.

`responses.process_nudges_seen` (migration 0024) records how many the observer
had seen *before* each answer. Not because the nudge could bias `choice` — it
is answer-neutral, unlike the can't-tell hint — but because it moves the effort
columns. Somebody just told to flick will flick more on every later trial, and
`switch_count` / `dwell_ms` are what difficulty is read from; without the column
an analyst sees that step and attributes it to the stimuli. NULL means not
recorded, which is deliberately not the same as a recorded zero.

The magnification and welcome-back nudges in the table above are **not built**.

### 4.3 After a session — aggregate outcome feedback is safe

At the end of a session, self-agreement over the whole session can be shown
without naming a single trial. `680`-style aggregates cannot be reverse-engineered
into "that pair was a repeat", so the ceiling survives.

Status: **built** — the end-of-session screen shows self-agreement beside volume.

## 5. The 20-comparison mark

`crowd_bt::MIN_OBS_FOR_ETA` is 20. Below it an observer's reliability cannot be
estimated at all, so their answers are stored but cannot be weighted or checked.
The lap bar and the milestone notices at 2/10/15/20 exist to make that boundary
visible, because it is the one threshold where "more answers" changes the *kind*
of data we have rather than just the amount.

At N=2 this matters more, not less: with two observers there is no pooling to
hide behind, so each one crossing 20 is half the study becoming analysable.

## 6. Open question — is η identifiable at N=2 at all?

Not established. Crowd-BT estimates η jointly with the latent scores; with two
observers the model can explain disagreement either as "observer A is noisy" or
as "these items are genuinely close", and nothing in the data separates those
without a third opinion or an anchor.

**What would settle it:** the golden pairs. A golden has a known answer
independent of the observers, so it pins the scale locally and gives η something
external to be measured against. Whether `p_golden_pair` (currently 0.083)
supplies enough of them for a stable η at N=2 is an empirical question that
should be checked against real data before any η is reported, not assumed.

Until it is checked, treat η at N=2 as a diagnostic to look at, not a number to
quote.
