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

(none yet)
