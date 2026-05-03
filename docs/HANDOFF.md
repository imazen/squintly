# Squintly — Handoff

Last full pass: 2026-05-01.
Live: https://squintly-production.up.railway.app
Repo: https://github.com/imazen/squintly

This document is the complete pickup sheet. If you can read this and only
this, you should be able to continue work, ship features, debug
production, integrate coefficient, and understand every methodology
decision back to its citation.

The other primary documents are read-once-and-keep-handy:

- `SPEC.md` — the original design (some sections superseded by methodology.md)
- `docs/methodology.md` — the contract: every parameter and its rationale
- `docs/participant-grading.md` — outlier detection at three layers
- `docs/motivation-and-compensation.md` — engagement, corpus, compensation
- `DEPLOY.md` — Railway-specific deployment walkthrough
- `CHANGELOG.md` — what shipped when
- `CLAUDE.md` — agent notes for future sessions

---

## 1. What Squintly is, in one paragraph

Squintly is a phone-first browser app that collects pairwise and 4-tier
ACR psychovisual judgments about compressed images, records the viewing
conditions under which each judgment was made, and exports the result in
schemas suitable for training a successor to SSIMULACRA 2 (zensim). It
runs as a single Rust binary (axum + sqlx + SQLite + embedded Vite TS
frontend) on Railway, talks to coefficient (the imazen codec-benchmark
store) over HTTP for image manifests and bytes, and ships every methodology
choice from CID22 (Sneyers, Ben Baruch, Vaxman 2023) it can without
losing the phone-first audience.

The data is collected with two intertwined objectives: (a) **relative
scoring** via Bradley–Terry-Davidson on pairwise comparisons, producing
per-(encoding, condition_bucket) quality scores comparable to CID22 MCOS;
and (b) **threshold calibration** via 4-tier ACR + Levitt 1971 adaptive
staircases, producing per-(source, codec, condition_bucket) imperceptibility,
annoyance, and "I hate it" thresholds — the data an encoder picker
actually wants. A unified Pérez-Ortiz et al. 2019 fit produces a single
MCOS-equivalent scale that combines both.

The viewing-condition record on every trial — devicePixelRatio, intrinsic-
to-device-pixel ratio, ambient-light bin, viewing distance from a Li 2020
virtual chinrest, color gamut, orientation — is what existing public IQA
datasets (CID22, KADID-10k, TID2013, KonIQ-10k) don't capture. That's the
raison d'être.

---

## 2. Why does this exist

Existing IQA datasets fix viewing conditions and bake them into the
labels. CID22 forced desktop with images displayed at "dpr1" (one image
pixel = one CSS pixel = 2×2 device pixels on a retina screen). That's
fine if the metric will be deployed on a desktop. It's not fine if the
metric needs to predict perceived quality at dpr=3 on a 5″ phone held at
~30 cm where one image pixel might map to a third of a CSS pixel and
chroma artifacts that are catastrophic at 1:1 are completely invisible.

zensim plateaus around SROCC 0.82 on KADID/TID. The residual is not
random noise; it's mostly the gap between training-time viewing
conditions and deployment-time viewing conditions. A condition-aware
metric should clear that ceiling.

Squintly's value-add is not the protocol (CID22 has the protocol) and not
the scale-fitting (pwcmp + Pérez-Ortiz 2019 already exist). It's the
dpr × viewing-distance × intrinsic-to-device-ratio × content × codec
coverage that you cannot collect on a controlled-desktop study.

---

## 3. Architecture in one screen

```
┌──────────────────┐         ┌──────────────────┐         ┌──────────────────┐
│   Observer's     │ HTTPS   │     Squintly     │  HTTP   │   coefficient    │
│   browser        │◀───────▶│   (Rust+axum)    │◀───────▶│   (image store)  │
│   (phone first)  │         │   on Railway     │         │   somewhere      │
└──────────────────┘         └────────┬─────────┘         └──────────────────┘
                                      │
                                      │  SQLite at /data/squintly.db
                                      │
                                ┌─────▼─────┐
                                │  Volume   │
                                │ (Railway) │
                                └───────────┘
```

The browser:
1. Loads the embedded TS frontend (built by Vite, served by rust-embed).
2. Probes codec support (JXL/AVIF/WebP) with 1×1 base64 test blobs.
3. Calibrates: credit-card resize → CSS-px-per-mm; optional blind-spot
   sweep → viewing distance.
4. Captures session-level conditions (dpr, screen size, gamut, OS, …).
5. Runs the 5-trial onboarding calibration with answer feedback.
6. Loops: GET /api/trial/next → render → POST /api/trial/{id}/response
   → repeat. Records per-trial intrinsic_to_device_ratio,
   image_displayed_w_css, viewport, orientation, dwell, reveal_count.
7. Optional: email-link sign-in to attach an account → cross-device
   resume.

Squintly server:
1. On startup loads the SQLx migrations (six of them now), the manifest
   from coefficient, the corpus_anchors pool, and the source_flags map.
2. Serves trial requests by sampling the manifest with monotonicity-aware
   pair selection, anchor reservation (30%), and honeypot insertion (1
   in 12). Filters by the session's supported_codecs.
3. Records every response with inline grading flags.
4. At session-end, computes the geometric-mean session_weight and grade.
5. Streams TSV exports: pareto.tsv, thresholds.tsv, unified.tsv,
   responses.tsv. Bootstrap-CI 200 iterations on the derived columns.

Coefficient (the missing piece):
- Owns the image corpus and the encoded variants.
- Exposes GET /api/manifest, GET /api/sources/{hash}/image, GET
  /api/encodings/{id}/image.
- Does not yet exist as a deployed Railway service; that's the biggest
  open task. See §10.

---

## 4. Repository structure

```
~/work/squintly/                                  ← workspace root
├── Cargo.toml                                    ← Rust manifest
├── Cargo.lock                                    ← committed
├── Dockerfile                                    ← three-stage Node→Rust→runtime
├── .dockerignore
├── railway.toml                                  ← deploy config
├── justfile                                      ← common ops
├── README.md
├── DEPLOY.md                                     ← Railway walkthrough
├── SPEC.md
├── CHANGELOG.md
├── CLAUDE.md                                     ← agent notes
├── docs/
│   ├── HANDOFF.md                                ← this file
│   ├── methodology.md                            ← parameter contract
│   ├── participant-grading.md                    ← outlier detection
│   ├── motivation-and-compensation.md            ← engagement / corpus / paid
│   └── motivation-research.md                    ← (if present)
├── migrations/
│   ├── 0001_init.sql                             ← core schema
│   ├── 0002_grading.sql                          ← outlier detection columns
│   ├── 0003_engagement.sql                       ← streaks, themes, badges
│   ├── 0004_codec_support.sql                    ← supported_codecs CSV
│   ├── 0005_auth.sql                             ← magic-link tokens
│   └── 0006_v02_rigor.sql                        ← anchors, calibration, held_out
├── src/                                          ← server crate
│   ├── main.rs                                   ← CLI, routes, startup
│   ├── lib.rs                                    ← re-exports modules
│   ├── handlers.rs                               ← all HTTP handlers + AppState
│   ├── coefficient.rs                            ← Coefficient trait + Http/Fs/Disabled
│   ├── sampling.rs                               ← pick_trial, anchors, honeypots
│   ├── grading.rs                                ← inline flags + session grade
│   ├── streaks.rs                                ← day-streak math + milestones
│   ├── auth.rs                                   ← magic-link tokens, Resend
│   ├── staircase.rs                              ← Levitt 1971 transformed up-down
│   ├── bt.rs                                     ← Bradley-Terry-Davidson + monotonicity
│   ├── unified.rs                                ← Pérez-Ortiz 2019 joint fit
│   ├── stats.rs                                  ← bias, bootstrap, disagreement
│   ├── asap.rs                                   ← Mikhailiuk 2020 EIG sampling
│   ├── export.rs                                 ← TSV streaming
│   └── db.rs                                     ← tiny query helpers
├── tests/
│   └── smoke.rs                                  ← end-to-end Rust smoke
├── web/                                          ← frontend
│   ├── package.json
│   ├── tsconfig.json
│   ├── vite.config.ts
│   ├── playwright.config.ts
│   ├── index.html
│   ├── src/
│   │   ├── main.ts                               ← entrypoint
│   │   ├── api.ts                                ← typed HTTP client
│   │   ├── style.css
│   │   ├── conditions.ts                         ← dpr/viewport/etc capture
│   │   ├── codec-probe.ts                        ← JXL/AVIF/WebP probes
│   │   ├── calibration.ts                        ← credit-card chinrest
│   │   ├── calibration-onboarding.ts             ← 5-trial onboarding
│   │   ├── trial.ts                              ← main rating loop
│   │   ├── auth-modal.ts                         ← email sign-in modal
│   │   └── (no other files)
│   └── e2e/
│       ├── mock-coefficient.ts
│       ├── global-setup.ts / global-teardown.ts
│       ├── helpers.ts
│       ├── welcome.spec.ts
│       ├── calibration.spec.ts
│       ├── trial.spec.ts
│       ├── api.spec.ts
│       ├── codec-filter.spec.ts
│       └── auth.spec.ts
└── target/  /  web/node_modules/  /  web/dist/   ← build outputs (gitignored)
```

JJ + Git colocated mode. Branch `main` tracks `origin/main` on
github.com/imazen/squintly. Commits go straight to main; no feature
branches. See CLAUDE.md root for the working-on-main rationale.

---

## 5. The two objectives, explained in detail

### 5.1 Relative scoring (pairwise → BT-Davidson → MCOS-equivalent)

For a fixed source image and condition_bucket, every encoding has a
latent quality. Observers rate pairs (which is closer to the original?)
producing tournament-style outcomes. We fit β_i for each encoding by
maximum-likelihood Bradley-Terry-Davidson with ties (Davidson 1970):

```
P(a > b) = exp(β_a) / (exp(β_a) + exp(β_b) + ν · exp((β_a+β_b)/2))
P(a ~ b) = ν · exp((β_a+β_b)/2) / Z
P(a < b) = exp(β_b) / Z
```

Plus a Gaussian prior on β with σ=1.5 (≈ 1.5 JOD) to regularise. Anchor
β_reference = 0. β maps to 0–100 quality with `100 + (β - β_ref) × 10`,
clamped.

**Critical: monotonicity injection.** Before fitting, for every same-
codec ordered pair (low_q, high_q), we inject **200 dummy "high beats
low" comparisons.** CID22 measured this single decision as the largest
single-step improvement in their pipeline (KRCC 0.99 with vs 0.56
without). It encodes the basic constraint "same codec, higher quality
setting → at least as good"; without it, sparse/noisy sampling at q40
vs q30 can produce inverted ranks.

Implementation: `bt::with_monotonicity(comparisons, monotone_pairs, 200)`,
called inside `pareto_tsv()` per-bucket.

### 5.2 Threshold calibration (4-tier ACR → staircase + logistic)

Phone observers rate ONE image at a time on:

| Tier | Caption | Definition |
|---|---|---|
| 1 | Imperceptible | I can't tell this from the reference. |
| 2 | I notice | I can see something is off, but it's fine. |
| 3 | I dislike | The artifacts bother me. |
| 4 | I hate | This is unacceptable. |

For each (session, source, codec, threshold-target), an adaptive Levitt
1971 transformed up-down staircase converges on:

- **3-down-1-up** → P=0.794 → `q_notice` (most observers stop calling it imperceptible)
- **2-down-1-up** → P=0.707 → `q_dislike`
- **1-down-1-up** → P=0.500 → `q_hate`

Step size halves at each reversal until reaching min_step (1 grid step).
Convergence = 8 reversals at min_step. Estimate = mean of the last 6
reversal qualities (drop the first as biased by start point).

Implementation: `src/staircase.rs`. The online estimate is per-session;
the offline estimate (in `thresholds.tsv`) does logistic interpolation
across all observers per (source, codec, condition_bucket), with
bootstrap CIs.

### 5.3 Unified scale (Pérez-Ortiz 2019)

The pairwise and rating data live on different scales. CID22 fits each
separately, then aligns by polynomial. Pérez-Ortiz 2019 fits both
*simultaneously* with a joint likelihood — and gives you per-observer
bias and noise as a free byproduct.

Model:
- Pairwise (Thurstone Case V): P(i > j) = Φ((m_i − m_j) / (√2 σ))
- Rating (cumulative-link 4-tier): P(rating ≤ k | i, observer o) =
  Φ((τ_k − m_i − δ_o) / σ_o)

Latent quality m anchored at m_reference = 0. Per-observer δ_o
(additive bias; lenient observer δ < 0) and σ_o (rating noise; precise
observer σ < 1). Three category thresholds τ_1 < τ_2 < τ_3 (the cuts
between 1↔2, 2↔3, 3↔4). Global pairwise σ.

Gaussian priors: σ on m=1.5, σ on δ=0.5, σ on log-σ_o=0.5. Gradient
descent with lr decay; tau monotonicity enforced after each step.

Implementation: `src/unified.rs::fit_unified()`. Output column in
`unified.tsv`. **This is the v0.2 contribution that does not exist
in CID22's pipeline** and is the cleanest single MCOS-equivalent score
to feed downstream.

---

## 6. Methodology — the magic numbers and where they come from

Every parameter referenced here lives in a function call somewhere; the
methodology doc is the contract that says what they must be. **If you
change a number in code without updating `docs/methodology.md`, that's a
methodology bug.**

### 6.1 Sampling

| Parameter | Value | Source |
|---|---|---|
| `p_single` (single-stim trial probability) | 0.65 | Squintly default; thresholds need denser sampling per bucket |
| `p_honeypot` | 0.083 (1 in 12) | CID22 used 2/30 = 0.067; we use slightly higher because phone sessions are shorter |
| `p_anchor` | 0.30 | CID22 §Anchors: ~30% session reservation |
| Quality-grid bias | 60% from lower half | Source-informing-sweep rule (CLAUDE.md): web codecs ship at low q |
| Trivial-pair threshold (same codec) | quality gap > 30 | CID22 §Selection of stimuli |
| Trivial-pair threshold (cross-codec) | bytes ratio > 4× | CID22: "0.5 bpp JPEG vs 1.5 bpp AVIF was considered trivial" |

### 6.2 Outlier detection

| Parameter | Value | Source |
|---|---|---|
| RT floor (single) | 800 ms | Meade & Craig 2012 §RQ6; phone-tuned upward |
| RT floor (pair) | 600 ms | Same; pair trials are faster intrinsically |
| Golden pass-rate floor | 0.70 | KonIQ-10k qualifier verbatim |
| Line-clicker ratio threshold | 1.5 (target), 2.5 (fail) | KonIQ "max-count-ratio" verbatim |
| Even-odd Spearman threshold | 0.30 / 0.50 | Meade & Craig Table 10 |
| Drop first-N trials | 3 | CID22 verbatim |
| Session F-rate target | ≤ 15% | CID22 actual was 14.7% |
| pwcmp dist_L threshold | 1.5 | pwcmp `pw_outlier_analysis.m` |

### 6.3 Score construction

| Parameter | Value | Source |
|---|---|---|
| BT prior σ on β | 1.5 | Pérez-Ortiz 2017 (≈ 1.5 JOD) |
| Monotonicity dummy count per pair | 200 | CID22 verbatim |
| Quality scale factor (β → 0–100) | × 10 | display convention; downstream rescales |
| Bootstrap iterations | 200 | CID22 verbatim |
| CI percentiles | 5th, 95th | CID22 90% CI |
| Unified prior σ on m | 1.5 | Pérez-Ortiz 2019 §IV |
| Unified prior σ on δ | 0.5 | Pérez-Ortiz 2019 §IV |
| Unified prior σ on log-σ_o | 0.5 | Pérez-Ortiz 2019 §IV |
| Disagreement multiplier (disjoint CIs) | 20 | CID22 §MCOS disagreement |
| Disagreement multiplier (overlap ties) | 200 | CID22 §MCOS disagreement |

### 6.4 Engagement

| Parameter | Value | Source |
|---|---|---|
| Streak freeze cadence | 1 per week | Duolingo telemetry: -21% churn at-risk |
| Trial-count milestones | 1, 10, 50, 100, 250, 500, 1000 | Squintly defaults; reasonable rewards |
| Calibration soft-fail threshold | 0.60 | KonIQ used 0.70 hard-fail; we soft-fail to keep phone observers |
| Onboarding trial count | 5 | CID22 used 4; we add an IMC for inattention |
| Forced-break cadence | every 25 trials | Sessions ≤ 30 min standard (BT.500-15) |
| Hard cap on counted trials/session | 100 | Anti-fatigue |

### 6.5 Conditions

| Parameter | Value | Source |
|---|---|---|
| Calibration card width | 85.6 mm × 53.98 mm | ISO/IEC 7810 ID-1 (credit/debit card) |
| Blind-spot eccentricity | 13.5° | Li 2020 virtual chinrest |
| Codec-probe TTL | 7 days in localStorage | Squintly default; flag toggles ≤ weekly |

### 6.6 Auth (when enabled)

| Parameter | Value | Source |
|---|---|---|
| Magic-link token length | 32 bytes (64 hex chars) | Weaver `convex/auth.ts` verbatim |
| Token TTL | 15 minutes | Weaver verbatim |
| Token storage | BLAKE3 hash only | Weaver verbatim — plaintext in URL only |
| Session TTL | 30 days | Weaver verbatim |
| Resend default sender | "Squintly <onboarding@resend.dev>" | sandbox-safe; override with RESEND_FROM_EMAIL |

---

## 7. Schema walkthrough

Every migration is irreversible. Existing data must not break under
schema changes — all ALTERs use `ADD COLUMN ... DEFAULT ...`. Read each
migration end-to-end before adding a 7th.

### 7.1 `0001_init.sql` — core

- **observers** (`id`, `created_at`, `user_agent`, `age_bracket`,
  `vision_corrected`)
- **sessions** — observer_id FK, started_at/ended_at, all condition
  fields (dpr, screen_w/h, color_gamut, dynamic_range_high, prefers_dark,
  pointer_type, timezone, viewing_distance_cm, ambient_light, css_px_per_mm)
- **trials** — session_id FK, kind ('single'|'pair'), source_hash,
  a_/b_ encoding/codec/quality/bytes, intrinsic_w/h, staircase_target,
  served_at
- **responses** — trial_id PK, choice, dwell_ms, reveal_count/ms_total,
  zoom_used, viewport_w/h, orientation, image_displayed_w/h_css,
  intrinsic_to_device_ratio, pixels_per_degree, responded_at
- **staircases** — id PK, session/source/codec/target/rule, started_at,
  converged, converged_quality, reversals

### 7.2 `0002_grading.sql` — outlier columns

- observers: `qualifier_passed`, `qualifier_score`, `trusted_pool`
- sessions: `session_grade`, `session_weight`, `flagged_terminate`,
  `golden_pass_rate`, `straight_line_max`, `straight_line_ratio`,
  `rt_below_floor_count`, `no_reveal_count`, `even_odd_r`, `n_trials`,
  `n_pair_trials`, `graded_at`
- trials: `expected_choice` (for goldens)
- responses: `response_flags` (CSV)
- new: **observer_grades** (cross-session aggregate; v0.3 batch will
  populate; v0.2 just the schema)

### 7.3 `0003_engagement.sql` — streaks/themes/badges

- observers gain: `account_tier`, `email`, `email_verified_at`,
  `display_name`, `streak_days`, `streak_last_date`, `freezes_remaining`,
  `skill_score`, `total_trials`, `compensation_mode`, `charity_choice`,
  `weekly_digest_optin`, `named_credit_optin`, `gdpr_consent_at/version`,
  `data_region`
- new tables:
  - **corpus_themes** (slug, display_name, description, is_default,
    is_wow, enabled). **Pre-seeded with 'nature', 'art', 'in_the_wild'.**
  - **corpus_image_themes** (source_hash, theme_slug)
  - **badges** + **observer_badges** — pre-seeded with 11 milestones
- sessions gain: `theme_slug`, `counted_trials`, `soft_capped_at_trial`

### 7.4 `0004_codec_support.sql` — codec probe results

- sessions: `supported_codecs` (CSV), `codec_probe_cached`

### 7.5 `0005_auth.sql` — magic-link tokens

- new tables: **auth_tokens** (token_hash PK, email, requesting_observer_id,
  expires_at, consumed_at), **observer_aliases** (alias_id, canonical_id,
  merged_at)
- index on `observers(email) WHERE email IS NOT NULL`

### 7.6 `0006_v02_rigor.sql` — anchors, held-out, calibration

- new tables:
  - **corpus_anchors** (source_hash, encoding_id, codec, quality, role,
    expected_choice, notes). **Empty by default — must be seeded.**
  - **source_flags** (source_hash, held_out, codec_version_blob, notes).
    **Empty by default — must be seeded.**
  - **calibration_pool** (id, kind, description, source_url, a/b_url,
    a/b_codec, a/b_quality, intrinsic_w/h, expected_choice, feedback_text,
    order_hint). **Empty by default — must be seeded.**
  - **calibration_responses** (id, observer_id, session_id, pool_id,
    choice, correct, dwell_ms, served_at, responded_at)
- observers gain: `calibrated`, `calibration_score`, `calibrated_at`
- trials gain: `held_out`

**The crucial v0.3 task is seeding the three new "Empty by default"
tables.** Until those have content, the app runs but the methodology
features (anchors, honeypots, calibration, held-out validation) all
no-op silently.

---

## 8. Module-by-module code tour

### 8.1 `src/main.rs`

CLI entry. Parses `--coefficient-http`, `--coefficient-path`, `--db`,
`--bind`. Resolves `PORT` env var (Railway) over `--bind`. Initializes
the SQLx pool, runs migrations, loads the manifest (non-fatal — warns and
continues with empty), loads the anchor pool and source flags. Builds the
axum router with all routes, mounts under `/api`, embeds the Vite-built
frontend via rust-embed at `/`. Spawns the listener.

**Don't change the path of the embedded frontend** without updating the
Dockerfile too — `web/dist/` must exist at the time of `cargo build`.

### 8.2 `src/handlers.rs`

The big file. ~900 lines. All HTTP handlers, plus:
- `AppState` — shared SQLx pool + coefficient + manifest RwLock + anchors
  RwLock + source_flags RwLock.
- `load_anchor_pool()` and `load_source_flags()` — reload helpers (called
  at startup and on `POST /api/manifest/refresh`).
- `AppError` — enum with variants for 404/409/400/503/500, derived from
  thiserror. Note that 503 specifically is used for "Resend not
  configured" — so the auth modal can recognize it and show the right
  copy.
- `award_badge()` — small helper for inserting into `observer_badges`,
  ignoring duplicates.

Sections (top-to-bottom):
1. AppState struct and load helpers
2. Session create/end (advances streak, awards milestone)
3. Trial fetch/respond (sampling, inline grading, milestone)
4. Static-image proxies
5. Magic-link auth (start, verify, HTML response page)
6. Onboarding calibration (list, response, finalize)
7. Observer profile
8. Stats and refresh
9. Static frontend serve via rust-embed

### 8.3 `src/coefficient.rs`

Three impls of the Coefficient trait:
- `HttpCoefficient` — talks to a coefficient viewer
- `FsCoefficient` — reads SplitStore directly (`meta/`+`blobs/`)
- `Disabled` — empty manifest, errors on fetch

The wire format tolerates **two naming conventions** (CID22-era
"codec_name" and "encoded_size" vs newer "codec" and "bytes"). Sources
are matched by a SHA-256 hex hash. Encodings carry an opaque `id`
string.

When integrating with a real coefficient deployment, the only thing that
matters is that those three GETs return the documented JSON / bytes.

### 8.4 `src/sampling.rs`

`pick_trial(manifest, cfg, allowed_codecs, anchors, flags)` is the
heart. The order of operations:
1. **Honeypot?** Roll p_honeypot. If a honeypot exists in `anchors.honeypots`
   that matches `allowed_codecs`, return it. `is_golden=true`,
   `expected_choice=Some(...)`.
2. **Anchor?** Roll p_anchor. If an anchor in `anchors.anchors`
   matches, return it as a normal trial.
3. **Random source.** Shuffle `manifest.sources`. For each source,
   group its encodings by codec, filter by `allowed_codecs`, pick a
   trial type (single biased toward thresholds), find a non-trivial
   pair if needed (up to 8 retries).

`is_trivial_pair()` filters out same-codec pairs with quality gap > 30
and cross-codec pairs with byte ratio > 4×.

`codec_browser_family()` maps "mozjpeg" / "rav1e" / "zenjxl" to "jpeg" /
"avif" / "jxl" so the codec_probe filter can work on either canonical or
derived names.

### 8.5 `src/grading.rs`

Per-trial inline flags: rt_too_fast/slow, no_reveal, golden_fail,
viewport_clipped. Per-session aggregate via `grade_session()`: drops the
first 3 trials, computes golden_pass_rate, straight_line_max,
line-clicker ratio, even-odd r (Pearson), no-reveal frac. Composites via
geometric mean → A/B/C/D/F.

The geometric mean is **product, then `.powf(1/n)`** — not log/exp/avg.
A single zero zeroes the weight; that's by design. If you change the
sub-score weights, update methodology.md §4.2.

### 8.6 `src/streaks.rs`

Day-streak math. Pure function `advance_streak(prev_state, today_local_date)`
returning `(StreakState, StreakOutcome)`. The frontend sends `local_date`
as ISO `YYYY-MM-DD`; the server doesn't try to know what day it is in
the observer's TZ. **This is deliberate** — chrono-tz is heavy and the
client always knows its own date.

Freezes auto-bridge a 2-day gap once a week. 3+ day gaps reset to 1.
Crossing a milestone awards a badge atomically.

### 8.7 `src/auth.rs`

32 random bytes from `OsRng`, hex-encoded → 64-char token. Persist only
the BLAKE3 hash. 15-minute TTL, single-use. Send via Resend HTTP API
(`api.resend.com/emails`) with reqwest. Sandbox-safe default sender.

`looks_like_email()` is a loose RFC-5322-ish check. Don't tighten this
into a real validator — the magic link itself is the verification.

### 8.8 `src/staircase.rs`

`Staircase::new(target, grid)` constructs an idle one starting at the
high-q end (so the first stimulus is "imperceptible" for most observers).
`step(rating)` updates state, returns next quality (or None on
convergence).

The reversal estimate drops the first reversal because it's biased by
start point — standard psychophysics practice.

### 8.9 `src/bt.rs`

Bradley-Terry-Davidson with ties + Gaussian prior on β + monotonicity
dummy injection. Fit by gradient descent (not L-BFGS) — converges in
tens to low-hundreds of iterations for our scale; if you fit thousands of
items you'll want L-BFGS via `argmin`.

The gradient derivation is in `derive_grad()`. If you change the
likelihood (e.g., to handle a separate σ per pair), redo the gradients
or switch to `nalgebra-autograd`.

`with_monotonicity(comparisons, monotone_pairs, n_dummy)` is the
CID22-style dummy injection. **Don't lower n_dummy below 200 without
running the KRCC ablation** — CID22 measured the cliff between with/without.

### 8.10 `src/unified.rs`

Pérez-Ortiz et al. 2019 joint fit. Pairwise Thurstone Case V + ordinal
cumulative-link rating, joint gradient descent with priors. Gradient is
derived in `fit_unified()` — math in the docstring. Tau monotonicity is
enforced after each step by sort.

The `phi()` and `erf()` use the Abramowitz-Stegun 7.1.26 polynomial
approximation (max error ≈ 1.5×10⁻⁷). The clippy `excessive_precision`
lint is silenced on those functions because the canonical AS coefficients
have more digits than f32 can represent — the constants are
self-documenting and shouldn't be truncated.

### 8.11 `src/stats.rs`

Three primitives:
- `session_bias_offsets()` — per-session additive correction. Z-normalize
  scores per stimulus (vs group mean+std), then offset = -mean(z) per
  session. Lenient observers get negative offsets (pulled toward mean),
  harsh observers positive.
- `bootstrap()` — resample-with-replacement, callback per resample.
  Deterministic per `seed`.
- `ci90()` — 5th and 95th percentile.
- `disagreement_dummy_count()` and `overlap_tie_count()` — CID22 §MCOS
  disagreement mitigation hooks.

### 8.12 `src/asap.rs`

Mikhailiuk 2020 EIG-maximizing pair selection. `eig(beta, sigma, a, b)`
returns the binary entropy of P(a > b) under current MAP β. Maximized
when the prediction is closest to 0.5 — i.e., the least-decided pair.

`pick_max_eig(beta, sigma, candidates, rng)` chooses the argmax with
random tie-breaking.

**Not yet wired into runtime sampling.** The integration is non-trivial
because it requires maintaining a Gaussian/Laplace approximation to the
β posterior per (source, condition_bucket) in memory and updating after
every response. Three approaches:

1. **Lazy refit (simplest, slowest).** On every `pick_trial()` call,
   refit BT from the bucket's responses, then pick max-EIG candidate.
   O(N²) per pick where N = number of responses in bucket; with 1000
   responses that's still under 100 ms.
2. **Online posterior cache.** Maintain (β_map, σ_map) per bucket;
   update by one Newton step on each response.
3. **Batch updates.** Refit every K responses (say K=20); use stale β
   in between.

Recommend (1) for v0.3 — simplest, doesn't break existing tests, and the
performance is fine until ~10k responses per bucket.

### 8.13 `src/export.rs`

Three TSV streams. `pareto_tsv` uses BT-Davidson with monotonicity +
bootstrap CI. `thresholds_tsv` applies bias correction first, then
logistic interpolation + bootstrap. `unified_tsv` runs the joint
Pérez-Ortiz fit. `responses_tsv` dumps every trial with full conditions
for downstream researchers.

**The TSV header changes break downstream consumers.** If you add a
column, append it at the end and update `methodology.md`. Don't reorder.

### 8.14 `src/db.rs`

Trivial helpers. `now_ms()` returns Unix milliseconds. `count(pool, sql)`
runs a `SELECT COUNT(*)` and returns the i64.

---

## 9. Frontend architecture

### 9.1 Render flow

Single SPA. `web/src/main.ts` is the entrypoint. It:
1. Calls `detectCodecs()` (cached 7d).
2. Renders welcome screen with the conditional JXL banner.
3. On Begin → calibration if not yet done → profile form → session
   create → 5-trial onboarding → infinite trial loop.

There's no router. Every screen is `root.innerHTML = '...'` followed by
event listeners. Why: SPA routers add 100-300 KB and provide nothing for
a 5-screen sequential flow.

### 9.2 State

Three localStorage keys:
- `squintly:observer_id` — UUID v4. Persisted across sessions.
- `squintly:calibration` — `{ css_px_per_mm, viewing_distance_cm }`
- `squintly:profile` — `{ age_bracket, vision_corrected, ambient_light }`
- `squintly:codec_support_v1` — codec-probe cache

Plus session-level state inside the trial loop: trialCount,
revealCount/Total, dwell timer.

### 9.3 The trial loop

`web/src/trial.ts::startTrials(root, sessionId)`:
1. Fetch `/api/trial/next?session_id=...`.
2. `renderTrial()` mounts the full trial UI; the container has
   `data-trial-id="<uuid>"` so e2e tests can wait for next-trial render.
3. Single trials show one image with rating-panel (4 buttons).
4. Pair trials show the same image area with a tap-to-toggle A/B
   carousel + a 3-button pair-panel.
5. Hold-to-reveal: pointerdown → swap to reference URL; pointerup →
   swap back. Tracks revealCount + revealMsTotal.
6. Click button → `submit(choice)` → `recordResponse()` → fetch next
   → re-render. Race-safe by the `data-trial-id` change.

Every 25 trials → 30s break screen. Menu (top-right) → end session
or keep going.

### 9.4 The codec probe

`web/src/codec-probe.ts::detectCodecs()` runs three `Image()` decode
attempts on 1×1 base64 blobs (JXL, AVIF, WebP). Plus `jpeg` and `png`
in `ALWAYS_SUPPORTED`. 1500 ms timeout per probe so a hang on one
codec doesn't wedge the whole probe. Result cached in localStorage with
a 7-day TTL.

`jxlEnableHint()` returns browser-aware copy: Chromium gets the
`chrome://flags/#enable-jxl-image-format` URL; Firefox gets "we'll
skip JXL"; Safari gets "try iOS 17+".

### 9.5 The calibration flow

`web/src/calibration.ts`:
- Stage 1 (card resize): a `<div class="card-mock">` sized in CSS px;
  observer drags a slider 80..600 to match a real ID-1 card. Pixels-
  per-mm = slider_value / 85.6.
- Stage 2 (blind-spot sweep): a 320-px stage with a fixation × on the
  left and a red dot on the right. "Start sweep" animates the dot
  leftward at 60 fps over 8 s. Tap when the dot disappears (in your
  blind spot, ~13.5° from fixation). Distance = horizontal_mm /
  tan(13.5°).
- Both stages are skippable; we record `null`.

### 9.6 Onboarding calibration

`web/src/calibration-onboarding.ts::runCalibration()`. Fetches up to 5
items from `/api/calibration`, shows each, records the response, gives
1.4s of feedback (correct/expected), then advances. On done, calls
`/api/calibration/finalize` to compute the score and update
`observers.calibrated`. **Empty pool → silent skip** so the app works
the moment Squintly is deployed even without a seeded calibration_pool.

### 9.7 Auth modal

`web/src/auth-modal.ts::openSignInModal()`. Email field + Send link
button. Calls `/api/auth/start`. On 503 ("not configured") shows a
helpful message and stays in the modal (no anonymous loss). On success
shows the "check your inbox" message and auto-closes.

The verify path (`/api/auth/verify?token=...`) is server-rendered HTML
that writes the resolved observer_id to localStorage and redirects to
`/`.

### 9.8 Build / bundle

Vite emits ~23 KB of JS (gzipped: ~8 KB), 3.5 KB CSS, plus index.html.
No code-splitting; the whole app is one chunk. Build time ~100 ms after
initial.

---

## 10. Coefficient integration — the open task

This is the biggest pending item. Squintly works without coefficient
(empty manifest, frontend shows "no trials available"), but to actually
collect data it needs an image source.

### 10.1 What coefficient must provide

Three HTTP endpoints:

```
GET /api/manifest
  → 200 application/json
  {
    "sources": [
      {
        "hash": "<sha256 hex>",
        "width": 1920,
        "height": 1080,
        "size_bytes": 12345678,
        "corpus": "cid22-train",        // optional
        "filename": "img_001.png"        // optional
      }, ...
    ],
    "encodings": [
      {
        "id": "<opaque id>",            // any unique string
        "source_hash": "<sha256 hex>",
        "codec_name": "mozjpeg",        // or "codec"
        "quality": 60,                  // numeric, codec-specific scale
        "effort": 1.0,                  // optional
        "encoded_size": 142000          // or "bytes"
      }, ...
    ]
  }

GET /api/sources/{hash}/image
  → 200 image/png  (or whatever — the browser must natively decode it)

GET /api/encodings/{id}/image
  → 200 image/<format>  (server forwards the Content-Type)
```

Either canonical naming (`codec_name`, `encoded_size`) or alternate
(`codec`, `bytes`) is fine; `parse_manifest_json()` accepts both.

### 10.2 Three deployment options

**Option A: Co-deploy on Railway (recommended).**

Add a second Railway service in the squintly project. Both services
share the project's private network. Configure squintly with:
```
SQUINTLY_COEFFICIENT_HTTP=http://coefficient.railway.internal:PORT
```

Pros: cheapest (no public coefficient), low latency, no auth needed.
Cons: requires running coefficient — non-trivial since it has its own
PostgreSQL and GCS dependencies.

**Option B: Public coefficient with auth.**

Stand up coefficient publicly (its own Railway project) and configure
squintly with the public URL. Add bearer-token auth on the three
endpoints — squintly forwards an env-var bearer.

Pros: straightforward; coefficient becomes a reusable service.
Cons: public exposure of the image manifest; requires auth glue we
haven't written yet (squintly's HttpCoefficient doesn't currently send
auth headers).

**Option C: Static manifest + S3-hosted blobs.**

Skip coefficient HTTP entirely. Run a one-off coefficient export that
produces:
- `manifest.json` matching the expected JSON
- One file per source under `sources/<hash>.png`
- One file per encoding under `encodings/<id>.<ext>`

Upload all of it to an R2/S3 bucket. Configure a tiny "static"
HttpCoefficient that fetches from there.

Pros: simplest infra; no live coefficient. Read-only is fine.
Cons: stale manifests; reproducing an export is its own task.

### 10.3 Anchor seeding

After coefficient is reachable, populate `corpus_anchors`. CID22's
recipe per source:
- mozjpeg q30, q50, q70, q90
- libjxl q30, q60, q85
- AVIF aurora cq37, cq32, cq28
- Reference (the source itself, role='reference')

Plus honeypots: reference vs ~q5 mozjpeg (expected: 'b' for pair, '4' for
single rating).

A seed script will look something like:

```sql
INSERT INTO corpus_anchors (source_hash, encoding_id, codec, quality, role, expected_choice) VALUES
  ('<hash>', '<source>__mozjpeg__q30', 'mozjpeg', 30, 'anchor', NULL),
  ('<hash>', '<source>__mozjpeg__q50', 'mozjpeg', 50, 'anchor', NULL),
  -- ... 10 anchors per source
  ('<hash>', '<source>__mozjpeg__q5',  'mozjpeg', 5,  'honeypot', '4'),
  ('<hash>', '<reference>',            'png',     100,'reference', '1');
```

Write this as a Rust `tools/seed-anchors.rs` that reads the manifest and
emits one INSERT per source. Once written, run via `cargo run --bin
seed-anchors -- --db /data/squintly.db --coefficient-http <URL>`.

### 10.4 Held-out seeding

20% of sources, random but stable:

```sql
INSERT INTO source_flags (source_hash, held_out)
SELECT hash, 1 FROM (
  -- pick 20% deterministically by hash
  SELECT hash FROM <coefficient sources>
  WHERE substr(hash, 1, 1) IN ('0', '3') -- 2 of 16 hex chars = ~12.5%
);
```

Or write a Rust seed tool that picks based on a hash seed for stability.

### 10.5 Calibration pool seeding

5 entries, ideally:
1. Single, reference vs reference (control). expected_choice='1'.
2. Single, q15 mozjpeg (must be noticed). expected_choice in {'2','3','4'}.
3. Pair, q90 vs q40 same codec (must pick q90). expected_choice='a'.
4. IMC, "tap tie". expected_choice='tie'.
5. Single, near-lossless q95 (should be imperceptible). expected_choice='1'.

For (1)-(3) and (5) you need actual image URLs that point at coefficient.
For (4) it's purely text.

```sql
INSERT INTO calibration_pool (id, kind, description, source_url, a_url, expected_choice, feedback_text, order_hint) VALUES
  ('cal_imc_1', 'imc', 'For this question only, please tap "tie".', NULL, NULL, 'tie', 'Just checking you''re paying attention.', 1),
  ('cal_lossless_1', 'single', 'Near-lossless. How does this compare to a typical original?', '<source URL>', '<near-lossless encoding URL>', '1', 'This is q95 — visually identical to most observers.', 2),
  -- ...
```

### 10.6 Coefficient codec-version metadata (v0.3+)

`source_flags.codec_version_blob` is for future use — when coefficient
exposes encoder versions in its manifest, populate this with a JSON
`{ "mozjpeg": "4.1.0", "libjxl": "0.6.1", ... }`. Squintly carries it
into exports so downstream training can correlate quality differences
to specific encoder versions.

This isn't currently in the manifest schema; add it when coefficient is
ready. Won't break anything if absent.

### 10.7 First test after wiring

Once coefficient is reachable:

```bash
curl -X POST https://squintly-production.up.railway.app/api/manifest/refresh
curl https://squintly-production.up.railway.app/api/stats
# should show non-zero manifest_sources / manifest_encodings
```

Then open the frontend in a browser: it should serve real trials.

---

## 11. Deployment operations

### 11.1 Railway

Project at https://railway.com/project/3da5e21d-98a9-44a3-8db7-5707e570e76b
under "Lilith River's Projects" workspace. Single service `squintly`.
Volume mounted at `/data`; SQLite lives there. Healthcheck on
`/api/stats` with 60s timeout.

Domain: `squintly-production.up.railway.app`. To add custom:
```bash
railway domain --custom squintly.imazen.io
```

### 11.2 Env vars (all optional)

| Var | Purpose |
|---|---|
| `SQUINTLY_COEFFICIENT_HTTP` | Coefficient HTTP base URL |
| `SQUINTLY_COEFFICIENT_PATH` | Or filesystem path to a SplitStore |
| `SQUINTLY_DB` | SQLite path; defaults to `/data/squintly.db` in Docker |
| `SQUINTLY_BIND` | Local-bind override; on Railway PORT wins |
| `RUST_LOG` | Tracing filter; default `info,squintly=info` |
| `RESEND_API_KEY` | Enables email magic-link sign-in |
| `RESEND_FROM_EMAIL` | "Squintly <noreply@…>"; default sandbox |

### 11.3 Deploy a change

```bash
cd ~/work/squintly
# edit code
just test
# all green?
git add -A && git commit -m "..."
git push                          # main → origin/main
railway up --detach --service squintly
```

Railway's GitHub integration (if enabled) auto-deploys on push; the
explicit `railway up` is redundant in that case but harmless. Prefer it
explicitly so you know exactly which deploy ID is running.

### 11.4 Watch a deploy

```bash
railway logs --service squintly --build      # build phase
railway logs --service squintly --deployment # runtime
railway logs --service squintly              # default (last)
curl https://squintly-production.up.railway.app/api/stats
```

If healthcheck fails:
1. Check build logs first — Railway returns "Application not found" 404
   from the edge if the most recent deploy failed and no replicas exist.
2. If build succeeded but runtime crashed, the deployment logs will show
   it. The most common cause is a SQLx migration that fails on existing
   data; test migrations on a copy of the prod DB before pushing.

### 11.5 Roll back

```bash
railway redeploy <previous-deployment-id>
# IDs visible in `railway logs --json`
```

The `/data` volume is shared across deployments, so rollback doesn't
lose data — but it doesn't undo schema migrations either. If a migration
broke things, you have to write a corrective forward migration; never
manually edit production.

### 11.6 SQLite backup

Periodically (cron, or by hand):
```bash
railway run --service squintly -- sqlite3 /data/squintly.db ".backup /data/squintly.db.bak"
# then download via railway run cat or scp from the volume
```

Better: when meaningful data accumulates, switch to Postgres (DEPLOY.md
§6 has the migration sketch). SQLite is fine for ≤100k trials but
backups are a manual chore.

---

## 12. Testing strategy

### 12.1 The four layers

1. **Unit tests** (`cargo test --lib`) — module-internal logic. ~33 tests
   across bt, unified, asap, stats, sampling, grading, streaks,
   staircase, auth.
2. **Integration / smoke** (`cargo test --test smoke`) — full HTTP
   round-trip: spawn a fake coefficient, spawn squintly against it,
   exercise session/trial/response/export over reqwest. 1 test.
3. **End-to-end Playwright** (`cd web && npx playwright test`) — real
   Chromium, real frontend, real Rust binary, mock coefficient. 36 tests
   across welcome / calibration / trial / api / codec-filter / auth.
4. **Manual / preview** — `just dev` for local iteration. Open
   `http://localhost:3030` against a local coefficient.

### 12.2 Running them

```bash
just test                                  # cargo test --all-targets
just ci                                    # fmt + clippy + test + tsc
just e2e-prep                              # one-time setup for Playwright
just e2e                                   # run e2e suite
```

The e2e suite takes ~16s once warm; the cold path (binary build +
playwright install) is ~10 min. Use `just e2e-prep` once after pulling.

### 12.3 What's not tested

- The server-rendered HTML in `auth_verify` (visual). Inspect the trace
  artifacts in `web/test-results/` to debug.
- The actual Resend email send (would require live API key). Mocked at
  the 503 boundary.
- The bootstrap CIs themselves. They're statistical; we test the
  underlying primitives but not "does the CI cover the truth 90% of the
  time" — that needs Monte Carlo and is v0.3 work.
- ASAP runtime integration (because there is no runtime integration yet).

### 12.4 Common test gotchas

- **iPhone 14 device descriptor uses WebKit, not Chromium.** Use Pixel 7
  for "phone-shape Chromium". WebKit is gated behind a separate install.
- **Playwright tests share state.** Each test calls `gotoFresh()` which
  clears localStorage; otherwise a prior test's observer_id leaks in.
- **Race in trial-loop tests.** The previous trial's panel can still be
  in the DOM when the next iteration begins. Solved with `data-trial-id`
  attribute on the trial container; tests wait for it to change after
  click. See `helpers.ts::submitOneTrial()`.

---

## 13. Gotchas and lessons learned

### 13.1 Axum 0.8 path syntax

`/path/:capture` is gone; it's `/path/{capture}` now. Affects every
route definition. If you copy from older axum docs you'll get "Path
segments must not start with `:`" at startup.

### 13.2 Dockerfile dep-cache trick

The classic pattern (build with empty `src/main.rs` to cache deps, then
COPY real src) **races cargo's incremental fingerprints on Railway's
buildkit.** The empty stub's intermediate gets reused when the real lib
appears, producing "could not find module X" on a clean source tree.
Don't use this trick. Plain `cargo build` is slower but reliable.

### 13.3 Railway VOLUME directive

Railway rejects `VOLUME ["/data"]` in the Dockerfile — they want their
own `railway volume add --mount-path /data`. Both can't coexist. Remove
the VOLUME directive; document the local-test mount in a comment.

### 13.4 Railway healthcheck timeout

Default 10s is too tight for a cold container. Bump to 60s in
`railway.toml`. Cold starts include sqlx migrations and the (now
graceful) coefficient probe.

### 13.5 Railway non-interactive

`railway init` and `railway add` prompt for a workspace + service name
when stdin is a TTY but bail with `--workspace required` when piped.
Pass workspace ID + name explicitly: `railway init --name squintly
--workspace <UUID>`. Get the UUID from `railway list --json`.

### 13.6 JJ + git colocated, after a bookmark rewrite

After `jj squash` or `jj describe -m` rewrites the working commit, git's
HEAD can be on a stale ref. Recovery:
```bash
git checkout -B main <new-commit>
git reset                              # re-syncs index from HEAD without touching working tree
git push                               # works
```

`git reset --soft origin/main` also works for "keep my code, just rebase
my one commit onto upstream".

### 13.7 Pre-push hooks

The repo (or your global config) may have a `cargo fmt --check` pre-push
hook. If it fails, the push is aborted but the commit is still local.
Run `cargo fmt`, commit again as a "style: cargo fmt" follow-up, then
push.

### 13.8 cargo fmt batch effects

`cargo fmt` modifies multiple files at once. After a fmt that touches
formatting in a way you didn't intend, the tool result will report all
the modified files; verify each one isn't broken (cargo build, run
tests). Don't blindly accept.

### 13.9 Float literal precision

Rust's `excessive_precision` clippy lint fires on `1.061405429_f32`
because that has more digits than f32 can represent. For canonical
constants (Abramowitz-Stegun, etc.) this is a false positive — keep the
canonical values and `#[allow(clippy::excessive_precision)]` the
function. Truncating loses the documentation value of the constant.

### 13.10 `impl Trait` in closures

Closures don't accept `impl Trait` parameters. If you need a generic
closure, define a fn item or use `Box<dyn ...>` — but usually you can
just inline.

### 13.11 Codec-probe timing

The probe runs three `Image()` decodes in parallel via `Promise.all`,
each with a 1500 ms timeout. Bound is 1500 ms total. Welcome screen
renders after the probe completes. If you change probe count or timeout,
the welcome flicker between "loading" and "ready" gets noticeable.

### 13.12 Resend domain verification

Resend will accept sends from `onboarding@resend.dev` (sandbox) without
domain setup, but for any other from-address you need to verify the
domain in their dashboard (DNS TXT records). Until that's done, set
`RESEND_FROM_EMAIL` to the sandbox or omit it.

### 13.13 SQLite ALTER TABLE limits

SQLite's `ALTER TABLE` is restrictive. You can `ADD COLUMN` (with
DEFAULT and constraints subject to a few rules) and `RENAME COLUMN/TABLE`,
but you cannot `DROP COLUMN` or change a column's type without recreating
the table. Plan migrations accordingly. All 6 of our migrations only ADD
columns or CREATE new tables; nothing destructive.

### 13.14 Monotonicity matters more than rater filtering

CID22's most counter-intuitive measurement: enforcing same-codec
monotonicity in the BT fit changes KRCC from 0.99 to 0.56 on their data,
while removing the entire participant-screening pipeline only drops it
to ~0.99. The implication: get the monotonicity constraint right before
you sweat the outlier rules.

### 13.15 Bias correction is in z-units

CID22's "additive offset" is described in MCOS units in the paper but
the actual computation is in z-score units (per-stimulus normalized
difference). Squintly implements it that way too. If you ever try to
apply it as a 4-point ACR offset directly, the math comes out wrong —
the offset has to scale with the per-stimulus standard deviation.

### 13.16 Pérez-Ortiz tau monotonicity

The category thresholds τ_1 < τ_2 < τ_3 must remain ordered after each
gradient step or the cumulative-link likelihood explodes. Squintly sorts
them post-step. Don't remove that sort; if τ goes non-monotonic, the
rating likelihood becomes negative-probability and the whole fit
diverges.

### 13.17 ASAP EIG max is at p=0.5

The peak of binary entropy is at p=0.5, where the prediction is most
uncertain. ASAP picks the pair whose β-difference is closest to zero
under current MAP. Don't accidentally invert that — picking the most-
*decided* pair would do the opposite of what you want.

### 13.18 The Vite shell vs the rendered page

`curl https://squintly-production.up.railway.app/` returns the Vite
shell — `<main id="app"></main>` plus a script tag. The actual welcome
copy is rendered by JS *after* the codec probe finishes. To verify
deployed copy is current, grep the JS bundle:
```bash
curl https://squintly-production.up.railway.app/assets/main.js | grep "make the web faster"
```

### 13.19 Railway's edge 404

When a deploy is in flight, Railway's edge returns 404 with body
`{"status":"error","code":404,"message":"Application not found"}` until
the new replicas are healthy. Don't mistake this for a routing bug; it's
a normal in-deploy state.

### 13.20 Don't fabricate adopters

The first welcome copy claimed Squintly's data trains a metric "used by
Wikipedia". It isn't. CLAUDE.md's verified-claims rule applies to UI
copy as much as commit messages. Marketing copy that lies erodes trust
when discovered.

---

## 14. v0.3 roadmap

Concrete next steps in dependency order. Each is shippable on its own.

### 14.1 Coefficient deployment (HIGHEST PRIORITY)

Pick option A/B/C from §10.2. Without this Squintly collects no data.
Recommend option A: co-deploy on Railway with private network. If
coefficient's PostgreSQL+GCS is too heavy, option C (static manifest +
R2-hosted blobs) is the fastest path to a usable test corpus.

### 14.2 Seed scripts

Three Rust binaries under `tools/`:
- `seed-anchors` — reads coefficient manifest, emits 10 anchor INSERTs
  per source + 1 honeypot.
- `seed-held-out` — picks 20% of sources by hash prefix, INSERTs into
  `source_flags`.
- `seed-calibration` — emits 5 calibration_pool entries with stable IDs.

Run once after coefficient is wired in.

### 14.3 ASAP runtime integration

Wire `asap::pick_max_eig` into `pick_trial`. Two passes:
1. First pass: lazy refit per pick (refit BT from bucket responses on
   each call). Slow but simple.
2. Second pass: posterior cache with online updates. Faster but needs
   careful Newton-step math.

Add a feature flag `--asap-active-sampling` so it can be toggled per
deployment.

### 14.4 v0.2 batch grader

A Rust binary `tools/grade-observers` that runs nightly (or on demand):
1. Computes pwcmp leave-one-out log-likelihood per observer.
2. Fits Pérez-Ortiz (δ_o, σ_o) for ≥30-trial observers.
3. Aggregates CID22 normalised disagreement.
4. Writes to `observer_grades` table.
5. Emits a quality report.

### 14.5 CSV / JSON manifest for held-out evaluation

When zensim training begins, you'll want a single file containing the
held-out subset of `unified.tsv`. Add a `?held_out=1` filter to the
export endpoints. Plumbed already; just toggle a query param.

### 14.6 Codec version metadata

Once coefficient exposes `codec_version` per encoding in the manifest,
plumb it through and emit in `responses.tsv`.

### 14.7 Frontend polish

- Show streak count + total trials in the trial progress bar.
- Milestone celebration card on crossing 10/50/100/250/500/1000.
- "This image was contributed by N raters today" widget on the welcome
  screen.

### 14.8 Postgres migration path

When trial volume crosses ~100k, swap SQLite for Railway's managed
Postgres. The schema is portable but TEXT/INTEGER → TEXT/BIGINT in some
places. DEPLOY.md §6 has the sketch.

### 14.9 Crowdsourcing integration

Once data starts flowing, choose a recruitment channel (volunteer-mode
default per `motivation-and-compensation.md`; Prolific cohort-completion
is the next tier; never MTurk in 2026).

### 14.10 v0.3 batch features (not yet started)

- Active sampling per-bucket
- Held-out eval pipeline
- Public dashboard at `/stats` showing sample-size targets vs current
- Email digest cron
- Observer wall (opt-in)
- Charity payout integration

---

## 15. How to run things locally

### 15.1 First time

```bash
git clone https://github.com/imazen/squintly
cd squintly
cd web && npm install
cd ..
cargo build
```

### 15.2 Dev loop

```bash
# Terminal 1: vite dev with HMR for the frontend
cd web && npm run dev   # http://localhost:5173 (proxies /api to :3030)

# Terminal 2: rust binary
cargo run -- --coefficient-http http://localhost:8081 --port 3030

# Terminal 3: a coefficient or a mock
# (until coefficient deploys, the e2e mock at web/e2e/mock-coefficient.ts
#  works as a stand-in: node --import tsx web/e2e/mock-coefficient.ts)
```

### 15.3 Production-shape build locally

```bash
just build                    # vite build → cargo build --release
just docker-build             # build the deploy image locally
just docker-run               # run with /tmp volume
```

---

## 16. Database operations

### 16.1 Inspect

```bash
sqlite3 /data/squintly.db
.schema observers
.tables
SELECT count(*) FROM trials;
SELECT count(*), session_grade FROM sessions GROUP BY session_grade;
SELECT * FROM observer_badges WHERE observer_id = 'XXX';
```

### 16.2 Migrate

Migrations run automatically on startup via `sqlx::migrate!(./migrations)`.
SQLx tracks applied migrations in `_sqlx_migrations`.

To add a 7th migration, create `migrations/0007_<name>.sql`. Test on a
copy of the prod DB before pushing:

```bash
cp /data/squintly.db /tmp/test.db
SQUINTLY_DB=/tmp/test.db cargo run -- --coefficient-http http://localhost:1
# verify no errors; check schema
```

### 16.3 Recover from a bad migration

There's no `down` migration in sqlx by default — migrations are forward-
only. If a migration broke things in production:

1. Immediately roll back the deploy (`railway redeploy <prev-id>`).
   This restores the binary, but the migration ran against the volume,
   so the schema is in the new state.
2. Write a forward fix: a new migration that adds back the missing
   column or restores the broken state.
3. Push the fix.

Never edit `_sqlx_migrations` to "undo" a migration; that splits prod
from the migration files and breaks everyone else's first-time setup.

---

## 17. Observability

What's instrumented:

- **`tracing`** — logs at info level go to stdout. Railway captures
  these. Use `RUST_LOG=info,squintly=debug` in dev.
- **No metrics yet.** Add Prometheus or push to Grafana Cloud when
  v0.3 traffic warrants it.
- **No request tracing** beyond axum's tower-http TraceLayer.

When something's wrong:

1. `railway logs --service squintly --tail` for live tail
2. `railway logs --service squintly --json | jq` for filtering
3. SQLite for cold-storage state inspection

---

## 18. Security posture

- **No PII collected by default.** UUID in localStorage, screen
  characteristics, trial responses. That's it.
- **Email is opt-in.** Stored only when an observer signs in.
- **GDPR posture:** legitimate-interest for trial data, explicit consent
  for email. Always-available data-export and erasure routes are v0.3.
- **No analytics, no third-party trackers, no cookies** (everything in
  localStorage, which the user can wipe at any time).
- **No raw IPs stored.** Railway sees them but they don't enter the DB.
- **Magic-link tokens** are 256 bits of `OsRng`, BLAKE3-hashed at rest,
  15-min TTL, single-use. Plaintext exists only in the email URL.

Don't add tracking or analytics without updating `motivation-and-
compensation.md` §Ethical & legal.

---

## 19. Reading list

These are the papers/docs every methodology choice in Squintly traces
back to. If you change a parameter and don't update the doc, you've
broken the contract.

### 19.1 Datasets

- **CID22**: Sneyers, Ben Baruch, Vaxman 2023. "AIC-3 Contribution from
  Cloudinary: CID22." [Cloudinary PDF](https://cloudinary-marketing-res.cloudinary.com/image/upload/v1682016636/wg1m99012-ICQ-AIC3_Contribution_Cloudinary_CID22.pdf).
  The single most important reference. Read `docs/methodology.md` and
  this PDF together; everything else is supporting.
- **TID2013**: Ponomarenko et al. 2015. "Image database TID2013."
- **KADID-10k**: Lin, Hosu, Saupe 2019.
- **KonIQ-10k**: Lin/Hosu/Saupe 2018. [arXiv:1803.08489](https://arxiv.org/abs/1803.08489).
- **PaQ-2-PiQ**: Ying et al. 2020.

### 19.2 Methodology

- **BT-Davidson**: Davidson 1970. "Extending the Bradley-Terry model to
  incorporate ties." JASA 65(329).
- **Levitt 1971**: "Transformed up-down methods in psychoacoustics."
  J. Acoust. Soc. Am. 49(2).
- **Pérez-Ortiz & Mantiuk 2017**: [arXiv:1712.03686](https://arxiv.org/abs/1712.03686).
  pwcmp + Thurstone Case V.
- **Pérez-Ortiz et al. 2019**: ["Unified quality scale" PDF](https://www.cl.cam.ac.uk/~rkm38/pdfs/perezortiz2019unified_quality_scale.pdf).
  Joint pair+rating likelihood.
- **Mikhailiuk 2020 ASAP**: Active SAmpling for Pairwise.
- **BT.500-14**: ITU-R subjective TV quality recommendation. [PDF](https://www.itu.int/dms_pubrec/itu-r/rec/bt/R-REC-BT.500-14-201910-S!!PDF-E.pdf).

### 19.3 Outlier detection

- **Meade & Craig 2012**: "Identifying Careless Responses in Survey Data."
  [PDF](https://ubc-emotionlab.ca/wp-content/uploads/2012/06/Meade-Craig-2012-Careless-Responding-in-Survey-Data.pdf).
- **pwcmp**: [github.com/mantiuk/pwcmp](https://github.com/mantiuk/pwcmp).

### 19.4 Engagement

- **Galaxy Zoo motivations**: Raddick 2013. [arXiv:1303.6886](https://arxiv.org/abs/1303.6886).
- **Foldit**: Cooper et al. 2010. [PNAS](https://www.pnas.org/doi/10.1073/pnas.1115898108).
- **Eyal et al. 2023 PLOS One** on Prolific vs MTurk. [PMC10013894](https://pmc.ncbi.nlm.nih.gov/articles/PMC10013894/).

### 19.5 Calibration

- **Li 2020 virtual chinrest**: [Sci. Rep.](https://www.nature.com/articles/s41598-019-57204-1).

### 19.6 Imazen ecosystem

- **zensim**: the perceptual quality metric we're training a successor
  to. github.com/imazen/zensim
- **zenanalyze + zentrain**: the training pipeline. github.com/imazen/zenanalyze
- **coefficient**: the codec benchmark store. github.com/imazen/coefficient
- **interleaved**: the CMS that informs Squintly's Railway deploy
  pattern. github.com/imazen/interleaved

---

## 20. Quick reference

| Want to | Run |
|---|---|
| Local dev | `just dev` |
| Test everything | `just test` |
| Run e2e | `just e2e-prep && just e2e` |
| Build for deploy | `just build` |
| Deploy | `railway up --detach --service squintly` |
| Tail logs | `railway logs --service squintly` |
| Check live | `curl https://squintly-production.up.railway.app/api/stats` |
| Inspect DB | `sqlite3 /data/squintly.db` (in container) |
| Add a migration | `migrations/000N_<name>.sql`; runs automatically |
| Add an env var | `railway variables --set "FOO=bar"` |

| Endpoint | Method | Purpose |
|---|---|---|
| `/api/stats` | GET | Counts; healthcheck |
| `/api/session` | POST | Create observer + session |
| `/api/session/{id}/end` | POST | Ends session, computes grade |
| `/api/trial/next` | GET | Fetch a trial |
| `/api/trial/{id}/response` | POST | Record a response |
| `/api/proxy/source/{hash}` | GET | Image proxy → coefficient |
| `/api/proxy/encoding/{id}` | GET | Image proxy → coefficient |
| `/api/observer/{id}/profile` | GET | Streak, badges, themes |
| `/api/auth/start` | POST | Send magic link |
| `/api/auth/verify` | GET | Click destination |
| `/api/calibration` | GET | Onboarding items |
| `/api/calibration/response` | POST | Record one onboarding answer |
| `/api/calibration/finalize` | POST | Compute observer.calibrated |
| `/api/manifest/refresh` | POST | Reload coefficient manifest + anchors + flags |
| `/api/export/pareto.tsv` | GET | BT-Davidson with monotonicity + bootstrap CI |
| `/api/export/thresholds.tsv` | GET | Bias-corrected thresholds + bootstrap CI |
| `/api/export/unified.tsv` | GET | Pérez-Ortiz 2019 joint fit |
| `/api/export/responses.tsv` | GET | Raw per-trial dump |

| File | What |
|---|---|
| `Cargo.toml` | Rust deps; pin minor versions |
| `Dockerfile` | Multi-stage build; final image ~85 MB |
| `railway.toml` | Healthcheck + restart policy |
| `migrations/*` | Six SQLx migrations, applied in order |
| `src/main.rs` | Routes, startup |
| `src/handlers.rs` | All HTTP handlers |
| `src/coefficient.rs` | Manifest + image-byte client |
| `docs/methodology.md` | The contract for every parameter |
| `docs/HANDOFF.md` | This file |

---

## 21. The single most important thing

**Squintly's value is the conditioned data, not the inference.** Every
trial is a 4-tuple (stimulus, observer, condition, response). The
condition is the part the field doesn't have. If a v0.3 change makes the
data collection more efficient but loses or homogenizes the conditions,
that's a regression. Defend the heterogeneity.

Concretely:
- Don't normalize away dpr / viewing-distance / orientation in the
  exports.
- Don't gate participation on calibration completion (soft-fail only).
- Don't transcode codecs the browser doesn't natively decode.
- Don't aggregate across condition_buckets at the storage layer; let
  downstream choose.

The methodology doc is the contract. The exports are the deliverable.
The conditions are the value. Everything else is plumbing.
