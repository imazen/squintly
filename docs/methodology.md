# Squintly Methodology

This document codifies *every methodology choice* in Squintly with the
rationale behind it. The reference point is CID22 (Sneyers et al. 2023, [PDF](https://cloudinary-marketing-res.cloudinary.com/image/upload/v1682016636/wg1m99012-ICQ-AIC3_Contribution_Cloudinary_CID22.pdf));
where we diverge, the divergence is intentional and documented here.

If you change a constant in code without updating this document, you've
introduced a methodology bug. The rule is: every magic number cited here is a
contract, and every contract has a citation.

## 1. Two objectives, not one

CID22 ran two protocols in parallel and produced two scales (RMOS from
TSBPC, MCOS from DSBQS) that they merged. Squintly has two distinct
objectives, deliberately:

1. **Relative scoring** via pairwise comparison → Bradley–Terry-Davidson
   (Davidson 1970) per (source, condition_bucket).
2. **Imperceptibility / annoyance thresholds** via 4-tier ACR + Levitt 1971
   adaptive staircase per (source, codec, condition_bucket): `q_notice`,
   `q_dislike`, `q_hate`.

The threshold table is what an encoder picker actually wants. The pairwise
scoring is what a metric trainer (zenanalyze/zentrain) wants. Both ride out
of the same session interleaving.

## 2. Stimulus presentation

| Choice | Squintly | CID22 |
|---|---|---|
| Reference shown | toggle (hold-to-reveal) | left-of-screen always (TSBPC) or in-place toggle (DSBQS) |
| Upscaling boost | **none** | TSBPC scales to fill screen height; DSBQS does not |
| Display ratio | observer's native dpr (intrinsic_to_device_ratio captured per trial) | dpr1 (1 image px = 1 CSS px); upscaled in TSBPC only |
| Time limit | none | none |
| Min interaction before answer | none enforced | min 2 toggles in TSBPC |

**Rationale for no upscaling boost.** CID22's boosting changes the
perceptual task — observers see artifacts the original presentation
wouldn't expose. Phone observers in the wild do not zoom every image; we
want data that reflects "as actually displayed." A separate v0.3 boost-mode
track is on the roadmap for direct CID22 comparability.

## 3. Sampling

### 3.1 Source selection

Random uniform over manifest sources for v0.1. v0.2 will inverse-weight by
existing-coverage to avoid starving low-N sources.

### 3.2 Trial-type mix

Per session, default `p_single = 0.65`. The single-stimulus (threshold)
data needs more samples per bucket to converge a staircase, so we bias
early-session toward singles.

### 3.3 Quality bias

When picking which encoding within a source/codec, we sample the lower
half of the quality grid 60% of the time. **Source:** the
source-informing-sweep rule in CLAUDE.md — web codecs ship at q5–q40 where
RD curves are hardest and existing IQA datasets undersample.

### 3.4 Trivial-triplet filter (IMPLEMENTED, matches CID22)

We never sample pairs whose answer is foregone:

- Same codec, same source, quality gap ≥ ⌈grid_size / 2⌉ → skip.
- Cross-codec where the encoded-bytes ratio > 4× → skip (likely the
  smaller one is unambiguously worse-or-better).

**Rationale (CID22 §Selection of stimuli):** "a 0.5 bpp JPEG image versus
an AVIF image at more than 1.5 bpp was considered a trivial comparison
(likely the AVIF would be better)." Trivial pairs eat opinions without
moving the BT posterior.

### 3.5 Codec-support filter

The session's `supported_codecs` set (from the client probe) hard-filters
the sampler. We never serve a codec the browser can't natively decode —
transcoding to PNG would compromise the perceptual measurement.

### 3.6 Anchor reservation (IMPLEMENTED in v0.2)

If a source has anchor stimuli registered (`corpus_anchors`), we
**reserve at least 30% of session slots** for them. Anchors are sampled
*outside* the active-learning queue once that lands; for v0.1 they're
distributed across the session.

**Rationale (CID22 §Anchors):** without anchored stimuli, there's nothing
to interpolate between, and the bias-correction step has no reference
distribution to normalize against.

### 3.7 Onboarding calibration (IMPLEMENTED in v0.2)

Every session starts with **5 calibration trials** with known answers,
showing each answer immediately. They cover:

1. Reference vs reference (any answer is correct; this is a no-op probe)
2. Reference vs ~q15 mozjpeg (must be detected as worse)
3. q90 vs q40 same codec (must pick q90)
4. An IMC: "tap *tie* for this question only" (Meade & Craig 2012)
5. Reference vs near-lossless (must be rated `imperceptible`)

Below 60% on calibration → `observers.calibrated=0` flag, but we still
sample. Their data is filtered out at training time, not at session time.

**Rationale:** CID22 used 4 training stimuli for "exposure to very low and
very high quality"; KonIQ-10k used a 6-question quiz that dropped 24% of
applicants. Phone observers in the wild won't tolerate KonIQ's gate, but
the *exposure* effect is real; we keep the exposure, soften the gate.

### 3.8 First-3 trials are warmup (CID22 verbatim)

Always discarded from analysis. `weight=0` at export. Rows stay in the DB.

### 3.9 Honeypots (IMPLEMENTED in v0.2)

1 in 12 trials is a honeypot. The sampler picks an anchor where the
expected outcome is unanimous (reference vs ~q5 mozjpeg, or reference
itself). `is_golden = 1`, `expected_choice` is set.

**Honeypot pass rate (KonIQ floor):** 70%. Below that → session F.

**Hard-gate honeypots (CID22 verbatim):**
- Reference image rated < 5 on the 4-tier ACR (i.e. ≥ "I dislike") → session F.
- ~q5 mozjpeg rated `imperceptible` → session F.

## 4. Outlier detection

Implemented in `src/grading.rs`. Three layers:

### 4.1 Inline (per-trial flags)

| Flag | Trigger | Citation |
|---|---|---|
| `rt_too_fast` | dwell_ms < 800 (single) / 600 (pair) | Meade & Craig 2012 §RQ6 |
| `rt_too_slow` | dwell_ms > 60_000 | (AFK) |
| `no_reveal` | pair AND reveal_count == 0 | CID22 disqualifies <2 switches |
| `golden_fail` | is_golden AND choice ≠ expected_choice | KonIQ 70% floor |
| `viewport_clipped` | image_displayed_w_css * dpr < 0.5 * intrinsic_w | (observer can't see what they're rating) |

### 4.2 Session-end aggregate (NEW: tightened to CID22 thresholds)

Geometric mean of five sub-scores → A/B/C/D/F:

- `golden_score`: 1.0 if pass_rate ≥ 0.70 else clamped((rate - 0.40)/0.30, 0, 1)
- `straight_line_score`: 1.0 if line_clicker ratio ≤ 1.5 else clamped((2.5 - r)/1.0, 0, 1) — KonIQ
- `rt_floor_score`: 1.0 if frac < 0.10 else clamped((0.30 - frac)/0.20, 0, 1)
- `even_odd_score`: clamp((r - 0.10) / 0.40, 0, 1) — Meade & Craig
- `no_reveal_score`: 1.0 if frac ≤ 0.20 else clamped((0.50 - frac)/0.30, 0, 1) — CID22

Geometric mean: any single zero zeroes the weight.

### 4.3 Cross-session (v0.2 batch)

- pwcmp leave-one-out log-likelihood, `dist_L > 1.5` flag (Pérez-Ortiz & Mantiuk 2017).
- Pérez-Ortiz 2019 (δ_o, σ_o) per-observer fit for ≥30-trial observers; weight ∝ 1/σ_o².
- CID22 normalised-disagreement aggregation across observers per stimulus.

**Target rate of session F:** ≤ 15% (CID22 actual was 14.7%). If we exceed
20% we tighten goldens; if below 8% we relax them. Sanity check runs nightly.

## 5. Score construction

### 5.1 Pairwise → BT-Davidson with monotonicity constraint (NEW)

For each (source, condition_bucket) we fit Bradley–Terry-Davidson:

```
P(a > b)  = exp(β_a) / (exp(β_a) + exp(β_b) + ν · exp((β_a + β_b)/2))
P(a ~ b)  = ν · exp((β_a + β_b)/2) / Z
P(a < b)  = exp(β_b) / Z
```

Anchor: β_reference = 0. Gaussian prior on β with σ = 1.5 (≈ 1.5 JOD,
matches Pérez-Ortiz 2017).

**Monotonicity constraint (CID22 §Monotonicity).** Before fitting, for
every same-codec pair `(e_low, e_high)` where `e_high.quality > e_low.quality`,
we inject **N_dummy = 200** fake "high beats low" comparisons.

**Why N=200?** CID22 found this enforces monotonicity "almost always"
without overpowering real disagreement, and that the constraint mattered
*more* than participant screening (KRCC 0.99 vs 0.56 without). We adopt
this verbatim.

### 5.2 β → 0–100 quality scale

`quality = clamp(100 + (β - β_ref) * 10, 0, 100)`. The factor 10 is a
display convention; downstream zentrain handles its own re-scaling. Final
calibration to MCOS-equivalent uses the alignment in §6.

### 5.3 4-tier ACR → q_notice/q_dislike/q_hate via staircase + logistic

Per (source, codec, condition_bucket):

- **Online (per-session)**: Levitt 1971 transformed up–down staircase.
  3-down-1-up → P=0.794 → `q_notice`. 2-down-1-up → P=0.707 → `q_dislike`.
  1-down-1-up → P=0.500 → `q_hate`.
  Mean of last 6 reversal qualities (drop first reversal — biased by start
  point) is the per-session estimate.

- **Offline aggregate** (v0.2 batch): logistic psychometric fit
  `P(rating ≥ k | q, c) = Φ((q - μ_k(c)) / σ_k(c))` per condition vector
  c. Bootstrap CI 200 iterations.

### 5.4 Per-session bias correction (IMPLEMENTED in v0.2)

Per CID22 §Bias Correction: each session gets an additive offset
`c_session` chosen so the mean normalized difference (z-score vs the
group mean+std per stimulus) across that session's observations is zero.
Adjusted ratings clamp to [1, 4] for the 4-tier ACR.

Implementation: `stats::session_bias_offsets()`, applied in `thresholds_tsv`
before the per-q histogram is built.

### 5.5 Unified pairwise+rating scale (IMPLEMENTED in v0.2)

Pérez-Ortiz et al. 2019 model: `π_ik = m_i + δ_k + ξ_ik` where m_i is
latent stimulus quality, δ_k is per-observer bias, ξ_ik is observer-noise
N(0, σ_k²). 4-tier ACR uses an ordinal cumulative-link likelihood with
shared category thresholds τ_1 < τ_2 < τ_3. Pair likelihood is Thurstone
Case V: P(i > j) = Φ((m_i - m_j) / (√2 σ)).

Joint fit by gradient descent with Gaussian priors (σ_β = 1.5 on m,
σ_δ = 0.5 on δ, log-σ_o ~ N(0, 0.5²)). Anchored at m_reference = 0.
Implementation: `unified::fit_unified()`, exported as `unified.tsv` per
(source, condition_bucket).

### 5.6 Disagreement mitigation between protocols (IMPLEMENTED in v0.2)

`stats::disagreement_dummy_count()` and `stats::overlap_tie_count()`
implement CID22's two-sided rule:
- If 90% CIs disjoint: add `multiplier × gap` "B-better-than-A" dummies
  (CID22 used multiplier=20).
- If 90% CIs overlap: add `multiplier × overlap_size / span` "I can't
  choose" ties (CID22 used multiplier=200).

The hooks are available to the v0.3 batch grader; v0.2 ships the unified
fit which encodes equivalent disagreement information natively via the
joint likelihood.

## 6. Quality-scale alignment

Approximate alignment of CID22 MCOS to other scales (CID22 Table 5):

| | medium | high | visually lossless |
|---|---|---|---|
| CID22 MCOS | 50 | 65 | 90 |
| TID2013 MOS | 4.5 | 5.5 | 6 |
| KADID DMOS | 3.7 | 4.3 | 4.5 |
| AIC-3 JND | 3 | 1.7 | 0 |
| KonJND-1k PJND | 1 | 0 | 0 |

Squintly's BT-derived 0–100 scale is anchored such that **100 = reference**
and the q_notice/q_dislike/q_hate thresholds correspond approximately to
**MCOS 88 / 65 / 50** based on the verbal anchors. Calibration holds with
±5 MCOS points pending the v0.2 unified-fit pass.

## 7. Confidence intervals

Bootstrap with replacement, 200 iterations (CID22 verbatim). Re-run the
*entire* pipeline (BT fit + monotonicity injection + threshold logistic)
per iteration. Reported CIs are 5th and 95th percentiles.

**v0.2 IMPLEMENTED.** `stats::bootstrap()` resamples observations 200
times and re-runs the full BT-Davidson fit (with monotonicity injection)
or threshold logistic interpolation. `pareto.tsv` and `thresholds.tsv`
both carry per-quantity 5th and 95th percentile columns; `unified.tsv`
does the same for the joint Pérez-Ortiz fit.

## 8. Sample-size targets

CID22 Figure 7 calibration: **80 single-stimulus opinions per anchor + 5
pair opinions per pair** matches the 90% CI of the full 22k-image sweep.

Squintly target per (source, condition_bucket):
- ≥ 80 single-stimulus opinions per anchor
- ≥ 5 pair opinions per pair

Below those floors, the row is exported with `weight = 0` regardless of
the BT/threshold fit.

## 9. Reproducibility

Every export TSV row carries:
- The codec name and version (when coefficient supplies it)
- The condition bucket (dpr × distance × ambient × gamut)
- Sample counts (observers, trials)
- Bootstrap CI (v0.2)

The full session/trial/response history is preserved in SQLite and
exportable raw via `responses.tsv`. Raw data > derived scores: anyone can
re-fit with their own model.

## 10. Held-out validation discipline (IMPLEMENTED in v0.2)

20% of source images can be marked `held_out=1` in `source_flags`. The
sampler propagates this onto every trial (`trials.held_out`) and all
exports carry the flag as a column. Held-out rows still feed the *raw*
data store but downstream training pipelines must filter `held_out=1` from
training, parameter selection, and threshold calibration — they're for
final-metric evaluation only.

## 11. Active sampling (IMPLEMENTED in v0.2 as `asap` module; not yet wired into the runtime sampler)

ASAP — Active Sampling for Pairwise (Mikhailiuk 2020). For each candidate
pair, EIG ≈ binary entropy of the predicted outcome under the current
MAP estimate, maximized at p=0.5 (least-decided pair). 30-50% sample
reduction vs random in published results.

`asap::pick_max_eig()` is the API; integration into `pick_trial` is a
v0.3 wiring task because it requires maintaining the per-(source, bucket)
β posterior in memory and updating after every response. v0.2 ships the
algorithm + tests.

## Citations

- **CID22**: Sneyers, Ben Baruch, Vaxman, "AIC-3 Contribution from Cloudinary: CID22" (2023). [PDF](https://cloudinary-marketing-res.cloudinary.com/image/upload/v1682016636/wg1m99012-ICQ-AIC3_Contribution_Cloudinary_CID22.pdf)
- **BT-Davidson**: Davidson, "Extending the Bradley-Terry model to incorporate ties" (JASA 1970)
- **Levitt 1971**: "Transformed up-down methods in psychoacoustics" (J. Acoust. Soc. Am.)
- **Pérez-Ortiz & Mantiuk 2017**: "A practical guide and software for analysing pairwise comparison experiments" ([arXiv:1712.03686](https://arxiv.org/abs/1712.03686))
- **Pérez-Ortiz et al. 2019**: "From Pairwise Comparisons and Rating to a Unified Quality Scale" (IEEE TIP) ([PDF](https://www.cl.cam.ac.uk/~rkm38/pdfs/perezortiz2019unified_quality_scale.pdf))
- **Meade & Craig 2012**: "Identifying Careless Responses in Survey Data" ([PDF](https://ubc-emotionlab.ca/wp-content/uploads/2012/06/Meade-Craig-2012-Careless-Responding-in-Survey-Data.pdf))
- **KonIQ-10k**: Hosu, Lin, Saupe (IEEE TIP 2020) ([arXiv:1803.08489](https://arxiv.org/abs/1803.08489))
- **BT.500-14**: ITU-R recommendation ([PDF](https://www.itu.int/dms_pubrec/itu-r/rec/bt/R-REC-BT.500-14-201910-S!!PDF-E.pdf))
- **SSIMULACRA 2**: weights tuned on 201/250 CID22 references; held-out KRCC=0.7033 ([code](https://github.com/cloudinary/ssimulacra2))
