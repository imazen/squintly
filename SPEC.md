# Squintly — Psychovisual Data Collection for zensim

## Mission

Two interlocking objectives:

1. **Relative scoring.** Collect pairwise human judgments → Bradley–Terry-Davidson →
   per-encoding JOD scores that replace zensim's training labels and are conditioned
   on viewing conditions.
2. **Imperceptibility / annoyance thresholds.** For each (content, viewing-condition)
   bucket, find the encoder quality level at which the typical observer (a) **first
   notices** distortion, (b) **starts to dislike** it, (c) **hates** it. These three
   thresholds — `q_notice`, `q_dislike`, `q_hate` — are what an encoder picker
   actually wants to know. DPI / pixels-per-degree / intrinsic-to-device-pixel
   ratio likely dominates them.

Existing public datasets (KADID-10k, TID2013, CID22) fix viewing conditions and bake
them into the labels. zensim plateaus around SROCC 0.82 on these because the residual
is dominated by *how* an image is being viewed: device pixel ratio, intrinsic-to-device
pixel ratio, viewing distance, ambient light, gamut. We record the conditions
explicitly so the metric — and the threshold model — can use them as inputs, not as
fixed assumptions.

## Target audience: phones

The app is **phone-first**. We need data at the high end of the dppx range (2.0–3.5)
and at typical phone viewing distances (~25–35 cm) which existing labs almost never
measure. Tablets and desktops still work, but the layout, gesture set, and trial
density are tuned for a held device in portrait orientation.

UX consequences:
- **Single-stimulus default.** A 4″ portrait viewport has no room for side-by-side.
  Show one image at a time; observer holds a button to reveal the reference for
  comparison (CID22-PTC style, ≤2 Hz toggle).
- **Pairwise becomes "compare A vs B sequentially"** with a fast toggle, instead of
  side-by-side slider. Same data semantics.
- **4-button rating panel** (imperceptible / notice / dislike / hate) with large
  thumb-friendly hit targets, captioned with one-line definitions on first use.
- Swipe-up = next; swipe-down = flag/skip. Tap-and-hold the image = reveal reference.
- Orientation captured per trial; rotation lock during a trial to avoid mid-trial
  flips corrupting the displayed-CSS-size measurement.
- iOS Safari: no Ambient Light Sensor API. Self-report 3-bin (dim / room / bright)
  is the only signal we can rely on. Android may surface ALS behind a flag — we use
  it opportunistically.

## Why "Squintly"

Short, descriptive (we are literally asking people to squint and discriminate), Google-
unique as a project name, easy to type. Codename, not a product brand — never appears
in observer-facing UI copy (which calls it the *Image Discrimination Study*).

## Scope (v0.1)

**In scope:**

- Web app served by a single Rust binary (axum)
- Pulls source images and pre-encoded variants from a coefficient SplitStore (HTTP or
  filesystem) — coefficient is the source of truth for the image corpus and the
  candidate encodings
- Presents 2AFC trials (forced-choice "which looks closer to the reference?")
- Captures per-trial viewing conditions on the client and persists them
- Stores everything in a local SQLite database
- Exports two TSVs in the zenanalyze/zentrain pareto-training schema, with a
  Bradley–Terry-derived scalar quality score replacing zensim
- Embedded static frontend (no separate Node server in production)

**Out of scope (v0.1):**

- Continuous-rating MOS or JND staircase (2AFC only — fastest signal per trial)
- Authenticated observers or any PII (only an anonymous random ID in localStorage)
- Cross-device session resumption
- Pushing aggregated scores back into coefficient's database. We emit TSV; coefficient
  consumes it later if useful. Coefficient itself does not yet model raters/observers.
- Adaptive trial selection (active learning / TrueSkill matchmaking) — random pair
  sampling is enough to bootstrap; v0.2 territory.
- Mobile-app form factor. The web app must work on mobile browsers (and we *want* the
  full range of dppx values), but no native wrapper.

## Data model

### Two trial types

The session interleaves two trial types, weighted ~70/30 toward thresholds (which need
more samples per bucket to converge a staircase) early, then ~50/50 once the staircase
has converged for the active bucket.

#### Type S (single-stimulus, threshold protocol)

The default. One encoded image at a time, with hold-to-reveal-reference. The observer
rates on a 4-tier ACR scale derived from BT.500-15 + CID22:

| Code | Caption | Definition shown on first use |
|---|---|---|
| 1 | Imperceptible | I can't tell this from the reference. |
| 2 | I notice | I can see something is off, but it's fine. |
| 3 | I dislike | The artifacts bother me. I would not use this. |
| 4 | I hate | This is unacceptable. |

Adaptive **transformed up–down staircase** (Levitt 1971) per (source, condition_bucket):
- 3-down-1-up converges on the 79.4% threshold for "imperceptible" → `q_notice`.
- 2-down-1-up converges on 70.7% for "dislike" → `q_dislike`.
- 1-down-1-up converges on 50% for "hate" → `q_hate`.

Step size halves after each reversal until reaching the codec-config grid resolution
(typically one quality step). Three independent staircases run in parallel; the next
trial picks the staircase whose CI is currently widest (within the active source).

#### Type P (pairwise, scoring protocol)

A triplet `(reference, A, B)`. Single-image carousel: `[ref][A][B]` with tap to switch
or auto-advance every 3 s; the observer answers "A is closer / they tie / B is closer"
on a fixed 3-button bar. Stored as a Bradley–Terry-with-ties observation per CID22-PTC.

This subsumes side-by-side comparison on phones where there's no horizontal room.

### Sampling strategy (v0.1)

Per-source, prefer pairs where A and B are at **adjacent quality steps for the same
codec**, or **same target_zq across different codecs**. Sample uniformly across:

- 4 size buckets (≤256px, ≤768px, ≤2048px, >2048px) — **mandatory**, per the
  source-informing-sweep rule in CLAUDE.md
- Quality range with extra weight at q5–q40 — web-focused, low-q is where structural
  problems live and where current metrics disagree most with humans
- All present codecs in the store

Concretely: each trial server-samples a `source` weighted by inverse-coverage in
existing responses (so under-rated images get more trials), then picks A and B from
the source's encodings under the constraints above.

### Viewing conditions (captured per session AND per trial)

**Per session** (stable):
- `device_pixel_ratio` — `window.devicePixelRatio`
- `screen_width_css`, `screen_height_css` — `screen.width` × `screen.height`
- `screen_width_device`, `screen_height_device` — × dpr
- `color_gamut` — `matchMedia('(color-gamut: srgb|p3|rec2020)')`
- `dynamic_range` — `matchMedia('(dynamic-range: high)')`
- `prefers_color_scheme` — `matchMedia('(prefers-color-scheme: dark|light)')`
- `pointer_type` — coarse vs fine
- `user_agent` — for OS/browser correlation
- `connection_type` if available (NetworkInformation API)
- `timezone` — proxy for time-of-day at trial time
- Self-reported (optional, dismissable):
  - `viewing_distance_cm` — chosen from preset (close/phone ~30 cm, lap ~50 cm, desk
    ~70 cm, couch ~150 cm, far/TV ~250 cm)
  - `ambient_light` — qualitative (dark / dim / bright / outdoors)
  - `vision_corrected` — yes/no/contacts
  - `age_bracket` — coarse
- `calibration` — captured via a known-CSS-size element ("hold a credit card to the
  rectangle, drag the slider until it matches"); yields physical CSS-px-per-mm for
  the user's screen, which combined with viewing distance gives us **angular
  resolution in cycles per degree** — the actually relevant quantity.

**Per trial** (variable):
- `viewport_width_css`, `viewport_height_css` — at trial render time (orientation can
  flip mid-session on phones)
- `image_intrinsic_w`, `image_intrinsic_h` — natural pixel dimensions of the served
  image
- `image_displayed_w_css`, `image_displayed_h_css` — bounding box at render
- `image_device_w`, `image_device_h` — × dpr (the actual physical pixels on screen)
- `intrinsic_to_device_ratio` — derived: `image_intrinsic_w / image_device_w`. This
  is the headline number; <1 means upscaling, >1 means downscaling, 1.0 means
  pixel-perfect. We want the full distribution.
- `dwell_ms` — time from first paint to response
- `zoom_used` — boolean (did the observer engage native zoom or the component's
  zoom controls)
- `swap_count` — slider/A-B toggles before answering (proxy for difficulty)

### SQLite schema

```sql
-- Random anonymous identity, persisted in localStorage
CREATE TABLE observers (
    id TEXT PRIMARY KEY,                -- UUID v4
    created_at INTEGER NOT NULL,        -- unix ms
    user_agent TEXT,
    age_bracket TEXT,                   -- '<25','25-35','35-50','50-65','65+', NULL
    vision_corrected TEXT               -- 'yes','no','contacts', NULL
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,                -- UUID v4
    observer_id TEXT NOT NULL REFERENCES observers(id),
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    device_pixel_ratio REAL NOT NULL,
    screen_width_css INTEGER NOT NULL,
    screen_height_css INTEGER NOT NULL,
    color_gamut TEXT,                   -- 'srgb','p3','rec2020','unknown'
    dynamic_range_high INTEGER,         -- 0/1
    prefers_dark INTEGER,
    pointer_type TEXT,
    timezone TEXT,
    -- Self-reported
    viewing_distance_cm INTEGER,
    ambient_light TEXT,
    -- Calibration (CSS px per millimeter on this physical screen)
    css_px_per_mm REAL,
    notes TEXT
);

CREATE TABLE trials (
    id TEXT PRIMARY KEY,                -- UUID v4
    session_id TEXT NOT NULL REFERENCES sessions(id),
    kind TEXT NOT NULL,                 -- 'single' or 'pair'
    source_hash TEXT NOT NULL,          -- coefficient SHA-256
    -- For 'single' trials: a_* is the encoding under test, b_* is unused
    -- For 'pair' trials: both used
    a_encoding_id TEXT NOT NULL,
    a_codec TEXT NOT NULL,
    a_quality REAL,
    a_bytes INTEGER,
    b_encoding_id TEXT,                 -- NULL for single
    b_codec TEXT,
    b_quality REAL,
    b_bytes INTEGER,
    intrinsic_w INTEGER NOT NULL,
    intrinsic_h INTEGER NOT NULL,
    -- Threshold-staircase tracking (NULL for pair trials)
    staircase_id TEXT,                  -- per (session, source, target_threshold)
    staircase_target TEXT,              -- 'notice' | 'dislike' | 'hate'
    staircase_step INTEGER,             -- monotone within a staircase
    -- Active sampling weights (for analysis)
    is_golden INTEGER NOT NULL DEFAULT 0,  -- 0/1: anchor/attention check
    served_at INTEGER NOT NULL
);

CREATE TABLE responses (
    trial_id TEXT PRIMARY KEY REFERENCES trials(id),
    -- For 'single': '1','2','3','4' (imperceptible/notice/dislike/hate)
    -- For 'pair':   'a','b','tie','flag_broken'
    choice TEXT NOT NULL,
    -- Conditions captured at response time (per-trial; orientation can flip)
    dwell_ms INTEGER NOT NULL,
    reveal_count INTEGER NOT NULL,      -- # times observer pressed "show reference"
    reveal_ms_total INTEGER NOT NULL,   -- ms with reference visible
    zoom_used INTEGER NOT NULL,         -- 0/1
    viewport_w_css INTEGER NOT NULL,
    viewport_h_css INTEGER NOT NULL,
    orientation TEXT NOT NULL,          -- 'portrait' | 'landscape'
    image_displayed_w_css REAL NOT NULL,
    image_displayed_h_css REAL NOT NULL,
    intrinsic_to_device_ratio REAL NOT NULL,
    -- Derived viewing condition (cached at trial time, joined w/ session calibration):
    pixels_per_degree REAL,             -- NULL if no calibration
    responded_at INTEGER NOT NULL
);

CREATE TABLE staircases (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    source_hash TEXT NOT NULL,
    codec TEXT NOT NULL,                -- staircase is per-codec
    target TEXT NOT NULL,               -- 'notice'|'dislike'|'hate'
    rule TEXT NOT NULL,                 -- '3down1up'|'2down1up'|'1down1up'
    started_at INTEGER NOT NULL,
    converged INTEGER NOT NULL DEFAULT 0,
    converged_quality REAL,             -- final estimate when converged
    reversals INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_trials_source ON trials(source_hash);
CREATE INDEX idx_responses_trial ON responses(trial_id);
```

## Backend (Rust, axum)

Single binary, embeds the built frontend with `rust-embed`. SQLite via `sqlx` (compile-
time-checked queries are fine; we only run a fixed schema).

**Endpoints:**

| Method | Path | Purpose |
|---|---|---|
| GET  | `/` | serve embedded frontend (index.html) |
| GET  | `/assets/*` | serve embedded JS/CSS |
| POST | `/api/session` | body: {observer_id?, conditions}; returns {observer_id, session_id} |
| POST | `/api/session/:id/end` | mark session ended |
| GET  | `/api/trial/next?session_id=…` | sample and return a new trial including signed image URLs |
| POST | `/api/trial/:id/response` | body: response payload; persists to SQLite |
| GET  | `/api/proxy/source/:hash` | proxy through to coefficient `/api/sources/:hash/image` |
| GET  | `/api/proxy/encoding/:id` | proxy through to coefficient `/api/encodings/:id/image` |
| GET  | `/api/export/pareto.tsv` | aggregated, BT-scaled per (source,encoding) |
| GET  | `/api/export/responses.tsv` | raw responses, one row per trial |
| GET  | `/api/stats` | counts (observers, sessions, trials, responses) for admin dashboard |

The proxy is required because (a) coefficient may be on localhost:8081 / a private GCS
prefix the browser can't reach, (b) we want to control caching headers, (c) we may want
to serve scaled variants later without changing the public surface.

**Coefficient client:** `Coefficient` trait with two impls:
1. `HttpCoefficient { base_url }` — talks to a running coefficient viewer.
2. `FsCoefficient { store_path }` — reads the SplitStore (`meta/` + `blobs/`) directly.

Configurable via `SQUINTLY_COEFFICIENT_HTTP=http://localhost:8081` or
`SQUINTLY_COEFFICIENT_PATH=/path/to/store`. HTTP wins if both set.

**Sampling:** `GET /api/trial/next` runs a SQL query that joins with a manifest cached
on startup from coefficient (`/api/manifest`) listing every (source, encoding) and its
codec/quality/size/bytes. Cache refreshes on demand or every 5 min.

## Frontend (vanilla TS + Vite, phone-first)

No framework. Bundle to `web/dist/`, which the Rust binary embeds. We do *not* import
`@aspect/image-compare` for the primary trial loop — it's tuned for desktop side-by-side
and our phone UX is single-stimulus. The component is still useful for the optional
"big-screen" mode (auto-detected by viewport width >= 900 CSS px), where pair trials
do show side-by-side.

Five screens:

1. **Welcome** — one screen, one paragraph, one big "Begin" button.
2. **Calibration** — Li 2020 virtual chinrest: (a) credit-card resize (slider until
   the on-screen rectangle matches a held card), (b) blind-spot sweep (close right
   eye, fixate dot, tap when red dot disappears). Solved for `css_px_per_mm` and
   `viewing_distance_cm`. Either step is skippable; we record `null`.
3. **Conditions form** — three taps total: ambient light (3-bin), vision-corrected,
   age bracket. Skippable.
4. **Trial loop** — full-bleed image area, four tap targets at the bottom for single-
   stimulus rating, hold-the-image to reveal the reference. Pair trials switch to a
   carousel with three tap targets. Progress dots at the top; swipe-down opens a
   menu (skip / flag / pause / end). After every 25 trials a forced 30 s break
   screen with a tap-to-resume.
5. **Done** — thanks, count of trials, link to a "what was this for?" page that
   explains the goal and shows aggregate stats.

The frontend captures viewing conditions on every trial via:

```ts
const intrinsic = { w: img.naturalWidth, h: img.naturalHeight };
const rect = img.getBoundingClientRect();
const displayedCss = { w: rect.width, h: rect.height };
const dpr = window.devicePixelRatio;
const deviceW = displayedCss.w * dpr;
const intrinsicToDeviceRatio = intrinsic.w / deviceW;
const orientation = matchMedia('(orientation: portrait)').matches ? 'portrait' : 'landscape';
const ppd = (cssPxPerMm && viewingDistanceCm)
    ? (cssPxPerMm * dpr) * (viewingDistanceCm * 10) * (Math.PI / 180)
    : null;
```

The frontend captures viewing conditions on every trial via:

```ts
const intrinsic = { w: img.naturalWidth, h: img.naturalHeight };
const rect = img.getBoundingClientRect();
const displayedCss = { w: rect.width, h: rect.height };
const dpr = window.devicePixelRatio;
const deviceW = displayedCss.w * dpr;
const intrinsicToDeviceRatio = intrinsic.w / deviceW;
```

## Threshold model

For each (source, codec, condition_bucket) we fit three quality thresholds:

- `q_notice` — the encoder quality below which P(rating ≥ 2) > 0.5
- `q_dislike` — the encoder quality below which P(rating ≥ 3) > 0.5
- `q_hate` — the encoder quality below which P(rating = 4) > 0.5

Online estimates come from the per-session staircase (Levitt 1971, transformed
up–down). Offline estimates come from a logistic psychometric fit:

```
P(rating ≥ k | q, c) = Φ((q - μ_k(c)) / σ_k(c))
```

where `c` is the condition vector (dpr, ppd, viewing_distance, intrinsic_to_device,
ambient_light_bin, color_gamut, ...). We fit per (codec, content-class), and report
the threshold dependence on `c`. The headline output is **`q_threshold(c, k)`**, the
function the encoder picker actually wants.

The training pipeline in zenanalyze/zentrain can consume the threshold table
directly: instead of "minimize bytes subject to zensim ≥ T", the picker becomes
"minimize bytes subject to q ≥ q_threshold(c, k)" for the deployment's condition
vector — exactly the win we promised.

## Bradley–Terry quality scaling

Pairwise responses (`a`/`b`/`tie`) over a session pool become per-(source, encoding)
scalar quality scores via a Bradley–Terry model with ties (Davidson 1970):

For a fixed source S, treat each encoding `e` as a player with skill `θ_e`. Per
comparison `(e_i, e_j) → outcome`, log-likelihood is:

```
P(i beats j) = exp(θ_i) / (exp(θ_i) + exp(θ_j) + ν √(exp(θ_i) exp(θ_j)))
P(tie)       = ν √(exp(θ_i) exp(θ_j)) / (exp(θ_i) + exp(θ_j) + ν √(...))
```

Fit `θ` and `ν` by L-BFGS, anchor `θ_reference = 0`, scale to a 0–100 range matched to
zensim's scoring conventions. Implementation lives in `squintly::bt::fit_session()`.

We fit **per-source** (not global) because the latent skill is "perceptual closeness to
*this* reference". Cross-source aggregation happens later in zenanalyze, not here.

## Output: zenanalyze TSV schema

We export three TSVs:

- `pareto.tsv` — per-(source, encoding, condition_bucket) BT-derived quality (drop-in
  for zensim's training column).
- `thresholds.tsv` — per-(source, codec, condition_bucket) `(q_notice, q_dislike,
  q_hate)` with bootstrap CI and observer count.
- `responses.tsv` — raw per-trial dump for researchers fitting their own models.

`/api/export/pareto.tsv` — one row per (source_hash, encoding_id, conditions_bucket):

```
image_id  size  config_name        target_zq  bytes   quality  encode_ms  observers  trials  conditions_bucket
abc12345  S     mozjpeg.q40        40         12450   38.2     12         8          24      dpr2_dist70_dim
abc12345  S     mozjpeg.q60        60         18920   55.9     12         9          26      dpr2_dist70_dim
…
```

- `image_id` = first 8 chars of source SHA-256
- `size` = S/M/L/XL bucket
- `config_name` = `{codec}.q{quality}` (codec-specific scheme; same as zenanalyze)
- `target_zq` = quality (0–100)
- `bytes` = encoded size from coefficient
- `quality` = BT-derived score, anchored 100 = reference, decreasing with distortion
- `observers` / `trials` = sample size for this row (filter low-N rows downstream)
- `conditions_bucket` = compact code: `dpr{1|2|3}_dist{30|50|70|150|250}_{dark|dim|bright|outdoors}`.
  We emit one row per non-empty bucket; downstream training can either pool or
  condition on the bucket.

`/api/export/thresholds.tsv` — one row per (source_hash, codec, condition_bucket):

```
image_id  size  codec   conditions_bucket  q_notice q_notice_lo q_notice_hi q_dislike q_dislike_lo q_dislike_hi q_hate q_hate_lo q_hate_hi observers trials
abc12345  S     mozjpeg dpr3_dist30_dim    78.2     71.4        82.0        62.5      55.0         68.7         38.4   30.1      45.0      8         42
```

`bucket` codes are compact strings: `dpr{1|2|3}_dist{20|30|50|70|150|250}_{dark|dim|bright|outdoors}_{srgb|p3|rec2020}`.
On phones the dpr3 / dist20-30 / portrait buckets dominate by design.

`/api/export/responses.tsv` — raw, one row per trial response, full conditions for
researchers who want to fit their own model. Schema follows the SQL exactly.

## Files

```
squintly/
├── Cargo.toml
├── README.md
├── SPEC.md                  (this file)
├── CLAUDE.md
├── CHANGELOG.md
├── .gitignore
├── .workongoing            (gitignored)
├── migrations/
│   └── 0001_init.sql
├── src/
│   ├── main.rs             # axum app entrypoint, config, embed
│   ├── db.rs               # sqlx pool, migration, queries
│   ├── coefficient.rs      # Coefficient trait + Http/Fs impls
│   ├── handlers.rs         # axum route handlers
│   ├── sampling.rs         # trial sampling logic
│   ├── bt.rs               # Bradley–Terry fit
│   ├── export.rs           # TSV streaming
│   └── lib.rs
├── web/
│   ├── package.json
│   ├── tsconfig.json
│   ├── vite.config.ts
│   ├── index.html
│   └── src/
│       ├── main.ts
│       ├── style.css
│       ├── api.ts
│       ├── conditions.ts   # browser-side viewing-condition capture
│       ├── calibration.ts  # credit-card calibration UI
│       └── trial.ts        # trial loop + image-compare wiring
└── tests/
    └── smoke.rs
```

## Versioning, license, attribution

- Apache-2.0 OR MIT (matches zen ecosystem)
- `imazen` org on GitHub when published
- v0.1.0 is "works locally with a coefficient store"; v0.2 adds remote deploy + active
  trial selection.

## Hard "v0.1 is done when":

1. `cargo run` starts the server; it serves the frontend at `http://localhost:3030`.
2. Pointing it at a local coefficient store, the trial loop loads real image triplets.
3. Submitting a response writes a row to SQLite that round-trips through the export
   TSV in valid zenanalyze schema.
4. Viewing conditions (dpr, intrinsic-to-device ratio, viewport size, displayed CSS
   size, calibration mm/px) are recorded and present in the export.
5. Smoke test passes (`cargo test --test smoke`).
