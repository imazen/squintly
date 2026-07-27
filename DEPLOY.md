# Deploy Squintly to Railway

> **Live deployment**: https://squintly-production.up.railway.app on Railway
> Hobby plan, ~$7-9/month with 5GB volume. All four flows work; the corpus is
> served from public R2 (§15).
>
> Historical note worth keeping: deploys were **silently broken from 2026-05-28
> to 2026-07-27** — the Dockerfile didn't `COPY build.rs`, so
> `env!("SQUINTLY_BUILD_COMMIT")` failed to compile and every `railway up`
> errored while the May 7 image kept serving a healthy `/api/stats`. See §13 for
> the guard that catches it now.

Single Rust service + persistent volume for the SQLite DB. Modeled on
[interleaved's deployment flow](../interleaved/DEPLOY.md). The image store is a
static coefficient-shaped store on public R2 (§15) — no coefficient instance
needs to run.

## Prerequisites

- [Railway CLI](https://docs.railway.com/guides/cli): `npm i -g @railway/cli`
- `railway login` (one-time)
- An image store: either the R2 static store (§15) or a running coefficient

## 1. Create the Railway project

```bash
cd ~/work/squintly
railway init --name squintly
```

This creates the project and links the CWD to it. Reuse an existing project
with `railway link` if you already have one.

## 2. Add a persistent volume for the SQLite DB

```bash
railway volume add --mount-path /data
```

This creates a Railway volume mounted at `/data` inside the container. The
Dockerfile sets `SQUINTLY_DB=/data/squintly.db`, so the database survives
redeploys.

If you'd rather use Postgres (recommended at any real scale), see §6.

## 3. Set environment variables

Required:

```bash
# The image store. Either the R2 static store (§15) or a coefficient viewer.
# NOTE: this takes precedence over SQUINTLY_COEFFICIENT_PATH — a stale value
# here is why the site served zero trials for months.
railway variables --set \
  "SQUINTLY_COEFFICIENT_HTTP=https://codec-corpus.r2.imazen.org/squintly/demo-corpus/imazen26-v1"
```

Optional:

```bash
railway variables --set "RUST_LOG=info,squintly=info"
# SQUINTLY_DB and SQUINTLY_BIND default fine; PORT is auto-injected by Railway.
```

**Security-relevant, set these on any public deployment:**

```bash
# Hosts the server is willing to fetch candidate blobs from. Curator write
# endpoints are unauthenticated, so blob_url is attacker-supplied; without an
# allowlist the only thing standing between a stranger and your internal
# network is the resolved-IP check in curator::guard_blob_url.
railway variables --set \
  "SQUINTLY_BLOB_HOST_ALLOWLIST=pub-7c5c57fd3e0842f0b147946928891d40.r2.dev"
```

**Never** set `SQUINTLY_ALLOW_PRIVATE_BLOB_HOSTS=1` in production — it disables
the private-address check that blocks `169.254.169.254` and friends. It exists
for local dev and e2e, where the mock coefficient serves blobs from
`127.0.0.1`.

The Dockerfile already sets `SQUINTLY_BIND=0.0.0.0:3030` and the binary auto-
overrides the port to whatever Railway puts in `PORT`, so you don't need to
manage that.

## 4. Deploy

```bash
railway up --detach
```

Railway picks up `Dockerfile` + `railway.toml` automatically. First build
takes ~5 minutes; iterative builds use the cargo deps cache layer and take
~1–2 minutes.

## 5. Watch the logs / health

```bash
railway logs --tail
railway open                                 # open the deployment in a browser
curl https://<your-railway-domain>/api/stats # liveness check (and the configured healthcheck)
```

## 6. Optional: swap SQLite for Postgres

SQLite is fine for the v0.1 single-instance shape. If you need multi-instance
or you simply want managed backups, switch:

```bash
railway add --plugin postgresql
```

Railway sets `DATABASE_URL`. We'd need to:

1. Add a `--db-url` CLI flag (or read `DATABASE_URL`) in `src/main.rs`.
2. Add a `postgres` feature to the `sqlx` dep and gate the pool type.
3. Translate `migrations/0001_init.sql` → Postgres-compatible
   (TEXT/INTEGER/REAL → TEXT/BIGINT/DOUBLE PRECISION; `INTEGER PRIMARY KEY`
   becomes `BIGSERIAL`).

Tracked as v0.2 work.

## 7. Coefficient access

Three options:

- **Public coefficient.** Set `SQUINTLY_COEFFICIENT_HTTP` to its public URL.
  Easy; risks exposing the image manifest.
- **Coefficient on the same Railway project.** Run coefficient as a second
  service; both services share the project's private network. Set the env
  var to the private URL (`http://coefficient.railway.internal:PORT`).
- **Private + bastion.** Run coefficient privately, expose to Squintly via
  Railway's TCP proxy or a sidecar. Heaviest but most isolated.

The first two are recommended for v0.1.

## 8. Custom domain

```bash
railway domain                                  # current domain
railway domain --custom squintly.imazen.io      # add a custom domain
```

DNS: add a CNAME pointing at the Railway-assigned hostname. Cert is auto.

## 9. Local Docker smoke

To validate the image before pushing:

```bash
just docker-build
just docker-run
# in another shell:
curl http://localhost:3030/api/stats
```

`docker-run` mounts `/tmp/squintly-docker` as the volume, so the SQLite DB
persists across container restarts.

## 10. Updating

```bash
git push                  # if you've configured Railway's GitHub integration
# or
railway up --detach       # CLI deploy from local repo state
```

Railway runs the new image, drains the old one. The SQLite DB on `/data`
survives the swap.

## 11. Rolling back

```bash
railway redeploy <deployment-id>   # find IDs via `railway logs --json | head`
```

The volume is shared across deployments, so a roll-back doesn't lose data.

## 12. Common failures

| Symptom | Likely cause | Fix |
|---|---|---|
| Healthcheck fails after deploy | DB volume issue | The binary boots and serves `/api/stats` even when the image store is unreachable (logs the failure but doesn't crash, see `src/main.rs`), so a failing healthcheck is almost never the store. |
| `manifest_sources: 0` | `SQUINTLY_COEFFICIENT_HTTP` points somewhere empty/stale | Fetch `<base>/api/manifest` by hand. A base URL with a path prefix needs the prefix-preserving join added 2026-07-27 — older binaries dropped it and silently 404'd. |
| 409 on `/api/trial/next` | Empty manifest, or no encoding matches the session's decodable codecs | Check `/api/stats` first; then whether the browser declared support for any codec in the store. |
| DB resets between deploys | Volume not mounted | Run `railway volume add --mount-path /data` and redeploy. |
| Cargo build OOM | Default Railway builder memory cap | Bump build resources in the Railway dashboard or pre-build locally and push the image. |
| `environment variable SQUINTLY_BUILD_COMMIT not defined at compile time` | Dockerfile lost its `COPY build.rs` | Restore it. `build.rs` emits `cargo:rustc-env=SQUINTLY_BUILD_COMMIT`; without the file the build script never runs. |
| Deploy "succeeds" but the site never changes | The *previous* deploy failed and Railway kept serving the last good image | `railway deployment list` — check the top entry says SUCCESS, not FAILED. A failed `railway up --detach` exits 0. |

## 13. Always `docker build` before deploying

Railway builds from the `Dockerfile`, so a Dockerfile-only break is **invisible
to `cargo test` and `just ci`** — both build from the working tree, where
`build.rs` obviously exists. That is precisely how main stayed undeployable for
two months: every `railway up` failed, `railway up --detach` still exited 0, and
the old image kept serving a healthy `/api/stats`.

`just railway-deploy` now depends on `just docker-build`, so the container build
is exercised locally first. If you deploy by another route, run `just
docker-build` yourself.

Provenance check after any deploy — this is the cheap way to confirm the running
image is the one you think it is:

```bash
curl -s https://squintly-production.up.railway.app/api/export/pareto.manifest.json \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["build_commit"])'
```

It must print the commit you deployed. `unknown` means the build script didn't
run (the binary also logs a startup warning in that case); anything else means
you are looking at an older image.

## 14. Loading a curator corpus into the live instance

The curator needs candidates; it doesn't need coefficient. Point it at the
public R2 corpus (30 MB JSONL, content-addressed blobs at
`blobs/{sha[:2]}/{sha[2:4]}/{sha}`):

```bash
R2=https://pub-7c5c57fd3e0842f0b147946928891d40.r2.dev
curl -s "$R2/manifest.jsonl" -H 'range: bytes=0-262144' \
  | head -n 200 > /tmp/slice.jsonl
python3 - "$R2" <<'PY'
import json, sys, urllib.request
body = open('/tmp/slice.jsonl').read()
req = urllib.request.Request(
    'https://squintly-production.up.railway.app/api/curator/manifest',
    data=json.dumps({'kind': 'jsonl', 'body': body, 'blob_url_base': sys.argv[1]}).encode(),
    headers={'content-type': 'application/json'})
print(urllib.request.urlopen(req).read().decode())
PY
```

`POST /api/curator/manifest` upserts, so re-loading the same slice is a no-op.

## 15. The imazen-26 demo corpus (hosted on R2)

The rating flow needs a coefficient store. Rather than running one, the corpus
is published to a **public R2 bucket as a static coefficient HTTP store** —
`HttpCoefficient` only ever issues three GETs, and object keys can contain
slashes, so a plain bucket answers all of them with no server:

```
GET <base>/api/manifest             -> {"sources": [...], "encodings": [...]}
GET <base>/api/sources/<hash>/image
GET <base>/api/encodings/<id>/image
```

Live: `https://codec-corpus.r2.imazen.org/squintly/demo-corpus/imazen26-v1`
(32 sources, 512 encodings, 4 codecs), selected via
`SQUINTLY_COEFFICIENT_HTTP`.

```bash
# 1. Build the store from /mnt/v/imazen-26 (~10 min).
python3 scripts/build_demo_corpus.py --out demo-corpus --per-bucket 1 \
    --qualities 15 30 45 75 --clean

# 2. Publish it under a NEW version prefix.
just publish-corpus imazen26-v2

# 3. Point the deployment at it (no redeploy of the service needed).
railway variables --set \
  "SQUINTLY_COEFFICIENT_HTTP=https://codec-corpus.r2.imazen.org/squintly/demo-corpus/imazen26-v2"
```

**The prefix is versioned on purpose.** Publishing a new corpus never mutates
what a running study is reading; rolling back is one env var. Corpus changes and
code deploys are independent — swapping images no longer requires rebuilding
and redeploying the service.

`demo-corpus/` stays gitignored (119 MB of image blobs, far past the 30 KB
commit limit); it is a local build artifact now, not part of the image.

What the builder guarantees, and why (all from CLAUDE.md):

| Rule | How it is enforced |
|---|---|
| All four `export.rs` size buckets (S/M/L/XL) | Each source is emitted at four target dimensions; the script exits non-zero if a bucket ends up empty |
| Low-q weighted ladder | Default `15 30 45 60 80 92` — most rungs at/below q60, where web compression actually lives |
| Non-photo weighted | Documents, archival scans, icons and charts are first-class categories (imazen/squintly#4) |
| Truthful codec names | `libjpeg-turbo`, `jpegli`, `libwebp`, `libavif` — the actual encoder, never a stand-in |
| Honest licensing | Four separate policies in `src/licensing.rs`; only public-domain / operator-owned folders are selected by default, because a public bucket redistributes these bytes |

Content types are set explicitly at upload: the keys end in `/image` with no
extension, and R2 serves `application/octet-stream` for anything it wasn't
told about.

`SQUINTLY_COEFFICIENT_HTTP` takes precedence over `SQUINTLY_COEFFICIENT_PATH`
in `src/main.rs`. A stale value there is why the live site served zero trials
for months — check it first if the manifest is empty.

Screenshots (`screen-ui`) are excluded from the default selection: they are
captures of third-party sites, fine for local measurement but a redistribution
question on a public bucket. Add them with `--include ... screen-ui` locally.

### Why `codec-corpus` and not `coefficient`

There is a public `coefficient` bucket, and the name is tempting. Don't use it:
it is the coefficient system's **live operational store** — 77k objects, 8.6 GiB
of `jobs/`, `claims/`, `partials/`, `heartbeats/`, `binaries/`. Publishing a
demo corpus into another system's working bucket is how unrelated things start
breaking each other.

`codec-corpus` is the corpus-images bucket, and squintly already uses it: the
curator's `manifest.jsonl` and the `suggestions/` store both live there. It also
already holds `imazen-26-png-v3/` (below).

### The canonical corpus already on R2

`codec-corpus/imazen-26-png-v3/` holds a richer, better-stratified imazen-26
(21 numbered strata + a `nope/` reject bin, 2639 objects, 15.5 GiB) including
plots, mobile/web screenshots, AI-generated imagery and patent scans — strata
the local `/mnt/v/imazen-26` layout does not separate. Moving
`build_demo_corpus.py` onto it would directly improve imazen/squintly#4
coverage. **Not done yet**; measured groundwork so the next attempt is short:

```bash
rclone lsf r2:codec-corpus/imazen-26-png-v3 --files-only -R > v3-keys.txt
```

- **Dimensions parse from the filename for 2639/2639 keys (100%)** — pattern
  `..._<W>x<H>.sdr.png`, e.g. `5300_noaa_nhc-verification-report-2024_page1_p01_2550x3300.sdr.png`.
  A stratified selection can therefore be made from the key listing alone; only
  the chosen files need downloading, not 15.5 GiB.
- **Size distribution is top-heavy**: XL 1263, L 1296, M 78, S 2. The four
  `export.rs` buckets still have to be produced by downscaling, exactly as the
  current builder does — do not expect to find natural S/M sources.
- **Exclude `nope/`** (404 objects) — it is a reject bin, not a stratum.
- Licensing for imazen-26 is already settled and documented with the corpus
  (`/mnt/v/imazen-26/PROVENANCE.md` + per-folder files); the builder just maps
  each stratum onto a policy id in `src/licensing.rs` for the UI badge.
