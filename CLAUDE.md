# Squintly — agent notes

Browser-based psychovisual data collection for zensim. See [SPEC.md](SPEC.md) for the
design and [README.md](README.md) for the elevator pitch.

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
