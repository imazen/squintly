# Squintly Participant Grading & Outlier Management

Design notes for grading observers and weighting/rejecting their responses
when fitting BT/Thurstone scales and ACR distributions. Companion to
`SPEC.md`. Implementation lives in `src/grading/`.

Sources are cited inline by `[short-tag]`. Full URLs in the methodology
playbook companion document.

---

## 1. What we are NOT going to do

* **Do NOT use ITU-R BT.500 Annex A β₂ rejection on pairwise data.** It is
  defined for MOS — it computes per-stimulus mean and SD across observers
  and counts how many of each observer's *direct ratings* fall outside
  ±2σ (or ±√20·σ for non-Normal stimuli) [BT.500-14, §A.1]. Pairwise data
  has no per-stimulus rating, so the test is undefined. (We can apply it
  to our 4-tier ACR data, but only as a sanity check — see §5.)
* **Do NOT reject observers before N≥20 trials.** Both pwcmp and CID22
  set thresholds in IQR units that need a stable distribution; rejecting
  early creates bias against fast responders who happened to draw a hard
  early sequence.
* **Do NOT delete data on rejection.** Set `weight = 0` and a
  `quality_grade` reason. Keep the rows.
* **Do NOT use criterion-style fixed-σ on every observer.** Thurstone Case
  V's constant-σ assumption [PerezOrtiz2017 eq. 6] is fine for a *fit*,
  but post-hoc per-observer noise estimation is what gets us a grade.

## 2. The Thurstone Case V likelihood we actually fit

From Pérez-Ortiz & Mantiuk 2017, eq. 6:

```
P(C | q, σ) = ∏_{i,j} C(n_ij, c_ij) · F(q_i − q_j, σ)^c_ij
                                   · (1 − F(q_i − q_j, σ))^(n_ij − c_ij)
```

with `F = Φ(·/σ_ij)`, σ_ij = √2·σ, and σ chosen so that ∆q = 1 maps to
P = 0.75 (1 JOD). For Normal: σ = 1.0484, σ_ij = 1.4826.

Priors per pwcmp `pw_scale.m`:
* **Gaussian regularisation prior** on q: `0.01 · mean(q)²` (mean-zero
  regularization) — fixes the additive ambiguity.
* **Gaussian comparison prior** that adds a "soft" likelihood term using
  the empirical comparison distribution to break ties on incomplete
  designs.

This is what `src/scaling/thurstone.rs` should implement. L-BFGS via
`argmin` works well; analytical gradients are in pwcmp's `exp_prob_grad`.

## 3. Per-observer noise: what's actually in the literature

The honest answer is that pwcmp does **not** put a per-observer σ_o into
the likelihood. Thurstone Case V assumes σ is constant across all
conditions and observers [PerezOrtiz2019 §III.A, case 5]. Per-observer
modelling would push us into Case I/II territory and is "insolvable" with
a single observer per condition [PerezOrtiz2019 §III.A.1].

What the gfx-disp group actually does — and what we will copy — is
**post-hoc leave-one-out likelihood scoring** (`pw_outlier_analysis.m`):

1. For each observer `o`, fit `q^(-o)` from all *other* observers.
2. Compute the log-likelihood of `o`'s comparison matrix `M_o` under
   `q^(-o)`, using the same Thurstone CDF and binomial pmf.
3. Normalise by the number of trials (added Dec 2024 to pwcmp; previously
   biased against observers with more trials).
4. Compute IQR-normalised distance below the 25th percentile:
   `dist_L_o = (Q1 − L_o) / IQR · 1[L_o < Q1]`
5. Recommended threshold: `dist_L > 1.5` flag for inspection;
   pwcmp recommends manual review per-observer rather than auto-rejection.

This is the **σ_o substitute**. It costs O(N_observers) refits but each
refit is on N−1 observers and converges fast with warm-starting.

For 4-tier ACR data we have an actual per-observer Gaussian (Pérez-Ortiz
2019 eq. 1: `π_ik = m_i + δ_k + ξ_ik` where `δ_k` is observer bias and
`ξ_ik` is observer noise). We CAN estimate these from a sufficient number
of repeated measures per observer. With ~300 single-stimulus trials per
observer we can fit `(δ_o, σ_o)` and use `1/σ_o²` as a likelihood weight.

## 4. The grade we compute

Each observer gets a **`quality_grade`** in `{A, B, C, D, F}` and a
**`weight`** in `[0, 1]` derived from a composite score. The score is a
weighted geometric mean of normalised sub-scores (so any one being 0
zeroes the weight).

| Sub-score | Source | Floor | Citation |
|-----------|--------|-------|----------|
| `golden_pass_rate` | golden anchors | ≥ 0.70 | [KonIQ-10k §5.2 quiz] |
| `rt_floor_pass` | dwell_ms ≥ 600 ms on ≥ 90% of trials | ≥ 0.90 | [Meade & Craig 2012 §RQ6 — response time outlier] |
| `not_straight_lining` | <50% same-button on 4-tier | ≥ 0.50 | [Meade & Craig 2012 LongStringMax]; [KonIQ "line-clicker" max-count ratio < 2.0] |
| `consistency_4tier` | even-odd Spearman-Brown corrected r on 4-tier | ≥ 0.30 | [Meade & Craig 2012 Even-Odd Cons.] |
| `pwcmp_dist_L` | leave-one-out pairwise log-lik | ≤ 1.5 | [pwcmp pw_outlier_analysis] |
| `cid22_norm_diff` | normalised disagreement on 4-tier | mean ∈ [-1, 1] AND SD(|·|) ≤ 1 | [CID22 §Outlier detection] |
| `cid22_init_slider` | fraction of 4-tier responses left at default | ≤ 0.20 | [CID22 §Participant screening] |
| `golden_anchor_low_high` | unanimous-bad on a near-lossless reference fails session | strict | [CID22 §Participant screening — "near-lossless image score < 5"] |

Grades:
* **A**: weight = 1.00 — all sub-scores ≥ 0.9 of floor margin
* **B**: weight = 0.85 — one sub-score in [floor, 1.1·floor]
* **C**: weight = 0.50 — two sub-scores at floor or pwcmp_dist_L ∈ [1.5, 2.5]
* **D**: weight = 0.20 — used only for σ_o-weighted ACR fits; dropped from BT
* **F**: weight = 0.00 — failed any hard gate (see §5)

## 5. Hard gates (immediate `weight = 0`, `quality_grade = F`)

These are unrecoverable session-killers. Borrowed from CID22 with adjustments
for Squintly's mobile-first, 4-tier ACR + pairwise mix:

1. **Golden-low fail.** A reference vs reference pair (or a q95+ vs q95+
   single) where the rater says "broken" / "annoying" / "very annoying"
   on the 4-tier. CID22 disqualifies if a near-lossless image gets 4-tier
   ≤ "low quality" [CID22 §Participant screening].
2. **Golden-high fail.** A heavily-distorted single (mozjpeg q5) rated
   "looks fine" or a reference-vs-broken pair where the broken one
   "wins". CID22 calls this "very poor image score above 5".
3. **Sliding-default trap.** > 20% of 4-tier responses have
   `dwell_ms < 1500 ms AND choice == default_button`. CID22's "more than
   20 percent of the responses of the session was exactly the score of 5,
   which corresponds to the initial position of the slider" applied to
   our 4-tier means: same button repeated with sub-floor RTs.
4. **Mobile/desktop mismatch.** Session declared desktop in qualifier but
   `pointer_type = 'touch'` in trial responses (CID22's discard rule).
   Squintly inverts this — we *want* mobile — but the principle holds:
   any qualifier-vs-trial mismatch kills the session.
5. **First-three discard.** First 3 trials of every session are dropped
   regardless of grade [CID22] — they are warm-up.
6. **Session count cap.** Same observer_id contributing > 4 sessions
   triggers human review (CID22's "up to 4 sessions, with a 24-hour break
   between sessions"). Don't auto-reject; gate on review.

## 6. Inline (per-trial) flags

Recorded in a new `response_flags` column (TEXT, comma-separated).
Cheap to compute, useful for live dashboards:

| Flag | Trigger | Action |
|------|---------|--------|
| `rt_too_fast` | `dwell_ms < 600` for pair, `< 800` for 4-tier single | Count toward grade only; don't drop trial |
| `rt_too_slow` | `dwell_ms > 60_000` | Count toward grade |
| `no_reveal` | `kind == 'pair' AND reveal_count == 0` | Pair without ever flickering. CID22 disqualifies sessions with "<2 switches" — count these |
| `golden_fail` | `is_golden == 1 AND choice mismatches expected` | Counted in `golden_pass_rate` |
| `viewport_clipped` | `image_displayed_w_css < intrinsic_w / 4` (image displayed at < 25% native) | Likely misconfigured device — don't reject the trial but flag for analysis |
| `zoom_zero_distortion` | `zoom_used == 1 AND a_quality == b_quality` | Burned a zoom on identical images — fine but informative |

Critical: **600 ms RT floor is a soft floor, not a hard gate.** Meade &
Craig found "no clear break point in the distribution, as individual
differences in response time were very large" [§RQ6]. We use it as one
of several signals, never alone.

## 7. Golden anchor design

CID22 used 2 honeypots per 30-question session (~6.7%). KonIQ used hidden
test questions throughout with a 70% pass rate (lost 6% of contributors).
For Squintly:

* **Density**: 1 golden in every 12 trials (~8%). Higher than CID22 because
  our trials are shorter, lower than would let observers learn the pattern.
* **Mix per session**: ≥ 2 *unanimous* (reference vs broken q5) and
  ≥ 1 *near-unanimous* (q95 vs reference, expected "tie" or "a"). All
  three must be present before grade is final.
* **Unanimous threshold**: derived from CID22 anchors — q30 mozjpeg vs
  reference is "MCOS ≤ 50" (medium quality) on a 0–100 scale. We treat
  q5–q15 as guaranteed-fail anchors.
* **Pass rate floor**: 70% on goldens, matching KonIQ's quiz threshold.
  Below that → grade F.
* **Active-learning interaction**: goldens are sampled *outside* the
  active-learning queue. ASAP picks information-maximising pairs from
  the *non-golden* pool; goldens are inserted by a fixed-density rule.
  Otherwise active learning would learn to avoid goldens (they are
  low-information for scale fitting).

## 8. Qualifier quiz: yes, 60 seconds

KonIQ's qualifier dropped 24% (553 of 2,302 workers; 1,749 passed).
That 24% is in addition to the 6% who failed in-task gates. Cost-benefit:

* 60 s × 25%-rejected = 15 s wasted per failed observer
* without it, those observers contribute 5–10 minutes of bad data we'd
  have to filter post-hoc, plus they still partially poison the ASAP
  posterior before being caught

**Squintly qualifier (60 seconds, 6 questions):**

1. Two reference-vs-q5 4-tier judgements (must rate q5 as "very annoying")
2. One reference-vs-reference pair (must rate "tie")
3. One q90 vs q40 pair (must pick q90)
4. One IMC: "for this question only, please tap 'tie'" [Meade & Craig
   IMC pattern]
5. One self-report: "are you on a phone right now?" cross-checked against
   `pointer_type`

Pass = 5 of 6 correct (no IMC failures). Failures don't disqualify
permanently — let them retry once.

## 9. Pipeline summary

```
[per-trial]   inline flags written to response_flags
              if hard-gate triggered (rt+default-button, golden-broken,
              etc.) → set session.flagged_terminate, stop serving trials

[session-end] compute session-level stats:
                - first 3 trials dropped
                - golden_pass_rate
                - rt_floor_pass
                - straight_line_max
              write to sessions.session_grade {pass, soft_fail, hard_fail}

[BT fit time] for each observer with N≥20 trials:
                - fit Thurstone Case V scale on all observers
                - per-observer leave-one-out likelihood
                - compute pwcmp_dist_L, CID22 norm_diff, even-odd r
                - assign quality_grade and weight
              re-fit BT scale weighting comparisons by sqrt(weight_o · weight_o')

[ACR fit]    use Pérez-Ortiz 2019 unified model:
                π_ik = m_i + δ_k + ξ_ik
              fit (δ_k, σ_k) per observer with N≥30 ACR ratings
              weight = 1 / max(σ_k², 0.5)  capped at weight ≤ 2.0

[nightly]    cross-session aggregation:
                - same observer_id across sessions: average grades
                - downgrade observers with one F session to D
                - upgrade observers with 3+ A sessions to "trusted" pool
                  for active-learning pair selection (give their judgements
                  more weight in ASAP info-gain)
              write to observer_grades table

[export]     responses.tsv includes per-row weight = session_weight ·
             observer_weight. Downstream training code multiplies the
             likelihood by this weight.
```

## 10. Schema additions

```sql
ALTER TABLE observers ADD COLUMN qualifier_passed INTEGER;
ALTER TABLE observers ADD COLUMN qualifier_score INTEGER;     -- 0..6
ALTER TABLE observers ADD COLUMN trusted_pool INTEGER NOT NULL DEFAULT 0;

ALTER TABLE sessions ADD COLUMN session_grade TEXT;            -- 'A'..'F'
ALTER TABLE sessions ADD COLUMN session_weight REAL NOT NULL DEFAULT 1.0;
ALTER TABLE sessions ADD COLUMN flagged_terminate INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN golden_pass_rate REAL;
ALTER TABLE sessions ADD COLUMN straight_line_max INTEGER;
ALTER TABLE sessions ADD COLUMN rt_below_floor_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN no_reveal_count INTEGER NOT NULL DEFAULT 0;

ALTER TABLE responses ADD COLUMN response_flags TEXT;          -- comma-sep
ALTER TABLE responses ADD COLUMN expected_choice TEXT;         -- for goldens

CREATE TABLE observer_grades (
    observer_id          TEXT PRIMARY KEY REFERENCES observers(id),
    computed_at          INTEGER NOT NULL,
    n_trials             INTEGER NOT NULL,
    n_sessions           INTEGER NOT NULL,
    quality_grade        TEXT NOT NULL,                        -- 'A'..'F'
    weight               REAL NOT NULL,                        -- 0..1
    pwcmp_log_lik        REAL,
    pwcmp_dist_l         REAL,
    cid22_mean_norm_diff REAL,
    cid22_sd_norm_diff   REAL,
    even_odd_r           REAL,
    sigma_acr            REAL,                                 -- per-obs σ_o
    delta_acr            REAL,                                 -- per-obs bias
    golden_pass_rate     REAL,
    notes                TEXT
);

CREATE INDEX idx_observer_grades_grade ON observer_grades(quality_grade);
```

## 11. Sanity-check rules (run nightly, alert if violated)

* No more than 30% of observers should be grade F. CID22 saw ~14% TSBPC
  rejection. Mobile is harder, so we expect 20–25%, but >30% means our
  goldens or floors are too strict.
* Inter-grade BT scale should converge: re-fitting using only A+B
  observers vs A+B+C should give ≤ 0.1 JOD shifts. Larger means our
  weighting is doing real work — log per-image deltas.
* SOS hypothesis check: per-image variance(MOS) should follow `α·MOS·(1-MOS)`
  with α ∈ [0.01, 0.22] [HSE]. Way outside that range → calibration drift
  or anchor failure.
* Per-image reference floor: if a reference (q95+) gets MCOS < 80 it's
  either a goldens failure or our reference image is bad — investigate.

## 12. Implementation pointers

* `src/grading/thurstone.rs` — pw_scale.m port, L-BFGS via `argmin`
* `src/grading/loo_likelihood.rs` — leave-one-out per observer
* `src/grading/inline_flags.rs` — runs in the response-write path
* `src/grading/session_grade.rs` — runs at session-end
* `src/grading/observer_grade.rs` — runs nightly
* `src/grading/qualifier.rs` — 6-question gate at observer creation

Tests: replay CID22 numbers (14.7% TSBPC, 6.5% DSBQS rejection) on
synthetic data with known fractions of careless responders to verify the
filter recovers them. Use Meade & Craig's mixture-model approach: 89%
careful, 9% inconsistent, 2% straight-liners. The Mahalanobis D index is
the gold-standard sensitive single index for total carelessness; even-odd
is best for partial carelessness [Meade & Craig Tables 10–11].
