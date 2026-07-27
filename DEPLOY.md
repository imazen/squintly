# Deploy Squintly to Railway

> **Live deployment**: https://squintly-production.up.railway.app on Railway
> Hobby plan, ~$7-9/month with 5GB volume.
>
> **Two known limitations of the live instance, both real:**
> 1. `SQUINTLY_COEFFICIENT_HTTP` is still the `coefficient.example.com`
>    placeholder, so the manifest is empty and **the rating flow serves no
>    trials** (`/api/trial/next` → 409). The welcome / calibration / suggest
>    screens work; the curator works once a manifest is POSTed (see §14).
> 2. Deploys were **silently broken from 2026-05-28 to 2026-07-27** — the
>    Dockerfile didn't `COPY build.rs`, so `env!("SQUINTLY_BUILD_COMMIT")`
>    failed to compile and every `railway up` errored while the May 7 build
>    kept serving. Fixed 2026-07-27; see §13 for the guard.

Single Rust service + persistent volume for the SQLite DB. Modeled on
[interleaved's deployment flow](../interleaved/DEPLOY.md). Coefficient should be
reachable from Railway for the app to serve trials — either run it on Railway too (and use the private
network) or expose it publicly.

## Prerequisites

- [Railway CLI](https://docs.railway.com/guides/cli): `npm i -g @railway/cli`
- `railway login` (one-time)
- A coefficient instance reachable over HTTP

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
# URL of a reachable coefficient viewer.
railway variables --set "SQUINTLY_COEFFICIENT_HTTP=https://coefficient.example.com"
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
| Healthcheck fails after deploy | `SQUINTLY_COEFFICIENT_HTTP` not set, OR DB volume issue | The binary boots and serves `/api/stats` even when coefficient is unreachable (logs the failure but doesn't crash, see `src/main.rs:90-105`). Most likely the env var is unset entirely — set it to any URL (a fake one is fine until coefficient is deployed): `railway variables --set "SQUINTLY_COEFFICIENT_HTTP=https://coefficient.example.com"`. |
| 500 on `/api/trial/next` | Empty manifest | Coefficient has no sources — check coefficient itself. |
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
