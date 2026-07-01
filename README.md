# cue

A self-hosted, single-user media-discovery app: it ingests your library (Plex)
and what's on your streaming services (Movie-of-the-Night), then lets you browse
and ask for recommendations in natural language. **Backend:** Rust + Actix-web +
SQLx + SQLite. **Frontend:** Vue 3 + TypeScript + Pinia, served as static files
by the same container that runs the API.

> [!WARNING]
> cue has **no authentication** — it is built for single-user, localhost use
> (`BIND_ADDR` defaults to `127.0.0.1`). Every endpoint, including the write,
> import, and sync endpoints, is unauthenticated. **Do not expose it directly to
> a network or the internet.** Put it behind a reverse proxy that handles auth
> (or a VPN/Tailscale) if you need to reach it from another machine.

## Quickstart (Docker)

cue ships as a single container that serves the Vue build alongside the REST API.

1. Copy the env template and fill in your keys:
   ```sh
   cp .env.example .env
   # edit .env — at minimum set the keys you want (all are optional; see below)
   ```
2. Start it:
   ```sh
   docker compose up -d
   ```
   `docker-compose.yml` builds the image locally by default and publishes the app
   on `127.0.0.1:8080`. The SQLite database lives in the `cue-data` named volume.

To run the **published image** instead of building locally, edit
`docker-compose.yml`: comment out `build: .` and use
`image: ghcr.io/progdroid/cue:latest`.

The container has a `HEALTHCHECK` on `/api/health`, so `docker ps` shows health
and `restart: unless-stopped` recovers a wedged process.

## Configuration

All configuration is via environment variables. For Docker, put them in `.env`
(loaded via `env_file`); for local runs they're read from the environment / `.env`.

### Secrets & runtime config (used by both Docker and local runs)

| Variable | Required | Default | Purpose |
|---|---|---|---|
| `ANTHROPIC_API_KEY` | optional | — | Enables the natural-language **Ask** ranking (Claude). Unset → Ask returns 503. |
| `OPENAI_API_KEY` | optional | — | Enables embeddings (retrieval for Ask + "similar"). Unset → Ask/similar disabled; `refine` still works. |
| `MOTN_API_KEY` | optional | — | Movie-of-the-Night (Streaming Availability) sync. Unset → that source is skipped. |
| `PLEX_URL` | optional | — | Base URL of your Plex server (e.g. `http://10.0.0.5:32400`). |
| `PLEX_TOKEN` | optional | — | Plex auth token. `PLEX_URL` + `PLEX_TOKEN` together enable the Plex source. |
| `PLEX_WEB_URL` | optional | `PLEX_URL` | Browser-facing Plex base URL for the **"Watch on Plex"** detail links. Set only when the browser can't reach `PLEX_URL` directly (e.g. `PLEX_URL` is a Docker-internal host); otherwise it defaults to `PLEX_URL`. |
| `REGION` | optional | `gb` | ISO-3166 alpha-2 country for Movie-of-the-Night (e.g. `gb`, `us`). |
| `SYNC_CRON` | optional | `0 0 3 * * *` | **6-field** cron (`sec min hour dom mon dow`) for the scheduled sync. Validated at startup. |
| `ASK_MODEL` | optional | `claude-sonnet-5` | Claude model to use for ranking. |

All keys are **server-side only** and are never serialized to the client. Missing
keys degrade gracefully (the relevant feature is disabled with a log line) rather
than crashing. A malformed `SYNC_CRON` or `BIND_ADDR` fails fast at startup with
an error naming the offending value.

### Local-run only (ignored under Docker)

`docker-compose.yml` pins these via `environment:` (which overrides `env_file`),
so editing them in `.env` has no effect in a container. The published image bakes
in production values.

| Variable | Default (local) | Notes |
|---|---|---|
| `BIND_ADDR` | `127.0.0.1:8080` | Must be `host:port`. In Docker this is `0.0.0.0:8080` (compose handles it) — a `127.0.0.1` bind is unreachable through Docker's port mapping. |
| `DATABASE_URL` | `sqlite:./data/cue.db` | SQLite URL. Docker uses `sqlite:/data/cue.db` (the `cue-data` volume). |
| `STATIC_DIR` | `frontend/dist` | Directory of the built Vue SPA. |
| `RUST_LOG` | `info,sqlx=warn` | `tracing` env-filter. |

> [!NOTE]
> The container runs as a non-root user (`cue`, uid 10001) that owns `/data`. A
> **fresh** named volume inherits that ownership automatically. If you are
> upgrading from an older image that ran as root, an existing root-owned volume
> may need a one-off `docker run --rm -v cue-data:/data busybox chown -R 10001 /data`.

## Local development

Backend (Rust):
```sh
cp .env.example .env       # optional: fill in keys
cargo run                  # serves the API on BIND_ADDR (default 127.0.0.1:8080)
cargo test                 # full suite
cargo clippy --all-targets -- -D warnings
```

Frontend (Vue), in `frontend/`:
```sh
npm install
npm run dev                # Vite dev server with API proxy
npm test                   # vitest
npm run build              # type-check + production build into frontend/dist
```

## How it works (brief)

- The catalogue is assembled by syncing from configured sources (Plex for owned
  media; Movie-of-the-Night for what's on your streaming services in `REGION`)
  into SQLite. Sync runs on a schedule (`SYNC_CRON`) and once on first startup.
- **Ask** embeds your query (OpenAI), retrieves the closest catalogue titles by
  cosine similarity, and has Claude rank *only those* — it can never recommend a
  title you don't have.
- See `docs/superpowers/specs/` for the full design (decisions D1–D9) and
  `CLAUDE.md` for backend conventions.

## License

Personal project; no license granted for redistribution.
