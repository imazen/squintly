# Changelog

## [Unreleased]

### Added
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
