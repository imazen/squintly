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

## 7. When an observer says they made a mistake

Real need, and it has a right answer that is not "delete".

### 7.1 Deletion is never the mechanism

Two independent reasons, either of which is sufficient:

**Self-selected deletion is missing-not-at-random.** An observer removing "the
ones I got wrong" is removing trials on a criterion correlated with the thing
being measured. MNAR is the one kind of missingness that cannot be corrected
for afterwards, because the deletion rule depends on the value that is now gone.
Random dropout costs power; this costs validity.

**It destroys the noise ceiling.** Self-agreement is computed from what somebody
actually did. Delete the trials where they were inconsistent and the ceiling
rises toward 1.0 — and every ρ/ceiling built on it is then divided by a number
that describes a filtered person rather than a real one.

Also worth naming: an observer cannot know which individual trials were bad.
They know they were tired; they do not know which of the forty pairs that
affected. Per-trial marking is guessing presented as data.

### 7.2 What to offer instead, in three layers

**A misclick on the current trial — undo.** Already built (migration 0020).
Bounded to the latest trial on purpose, so it cannot walk back through a run,
and the first answer survives in `original_choice` because "answered A, then
changed to B" is itself signal about difficulty.

**A compromised sitting — a session-level self-report, at the debrief.** Scoped
to the *session*, never a range of trials: session-level statements are about
CONDITIONS ("I was in sunlight", "I misunderstood the task at first"), which
squintly already treats as first-class data, and conditions are something an
observer genuinely knows. Recorded as a disposition, the same shape as
`observer_dispositions` — the analyst gets a filter, and can run with and
without. Never a delete.

Offer it at session END, not mid-session. Mid-session it becomes an undo by
another name, and it is also a demand characteristic: being asked "was that
alright?" implies you are doing badly.

**A reason from a fixed list**, because reasons map to different analyses:

| reason | what it licenses |
|---|---|
| I misunderstood the task at first | truncating from the start — a learning effect is real and modelable |
| Bad viewing conditions | corroborates what `conditions` already recorded |
| Someone else was using my device | an IDENTITY problem, not a quality one — different fix |
| I was rushing or distracted | check it against `switch_count` / `dwell_ms` before acting |

### 7.3 The elegant part: a self-report is checkable

We already record `switch_count`, `dwell_ms`, `ms_on_*`, `zoom_factor`,
`cant_tell_hint_ms` and the golden pass rate. So "I rushed the last twenty"
either corroborates the effort columns or it does not, and a self-report that
disagrees with the instruments is itself informative. The flag never has to be
trusted blindly, which is what makes accepting it safe.

### 7.4 Wording

Never "your data was excluded" — it is not, and saying so would be false given
`ExclusionPolicy::enabled` is off. "Anything we should know about this session?"
is the neutral form: it asks about circumstances, not performance, which is the
same process-versus-outcome line as §3.

### 7.5 Should observers self-rate their work?

**No — not a rating. Yes — specific circumstances.**

A "rate your attention 1–5" is an *outcome* self-judgement, and it fails on both
counts that matter here. People are poorly calibrated about their own
performance (the ones who did worst are least able to tell), and the question
invites answering whatever seems safest — especially from somebody who suspects
a low score might get their work discarded. It also has no analysis attached:
what would you actually *do* with a 3?

A checkbox saying "I didn't realise I could answer can't-tell" is different in
kind. It is a **fact about what happened**, which the observer genuinely knows,
and it maps to something concrete: their tie rate is artificially zero and their
forced choices on threshold pairs were guesses, so you can condition on it.

The rule to apply to any new option: *would the observer be reporting, or
grading?* Reporting is safe; grading is not. `src/debrief.rs` enforces this with
a test that fails any label containing "rate", "how well", "score" or "accurate",
and another that fails any reason without an `analysis` note — because a
checkbox nobody knows how to read is friction with no payoff.

### 7.6 When to ask, given nobody signs off

Almost nobody clicks "End session". So the debrief is keyed on a **bout** — a
contiguous run of answers with no gap longer than `BOUT_GAP_MS` (45 min) —
computed from `responses.responded_at`, which exists whether or not anyone
signed off.

- **Primary moment: the next visit.** "Last time you did 21 comparisons —
  anything we should know?" This is also the only point where the question
  cannot interrupt a sitting.
- **Better moment when it happens: sign-off.** If somebody clicks End session,
  the same prompt is raised there, where it is immediate rather than recalled.
  The instructions ask people to sign off for exactly this reason — it is a
  strictly better measurement, so it is worth asking for even though most people
  will not do it.
- **Never mid-session.** `pending(include_current: false)` on a return visit.

Not asked at all: bouts under `MIN_BOUT_RESPONSES` (nobody has an impression of
three answers), and anything older than `MAX_BOUT_AGE_MS` (14 days) — past that
a self-report is reconstruction, and a confident wrong answer is worse than none.

A skip is recorded, not absent. Otherwise the only evidence of having asked
would be a missing row, which is indistinguishable from never having asked, and
the observer would meet the same question about the same evening forever.

Status: **built** — `src/debrief.rs`, `web/src/debrief.ts`, migration 0026.


## 8. First live reading, 2026-08-06 — and why ρ/ceiling > 1 is a warning

With ssim2 ingested for all 4032 encodings, the disposition report produced its
first complete reading:

| | |
|---|---|
| comparisons | 222 (208 distinct pairs, 5 observers) |
| noise ceiling | **0.90** (9 of 10 repeated pairs answered the same way) |
| golden pairs passed | 100% of 13 |
| ssim2 ρ | **0.988** over 84 scored comparisons |
| ρ / ceiling | **1.10** |

**A ρ/ceiling above 1 is not a metric beating humans.** It means the metric
agreed with the observers more often than the observers agreed with themselves,
which can only happen when the pairs being served are easy enough that both get
them right — while the repeats that *did* disagree were the genuinely hard ones.
It is a statement about the stimuli, not about ssim2.

That corroborates the calibration in `sampling::TRIVIAL_SSIM2_GAP`: human
agreement reaches 100% by a 5-point ssim2 gap, and the live ladder's median
adjacent-rung gaps are 5.7–17.3. The instrument is currently posing questions
whose answers are not in doubt.

So the headline question is **not yet answerable**, and for a reason that has
nothing to do with sample size: a corpus of easy pairs cannot discriminate
between a metric that tracks human judgement and one that merely tracks
"obviously worse". The fix is a denser quality ladder (imazen/squintly#8 rows
5–6), not more observers on this one.

Read ρ/ceiling > 1 as a prompt to make the corpus harder, never as a result.
