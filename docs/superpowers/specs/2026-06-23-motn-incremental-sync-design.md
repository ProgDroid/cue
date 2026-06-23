# cue — Incremental MOTN Sync Design

**Date:** 2026-06-23
**Status:** Approved (brainstorming) — pending implementation plan
**Author:** Fernando Ferreira (with Claude)
**Parent spec:** `docs/superpowers/specs/2026-06-22-cue-catalogue-sync-design.md` (Plan 4 sync subsystem)

## 1. Problem

The Movie-of-the-Night (MOTN) / Streaming Availability API plan allows only **500
requests/month**. The current `MotnClient::fetch()` is a **full snapshot**: it
paginates `/shows/search/filters` across the entire UK Disney+ + Crunchyroll
catalogues (~5000+ items) on every run. A single daily sync plus a few test runs
exhausts the monthly budget. We need to fetch only **what changed** between syncs,
using MOTN's `GET /changes` endpoint, while keeping the catalogue accurate.

## 2. Constraint: the full-snapshot contract

This is not a localized edit to `fetch()`. The orchestrator (`src/sync/mod.rs::run_sync`)
is built around a full-snapshot model:

1. each `CatalogueSource::fetch()` returns its **complete** current catalogue,
2. `store::reconcile_service` sets membership to **exactly** that set,
3. `store::prune_orphans_scoped` deletes any title with no remaining membership.

A delta endpoint breaks steps 2–3 directly: if MOTN returned only changed titles,
reconcile/prune would wrongly drop every unchanged title. **The design must preserve
the full-snapshot contract** so the orchestrator, `merge`, `store`, and the Plex
source stay unchanged.

## 3. The `/changes` endpoint (from MOTN docs)

- `GET /v4/changes`, header `X-API-Key`.
- Required params: `country`, `change_type` — one of `new` / `removed` / `updated` /
  `expiring` / `upcoming` (**only one change_type per request**).
- Optional: `item_type` (`show` / `season` / `episode`), `show_type`, `catalogs`,
  `from` / `to` (Unix seconds), `cursor`.
- **`from` is limited to the last 31 days** for past changes (`new` / `removed` /
  `updated`). It **cannot bootstrap an empty catalogue**, and any gap > 31 days
  between syncs would miss changes.
- 25 changes/page; paginated via `cursor` + `hasMore` + `nextCursor`.
- Response carries both a `changes` list and an embedded `shows` list (full show
  objects), matched by `showId`. So one call yields change + show metadata — **no
  extra `GET /shows/{id}` calls needed**.
- Change object fields: `changeType`, `itemType`, `showId`, `showType`, `season`,
  `episode`, `service`.

### Why `new + removed` only (not `updated`)

`updated` fires at the **streaming-option level** (the docs' own example is "updated
Max US Episodes") for a title **already in the catalogue**; it tracks a streaming
option's link/type, not show metadata (rating/description). cue stores title
*membership* + basic metadata, not streaming links, so `updated` adds request cost
without changing what we keep accurate. `new` = a title appears on
Disney/Crunchyroll; `removed` = it leaves. With `item_type=show`, per-season/episode
churn is ignored. Stale metadata is refreshed whenever a recovery re-seed runs (§6).

## 4. Key property

Because the `/changes` window is 31 days and cue syncs **daily**, deltas alone keep an
incrementally-maintained local cache **perfectly accurate in steady state** — no
periodic full re-fetch is needed for correctness. A full fetch is required only:

- **once** to seed an empty cache (fresh install), and
- to **recover** if the app was down longer than the window (last successful sync
  > ~25 days ago, safely under the 31-day limit).

Steady-state cost drops from ~50 requests/day to a handful (1 `/countries` resolve +
1–2 pages each of `new` and `removed`).

## 5. New state (migration `0003`)

A self-contained cache owned entirely by the MOTN source:

```sql
CREATE TABLE motn_catalog_cache (
    show_id    TEXT PRIMARY KEY,   -- MOTN internal show id (stable correlation key)
    payload    TEXT NOT NULL,      -- JSON: cached title fields + attributed services
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

- `payload` holds everything needed to re-emit a `FetchedTitle` (imdb_id, tmdb_id,
  title, year, kind, imdb_rating, length, description, genres, cast, attributed
  services).
- **No new timestamp table.** "Last successful sync" is derived from the existing
  `sync_runs` table: `MAX(finished_at) WHERE source IN ('disney','crunchyroll') AND
  status = 'ok'`. This drives both the `from` parameter and the 25-day recovery check.

## 6. `MotnClient::fetch()` — decide then act

`fetch()` keeps its signature (`async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>>`)
and still returns MOTN's **complete** current catalogue. Internally:

1. **Resolve catalogs** via the existing `/countries` call + `resolve_services` (1 cheap
   request; reused unchanged).
2. **Decide mode** (querying cache count + last-ok timestamp via a small DB handle —
   see §8):
   - cache empty → **full seed**
   - no prior ok run, or last-ok > 25 days ago → **full re-seed** (recovery)
   - otherwise → **delta**
3. **Full seed / re-seed:** run the existing `/shows/search/filters` pagination; write
   every parsed show into `motn_catalog_cache` keyed by `showId` (replacing prior
   cache contents on a re-seed).
4. **Delta:** for `change_type` in (`new`, `removed`): `GET /changes` with `country`,
   `catalogs`, `item_type=show`, `from` = last-ok minus a small overlap buffer;
   paginate `cursor`/`hasMore`. Apply:
   - `new` → upsert cache row built from the embedded `shows` detail (reusing the
     current `Show` → `FetchedTitle` attribution),
   - `removed` → delete cache row by `showId` (needs only the change object's `showId`;
     works even if the show isn't embedded).
5. **Return the full reconstructed snapshot:** read all `motn_catalog_cache` rows →
   `Vec<FetchedTitle>`.

The orchestrator then reconciles + prunes exactly as today.

## 7. Parsing changes (within `motn.rs`)

- The `Show` struct must capture MOTN's `id` (showId) — it currently does not — since
  the cache is keyed on it.
- The `/changes` response is parsed with a small new struct (`changes` list +
  embedded `shows`), reusing the existing `Show` → `FetchedTitle` attribution logic
  (`parse_page`'s per-country `streamingOptions` service attribution). Factor that
  attribution into a shared helper so seed and delta paths share one implementation.
- The parser fails safe: a change whose show detail is missing required fields (e.g.
  no `imdbId`) for a `new` change is skipped with a `tracing::warn!` rather than
  aborting the whole sync.

## 8. Where the DB handle comes from

`MotnClient` currently holds only an HTTP client + api_key + country. To read/write the
cache and read last-ok, it needs a `SqlitePool`. Options (decide in the plan):

- pass the pool into `MotnClient::new(...)` when constructed in `main.rs`, or
- add cache access through a thin `src/db/motn_cache.rs` module that `fetch()` calls.

Recommended: a `src/db/motn_cache.rs` module (`count`, `replace_all`, `upsert`,
`delete`, `load_all`) plus a `last_ok` query helper (in `sync_runs.rs`), with the pool
held by `MotnClient`. This keeps SQL out of `motn.rs` and matches the existing
`db/`-module layering.

## 9. Idempotency & failure behavior

- Applying `new`/`removed` by `showId` is idempotent (upsert / delete-by-key), so the
  `from` overlap buffer cannot cause double counting if a change appears in two
  consecutive windows.
- If `/changes` (or `/countries`) fails, `fetch()` returns `Err` exactly as today; the
  orchestrator records the source as failed and its scoped-prune leaves existing MOTN
  titles untouched (no catalogue wipe on a transient outage).
- A full re-seed replaces the entire cache atomically (within a transaction) so a crash
  mid-seed cannot leave a half-populated cache that would then be treated as the truth.

## 10. Unchanged components

`src/sync/mod.rs` (orchestrator, `run_sync`, `CatalogueSource`, `FetchedTitle`),
`src/sync/merge.rs`, `src/sync/store.rs`, `src/sync/plex.rs`, and all sync routes
remain unchanged. All new logic is contained in `src/sync/motn.rs`, the new
`migrations/0003_*.sql`, and the new `src/db/motn_cache.rs` (+ one `sync_runs` query).

## 11. Testing

- Existing `parse_page` tests stay green.
- New unit tests, all drivable with fixture JSON (no live API):
  - mode selection (empty cache → seed; stale last-ok → re-seed; recent → delta),
  - delta apply: `new` inserts/updates a cache row; `removed` deletes one,
  - snapshot reconstruction: cache rows → expected `Vec<FetchedTitle>`,
  - parse of a `/changes` fixture (changes + embedded shows) with service attribution,
  - fail-safe skip when a `new` change's show detail lacks required fields.
- Test DBs follow project convention: `tempfile::tempdir()` + `sqlite:` URL backslash
  fixup, `_dir` guard bound.
- **Live verification** on the server (one delta run + confirm request count drop)
  after merge, consistent with Plan 4's offline-then-live posture.

## 12. Assumption to verify in implementation

That the embedded `shows` objects in a `/changes` response carry `id`, `imdbId`, and
`streamingOptions` like `/shows/search/filters` results do — the docs state they are
full show objects. The fail-safe parser (§7) guards against absence. This is verified
against a real fixture captured during the first live delta run, **not** by burning
live requests during development.

## 13. Out of scope

- `updated` / `expiring` / `upcoming` change types.
- Changing Plex's full-snapshot path or the orchestrator contract.
- Surfacing cache/seed state in the `/api/sync/status` UI (could be a later follow-up).
