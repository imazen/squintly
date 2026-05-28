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

### unified.rs solver diverges on synthetic data (2026-05-28)

`src/unified.rs::fit_unified` does not converge when both pair and rating
observations are present and consistent with the same latent m. On a 4-item
4-tier-rating synthetic dataset (SmallRng seed 7, 240 pair obs, 120 rating
obs, m range [0, −1.2]):

| Param | Unified fit (broken) | BT-only fit (working) |
|---|---|---|
| `m` | [0, +0.31, −0.05, −0.41] | [0, −0.27, −0.59, −0.91] |
| `σ` | 2361.16 | 0.343 |
| `τ` | [−1356.29, 0.19, 1.05] | (n/a) |
| `log_σ_o` | NaN | (n/a) |
| `iterations` | 800 (max) | 204 (converged) |
| held-out pair LL | −24.95 | −7.08 |

Held-out pair log-likelihood is **0.5 nats/trial worse** for unified
than BT-only on the same training set — the unified-spec literature
predicts the opposite direction.

Suspected causes, ranked:
1. `grad_log_sigma_o` chain rule wrong — the `lower.max(-1e6)` hack at
   `src/unified.rs:~181` suggests the `−∞` lower branch was patched
   rather than re-derived. NaN in `log_σ_o` confirms this branch is broken.
2. `grad_log_sigma` for the pair Thurstone term has wrong sign / scale.
   Visible as σ running away when ratings are added but staying sane
   in BT-only mode.
3. `tau` monotonicity sort after each update step can clobber the
   gradient direction for any tau that wasn't violating monotonicity.

What still works: `pair_log_likelihood` / `rating_log_likelihood` /
`total_log_likelihood` operate correctly on any `UnifiedFit` regardless
of how it was produced. H3 pilot evaluation can use these against fits
from the reference implementations under `/mnt/v/repos/iqa-tools/`
(pwcmp + a Pérez-Ortiz cumulative-link port) until the in-tree solver
is rewritten.

Production: v0.1 squintly uses `src/bt.rs::fit` (BT-Davidson), which
converges correctly. The threshold staircase covers the rating modality
independently. The unified solver is not on any v0.1 hot path.
