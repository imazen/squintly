# Squintly — agent notes

Browser-based psychovisual data collection for zensim. See [SPEC.md](SPEC.md) for the
design and [README.md](README.md) for the elevator pitch.

## NEVER PAUSE NEAR THE END OF CONTEXT — DOC IT AND PUSH THROUGH COMPACTION

**Running low on context is not a reason to stop, hand off, or ask whether to
continue.** Compaction is the harness's job and it works; the way to survive it
is not to wind down but to make the durable record complete enough that the next
window resumes mid-stride.

When context is running short, the correct sequence is:

1. **Write everything durable down, in the RIGHT files** — CLAUDE.md invariants,
   the relevant `docs/*.md`, CHANGELOG, and `.workongoing` with the literal next
   commands. Never a CONTEXT-HANDOFF.md (see "NEVER DELAY DUE TO CONTEXT").
2. **Correct historical docs in place, with editorial notes.** A dated document
   that is now wrong is worse than no document, because it is read and believed.
   Do not rewrite history silently: mark the correction `[note YYYY-MM-DD]`,
   state what the old claim was and what the measurement says instead, and leave
   both readable. Stale dated docs are still in scope — "it says 2026-05-01 at
   the top" is not permission to leave it lying.
3. **Commit and push** so nothing depends on this window surviving.
4. **Keep working.** Then continue the assigned task — and the *implied* one:
   the next obvious step in the work, not merely the literal instruction.

Forbidden: "I'm running low on context, so I'll stop here", "let me leave this
for a fresh session", ending a turn with a status report when there is obvious
next work, or asking permission to continue something already asked for. A
status report is what you write on the way past, not instead of the next step.

This is the counterpart to the continuation discipline in the global CLAUDE.md,
sharpened for the specific failure of treating a shrinking context window as a
natural stopping point. It is not.

## Architecture in one paragraph

Single Rust binary (axum). Embeds the Vite-built TS frontend via `rust-embed`. SQLite
via `sqlx` for trial/response storage. Talks to coefficient over HTTP (default) or by
reading its SplitStore directly. Frontend is vanilla TS with the `@aspect/image-compare`
web component from `~/work/efficient-ui/`. No framework.

## Key invariants

- **Never mutate coefficient.** Squintly is a *consumer* of coefficient's image store;
  it never writes back. Aggregated TSVs are the export channel.
- **Viewing conditions are first-class data, not telemetry.** Every response row carries
  the conditions that produced it. We never aggregate them away in storage.
- **Anonymous by default; sign-in is optional and adds nothing to a trial.** The
  observer ID is a UUID in localStorage and taking part never requires an
  account. Email sign-in exists only so someone can carry that ID to a second
  device (`src/auth.rs`), and is open to any address — an allowlist there would
  lock participants out of their own data. `SQUINTLY_ADMIN_EMAILS` gates *admin*
  instead, where unset grants nobody. Client addresses are never stored: only a
  salted BLAKE3 bucket, used for the sign-in rate limit. (This invariant used to
  read "No login, no email", which stopped being true once `auth.rs` landed.)
- **Studies are selected at runtime; the trial mix belongs to the study.**
  `src/studies.rs` — `main` (65% single-stimulus / 35% pairwise, matching
  docs/STUDY.md §4.2) and `ssim2-nonphoto` (forced choice only, for
  imazen/squintly#4). Observers pick on the welcome screen; `sessions.study_id`
  records it and `responses.tsv` carries it, so the two are separable in
  analysis. CLAUDE.md previously claimed "2AFC by default" — untrue of the
  code, and it misled #4 into assuming pairwise was the native path.
  Note `p_single = 0` does NOT give a forced-choice run: the sampler falls back
  to a single when no non-trivial pair exists, and honeypots and anchors are
  themselves single-stimulus. Use the study (or `SQUINTLY_PAIRWISE_ONLY=1`).
- **The surround, not the stimulus, carries UI signal.** The letterbox is tiled
  with A / B / ORIG so the current variant is readable without looking away
  from the picture. It stays dark and strictly neutral grey on purpose: it is
  the surround of a psychovisual stimulus, so luminance changes shift local
  adaptation and tints bias colour judgements. Nothing may be painted *over*
  the stimulus (see the hint-pill note below).
- **One table, one stack: `web/src/hold-stack.ts` owns every input.** What a
  press shows, and what wins when several are held, is decided in exactly one
  place. It was four that disagreed — a pointer resolver, a release resolver, a
  keyboard `cycle()` that *toggled* instead of holding, and a space branch.
  - **The right button always means B**, in every mode. The left button is the
    mode-dependent one (`tap` peeks at the reference, `hold` is positional,
    `buttons` is plainly A). Arrows mirror the buttons: left is A, right is B.
  - **Most recent still-held press wins**; releasing falls back to the next one
    still down, not to the resting view. Keyboard and pointer share the stack.
  - **A second mouse button fires no `pointerdown`** — per Pointer Events that
    only fires on the no-buttons→some-button transition, so the second press
    arrives as `pointermove` with a changed `buttons` mask (measured: Chromium
    delivers `contextmenu buttons=3` and nothing else). Button state is
    reconciled by diffing the mask on every pointer event; anything driven off
    down/up alone cannot see the middle of the sequence.
  - Mouse holds key by *button*, touches by *pointer id* — a mouse reports every
    button on one pointer id.
- **Trial input modes**: `tap` (default; segmented control + hold-to-peek),
  `hold` (reference at rest, press the left/right half of the frame for A/B),
  `buttons` (same, but the mouse button picks the side — `pointer: fine` only).
  All three are recorded in `responses.input_mode` because they change what
  `reveal_ms_total` measures.
- **Reopening resumes into trials.** `main.ts::boot` skips onboarding when an
  observer id and profile exist, starting a *new* session (conditions are
  re-captured — the device or lighting may have changed). Curator is opt-in per
  browser via `#curator`; calibration is a welcome-screen link, seeded from the
  stored measurement, whose Skip preserves rather than clears it.
- **`web/tsconfig.json` covers only `src`.** The e2e specs are type-checked by
  `tsconfig.e2e.json` (`just ci` runs both). They went unchecked for their whole
  life before that and had four real type errors sitting in them.
- **Pair slots are counterbalanced; never assume A/B ordering.**
  `sampling::counterbalance_pair` randomises which encoding lands in slot A,
  applied once in `next_trial` after every pair-building path. Before it,
  `try_pair`'s `(sorted[i], sorted[i+1])` put the better image in B on 60/60
  live trials, making the 2AFC answer constant. `expected_choice` flips with
  the slots. Anything downstream must key on `a_encoding_id`/`b_encoding_id`,
  never on "b is the better one".
- **A study's content restriction is as load-bearing as its trial mix.**
  `src/content_class.rs` maps a source's `corpus` (the stratum name — the only
  content signal `SourceMeta` carries) to photo / non-photo, and
  `SamplerConfig::content` restricts the draw. `ssim2-nonphoto` served ~38%
  photographs while constraining only `pairwise_only`, because the name was the
  only thing asserting the content. Unknown strata are REFUSED by a restricted
  study, never admitted; keep the table in sync with
  `scripts/build_demo_corpus.py::R2_STRATA` (guarded by
  `strata_agree_with_the_corpus_builder`).
- **Metric efficacy on one content class is only measurable against another
  arm of the SAME instrument.** `ssim2-photo-control` is identical to
  `ssim2-nonphoto` except for `ContentFilter::PhotoOnly` — that sameness IS the
  control, so do not "tidy" a field into differing. Comparing our non-photo
  number against a published photographic one (CID22/KADID) is invalid:
  different observers, UI, pair selection and protocol. And compare
  `ρ / ceiling`, not `ρ` — humans may just be noisier on one class.
- **A rank-agreement number is meaningless without a noise ceiling.**
  `Study::p_repeat` re-serves pairs the observer already answered; their
  agreement with themselves is the ceiling any metric could reach. "ssim2
  scored 0.7" reads completely differently against a ceiling of 0.95 than
  against 0.72, so the repeat data is not optional colour — it is what licenses
  a conclusion about the metric rather than about the data collection.
  `Study::p_golden_pair` is the attention check: `p_honeypot`/`p_anchor` are
  necessarily 0 in a forced-choice study (both build single-stimulus trials),
  so before this the study had **no controls at all**.
- **Feedback to an observer is about PROCESS during a session, and about
  OUTCOME only in training or in aggregate afterwards.** Two things would
  destroy instruments we depend on, and neither is recoverable after the fact:
  telling somebody a trial is a *repeat* makes them recall their previous answer
  instead of judging again, which measures memory and reports it as the noise
  ceiling; and telling somebody they got a *golden pair* wrong both identifies
  the attention checks and trains them toward whatever produced the "correct"
  answer — which is the metric under test. So a mid-session notice may say
  "flick between them before answering" (what to do) and never "you are
  answering too fast" (how you did). See `docs/OBSERVER-FEEDBACK.md` for the
  full design and the literature it comes from (zenpapers ch10 §10.2.3,
  training-as-tuning).
- **At N≈2 observers the screens become operator diagnostics, not gates.**
  §4.4 peer-mean correlation has no peers to be an outlier against, and
  Crowd-BT η is weakly identified — with two observers the fit cannot separate
  "this observer is noisy" from "these items are genuinely close". Acting on a
  screen would also cost half the dataset. `exclusion.rs` already records
  without enforcing (`ExclusionPolicy::enabled`); keep it that way and improve
  the data by calibrating observers instead of dropping them. What still works
  at N=2 is strictly within-observer: `p_repeat` self-agreement, `p_golden_pair`
  attention checks, and transitivity. Do NOT quote an η at this N without first
  checking it against the goldens.
- **Difficulty is recorded per view, raw.** `reveal_ms_total` only ever measured
  the reference, which under `hold`/`buttons` is the *resting* view — so it
  tracks "not pressing anything", not effort. `switch_count` + `ms_on_a/b/ref`
  (migration 0019) are the real signal: a pair flipped six times over twenty
  seconds sits near the observer's threshold, and BT cannot tell that from a
  two-second answer. Stored raw and normalised in analysis — the useful form is
  relative to that observer's other trials, and the session is not finished when
  the row is written.
- **The comparison mode is chosen by the observer, once, before the first
  trial.** `web/src/mode-chooser.ts`; `hasChosenInputMode` is deliberately
  separate from "which mode are we in" — `loadInputMode` always returns
  something, so a device default silently applied would look like a choice and
  the prompt would never fire for the observers already in the study. Defaults
  are `hold` on touch and **`buttons` on mouse** (was `tap`): both keep the eye
  on the picture and change it underneath, which is the comparison being asked
  for. **Touch offers `hold` alone** — `tap` costs a look away from the stimulus
  per switch on the smallest screen, and splitting mobile data across two modes
  bought nothing — so `availableInputModes()` returns one entry there and the
  chooser renders a how-to instead of a one-option "choice". `loadInputMode`
  demotes a stored mode the device no longer offers, or a phone that picked
  `tap` earlier would be stranded in an unavailable UI.
- **No double-tap-to-fit; the comparison already owns that gesture.** It reset
  magnification to "the whole image just fits" on two quick presses in the same
  place — which under `hold`, the only touch mode, IS the comparison: press a
  half, release, press again. So the magnification reset itself mid-judgement,
  worse than not having the shortcut. Removed along with `resetToFit`/
  `fitFactor`, which existed only for it. Magnification is pinch on touch,
  digits and the wheel on a mouse. Do not re-add a tap-count shortcut here: a
  gesture cannot be reserved for a shortcut when the task has already claimed
  it. (Guarded by "a repeated tap does not disturb the magnification".)
- **A moving touch is still a touch.** `syncButtons`'s touch branch must
  release only on `pointerup`/`pointercancel`. It was
  `if (pointerdown) press else release`, and `pointermove` routes through it, so
  a single pixel of drift released the hold — and under `hold` (the only touch
  mode) holding IS the gesture, so the comparison collapsed as soon as a thumb
  moved. Panning is driven separately from the hold stack, so nothing there
  needs to know about the drag. The mouse branch diffs the button mask and was
  never affected, which is exactly why this survived: the drag test uses
  `page.mouse`, so touch-plus-movement had no coverage. Any new hold logic needs
  a *touch* test, not just a mouse one.
  `touchstart` also carries `preventDefault()` (`passive: false`) against
  Android's long-press callout, whose `pointercancel` is not cancellable — real
  hardening for a press-and-hold UI, but it was not this bug's cause. Beware
  diagnosing hold problems as the callout: headless Chromium does not run the
  gesture recogniser, so that hypothesis is unfalsifiable here, while the
  release-on-move bug reproduces in a plain e2e test.
- **The variant indicator must survive the stimulus covering the frame.** The
  tiled letterbox surround only paints where the picture does not reach, so it
  vanishes exactly when someone magnifies — most of a careful session. The edge
  frame (`.stage` + `.edge-*`) is the durable cue: **outside** the stimulus, via
  padding on `.stage`, so it can never occlude pixels under judgement (same rule
  the reveal hint had to obey). Position carries identity — A lights the LEFT
  edge, B the RIGHT, the original the TOP — matching `hold`'s halves and the A/B
  order of the answer buttons.
  **Two contacts are not a pinch.** `applyHolds` used to refuse whenever
  `held.size >= 2`, and `pointerdown` committed to `gesture = 'pinch'` on the
  second finger — so holding one half and tapping the other did nothing at all,
  silently disabling two-finger comparison on the only device the study runs
  on. A pinch is now committed only once the contacts separate by
  `PINCH_COMMIT_CSS`; until then the second finger is an ordinary press and the
  hold stack decides, most-recent-still-held. The first contact ALWAYS resets
  `gesture` to `none` — guarding that reset with `if (gesture !== 'pinch')`
  stranded the state and swallowed every later single-finger drag.
  **The lit bar is COLOURED, and that is a deliberate exception to the
  neutral-surround rule.** The first version obeyed it (dark neutral grey) and
  was invisible on a phone — a cue nobody can see is not a cue. The bar now
  takes the same colour that arm's button takes when active (`--accent` for A/B,
  `--good` for the original), so the two agree. What keeps the psychovisual
  argument intact: the strip is ~7px at the extreme edge, far outside the
  judged region; only ONE bar is lit at a time (the others sit near-black), so
  total frame luminance barely moves between views; and the large-area
  letterbox surround — where adaptation actually bites — is still strictly
  neutral. Do not "restore consistency" by neutralising the bars or by
  colouring the letterbox.
  `.stage` is full-bleed: `.trial` carries no horizontal padding and each other
  row re-adds its own, so the bars reach the screen edge. A gutter outside them
  would waste exactly the space they are kept under 8px to save.
- **A `hidden` grid item is not a grid item.** `hidden` is `display: none`, so a
  conditionally-hidden child does not occupy its track — every later child
  shifts up one, and `.stage` lands on an `auto` row instead of the `1fr` one.
  The viewport then collapses to content height, the stimulus never sizes, and
  `.viewport.all-ready` never arrives. Adding the lap bar this way cost **257
  e2e failures**, every one of them timing out somewhere that looked nothing
  like a layout bug — and the suite ran 25 minutes instead of 2 because each
  failure burned its timeout. `.lap[hidden]` therefore sets `display: block;
  height: 0` rather than letting `hidden` do it. Any new row in `.trial`'s
  fixed `grid-template-rows` needs the same treatment.
- **The lap bar counts what `MIN_OBS_FOR_ETA` counts.** 20 comparisons is the
  point at which an observer's reliability becomes estimable, so the bar tracks
  a real boundary rather than an invented one — that is the whole licence for
  gamifying it. Server-side and lifetime (`ResponseAck.total_comparisons`), not
  per session: a returning observer who already passed 20 must not be shown
  zero. Pair trials only; a 4-tier rating does not feed η, so counting it would
  move a bar toward a milestone it cannot reach. A revision reads the count back
  instead of incrementing, so undo cannot advance it.
- **Notifications live in one place, and must stay one line.** `web/src/notify.ts`
  owns placement, dwell, fade and tap-to-dismiss, and renders into a fixed
  `#notice-layer` on `document.body` — NOT inside the current screen, because a
  notice is raised by an answer landing and the app replaces `root.innerHTML`
  milliseconds later, which destroyed it. `white-space: nowrap` is load-bearing:
  a wrapped sentence turns a notice pinned to the chrome into a band across the
  picture (measured at 29–45% of the visible stimulus on a 304px screen). Keep
  milestone copy short for the same reason.
  It DOES cross the picture's top edge when the stimulus fills the frame — a
  deliberate, bounded exception like the coloured edge bars: extreme top edge
  only, two seconds, tap-dismissable, semi-transparent so the pixels are not
  replaced, and raised at the START of a trial where the seen-both gate
  guarantees comparison has not begun. Guarded at <18% of the VISIBLE picture
  (a magnified stimulus extends past the frame, so its own rect is the wrong
  denominator).
- **A UI nudge toward a specific answer must be recorded, or it contaminates
  silently.** After `CANT_TELL_HINT_AFTER_HELD_MS` of *held* time the tie button
  breathes, because at threshold the truthful answer is a tie but the button
  reads as giving up — so people grind on and guess, and a guess recorded as a
  preference is worse data than a recorded tie (Davidson's model has a tie term
  precisely so it is an outcome, not noise). It fires on exactly the hardest
  trials, so `responses.cant_tell_hint_ms` (migration 0021) is what lets an
  analyst compare hinted against unhinted tie rates or drop hinted trials.
  "Held" means time on any view that is **not** the resting one — under `tap`, A
  *is* the resting view, so `ms_on_a + ms_on_b` would grow while someone sits
  doing nothing. Never fires on a single-stimulus trial (no tie to offer) or
  before the seen-both gate is satisfied.
- **A DEFAULT-0 backfill is not data.** Columns added by a migration arrive on
  every existing row as `NOT NULL DEFAULT 0`, which means *not recorded* — never
  *measured as zero*. The leaderboard averaged the two together and reported "0
  swaps" for everybody, because 91 of the first 154 live responses predate
  migration 0019 and the median landed in the backfill (measured 2026-08-04; the
  same rows median 69 once excluded). `leaderboard` now filters on
  `switch_count = 0 AND ms_on_a = 0 AND ms_on_b = 0 AND ms_on_ref = 0` — reliable
  rather than a date guess, because an instrumented trial always accrues time on
  *some* view when `closeViewAccounting` closes the open interval at commit —
  and publishes `instrumented_trials` so the figure can be read against its
  sample. Any future effort column needs the same treatment.
- **Billable time is engaged time, with an idle cap.** `active_seconds` on the
  leaderboard: within a session, the gap between consecutive answers, each
  capped at `IDLE_CAP_MS` (5 min), plus the first answer's dwell. Neither naive
  measure is fair — summing `dwell_ms` undercounts (it starts at first paint, so
  it excludes waiting for the next trial to be chosen and fetched), and
  last-minus-first overcounts by every break and overnight gap. The cap is
  generous against the observed distribution on purpose: trials run a median
  ~14s but the tail reaches 163s, so a tighter one would discard genuine
  deliberation on hard pairs. Deliberately reproducible from `responses.tsv`
  alone (`session_id`, `responded_at`, `dwell_ms`) so the number can be checked
  rather than trusted.
- **A response can be revised, never deleted — and the first answer survives.**
  Undo (migration 0020) writes `original_choice` / `revised_at` /
  `revision_count`; `choice` holds the answer that counts. Deleting the first
  one would let an observer retroactively tidy their own data, and "answered A,
  then changed to B" is itself a signal about difficulty or attention. Two
  guards that must not be relaxed: `record_response` revises only the *latest*
  trial in the session (else 409), so undo cannot walk back through a whole
  run; and attention checks score `COALESCE(original_choice, choice)` in BOTH
  `grading.rs` and the leaderboard, or undo defeats the honeypot — fail it,
  notice, take it back. Reopening a trial resets its seen-gate on purpose: an
  undo is for a misclick, not for re-answering without looking again.
- **Answering requires having seen every arm being judged.** Arrival of the
  images was never evidence anyone looked at them: under `tap`, A is the resting
  view, so B could be rated having never been on screen. `trial.ts` tracks a
  `seen` set and gates on `requiredViews()` — both arms on a pair, the
  compressed image on a single. Enforced inside `commit` as well as on the
  buttons, because keys reach `commit` directly. The hint text is
  **mode-specific** (`gateHintFor`): "look at B first" names a control only
  `tap` renders, so under `hold` it would leave someone at a dead panel. A
  single-stimulus trial IS gated under `hold`/`buttons` — the reference rests
  there and the image being rated is genuinely not on screen yet; that is the
  gate working, not a bug to remove. Note the A/B/Original switch is rendered in
  every mode, which is why `satisfyGate` in the e2e helpers works everywhere.
- **The front page is `/`; the session is `/rate`.** Two routes, not two screens
  behind one URL. `/` explains the study, shows each study against its
  pre-registered targets and the reviewer board, and offers guest-or-sign-in;
  `/rate` is onboarding + trials. This is also what makes the suite tractable —
  a test reaches the study by navigating, not by simulating a click through the
  front page, which is what broke ~40 specs when the page first landed. The SPA
  fallback must serve `text/html` for extensionless routes, or a browser
  navigating to `/rate` downloads a file instead of rendering.
- **The pairwise screen is Crowd-BT η, not BT.500 or peer-mean correlation.**
  Both of those need a score per stimulus, so `rebuild_dispositions` skipped
  forced choice and every observer in the DEFAULT study landed on
  `insufficient_data` — no working screen at all. `crowd_bt.rs` estimates a
  per-observer reliability jointly with the latent scores: η=1 reliable, η=0.5
  carries no information, η<0.5 anti-correlated (a reversed UI, or answering the
  opposite of what was meant — a failure a symmetric outlier test cannot see).
  Named by `ch3-5_sampling_screening_cis.md` §4.6 for exactly this design
  (crowdsourced PC + active sampling). NOT for sampling — the same chapter shows
  its active variant losing to ASAP at every budget, so ASAP still picks pairs.
  η is NULL below `MIN_OBS_FOR_ETA`, never 0.
- **Participant exclusion is a recorded disposition, never a delete.**
  `src/exclusion.rs` runs the zenpapers Ch. 4 screens (§4.4 peer-mean
  correlation, §4.2.1 BT.500 kurtosis-2 band) and writes
  `observer_dispositions`; `responses.tsv` carries the verdict per row. The
  screens run regardless of the switch — `Study::exclusion_default` /
  `SQUINTLY_EXCLUSION` only decide whether consumers act on `excluded`. §4.2.2
  is why: hard reject loses all data from rejected subjects and draws a sharp
  boundary, so soft weighting (which `grading.rs` already does) supersedes it.
  `insufficient_data` ≠ `included`; a solo expert lands there because there are
  no peers to be an outlier against.
- **Trial variants are preloaded; the next trial is not.** `trial.ts` stacks A,
  B and the original as separate `<img>` layers loaded up front, so switching is
  a visibility toggle rather than a `src` rewrite (a real source is 9.5 MB /
  0.33–1.1 s cold from R2 — measured 2026-07-30 — and that was paid on the first
  flick to the original). The response panel is disabled until the judged image
  paints. The *next trial* is deliberately not prefetched: `enhance_pair_with_asap`
  picks it by expected information gain over the answers so far, so fetching
  early would choose the stimulus without the current response.
- **`input_mode` is recorded because it changes what other columns mean.**
  `tap` shows the encoding and peeks at the reference; `hold` inverts that — the
  reference is the resting view, and holding the left/right half of the
  *viewport* flicks to A/B (decided on press, so a drag across the midline can't
  swap the variant mid-comparison). So `reveal_ms_total` measures
  a peek in one and the default state in the other. Migration 0017; same
  reasoning as `study_id`.
- **The corpus must come from the imazen-26 TEST split.** The split is by
  ORIGIN — last digit of the leading numeric filename stem, {0,2,4,6,8} train /
  {1,3,5} val / {7,9} test — and every rendition inherits its origin's bucket
  (`zensim/docs/DATA_SPLITS.md` §2a). `build_demo_corpus.py --split` defaults to
  `test` and **imports** `origin_split.py::split_of` rather than
  re-implementing it; a missing canonical file is a hard failure, because a
  drifted copy of a split rule is indistinguishable from a correct one until the
  results are already wrong.
  This was missed entirely at first: the builder was split-blind and ranked by
  pixels, so the shipped corpus measured **60% train / 29% val / 11% test**
  (2026-08-04, imazen-26-png-v3). Human judgements collected on training images
  cannot score a metric fitted on them. Switching to test-only costs nothing —
  the same 45 origins, every stratum able to fill its quota — so there is no
  trade to reconsider here.
  The rule is CROSS-CHECKED, not merely applied: every pick is compared against
  its recorded label in
  `/mnt/v/output/imazen-26-features/imazen26_split_evenodd.tsv` and a single
  disagreement fails the build. A derivation that silently diverges from the
  labels the rest of the pipeline was built against is worse than no rule —
  every downstream number lands in the wrong bucket and nothing says so.
  Verified 2026-08-05: `split_of` reproduces all 2,157 labels exactly. Live
  corpus WAS `imazen26-v4-test`, 180/180 test-split; then
  `imazen26-v5-test-noai` (168 sources / 4032 encodings, AI strata dropped).
  As of 2026-08-06 a v6 with the 17-rung ladder is built (168 sources / 11,424
  encodings) and pending publish — check `SQUINTLY_COEFFICIENT_HTTP` on the
  deployment for what is actually live rather than trusting this line.
  `trials.source_filename` (migration 0022) exists so the split is recomputable
  from `responses.tsv`: `source_hash` cannot answer it, since the rule reads the
  filename. Rows collected before it are NULL, which is deliberately different
  from "present but unsplittable".
  A stratum that cannot fill from the chosen split is a hard error, never a
  silent under-fill — and an *excluded* stratum is subtracted from that check,
  because deliberately dropping one is not the same as one silently vanishing.
- **No AI-generated images in the study corpus.** `R2_EXCLUDE_STRATA` drops the
  three `*-ai-*` strata. The study measures how compression artefacts look on
  real web content: a diffusion model's output is already smooth in ways a
  camera or a scan never is, carries no sensor noise or scan grain for a codec
  to spend bits on, and has synthesis artefacts an observer can mistake for
  compression. Judgements on it generalise to other generated images, not to
  the photographs, scans, screenshots and documents the metric is pointed at.
  Costs no coverage — ten non-photo strata remain.
- **The quality ladder is spaced by MEASURED perception, not by q units.**
  `DEFAULT_QUALITIES` is 17 rungs `[15,18,22,26,30,38,45,52,60,68,75,82,88,92,
  95,97,100]`, chosen by interpolating the measured q→ssim2 curve so no adjacent
  gap exceeds ~5 points (largest is 4.9). The old 6-rung grid
  `[15,30,45,60,80,92]` was even-ish in q and wildly uneven in what an observer
  sees: median adjacent ssim2 gaps of 17.3 / 7.9 / 5.7 / 8.0 / 6.2. Eight rungs
  sit below q60 and eight above, so the low-q half stays as dense as the high-q
  half. Do NOT "simplify" this back to round numbers — the unevenness is the bug
  it fixes. Rung count does not worsen the ASAP cold-start problem, which counts
  observations per (source, codec) cell.
- **Human agreement with ssim2 hits 100% at a 5-point gap.** Measured
  2026-08-06 on 84 live comparisons with both arms scored: 0–5 points → 94%
  agreement (n=16), 5–10 → 100% (n=44), 10+ → 100% (n=24). That is what
  "foregone" means numerically, and it is why `TRIVIAL_SSIM2_GAP` exists.
  The constant is 15 rather than 5 — a compromise named in the code, because at
  5 the corpus could build almost no servable pairs.
- **ρ/ceiling above 1 is a warning about the STIMULI, never a result.**
  First complete reading, 2026-08-06: ceiling 0.90, ssim2 ρ=0.988 over 84 scored
  comparisons, ρ/ceiling **1.10**. A metric cannot really beat humans; that
  number appears when the pairs are easy enough that both get them right while
  the repeats that *did* disagree were the genuinely hard ones. Read it as
  "make the corpus harder", which is what the 17-rung ladder is for. See
  `docs/OBSERVER-FEEDBACK.md` §8.
- **ASAP is wired but has never engaged.** `enhance_pair_with_asap` runs in
  `next_trial`, but it needs `ASAP_MIN_OBS` (8) observations on one
  (source, codec) cell and the live maximum is 4 across 186 cells — so every
  pair ever served came from the random-adjacent fallback. Do not attribute
  pair selection to ASAP when reasoning about collected data. Docs that said
  "not yet wired" were wrong in the other direction and are corrected in place.
- **Metric scores are ingested, never computed by the server.**
  `cargo run --release --bin squintly-score -- --fs demo-corpus --out x.tsv`
  then `POST /api/admin/metrics`. The scorer FETCHES existing encodings and
  decodes them rather than re-encoding, so scores are of exactly the bytes
  observers saw. Nothing pre-existing was reusable and this was checked: all 16
  parquets under `/mnt/v/output/imazen-26-features/` are zensim SOURCE features
  with no metric columns, and the one sweep carrying `score_ssim2` is 100%
  zenjxl on differently-named files.
- **JXL enriches the near-lossless band; it is not the only way in.** Measured
  2026-08-06 on the 17-rung corpus: jpegli medians **93.4 at q100** and 863 of
  11,424 encodings score ≥90, where the old q92-capped ladder had essentially
  none. (An earlier note here said the builder codecs "top out near 86–90 even
  at q100" — that was extrapolated from the OLD ladder, which stopped at q92,
  and the measurement corrects it.) `scripts/add_jxl_rungs.py` adds high-q JXL
  as a fifth codec and extra density up top, not as a rescue. Chromium keeps JXL behind a flag;
  the codec probe means the sampler only serves what a session declared support
  for, so they degrade safely — but on a panel with no JXL-capable browser they
  are scored and never judged.
- **Source-informing-sweep rule applies.** Sampling MUST cover all 4 size buckets and
  weight low-q encodings. See `src/sampling.rs`.

## Where to look

- `src/main.rs` — entrypoint, CLI, axum router
- `src/coefficient.rs` — both Http and Fs impls of the Coefficient trait
- `src/handlers.rs` — HTTP route handlers
- `src/sampling.rs` — trial pair selection
- `src/bt.rs` — Bradley–Terry-with-ties fit (Davidson 1970)
- `src/export.rs` — TSV streaming in zenanalyze schema
- `src/curator.rs` — corpus curator backend (`/api/curator/*`)
- `src/licensing.rs` — per-corpus license registry surfaced in UI + exports
- `web/src/curator.ts` — curator Stream/Curate/Threshold screens
- `web/src/curator-encoder.ts` — browser-canvas JPEG encoder for the slider
- `web/src/conditions.ts` — browser-side viewing-condition capture
- `web/src/calibration.ts` — credit-card mm-per-px calibration

## Curator data flow

1. Operator POSTs a candidate manifest to `/api/curator/manifest`. Either
   corpus-builder TSV (e.g. `/mnt/v/output/corpus-builder/curated_manifest_2026-04-16.tsv`)
   or the unified R2 JSONL at
   `https://pub-7c5c57fd3e0842f0b147946928891d40.r2.dev/manifest.jsonl`.
   Inserted into `curator_candidates` with per-corpus license attribution
   from `src/licensing.rs`.
2. The browser fetches `/api/curator/stream/next?curator_id=<uuid>` to get
   the next undecided candidate plus a default-on suggestion (groups + size
   chips) computed from the source's detected q (when available).
3. Curator swipes left/right or taps Skip/Take. `Take` advances to the
   Curate screen for group selection + size-chip toggling. `Find threshold`
   opens the slider with both 1:1-device-px and 1:1-CSS-px split panels.
4. `POST /api/curator/threshold` saves `q_imperceptible` along with the
   measurement DPR, distance, and encoder identity. `GET
   /api/curator/export.tsv?curator_id=…` joins everything into one TSV
   carrying the license columns downstream consumers need.

## License posture

Squintly never claims to know per-image licenses unless the manifest
provides them. The `licensing` registry maps **corpus** to policy. The
welcome screen shows a credits panel; the curator screens show inline
badges; the trial header shows the **corpus alone**. The license label moved
off the trial card to the `i` identifier panel and the header's `title` — it
cost a scarce line on a phone to print a string nobody reads mid-trial, and
attribution still ships in two places a reader can reach without leaving the
trial. Do not put it back in the header. When the live R2
manifest grows per-image `license_url` fields, the existing
`curator_candidates.license_url` column carries them through to exports.

## Running locally

```bash
just dev       # cargo watch + vite dev with proxy
just build     # build frontend then cargo build --release
just test
```

## Investigation Notes

### Desaturated reds in encodings are chroma subsampling, not a colour bug (2026-08-03)

Reported from the live study: "so many a and b variants have super desaturated
reds". Real, measured, and **not** a pipeline defect — but it does have a
recording gap worth knowing about.

**Not colour management.** Nothing in the corpus carries an ICC profile —
neither the source PNGs nor any encoding (measured: `icc_profile` is absent on
all of them), so a browser treats every arm as sRGB and the reference and the
encodes are handled identically. `build_demo_corpus.py` contains no ICC,
colorspace or profile handling at all.

**It is chroma.** Red lives almost entirely in Cr, so it takes the brunt of both
half-resolution chroma and the harsher chroma quantization at low q. Measured on
`imazen26-6600-ia-scans-manuscript-illustrations` (red-heavy botanical plate),
saturation of red pixels vs the source:

| codec | subsampling | q15 | q92 |
|---|---|---|---|
| jpegli | 4:4:4 | −9.6% | **−2.1%** |
| libjpeg-turbo | 4:2:0 | −10.8% | −5.2% |
| libwebp | 4:2:0 | −9.7% | −4.7% |
| libavif | 4:2:0 | −8.1% | −4.5% |

Overall saturation moves ±1%; the loss is specifically red. The tell that it is
subsampling and not quantization is that it **persists at q92**, and that jpegli
— the only 4:4:4 encoder here — is less than half as bad there.

**The subsampling is inherited, not chosen.** The builder passes no subsampling
argument to any encoder, so each takes its default: Pillow's JPEG is 4:2:0 below
quality 95 (so q92 is 4:2:0), lossy WebP is 4:2:0 by format, Pillow's AVIF
defaults to 4:2:0, and `cjpegli` defaults to 4:4:4. That is realistic web
behaviour, which is the right thing for the study to measure — but it means the
corpus mixes 4:4:4 and 4:2:0 **by accident of defaults**.

**The gap: nothing records it.** `EncodingMeta` carries codec and quality but
not subsampling, so an analyst comparing jpegli against libjpeg-turbo would
attribute a subsampling difference to the codec. Within-codec pair trials are
unaffected (subsampling is constant inside a pair), so this bites cross-codec
aggregation, not the pairwise data.

**Why it matters more now:** the live study is non-photo only, and 4:2:0 on
saturated hard-edged graphics is a known pathology — zensr's own routing treats
`4:2:0 ∨ q≲50` as a special case. So the content class under test is exactly the
one this hits hardest.

## Known Bugs

### ICC: 16 sources are Display P3, and half the encoders threw the profile away (2026-08-06, FIXED in the builder)

**Measured on the live corpus build, not inferred.** 16 of 168 sources (9.5%)
carry an ICC profile. For each of those 16, across the 17-rung ladder:

| encoder | ICC kept? |
|---|---|
| jpegli (`cjpegli`) | **yes** — 272 encodings |
| libjpeg-turbo (Pillow) | **no** — 272 encodings |
| libwebp (Pillow) | **no** — 272 encodings |
| libavif (Pillow) | **yes** — 272 encodings |

**All 16 profiles are `Display P3 Gamut with sRGB Transfer`** — genuinely
wide-gamut, not a no-op tag. Converting one to sRGB moves saturated pixels by up
to **79 levels per channel** (measured; a saturated red goes
`(237,61,37) -> (255,30,0)`). That is not a subtlety, it is a different picture.

**This is very likely a second and larger cause of the "desaturated reds"
report** recorded further down as chroma subsampling. That investigation
measured 4:2:0 effects of ~5-10% and never checked ICC; on these 16 sources the
profile loss is a far bigger colour move, and it hits exactly the saturated reds
that were reported. The subsampling finding stands on its own merits — but it is
not the whole explanation, and the note below should be read with this one.

Two consequences, and the first is a pixels bug:

1. **The observer sees a colour difference that is not compression.** A browser
   renders the source PNG through its profile and a profile-stripped JPEG as
   sRGB. On a wide-gamut source that is a visible shift, and it is attributed to
   the codec. Same class as the desaturated-reds finding below, but this one is
   an outright loss rather than a subsampling artefact.
2. **`squintly-score` ignores ICC entirely** (`decode_rgb8` reads raw samples),
   so its ssim2 is self-consistent but does not measure what the observer saw
   for those 16 sources. Cross-codec comparisons are the ones affected —
   within-pair is safe because pairs are same-codec.

This ALSO corrects the investigation note further down that says "Nothing in the
corpus carries an ICC profile — measured: `icc_profile` is absent on all of
them". That was true of an older build and is not true now; do not rely on it.

**Fixed 2026-08-06 in `load_rgb`**: sources are now converted through their
profile to sRGB once at build time, and the profile is then STRIPPED (Pillow's
`profileToProfile` attaches the destination profile, which would have
reintroduced the same split). Convert-and-strip is chosen over preserve-
everywhere because it depends on Pillow alone rather than on four encoders each
continuing to agree about metadata they treat as optional. A profile that fails
to convert skips the source rather than being assumed sRGB — a missing image is
visible in the stratum counts, a mis-coloured one is not.
**The v6 corpus was built BEFORE this fix and must be rebuilt before publishing.**

Note that switching scorers does NOT fix this: verified 2026-08-06 that
`zenmetrics batch --metric ssim2` and `squintly-score` agree to 0.3 ssim2 points
on an ICC-carrying source, i.e. **neither applies ICC** — they both read raw
samples. The confound was in the corpus, not the metric.

**The right fix for the DUPLICATION is not to patch the hand-rolled paths.** `zenmetrics-cli`
already does correct decode + colour handling and computes ssim2 (and five other
metrics) — `squintly-score` re-implemented decoding for PNG/JPEG/WebP/AVIF/JXL
that zenmetrics already had. Prefer wiring zenmetrics-cli in over extending
`src/bin/score.rs`. Likewise the corpus builder's encode paths go through Pillow
rather than zencodec, which is where the ICC inconsistency comes from.

Also unverified: imazen-26 contains ultra-quality JPEGs and HEICs upstream, and
what the PNG conversion into `imazen-26-png-v3` did to their colour metadata has
not been checked here.


### Metric scores live in `encoding_metrics`, ingested — never computed here

Squintly does not compute metrics; it ingests them. `POST /api/admin/metrics`
takes a wide TSV/CSV/Parquet and writes long rows to `encoding_metrics`
(migration 0025), joined at report time. Resolved 2026-08-05; the gap it closed
is recorded in the resolved-bug log below.

Three rules that are load-bearing rather than stylistic:

- **Names are open, directions are closed.** Ingest accepts ANY metric name,
  because zenmetrics bakes an implementation version into several of them
  (`cvvdp_imazen_v0_0_1`, `ssim2_imazen_iir_v3`) and a retuned kernel mints a
  new one. But `metrics::direction_of` must RECOGNISE a name to say which way
  it points, and an unknown one gets `Direction::Unknown` — storable and
  exportable, never correlatable. A rank correlation with an unknown direction
  is a coin flip on the SIGN, and a flipped sign reads exactly like the finding
  the study exists to make. The report lists such metrics under `unusable`
  rather than omitting them.
- **A blank cell is NOT MEASURED, never 0.0.** cvvdp needs a GPU, iwssim needs
  `min(W,H) >= 176` — gaps are normal, and a 0.0 puts a worst-possible score on
  an encoding nobody measured. Same distinction as the DEFAULT-0 backfill rule.
- **Metadata columns never become metrics.** `NON_METRIC_COLUMNS` drops
  `bytes`, `quality`, `codec` and friends. `bytes` would correlate beautifully
  with human judgement and be entirely an artefact of bigger files looking
  better.

**Everything about metrics is admin-gated, including reading.** Showing an
observer the ssim2 of the image they are about to judge hands them the answer to
the question being asked. The trial identifier panel therefore fetches scores
through an admin endpoint and renders nothing on the 403 an ordinary observer
gets — no gap, no error, just the panel they always saw.

`src/disposition.rs` + `/api/admin/disposition` is the report. Self-agreement is
computed over pairs normalised to the SORTED pair, not the A/B slot:
counterbalancing flips slots on about half of all repeats, so comparing raw
`choice` strings would score a consistent observer as inconsistent every time,
halving the ceiling and making every ρ/ceiling look twice as good as it is.
`rho_over_ceiling` is `None` whenever either input is, so a ρ can never be
printed against an assumed ceiling of 1.

### Curator write endpoints are unauthenticated on a public instance

`POST /api/curator/manifest`, `/api/curator/decision`, `/api/curator/threshold`
and `/api/curator/generate-variant` take no token — only `load-r2-public` and
`backfill-dims` call `require_curator_admin`. On the public Railway deployment
that means anyone can inject candidates, record decisions, and drive variant
generation. The SSRF that fell out of this is now closed (`guard_blob_url`,
2026-07-27), but the underlying posture is unresolved: the curator is a
*researcher* tool exposed on an *anonymous-participant* origin.

**Partially addressed 2026-07-29.** There is now a real admin identity to gate
on: signing in mints an `auth_sessions` cookie and `curator::require_admin`
accepts either a signed-in address on `SQUINTLY_ADMIN_EMAILS` or the shared
token. `load_r2_public`, `backfill_dims` and `delete_candidate` go through it.

Still ungated: `POST /api/curator/manifest`, `/decision`, `/decision/undo`,
`/generate-variant`, `/threshold`. Wrapping them is now a small mechanical
change (add `headers`, call `require_admin`) — the reason not to do it blindly
is that the curator UI is currently usable by an anonymous operator, so gating
these turns curation into a sign-in-required flow. **Decide that trade before
advertising the live URL widely.**

### Trial images are served DIRECT; the proxy is for canvas paths only

`handlers::source_url` / `encoding_url` hand the browser the store's own URL
whenever the store is web-reachable (`CoefficientSource::public_source_url`),
falling back to `/api/proxy/...` for `Fs`/`Disabled` or when
`SQUINTLY_DIRECT_BLOBS=0` forces it (an IP-allowlisted origin). Everything used
to be proxied, so the server fetched and re-served every stimulus — up to 9.5 MB
each — and added a whole round trip to the thing the observer is waiting for.
`HttpCoefficient` sends no credentials, so anything it can fetch is by
construction fetchable by the browser too.

The proxy is NOT dead: the note below is why. Anything reading canvas pixels
must keep using it.

### R2 corpus blobs have no CORS — canvas paths must use the proxy

The canonical bucket (`pub-…​.r2.dev`) serves blobs with **no**
`access-control-allow-origin` and `content-type: application/octet-stream`
(measured 2026-07-27). Plain `<img>` display works; anything that reads canvas
pixels back — the curator threshold encoder, the preview strip — must load via
the same-origin `GET /api/curator/blob/{sha256}` proxy (`curator::blob_proxy`),
because `<img crossOrigin="anonymous">` against R2 fails to load outright.
`web/e2e/mock-coefficient.ts` is deliberately CORS-less so e2e reproduces this;
two specs in `curator.spec.ts` guard it. Fixing the bucket's CORS config would
be a fine *additional* step but must not be the only defence — the proxy works
regardless of who owns the bucket.

### Small stimuli are magnified to cover the frame (updated 2026-08-01)

`trial.ts` renders the stimulus at a hard minimum of **1:1 device pixels** and
never downscales — anything larger than the screen is panned, so an XL source
really does need dragging to see. That part is unchanged and may not be "fixed"
by scaling to fit: a display downscale means the observer is rating the
browser's resample rather than the encode.

**What changed:** an undersized stimulus is now magnified to *cover* the frame
(`trial.ts::ensureCovers`), at whole factors only. An S-bucket source at 1:1 on
a DPR-3 phone is ~80 CSS px — unjudgeable — and magnifying is the one remedy the
display rule permits, because integer nearest-neighbour invents nothing: one
source pixel becomes an exact N×N block. It only ever *raises* the factor, so a
magnification the observer chose survives a small source.

Zooming in beyond 1:1 is acceptable. Going below it is not.
`responses.zoom_factor` records the factor per response, so an analyst can
condition on visual angle rather than assume 1:1.

### Unknown `/api/*` paths return HTML with the extension's content-type

The SPA catch-all serves `index.html` for unmatched routes, and the content-type
is guessed from the *request path*, so `GET /api/nope.json` answers `200` +
`content-type: application/json` with an HTML body. Harmless in the browser,
confusing for API clients probing for a route's existence — it's why a missing
endpoint reads as "present but unparseable" rather than 404.

## Resolved bug log

### Docker/Railway deploys broken for two months (fixed 2026-07-27)

`src/handlers.rs` read the git commit with `env!("SQUINTLY_BUILD_COMMIT")` (set
by `build.rs`), but the **Dockerfile never copied `build.rs`** — so the build
script didn't run in the container and the crate failed to compile. Landed with
the build_commit feature on 2026-05-28; the last successful deploy was
2026-05-07, so `main` was undeployable for ~2 months while the stale May image
kept serving a healthy `/api/stats`.

Why it hid so well: `cargo test` / `just ci` build from the working tree where
`build.rs` exists, so they were always green; `railway up --detach` exits 0 even
when the build later fails; and the healthcheck passed because the *old* image
was still running. Nothing in the local loop touched the Dockerfile.

Fixes: `COPY build.rs` (+ a `SQUINTLY_BUILD_COMMIT` build arg) in the
Dockerfile; `option_env!(...).unwrap_or("unknown")` so a missing build script
degrades provenance instead of bricking the build (which is what the doc comment
already claimed); a startup `warn!` when the commit is `unknown`; and
`just railway-deploy` now depends on `just docker-build`. Verify any deploy with
the `build_commit` check in DEPLOY.md §13.

### unified.rs solver diverged on synthetic data (fixed 2026-05-28)

Three issues found and fixed in `src/unified.rs::fit_unified`:

1. **NaN in `d_log_sigma_o` when rating == 4** (`upper = +∞`). `pdf_u`
   was guarded to 0 for the infinite case but `upper * pdf_u = ∞ · 0`
   still produced NaN, cascading into `log_σ_o`. Fix: explicit `0.0`
   guards on both infinite arms.
2. **No prior on global `log_sigma`** → drifted into the σ ≫ 1 flat
   region where `dl_dz · (-z) → 0`. Observed σ = 2361 (BT-only ≈ 0.34).
   Fix: added `log_σ ~ N(0, 1²)` matching `log_σ_o`'s prior shape.
3. **Rating-index sign convention inverted**. Squintly's UI uses
   rating 1 = imperceptible = BEST quality, but the cumulative-link
   model treated higher μ as worse — so pair and rating signals
   contradicted, and the fit inferred m upside-down. Fix:
   `k_idx = 4 - rating` inside both the fit loop and
   `rating_log_likelihood`. Now both modalities agree that higher
   m = better quality.

Regression test:
`src/unified.rs::tests::unified_competitive_with_bt_only_on_heldout_pairs`
seeds 4 items × 240 pair obs × 120 rating obs (incl. rating = 4 trials
that exercise bug #1) and asserts the held-out per-trial Δ vs BT-only
is within ±0.1 nats. Pre-fix Δ was −0.5 nats; post-fix ≈ 0.
