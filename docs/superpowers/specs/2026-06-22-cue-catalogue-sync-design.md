# cue — Catalogue Sync Design (Plan 4)

**Date:** 2026-06-22
**Status:** Approved (brainstorming) — pending implementation plan
**Author:** Fernando Ferreira (with Claude)
**Parent spec:** `docs/superpowers/specs/2026-06-21-cue-design.md` (§6 sync subsystem, decisions D1/D6/D8/D9)

## 1. Overview

Plan 4 replaces the 28-title development seed with a live catalogue synced from the
user's real sources: the **Plex** library plus the **UK Disney+ and Crunchyroll**
catalogues (via the Movie-of-the-Night / Streaming Availability API). A background
scheduler refreshes daily; the user can also trigger a sync on demand from a new
settings page reached by clicking the header avatar. New/changed titles are embedded
by reusing the existing Plan 3 backfill routine. This is the first plan to talk to the
real external catalogue services; it is built and tested **offline against recorded
fixtures**, then **verified live on the server** (the same posture Plan 3 used).

Scope boundary: Plan 4 is **read-only ingestion** of catalogue data. Writing personal
data (ratings/watched) is Plan 5; importers and artwork are deferred (see §11).

## 2. Module layout

```
src/sync/
  mod.rs       — orchestrator: run_sync(), CatalogueSource trait, sync bookkeeping, in-process run guard
  plex.rs      — PlexClient (HTTP impl of CatalogueSource): library sections → FetchedTitle
  motn.rs      — MotnClient (HTTP impl of CatalogueSource): UK Disney+/Crunchyroll, paginated
  merge.rs     — pure functions: dedup, service accumulation, genre normalization, reconcile diff
src/routes/sync.rs    — POST /api/sync, GET /api/sync/status (+ registered in routes/mod.rs)
src/db/sync_runs.rs   — write a per-source sync_runs row; latest-per-source + derived overall queries
src/db/catalogue.rs   — extended with upsert + scoped-prune helpers used by reconciliation
```

`merge.rs` holds the well-tested pure core. `plex.rs`/`motn.rs` are thin HTTP +
parsing wrappers behind a trait. `mod.rs` orchestrates and owns the persistence side
effects. The scheduler is wired in `main.rs`.

Per project convention, all new modules are declared in `src/lib.rs` as `pub mod …`.

## 3. External clients — the trait seam

External sources sit behind a trait, mirroring the existing `Embedder` / `AskModel`
seams that keep the suite offline:

```rust
#[async_trait]
pub trait CatalogueSource: Send + Sync {
    /// Client name, for logging/orchestration (e.g. "plex", "motn").
    fn name(&self) -> &'static str;
    /// The service membership(s) this client owns, at title_services granularity
    /// (Plex → [plex]; MOTN → [disney, crunchyroll]). Drives scoped reconciliation
    /// and the per-service sync_runs rows.
    fn services(&self) -> &'static [ServiceKey];
    /// Fetch the client's current catalogue as source-agnostic rows.
    async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>>;
}
```

Note the two granularities: a **client** (Plex, MOTN) is the unit of fetching and
failure, while a **service** (`plex`/`disney`/`crunchyroll`) is the unit of membership
and reconciliation. The MOTN client owns two services; a successful MOTN fetch yields
success rows (with per-service item counts) for both `disney` and `crunchyroll`, and a
MOTN failure marks both `partial`/`error`. Scoped prune (§4.3) operates per **service**
owned by a **successfully-fetched** client.

`FetchedTitle` is the common shape both clients emit, so `merge.rs` never knows which
source a row came from:

```rust
pub struct FetchedTitle {
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub plex_guid: Option<String>,
    pub title: String,
    pub year: Option<i64>,
    pub kind: TitleKind,          // Movie | Series
    pub imdb_rating: Option<f64>,
    pub length: Option<String>,   // "164 min" | "28 eps"
    pub description: Option<String>,
    pub genres: Vec<String>,      // raw; normalized in merge
    pub cast: Vec<String>,        // ordered
    pub services: Vec<ServiceKey>,// plex | disney | crunchyroll
}
```

### 3.1 `PlexClient`
- `GET {PLEX_URL}/library/sections` (header `X-Plex-Token`, `Accept: application/json`)
  to list movie/show sections.
- For each section, `GET /library/sections/{key}/all` to list items; extract `Guid`
  children (`imdb://…`, `tmdb://…`, `tvdb://…`), `Genre`, `Role` (cast, ordered),
  `year`, `type`, `summary`, and duration → `length`.
- All items carry service `plex`.
- The exact Plex JSON field paths are confirmed during live verification (§8); fixtures
  are corrected from a captured real response if they differ.

### 3.2 `MotnClient`
- Base `https://api.movieofthenight.com/v4`, header `X-API-Key` (`MOTN_API_KEY`).
- **Service discovery first:** `GET /v4/countries` → resolve the real catalog ids for
  Disney+ and Crunchyroll in the `gb` region. If a desired service is **not listed for
  `gb`**, log a warning and skip it (do not fail the run). This avoids hardcoding a
  Crunchyroll id that the API may not carry for the UK.
- **Catalogue fetch:** `GET /v4/shows/search/filters?country=gb&catalogs=<resolved ids>`
  with **cursor pagination** — response `{ shows: [...], hasMore, nextCursor }`; pass
  `nextCursor` back as `cursor` until `hasMore` is false.
- Map each show: `imdbId`, `tmdbId`, `title`, `overview`, `releaseYear` (movie) /
  `firstAirYear`/`lastAirYear` (series), `genres`, `cast`, `rating`, `runtime`
  (movie) / `seasonCount`+`episodeCount` (series → "N eps"), `showType` → `kind`. The
  `streamingOptions[gb]` entries determine which of `disney`/`crunchyroll` the title
  belongs to.

## 4. Merge, normalize, reconcile (`merge.rs` + orchestrator)

### 4.1 Dedup & identity (D6)
- Dedup by `imdb_id` when present (canonical key). Titles lacking an IMDb id use a
  prefixed fallback identity (`plex:<guid>` / `tmdb:<id>`), so personal-data keying and
  cross-sync stability are preserved.
- A title returned by multiple sources collapses to one `titles` row with the **union**
  of `title_services` rows (e.g. on Plex *and* Disney+ → both memberships). This is the
  D8 multi-service model.

### 4.2 Genre normalization
Lowercase, trim, de-duplicate per title, then apply a small hand-maintained alias map
in `merge.rs` for obvious collisions across sources (e.g. `sci-fi` → `science fiction`).
Unknown genres pass through normalized (not dropped) — the filter stays clean without a
brittle canonical whitelist.

### 4.3 Reconciliation — upsert + **scoped** prune
A run builds the desired-state catalogue from the sources that **succeeded**:

1. **Upsert** every fetched title (insert new; update mutable fields on existing,
   matched by identity). The surrogate `titles.id` is stable across syncs.
2. **Reconcile service membership** for each **successful** source: remove
   `title_services` rows for that source that the source no longer returns; add new
   ones.
3. **Prune** titles left with **zero** services after reconciliation.

**Prune is scoped to sources that synced successfully.** If MOTN fails but Plex
succeeds, only stale `plex` memberships are reconciled; `disney`/`crunchyroll` rows are
untouched, and the run is recorded `partial`. This prevents a transient outage from
wiping half the library.

`user_ratings` / `watch_history` are keyed on `imdb_id` (D6), never on the surrogate
`id`, so reconciliation never orphans personal data. The 28-title seed is dev/offline
data: it carries no real service membership the live sources return, so the first
successful real sync prunes it naturally.

## 5. Embedding integration (reuse — no new code)

After a successful merge, the orchestrator calls the existing
`cue::services::embeddings::backfill(pool, &embedder)` (Plan 3). It is idempotent and
embeds only titles lacking a current-model vector, so an initial sync embeds the whole
new catalogue once and subsequent syncs embed only genuinely new/changed titles. No new
embedding logic is introduced.

## 6. Scheduler & triggers

- **Scheduler:** `tokio-cron-scheduler`. A single `JobScheduler` is created and started
  in `main.rs`; one `Job::new_async` runs on `SYNC_CRON` (default daily). The scheduler
  handle is held for the process lifetime. In-memory only (no Postgres/Nats feature).
- **Startup-if-empty:** on boot, if the catalogue is empty or contains only the seed,
  spawn one background sync so a fresh deploy populates itself.
- **Manual trigger** `POST /api/sync`: returns `202 {started:true}` immediately and runs
  the sync on a spawned task. An in-process guard (e.g. an `AtomicBool`/`Mutex` in shared
  app state) prevents concurrent runs — a second trigger while running returns
  `409 {running:true}`.

Scheduler-touching tests use `#[tokio::test(flavor = "multi_thread")]` (the crate's
`add()` hangs on a single-threaded runtime).

## 7. Backend API surface

| Method | Path | Returns |
|---|---|---|
| POST | `/api/sync` | `202 {started:true}` or `409 {running:true}` |
| GET | `/api/sync/status` | latest run + per-source breakdown + catalogue stats |

`/api/sync/status` response:

```json
{
  "running": false,
  "lastRun": { "startedAt": "...", "finishedAt": "...", "status": "ok|partial|error", "itemCount": 0, "error": null },
  "sources": [
    { "source": "plex",        "lastRun": "...", "status": "ok",    "itemCount": 0 },
    { "source": "disney",      "lastRun": "...", "status": "ok",    "itemCount": 0 },
    { "source": "crunchyroll", "lastRun": "...", "status": "error", "itemCount": 0 }
  ],
  "catalogue": { "titles": 0, "movies": 0, "series": 0, "embedded": 0 }
}
```

**No schema migration needed:** the existing `sync_runs` table already has a `source`
column. Each run writes **one row per source** (`plex` / `disney` / `crunchyroll`).
`sources` = latest row per source; `lastRun` = derived from the most recent run batch
(latest `started_at`, combined status). `catalogue` stats are simple aggregate counts
(`titles`, `movies`/`series` split, count with a current-model embedding).

The reserved no-op auth-middleware slot (D7) continues to apply to these routes.

## 8. Testing strategy

**Offline by default; live verification on the server** (per user decision, matching
Plan 3).

- **`merge.rs` pure-function tests:** dedup by imdb/fallback identity, multi-source
  service accumulation, genre normalization + alias map, and reconciliation — including
  the **partial-failure case** (one source fails → its memberships are not pruned).
- **Client parsing tests** against recorded JSON fixtures in `tests/fixtures/`: a Plex
  `sections` + `all` response, an MOTN `search/filters` page (with `hasMore`/`nextCursor`)
  and a `/countries` response. Verifies field extraction, pagination loop, and
  service-id resolution / graceful skip when a service is absent for `gb`.
- **Orchestrator end-to-end** with fake `CatalogueSource` impls over a temp SQLite DB
  (the Windows `tempfile::tempdir()` + `sqlite:` URL backslash-fixup pattern from
  CLAUDE.md). Asserts upsert, scoped prune, seed replacement, `sync_runs` rows, and that
  `backfill` is invoked.
- **Scheduler tests** use `#[tokio::test(flavor = "multi_thread")]`.
- **Live verification:** deploy, click **Sync now** on the settings page (or
  `POST /api/sync`), watch `/api/sync/status`, and confirm real titles land, embed, and
  render in Browse. Fixtures are corrected from captured real responses if shapes differ.

All external keys remain server-side `Config` only (never serialized to Vue), and
`BIND_ADDR` keeps its container/host posture (Docker deploy gotcha already resolved).

## 9. Frontend — settings page

- The header avatar (`FF`) in `AppHeader.vue` becomes an interactive control
  (button / `RouterLink`) that navigates to a new route `/settings` (`SettingsView.vue`).
- The page shows:
  - **Sync now** button → `POST /api/sync`; disabled while a run is active.
  - **Sync status** — last run time, status, and any error.
  - **Per-source breakdown** — Plex / Disney+ / Crunchyroll last-run time, status, and
    item count.
  - **Catalogue stats** — total titles, movies vs series, count embedded.
  - While a run is active, poll `/api/sync/status` every few seconds; stop when idle.
- New `api/client.ts` methods `triggerSync()` and `getSyncStatus()` (typed). Styling
  uses the existing design tokens — consistent with the rest of the app, no new UI lib.
- Vitest covers the view's states (idle / running / error / populated stats).

## 10. Config / env

All env vars already exist in `Config`: `MOTN_API_KEY`, `PLEX_URL`, `PLEX_TOKEN`,
`REGION` (default `uk`/`gb`), `SYNC_CRON`. Plan 4 adds a sensible `SYNC_CRON` default
(daily) and documents the new vars in `.env.example` (split in-container secrets vs
local infra, per the existing convention). No secret ever reaches Vue.

## 11. Sequencing & deferred items

Each deferred item is tracked to a concrete home so nothing floats:

| Item | Home |
|---|---|
| Personal-data **write** endpoints (`PUT /rating`, `PUT /watched`) + persistence + frontend wiring | **Plan 5** (already planned — the next plan after this one) |
| **Plex watch-history import** into `watch_history` | **Plan 6 / follow-up** (schema ready; master spec §11) |
| **IMDb ratings-export import** into `user_ratings` | **Plan 6 / follow-up** (schema ready; master spec §11) |
| **Real poster artwork** (wire `<img>` from sync-surfaced art URLs; replaces oklch placeholder) | **Plan 6 / follow-up** (master spec §11) |
| **"Not on your services" discovery row** (open recommendations on the retrieval backbone) | **Plan 6 / follow-up** (master spec §11) |
| **Auth** (token/login) | Deferred; middleware slot reserved (D7) |

Plan 4 itself does **not** implement any of the above. Its terminal deliverable is a
live, self-refreshing catalogue with on-demand sync and an observable settings page.

## 12. Items confirmed during planning

- **MOTN API v4** verified current (Context7, 2026-06-22): `GET /v4/shows/search/filters`,
  `X-API-Key`, `country=gb`, `catalogs=<ids>`, cursor pagination
  (`hasMore`/`nextCursor`/`cursor`); show object fields per §3.2; `GET /v4/countries`
  lists per-country services + addon ids.
- **Crunchyroll-in-`gb` is resolved at runtime** against `/v4/countries`, not assumed.
  If the UK catalogue does not expose Crunchyroll, the client degrades gracefully (logs
  + skips) and Crunchyroll sourcing is revisited as a follow-up.
- **`tokio-cron-scheduler`** usage verified: `JobScheduler::new().await` →
  `Job::new_async(cron, |uuid,l| Box::pin(async {…}))` → `add().await` → `start().await`;
  multi-thread runtime required in tests.
- **Plex** field paths to be confirmed during live verification (offline fixtures built
  from documented shapes, corrected from a captured real response if needed).
