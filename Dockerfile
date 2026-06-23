# syntax=docker/dockerfile:1

# --- Stage (enabled in the frontend plan): build the Vue SPA ---
FROM node:22-alpine AS frontend
WORKDIR /app/frontend
COPY frontend/package*.json ./
# package-lock.json is regenerated on Linux (see learnings) so it carries the
# Linux/musl optional deps strict `npm ci` requires.
RUN npm ci
COPY frontend/ ./
RUN npm run build            # outputs /app/frontend/dist

# --- Build the Rust backend ---
FROM rust:1-bookworm AS backend
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY seed ./seed
RUN cargo build --release

# --- Runtime ---
FROM debian:bookworm-slim
WORKDIR /app
# wget is used by the HEALTHCHECK below (bookworm-slim ships neither curl nor wget).
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates wget \
 && rm -rf /var/lib/apt/lists/*
# Run as a non-root user that owns the data dir. A FRESH named volume mounted at
# /data inherits this ownership, so SQLite can write cue.db without running as
# root. (An already-root-owned volume from an older deployment would need a
# one-off `chown` — see README.)
RUN useradd -r -u 10001 cue \
 && mkdir -p /data \
 && chown cue:cue /data
COPY --from=backend /app/target/release/cue /usr/local/bin/cue
COPY --from=frontend /app/frontend/dist ./frontend/dist
ENV BIND_ADDR=0.0.0.0:8080
ENV DATABASE_URL=sqlite:/data/cue.db
ENV STATIC_DIR=/app/frontend/dist
# Log at info by default (the binary uses EnvFilter::from_default_env(), which
# is silent with RUST_LOG unset). sqlx=warn suppresses per-query statement logs.
ENV RUST_LOG=info,sqlx=warn
EXPOSE 8080
USER cue
# Liveness probe so `restart: unless-stopped` can recover a wedged process
# (the app serves /api/health). Hits loopback inside the container; BIND_ADDR is
# 0.0.0.0:8080 here so loopback is reachable.
HEALTHCHECK --interval=30s --timeout=3s --start-period=20s --retries=3 \
  CMD wget -q -O /dev/null http://127.0.0.1:8080/api/health || exit 1
CMD ["cue"]
