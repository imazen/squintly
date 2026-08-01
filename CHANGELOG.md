# Changelog

## [Unreleased]

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
