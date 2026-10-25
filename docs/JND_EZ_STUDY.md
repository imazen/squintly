# jnd-ez: a careful-expert JND from one to three phone observers (design, 2026-10-24)

Status: **design only.** Nothing is registered or collected, and squintly does not run this study yet (see "What
squintly needs"). This is a separate study from the paid crowd plan in
[STUDY_READINESS_2026-09-15.md](STUDY_READINESS_2026-09-15.md). It follows that document's phone pixel and colour
contract (§3), its tie rule (§4) and its holdout rule for AIC data (§1).

## What it measures

**jnd-ez** (working name, for "expert, zoomed") is the distortion a careful observer picks out 75% of the time in a
forced choice. The observer inspects the picture on a phone at whole-step zoom (1–8×, no smoothing). They switch
between the original and a candidate by press and hold, so the observer controls the flicker. Each trial takes as
long as the observer wants.

- **The unit.** One jnd-ez is 75% correct: Thurstone Case V, `d = Φ⁻¹(P) / Φ⁻¹(0.75)`. That is the same unit AIC-3
  uses, so jnd-ez differs from AIC-3's JND only in observers and viewing.
- **The stimuli.** Real decoded pixels only. AIC-3's boosted trials amplified the artifacts; jnd-ez does not, so it
  describes the images people actually receive.
- **Two planned companion units.** jnd-ctc would cover committee viewing conditions and jnd-typ a typical crowd.
  Neither name is settled. Neither unit is part of this design, beyond an optional block that helps link them.
- **What one to three observers can and cannot tell us.** They measure those observers, not experts in general.
  Every observer runs the same anchor block on AIC-3's public stimuli. That ties each observer's unit to AIC-3's
  boosted scale, which comes from 429 screened crowd workers. A later observer can then be placed on the same
  unit.

## The task

- **What the observer sees.** The squintly triplet in `hold` mode. The original shows at rest. Holding the left
  half shows A, holding the right half shows B, and releasing returns to the original. Zoom and pan stay matched
  across all three.
- **Detection trials** make up most of the study. One of A and B is the original itself, decoded through the same
  path, and the other is the candidate. The side is random and balanced.
- **The question** follows AIC-3's triplet question (which side differs more from the reference): "Which one differs
  from the original?" The optional suprathreshold trials, where A and B are both candidates, ask which one differs
  more.
- **One tap gives the answer and its confidence:** A, A?, B? or B, where "?" means a guess. There is no "can't
  tell" button and no hint pointing at one.
  - Detection is threshold psychophysics, and a forced choice keeps each observer's criterion for "can't tell" out
    of the unit.
  - Ties are still analysed in the open (readiness §4). A second fit counts every "?" answer half and half, as
    AIC does with "not sure", and both fits are reported.
- **Zoom.** The existing integer ladder, 1–8×. Observers hold the phone at their normal reading distance and zoom
  rather than bring it closer. Pixels per degree then follow from the zoom and the calibrated distance (readiness §3:
  `D·tan(1°)/(k·p)`).
  - For example, a 460 ppi panel at 30 cm puts about 95 source pixels in a degree at 1×, 47 at 2×, 24 at 4× and 12 at
    8×.
  - For comparison, a 27-inch monitor 2,560 pixels wide, seen from 60 cm, gives about 45. A phone at 1× shows each
    pixel at about half that angle, and zoom is how an expert makes up the difference.
- **Sessions.** Blocks of 120 answers and at most two blocks per session, with a break between them. This is
  squintly's existing AIC-HDR2025 cap (`Progress::break_recommended`).
- **Ladders are interleaved inside a block**, and anchor and zen ladders are mixed in every session. Learning across
  sessions then affects both kinds alike.

## Phones and viewing

- **Each observer's phone is qualified before its first labelled block** (readiness §3). The checks are one-pixel
  checkerboards, impulses, odd sizes, every zoom factor, pan, orientation and toolbar changes. A phone or browser that
  has not passed is not used.
- **Settings.** Fixed brightness. Auto-brightness, True Tone and Night Shift (or their equivalents) off. Indoors,
  without glare. Each session records these as self-report.
- **Stimuli** are sRGB PNGs decoded through one Rust route, with the same presentation path for the original and
  the candidates.
  - All three layers are decoded before the trial unlocks. A byte-identical copy of the original must not betray
    itself with a load flash.
  - The null trials below test exactly that.

## Controls in every block

- **Null trials, about 3%.** A and B are both the original, and one is labelled the candidate at random. Accuracy has
  to stay at 50% (binomial test per observer). Above chance means something other than the picture gives the answer
  away. Collection stops until that is fixed.
- **Catch trials, about 3%.** The candidate is far above threshold. Misses measure the lapse rate that the model
  uses.
- **Repeated ladders.** Two anchor ladders are run again in later sessions. Their thresholds measure test–retest
  agreement and any drift from learning.
- **Practice.** One block of practice ladders comes before the first labelled block. It is not analysed.

## Stimuli

1. **Anchor block: AIC-3's plain stimuli.** These come from the JPEG AIC-3 BTC/PTC dataset (CC BY 4.0; cite Testolina
   et al., [arXiv:2410.09501](https://arxiv.org/abs/2410.09501)).
   - Sources 00002, 00006, 00007, 00009 and 00010 × JPEG, JPEG 2000, JPEG XL, AVIF and VVC, at levels 0–10.
   - The crops are 620×800. Use the PTC images: the BTC images are the boosted versions.
   - **Holdout.** Answers on these images set and tie the unit and support evaluation. They never train or tune a
     metric.
2. **Zen block: zenjpeg, zenwebp, zenjxl and zenavif on imazen-26 TRAIN sources.**
   - The sources are train picks from `variant-sets/squintly-candidates@2026-09-22`, stratified into photo,
     screenshot or UI, illustration or line art, text or document, and mixed.
   - Each ladder is one source and one codec. Its modes are fixed and recorded, and its quality grid is dense: step 1
     in the codec's quality knob.
   - Each grid runs from the codec's maximum down to a quality clearly past 3 jnd-ez. Every codec's range is fixed
     before collection, starting from the AIC-3 settings below and checked against zensim.
   - Encoding goes through zenmetrics sweep and zencodecs, never squintly's Python builder (DEPLOY.md §15), and
     decoding uses the anchor route.
   - These are TRAIN/development labels. A confirmatory claim needs its own block on held-out sources.
3. **Optional: a 1× block on 10 anchor ladders.** Zoom is locked at 1× and the block order is counterbalanced.
   - It separates the effect of zoom from the effect of attention (readiness §2: a fixed-1× block, registered on its
     own).
   - It also gives the first link toward jnd-ctc and jnd-typ.

For where to place the zen grids, AIC-3's 2023 encodes reach 1 and 3 boosted AIC-3 JND at these settings:

| boosted JND | JPEG | JPEG 2000 | JPEG XL | AVIF quantizer | VVC QP |
|--:|--:|--:|--:|--:|--:|
| 1 | q84 | q82 | q82 | 16 | 23 |
| 3 | q53 | q50 (4 of 5 sources) | q59 | 29 | 29 |

These come from zenpapers' refit of the public AIC-3 answers (medians over the five sources). Today's zen encoders
and these observers will differ.

## Choosing trials: adaptive, per ladder

- **One ladder is one source × one codec × one observer.** The server keeps a psi-method posterior for each ladder
  and serves the level that minimises the expected posterior entropy.
- **The model is a two-parameter curve:** `d(x) = (x/α)^β` and `P(correct) = λ/2 + (1−λ)·Φ(0.6745·d)`.
  - `x` is the level counted from the original. For an AIC-3 ladder that is levels 1–10. For a zen ladder it is the
    position on the quality grid, starting at the top.
  - The lapse rate λ is 0.02.
  - The priors are log-uniform: α over the grid and β over 0.6–4.
- **A ladder stops at 80 answers.**

## Trial budget (simulated)

Simulations for one observer and one ladder, with the AIC-3 boosted refit's level values as the true scale (code:
`scripts/jnd_ez/`, `just jnd-ez-budget`; outputs:
[benchmarks/jnd_ez_budget_2026-10-24/](../benchmarks/jnd_ez_budget_2026-10-24/)).

Precision is the error in jnd-ez at the level where the observer reaches 1 jnd-ez. It is reported as a standard error
for the fixed designs and as an RMSE over 200 runs for the adaptive one.

| design | answers per ladder | error at 1 jnd-ez |
|---|--:|--:|
| A free value per level, pairs to the original plus neighbour and skip-one pairs (27 pairs) | 400 / 800 | 0.31 / 0.19 |
| A smooth 3-parameter curve, pairs to the original only, fixed allocation | 200 / 400 | 0.18 / 0.14 |
| Psi, dense grid, truth not a power law | 80 / 120 / 200 | 0.31–0.34 / 0.32 / 0.26–0.28 |
| Psi, AIC-3's ten levels, truth not a power law | 80 / 120 / 200 | 0.34–0.37 / 0.27–0.29 / 0.24–0.25 |
| Psi, dense grid, power-law truth (model exactly right) | 80 / 120 / 200 | 0.27–0.28 / 0.27 / 0.22–0.23 |

The psi rows give the range over two observer sensitivities: the boosted scale itself, and twice as sensitive.

- **Use about 80 answers per ladder.** Beyond that, extra answers help little, so spend them on more ladders.
  - Pooling five sources per codec divides a ladder's error by about √5, if the ladders are independent.
  - Images differ by about as much as one ladder's error. At the setting where the median source reaches 1 boosted
    JND, AIC-3's five sources spread by 0.17 (JPEG) to 0.43 (AVIF) boosted JND (standard deviation; measured on the
    refit table).
  - A codec's target therefore needs several images either way. More answers per ladder shrink only the measurement
    part of the error; more images shrink both parts.
- **Fixed allocation does as well as psi or better here, but only because its levels were known to span the
  threshold.** The smooth-curve row spreads answers over all ten AIC-3 levels and uses exactly the right curve.
  - An observer twice as sensitive as the boosted scale passes 3 jnd-ez by level 4, which would leave most of a
    fixed ladder's answers in saturation.
  - A zen grid with dozens of rungs gives no such span to start from.
  - Psi finds the range by itself.
- **At 0.5 jnd-ez**, psi's error is 0.18–0.29 at every count tested.
- **At 2 jnd-ez** it is 0.45–0.62 at 80 answers and 0.29–0.37 at 200. Where the 2-jnd-ez point matters, the ladder
  gets about 200 answers.
- **Answers carry information only up to about 3 jnd-ez.** At d = 3 a careful observer is right 98% of the time,
  before lapses, so detection trials learn almost nothing there. Values above 3 need comparisons between two
  candidates, as in AIC-3's suprathreshold pairs, or a crosswalk.

Per observer:

| block | ladders | answers |
|---|--:|--:|
| practice | 2 | 120, not analysed |
| anchors, minimal | 10 (5 codecs × 2 sources) | 800 |
| anchors, full | 25 | 2,000 |
| zen, minimal | 24 (4 codecs × 6 sources) | 1,920 |
| zen, full | 48 (4 codecs × 12 sources) | 3,840 |
| 1× block (optional) | 10 | 800 |
| null and catch trials | | +6% |

- **The minimal plan** is about 3,000 answers: 25 blocks of 120, or 13 sessions of two blocks.
- **The full plan for one observer** is about 6,300 answers without the 1× block: 53 blocks, or 27 sessions.
- **With three observers**, everyone runs the anchors. The zen ladders are split between them, and 8 ladders are
  run by all three to estimate each observer's scale factor.
- **Time.** Hours are answers × seconds per answer ÷ 3600. Seconds per answer is not yet known for this task. The
  practice session measures it from `dwell_ms`, which squintly already logs.

## Analysis plan

1. **Instrument checks, per observer.** Null-trial accuracy against 50%, and the lapse rate from the catch trials.
2. **Per ladder.** From the posterior, the level at 0.5, 1 and 2 jnd-ez, with 95% intervals.
3. **Pooled scale.** Ladders run by more than one observer share a curve, scaled for each observer (`d_o = s_o·d`).
   - jnd-ez is set at the geometric-mean observer.
   - The most sensitive observer is reported too, since "visually lossless under inspection" is that observer's
     threshold.
4. **Crosswalk to AIC-3**, per codec.
   - Fit `jnd-ez = a_c · b^γ_c` over boosted values `b ≤ 3`, where `b` is the AIC-3 boosted JND of the same image and
     level.
   - Bootstrap over ladders and over the refit's 1,000 bootstrap replicates of the boosted scale.
   - The boosted refit's medians over the 25 ladders, levels 1–10, are 0.18, 0.50, 1.15, 1.65, 2.19, 2.60, 2.97, 3.29,
     3.71 and 4.09. They run 1.45–1.67× the published AIC-4 sample JND for the same images, so the published values
     give the link to AIC's plain-viewing unit.
5. **Ties.** Refit with "?" answers counted half and half, and report both fits.
6. **How observers inspected.** The zoom history gives the zoom, the visible region at each switch and the time on
   each view at each zoom. From those come pixels per degree at the moment of decision, and the zoom observers chose
   near threshold.
   - This is observational: chosen zoom is not a randomised treatment (readiness §2).
7. **Encoder targets.** For each codec, the quality at 0.5, 1 and 2 jnd-ez for each image, with the median and spread
   over images.

## What squintly needs

**Already there:** `hold` switching; integer zoom 1–8× with shared pan; DPR and calibration; per-trial data (final
zoom, pan totals, switch count, time on each view, dwell); named studies (`src/studies.rs`); the 120-answer block
progress.

**To add:**

1. **A `jnd-ez-v1` study.** It needs:
   - a detection trial type, where a byte-identical copy of the original takes A or B and the side is balanced
   - the AIC question wording
   - the four confidence answers, with no tie hint
2. **A psi selector** for each observer and ladder, stored server-side. It interleaves ladders and stops each one at
   80 answers.
3. **Zoom history**, the readiness contract's "zoom history or exposure by factor". Each trial gets an event trace
   (time, view, zoom, visible source rectangle) and the time on each view at each zoom.
4. **Null, catch and practice trials**, each designation stored with the trial.
5. **The break prompt at 120 answers**: the frontend has to read `break_recommended`.
6. **A fixed-1× condition flag** for the optional block.
7. **Export columns:** study, ladder, level, designation, confidence and trace.
8. **A stimulus manifest.**
   - It holds the AIC-3 PTC images, with attribution, and the zen ladders, all with hashes.
   - Every image must be fetched and decoded before use, with zero unresolved.
9. **Qualification receipts** for each observer's phone.

## Open decisions

- The unit names: jnd-ez, jnd-ctc and jnd-typ.
- The plan size (minimal or full) and the number of observers.
- For the zen block: the encoder versions, the fixed modes and each codec's quality range.
