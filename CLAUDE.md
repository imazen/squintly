# Squintly — agent notes

Browser-based psychovisual data collection for zensim. See [SPEC.md](SPEC.md) for the
design and [README.md](README.md) for the elevator pitch.

## Architecture in one paragraph

Single Rust binary (axum). Embeds the Vite-built TS frontend via `rust-embed`. SQLite
via `sqlx` for trial/response storage. Talks to coefficient over HTTP (default) or by
reading its SplitStore directly. Frontend is vanilla TS with the `@aspect/image-compare`
web component from `~/work/efficient-ui/`. No framework.

## Key invariants

- **Never mutate coefficient.** Squintly is a *consumer* of coefficient's image store;
  it never writes back. Aggregated TSVs are the export channel.
- **Viewing conditions are first-class data, not telemetry.** Every response row carries
  the conditions that produced it. We never aggregate them away in storage.
- **Anonymous only.** No login, no email, no IP logging beyond a hashed bucket. The
  observer ID is a UUID in localStorage.
- **2AFC by default.** Faster signal-per-second than continuous rating, easier to model
  via Bradley–Terry. v0.2 may add JND staircase.
- **Source-informing-sweep rule applies.** Sampling MUST cover all 4 size buckets and
  weight low-q encodings. See `src/sampling.rs`.

## Where to look

- `src/main.rs` — entrypoint, CLI, axum router
- `src/coefficient.rs` — both Http and Fs impls of the Coefficient trait
- `src/handlers.rs` — HTTP route handlers
- `src/sampling.rs` — trial pair selection
- `src/bt.rs` — Bradley–Terry-with-ties fit (Davidson 1970)
- `src/export.rs` — TSV streaming in zenanalyze schema
- `src/curator.rs` — corpus curator backend (`/api/curator/*`)
- `src/licensing.rs` — per-corpus license registry surfaced in UI + exports
- `web/src/curator.ts` — curator Stream/Curate/Threshold screens
- `web/src/curator-encoder.ts` — browser-canvas JPEG encoder for the slider
- `web/src/conditions.ts` — browser-side viewing-condition capture
- `web/src/calibration.ts` — credit-card mm-per-px calibration

## Curator data flow

1. Operator POSTs a candidate manifest to `/api/curator/manifest`. Either
   corpus-builder TSV (e.g. `/mnt/v/output/corpus-builder/curated_manifest_2026-04-16.tsv`)
   or the unified R2 JSONL at
   `https://pub-7c5c57fd3e0842f0b147946928891d40.r2.dev/manifest.jsonl`.
   Inserted into `curator_candidates` with per-corpus license attribution
   from `src/licensing.rs`.
2. The browser fetches `/api/curator/stream/next?curator_id=<uuid>` to get
   the next undecided candidate plus a default-on suggestion (groups + size
   chips) computed from the source's detected q (when available).
3. Curator swipes left/right or taps Skip/Take. `Take` advances to the
   Curate screen for group selection + size-chip toggling. `Find threshold`
   opens the slider with both 1:1-device-px and 1:1-CSS-px split panels.
4. `POST /api/curator/threshold` saves `q_imperceptible` along with the
   measurement DPR, distance, and encoder identity. `GET
   /api/curator/export.tsv?curator_id=…` joins everything into one TSV
   carrying the license columns downstream consumers need.

## License posture

Squintly never claims to know per-image licenses unless the manifest
provides them. The `licensing` registry maps **corpus** to policy. The
welcome screen shows a credits panel; the curator screens show inline
badges; trial cards show a corpus + license label. When the live R2
manifest grows per-image `license_url` fields, the existing
`curator_candidates.license_url` column carries them through to exports.

## Running locally

```bash
just dev       # cargo watch + vite dev with proxy
just build     # build frontend then cargo build --release
just test
```

## Investigation Notes

(none yet)

## Known Bugs

### Curator write endpoints are unauthenticated on a public instance

`POST /api/curator/manifest`, `/api/curator/decision`, `/api/curator/threshold`
and `/api/curator/generate-variant` take no token — only `load-r2-public` and
`backfill-dims` call `require_curator_admin`. On the public Railway deployment
that means anyone can inject candidates, record decisions, and drive variant
generation. The SSRF that fell out of this is now closed (`guard_blob_url`,
2026-07-27), but the underlying posture is unresolved: the curator is a
*researcher* tool exposed on an *anonymous-participant* origin.

Options, none chosen yet: gate every `/api/curator/*` mutation behind
`SQUINTLY_SUGGESTION_ADMIN_TOKEN`; serve the curator from a separate
deployment; or accept junk-injection risk and treat `curator_candidates` as
untrusted at export time. **Decide before advertising the live URL widely.**

### R2 corpus blobs have no CORS — canvas paths must use the proxy

The canonical bucket (`pub-…​.r2.dev`) serves blobs with **no**
`access-control-allow-origin` and `content-type: application/octet-stream`
(measured 2026-07-27). Plain `<img>` display works; anything that reads canvas
pixels back — the curator threshold encoder, the preview strip — must load via
the same-origin `GET /api/curator/blob/{sha256}` proxy (`curator::blob_proxy`),
because `<img crossOrigin="anonymous">` against R2 fails to load outright.
`web/e2e/mock-coefficient.ts` is deliberately CORS-less so e2e reproduces this;
two specs in `curator.spec.ts` guard it. Fixing the bucket's CORS config would
be a fine *additional* step but must not be the only defence — the proxy works
regardless of who owns the bucket.

### Live instance serves no trials

`SQUINTLY_COEFFICIENT_HTTP` on Railway is still the `coefficient.example.com`
placeholder, so the manifest is empty and `/api/trial/next` returns 409. The
rating flow cannot be exercised in production until a real coefficient (or a
baked `SQUINTLY_COEFFICIENT_PATH` corpus) is wired in. Welcome / calibration /
suggest work; curator works once a manifest is POSTed (DEPLOY.md §14).

### Unknown `/api/*` paths return HTML with the extension's content-type

The SPA catch-all serves `index.html` for unmatched routes, and the content-type
is guessed from the *request path*, so `GET /api/nope.json` answers `200` +
`content-type: application/json` with an HTML body. Harmless in the browser,
confusing for API clients probing for a route's existence — it's why a missing
endpoint reads as "present but unparseable" rather than 404.

## Resolved bug log

### Docker/Railway deploys broken for two months (fixed 2026-07-27)

`src/handlers.rs` read the git commit with `env!("SQUINTLY_BUILD_COMMIT")` (set
by `build.rs`), but the **Dockerfile never copied `build.rs`** — so the build
script didn't run in the container and the crate failed to compile. Landed with
the build_commit feature on 2026-05-28; the last successful deploy was
2026-05-07, so `main` was undeployable for ~2 months while the stale May image
kept serving a healthy `/api/stats`.

Why it hid so well: `cargo test` / `just ci` build from the working tree where
`build.rs` exists, so they were always green; `railway up --detach` exits 0 even
when the build later fails; and the healthcheck passed because the *old* image
was still running. Nothing in the local loop touched the Dockerfile.

Fixes: `COPY build.rs` (+ a `SQUINTLY_BUILD_COMMIT` build arg) in the
Dockerfile; `option_env!(...).unwrap_or("unknown")` so a missing build script
degrades provenance instead of bricking the build (which is what the doc comment
already claimed); a startup `warn!` when the commit is `unknown`; and
`just railway-deploy` now depends on `just docker-build`. Verify any deploy with
the `build_commit` check in DEPLOY.md §13.

### unified.rs solver diverged on synthetic data (fixed 2026-05-28)

Three issues found and fixed in `src/unified.rs::fit_unified`:

1. **NaN in `d_log_sigma_o` when rating == 4** (`upper = +∞`). `pdf_u`
   was guarded to 0 for the infinite case but `upper * pdf_u = ∞ · 0`
   still produced NaN, cascading into `log_σ_o`. Fix: explicit `0.0`
   guards on both infinite arms.
2. **No prior on global `log_sigma`** → drifted into the σ ≫ 1 flat
   region where `dl_dz · (-z) → 0`. Observed σ = 2361 (BT-only ≈ 0.34).
   Fix: added `log_σ ~ N(0, 1²)` matching `log_σ_o`'s prior shape.
3. **Rating-index sign convention inverted**. Squintly's UI uses
   rating 1 = imperceptible = BEST quality, but the cumulative-link
   model treated higher μ as worse — so pair and rating signals
   contradicted, and the fit inferred m upside-down. Fix:
   `k_idx = 4 - rating` inside both the fit loop and
   `rating_log_likelihood`. Now both modalities agree that higher
   m = better quality.

Regression test:
`src/unified.rs::tests::unified_competitive_with_bt_only_on_heldout_pairs`
seeds 4 items × 240 pair obs × 120 rating obs (incl. rating = 4 trials
that exercise bug #1) and asserts the held-out per-trial Δ vs BT-only
is within ±0.1 nats. Pre-fix Δ was −0.5 nats; post-fix ≈ 0.
