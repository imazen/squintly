# Paid phone study: current priorities and agent work packet

Status on 2026-09-15: **NOT READY for full paid collection.** This is an
implementation and research handoff, not a claim that the instrument or sample
size has been validated. No workers have been recruited or paid by this task.

This document takes precedence over conflicting earlier Squintly study plans.
The corresponding model priorities are in
[zensim's production plan](https://github.com/imazen/zensim/blob/main/docs/PRODUCTION_PRIORITIES_2026-09-15.md).
The immediate objective is a trustworthy **phone, SDR, reference-based comparison
with integer, unsmoothed inspection zoom**, followed by a bounded usability
pilot. Native HDR is a separate qualification track. Preparing that track must
not delay the first properly scoped SDR study or zensim's automated work.

## 1. What is known, and what is historical

Source audit: Squintly `502205641d287c79a4aa69d73e77cdd596996c97`, fetched on
2026-09-15; latest main code is dated September 1. This is a source audit, not
verification of the currently hosted binary, database, phone screens, or exports.

| Finding | Evidence and implication |
|---|---|
| Old adjudication is for old candidates and one observer | Zensim's `benchmarks/squintly_adjudication_protocol_2026-09-01.md` names September 1 models and a 2,536-row, ten-hour plan. These counts are not a power calculation for a paid cohort or current Rev3 models. Keep historical manifests immutable; create a new study/protocol ID. |
| Integer zoom already exists | `web/src/trial.ts::sizeLayer`, `applyZoom`, `ensureCovers`, pinch handler; CSS uses `image-rendering: pixelated`. Shared pan and size keep reference/A/B aligned. Do not reimplement the viewer. Verify actual raster placement and phone compositing. |
| Answer choices contradict the old no-ties text | Pair trials offer A / can't tell / B, including keyboard ties and a delayed tie hint. Make the new protocol match a deliberate, study-specific choice. |
| Reference exposure is not required | `requiredViews()` requires A and B only for pairs. A reference-based task needs an observed-reference gate, with exposure logged, before submission. |
| Each observer starts the same ordered queue | `src/pair_manifest.rs::next_pair` excludes rows served to that observer, then orders by `seq`. Other observers do not consume each other's rows. A cohort would repeatedly cover the same prefix without deliberate allocation. |
| Served is not answered | The same query excludes abandoned served trials. Record assignment, delivery, answer, skip, timeout and abandonment separately; define resumption and reassignment. |
| Break support is incomplete | Backend progress exists; the frontend does not consume `study-pairs/progress` or `break_recommended`. A document's prescribed break does not implement one. |
| Ingest can leave unresolved images | Require zero unresolved stimuli, successful actual fetch/decode, and a byte/pixel provenance check before any paid assignment. |
| Human data and app state remain unaudited here | The September 1 staged stimulus directory is not proof of zero current judgments, successful collection, or the live app's commit. Audit metadata/counts before reusing any study; do not inspect sealed labels. |

The September 15 zensim candidates are frozen recovery models, **not production
qualified**. Human time cannot fix codec bounds, broken color ingress, missing
provenance, incomplete gauntlet computation or known software failures. Existing
AIC3/AIC4/JPEG-AI-SDR human results share a source family and are holdout-only;
they are not three independent training corpora. Do not fit a new model to those
labels or to repeatedly inspected evaluation residuals.

## 2. Scientific question and the worker's answer

The first instrument measures: **given a reference, which decoded image is
closer in appearance during phone inspection with optional unsmoothed integer
zoom?** This is an inspection condition. It does not by itself establish quality
at native scale, normal fit-to-screen browsing, or all phone viewing conditions.

The recommended first paid batch is **TRAIN/development labels for nearby-quality
ordering and cross-codec disagreements on newly admitted TRAIN source families**.
This supplies information that an SSIM2-oracle training set cannot independently
provide. It is not a paid vote to pass the currently frozen candidates. A later
confirmatory study needs a frozen candidate and a separately held source split.

Use A / B / can't tell for perceptual comparisons; record a separate technical
problem/skip action. A tie means insufficient perceptual distinction, not a
broken image. Do not force ties into A/B or silently discard them. Hide models,
codec names, metric scores, quality knobs and byte counts. Do not ask workers to
invent a zensim score, certify an encoder bug, or guess which output is cheaper.

Three roles have different jobs:

1. **Engineering operator:** validates image lineage, codecs, color, display
   behavior, task operation, assignment and exports. Reproduces known software
   failures using logs/decoders. This happens before ordinary collection.
2. **Study designer/analyst:** fixes the question, source split, sampling,
   observer allocation, analysis, precision target and payment/time cap before
   collecting the corresponding labels.
3. **Paid observer:** checks the onboarding examples, looks at reference/A/B,
   pans/zooms as instructed and gives a perceptual answer. A separate, explicitly
   designed defect triage module may ask about visible damage; it cannot assign
   software causality or decide the corruption head's product policy.

If a native-scale result is needed, register a separate fixed-1x block and
counterbalance block order. Crops, allowed zoom, pan freedom, exposure and
instructions are parts of the condition. Do not infer a causal zoom effect from
people choosing to zoom on difficult trials. Zoom-conditioned observational
reports are useful, but optional zoom is not randomized treatment.

## 3. Phone pixels and color: implementation contract

For source width `W`, integer zoom `k >= 1`, and effective raster density `d`,
the intended CSS width is `W*k/d`. At 1x each source sample covers one raster
pixel; at 2x it covers a 2x2 block. A 256-pixel image at DPR 3 therefore needs
`256/3` CSS pixels at 1x, not an invented higher-resolution source. Also align
the rendered origin and pan translation to the raster grid. Correct width alone
does not prevent half-pixel translation caused by centering an odd-size image.

The intended result is **no spatial interpolation**. Color management is still
required; preserving source RGB code values through an ICC conversion is not
the goal. Browser raster pixels are not proof of identical physical RGB
subpixels on every panel. Browser/OS scale changes and device composition can
break an assumed DPR mapping, so qualification must test real devices.

At least one supported iPhone/Safari and Android/Chrome configuration must pass
the actual phone tests before claiming cross-platform support. Include fractional
DPR cases in automated geometry tests. Admit only configurations with evidence;
do not turn a browser user-agent allowlist into a calibration certificate.

Required tests and recorded conditions:

- One-pixel checkerboard, single-pixel impulses, colored edges, odd dimensions,
  off-center crops and real images at every allowed integer factor; test initial
  cover zoom, pan, pinch, orientation change, browser toolbar changes and resume.
  Check no intermediate spatial colors in the rendered raster at integer zoom,
  apart from the independently characterized color transformation. Check matching
  reference/A/B location, no seams, no stale frame and no downscaling.
- Test the actual touch interaction and physical phones, not only desktop mobile
  emulation. Use app zoom; detect browser visual-viewport scaling and either
  restore/requalify or mark the trial invalid. Never claim that a viewport meta
  tag alone prevents browser/accessibility zoom.
- Store study/condition version, device/browser/OS, DPR, viewport scale, source
  dimensions, crop and pan, zoom history or exposure by factor, reference/A/B
  exposure, foreground state, orientation, input mode and image load errors.
  Existing final `zoom_factor` alone cannot reconstruct mixed-scale viewing.
- Preserve one neutral surround and identical presentation paths. No overlays on
  stimuli. Pause on backgrounding, color-mode changes or an interrupted session;
  record the reason rather than treating elapsed wall time as exposure.
- Ask the observer to use the registered orientation, normal viewing distance,
  stable indoor lighting without glare, stable comfortable brightness, and to
  disable automatic brightness/color-temperature/night modes for the session.
  Record self-report as self-report. It does not measure luminance or vision.

For geometry, if a physical display pixel has pitch `p` at distance `D`, its
angular size is approximately `p/D` radians. Source pixels per degree are
approximately `D*tan(1 degree)/(k*p)`. Magnification reduces source pixels per
degree and changes visibility even without smoothing. Higher phone pixel density
does not universally make a given artifact more visible; source scale, distance,
panel, contrast and observer all matter.

### SDR and wide gamut

Start with a declared, tagged **sRGB SDR display-stimulus condition**, using a
single canonical Rust decode/color route for reference and distorted images.
Retain source ICC/CICP, resolved transfer/primaries/range, bit depth, alpha
handling, decode/conversion version and output hashes. A same-profile RGB8 pair
is a special case, not a license to ignore mismatched profiles or high bit depth.

Display lossless, identically tagged decoded stimuli if the aim is to assess
encoder reconstruction independently of phone decoder support. Record that
transformation and its limits. A gamut/bit-depth conversion may change the
scientific stimulus; do not silently attach native HDR/WCG labels or original
pixel-model scores to its SDR version. Score the admitted image path through
zensim's public API and identify which representation each score describes.
Native browser-codec behavior is a separately labeled product-path arm.

Wide-gamut SDR and native high-bit-depth claims require their own qualified
display/rendering cells. Do not flatten Display P3 into sRGB and call it native
P3 evaluation. Use `zenmetrics` and the existing zen color/codec owners; do not
extend the historical duplicate `squintly-score` decoder/scorer.

### Native HDR: separate, not yet ready

`dynamic-range: high`, an HDR-capable phone model, metadata, or a screenshot is
insufficient proof of displayed native HDR. A gain-map file's SDR fallback is a
different rendition, not an HDR observation. PQ/HLG, gain-map reconstruction and
tone-mapped SDR must be distinguishable in manifests and exports.

Before a paid HDR cell, an engineering operator must qualify its concrete
device/OS/browser/display settings and reference/A/B transport: resolved
primaries, transfer, range, physical luminance interpretation, bit depth, ICC/CICP
agreement, gain-map behavior when used, gamut mapping, clipping/quantization,
tone mapping, SDR white and HDR peak. Verify the actual physical display with
appropriate calibrated measurement or an already characterized measurement setup,
including brightness/thermal behavior and adaptation when switching layers.
Store the measurement receipt and allowed operating conditions. A worker looking
at a highlight chart cannot certify absolute nits or a PQ transfer curve.

Use native Rust paths and existing `zenmetrics score-pairs --hdr` infrastructure
for stimulus/scoring provenance. The corrected September 15 BT.709-native judge
contract does not itself qualify a P3 phone display. For HDR-to-SDR experiments,
name the tone mapper and display condition and analyze them separately. Do not
pool their votes into native HDR labels. Lab AIC-HDR methods inform this design;
they do not validate arbitrary unmeasured phones.

## 4. Allocation, labels and analysis before collection

Create an immutable new protocol, candidate hash list, stimulus manifest and
split manifest. Register the first batch as **TRAIN/development** before mining;
any later confirmatory experiment gets a separate registration. Assign entire source families, including
crops, HDR/SDR renditions and derivatives, together. Recheck the old crop/source
audit; its partial visual review is not proof of zero overlap. No secret holdout
access. A visible development panel is never subsequently called untouched test.

For TRAIN, make rank-conflict and nearby-quality samples on TRAIN sources. Keep
a probability-sampled coverage arm as well as targeted disagreements, and retain
selection probabilities/strata. For confirmation, freeze the candidate and
protocol before opening labels and use an independently held source partition.
Do not report an unweighted disagreement-enriched sample as population accuracy.

Use codec-specific attainable ranges fixed before scoring-model steering tests;
codec knob numbers are not shared perceptual bands. Include same-codec nearby
quality and cross-codec comparisons, content strata, low-quality honest outputs,
and known visible failures as distinct purposes. Corruption activation remains a
software-failure decision separate from ordinary low-quality distortion.

Implement balanced incomplete blocks with recorded random seeds, coverage goals
across independent observers and sources, balanced A/B slots, separated hidden
repeats and a connected comparison graph for each scale reconstruction. Randomize
order while respecting these constraints. Don't simply clone one 2,536-row queue
per worker. Preserve actual encoding identity when counterbalancing and scoring
repeats. Define dropped/abandoned trial allocation and prevent duplicate submission.

Set the primary comparison and practical effect/precision target before full
collection. Model source and observer dependence; resample/model both levels,
report pair coverage, ties, exclusions, fatigue and device strata. Specify a
tie-aware likelihood or a separate tie analysis, and its sensitivity analysis;
do not silently interpret ties as half a vote. Repeated responses are not
independent new observers. Correct multiplicity for confirmatory comparisons.
Avoid single-row IID McNemar tests for a clustered repeated-observer design.

Hidden repeats estimate repeatability; that is not a universal numerical ceiling
on every correlation metric. Inspect uncertainty and lapse models. Goldens must
test understood instructions or independently established obvious differences,
not agreement with the metric being evaluated. Disputed pairs are not goldens.
Do not penalize workers for disagreeing with models or expose hidden controls.

## 5. Ordered work packet for the next agent

1. **Register and audit.** Read this document, `CLAUDE.md`, zensim's current
   production plan and the research below. Pin repo/build/stimulus identities;
   inventory actual hosted study metadata and response counts without reading
   sealed responses. Write `protocol.json`, `stimuli` manifest, `splits` manifest,
   `analysis-plan.md` and `readiness.json` under one new versioned study authority.
2. **Fix existing owners.** In `src/studies.rs` and `src/pair_manifest.rs`, add
   the new study/condition and cohort allocation. In `web/src/trial.ts` and
   `hold-stack.ts`, enforce reference exposure, qualified integer pixel zoom,
   stable A/B geometry, explicit problem/skip, breaks and condition logging.
   Extend existing migrations, exports and tests rather than making another app.
3. **Build a reproducible receipt.** Exercise every manifest image through actual
   fetch/decode; require zero unresolved images. Reconcile ingest → assignment →
   trial → response → export by stable IDs, including reversed slots, repeats,
   ties, skips, reloads, interruption and incomplete sessions. Verify metric
   information remains invisible to ordinary observers. Confirm the served
   binary embeds the tested frontend and reports the pinned build commit.
4. **Test without paid collection.** Run focused Rust tests and `npm run
   typecheck` in `web`, then affected Playwright tests via the existing built
   app/mock-coefficient harness. Test at least two observers, partial completion,
   resume and independent assignment coverage. Conduct the physical-phone checks
   in section 3. Record failures, don't replace them with emulation screenshots.
5. **Bounded pilot only after engineering gates pass.** Proposed cap: five
   distinct observers, one session each, at most 20 minutes including onboarding
   and required breaks (100 observer-minutes maximum). This is a usability/timing
   pilot, not a powered scientific comparison or an authorization to spend.
   Proposed time budget: up to five minutes onboarding, two comparison blocks
   of at most six minutes with a two-minute break between, and one minute to
   finish/report problems. These are pilot design choices, not literature-derived
   guarantees. Stop at the time cap even if assigned comparisons remain.
   Balance supported phone conditions when feasible; name untested conditions.
   Keep pilot labels development-only.
6. **Calculate the full study before buying it.** Use pilot timing, attrition,
   tie rate, repeatability and clustered variance to simulate the registered
   decision/precision target. Five observers cannot reliably estimate all crossed
   source/observer variance components: use conservative variance and attrition
   ranges and sensitivity simulations, not a precise power claim from this pilot.
   If uncertainty requires a larger pilot, publish its revised cap and cost before
   purchasing that extension. Produce unique observers/sources/pairs, overlap,
   expected accepted ratings, exclusion allowance, time, compensation and total
   cost including required onboarding/breaks and provider fees. Fix a stopping
   rule and maximum budget. If the budget cannot answer the question, narrow
   the question before collection. Do not extend opportunistically until a model
   wins. No claim of 100% statistical certainty or guaranteed sample size.

Definition of done for this agent assignment: reviewable code/tests and an
evidence-linked readiness packet, a phone demonstration matching the exact
worker brief below, and an explicit **ready / not ready / unsupported** result
for each registered SDR/WCG/HDR condition. Full collection can start only when
its required receipts and bounded cost/analysis plan exist. A green app test
does not substitute for study validity or actual display qualification.

## 6. Worker brief to implement and test

This is the intended participant text. It must match the completed UI before use;
do not send it to workers while controls or instructions differ.

> You will compare two versions of the same picture with an original reference.
> Choose the version that looks closer to the original. Choose “can't tell” when
> neither looks closer to you; there is no need to guess.
>
> Use the supported phone and browser shown in your invitation. Follow the setup
> check for screen orientation, brightness and color settings. Sit comfortably
> indoors without screen glare. Use your normal glasses if you need them.
>
> First look at the original and both A and B. You can switch between them while
> staying on the same part of the picture. Drag to inspect another area. Use the
> study's pinch/zoom controls to enlarge pixels in whole steps; the blocky edges
> when enlarged are intentional. Do not use browser/page zoom. The zoom and view
> stay matched across all three images. You do not need to use maximum zoom.
>
> Choose A, B, or “can't tell” based on the picture, not speed. If an image fails
> to load, controls misbehave, or your viewing conditions change, use the problem
> or pause control. Do not answer “can't tell” to report a technical problem.
>
> Take the breaks the study offers and stop if your eyes are uncomfortable.
> Your invitation states the total time, payment, and how to finish or stop early.
> Agreement with a scoring model is not a condition of payment.

Operator must fill the invitation from the registered protocol, including actual
supported devices, time cap, break schedule, pay and technical-failure handling.
Keep instructions about codecs, bugs, metrics and statistical analysis out of the
participant task. A technician, not the worker, owns any HDR calibration.

## 7. Research read, and limits of transferring it

Read the local research collection in `zen/zenpapers`:

- `docs/iqa-methods/reference-book/ch11_mobile_specific.md`: geometry, phone
  interactions and color confounds. It is a research synthesis, not a device
  qualification report. Use the explicit derivation in section 3; do not copy
  its overbroad claims about intrinsic CSS sizing or universal phone visibility.
- `ch3-5_sampling_screening_cis.md` and `ch10_human_eval_collection.md` in the
  same directory: allocation, observer models, screening, intervals and fatigue.
  Old platform rates and generic participant-count rules are not this study's
  current budget or power calculation.
- `docs/iqa-methods/subjective-scaling-jod.md`: reference comparisons and scale
  reconstruction. Verify scale/noise conventions against the primary model
  before implementing formulas; a per-stimulus standard deviation differs from
  a difference-distribution standard deviation. No fixed number of SSIM2 points
  is a universal human JND, and neither JOD nor JND means the worker supplies a
  product target score.

[Testolina et al., fine-grained high-fidelity assessment](https://arxiv.org/html/2410.09501v1)
distinguishes plain and boosted observations and calibrates their relationship.
Its reference-comparison design informs this work; our phone, optional integer
zoom and tie condition is a declared adaptation, not an exact PTC/BTC replication.
Do not use boosted-image human labels as if collected on unboosted native pixels.

[Jenadeleh et al., AIC-HDR2025](https://arxiv.org/html/2506.12505v1)
uses controlled HDR labs and explicitly tagged HDR stimuli. The local paper is
`/mnt/v/input/papers/03/031d1417a29c6caaef9b32935fac8490aeb32e499920c014d2fe82458c9dd9b4.md`.
Its display and adaptation discipline motivates the separate physical-phone HDR
qualification; its laboratory results do not establish remote phone equivalence.

[CSS Images 3, image rendering](https://www.w3.org/TR/css-images-3/#the-image-rendering)
specifies scaling behavior; `pixelated` can involve a final smooth rescale at
noninteger dimensions. Integer geometry plus actual rendering tests are required.
That specification is not a guarantee of a phone's final physical panel output.
