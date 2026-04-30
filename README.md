# Squintly

> Pairwise psychovisual data collection for [zensim](https://github.com/imazen/zensim).
> Browser-based, viewing-condition-aware, coordinates with [coefficient](https://github.com/imazen/coefficient).

Existing public IQA datasets (KADID-10k, TID2013, CID22) fix viewing conditions and
bake them into the labels. zensim plateaus around SROCC 0.82 on these — the residual is
dominated by *how* an image is being viewed: device pixel ratio, intrinsic-to-device
ratio, viewing distance, ambient light, gamut. Squintly collects pairwise judgments
**with those conditions recorded**, so zensim can learn to condition on them.

## Quick start (local)

```bash
# 1. Have a coefficient store available (HTTP or filesystem path)
just build
cargo run -- --coefficient-http http://localhost:8081 --port 3030

# Or:
cargo run -- --coefficient-path /path/to/coefficient/benchmark-results --port 3030

# 2. Open http://localhost:3030 in any browser, on any device. Calibrate, rate a few.

# 3. Export when done:
curl http://localhost:3030/api/export/pareto.tsv     > pareto_human.tsv
curl http://localhost:3030/api/export/thresholds.tsv > thresholds.tsv
curl http://localhost:3030/api/export/responses.tsv  > responses_raw.tsv
```

## Deploy to Railway

```bash
railway init --name squintly
railway volume add --mount-path /data
railway variables --set "SQUINTLY_COEFFICIENT_HTTP=https://your-coefficient-host"
railway up --detach
```

Full walkthrough in [DEPLOY.md](DEPLOY.md). The Dockerfile builds the embedded
TS frontend, then the Rust binary, then ships a minimal `debian:bookworm-slim`
runtime — no Node at runtime.

## What gets collected

Per session (stable):
- `devicePixelRatio`, screen size, color gamut, dynamic range, OS/browser
- Optional self-reported: viewing distance, ambient light, vision-corrected, age bracket
- Optional credit-card calibration → CSS px per millimeter on this physical screen

Per trial (variable):
- Viewport size (handles orientation flips on phones)
- Image intrinsic, displayed-CSS, device-pixel dimensions
- `intrinsic_to_device_ratio` — the headline number
- Dwell time, swap count, zoom usage

## Output

`pareto.tsv` is in the [zenanalyze/zentrain](https://github.com/imazen/zenanalyze)
training schema, with `quality` set to a Bradley–Terry-fitted scalar score derived
from pairwise preferences (anchored at 100 = reference). Drops in as a replacement
for the zensim column.

See [SPEC.md](SPEC.md) for the full design.

## Status

**v0.1** — works locally pointed at a coefficient store. No remote deploy.

## License

Apache-2.0 OR MIT
