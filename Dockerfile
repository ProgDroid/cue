# syntax=docker/dockerfile:1

# --- Stage (enabled in the frontend plan): build the Vue SPA ---
FROM node:22-alpine AS frontend
WORKDIR /app/frontend
COPY frontend/package*.json ./
# `npm install` (not `npm ci`): package-lock.json is generated on Windows and
# omits Linux/musl optional deps (@emnapi/*, rollup musl bindings), so strict
# `npm ci` fails here. install resolves the right platform deps at build time.
RUN npm install --no-audit --no-fund
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
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=backend /app/target/release/cue /usr/local/bin/cue
COPY --from=frontend /app/frontend/dist ./frontend/dist
ENV BIND_ADDR=0.0.0.0:8080
ENV DATABASE_URL=sqlite:/data/cue.db
ENV STATIC_DIR=/app/frontend/dist
EXPOSE 8080
CMD ["cue"]
