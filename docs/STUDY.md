# Squintly — formal study design & pre-registration

**Status:** v0.2 protocol — pre-registered for main data collection.
**Last revised:** 2026-05-28.
**Cross-refs:** [`SPEC.md`](../SPEC.md), [`docs/methodology.md`](methodology.md),
[`docs/participant-grading.md`](participant-grading.md), [`docs/motivation-and-compensation.md`](motivation-and-compensation.md).

This document fixes the **research questions, hypotheses, design, methods, success criteria,
and analysis plan** that the squintly data collection is designed to answer. The point of
pre-registering them is that downstream zensim work — whether we ship a v48 with a new
training group or not — must judge itself against criteria written down before the data
arrived.

---

## 1. Background and motivation

Existing public FR IQA datasets (KADID-10k, TID2013, CSIQ, CID22, AIC-3) bake fixed
viewing conditions into their labels: lab monitor, specified viewing distance, specified
ambient. zensim trained on those plateaus around **SROCC 0.82 on CID22** and the
residual is **structured by viewing conditions**: phone DPR 2–3.5 at 25–35 cm changes
which artefacts are visible vs invisible (see [`docs/iqa-methods/reference-book/ch11_mobile_specific.md`](https://github.com/imazen/zenpapers/blob/main/docs/iqa-methods/reference-book/ch11_mobile_specific.md)
in zenpapers — mobile foveal Nyquist 35–47 cpd vs desktop 16–19 cpd).

The squintly hypothesis: **a metric that takes viewing conditions as input — rather than
assuming them — should close that gap**. Squintly collects pairwise + threshold judgments
**with the conditions recorded as first-class data per trial**, on the phones that
actually deliver the bulk of web image traffic.

This makes squintly a different ground truth from CID22/AIC-3 — not a redo. CID22 answers
"on a calibrated lab monitor, which encoding is preferred"; squintly answers "at the
typical phone DPR/distance/ambient mix, which encoding crosses **q_notice / q_dislike /
q_hate** — and how does the threshold depend on conditions."

---

## 2. Research questions

**RQ1 (primary).** Can a viewing-condition-conditioned quality model substantially
outperform a condition-blind model at predicting human perception across the realistic
distribution of web viewing conditions (DPR 1.0–3.5, viewing distance 25–250 cm,
ambient dim–bright)?

**RQ2 (primary).** Can the *threshold function* `q_threshold(c, k)` for
`k ∈ {notice, dislike, hate}` be predicted from the viewing-condition vector `c` well
enough to drop into an encoder picker as "minimize bytes subject to `q ≥ q_threshold(c, k)`"?

**RQ3 (secondary).** Does the **unified pairwise + ACR-rating scale** (Pérez-Ortiz 2019
mixed BT + categorical model) recover a single per-encoding latent quality that
predicts held-out responses better than either protocol alone?

**RQ4 (secondary).** Which of the 10 ranked human-effort amplifiers (README §3) yields
the largest empirical reduction in human-time-per-converged-cell on our content?

---

## 3. Hypotheses (pre-registered)

| # | Hypothesis | Operationalisation | Pre-registered effect / pass criterion |
|---|---|---|---|
| **H1** | Condition-conditioned zensim beats condition-blind v47 on held-out squintly data. | Train zensim-v48 with squintly's `(stimulus_id, viewing_condition_bin)` JOD labels; eval held-out (15 % by reference) squintly responses. | **Pooled held-out SROCC ≥ v47 + 0.05**, with the gain concentrated (≥ ⅔) in the high-DPR or short-viewing-distance bins. Bootstrap-CI non-overlapping with v47. |
| **H2** | `q_threshold(c, k)` is predictable from `c`. | Fit logistic `P(rating ≥ k \| q, c) = Φ((q − μ_k(c)) / σ_k(c))`; report R² of `μ_k(c)` on held-out. | **R² ≥ 0.5** for at least `k = notice` on held-out. If H2 fails for `k = hate`, that's a finding about thresholds being content-driven, not a study failure. |
| **H3** | Unified BT + ACR > either alone. | 5-fold CV held-out-rating log-likelihood on the unified model vs BT-only vs ACR-only on the same trial pool. | **Held-out log-likelihood gain ≥ 2 nats per trial** with unified, with the gain robust across folds. |
| **H4** | Active sampling (ASAP, EIG ranker) reduces human-time to a per-cell CI ≤ 0.5 JOD by ≥ 5× vs random sampling. | Simulate replay over random-subsample vs ASAP-subsample of the same trial pool; measure number-of-trials-to-CI-target. | **≥ 5× speedup** for the median cell; on-par on the worst-case cell. |
| **H5** | Phone-recruited data is **not** strictly noisier than desktop after Crowd-BT η weighting. | Per-platform η distribution + held-out SROCC of phone-only vs desktop-only subsets. | **Phone-only SROCC within 0.03 of desktop-only** on the same held-out pool (i.e., the device is a useful covariate, not a noise floor we have to throw out). |

**Decision rules** (locked):

- **Ship v48** to zensim's `Profile::A` iff H1 AND H2(notice) hold, AND no regression on
  CID22 / AIC-3 / AIC-4 SROCC ≥ 0.005, AND blur-above-identity = 0 (the v39→v47 invariant
  preserved).
- **Document an honest-stop** if H1 fails — with the next hypothesis explicitly named
  (likely "conditions need a nonlinear interaction with content class" or "the
  intrinsic-to-device ratio isn't the dominant axis we thought it was").

---

## 4. Design

### 4.1 Factorial structure

The factorial we *aim* to fill — actual coverage is shaped by active sampling (§4.4):

| Factor | Levels | Source |
|---|---|---|
| Reference image | ~300 (curated from imazen-26-synth-500 + suggestions; size-balanced) | [`docs/CORPUS_CURATOR_SPEC.md`](CORPUS_CURATOR_SPEC.md) |
| Size bucket | 4: ≤ 256 px, ≤ 768 px, ≤ 2048 px, > 2048 px | SPEC.md §3 + CLAUDE.md "Sweep / Calibration" rule |
| Codec | 5: mozjpeg, WebP, AVIF, JXL, JPEG-AI (planned) | distortion plugins, see [`zenpapers/docs/study-system/01_distortion_framework.md`](https://github.com/imazen/zenpapers/blob/main/docs/study-system/01_distortion_framework.md) |
| Quality grid | 12 log-spaced, low-q-dense {q5, q10, q15, q20, q25, q30, q40, q50, q65, q80, q90, q95} | CLAUDE.md sweep discipline |
| Viewing condition bin | 6 (3 DPR × 2 distance) at minimum; finer if sample size permits | per-trial captured (§4.3) |

### 4.2 Trial types (interleaved)

- **Type S** (single-stimulus threshold, 70 % early / 50 % late):
  - 4-tier ACR (imperceptible / I notice / I dislike / I hate).
  - **Adaptive transformed up–down staircase** (Levitt 1971): 3-down-1-up for `q_notice`
    (79.4 %), 2-down-1-up for `q_dislike` (70.7 %), 1-down-1-up for `q_hate` (50 %).
  - Three staircases per (source, condition-bucket), step halves at each reversal until
    the codec-config grid resolution.
  - Already implemented in `src/staircase.rs` (233 LOC).
- **Type P** (pairwise + tie, 30 % early / 50 % late):
  - Triplet `(reference, A, B)`; observer answers "A closer / tie / B closer".
  - Stored as **Bradley–Terry-Davidson** observation (ties first-class).
  - Already implemented in `src/bt.rs` (240 LOC).

The protocol is fixed; **the schedule** (which trial next) is driven by the active sampler
(§4.4).

### 4.3 Viewing-condition vector `c` (captured per trial)

Per-session (stable): `device_pixel_ratio`, screen CSS + device dims, `color_gamut`,
`dynamic_range`, `prefers_color_scheme`, `pointer_type`, `user_agent`, `connection_type`,
`timezone`, self-reported {`viewing_distance_cm`, `ambient_light`, `vision_corrected`,
`age_bracket`}, **`calibration`** (credit-card → CSS-px-per-mm → angular resolution in
cycles per degree given viewing distance).

Per-trial (variable): `viewport_*` (orientation-aware), `image_intrinsic_w/h`,
`image_displayed_w/h_css`, `image_device_w/h`, **`intrinsic_to_device_ratio`** (headline
condition variable), `dwell_ms`, `zoom_used`, `swap_count`.

Schema is in `migrations/0001_init.sql` + `0006_v02_rigor.sql`. **Critical: no aggregation
in storage**; the condition vector lives unaggregated on every response row.

### 4.4 Sampling strategy

Per [`docs/methodology.md`](methodology.md) §3 + §11:

- **Source selection:** inverse-coverage weighted (under-rated sources get more trials).
- **Trivial-triplet filter** (`docs/methodology.md` §3.4): drop A/B pairs where the
  metric ensemble unanimously predicts ≥ 95 % preference → free pre-filling, doesn't
  burn human time.
- **Anchor reservation:** CID22-style reference-pinned anchors interspersed at ~5 %
  rate to align our scale to CID22's where the content overlaps (`corpus_anchors` table).
- **First-3-trials warmup** (CID22 verbatim).
- **Active sampler (ASAP):** `src/asap.rs` IS implemented (106 LOC) **but not yet wired
  into the runtime trial selector**. This is the v0.2-finishing chunk: route
  `next_trial` through `asap::next_pair` once the BT posterior is non-degenerate
  (≥ 50 trials per study).

### 4.5 Screening, calibration, QC

- **Onboarding calibration** (`docs/methodology.md` §3.7): credit-card or known-element
  CSS-px-per-mm; codec-probe ensures the browser can actually decode each codec; failed
  decoders excluded from sampling for that session.
- **Qualifier** (post-calibration, before main session): 8 trials with known answers
  (golden); ≥ 6/8 correct → `qualifier_passed = 1`; `trusted_pool = 1` after sustained
  pass rate (`migrations/0002_grading.sql`).
- **Honeypots in-session** (`docs/methodology.md` §3.9): 1 in ~30 trials is a known-
  reference vs heavily-distorted pair; failing one is a flag, failing two ends the
  session.
- **Outlier flags** (`docs/methodology.md` §4.1): dwell_ms < 500 or > 30 000; ≥ 7
  straight-line responses (no variance); reversed-monotonicity within a staircase.
- **Session grade** A–F (§4.2) drives `session_weight` ∈ {0, 0.5, 1.0, 1.5} multiplier
  applied at scale-reconstruction time.
- **Cross-session reliability** (§4.3, v0.2 batch): **Crowd-BT-style η per observer**,
  re-weights the observer's whole history. Hooks exist in `grading.rs` but pwcmp-style
  leave-one-out per-observer log-likelihood is a TODO at `src/grading.rs:340`.

---

## 5. Statistical analysis plan (pre-registered)

### 5.1 Scale reconstruction

**Primary fit:** unified Pérez-Ortiz 2019 mixed Bradley–Terry-Davidson + ordinal-ACR
model. Per source `s`, per condition-bucket `b`, fit a **scalar latent quality `θ_{s,e,b}`**
per encoding `e`, with:

- BT-Davidson likelihood on Type-P responses (ties via the `ν √(exp θᵢ exp θⱼ)` term,
  Davidson 1970; spec'd in `src/bt.rs`).
- Ordinal threshold likelihood on Type-S responses (cumulative-logit with thresholds
  `τ_k` shared per condition-bucket).
- Per-observer bias `δ_o` and slope `σ_o` (Pérez-Ortiz 2019 §3).
- Gaussian prior on `θ` (σ_prior = 1.0; the value pwcmp's standalone paper recommends —
  fetched 2026-05-27 as arXiv 1712.03686, was previously honest-stop).

**Anchor:** `θ_reference = 0`; cross-source comparability via CID22 anchors (§4.4).

**CIs:** 1000-sample bootstrap over **observers**, not trials (the observer is the unit
of resampling because errors are correlated within observer per BT.500 / Pérez-Ortiz).

### 5.2 Threshold inference

For each `k ∈ {notice, dislike, hate}`:

```
P(rating ≥ k | q, c) = Φ((q - μ_k(c)) / σ_k(c))
```

where `c` is the viewing-condition vector (binned + interaction with codec). Fit `μ_k(c)`
as a generalised-additive model (GAM) with smooths on `dpr`, `intrinsic_to_device_ratio`,
`viewing_distance_cm`; offsets per codec; per-source random intercept.

Online (per-session) threshold estimate comes from the staircase reversals (Levitt 1971);
offline is the GAM. **H2 is tested on offline only.**

### 5.3 Condition conditioning for zensim

**Method:** add `(dpr, intrinsic_to_device_ratio, viewing_distance_proxy_cpd)` as input
features to zensim's 372-feature input (so input becomes 375). Train v48-cond with the
existing `zensim_mlp_train` recipe + the new squintly group, target `human_score`
normalised per (source, condition-bucket).

**Comparison:** v48-cond vs v48-blind (same data, conditions zeroed out) vs v47-strict
(no squintly data). Held-out: 15 % of squintly responses by reference + the **condition
holdout** = one bin combination withheld entirely (per §4.6).

### 5.4 Significance tests

Per [`zenpapers/docs/iqa-methods/reference-book/ch3-5_sampling_screening_cis.md`](https://github.com/imazen/zenpapers/blob/main/docs/iqa-methods/reference-book/ch3-5_sampling_screening_cis.md):

- **Δ-SROCC** between metrics: bootstrap CI over observers + Krasula different-vs-similar
  AUC.
- **F-test** on residual variance for nested model comparisons (v48-cond ⊂ v48-blind ⊂
  v47).
- **Bonferroni correction** across the 5 pre-registered hypotheses (α_individual =
  0.05 / 5 = 0.01).

### 5.5 Sensitivity / robustness analyses (announced, not pre-registered as primary)

- Per-codec breakout: SROCC per codec to detect codec-specific overfit.
- Per-content-class breakout: photo / detail / texture / lineart / graphic / flat
  (from imazen-26-synth-500 clustering).
- Drop low-grade sessions (`session_weight < 1.0`): does H1 still hold on A-grade only.
- Drop intrinsic_to_device_ratio outliers (< 0.5 or > 3): does H1 still hold without
  the extremes.

---

## 6. Sample size & stopping

### 6.1 Per-cell target

A cell is `(source, codec, q-step, condition-bucket)`. Squintly's active sampler
populates cells with high EIG; we don't target uniform coverage.

**Convergence target:** per fitted-condition-bucket, **bootstrap 95 % CI on `θ` ≤ 0.5 JOD**
for ≥ 80 % of (source, codec) pairs in that bucket.

### 6.2 Pilot → main staging

- **v0.2 pilot:** 100 sessions across the existing curator-curated corpus. Goal: validate
  the trial loop end-to-end + populate the per-observer reliability `η` baseline + tune
  the ASAP cold-start.
- **v1.0 main:** target 1500 sessions total, monitored in batches of 250. Stopping rule:
  cease when §6.1 convergence criterion is met OR after 1500 sessions, whichever first.

### 6.3 Power (informal)

At pwcmp-typical noise floor (σ_observer ≈ 1.5 JOD), 4–6 observations per cell yields
~0.4 JOD per-cell CI. With ASAP and ≥ 5× speedup vs random (H4), 1500 sessions × 30
trials = 45,000 trials → ~12,000 informative cells covered. The full factorial is
300 sources × 5 codecs × 12 q × 6 conditions = 108,000 cells; **we cover ~10 %, chosen
by EIG**.

---

## 7. Holdout & generalisation tests

- **By-reference holdout:** 15 % of sources held out from training (matching CID22's
  49/250 fraction); reported in all H1/H2 evaluations.
- **By-condition holdout:** one (DPR × distance) bin (e.g., DPR ≥ 3 + distance ≤ 30 cm)
  withheld **entirely** during training; H1 measured on this bin tests extrapolation.
- **Cross-dataset hold:** CID22-49 (zensim's existing held-out) MUST NOT regress
  beyond −0.005 SROCC. AIC-3 SROCC SHOULD go up (squintly's BTC-style triplets are the
  closest match to AIC-3's regime).

Per [`zenpapers/docs/iqa-methods/reference-book/ch8_better_than_cid22_spec.md`](https://github.com/imazen/zenpapers/blob/main/docs/iqa-methods/reference-book/ch8_better_than_cid22_spec.md)
§5: holdout is **by reference**, never by row. Random row-splits leak; we enforce
by-reference at the export-table level.

---

## 8. Ethics, consent, data handling

### 8.1 Consent posture

- **Default:** anonymous, no login, observer UUID in localStorage; consent is the click
  through "Start" after reading the welcome screen (consent text linked from welcome).
- **Email tier (optional, T1):** required to participate in streaks/leaderboards; email
  is verified via Postmark magic-link, used only for re-auth, never shared.
- **Researcher tier (T3):** internal; full access; identified.
- **No PII collected outside opt-in tiers.** No IP logging beyond hashed bucket.

### 8.2 GDPR baseline

- EU users default to **Hetzner Helsinki** (EU-resident). All retention is local; no
  third-party trackers.
- Right-to-erasure: observer UUID deletes their row in `observers` + cascade-deletes
  all `sessions`/`responses` (FK ON DELETE CASCADE).
- Aggregate statistics (BT scores, threshold tables) are **not** PII once observer IDs
  are dropped; safe to release CC-BY post-study.

### 8.3 Data sharing & redistribution

- **Per-image license:** every source has a license badge (`src/licensing.rs`); we
  redistribute only what each license permits.
- **Per-response license:** the human responses are ours to release CC-BY; the
  **derived score tables** (`pareto.tsv`, `thresholds.tsv`) ship CC-BY-4.0; the **raw
  images** ride their source license.
- Commercial-training disclosure: surfaced per-corpus in `welcome` (see
  `src/licensing.rs` registry).

---

## 9. What's locked vs what's exploratory

**Locked (changing these requires a documented amendment to this file):**

- Trial protocols (S = 4-tier ACR staircase; P = 3-option BT-Davidson).
- Holdout fractions (15 % by-reference + 1 condition bin).
- Primary hypotheses (H1–H3) and their pass criteria.
- Bonferroni correction.
- Anonymous-only consent default.

**Exploratory (any post-hoc analysis must be flagged as such):**

- Per-codec / per-content-class breakouts.
- Mobile-only vs desktop-only sub-analyses.
- Time-of-day, ambient-light interactions.
- Engagement-tier (anon vs email vs passkey vs researcher) noise differences.

---

## 10. Reproducibility checklist (must hold at study close)

- [ ] Every response row carries: `observer_id, session_id, source_hash, encoding_id,
      viewing_condition_vector, dwell_ms, response, session_grade, session_weight,
      qualifier_passed, η`.
- [ ] Every `corpus_anchor` row carries the CID22 anchor it pins to + the
      `build_commit` of the corpus snapshot.
- [ ] Every export TSV ships with a `_MANIFEST.json` containing
      `study_commit, db_snapshot_sha256, n_observers, n_sessions, n_responses,
      stopping_rule_met_at`.
- [ ] Every figure / claim in the v48-zensim writeup cites the response-row SHAs it
      drew from.
- [ ] The full SQLite DB snapshot is hashed + mirrored to Tower NAS at study close.

---

## 11. Phased plan

| Phase | What | Done-when | Where it lives |
|---|---|---|---|
| **v0.1** (shipped) | Curator + variant generation; backend skeleton; trial routes wired but not end-to-end exercised | Curator export round-trips; `cargo test` green | recent commits |
| **v0.2** (in progress) | Trial loop end-to-end on phone; ASAP wired into `next_trial`; honeypots active; v0.2 rigor in methodology.md | Real session start→end→export on a real phone; smoke test green | this study + README |
| **v0.3** (pilot) | 100-session pilot; H4/H5 measured; Crowd-BT η baseline; ASAP cold-start tuned | All five hypotheses' analysis pipelines green-on-pilot-data | future |
| **v1.0** (main) | 1500 sessions; H1–H5 evaluated; v48-cond bake decision | Decision rule (§3) executed; v48 ships or honest-stop | future |

The amplifier-by-amplifier roadmap lives in the README; this file fixes the
"what we're measuring and what counts as winning."
