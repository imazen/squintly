# Squintly Corpus Curator — Spec

> Status: v0.1 design — not yet implemented. Adds a corpus-development mode
> to Squintly alongside the existing rating UI. Phone-first, gesture-driven,
> with WASM background encoders for low-latency quality-threshold finding.

## 1. Mission

Curate the image corpora that feed downstream training:

| Corpus group | Target consumer | Typical use |
|---|---|---|
| **core × zensim** | zensim metric training (small, golden) | tight feature-vector calibration |
| **medium × zensim** | zensim training (larger, validation-friendly) | held-out SROCC reporting |
| **full × zensim** | zensim training (everything) | production-scale fitting |
| **core × encoding** | codec sweep (small, fast) | smoke + regression sweeps |
| **medium × encoding** | codec sweep (production) | picker training data |
| **full × encoding** | codec sweep (everything, archival) | offline benchmarking |

A single source image can be promoted into any combination of these six
groups. The curator UI is the human-in-the-loop that decides which
images deserve inclusion in which groups.

For each accepted image the curator also produces:

1. **Size variants** — Mitchell-Netravali downscales via `zenresize` to a
   user-selected subset of target maxdims. Each variant becomes a
   distinct corpus entry tagged with its `size_class`.
2. **Per-image quality thresholds** — `q_imperceptible`: the lowest jpegli
   `q` at which the curator (a single trusted human, not an external
   observer pool) cannot tell the variant apart from the source under
   normal viewing. This becomes a strong supervisory signal for
   threshold modelling — orthogonal to the Squintly observer-pool
   `q_notice/q_dislike/q_hate` distributions.
3. **Auto-downscale recommendation** — based on the source's detected
   compression level, the curator never produces variants larger than
   `source_dim / detected_downsample_factor` so we don't oversample
   generation loss.

## 2. UX overview

Three screens:

```
┌─────────────────────┐    ┌─────────────────────┐    ┌─────────────────────┐
│  STREAM             │───▶│  CURATE (per image) │───▶│  THRESHOLD          │
│  Pick next image    │    │  Group toggles +    │    │  q-slider w/ split  │
│  Swipe LEFT = skip  │    │  size selectors +   │    │  preview, releases  │
│  Swipe RIGHT = take │    │  Confirm / Back     │    │  → save threshold   │
└─────────────────────┘    └─────────────────────┘    └─────────────────────┘
        ▲                                                     │
        └─────────────────  next image  ──────────────────────┘
```

### 2.1 Stream screen (default landing)

- Single image filling the viewport (CSS `object-fit: contain`).
- Top status row: `i/N`, undo button, settings cog.
- Bottom tab bar: `Curator` / `Rate` (existing rating mode) / `Calibrate`.
- **Gestures (touch)**:
  - Swipe right (>30% width) → mark as candidate, advance to Curate screen
  - Swipe left (>30% width) → reject, skip to next image
  - Swipe down → flag/skip with reason picker
  - Swipe up → next without recording (peek mode)
  - Tap and hold image → metadata overlay (source, dimensions, detected source-q)
- **Keyboard equivalents** (desktop / Bluetooth keyboard):
  - `→` / `f` → take
  - `←` / `s` → skip
  - `space` → peek metadata

The next image is preloaded as soon as the current one mounts (single-
image lookahead — phone bandwidth is the constraint, not RAM).

### 2.2 Curate screen

Reached via swipe-right from Stream. Shows the source image at top
(half viewport) plus a control panel below.

**Group matrix** — 2×3 toggle grid with large hit targets (≥56×56 dp):

```
              zensim    encoding
   core         ☐         ☐
   medium       ☐         ☐
   full         ☐         ☐
```

Default-on selection is determined by source heuristics (see §6). The
curator can quickly accept defaults with one tap or override per cell.

**Size variants** — horizontal chip strip with 8 dimensions:

```
[ 64 ] [ 128 ] [ 256 ] [ 384 ] [ 512 ] [ 768 ] [ 1024 ] [ 1536 ]
```

Each chip is a checkbox. Default-on chips are computed from the
detected source-q + native size (see §6). Greyed-out chips are sizes
that would *upscale* the source — not allowed (see §6).

Two action buttons at the bottom:

- **Find threshold** → pushes to Threshold screen for the largest
  enabled size variant; on return, threshold value is saved + the
  curator proceeds to Confirm.
- **Save without threshold** → records the entry + queues threshold
  finding for later (batch mode).

### 2.3 Threshold screen — the q-slider

Reached from Curate's "Find threshold" button.

This is the high-attention interaction. UX optimized for the curator
to drag the slider until they perceive degradation, releasing at the
lowest imperceptible q.

**Layout while dragging**:

```
┌──────────────────────────────────────────────┐
│   ┌────────────┐  ┌────────────┐             │
│   │  ENCODED   │  │ UNCOMPRESS │             │
│   │   (1:1     │  │    (1:1    │             │
│   │   device)  │  │    CSS)    │             │
│   │            │  │            │             │
│   └────────────┘  └────────────┘             │
│                                              │
│   ━━━━━━━━━━━○━━━━━━━━━━━━━━━━━━━━           │
│   q = 67                                     │
└──────────────────────────────────────────────┘
```

- Two crop panels side-by-side, **identical pixel-coordinate window**
  on the source.
- **Left panel**: encoded variant rendered at **1:1 device-pixel ratio**
  (`image-rendering: pixelated`, `transform: scale(1/dpr)` adjustments
  so each device pixel of the encoded buffer maps to one device pixel
  of the screen). This is the "what the file actually contains" view.
- **Right panel**: same encoded variant rendered at **1:1 CSS-pixel
  ratio** — i.e., the image's intrinsic pixels are scaled to CSS pixels
  at the device's DPR. This is the "what the user sees" view at typical
  rendering pipeline. On a 3× DPR phone the right panel shows the same
  image 3× smaller than the left.
- All other UI hidden — only the slider remains.
- Slider is a `<input type="range">` styled large; native iOS/Android
  haptic clicks on integer steps when supported.

**On slider release** (`pointerup` / `touchend` / `change`):
- All UI reappears (group panel, navigation, etc.)
- Both panels swap to the **uncompressed source** at the same 1:1 split,
  giving the curator a fixed reference for the saved threshold.
- The released q value becomes `q_imperceptible_candidate`.

**Auto-bisection assist**: at slider mount we encode at q ∈ {30, 50, 70,
85, 95} in parallel (via the WASM workers, §5). The slider snaps to
visited values plus shows tick marks where samples exist; intermediate
values trigger a JIT encode on slider hover-pause (>80 ms).

**Threshold confirmation**: a "−1 q" / "+1 q" pair of micro-buttons
appears post-release for the curator to nudge by 1 step before saving.

### 2.4 Background activity

- Source preloading (next image in stream) runs continuously.
- WASM workers idle between curations; pre-warm jpegli at app start.
- All HTTP traffic goes through the existing `coefficient` SplitStore
  proxy — same pattern as the rating UI.

## 3. Data model

### 3.1 Storage location

Curator output writes to **two** sinks:

1. **Squintly internal DB** (sqlx + Postgres on Railway): authoritative
   record of curator decisions, threshold candidates, group memberships,
   and the curator's identity/timestamp. Schema in §3.3.
2. **Coefficient SplitStore**: the curated corpus itself, with the
   manifest extension defined in §3.4. Squintly writes via the existing
   coefficient API; coefficient is the durable source of truth for
   downstream consumers (zenmetrics sweeps, zensim training).

### 3.2 Source manifests Squintly reads

The stream screen feeds from a `candidate_manifest.tsv` that the curator
selects on first use:

```
sha256          width   height  size_bytes  source           relative_path
a1b2…           4624    3468    5014086     source_jpegs     foo.jpg
…
```

Compatible with `corpus-builder`'s output (e.g.
`/mnt/v/output/corpus-builder/curated_manifest_2026-04-16.tsv`); the
curator runs *after* an automated curation pass narrows the candidate
pool.

### 3.3 Database schema (Postgres on Railway)

```sql
CREATE TABLE curator_decisions (
    id              BIGSERIAL PRIMARY KEY,
    source_sha256   BYTEA       NOT NULL,
    curator_id      TEXT        NOT NULL,
    decided_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    decision        TEXT        NOT NULL,           -- 'take' | 'reject' | 'flag'
    reject_reason   TEXT        NULL,                -- when decision='flag'
    -- Group membership (NULL means: not in this group)
    in_core_zensim       BOOLEAN  NOT NULL DEFAULT false,
    in_medium_zensim     BOOLEAN  NOT NULL DEFAULT false,
    in_full_zensim       BOOLEAN  NOT NULL DEFAULT false,
    in_core_encoding     BOOLEAN  NOT NULL DEFAULT false,
    in_medium_encoding   BOOLEAN  NOT NULL DEFAULT false,
    in_full_encoding     BOOLEAN  NOT NULL DEFAULT false,
    -- Source profile (filled by detect-pass, §6)
    source_codec         TEXT     NULL,             -- 'jpeg' | 'png' | 'webp' | 'avif' | 'jxl' | 'gif'
    source_q_detected    REAL     NULL,             -- [0, 100] for jpeg; NULL otherwise
    source_w             INT      NOT NULL,
    source_h             INT      NOT NULL,
    -- Auto-downscale recommendation (informative, not enforcing)
    recommended_max_dim  INT      NULL,
    UNIQUE (source_sha256, curator_id)
);

CREATE TABLE curator_size_variants (
    decision_id     BIGINT      NOT NULL REFERENCES curator_decisions(id) ON DELETE CASCADE,
    target_max_dim  INT         NOT NULL,           -- 64, 128, …, 2048
    generated_sha256 BYTEA      NULL,               -- filled when zenresize completes
    generated_path   TEXT       NULL,               -- coefficient SplitStore key
    PRIMARY KEY (decision_id, target_max_dim)
);

CREATE TABLE curator_thresholds (
    decision_id     BIGINT      NOT NULL REFERENCES curator_decisions(id) ON DELETE CASCADE,
    target_max_dim  INT         NOT NULL,           -- threshold can vary with size
    q_imperceptible REAL        NOT NULL,
    measured_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    measurement_dpr REAL        NOT NULL,           -- device DPR at time of measurement
    measurement_distance_cm REAL NULL,              -- if we have a calibration value
    PRIMARY KEY (decision_id, target_max_dim)
);
```

### 3.4 Manifest extension for coefficient

Coefficient SplitStore manifest gains four optional columns:

```
… (existing columns) … curator_groups  q_imperceptible  source_q_detected  downscale_factor
```

`curator_groups` is comma-separated — `core_zensim,medium_zensim,full_zensim,core_encoding,medium_encoding,full_encoding`.

## 4. API surface (Rust / axum)

All endpoints under `/api/curator/*`.

```
GET  /api/curator/stream/next?manifest=<path-or-sha>
   → { source_sha256, source_url, source_w, source_h, source_codec,
       source_q_detected, recommended_max_dim, suggested_groups,
       suggested_sizes }

POST /api/curator/decision
   { source_sha256, decision, groups, sizes }
   → { decision_id }

POST /api/curator/threshold
   { decision_id, target_max_dim, q_imperceptible, measurement_context }
   → { ok: true }

POST /api/curator/generate-variant
   { decision_id, target_max_dim }
   → { variant_url, generated_sha256 }
   (zenresize Mitchell downscale; fires only after the curator confirms;
    backend dispatches to a worker pool — async with progress events
    over SSE on /api/curator/events).

GET  /api/curator/events  (SSE)
   → variant_ready, threshold_saved, error events.

GET  /api/curator/export.tsv?run_id=<id>
   → curator_decisions × curator_size_variants × curator_thresholds joined
     for downstream consumers (zenmetrics sweep manifest format).
```

## 5. Frontend architecture (TypeScript / vanilla)

Add four new modules under `web/src/`:

| Module | Role |
|---|---|
| `curator.ts` | Top-level coordinator: routing between Stream / Curate / Threshold; reuses `trial.ts`'s gesture wiring |
| `curator-api.ts` | Wraps `/api/curator/*` endpoints |
| `curator-encoder.ts` | WASM-side jpegli encode for the threshold slider; Web Worker |
| `curator-detector.ts` | WASM-side zenjpeg `detect` for source-q recovery |

### 5.1 WASM workers

Two WASM modules ship in `web/wasm/`:

1. **zenjpeg-encoder.wasm** — built from `zenjpeg` with a thin
   `wasm-bindgen` shim exposing:
   ```rust
   #[wasm_bindgen]
   pub fn encode(rgba_pixels: &[u8], w: u32, h: u32, q: f32) -> Vec<u8>
   ```
   Wrapped in a `Worker`-side message handler so the main thread stays
   responsive.

2. **zenjpeg-detector.wasm** — built from `zenjpeg::detect` with:
   ```rust
   #[wasm_bindgen]
   pub fn detect(jpeg_bytes: &[u8]) -> JsValue  // { q_estimated, downsampled_factor, ... }
   ```

   For non-JPEG sources (PNG / WebP / AVIF / JXL), the detector returns
   `null` — handled by the frontend with a default `q_estimated = 100,
   downsampled_factor = 1.0`.

3. **zenresize.wasm** is **not** in the browser. Mitchell downscale
   happens on the Rust backend (axum side), invoked via
   `/api/curator/generate-variant`. Reasons:
   - zenresize is a heavy crate (multiple downsample kernels with SIMD)
   - Variants are persistent artifacts written to coefficient — backend
     writes them once with the canonical, deterministic Rust path.
   - Browser-side resize would be redundant work the curator can't
     verify against the production pipeline.

### 5.2 Slider responsiveness target

- p50 round-trip from slider input event → encoded bytes → split-panel
  paint: **≤ 80 ms** at q < 90 on a typical 1024 px source.
- Pre-encoded snapshot at 5 anchor q values during slider mount means
  any cold drag has visible bytes within ≤16 ms.

### 5.3 Split panel rendering

Two `<canvas>` elements side-by-side. The encoded buffer (RGBA from
WASM jpegli decode) is drawn to both:

```ts
function renderSplit(encodedRgba: Uint8Array, sw: number, sh: number) {
  const dpr = window.devicePixelRatio;
  // Left: 1:1 device pixels — canvas.width = sw, scale by 1/dpr in CSS
  drawAt(left,  encodedRgba, sw, sh, 1.0,   /* css scale */ 1.0 / dpr);
  // Right: 1:1 CSS pixels — canvas.width = sw * dpr, css scale 1.0
  drawAt(right, encodedRgba, sw, sh, dpr,   /* css scale */ 1.0);
}
```

The crop window in source coordinates is identical on both panels.
Curator can pan the crop with a two-finger drag; default crop window
is image center, panel-sized.

## 6. Source compression detection + auto-downscale

This is the "don't oversample generation loss" rule.

`zenjpeg::detect::analyze(bytes)` (existing, see
`/home/lilith/work/zen/zenjpeg/zenjpeg/src/detect/`) returns an estimate
of the original encoding quality and detected downsampling factor — for
JPEG sources this is well-defined; for other formats we treat the input
as effectively lossless (q ≈ 100).

### 6.1 Auto-downscale rule

Given source dimensions `(W, H)` and detected `q_source`:

```
detected_q ≥ 95 → safe_max_dim = max(W, H)              # no oversampling guard
detected_q ≥ 85 → safe_max_dim = max(W, H) / 2          # 1 step down
detected_q ≥ 75 → safe_max_dim = max(W, H) / 4          # 2 steps down
detected_q ≥ 60 → safe_max_dim = max(W, H) / 4          # still 2 — but mark "low source q" warning
detected_q <  60 → safe_max_dim = max(W, H) / 8         # 3 steps; flag in UI
```

Then the size-variant chips in the Curate screen are masked: any
target dim > `safe_max_dim` is greyed out and unselectable.

Rationale: a JPEG already at q=70 has baked-in quantization noise that
becomes part of the "ground truth" if we resize to its native dim.
Downsampling 2-4× before further encoding breaks the per-pixel
correlation enough that the new encoder can't trivially "see" the
source's quantization fingerprint.

### 6.2 Group default suggestions

Heuristic populating the group matrix's default-on cells:

```
if source_q_detected ≥ 95 OR source_codec ∈ {png, jxl_lossless}:
    default-on:  core_zensim, core_encoding
elif 85 ≤ source_q_detected < 95:
    default-on:  medium_zensim, medium_encoding
elif source_q_detected ≥ 70:
    default-on:  full_zensim, full_encoding
else:
    default-on:  (nothing — curator must explicitly opt in)
```

Curator overrides freely; defaults are just to speed up high-volume
runs.

## 7. Integration with existing pipelines

- **corpus-builder** (already at `/mnt/v/output/corpus-builder/`) feeds
  Squintly the candidate manifest. Squintly outputs an enriched
  manifest that becomes the next sweep's source list.
- **zenmetrics sweep** (`/home/lilith/work/zen/zenmetrics/scripts/sweep/`)
  consumes the curator-output manifest's `curator_groups` column to
  select images for `core_encoding` / `medium_encoding` / `full_encoding`
  sweeps. The sweep launcher (`launch_gpu.sh`) gains a
  `--curator-group=<name>` flag.
- **zensim training** (`zensim/zensim-validate/`) consumes the
  `q_imperceptible` thresholds as a supervisory signal — pairs
  near-but-below the threshold get higher loss weight than pairs deep
  in the imperceptible zone.

## 8. Phone-first UX guarantees

- All gestures work in portrait. Threshold screen rotates to landscape
  for wider crop window — but the UI lock prevents mid-trial rotation
  changes from corrupting the recorded measurement DPR.
- Tap targets ≥56 dp.
- One-handed-thumb operation possible — the slider is at the bottom
  third of the viewport.
- Battery-aware: WASM jpegli encoder shuts down after 60 s of
  inactivity; resumes on next slider interaction.

## 9. Decisions I need from you before building

1. **Curator identity model.** Do we have one curator (you) or a small
   pool? If multiple, do we record per-curator decisions independently
   and aggregate at export time, or do we want consensus voting?

2. **Source manifest format.** Stick with `corpus-builder`'s TSV, or
   switch Squintly to a richer JSON-Lines candidate stream that can
   carry the detected source-q already?

3. **Variant generation cadence.** Generate Mitchell variants
   *immediately* on accept (synchronous, blocks the next image), or
   queue them in the backend and let the curator keep moving (async,
   variants land minutes later)? I recommend async; UX feels lighter.

4. **Storage of WASM-encoded buffers.** Do we save threshold-
   measurement-time encoded bytes back to coefficient (so downstream
   trainers can use exactly-the-bytes-the-curator-saw), or do we only
   record the q value and re-encode at consumption? Saving costs
   storage but kills any encoder-version-skew bugs.

5. **`zensim` group definitions.** Do `core/medium/full` translate to
   ~50 / ~500 / ~5000 images respectively, or different numbers?
   `corpus-builder` already produces a curated set of 983 — does
   "full" map to the entire candidate-manifest output, or a subset?

## 10. Out of scope for v0.1

- HDR / wide-gamut threshold work (separate calibration; needs
  display profile read-back which iOS doesn't expose)
- Video / animated images
- Cross-device aggregation (a curator's decisions on phone vs laptop
  will diverge — model later, not now)
- Active learning to prioritize candidates (the manifest comes
  pre-curated by `corpus-builder`; Squintly is the human filter on
  top, not the prioritizer)
- Integration with the Squintly observer-pool rating UI — this is a
  developer-facing curator tool with one user; consumer rating is the
  existing flow, not extended here.
