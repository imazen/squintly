# Deploy Squintly to Railway

> **Live deployment (2026-05-04)**: https://squintly-production.up.railway.app
> on Railway Hobby plan, ~$7-9/month with 5GB volume. Currently configured
> with `SQUINTLY_COEFFICIENT_HTTP=https://coefficient.example.com` (placeholder).
> Set the real coefficient URL when coefficient itself is deployed.

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
