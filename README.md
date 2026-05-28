# Squintly

> Phone-first, viewing-condition-aware psychovisual data collection for
> [zensim](https://github.com/imazen/zensim). Pre-registered study of how a
> condition-conditioned quality metric can close zensim's SROCC plateau on real
> web content.

Existing public IQA datasets (KADID-10k, TID2013, CID22, AIC-3) **bake fixed viewing
conditions** into their labels: lab monitor, specified viewing distance, specified
ambient. zensim trained on them plateaus around SROCC 0.82 on CID22, and the residual
is structured by viewing conditions — DPR, intrinsic-to-device-pixel ratio, viewing
distance, ambient, gamut. Squintly collects pairwise + threshold judgments **with those
conditions recorded as first-class data per trial**, on the phones that actually deliver
the bulk of web image traffic.

**Two outputs, both consumed by zensim training:**

1. **Per-encoding latent quality `θ_{s,e,c}`** (BT-Davidson scale, anchored to CID22
   via shared anchors) — replaces zensim's training labels and adds a condition vector.
2. **Threshold function `q_threshold(c, k)`** for `k ∈ {notice, dislike, hate}` — what
   an encoder picker actually wants: "minimize bytes subject to `q ≥ q_threshold(c, k)`".

## Status

| | |
|---|---|
| Code | Rust+axum backend, Vite+vanilla-TS frontend embedded via `rust-embed`, SQLite via `sqlx`, deploy via Railway + Docker. |
| Study | **v0.2 protocol pre-registered** in [`docs/STUDY.md`](docs/STUDY.md) — H1–H5 hypotheses, holdout discipline, decision rules for zensim v48 ship locked. |
| Jargon | New to IQA? Start with [`docs/GLOSSARY.md`](docs/GLOSSARY.md) — every term, unit, method, and stat in this README, in plain English, with links to the underlying papers. |
| Phase | **v0.2 finishing** — ASAP active sampler now wired into `next_trial` ([§3 amplifier #1](#3-how-we-amplify-human-effort-15-levers)). Remaining: end-to-end smoke on a real phone + nightly per-observer η batch (#9). |
| Pilot | Planned: 100 sessions once v0.2 ships ([§8 phases](#8-phases)). |

---

## 1. Quick start (local)

```bash
# Build the embedded frontend + the binary.
just build

# 1. Local file-system coefficient store (fastest):
cargo run -- --coefficient-path /path/to/coefficient/benchmark-results --port 3030

# 2. Or HTTP coefficient store (if one's running):
cargo run -- --coefficient-http http://localhost:8081 --port 3030

# 3. Open http://localhost:3030 in any browser, on any device.
#    Calibrate (credit-card slider), pass the qualifier, rate trials.

# 4. Export when done (all TSV in zenanalyze schema):
curl http://localhost:3030/api/export/pareto.tsv      > pareto_human.tsv
curl http://localhost:3030/api/export/thresholds.tsv  > thresholds.tsv
curl http://localhost:3030/api/export/responses.tsv   > responses_raw.tsv
curl http://localhost:3030/api/export/unified.tsv     > unified.tsv

# 5. Smoke test (gates v0.2 done):
cargo test --test smoke
```

Deploy to Railway: see [`DEPLOY.md`](DEPLOY.md). Dockerfile builds the embedded TS,
then the Rust binary, then ships a `debian:bookworm-slim` runtime — no Node at
runtime.

---

## 2. The study

The study is **pre-registered** in [`docs/STUDY.md`](docs/STUDY.md). One-paragraph
summary:

> Train zensim-v48 on squintly's `(stimulus, viewing_condition_bin)` JOD labels and
> see whether it beats v47-strict (the current production bake) on held-out squintly
> data by **≥ 0.05 SROCC**, with the gain concentrated in the high-DPR / short-viewing-
> distance bins, **without regressing on CID22 / AIC-3 / AIC-4**. Threshold function
> `q_threshold(c, k)` must achieve R² ≥ 0.5 on held-out for at least `k = notice`.
> Decision rule for shipping v48 to `zensim::Profile::A` is locked in §3 of STUDY.md.

Methodology references: [`docs/methodology.md`](docs/methodology.md) (squintly's
internal methods), [`docs/participant-grading.md`](docs/participant-grading.md),
[`docs/motivation-and-compensation.md`](docs/motivation-and-compensation.md). For the
broader reproduction-grade methods reference (BT-MLE math, active-sampling info
criteria, software inventory) see the [zenpapers reference book](https://github.com/imazen/zenpapers/blob/main/docs/iqa-methods/reference-book/README.md).

---

## 3. How we amplify human effort — 15 levers

Ranked by ROI for converged-cells-per-human-hour. Status legend: ✅ done · 🟡 partial
(impl exists, not fully wired) · 🔵 planned · ⚫ explicitly out of scope.

| # | Lever | Status | Where | Plan |
|---|---|---|---|---|
| **1** | **Active sampling (ASAP EIG)** — pick next pair by expected information gain | ✅ | `src/asap.rs`, `src/sampling.rs` (`select_pair_with_eig`), `src/handlers.rs` (`enhance_pair_with_asap`), `tests/asap_wire.rs` | Wired into `next_trial` for non-golden pairs: on each pair request, refit BT-Davidson (σ_prior=1.0) over historical pair responses for the source and pick the highest-EIG adjacent non-trivial pair. Falls back to the random adjacent pair when comparisons < `ASAP_MIN_OBS=8`. Validate the ≥ 5× speedup-vs-random H4 prediction in pilot. |
| **2** | **Viewing-conditions captured per trial** (DPR, intrinsic-to-device, calibrated CSS-px/mm, ambient, distance, gamut) | ✅ | `migrations/0001_init.sql`, `web/src/conditions.ts`, `web/src/calibration.ts` | — |
| **3** | **Staircase thresholds** (Levitt 1971 transformed up–down, 79.4 / 70.7 / 50 % for notice / dislike / hate) | ✅ | `src/staircase.rs` (233 LOC) | tune step-halving cadence after pilot |
| **4** | **Trivial-triplet filter** — pre-filter pairs the metric ensemble unanimously predicts (≥ 95 % preference); humans never see them | ✅ | [`docs/methodology.md`](docs/methodology.md) §3.4, `src/sampling.rs` | verify firing rate in pilot |
| **5** | **Anchor reservation** — CID22-style reference-pinned anchors at ~5 % rate align our scale across studies | ✅ | `migrations/0006_v02_rigor.sql` (`corpus_anchors`), [`docs/methodology.md`](docs/methodology.md) §3.6 | populate the table from CID22 source overlap |
| **6** | **Onboarding calibration** — codec-probe + credit-card slider for physical CSS-px/mm | ✅ | `web/src/calibration-onboarding.ts`, `web/src/codec-probe.ts`, `migrations/0004_codec_support.sql` | — |
| **7** | **Honeypots** — known-answer trials at ~1/30 rate; fail-two-end-session | ✅ | [`docs/methodology.md`](docs/methodology.md) §3.9 | tune fail rate to ≤ 5 % on T1 observers |
| **8** | **Per-session grading A–F** + `session_weight ∈ {0, 0.5, 1.0, 1.5}` multiplier at scale reconstruction | ✅ | `src/grading.rs`, `migrations/0002_grading.sql` | — |
| **9** | **Per-observer reliability** — nightly `observer_grades` aggregation (trial-weighted golden, session-count-weighted even-odd, geometric-mean composite weight, trusted-pool promotion at weight ≥ 0.70 ∧ n_trials ≥ 50); Crowd-BT η + pwcmp LOO log-likelihood queued | 🟡 | `src/grading.rs::rebuild_observer_grades`, hooked into a 24h tokio task at `src/main.rs` | implement the pwcmp LOO (`dist_L > 1.5`) + Pérez-Ortiz 2019 (δ_o, σ_o) ACR fits post-pilot once we have ≥ 50 sessions/observer signal |
| **10** | **Engagement / retention** — streaks, freezes, tier ladder (anon → email → passkey → researcher), leaderboard | ✅ | `migrations/0003_engagement.sql`, `migrations/0005_auth.sql`, `web/src/auth-modal.ts` | iterate UX based on drop-off telemetry |
| **11** | **Phone-first single-stimulus + tap interaction** — short attention sessions, no mouse, portrait orientation | ✅ | [`SPEC.md`](SPEC.md) §"Target audience: phones", Playwright `zfold7-cover/inner` device profiles | — |
| **12** | **Unified pairwise + ACR-rating scale** (Pérez-Ortiz 2019 mixed BT-Davidson + ordinal model) | ✅ | `src/bt.rs`, `src/unified.rs` (solver fixed 2026-05-28 — see CLAUDE.md resolved-bug log), [`docs/methodology.md`](docs/methodology.md) §5.5 | `fit_unified` converges on consistent pair+rating evidence (regression-tested via `unified_competitive_with_bt_only_on_heldout_pairs`). `pair_log_likelihood` / `rating_log_likelihood` / `total_log_likelihood` shipped as the H3 evaluation entry point. The 2-nats/trial H3 gate itself is the pilot's job, not a unit test. |
| **13** | **Predictive sampling (PS-PC)** — offline classifier marks pair candidates `predict` vs `defer` using a metric ensemble (CVVDP / butteraugli / ssim2-gpu / zensim / PaQ-2-PiQ); humans only see `defer` | 🔵 | not yet | v0.3 add: classifier runs as a one-shot pre-flight, populates `defer` queue in `sampling.rs`. Expected: 8–22 % defer ratio at η ∈ {0.97, 0.995} ([Mohammadi 2311.03850](https://arxiv.org/abs/2311.03850)). |
| **14** | **Boosted Triplet Comparison (BTC)** — quadratic boosting `h(d) = γ₁d + γ₂d²` for high-fidelity near-JND sensitivity | 🔵 | not yet | v0.3 optional add for the q ≥ 80 band; expected ~3× discrimination ([AIC-3 2410.09501](https://arxiv.org/abs/2410.09501)) |
| **15** | **Hybrid expert + crowd** — route highest-EIG pairs to expert raters | ⚫ | not in scope for v1 (squintly is anonymous by design); zensim-level hybrid possible later via a separate study |

The first 11 are done; #1 (ASAP→next_trial) and #9 (nightly observer_grades
batch) both shipped recently. The remaining v0.2 work is the end-to-end smoke
test on a real phone plus the deeper #9 follow-up (pwcmp LOO + Pérez-Ortiz
2019 per-observer ACR fits) once we have ≥ 50 sessions/observer to fit on.

---

## 4. Workflows

```
                         ┌────────────────────────────────────────┐
                         │ coefficient (image store)              │
                         │   HTTP or local SplitStore             │
                         └──────────────┬─────────────────────────┘
                                        │ src/coefficient.rs (consumer-only)
            ┌───────────────────────────┴────────────────────────────────┐
            │                                                            │
            ▼                                                            ▼
┌──────────────────────┐                                ┌──────────────────────────┐
│ Curator workflow     │                                │ Trial workflow           │
│ /api/curator/*       │                                │ /api/session, /api/trial │
│ src/curator.rs       │                                │ src/handlers.rs          │
│ Stream → Curate →    │                                │   sampler ─▶ next_trial  │
│ Threshold → Export   │                                │   response → grading →   │
│ → curated_manifest   │                                │   weighted BT-Davidson   │
└──────────┬───────────┘                                └──────────┬───────────────┘
           │ TSV upload                                            │ inserts
           ▼                                                       ▼
┌──────────────────────────────────────────────────────────────────────────┐
│ SQLite (sqlx) — single canonical store                                   │
│   observers · sessions · trials · responses · staircases · corpus_anchors│
│   curator_candidates · curator_decisions · curator_size_variants ·       │
│   curator_thresholds · suggestions · calibration_pool                    │
└──────────────────────────────────────┬───────────────────────────────────┘
                                       │
              ┌────────────────────────┼────────────────────────┐
              ▼                        ▼                        ▼
     ┌─────────────────┐    ┌─────────────────────┐   ┌──────────────────────┐
     │ Online (per     │    │ Nightly batch       │   │ Export TSVs           │
     │ session)        │    │ grading + Crowd-BT η│   │ pareto / thresholds /  │
     │ staircase + BT  │    │ (planned, §3 #8/#9) │   │ responses / unified    │
     │ online estimate │    └─────────────────────┘   │ → zenanalyze/zentrain  │
     └─────────────────┘                              └──────────────────────┘
```

Workflows in detail:

- **Curator** ([`docs/CORPUS_CURATOR_SPEC.md`](docs/CORPUS_CURATOR_SPEC.md)) — researchers
  iterate the corpus offline: stream candidates from R2/local, curate (include/exclude
  + size bucket), set per-source threshold q ∈ {30, 50, 70, 85, 95}, export
  `curated_manifest.tsv`. Frontend: `web/src/curator.ts` + `curator-encoder.ts`. Backend:
  `src/curator.rs` + `src/variant_gen.rs`.
- **Suggestion** — anyone can submit a content URL via `/api/suggestions`; researcher
  triages via `/suggestions/{id}/{accept,reject}`. Backend: `src/suggestions.rs`,
  `src/suggestion_store.rs`. Migration: `0008_suggestions.sql`.
- **Trial** — observer arrives, opt-in calibration, qualifier (8 trials → ≥ 6 correct →
  `qualifier_passed`), main session of mixed Type-S + Type-P (sampler-chosen) with
  honeypots interleaved. Frontend: `web/src/trial.ts`. Backend: `src/handlers.rs`,
  `src/sampling.rs`, `src/asap.rs` (post v0.2-wiring).
- **Auth** — passwordless magic-link via Postmark for T1+ observers
  (`src/auth.rs`, `migrations/0005_auth.sql`). Observer aliases across devices
  (`observer_aliases`).
- **Grading** — per-session: golden-pass rate, straight-line max, honeypot failures →
  `session_grade A..F` → `session_weight ∈ {0, 0.5, 1.0, 1.5}`. Per-observer: trusted
  pool, skill score, per-observer reliability η (planned).
- **Export** — four TSVs in [zenanalyze/zentrain](https://github.com/imazen/zenanalyze)
  schema: `pareto.tsv` (BT-Davidson per-encoding scores), `thresholds.tsv`
  (q_notice/dislike/hate per-source-per-condition), `responses.tsv` (raw per-trial),
  `unified.tsv` (joint BT + ACR fit).

---

## 5. Data storage

### 5.1 Canonical paths

| What | Where | Format | Why there |
|---|---|---|---|
| SQLite DB | `${SQUINTLY_DB:-/data/squintly.db}` | sqlx-managed; 9 migrations applied at startup | Single source of truth for human responses; backups + Tower mirror per CLAUDE.md ML rules |
| Source images | `coefficient` store (HTTP or local SplitStore) | content-addressed by sha256 | **Squintly never writes back to coefficient**; consumer-only |
| Variant encodings | served via `/api/proxy/encoding/{id}`; backed by coefficient | content-addressed | reuses coefficient's content-addressing |
| Curated manifest snapshots | uploaded R2 JSONL at `pub-7c5c57fd3e0842f0b147946928891d40.r2.dev` | one JSONL per snapshot | reproducible curator inputs |
| Export TSVs | downloaded by user; canonical location is whoever downloaded last | TSV (zenanalyze schema) | `_MANIFEST.json` per export, includes `study_commit` + `db_snapshot_sha256` (planned) |
| Auth tokens | `auth_tokens` table | Postmark magic-link tokens, ephemeral | TTL'd; cleaned by `cleanup_expired_tokens` |
| Tower mirror | `/mnt/tower/output/squintly-archive-<study-version>/` | full SQLite snapshot + image manifest | per CLAUDE.md "Mirror canonical data to Tower NAS BEFORE any cleanup" |

### 5.2 SQLite schema (current — 9 migrations)

| Table | Purpose | Migration |
|---|---|---|
| `observers` | UUID + tier + calibration + email + skill_score + grading flags | 0001, 0002, 0003, 0006 |
| `sessions` | viewing-conditions snapshot, grade, weight, golden-pass-rate, codec-support | 0001, 0002, 0004, 0006 |
| `trials` | individual trial: source_hash, encoding id, condition vector, type S/P, staircase id | 0001 |
| `responses` | observer answer + dwell + swap_count + zoom + viewport snapshot at response time | 0001 |
| `staircases` | per-(source, condition_bucket) staircase state (reversals, current q, exit) | 0001 |
| `corpus_anchors` | CID22-style reference-pinned anchors for cross-study scale alignment | 0006 |
| `source_flags` | curator decisions on excluded/flagged sources | 0006 |
| `calibration_pool` + `calibration_responses` | shared calibration trials whose answers we know | 0006 |
| `auth_tokens` + `observer_aliases` | passwordless auth + multi-device merging | 0005 |
| `curator_candidates` / `_decisions` / `_size_variants` / `_thresholds` | curator workflow state | 0007, 0009 |
| `suggestions` | user-submitted content with workflow status | 0008 |

DDL in [`migrations/`](migrations/). Schema is **the source of truth** for fields; this
table is a navigation aid.

---

## 6. Statistical methods

### 6.1 Scale reconstruction (online + offline)

- **Online:** per session, BT-Davidson online estimate of `θ` (the BT score) via
  `src/bt.rs::fit_session()`; cheap, only over the current session's trial pool.
- **Offline:** full unified Pérez-Ortiz 2019 mixed BT-Davidson + ordinal-ACR with
  per-observer `(δ_o, σ_o)`; cross-source comparability via `corpus_anchors`. Gaussian
  prior σ_prior = 1.0 ([pwcmp standalone, arXiv 1712.03686](https://arxiv.org/abs/1712.03686);
  now in zenpapers corpus).

### 6.2 Threshold inference

For `k ∈ {notice, dislike, hate}`:

- **Online:** per-session staircase reversal-average estimate of `q_k` per
  (source, condition_bucket).
- **Offline:** GAM logistic `P(rating ≥ k | q, c) = Φ((q − μ_k(c)) / σ_k(c))` with
  smooths on `dpr`, `intrinsic_to_device_ratio`, `viewing_distance_cm`, codec offsets,
  per-source random intercept. **H2 in [`docs/STUDY.md`](docs/STUDY.md) pre-registers
  R² ≥ 0.5 for `k = notice` on held-out.**

### 6.3 Per-observer reliability (Crowd-BT η)

- Hooks in `src/grading.rs`; full η + pwcmp leave-one-out per-observer log-likelihood
  is the TODO at `src/grading.rs:340–343`. Plan: nightly batch reading the last 30 days
  of trials, writing `observers.skill_score` + a per-observer `η`.

### 6.4 Confidence intervals

- **Bootstrap over observers** (1000 resamples) for `θ` and `q_k`. The observer is the
  unit of resampling because errors are correlated within observer (BT.500 + Pérez-Ortiz).
- **Laplace / Hessian-inverse** for active-sampling EIG (cheap, online); switch to
  bootstrap for the final reported CIs.

### 6.5 Significance tests

For metric-vs-metric (v48 vs v47):

- **Δ-SROCC** with bootstrap 95 % CIs over observers.
- **Krasula different-vs-similar AUC**.
- **F-test** on residual variance for nested model comparisons.
- **Bonferroni** across the 5 pre-registered hypotheses (`α_individual = 0.05 / 5 = 0.01`).

Full math + citations: [`docs/methodology.md`](docs/methodology.md) §5–7 + the zenpapers
reference book [`ch3-5_sampling_screening_cis.md`](https://github.com/imazen/zenpapers/blob/main/docs/iqa-methods/reference-book/ch3-5_sampling_screening_cis.md).

---

## 7. Data sprawl — concerns + the rules we follow

ML projects accumulate sprawl faster than any other code; we apply the discipline from
the global CLAUDE.md "ML Data Pipeline Discipline" section verbatim:

1. **One canonical DB.** `${SQUINTLY_DB}` is the only store of human responses. No
   shadow exports, no "fast-iter" copies in `/tmp`. Backups via SQLite `VACUUM INTO`
   nightly to `/data/snapshots/squintly-<iso>.db`.
2. **Every export carries `build_commit`.** `/api/export/{kind}.tsv` ships a sibling
   `/api/export/{kind}.manifest.json` carrying `build_commit` (the git SHA the binary
   was built from, via `build.rs`), `schema_version`, `exported_at`, `row_count`,
   `sha256` of the TSV body, and a `source_query` pointer at the Rust source that
   produced the rows. The TSV response also carries a `Link: <…>; rel="describedby"`
   header pointing at the sidecar. Without this, "is this export still valid?"
   becomes a forensic audit.
3. **No source-image duplication.** Squintly *consumes* the coefficient store; never
   copies images, never writes back. The R2 manifest at the public bucket is the
   canonical curator input.
4. **Tower mirror BEFORE cleanup.** Per CLAUDE.md: before deleting any local DB
   snapshot, sha256-verify a Tower mirror copy. The snapshot directory grows; at
   ~10 snapshots a one-time consolidate-to-Tower run is fine; never deletes without the
   mirror.
5. **Block storage for anything > 30 KB.** Curator manifests, exported TSVs, image
   archives all live under `/mnt/v/output/squintly/<study-version>/` with a pointer
   file in this repo. **Never commit images, never commit the DB.**
6. **Cleanup of stale agent worktrees.** If multi-agent work creates sibling jj
   workspaces, `jj workspace forget` + `rm -rf` same-day per CLAUDE.md.
7. **Dated experiment dirs OK ≤ 7 days; promote or archive by day 14.** Any
   `docs/explorations/<date>-<topic>/` either gets promoted into the main `docs/` tree
   or moved to `_archive/`.

What we don't yet enforce automatically (tracking these as v0.2 exit gates):

- [x] `_MANIFEST.json` emitted alongside every export TSV
      (`/api/export/{kind}.manifest.json` paired with the TSV; describedby Link header).
- [x] Nightly Tower-mirror snapshot — `main.rs` auto-detects `/mnt/tower` and spawns
      a daily `VACUUM INTO /mnt/tower/output/squintly-archive/<iso>.db` task. Silently
      no-ops on Railway / CI / remote deploys where the mount is absent.
- [x] Per-table row counts in a `db_health` table updated hourly
      (`src/db_health.rs::refresh` + `migrations/0010_db_health.sql`; hourly tokio task
      in `main.rs`).
- [x] Curator R2 snapshot tag pinned in each session — `manifest_snapshots` table
      (`migrations/0011_manifest_snapshots.sql`) records `(r2_public_base,
      manifest_path, manifest_sha256, body_bytes, n_candidates)` per load; UNIQUE
      key on the three identity columns means re-loading an unchanged manifest is
      a cheap no-op. `sessions.manifest_snapshot_id` FK pins the latest snapshot
      at session creation so analysis six months later can join from any session
      to the exact candidate pool it drew from.

---

## 8. Phases

Locked sequence; the order matters because each phase depends on the previous one's
data:

| Phase | Goal | Done-when | Status |
|---|---|---|---|
| **v0.1** | Backend skeleton + trial routes wired; curator + variant generation; license surfacing | `cargo test` green + curator export round-trips | ✅ |
| **v0.2** | ASAP wired into `next_trial`; per-session grading active; export `_MANIFEST.json`; smoke test exercises the full session loop on a real phone | All 5 of SPEC.md §"Hard v0.1 is done when" + the 4 sprawl-enforcement checkboxes above | 🟡 in progress |
| **v0.3 — pilot** | 100 sessions across the existing curator corpus; H4 (active-sampling speedup) + H5 (phone-vs-desktop noise) measured; Crowd-BT η baseline | All five hypotheses' analysis pipelines green on pilot data | 🔵 |
| **v1.0 — main** | 1500 sessions; H1–H3 evaluated; v48-cond zensim bake decision | The decision rule in [`docs/STUDY.md`](docs/STUDY.md) §3 executes — v48 ships **or** an honest-stop documenting the next hypothesis | 🔵 |
| **v1.1+** | PS-PC predict-fill turned on; BTC boosting for high-fidelity; per-observer η; mobile-specific (Galaxy Z Fold) sub-study | gated on v1.0 finding | 🔵 |

Full study design + decision rules: [`docs/STUDY.md`](docs/STUDY.md).

---

## 9. Contributing / open decisions

The amplifier table in [§3](#3-how-we-amplify-human-effort-15-levers) is the authoritative
checklist of what's done vs planned. The next-most-impactful tractable chunks:

1. **End-to-end smoke on a real phone** (v0.2 exit gate). The Playwright e2e tests run
   in CI but a human-on-Galaxy-Fold-7 walkthrough catches everything the headless
   browser misses.
2. **pwcmp LOO + Pérez-Ortiz 2019 per-observer ACR fits** (amplifier #9 deep dive).
   The nightly `observer_grades` aggregation already fills the trial-weighted golden
   pass rate, even-odd consistency, geometric-mean weight, and trusted-pool promotion;
   the LOO and per-observer ACR fits fill the `pwcmp_*` and `sigma_acr` / `delta_acr`
   columns once we have ≥ 50 sessions per observer.
3. **Tower-mirror cron + db_health** — the two remaining data-sprawl v0.2 exit gates.

Open decisions worth surfacing:

- [ ] **PS-PC at v0.3 or v1.0?** The metric ensemble (CVVDP / butteraugli / ssim2 /
      zensim / PaQ-2-PiQ) isn't yet wired into squintly's pair-selection. v0.3 is the
      natural slot.
- [ ] **Anchor source.** CID22 is the obvious anchor, but we don't yet have a
      content-overlap map; need to derive which of CID22's 250 originals overlap our
      curator corpus (zero, probably — different content pool — in which case we anchor
      via a controlled stimuli set instead).
- [ ] **Recruitment.** Are we self-recruiting friends + colleagues for the pilot, or
      going Prolific from day 1? Prolific has the screener depth but anonymous T0
      observers should be a viable path on our own site.
- [ ] **Galaxy Z Fold 7 sub-study.** The Playwright device profiles exist; running an
      actual cohort on foldables would be a separate paper.

---

## 10. Deploy / ops

Production deploy uses Railway + a 1 GB volume mounted at `/data`. See
[`DEPLOY.md`](DEPLOY.md) for the full walkthrough.

Monitoring: stdout JSON logs (Railway captures them); per-endpoint latency in tracing
spans. No third-party telemetry.

---

## 11. License & attribution

- **Code:** Apache-2.0 OR MIT (dual).
- **Per-image content:** runs under each source corpus's license (per
  `src/licensing.rs` registry — 7 policies surfaced on the welcome screen + per-trial
  badge). See [`docs/motivation-and-compensation.md`](docs/motivation-and-compensation.md).
- **Derived score tables** (`pareto.tsv`, `thresholds.tsv`, `unified.tsv`): planned for
  **CC-BY-4.0** release after study close, conditional on consent posture (see
  [`docs/STUDY.md`](docs/STUDY.md) §8).

---

## See also

- [`SPEC.md`](SPEC.md) — full v0.1 spec (data model, schema, threshold/BT math).
- [`docs/STUDY.md`](docs/STUDY.md) — formal pre-registration: research questions,
  hypotheses, design, analysis plan, decision rules.
- [`docs/GLOSSARY.md`](docs/GLOSSARY.md) — **layperson glossary** of every term, unit,
  method, and statistic this README and STUDY.md use, with hyperlinks to all the
  underlying research papers, datasets, and tools.
- [`docs/methodology.md`](docs/methodology.md) — methods reference internal to squintly.
- [`docs/participant-grading.md`](docs/participant-grading.md) — session-grade + weight
  policy.
- [`docs/motivation-and-compensation.md`](docs/motivation-and-compensation.md) —
  recruitment economics + ethics posture.
- [`docs/CORPUS_CURATOR_SPEC.md`](docs/CORPUS_CURATOR_SPEC.md) — curator workflow.
- [`docs/HANDOFF.md`](docs/HANDOFF.md) — agent / contributor handoff notes.
- External: [zensim](https://github.com/imazen/zensim) (what we feed),
  [zenanalyze](https://github.com/imazen/zenanalyze) (TSV consumer),
  [coefficient](https://github.com/imazen/coefficient) (image store we consume),
  [zenpapers reference book](https://github.com/imazen/zenpapers/blob/main/docs/iqa-methods/reference-book/README.md)
  (methodology citations).
