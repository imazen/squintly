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

# Build the Docker image locally.
docker-build:
    docker build -t squintly:local .

## End-to-end Playwright suite (production-shape: built frontend embedded in
## the release binary, mock coefficient on a side channel).
e2e-prep:
    cd web && npm install
    cd web && npm run build
    cargo build --release --bin squintly
    cd web && npx playwright install --with-deps chromium

e2e:
    cd web && npx playwright test

# Smoke-run the docker image (binds to localhost:3030; uses an in-memory store at /tmp/squintly-docker).
docker-run:
    mkdir -p /tmp/squintly-docker
    docker run --rm -p 3030:3030 -v /tmp/squintly-docker:/data \
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

railway-deploy:
    railway up --detach
