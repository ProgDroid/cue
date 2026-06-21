# cue — Design Spec

**Date:** 2026-06-21
**Status:** Approved (brainstorming) — pending implementation plan
**Author:** Fernando Ferreira (with Claude)

## 1. Overview

`cue` is a self-hosted, single-user, dark-themed personal media discovery app. It
presents a filterable poster grid over a personal library spanning **Plex**,
**Disney+ (UK)**, and **Crunchyroll (UK)**, with a persistent natural-language
**ask bar** integrated directly into browsing (not a separate chatbot). Desktop-only,
mouse-navigated.

Two screens plus one integration pattern (all UI behaviour is specified in the design
handoff at `design_handoff_cue/README.md`, which is authoritative for visual fidelity):

1. **Browse** — poster grid + sticky ask bar + filters + thread breadcrumb.
2. **Detail** — title info, mark-as-watched, personal rating, similar-in-library.
3. The **Integrated** ask pattern that reshapes the grid in place.

The UI is **recreated in Vue from the design spec** — the prototype HTML
(`design_handoff_cue/cue.dc.html`) is a reference, not code to copy. `tokens.css` and
`tokens.ts` are carried over verbatim (they are spec, not prototype runtime).

### Stack
- **Backend:** Rust, Actix-web, SQLx, SQLite.
- **Frontend:** Vue 3, TypeScript, `<script setup>`, Pinia, Vite.
- **Infra:** single Docker container (Actix serves the Vue build as static files), `docker-compose`.
- **External services:** Anthropic Claude (ask), OpenAI embeddings (retrieval), Plex API + Movie-of-the-Night / Streaming Availability API (catalogue sync).

## 2. Key decisions (from brainstorming)

| # | Decision | Choice | Rationale |
|---|---|---|---|
| D1 | Catalogue scope | Full Plex library ∪ entire UK Disney+ & Crunchyroll catalogues | The product premise; UI made navigable via filters + retrieval. |
| D2 | Ask engine architecture | **Retrieval → LLM ranks** | Single-user + sparse usage kills prompt-cache economics, so full-catalogue-in-prompt (~$0.27/ask) is uneconomical. Retrieval keeps each ask ~$0.02 and fast, and stays strictly in-catalogue. |
| D3 | Embeddings provider | **OpenAI `text-embedding-3-small`** | User already holds an OpenAI key; 1536-dim, cheap, sufficient for ranking blurbs. |
| D4 | Output constraint | Claude **structured outputs** (`output_config.format`, JSON schema) | Server-validated `{ids, answer, sub}`; better than prefill (400s on Sonnet 4.6) or hand-parsed free-text JSON. |
| D5 | Ask model | **`claude-sonnet-4-6`**, `thinking: disabled`, `effort: low` | User-specified; cost/latency fit. Ranking ~150 candidates is light reasoning → favour latency. |
| D6 | Identity / personal-data key | Surrogate `id` for the frontend; `imdb_id` as canonical key for `user_ratings`/`watch_history` | `imdb_id` keying lets a future IMDb ratings-export import drop straight in. Titles lacking IMDb IDs use a prefixed fallback key (`plex:<guid>`/`tmdb:<id>`). |
| D7 | Security posture | **No auth, bind `127.0.0.1`** | Single-user, self-hosted; all secrets server-side, never sent to Vue. Matches hardcoded "JD" avatar. An auth-middleware slot is reserved so a token/login can be added later without rework. |
| D8 | Multi-service titles | **Set of services** (`title_services`), multi-dot only on genuine overlap | Truthful model; clean membership-based filter; ~zero cost; preserves single-dot prototype fidelity in the common (no-overlap) case. |
| D9 | Build sequencing | **Live sync from day one** | Not time-pressured; want to refine against the real library before use. 28-title seed retained as dev fixture / offline fallback / test data. |

## 3. Architecture & request flow

Single container. Actix-web serves `/api/*` and the built Vue SPA (static + SPA
fallback). SQLite is the only datastore (file on a mounted volume). A background
scheduler runs catalogue sync. External clients are wrapped in small typed modules.

```
Vue SPA ──REST──> Actix ──> SQLite (catalogue, embeddings, user data)
                    │
                    ├── ask_engine ──> OpenAI (embed query) + Claude Sonnet 4.6 (rank)
                    └── scheduler ──> Plex API + Movie-of-the-Night API ──> merge ──> OpenAI (embed new titles)
```

**Boundary principle:** Vue only ever talks to cue's own REST API and never sees an
external key. Only the natural-language ask bar calls Claude; detail-page
"similar in your library" and the grid's similarity are pure genre/vector math
computed server-side.

## 4. Data model (SQLx migrations)

Schema is defined in full from day one (D9), even where UI for a table comes later.

- **`titles`** — `id` INTEGER PK (surrogate, stable across syncs via upsert);
  `imdb_id` TEXT UNIQUE NULL (canonical key); `tmdb_id` TEXT NULL, `plex_guid` TEXT NULL
  (cross-source matching); `title` TEXT; `year` INTEGER; `type` TEXT
  CHECK in ('movie','series'); `imdb_rating` REAL NULL; `length` TEXT ("164 min" /
  "28 eps"); `description` TEXT; `added_at`, `updated_at`.
- **`title_services`** (`title_id`, `service` in ('plex','disney','crunchyroll')) — the
  service set; drives the Service filter.
- **`title_genres`** (`title_id`, `genre`) — Genre filter + shared-genre similarity.
- **`title_cast`** (`title_id`, `person`, `ord`) — detail page.
- **`title_embeddings`** (`title_id`, `vector` BLOB, `model` TEXT, `dims` INTEGER) —
  retrieval; brute-force cosine in Rust over the (few-thousand-row) catalogue.
- **`user_ratings`** (`imdb_id` TEXT, `rating` INTEGER CHECK 1..5, `rated_at`) — keyed on
  the canonical key (D6).
- **`watch_history`** (`id`, `imdb_id` TEXT, `watched_at`, `source` in ('manual','plex'),
  `season` NULL, `episode` NULL).
- **`sync_runs`** (`id`, `source`, `started_at`, `finished_at`, `status`, `item_count`,
  `error` NULL) — scheduled-job observability.

`user_ratings`/`watch_history` use `imdb_id` when present, else a prefixed fallback
(`plex:<guid>` / `tmdb:<id>`) so personal data always has a stable key and real IMDb
imports still match.

## 5. Ask engine (`POST /api/ask`)

Request `{ query: string, baseIds?: number[] }` (`baseIds` = current answer set when
refining within an active thread).

1. Embed `query` via OpenAI `text-embedding-3-small`.
2. Cosine-rank the query vector against catalogue vectors (restricted to `baseIds`
   when present) → top ~150 candidate titles.
3. Send candidates (compact: `id/title/year/genres/type/imdb`) to Claude
   **`claude-sonnet-4-6`** with `output_config.format` constraining output to
   `{ ids: number[], answer: string, sub: string }`; `thinking: {type:"disabled"}`,
   `effort: low`.
4. Validate returned `ids` ⊆ candidate set; drop strays. Return to Vue.

Vue reshapes the grid (300ms fade) and drives the resolve shimmer from the request's
pending state (not a timer). Per-card "More like {title}" hits the same endpoint,
anchored on the title's own embedding + genres instead of a text query, and pushes a
`≈ {title}` thread step.

Filters/sort/search compose on top of the answer set entirely in the Pinia store, per
the README.

## 6. Sync subsystem (scheduler)

- **Scheduler:** `tokio-cron-scheduler`; default daily; cadence configurable via
  `SYNC_CRON`.
- **`plex.rs`:** fetch library sections via Plex API (`PLEX_URL` + `PLEX_TOKEN`);
  extract GUIDs (imdb/tmdb/tvdb), genres, cast.
- **`motn.rs`:** Movie-of-the-Night / Streaming Availability API; UK region; filtered to
  Disney+ & Crunchyroll; paginated.
- **`merge.rs`:** dedup by `imdb_id`; accumulate services (a title on Plex *and* Disney+
  gets both rows in `title_services`).
- After merge: embed new/changed titles via OpenAI, store vectors. Write a `sync_runs`
  row.
- The 28-title prototype seed is retained as a dev fixture / offline fallback / test
  data.

## 7. Backend API surface

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/catalogue` | Full library, hydrated with services/genres/cast/watched/rating. Fetched by Vue on startup. |
| POST | `/api/ask` | Natural-language ask → `{ ids, answer, sub }`. |
| PUT | `/api/titles/:id/rating` | Set/clear personal rating (1–5). |
| PUT | `/api/titles/:id/watched` | Toggle watched. |
| GET | `/api/sync/status` | Latest `sync_runs` info. |

An auth-middleware slot is present (no-op now per D7). Title resources are addressed by
the surrogate `id`; the backend translates to/from `imdb_id` for personal-data tables.

## 8. Frontend (Vue 3 / `<script setup>` / TS / Pinia)

Recreated from the design spec. One Pinia store matching the README shape:
`catalogue: Title[]`; filters (`query`, `service`, `type`, `genre`, `sort`); answer
(`active`, `resultIds`, `line`, `sub`, `thread[]`); user data (`watched`, `ratings`);
nav (`screen`, `selectedId`); derived `visibleTitles` and `similar(id)` (shared-genre,
top 5).

- **Views:** `BrowseView`, `DetailView`.
- **Components:** `AppHeader`, `AskBar`, `ThreadBreadcrumb`, `FilterBar`,
  `AnswerContext`, `PosterGrid`, `PosterCard`, `RatingStars`.
- **Composables:** `useKeyboard` (`/` focuses ask bar, `Esc` clears answer/thread or
  blurs), `usePosterPlaceholder` (oklch gradient + monogram from `tokens.ts`).
- **Design tokens:** `tokens.css` / `tokens.ts` carried over verbatim.
- **API client:** `api/client.ts` — typed REST wrapper.

`Title` (frontend): `{ id, imdbId, title, year, services: ServiceKey[], type, genres,
imdb, len, desc, cast }` plus derived `watched`/`rating`.

## 9. Config, secrets, Docker

- **`Config`** from env: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `MOTN_API_KEY`,
  `PLEX_URL`, `PLEX_TOKEN`, `REGION=uk`, `SYNC_CRON`, `BIND_ADDR=127.0.0.1:8080`,
  `DATABASE_URL`.
- `.env.example` committed; `.env` and `data/` gitignored. No secret ever reaches Vue.
- **Dockerfile (multi-stage):** node stage builds Vue → `dist`; rust stage builds the
  backend; final slim image runs the binary, serves `dist` + API, SQLite on a volume.
- **`docker-compose.yml`:** wires the SQLite volume + `env_file`.
- **`Cargo.toml`:** canonical `[lints.clippy]` table (pedantic + nursery) per project
  Rust standards.

## 10. Proposed directory tree

```
cue/
  Cargo.toml  .env.example  .gitignore  Dockerfile  docker-compose.yml
  migrations/0001_init.sql
  src/
    main.rs  config.rs
    db/mod.rs   models.rs
    routes/{mod,catalogue,ask,user_data,sync_status}.rs
    services/{ask_engine,embeddings,anthropic,similarity}.rs
    sync/{mod,plex,motn,merge}.rs
    static_files.rs
  frontend/
    package.json  vite.config.ts  tsconfig.json  index.html
    src/
      main.ts  App.vue  router/index.ts
      stores/cue.ts   api/client.ts
      design/tokens.ts   assets/tokens.css
      views/{BrowseView,DetailView}.vue
      components/{AppHeader,AskBar,ThreadBreadcrumb,FilterBar,AnswerContext,PosterGrid,PosterCard,RatingStars}.vue
      composables/{useKeyboard,usePosterPlaceholder}.ts
```

## 11. Out of scope (v1) / deferred

- Importing IMDb ratings export / Plex watch history into `user_ratings` /
  `watch_history` (schema is ready; importer UI deferred).
- "Not on your services" discovery row (open-recommendation extras) — clean later add-on
  on the retrieval backbone.
- Auth (token/login) — middleware slot reserved.
- Real poster artwork beyond the generated oklch placeholder (wire `<img>` when sync
  surfaces artwork URLs).

## 12. Open items to confirm during planning

- Exact Movie-of-the-Night / Streaming Availability API endpoints, pagination, and
  rate limits (verify against current docs at plan time).
- Plex API specifics for GUID extraction across imdb/tmdb/tvdb agents.
- Genre vocabulary normalisation across the three sources.
