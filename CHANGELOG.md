# Changelog

## [Unreleased]

### Changed
- **The live corpus is now the TEST split** (`imazen26-v4-test`, 180 sources /
  4320 encodings — the same scale as the mixed v3 it replaces). Every source was
  cross-checked against its recorded label before publishing: 180/180 test, zero
  violations. The builder now performs that check itself and fails on a single
  disagreement, so a corpus whose split cannot be reproduced from the labels
  cannot be shipped. Verified 2026-08-05: the derived rule reproduces all 2,157
  recorded labels exactly (1082 train / 657 val / 418 test).
- **The corpus is drawn from the imazen-26 TEST split.** The builder was
  split-blind — it ranked by pixel count and took what surfaced — so the shipped
  corpus measured 60% train / 29% val / 11% test (2026-08-04, imazen-26-png-v3).
  Human judgements collected on training images cannot be used to score a metric
  that was fitted on those same images. `--split` now defaults to `test` and
  imports the canonical `origin_split.py::split_of` rather than re-implementing
  the rule; a stratum that cannot fill from the chosen split is a hard error
  rather than a silent under-fill. Switching costs nothing: the same 45 origins,
  every stratum able to fill.
  - `trials.source_filename` (migration 0022) is exported so the split can be
    recomputed downstream — `source_hash` cannot answer it, because the rule
    reads the leading numeric stem of the filename. `responses` schema_version
    8 → 9 (65 columns). **Data already collected is against the mixed corpus and
    is NULL here; it needs re-collection against a test-only corpus before it
    can score a metric.**
- **An instructions screen stands in front of every open**, with its continue
  button held for 3 seconds. A button that is clickable on arrival gets clicked
  on arrival, before the text under it has been read — and what "closer to the
  original" means is the one thing an observer has to hold steady across a
  session. Once per browser session, not per page load (a reload mid-sitting is
  not somebody arriving to work), and re-readable from the pause menu.
- **The trial header names the picture, not just the collection.** The corpus
  alone identified nothing — it holds dozens of images — so "this one has a
  green band" could not be acted on. It now shows the filename, trimmed of
  extension, size rung and trailing dimensions, prefixed only by the part of the
  corpus name the filename does not already repeat (`source_label.rs`, unit
  tested): `imazen26-6600-ia-scans-manuscript-illustrations` beside
  `6605_scans-illustrations_haeckel-...` contributes just "ia manuscript".
- **Exactly one study is marked the default**, asserted against
  `DEFAULT_STUDY_ID`. "Whichever is listed first" moves the moment a study is
  added or reordered, and every session recorded under the old one becomes hard
  to interpret afterwards.
- **Studies carry a two-word short name**, shown top-left on the trial screen.
  The id is not a thing to read at a glance and the full label is a sentence, so
  an observer who switched studies mid-run had no way to see which one they were
  in without opening the menu.
- **Trial images are fetched straight from the store.** Every stimulus was
  proxied through the server, which paid for the bytes twice — a real source is
  9.5 MB — and added a round trip to the thing the observer is waiting on. The
  browser now gets the store's own URL wherever the store is web-reachable;
  `HttpCoefficient` sends no credentials, so anything it can fetch is by
  construction fetchable directly. The proxy stays for the canvas paths (R2
  serves the corpus without CORS) and for stores the browser cannot reach, and
  `SQUINTLY_DIRECT_BLOBS=0` forces it back for an IP-allowlisted origin.
- **The identifier panel lists the source filename.** Every imazen26 source
  carries a meaningful one, and it is what a person uses to find the picture
  again — a sha256 identifies an image but says nothing about what it is.

### Fixed
- **The leaderboard reported "0 swaps" for everyone.** Responses written before
  migration 0019 carry that column's `NOT NULL DEFAULT 0`, which means *not
  recorded*, not *did not switch* — and with 91 of the first 154 live responses
  predating it, the median landed in the backfill. Un-instrumented rows are now
  excluded (identified by all four per-view columns being zero, which an
  instrumented trial cannot be), and `instrumented_trials` is published so the
  figure can be read against its sample. Measured 2026-08-04: median 0 over all
  rows, 69 over instrumented ones.
- **Two fingers could not compare.** Holding one half and tapping the other did
  nothing: `pointerdown` committed to a pinch on the second contact and
  `applyHolds` refused to run while two were down, so the hold stack's ordering
  was disabled on the only device the study runs on. A pinch is now committed
  only once the contacts actually separate; until then the second finger is an
  ordinary press and most-recent-still-held wins.
- **The tie prompt was steady, not breathing, wherever the OS asks for reduced
  motion.** The animation moves nothing — no transform, no reflow, only colour —
  so it is not the vestibular trigger the preference exists to suppress, and
  replacing it with a static fill meant those machines got no hint at all, just
  a button that had quietly changed colour. It now slows to 4.5s instead.
- **The trial control row was ragged.** Its children sat on their own baselines
  — a 44px switch, a 30px readout, a 40px icon — which read as a wrapped row and
  left the magnification readout floating off-centre between the -/+ buttons.
  They share a centreline now. On touch the keyboard-cheatsheet button is gone
  (meaningless without a keyboard, and in the pause menu anyway) and so is the
  zoom stepper (pinch and double-tap both cover it); the readout stays, since
  the factor is part of what is being judged.
- **The trial header no longer prints the licence.** "imazen26-7000-lilith-plots
  · Operator's own work" spent a scarce line on a string nobody reads mid-trial.
  The corpus stays — it identifies the picture — and attribution still ships in
  the credits panel and the `i` panel, with the full label on the header's
  `title`.
- **A held touch released on the slightest movement.** `syncButtons` handled a
  touch as `if (pointerdown) press else release`, and `pointermove` routes
  through it — so one pixel of drift released the hold and the variant snapped
  back to the resting view. A thumb on glass is never perfectly still, and
  under `hold` (the only touch mode) that is the entire gesture, so the
  comparison collapsed almost immediately. Only `pointerup`/`pointercancel`
  release now; a moving contact is still a contact, and panning is driven
  separately from the hold stack.
  - The mouse path was never affected — it diffs the button mask — which is why
    the existing "survives a drag across the midline" test, driven by
    `page.mouse`, passed throughout. Touch-plus-movement had no coverage at all;
    it does now.
  - `touchstart` also gets `preventDefault()` (`passive: false`) to suppress
    Android's long-press callout, whose `pointercancel` is not cancellable.
    That is real hardening for a press-and-hold UI, but it was **not** the cause
    of this bug and fixed nothing on its own.
- **`main.ts` carried three byte-identical copies of `boot()`**, two of them
  spliced into the middle of an expression (between `void ` and `boot();`, so
  the calibration callback and the profile screen each ran their own shadowed
  re-declaration). Legal TypeScript, which is why tsc, the build and the whole
  e2e suite never noticed. Nothing misbehaved — the copies agreed — but they
  could not stay that way: an edit to onboarding lands in one copy while the
  other two keep running the old flow. 94 lines removed, no behaviour change.

### Added
- **The observer picks a comparison mode before the first trial.** The mode
  decides what the task physically is — whether the reference is what you see at
  rest, and what your hand does to switch — and it was picked by device class
  and reachable only from a dropdown on the trial screen labelled "Interaction
  mode". Anyone not already fluent in the UI rated a whole session without
  knowing the alternatives existed. Now a one-off screen, shown last before the
  first trial (the only place a how-to about a gesture is read), with the hand
  movement spelled out per mode. Asked once; observers already in the study get
  it on their next visit, since "has chosen" is tracked separately from "has a
  mode".
- **The edge frame is colour-coded and full-bleed.** Its first form obeyed the
  letterbox's neutral-grey rule and was invisible on a phone. The lit bar now
  takes the same colour its button takes when active (`--accent` for A/B,
  `--good` for the original), only one bar is lit at a time, and the bars are
  under 8px and reach the screen edge — `.trial` gives up its horizontal gutter
  so nothing is wasted outside them. The trial header lost ~20px of height with
  it (the menu button drove a 52px row); the corpus + license label stays, since
  attribution is a standing commitment, but truncates instead of pushing the row
  taller.
- **Engaged time per reviewer, in a form that can be billed.**
  `active_seconds` on the leaderboard, shown as Hours: within a session, the gap
  between consecutive answers with each gap capped at 5 minutes, plus the first
  answer's own dwell. Summing `dwell_ms` would undercount (it starts at first
  paint, missing the wait for the next trial), and last-minus-first would
  overcount by every break. Reproducible from `responses.tsv` alone, so the
  figure can be checked rather than trusted.
- **The calibration card can be turned upright.** A card is 85.6mm on its long
  edge and a phone is about 65mm wide, so a landscape card cannot fit across a
  portrait screen — the slider ran out of travel before the rectangle reached a
  real card, making calibration impossible on the device the study mostly runs
  on. It now starts turned where the screen is taller than it is wide, and a
  button flips it either way. CSS pixels are square, so measuring along either
  axis gives the same mm-per-px and a value stored in one orientation is valid
  in the other.
- **The reviewer leaderboard is reachable.** `/api/leaderboard` existed but
  nothing in the app linked to it, so the board was unreachable from the thing
  it was built for. Now a pause-menu entry, rendered inline (the menu is already
  a modal; stacking another on a phone leaves no obvious way back). Self-agreement
  sits beside the trial count on purpose — a board that ranks on volume alone
  rewards exactly the behaviour the attention checks exist to catch.
- **An `i` panel names the images on screen, with a Copy button.** An observer
  who meets a corrupt encode or an artefact nobody can explain had no way to say
  *which* image they meant — "the B one with the green band" is not a bug
  report, an encoding id is. Lists the trial, session and study, the source
  sha256 / size / corpus / license, and per arm the encoding id, codec+quality,
  byte size and URL, plus input mode, magnification, DPR and the build commit so
  a report is attributable to a version. Copy emits `label: value` lines,
  pasteable into an issue as-is; where the clipboard API is blocked it selects
  the text instead of silently doing nothing. Opens with the button or `i`.
- **Only one hint shows at a time.** The how-to pill and the seen-both gate hint
  both explained the same left/right gesture, one line apart, on the screen with
  the least room for either. The gate's version wins while the panel is locked
  (it is the actionable one, and it goes away once you have looked); the pill
  returns when the gate opens.
- **The how-to pill can be dismissed.** The gesture is learned in a trial or
  two; after that it was a permanent band of text beside the picture, on the
  screen with the least room for one. An ✕ hides it for good
  (`squintly_hint_dismissed` in localStorage — a preference about chrome, not
  something the database should carry). It also no longer turns full accent blue
  whenever the original is up: the lit edge bar carries that in colour now, so
  the loudest thing on screen does not need to be the help text.
- **The "can't tell" prompt breathes its fill, not a 1px outline** — the outline
  was too small a change to notice while the eye is on the picture, which made
  the hint useless. Still a slow 2.8s cycle at low contrast, and nothing moves
  or reflows.
- **An edge frame shows which variant is live even when the picture covers the
  frame.** The tiled letterbox surround only paints where the stimulus does not
  reach, so it disappeared exactly when someone magnified. The frame sits
  *outside* the stimulus (padding on `.stage`, never overlapping it) and uses
  position for identity: A lights the left edge, B the right, the original the
  top — matching `hold` mode's halves. Strictly neutral grey, one bar glyphed at
  a time, so the frame's luminance does not move between views.
- **After a long comparison, the UI says a tie is a real answer.** At threshold
  the truthful answer is "can't tell", but the button reads as giving up, so
  people grind on and eventually guess — and a guess recorded as a preference is
  worse data than a recorded tie. The tie button breathes slowly after ~9s of
  *held* time (time on a non-resting view, so it means the same thing in all
  three modes). Because this nudges one specific answer on exactly the hardest
  trials, `responses.cant_tell_hint_ms` (migration 0021) records when it fired;
  `responses` schema_version 7 → 8 (64 columns).
- **An answer can be taken back.** One stray tap used to record a judgement the
  observer knew was wrong, permanently — and the trial screen is now driven by
  thumb-sized buttons, held mouse buttons and single keystrokes, so stray taps
  are *more* likely than they were. A known-wrong response is worse than a
  missing one: it enters the fit as a real opinion. The correction is recorded
  rather than destructive — `responses.original_choice` / `revised_at` /
  `revision_count` (migration 0020) keep the first answer while `choice` holds
  the one that counts, because deleting it would let someone tidy their own data
  and "changed their mind" is itself a difficulty signal. Revising is limited to
  the latest trial in the session (else 409) so it cannot walk back through a
  whole run, and attention checks score `COALESCE(original_choice, choice)` so
  undo cannot defeat a honeypot. Export gains all three columns;
  `responses` schema_version 6 → 7 (63 columns). (dc3bfc7)

### Changed
- **Touch devices are offered `hold` and nothing else.** `tap` works with a
  thumb but is the wrong instrument on a phone — three ~44px targets below the
  picture, and a look away from the stimulus on every switch, on the device
  where the picture is already smallest. It also split the mobile data across
  two interaction modes for no analytic gain. The trial screen's mode select is
  absent there rather than showing one dead option, and a phone with `tap`
  stored is moved to `hold` instead of being stranded in a UI the device no
  longer offers.
- **The mouse default is `buttons`, not `tap`.** Both defaults now keep the eye
  on the picture and change it underneath — a thumb on one half of the glass, or
  the two buttons already under the hand. `tap` asks for a click on a 44px
  target below the frame per switch, which is a look away from the stimulus on
  every comparison: the exact cost `hold` exists to avoid on touch. Still
  available, and still what an explicit choice selects.
- **Answering now requires having seen both arms.** The panel was locked until
  the images *arrived*, which says nothing about whether anyone looked at them:
  under `tap`, A is the resting view, so B could be rated having never been on
  screen. The gate now also requires every arm being judged — both on a pair,
  the compressed image on a single — enforced in `commit` as well as on the
  buttons, since keys reach `commit` directly. The hint names the gesture the
  current mode actually uses; under `hold` a single-stimulus trial is genuinely
  gated, because the reference rests there and the image being rated is not yet
  on screen. (dc3bfc7)
- **Every curator write is admin-only.** `POST /api/curator/manifest`,
  `/decision`, `/decision/undo`, `/threshold` and `/generate-variant` were
  ungated — a standing Known Bug, since they mutate the corpus every
  participant is then shown, on an anonymous-participant origin. All five now
  go through `require_admin`, which takes a signed-in address on
  `SQUINTLY_ADMIN_EMAILS` or the shared token for scripts. The curator UI has no
  token; it relies on the admin cookie, which is the real operator path — and
  the e2e suite now exercises exactly that, signing in through the actual
  magic-link flow against a mail sink rather than through a test-only backdoor.
- **The non-photo study interleaves a photographic control minority**
  (`ContentFilter::Mixed { photo_fraction: 0.25 }`), and `ssim2-photo-control`
  is unlisted as superseded. A control run as a *separate study* draws its data
  from different sessions than the data it controls for, so content is
  confounded with session — fatigue, lighting, screen and adaptation all differ
  between sittings. Interleaving is within-session and within-observer by
  construction.
  - The usual objection to mixing content, that an observer's criterion drifts
    with the stimulus ensemble, is a **rating-scale** problem. This is 2AFC:
    "which is closer to the original" is judged inside the pair, so there is no
    absolute criterion to drift, and the BT fit is per-reference anyway.
  - Asymmetric on purpose: 50/50 would halve non-photo throughput, which is the
    thing being measured. A minority suffices to estimate the photographic
    ceiling and correlation beside it.
  - When the drawn class is absent from the corpus the sampler serves the other
    rather than refusing — a mixed filter states a preferred ratio, not a
    requirement that the corpus hold both. An intermittent 409 whose frequency
    tracks a probability is a baffling thing to debug.

### Fixed
- Two test races that only appeared under full-suite load, both from mutating
  process-global env vars while sibling tests read them: `tests/curator.rs`
  cleared `SQUINTLY_SUGGESTION_ADMIN_TOKEN` in one test's teardown (yanking the
  credential from whichever test was mid-request, seen as an intermittent 503)
  and re-set it per `spawn_app` call. It is now set exactly once per binary and
  never cleared.

### Added
- **`ssim2-photo-control` — the arm that makes the non-photo result mean
  something.** "Is ssim2 good at non-photo content?" has no answer alone; a
  correlation of 0.7 is only interpretable against something. Comparing against
  a *published* photographic number (CID22, KADID) is invalid — different
  observers, UI, pair selection and protocol, so any gap could be the
  instrument. This arm is byte-for-byte the same `SamplerConfig` as
  `ssim2-nonphoto` apart from `ContentFilter::PhotoOnly`, so the two differ in
  content and nothing else (guarded by `the_photo_arm_differs_only_in_content`).
  - **Compare efficiencies, not raw correlations.** Humans may simply be noisier
    on one class: if self-agreement is 0.95 on photographs and 0.75 on
    non-photo, a lower ssim2 correlation there could be entirely human noise.
    `p_repeat` measures that ceiling per class, so the statistic is
    `ρ / ceiling` — how much of the achievable agreement the metric captured.
- **Reviewer leaderboard** (`GET /api/leaderboard`) with salted, unreversible
  handles (`src/handle.rs`, `SQUINTLY_HANDLE_SALT`). Derived from the email when
  there is one — so the handle follows a reviewer across devices, which is what
  email sign-in is for — and from the observer id otherwise. Salted because a
  public board plus an unsalted digest of a low-entropy input is an
  email-membership oracle, not anonymisation.
  - Carries both halves of "should I trust this reviewer": work (trials,
    sessions, active days) and quality (golden pass rate, **self-agreement on
    re-served pairs**, median seconds, median switches). Self-agreement is the
    one that matters — a reviewer with high volume and low self-agreement is
    contributing noise, and it is the ceiling any metric could reach against
    them.
  - Self-agreement compares the **encoding chosen**, never the slot letter,
    since repeats are counterbalanced independently — otherwise it would measure
    whether they remembered the layout.

### Changed
- **The pause menu does more than continue/end.** Switch study, change
  comparison mode, re-measure screen size, open the shortcut list. Changing any
  of these used to mean abandoning the session and hunting the welcome screen.
  Switching study starts a fresh session on the new study rather than mutating
  the current one, whose trials are all filed under the study it began on.

### Added
- **Per-view dwell and switch count** (migration 0019; `responses.tsv`
  schema_version 5 → 6): `switch_count`, `ms_on_a`, `ms_on_b`, `ms_on_ref`.
  `reveal_count`/`reveal_ms_total` only ever measured time on the *reference*,
  and under `hold`/`buttons` the reference is the resting view — so that column
  is dominated by "not currently pressing anything" and says nothing about
  effort. The informative quantity is the inverse: time holding a variant up
  against the original, and how often the observer went back and forth before
  committing. A pair flipped six times over twenty seconds sits near that
  observer's discrimination threshold; one answered in two seconds does not. BT
  treats both answers identically, so this is the only record that the pair was
  hard — which matters because a metric disagreeing on *hard* pairs is a
  different finding from one disagreeing on easy ones.
  - Stored **raw**, not normalised. The useful form is relative to that
    observer's other trials in the same session, and the session is not finished
    when the row is written; baking in a z-score against a partial session would
    be unrecoverable, while analysis can always normalise afterwards.
- **Study controls — the rank-agreement study had none.** `p_honeypot` and
  `p_anchor` are necessarily `0.0` for `ssim2-nonphoto` because both build
  single-stimulus trials that a forced-choice study excludes, so nothing in it
  distinguished a careful observer from a careless one. Two controls now:
  - **`Study::p_golden_pair`** (0.083) serves a pair far enough apart that the
    answer is not in doubt, with `expected_choice` set — reusing
    `is_trivial_pair`, the existing predicate for "obvious", which measurement
    *excludes*. A source whose ladder is too narrow yields no golden rather than
    a control whose correct answer is arguable. `grading.rs` already flags
    `golden_fail`.
  - **`Study::p_repeat`** (0.08) re-serves a pair the observer already answered,
    recording the link in `trials.repeat_of_trial_id`. This is the control that
    makes the headline number interpretable: if an observer agrees with
    *themselves* only 80% of the time, ssim2 cannot exceed roughly that, and
    "ssim2 scored 0.7" means something completely different against a ceiling of
    0.95 than against 0.72. Repeats are counterbalanced independently, so a
    repeat measures "do they judge it the same" rather than "do they remember
    the layout".

### Changed
- **All input logic is centralised in `web/src/hold-stack.ts`** — one table for
  what a press shows, one stack for what wins when several are held. It was four
  places that each half-decided the answer and disagreed: a pointer resolver, a
  release resolver, a keyboard `cycle()` that toggled instead of holding, and a
  space-bar branch.

  | input | tap | hold | buttons |
  |---|---|---|---|
  | LMB / touch | ref (peek) | half: L→a R→b | a |
  | RMB | **b** | **b** | **b** |
  | ArrowLeft | a | a | a |
  | ArrowRight | **b** | **b** | **b** |
  | Space | ref | ref | ref |

  - **The right button always means B**, in every mode — the one binding that
    never changes meaning. The left button is the mode-dependent one.
  - **Arrow keys are held, not tapped.** They used to step a carousel, so the
    keyboard and the mouse disagreed about what "left" does.
  - **Ordering is a stack**: the most recent still-held press wins, and
    releasing falls back to the next one still down. A "current wins" rule gets
    the first case right and the rest wrong; "first wins" gets the opposite set
    wrong. Keyboard and pointer share one stack.
  - On a single-stimulus trial anything selecting B collapses to the other
    available view, so no binding goes dead. A and the reference exist on every
    trial and are unaffected.

### Fixed
- **A second mouse button was invisible.** Per Pointer Events, `pointerdown`
  fires only on the no-buttons→some-button transition, so pressing a second
  button while one is held fires *no* `pointerdown`, and releasing one of two
  fires no `pointerup`. Measured in Chromium: `pointerdown buttons=1`, then only
  `contextmenu buttons=3`, then a single `pointerup buttons=0`. Button state is
  now reconciled by diffing the `buttons` mask on every pointer event.
- **Mouse holds were keyed by pointer id**, so the second button replaced the
  first and one release dropped both — a mouse reports every button on one
  pointer id. Mouse holds key by button; touches still key by pointer.
- **A transient hold redefined the resting view.** `showView` set `choiceSrc` on
  every call, so peeking at B with the right button made B the resting view and
  releasing left it there. Only an explicit pick on the view switch moves it now.
- **Two fingers cleared the hold stack**, reintroducing "two fingers, lift one,
  the original appears" — the bug the stack was added to fix. Holds are kept
  during a pinch and only the *visual* update is suppressed, so lifting one
  finger falls back to the other rather than to the resting view.

### Changed
- **Touch devices default to `hold` mode.** On a phone the segmented control is
  three small targets below the picture, and every switch is a look away from
  the thing being compared; holding one half keeps the eye on the stimulus and
  changes the picture under it. Mouse devices keep `tap`, since a pointer costs
  nothing to move and a click is not a sustained gesture. Gated on
  `pointer: coarse`, so a touchscreen laptop still gets the mouse default. An
  explicit choice always outranks the device default and survives reloads.
- **An undersized stimulus is magnified to cover the frame.** An S-bucket source
  at 1:1 on a DPR-3 phone is ~80 CSS px — a postage stamp with acres of black
  around it, and no way to see the artefacts being rated. Magnifying is the only
  remedy the display rule permits: below 1:1 resamples the encode, above it at
  integer nearest-neighbour invents nothing. It only ever *raises* the factor,
  so magnification carried across trials survives a small source.

### Fixed
- **Switching between projects was impossible.** Unlisting `main` to force the
  non-photo focus left one listed study, so the picker hid itself and there was
  no way to move between projects at all. `main` is listed again; the focus is
  carried by `DEFAULT_STUDY_ID` plus the content filter, which is enough — a
  default nobody chose away from does the job, and an operator who needs
  photographic work should not have to edit an env var to get it. Guarded by
  `more_than_one_study_is_offered_so_the_picker_exists`.

### Added
- **`zensr-dejpeg` project — JPEG artifact removal.** Each pair is one JPEG
  against zensr's restored version of *that exact file*, asking which is closer
  to the original. That question matters because zensr's entire quality story is
  **ssim2 gain**, including a dedicated graphics route (`dejpeg9_gfxycc`) — the
  same oracle imazen/squintly#4 exists to validate. It is also the right
  question rather than "which looks better": artifact removal can invent
  plausible detail that was never there, which reads as an improvement on a
  preference test and as a fidelity failure on a reference one.
- **`PairingRule` on `SamplerConfig`.** Adjacent-quality pairing cannot express
  a restoration comparison — it picks two rungs *within one codec*, so it would
  never put `mozjpeg q30` beside its own restored output.
  `RestorationVsBaseline` matches a restored encode to its input at the **same
  quality**, and refuses rather than falling back to an adjacent-quality pair,
  which would silently answer a different question under the same label.

### Known gaps
- `zensr-dejpeg` is **unlisted until the corpus has restorations**, so nobody
  lands on a study that can only 409. Producing them needs
  `zensr-zenjpeg::restore_jpeg` run over the corpus's JPEG rungs, and the
  dejpeg weights are not in the zensr tree, on `/mnt/v`, or in any R2 bucket
  (checked 2026-08-01). The study, its pairing rule and its tests are done; only
  the encodings are missing.

### Changed
- **Text-heavy strata now contribute 4 origins each instead of 1** (corpus
  `imazen26-v3`, live). The non-photo pool went from **12 distinct images to
  36** (48 → 144 sources). With one origin per stratum, "ssim2 fails on
  screenshots" could not be told apart from "ssim2 fails on *this* screenshot",
  and catching a collapsed or inverted *category* is imazen/squintly#4's primary
  deliverable — no amount of extra comparisons on a single image fixes that.
  Text-heavy got the depth because that is where a windowed SSIM-family metric
  is most likely to diverge from a human: glyph-edge ringing is highly salient
  and easily pooled away.
  - `build_demo_corpus.py` gains `TEXT_HEAVY` / `TEXT_HEAVY_ORIGINS`; the
    photographic strata stay at 1, since `content_class` excludes them from the
    live study anyway and quadrupling them would multiply encode time and R2
    storage for content nobody is served.
  - Selection now spreads across the largest quarter of a stratum rather than
    taking the top N. The comment already claimed it "spread the picks" while
    the code took the top N — harmless at N=1, but at N=4 the four largest files
    in a stratum are often near-duplicates (consecutive pages of one document),
    which would have bought breadth in name only. Verified by eye: the eight
    selected screenshots are eight different sites and apps.
  - Published as a new prefix, so the running study was never reading a corpus
    mid-swap.

### Fixed
- **AI product shots were classified as non-photo, so the non-photo study was
  serving photographs.** `9226-lilith-ai-products` is photorealistic by design —
  continuous tone, fabric texture, soft studio shadow, seamless background — and
  reading it back from the live corpus confirms it looks exactly like a studio
  product photo. The corpus builder's `is_photo` flag records **provenance**
  (was a camera involved); this module needs **appearance** (does it carry
  photographic image statistics, which is what decides whether SSIMULACRA2 is
  being asked about the regime it was tuned on). Those diverge precisely for
  photorealistic synthetic content. Reported from the live study.
  - `INTENTIONAL_OVERRIDES` records the divergence and its reason, and the drift
    guard still fails on any *other* disagreement with the builder — so this is
    a documented exception, not a silenced test.
  - All 13 non-photo strata were reviewed by eye against the served images, not
    just the reported one. The rest hold up: documents and screenshots that
    embed photographs are still document and screenshot *content*, which is the
    regime the gate targets.
- **`responses.tsv` carried no corpus or content column** (migration 0018;
  schema_version 4 → 5). Two consequences: imazen/squintly#4's per-category
  SROCC could not be run from the export at all, and a check for "did the
  non-photo study serve photographs" silently read a missing field and always
  answered no — a vacuous check is worse than a missing one, because it reports
  reassurance. Classification is recorded **at serve time**, so reclassifying a
  stratum later cannot relabel history.
- `migrations_immutable` now fails on any *unpinned* migration. 0018 would
  otherwise have been unguarded, and editing an unguarded migration is what took
  production down earlier.

### Changed
- **The deployment now serves non-photo content only.** `ssim2-nonphoto` is the
  compiled default and `main` is unlisted, so a visitor who names no study lands
  on the non-photo forced-choice validation and the picker hides itself (one
  listed study means nothing to choose). Validating SSIMULACRA2 as the non-photo
  oracle (imazen/squintly#4) is the live priority, and a judgment spent on a
  photograph is one not spent on it.
  - `main` is unlisted, not deleted: still selectable by id, sessions already on
    it keep working, and its 65/35 mix still matches the pre-registered
    `docs/STUDY.md` §4.2. Flip `unlisted` back when #4 has its data.
  - In code rather than an env var so the intent travels with the repo and is
    covered by tests. `the_resolved_default_study_is_listed` caught the broken
    intermediate state where the default was a study nobody could reach.
  - The e2e harness pins `SQUINTLY_DEFAULT_STUDY=main`, because most specs
    exercise the single-stimulus path and a pairwise-only default with no picker
    cannot reach a rating trial. Two integration tests (`smoke`, `asap_wire`)
    now name their study explicitly rather than inheriting whichever is default.

### Fixed
- **An oversized stimulus could not be panned to its right edge.**
  `inset: 0; margin: auto` only centres a box *smaller* than its container. Once
  the image overflowed, CSS resolved the over-constraint by honouring `left` and
  dumping the excess into `margin-right`, so the image sat flush left while the
  pan limits still assumed a centred crop (± half the overflow). Horizontal
  panning therefore reached only halfway to the right edge, and could drag past
  the left one. Measured on a 3000×2200 source in a 396×487 frame: left gap 0,
  right gap −747, against a correctly centred vertical axis at −176/−176 — which
  is why it looked orientation-dependent, and why square and landscape sources
  (where the overflow is horizontal) were the visible cases. Layers now centre
  on a point (`left/top: 50%` plus a −50% translate), which holds at any size on
  both axes.

### Fixed
- **Switching A/B could show a blank frame or flash.** Two causes, both closed:
  - The response panel unlocked as soon as the *judged* layer arrived, not when
    all of them had. On a pair trial you could press B — or the view switch —
    while B was still on the wire, and a real source is ~9.5 MB, so that is an
    empty viewport rather than a flicker. `all-ready` was computed but nothing
    gated on it. Everything now waits for every variant.
  - `load` only means "decodable". A `visibility: hidden` layer is never
    painted, so the first flip had to decode and rasterise on the spot, costing
    a frame. Layers now await `img.decode()` and hide by **opacity 0 with
    `will-change: opacity`**, giving each its own compositor layer so a swap is
    a compositor property change — no repaint, no reflow, no decode. Exactly 0,
    never 0.001: a faintly visible second variant would composite over the
    stimulus under test, which is worse than any flash. The cost is one GPU
    texture per variant while a trial is mounted (three at most).
  - A flash here is not cosmetic: it injects a visual transient between the two
    pictures being compared, at the instant of comparison.
- **Readiness could be reported early.** A variant finishing between its `load`
  listener being attached and the already-cached sweep satisfied both paths and
  decremented the pending count twice, so a trial marked itself ready while
  another variant was still loading — the exact bug the new gate exists to
  close, through the back door. Settling is now idempotent per layer. Found by
  instrumenting a flaky test rather than guessing: `all-ready` was true while
  layer `a` reported `complete: false, naturalWidth: 0`.
- Two e2e specs waited on `#stimulus` being sized, which is now weaker than the
  app's own gate, so they measured mid-setup (`ratio: NaN`, empty hint). They
  wait on `.viewport.all-ready` and advance by trial-id change instead.
- `trial-input.spec.ts`'s panel-disabled test was **vacuous**: it routed
  `**/api/sources/**`, but references are served from `/api/proxy/source/`, so
  the gate never applied and its `if (loading)` branch never ran. Corrected and
  made unconditional.

### Fixed
- **Multi-touch: lifting one finger while another was down showed the
  original.** The pointer handling tracked a single `pointerId` and ignored any
  pointer that arrived while one was already down, so on a phone — one finger
  left, a second right, lift the first — the release ran the end-of-gesture
  handler even though a finger was still held, and the second finger's own
  release was then discarded because its id no longer matched. Every active
  pointer is tracked now, and the most recent one still held decides what stays
  on screen; the resting view returns only when the last finger lifts.

### Added
- **Pinch to zoom**, snapping onto whole factors. It could not work before: the
  second finger was never admitted, so no gesture with two contacts existed.
  The ladder is walked by finger-distance ratio, so the gesture feels
  continuous while the result never is — a fractional factor would resample the
  stimulus. A second finger arriving no longer swaps the variant, either:
  changing the picture mid-pinch would move the thing being sized.
- **Double tap fits the image at a whole factor** and re-centres. "Fits" can
  only magnify a small stimulus up to the frame, never shrink a large one down,
  because below 1:1 the browser resamples the encode — so an oversized source
  resolves to 1×, which is also the useful answer there ("put me back to the
  start").

### Fixed
- **The blind-spot distance test used the wrong eye, so it could never work.**
  It said "close your right eye" while sweeping the target to the *right* of
  the fixation ×. Each eye's blind spot sits ~12–15° into its **temporal**
  (outer) field — the left eye's to the left, the right eye's to the right — so
  a target on the right viewed with the *left* eye lands in the nasal field,
  where there is no blind spot. The dot could not disappear, the sweep always
  ran to its timeout, and the step silently returned no distance while looking
  like a working feature. Now: close the **left** eye. The dead
  `// wait — dot is positioned via right` calculation left mid-thought beside it
  is gone too.
- **Calibration was not sticky, and Skip destroyed it.** The card screen always
  opened at a fixed slider value, ignoring the stored measurement, and its Skip
  returned nulls that the caller wrote straight over a good calibration — so
  re-entering merely to check could erase it. It now seeds from the stored value
  and Skip preserves it.
- **The e2e specs were never type-checked.** `web/tsconfig.json` includes only
  `src`, so `tsc --noEmit` skipped `e2e/` and `scripts/` entirely. Added
  `tsconfig.e2e.json`, wired into `just ci`; it found four real errors on its
  first run.

### Changed
- **Reopening the app resumes into trials.** A returning observer with a stored
  observer id and profile goes straight back to rating instead of being walked
  through welcome → Begin → calibration → profile again. A new session row is
  started rather than the old one resumed, because conditions are re-captured
  and the screen, lighting or device may have changed — filing new responses
  under stale viewing conditions would be worse than the extra row. The
  interstitial keeps a "start from the beginning instead" link up for as long
  as the session request is in flight.
- **Curator and Calibrate are no longer participant tabs.** Curator is an
  operator tool on an anonymous-participant origin (see CLAUDE.md Known Bugs)
  and is now opt-in per browser via the `#curator` hash. Calibrate is a one-off
  measurement the app remembers, reached from a welcome-screen link that says
  whether it is already done.
- **Magnification is every whole factor 1–8, not 1/2/4/8**, driven by a
  `− N× +` stepper. Integer factors remain non-negotiable — a fractional one
  sizes some source pixels 2 device px and others 3, fabricating structure in a
  study about which structure is real — but there was no reason to skip 3, 5, 6
  and 7.
- **The scroll wheel magnifies, snapping onto whole factors.** Deltas
  accumulate so a high-resolution trackpad does not fly 1× → 8× in one flick
  while a notched wheel still moves one stop per notch.

### Added
- **`buttons` input mode**: left mouse button shows A, right shows B, release
  shows the original — the button decides, not where the pointer is. Desktop
  only (`pointer: fine`); `hold` covers the same idea with a thumb. Recorded as
  `input_mode='buttons'`.
- **The surround is tiled with the variant on screen** (A / B / ORIG). The
  letterbox carried no information while the only persistent cue was a button
  below the frame — in a task entirely about telling two pictures apart,
  knowing which one you are looking at should not need a glance away from it,
  least of all under `hold`/`buttons` where it changes as fast as you can press.
  Deliberately dark and strictly neutral grey: this is the surround of a
  psychovisual stimulus, so raising its luminance would shift local adaptation
  and tinting it would bias colour judgements. The glyph carries the meaning,
  not the colour. It cannot occlude anything — it paints the viewport
  background, beneath the opaque stimulus layer.

### Fixed
- **The better image was always B on pair trials** (this change). `try_pair`
  takes two adjacent rungs from a quality-ascending list as
  `(sorted[i], sorted[i+1])`, so slot B held the higher-quality — and larger —
  encoding on every trial: **60/60 measured against the live deployment**, both
  by quality and by bytes. In a 2AFC asking "which is closer to the original"
  that makes the answer constant, so an observer who notices scores perfectly
  without looking, and every pair response conflates a judgement about quality
  with a preference for a side. Neither the Bradley–Terry fit nor a SROCC
  against a metric can separate the two afterwards.
  - `sampling::counterbalance_pair` randomises the slots, applied at **one**
    choke point in `next_trial` after every path that can build a pair (the
    ASAP override included), so no route can bypass it.
  - `expected_choice` is flipped with the slots. A golden pair whose answer is
    recorded as "a" becomes "b" once the encodings trade places; not flipping it
    would turn counterbalancing into a honeypot that fails every honest
    observer. `"tie"` names no side and is left alone.
  - zenpapers `ch3-5_sampling_screening_cis.md` §4.6 prescribes exactly this —
    a suspected side-biased UI needs "explicit position-counterbalancing" (JPEG
    XL CfP) before per-subject modelling means anything.
  - Verified by reintroducing the bug: the new e2e went red reporting slot B at
    **100%** of trials.
  - **Pair responses recorded before this are not usable for rank agreement.**
    They cannot be distinguished from position preference. 23 responses existed
    live at the time of the fix, most of them single-stimulus.
- **`ssim2-nonphoto` was serving photographs** (this change). The study
  constrained only the trial *mix* (forced choice) and never the *content*, so
  it drew from all 21 canonical strata — 8 of which are photographic. On the
  live corpus (4 sources per stratum) that is roughly **38% of its trials**.
  Nothing looked broken: each one is a valid pairwise judgement, filed under a
  label that says it is about non-photo content, which makes it an answer to a
  question nobody asked. `src/content_class.rs` classifies a source from its
  stratum and `SamplerConfig::content` restricts the draw; the study now
  declares `ContentFilter::NonPhotoOnly`.
  - The restriction covers honeypots and anchors too — an anchor from a
    photographic stratum is still a photo trial.
  - An **unregistered stratum is refused**, not admitted. Defaulting the other
    way would let a stratum added to `build_demo_corpus.py` but not to the Rust
    registry quietly enter the non-photo pool — the same silent mislabelling,
    moved. Failing closed makes it a visible shortage instead.
  - An emptied pool returns a 409 that **names the restriction and counts the
    eligible sources**, rather than the generic "empty manifest or no matching
    codecs" — which would send an operator to inspect the sampler when the
    answer is the corpus.
  - Guards: `content_class::strata_agree_with_the_corpus_builder` (the
    counterpart to the licensing drift guard),
    `studies::studies_claiming_a_content_type_restrict_it` (catches the next
    study added by copy-paste), and `tests/nonphoto_live_manifest.rs`, which
    asserts the classification against the corpus values the live deployment
    actually serves. Verified by reintroducing the bug: both the unit guards and
    the e2e went red, the latter naming the stratum it served.
  - `web/e2e/mock-coefficient.ts` now carries real stratum names instead of
    `corpus: 'test'`. Everything classified as `Unknown` before, which made the
    restriction untestable — and, prior to the filter, hid that the study was
    serving photographs at all.

### Changed
- **Every variant of a trial is preloaded; switching A / B / original is now a
  visibility toggle** (this change). One `<img>` had its `src` rewritten on each
  switch, so every flip re-fetched and re-decoded — and the cost is invisible in
  the test suite (512 px mocks over localhost, ~4 ms) but not in the study:
  measured against the live corpus, a real source is **9.5 MB and 0.33–1.1 s
  cold** from R2. That was being paid on the first flick to the original, i.e.
  exactly when the comparison matters. A/B comparison is a
  same-place-different-picture task, and latency between the two pictures is
  latency the observer has to bridge from memory.
- **The trial screen says when it is loading.** A spinner covers the frame and
  the response panel is disabled until the judged image is painted — answering
  before it appears would record a judgement of something never seen. Time from
  render to paint is recorded as `ui_ready_ms` so it stays separable from
  `dwell_ms`: waiting for a decode is not deliberation.
- The *next* trial is deliberately **not** prefetched. `enhance_pair_with_asap`
  chooses it by expected information gain over the responses so far, so fetching
  it early would pick the next stimulus without the current answer — trading
  measurement efficiency for a saved round trip.

### Added
- **Keyboard control of the whole trial loop.** Letters commit, arrows look,
  digits zoom or rate: `←`/`→` cycle A → B → original, `space` (held) peeks at
  the original, `a`/`b`/`c` answer a pair trial, `1`–`4` rate a single-stimulus
  one, `+`/`−`/`0` magnify, `?` shows a cheatsheet. One deliberate asymmetry:
  digits rate on single trials (matching the numerals on the buttons) and
  magnify on pair trials (where nothing owns them); `+`/`−`/`0` are the mapping
  that never changes meaning.
- **`hold` interaction mode, on every device.** The *original* is what you see
  at rest; press and hold the **left half** of the picture for A, the **right
  half** for B, release to snap back. Faster for spotting a difference, because
  the eye stays fixed and the picture changes under it. Selectable from the
  trial screen; `tap` remains the default.
  - Splitting by *half* rather than by mouse button is what makes it work with a
    thumb. The first cut used left/right buttons and was wrong twice over: it
    was desktop-only on a phone-first study, and it needed the context menu
    suppressed. Halves also land where the labels already are — A left, B right,
    matching the view switch and the answer buttons.
  - The half is decided on press and held for the whole gesture. Re-deciding as
    the pointer moves would fight panning: a drag crossing the midline would
    swap the variant out from under a comparison in progress.
  - No overlay marks the split. The stimulus is never occluded (see the hint-pill
    note in CLAUDE.md), so the affordance is the hint text plus the view switch
    highlighting live as you hold.
- `responses.input_mode`, `keyboard_used`, `ui_ready_ms` (migration 0017;
  `responses.tsv` schema_version 3 → 4). `input_mode` is stored rather than
  inferred because it changes what `reveal_ms_total` measures — under `hold` the
  reference is the resting state, so that column is naturally large. Pooling the
  two without knowing which is which would put two quantities in one column.

### Fixed
- Switching to a variant that had not finished decoding computed pan limits of
  zero and clamped the pan back to centre, losing the observer's place — the one
  thing carrying pan across views exists to prevent. Caught by the existing
  pan-preservation spec on the Z Fold inner display.

### Security
- **`/api/auth/start` is rate limited** (this change), per address *and* per
  client network: a 60 s cooldown, 5 links per address per hour, 20 per network
  per hour, all overridable. Either limit alone is trivially defeated — the
  address cap by cycling through other people's addresses, the network cap by
  spraying one inbox from many sources. Refusals are `429` with `Retry-After`
  and send no mail. Counts come from `auth_tokens`, which already gets a row per
  accepted request, so the request log and the token store cannot disagree.
  Client addresses are stored only as a salted BLAKE3 bucket
  (`SQUINTLY_IP_HASH_SALT`) — an unsalted hash of an IPv4 address is reversible
  by brute force in seconds.
- **Sign-in is open to any address; `SQUINTLY_ADMIN_EMAILS` gates admin
  instead.** A short-lived `SQUINTLY_LOGIN_ALLOWLIST` (never released) gated
  sign-in itself, which had the effect of locking ordinary participants out of
  their own data on a second device — linking an email is a participant
  feature, not a privilege. The allowlist now grants *admin*, where an unset
  variable correctly grants it to nobody.
- **Signing in mints a real session** (migration 0015, `auth_sessions`).
  `auth_verify` previously handed the browser an observer id and nothing else,
  so "signed in" was a client-side claim the server never checked — which is
  why admin could only be a shared bearer token. The session is a second
  32-byte secret in an HttpOnly `SameSite=Lax` cookie, stored hashed like a
  magic-link token. `SameSite=Lax` specifically because `Strict` would drop the
  cookie on the top-level navigation out of the mail client into
  `/api/auth/verify`, i.e. on exactly the hop that establishes it.
- **Curator admin routes accept a signed-in admin** (`curator::require_admin`),
  so a deployment no longer has to keep a shared secret in its environment for
  an operator with a browser to work. `admin_token` became optional on those
  requests — as a required field it made the JSON extractor reject a
  cookie-authenticated call with a 422 before the gate ever ran. Admin is
  resolved from the *current* roster on every request rather than snapshotted
  at sign-in, so removing an address revokes it immediately.
- Added `GET /api/auth/whoami` and `POST /api/auth/signout`.

### Added
- **Participant exclusion disposition** (`src/exclusion.rs`, migration 0016),
  following zenpapers `ch3-5_sampling_screening_cis.md` Ch. 4: §4.4 correlation
  to the per-stimulus mean over *other* observers, and §4.2.1's BT.500
  kurtosis-2 band (`2σ` when `2 ≤ β₂ ≤ 4`, else `√20 σ`). The screens **record**
  a verdict and never delete a rating — §4.2.2 is explicit that hard reject
  "loses all data from rejected subjects" and draws a sharp boundary, which is
  why soft per-subject weighting supersedes it. `responses.tsv` gains
  `observer_disposition`, `observer_r_s`, `observer_outlier_rate` and
  `exclusion_enforced` (schema_version 2 → 3), so one export yields both the
  screened and the unscreened numbers.
  - Default is per study (`Study::exclusion_default`) and overridable with
    `SQUINTLY_EXCLUSION`: on for `main` (un-gated crowd, the regime §4.4's
    sieve is for), off for `ssim2-nonphoto` (§4.6 puts the modelling
    under-identified below ~15 subjects).
  - `insufficient_data` is a distinct verdict from `included`. A solo expert
    has no peers to be an outlier against, so they land there by construction
    rather than being excluded wholesale — no special-casing needed.
  - BT.500's own rejection ratio is marked `[unverified]` in the corpus, so
    `outlier_rate_ceiling` is an explicit configurable rather than a number
    invented here and presented as ITU-R.
- `POSTMARK_API_BASE` overrides the Postmark origin (default unchanged) so
  `tests/auth_rate_limit_and_admin.rs` can assert both directions against a
  local stub. Testing only the refusal would leave an inverted condition
  undetectable, and the magic-link flow previously had no way to be exercised
  at all without sending real mail.

### Fixed
- **Pair trials had no way to see the reference** (this change). The screen asks
  which encode is "closer to original" while `startReveal` was gated behind
  `!isPair` and nothing else reached the source — a preference test wearing a
  reference comparison's label. There is now an A / B / **Original** segmented
  control, and press-and-hold works in both trial types.
- **The A/B indicator was small muted text** in the hint pill — the only cue for
  which stimulus you were looking at, in a task entirely about telling them
  apart. It is a 44px segmented control with a filled active state now.

### Added
- **Integer nearest-neighbour magnification (1× / 2× / 4× / 8×)**. Zoom in only,
  never below 1:1. Nearest-neighbour because interpolation synthesises values
  the codec never produced — smoothing exactly the ringing, blocking and
  banding under test; integer factors because a fractional one makes some
  source pixels cover 2 device px and neighbours 3, a visible beat pattern that
  is not in the encode. `responses.zoom_factor` (migration 0014) records it per
  response: an artefact judged at 4× subtends four times the visual angle.
- **CLAUDE.md claimed "2AFC by default"; the sampler is 65% single-stimulus**
  (37005db9). `SamplerConfig::p_single = 0.65`, which matches the
  pre-registered `docs/STUDY.md` §4.2 mix — the doc line was the error, and it
  had propagated into imazen/squintly#4 as "squintly already defaults to 2AFC …
  so this is the native path". Corrected in CLAUDE.md, methodology.md, DEPLOY.md
  and on the issue.
- **The stimulus was being downscaled to fit, which invalidated the rating**
  (2e386795). `trial.ts` scaled with `Math.min(1, …)`, so any stimulus larger
  than the viewport was resampled by the browser and the observer rated *that*
  instead of the encode — averaging away the artefacts under test, worst on
  high-DPR phones with large sources (~4x shrink on a 304 CSS px cover
  display). Display is now a hard minimum of 1:1 device pixels, with panning to
  explore anything bigger; measured at exactly 1.000 on every trial. Pan is
  preserved across encoded↔reference and A↔B swaps, so the same region is
  compared. Migration 0012 records `pan_count`, `pan_distance_css`,
  `pannable_*` and `visible_*`, because `image_displayed_*` no longer describes
  what was on screen.
- **The trial hint pill covered the stimulus** (82ad86ba). The
  "hold to compare with original" / "tap A or B" pill was absolutely positioned
  inside the viewport, so any image that filled the frame was partly hidden
  behind it — an occluded stimulus is a measurement problem in a psychovisual
  study, not a cosmetic one. It now has its own row below the image. Same
  commit: `.viewport` gets `min-width: 0`, because grid items refuse to shrink
  below their content's min-content width and a real 2400px stimulus therefore
  blew the layout viewport open on a 304 CSS px screen. Both were unreachable
  with 1x1 mock images; `layout.spec.ts` now asserts the geometry directly.
- **The sampler served exactly one codec for every trial** (2661193c).
  `pick_trial` chose a codec with `by_codec.iter().max_by_key(|(_, v)| v.len())`
  over a `BTreeMap`; on a balanced ladder every codec ties and `max_by_key`
  returns the last maximum in key order, so the alphabetically-last codec won
  deterministically — measured 27/27 `libwebp` on imazen-26, with
  `libjpeg-turbo`, `jpegli` and `libavif` never shown. That would have emptied
  every cross-codec comparison in `pareto.tsv`. Codec choice is now random,
  weighted by rung count.
- **Deploys were silently broken for two months** (6a7de807). The Dockerfile
  never `COPY`ed `build.rs`, so `env!("SQUINTLY_BUILD_COMMIT")` failed to
  compile in the container and every `railway up` errored while the 2026-05-07
  image kept serving. `cargo test` / `just ci` build from the working tree
  where `build.rs` exists, and `railway up --detach` exits 0 regardless, so
  nothing surfaced it. Fixed with `COPY build.rs` + a build arg;
  `option_env!(…).unwrap_or("unknown")` so a missing build script degrades
  provenance instead of bricking compilation; a startup `warn!` when the commit
  is unknown; and `just railway-deploy` now depends on `just docker-build`.
- **Curator threshold + preview were broken against the canonical R2 corpus**
  (dc53f846). The bucket sends no `access-control-allow-origin`, so the
  `<img crossOrigin="anonymous">` loads those screens need for canvas readback
  failed outright — measured, not inferred. Candidate bytes now go through a
  same-origin proxy, `GET /api/curator/blob/{sha256}`, which also sniffs the
  real image type (R2 answers `application/octet-stream`). The e2e mock is
  deliberately CORS-less so this can't regress unnoticed.
- **Narrow-viewport layout: page no longer pans sideways / mis-taps on the
  Galaxy Z Fold 7 cover display (304 CSS px)** (830221d, 7f37a607, ef0a9e2b).
  Root cause was mobile Chrome's shrink-to-fit layout viewport: nowrap flex
  rows (curator header, trial header, status row), bare-`1fr` grids (groups,
  pair/rating panels, action row), the closed-details license table, an
  unbreakable blob-URL in an error message, and the thumbnail-strip scroll
  container all propagated min-content widths past the device width, so the
  layout viewport widened and taps landed on the wrong controls (the
  Calibrate tab swallowed the curator exit ×'s taps; `#find-thr` was
  unreachable). Fixes: `flex-wrap: wrap` on header rows, `minmax(0, 1fr)`
  columns, `contain: inline-size` on the credits panel + thumbnail strip,
  `overflow-wrap: anywhere` on URL-bearing lines, title hidden below 340px.
  Measured and documented: `overflow-x: clip` on the root does NOT prevent
  the expansion.
- **Pair-trial buttons rendered "Acloser to original"** — the A/≈/B glyphs
  now stack above their labels like the rating panel (pair-panel flex-column
  styling was missing).
- Stacked (vertical) threshold split below 480 px — side-by-side left each
  panel ~150 CSS px on the cover display; the `min-width: 1600px` rule that
  intended this targeted device px and could never fire.
- e2e harness state moved `/tmp` → `~/tmp`; e2e/dev servers set
  `SQUINTLY_DISABLE_TOWER_MIRROR=1` (new env kill-switch in `main.rs`) so
  wiped-per-run test DBs stop writing snapshots to the Tower NAS.
- Mock coefficient is deliberately **CORS-less**, matching the real R2 bucket.
  It briefly sent `access-control-allow-origin: *` during this cycle, which
  made the canvas paths pass locally while they were broken in production —
  a mock more permissive than production hides the bug it exists to catch.

### Security
- **SSRF in the curator's server-side blob fetches** (25d898b7). `POST
  /api/curator/manifest` is unauthenticated and stores a caller-supplied
  `blob_url_base`, so any endpoint that fetched that URL server-side aimed the
  deployment's egress wherever a stranger pointed it — demonstrated
  unauthenticated against a running server: manifest POST with
  `blob_url_base: http://169.254.169.254`, then `GET /api/curator/blob/{sha}`
  returned the metadata response verbatim. `generate-variant` (also
  unauthenticated) had the same exposure already. `curator::guard_blob_url` now
  gates both: http/https only, `SQUINTLY_BLOB_HOST_ALLOWLIST` when set, and
  every resolved address must be publicly routable. Set the allowlist on any
  public deployment (DEPLOY.md §3); never set
  `SQUINTLY_ALLOW_PRIVATE_BLOB_HOSTS=1` there.

### Added
- **Runtime study selection** (cbdf7945). One deployment hosts several named
  studies (`src/studies.rs`) and observers pick one on the welcome screen:
  `main` (65/35 rating/pairwise, the pre-registered crowd study) and
  `ssim2-nonphoto` (forced choice only, for imazen/squintly#4). The sampler
  config belongs to the study, not the process — an ACR rating and a 2AFC
  judgement are different quantities and must not be pooled. Migration 0013
  adds `sessions.study_id`; `responses.tsv` carries it (appended, with
  `schema_version` 1 → 2) so the studies are separable in analysis. An unknown
  `study_id` returns 400 rather than being coerced. `GET /api/studies` lists
  what's offered.
- **`SQUINTLY_PAIRWISE_ONLY=1` — forced-choice-only trial stream** (37005db9).
  Needed for rank-agreement studies (imazen/squintly#4): SROCC against a metric
  is a 2AFC test and an ACR rating is a different quantity. Strict on purpose —
  `p_single = 0` alone still leaks ratings, because `pick_trial` falls back to a
  single when a source has no non-trivial adjacent pair, and honeypots and
  anchors are themselves single-stimulus and injected ahead of the main draw.
  The flag suppresses all three and 409s instead of degrading. `p_single`,
  `p_honeypot` and `p_anchor` are also env-overridable now
  (`SamplerConfig::from_env`, read once at startup).
- **The rating flow serves real trials** (2661193c, 7c8aea13, ae4aeee8), built
  from the canonical stratified corpus `codec-corpus/imazen-26-png-v3`:
  **84 sources across 21 strata** (21 per size bucket), **2016 encodings**
  across `libjpeg-turbo` / `jpegli` / `libwebp` / `libavif`, low-q-weighted, and
  **52 of 84 sources non-photo** — plots, screenshots, AI imagery, patent scans
  and manuscript scans are first-class strata, which is what
  imazen/squintly#4 needs. Selection reads dimensions out of the filename
  (2639/2639 keys), so only the chosen origins are downloaded, not 15.5 GiB.
  Five `licensing.rs` policies label each trial truthfully.
- **The corpus is hosted on public R2, not baked into the image** (7c8aea13).
  `HttpCoefficient` issues only three GETs and object keys may contain slashes,
  so a public bucket serves the whole store with no server:
  `https://codec-corpus.r2.imazen.org/squintly/demo-corpus/imazen26-v1`. Image
  222 MB → 100 MB, no 121 MB build-context upload per deploy, and swapping the
  corpus is one env var instead of a rebuild. `HttpCoefficient` now preserves a
  path prefix on the base URL (it previously joined `/api/...` with a leading
  slash, which resolves against the origin and silently drops the prefix).
  `scripts/publish_corpus_r2.py` + `just publish-corpus`. See DEPLOY.md §15.
- **`POST /api/curator/candidates/delete`** (dd47a8bd) — admin-gated removal of
  a candidate and its decisions. The pool was previously append-only: manifests
  upsert, and a `reject` is per-`curator_id`, so a bad row surfaced forever for
  everyone else.
- **`e2e/layout.spec.ts`** — per-screen guard across all four device
  projects: layout viewport must equal device width, no horizontal scroll,
  no element painted past the right edge (deliberate horizontal scrollers
  exempt).
- **`web/scripts/ux-audit.ts` + `just audit` / `just audit-serve`** —
  scripted demo user that walks every screen (welcome → calibration →
  profile → trials → curator stream/curate/threshold → suggest) at Z Fold 7
  cover + inner, Pixel 7, and desktop viewports; captures a screenshot per
  screen and reports horizontal overflow, intercepted taps, and sub-40px tap
  targets to `/mnt/v/output/squintly/ux-audit-<date>/`.
- Mock coefficient generates real structured PNGs (rings + gradient +
  checker) with quality-dependent posterization instead of 1×1 blobs, so
  trials and threshold screens exercise visible quality differences.
- 40 px tap targets for the trial menu button and curator exit; credits
  links padded; tab bars shrink gracefully with ellipsis.
- **Curator mode** (`docs/CORPUS_CURATOR_SPEC.md`). New `/api/curator/*` HTTP
  surface for corpus development: `stream/next`, `decision`, `threshold`,
  `progress`, `manifest`, `licenses`, `export.tsv`. Migration
  `0007_curator.sql` adds `curator_candidates`, `curator_decisions`,
  `curator_size_variants`, `curator_thresholds`. Frontend ships three screens
  (Stream / Curate / Threshold) reachable from the welcome tab bar; the
  threshold slider pre-encodes at q ∈ {30, 50, 70, 85, 95} and JIT-encodes
  intermediate values via `OffscreenCanvas.convertToBlob` (encoder identity
  recorded as `encoder_label = 'browser-canvas-jpeg'` until WASM jpegli
  ships). `src/curator.rs` parses both corpus-builder TSV
  (`curated_manifest_*.tsv`) and the unified R2 JSONL manifest emitted by
  `scripts/upload_all.py`. Auto-downscale rule masks size chips against
  detected source-q so a JPEG already at q=70 cannot oversample its baked-in
  quantization. Three integration tests + five Playwright specs covering
  stream → curate → threshold → export round-trip. An opt-in spec
  (`CURATOR_R2_LIVE=1`) hits the live R2 manifest at
  `pub-7c5c57fd3e0842f0b147946928891d40.r2.dev` to validate the production
  data path.
- **License surfacing**. New `src/licensing.rs` registry with seven
  per-corpus policies (Unsplash, Wikimedia, CommonCrawl, Flickr, GitHub
  issues, generated/built, mixed-research fallback). Welcome screen has a
  collapsible "Image sources & licensing" credits panel listing every
  policy's terms URL, redistribution posture, and commercial-training
  posture. Every curator stream/curate screen shows a license badge with
  a deep-link to the canonical terms page. Trial UI displays the corpus
  name + license label inline at the top of every rated trial. Curator
  `export.tsv` carries five license columns (id, label, terms_url,
  attribution_url, redistribute, commercial_training). `TrialPayload`
  carries `source_corpus`, `source_license_id`, `source_license_label`.
- **Galaxy Z Fold 7 layouts**. New `zfold7-cover` (304×772 CSS px portrait,
  DPR 3) and `zfold7-inner` (749×832 CSS px portrait, DPR 2.625) Playwright
  device descriptors. Curator CSS picks up a side-by-side preview layout
  via `@media (min-width: 720px) and (orientation: portrait)` for the inner
  display, and stays single-column on the cover. The threshold split panel
  switches to top/bottom orientation at `min-width: 1600px` for tablet-class
  unfolded screens. Two regression tests assert the layouts.
- `docs/methodology.md` — codifies every methodology choice (stimulus
  presentation, sampling, outlier detection, score construction, scale
  alignment, CIs, sample sizes) with the rationale behind each parameter,
  cited to CID22 / pwcmp / KonIQ / BT.500 / Levitt / Pérez-Ortiz / Meade
  & Craig. Every magic number in the codebase is now a contract here.
- **Monotonicity constraint** in BT pareto export (CID22 §Monotonicity).
  Same-codec pairs get 200 dummy "higher-q wins" opinions injected before
  the BT-Davidson fit. CID22 measured this as the single highest-leverage
  rigor lever — KRCC dropped 0.99 → 0.56 in their dataset without it.
  `bt::with_monotonicity()`, plus an explicit unit test that proves the
  fit pins the ordering against contradictory raw votes.
- **Trivial-triplet filter** in the sampler (CID22 §Selection of stimuli):
  same-codec pairs with quality gap > 30 are foregone; cross-codec pairs
  with byte-ratio > 4× are foregone. Pair sampling skips trivial outcomes
  rather than burning observer attention on them.
- Optional email magic-link sign-in (pattern adapted from Weaver
  `convex/auth.ts`): `migrations/0005_auth.sql` adds `auth_tokens` +
  `observer_aliases`. `src/auth.rs` generates 32-byte cryptographic tokens,
  hex-encodes them, persists only the BLAKE3 hash, 15-min TTL, single use.
  `POST /api/auth/start` calls Resend (`RESEND_API_KEY`,
  `RESEND_FROM_EMAIL` envs); `GET /api/auth/verify?token=…` returns a tiny
  HTML page that writes the resolved observer_id into localStorage and
  redirects. Cross-device sign-in merges via `observer_aliases` so a
  returning observer's existing record always wins. Without
  `RESEND_API_KEY`, `/api/auth/start` returns a 503 with a clear hint —
  anonymous use is unaffected. Frontend: opt-in modal from the welcome
  screen. 4 new e2e tests.
- Welcome copy now leads with "make the web faster"; zensim is the
  mechanism, not the headline.

### Fixed
- Welcome copy + motivation doc had a fabricated "used by Wikipedia" claim.
  Replaced with honest framing; the doc now warns explicitly against
  claiming adopters that don't exist.

### Added (earlier)
- Initial scaffolding: SPEC, README, CLAUDE.md
- Cargo manifest with axum + sqlx + rust-embed + reqwest stack
- Railway deployment: Dockerfile (3-stage Node→Rust→debian:slim),
  `.dockerignore`, `railway.toml` with healthcheck, `DEPLOY.md` walkthrough
  modeled on interleaved's flow, `justfile` shortcuts. Binary auto-honours
  Railway's `PORT` env (binds 0.0.0.0:$PORT) when set.
- Engagement v0.1 footprint: day-streak math (`src/streaks.rs` with weekly
  freeze, milestone crossings), `corpus_themes` + `badges` + `observer_badges`
  tables, `account_tier` / `compensation_mode` / GDPR-consent columns on
  observers, theme picker plumbed through session create.
- `GET /api/observer/{id}/profile` returning streak/total_trials/skill_score/
  badges/themes.
- Playwright e2e suite (`web/e2e/`): mock-coefficient TS server,
  global-setup/teardown, helpers, 14 spec scenarios across welcome /
  calibration / trial-loop / API / codec-filter. Production-shape: real
  release binary embeds the built frontend, runs against a side-channel
  mock coefficient. Two browser projects (`chromium-phone` Pixel 7,
  `chromium-desktop`). 27/28 green (1 conditional skip on the first-trial-
  is-a-pair branch). Justfile gains `e2e-prep` and `e2e` targets.
- `data-trial-id` attribute on the trial container so e2e tests can
  reliably wait for next-trial render after a click (eliminates a race that
  surfaced in long rating loops).
- Startup is non-fatal on unreachable coefficient: log a warning, start
  with an empty manifest, expose `POST /api/manifest/refresh` for retry.
  Lets Railway's healthcheck pass even before coefficient is up.
- Codec support detection: `web/src/codec-probe.ts` runs 1×1 base64 decode
  probes for JXL/AVIF/WebP at session start (cached 7 days in localStorage),
  posts the supported set with `POST /api/session`. Sampler (`pick_trial`)
  filters trials to encodings whose codec family the observer can natively
  decode — never transcode-to-PNG, since that would compromise the perceptual
  measurement. New `migrations/0004_codec_support.sql` adds
  `sessions.supported_codecs` (CSV) + `codec_probe_cached` flag. Welcome
  screen surfaces a `chrome://flags/#enable-jxl-image-format` hint to
  Chromium observers when JXL isn't detected; Firefox and Safari get
  honest "we'll skip JXL trials" copy. 3 new sampler unit tests.
- `docs/motivation-and-compensation.md` — playbook citing Galaxy Zoo
  motivations (39.8% research-impact primary), Eyal et al. 2023 (Prolific
  vs MTurk: 67.94% vs 26.40% high-quality), AAAI volunteer-vs-paid 92% vs
  78% accuracy, Duolingo streak-freeze -21% churn, 90-9-1 participation
  inequality. Recommends volunteer-mode-by-default + charity-mode in v0.3 +
  Prolific only for cohort completion. Never MTurk.
- Participant grading & outlier management (v0.1 inline + session-end scope):
  - `migrations/0002_grading.sql` — observers/sessions/trials/responses columns
    + `observer_grades` table
  - `src/grading.rs` — per-trial flags (rt floor, no_reveal, golden_fail,
    viewport_clipped) and session-end composite grade (geometric mean of
    golden_pass_rate, KonIQ line-clicker ratio, RT-floor frac, even-odd Spearman,
    no-reveal frac → A/B/C/D/F)
  - `docs/participant-grading.md` — methodology playbook citing BT.500-14 §A.1,
    pwcmp, Pérez-Ortiz 2017/2019, CID22, KonIQ-10k, Meade & Craig 2012
