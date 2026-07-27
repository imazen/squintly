# Squintly — common dev/deploy ops.

default:
    @just --list

# Local dev: cargo watch + vite dev with proxy.
dev:
    cd web && npm install
    (cd web && npm run dev) &
    cargo run -- --coefficient-http http://localhost:8081 --port 3030

# Build the frontend then the release binary.
build:
    cd web && npm install && npm run build
    cargo build --release --bin squintly

# Test everything.
test:
    cargo test --all-targets

# Strict CI gate.
ci:
    cargo fmt -- --check
    cargo clippy --all-targets -- -D warnings
    cargo test --all-targets
    cd web && npx tsc --noEmit

# Build the Docker image locally. Passes the git commit through as a build arg
# so the image's export manifests carry real provenance (build.rs also derives
# it from git, but the .git dir is dockerignored).
docker-build:
    docker build --build-arg SQUINTLY_BUILD_COMMIT=$(git rev-parse HEAD) -t squintly:local .

## End-to-end Playwright suite (production-shape: built frontend embedded in
## the release binary, mock coefficient on a side channel).
e2e-prep:
    cd web && npm install
    cd web && npm run build
    cargo build --release --bin squintly
    cd web && npx playwright install --with-deps chromium

e2e:
    cd web && npx playwright test

# Run the e2e suite on Galaxy Z Fold 7 cover + inner viewports.
e2e-zfold:
    cd web && npx playwright test --project=zfold7-cover --project=zfold7-inner

## Interactive UX audit / demo-user driver. `audit-serve` boots the mock
## coefficient + production binary on ports 18181/18130 (foreground; Ctrl-C
## to stop); `audit` drives every screen at Z Fold cover/inner + Pixel 7 +
## desktop, writing screenshots + a findings report to
## /mnt/v/output/squintly/ux-audit-<date>/ (view at
## http://localhost:3300/squintly/ux-audit-<date>/REPORT.md).
audit-serve:
    cd web && npm install && npm run build
    cargo build --release --bin squintly
    mkdir -p ~/tmp/squintly-audit
    (cd web && COEFFICIENT_PORT=18181 nohup node --import tsx e2e/mock-coefficient.ts > ~/tmp/squintly-audit/mock.log 2>&1 &)
    sleep 1
    SQUINTLY_DISABLE_TOWER_MIRROR=1 ./target/release/squintly \
        --coefficient-http http://127.0.0.1:18181 \
        --bind 127.0.0.1:18130 --db ~/tmp/squintly-audit/squintly.db

audit:
    cd web && AUDIT_BASE_URL=http://127.0.0.1:18130 npx tsx scripts/ux-audit.ts

# Run the curator-mode e2e suite plus the live R2 fixture.
e2e-curator-live:
    cd web && CURATOR_R2_LIVE=1 npx playwright test e2e/curator.spec.ts e2e/curator-r2-live.spec.ts

# Smoke-run the docker image (binds to localhost:3030; scratch store under ~/tmp).
docker-run:
    mkdir -p ~/tmp/squintly-docker
    docker run --rm -p 3030:3030 -v ~/tmp/squintly-docker:/data \
        -e SQUINTLY_COEFFICIENT_HTTP=http://host.docker.internal:8081 \
        squintly:local

# Railway deployment shortcuts (assumes you've run `railway login` and `railway link`).
railway-init:
    railway init --name squintly
    railway add --plugin postgresql || true   # optional; v0.1 uses SQLite

railway-volume:
    railway volume add --mount-path /data

railway-vars:
    @echo "Set these via 'railway variables --set key=value':"
    @echo "  SQUINTLY_COEFFICIENT_HTTP=https://<your-coefficient-host>"
    @echo "  RUST_LOG=info,squintly=info"
    @echo "  SQUINTLY_DB=/data/squintly.db   # already set in Dockerfile"

# Deploy to Railway. ALWAYS docker-build first: Railway builds from the
# Dockerfile, and a Dockerfile-only break (e.g. a missing COPY) is invisible to
# `cargo test` / `just ci`. That exact gap left main undeployable for two
# months — see DEPLOY.md §13.
#
# The variable is what gives the image real provenance: `.git` is dockerignored
# so build.rs can't shell out to it, and Railway CLI deploys carry no
# RAILWAY_GIT_COMMIT_SHA. Railway passes service variables to the Docker build,
# where the Dockerfile's `ARG SQUINTLY_BUILD_COMMIT` picks it up. Without this
# every export reports build_commit="unknown".
railway-deploy: docker-build
    railway variables --set "SQUINTLY_BUILD_COMMIT=$(git rev-parse HEAD)" --skip-deploys
    railway up --detach

# Publish the built demo corpus to public R2 as a static coefficient HTTP store.
# Versioned prefix: publishing never mutates what a running study is reading —
# roll forward by publishing a new prefix and changing SQUINTLY_COEFFICIENT_HTTP.
publish-corpus version="imazen26-v1":
    python3 scripts/publish_corpus_r2.py \
        --store demo-corpus \
        --bucket codec-corpus \
        --prefix squintly/demo-corpus/{{version}} \
        --public-base https://codec-corpus.r2.imazen.org
    @echo "verify once live:  curl -s https://squintly-production.up.railway.app/api/export/pareto.manifest.json | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"build_commit\"])'"
