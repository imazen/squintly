# syntax=docker/dockerfile:1.7
# Three-stage build: TS frontend → Rust binary → minimal runtime.

# 1. Build the Vite TS frontend so it can be embedded into the Rust binary.
FROM node:22-bookworm-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

# 2. Build the Rust binary with the embedded frontend.
FROM rust:slim-bookworm AS rust
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
# Cache deps separately so iterative builds don't recompile the world.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src tests web/dist \
 && echo "fn main() {}" > src/main.rs \
 && echo "" > src/lib.rs \
 && echo "<!doctype html>" > web/dist/index.html \
 && cargo build --release --bin squintly || true \
 && rm -rf src tests
COPY src/ ./src/
COPY migrations/ ./migrations/
COPY tests/ ./tests/
COPY --from=web /web/dist ./web/dist
RUN cargo build --release --bin squintly \
 && strip target/release/squintly

# 3. Minimal runtime. ca-certificates is needed for outbound HTTPS to coefficient.
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tini \
 && rm -rf /var/lib/apt/lists/*
COPY --from=rust /app/target/release/squintly /usr/local/bin/squintly
ENV SQUINTLY_BIND=0.0.0.0:3030 \
    SQUINTLY_DB=/data/squintly.db \
    RUST_LOG=info,squintly=debug
EXPOSE 3030
# NOTE: do not declare VOLUME — Railway rejects it in favour of its own volume
# attachment (`railway volume add --mount-path /data`). For local docker-run
# tests, mount with `-v /tmp/squintly-docker:/data`.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/squintly"]
