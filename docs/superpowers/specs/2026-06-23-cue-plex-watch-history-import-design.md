# cue — Plex watch-history import (design)

**Date:** 2026-06-23
**Status:** Approved (brainstorming complete; ready for implementation plan)
**Tracking:** `docs/superpowers/deferred-followups.md` → "Plex watch-history import"
**Related:** master spec `2026-06-21-cue-design.md` (§11, D5.2, D6); catalogue-sync spec
`2026-06-22-cue-catalogue-sync-design.md`; user-data-writes spec
`2026-06-23-cue-user-data-writes-design.md`; IMDb-ratings-import spec
`2026-06-23-cue-imdb-ratings-import-design.md`.

## 1. Goal

Populate `watch_history` with `source='plex'` rows so titles the user has watched
on Plex surface as **watched** in cue, **folded into the existing catalogue sync**
(no separate user action). The catalogue read already derives the per-title
`watched` boolean from `watch_history`, so this is purely a *write* feature — no
read-path, model, or display changes are required beyond a small frontend refresh.

## 2. Why this is small

- `watched` is derived **at read time** in `db/catalogue.rs`
  (`SELECT DISTINCT imdb_id FROM watch_history`), keyed on the title's `imdb_id`.
  Any imported `plex` row makes its title light up — nothing else to wire.
- The `watch_history` schema already exists (migration `0001`) with a
  `source IN ('manual','plex')` column reserved for exactly this.
- `PlexClient` already scans `/library/sections/{key}/all`, parses the `Guid`
  array (incl. `imdb://`), and holds a configured token (`PLEX_URL`/`PLEX_TOKEN`).
- **D5.2 is already honored** by `set_watched`: manual un-watch deletes only
  `source='manual'` rows, so imported `plex` rows are preserved.

## 3. Decisions

| # | Decision | Rationale |
|---|----------|-----------|
| W1 | **Trigger: folded into catalogue sync** (not a separate Settings button). | The Plex section scan already runs during sync; watch state refreshes whenever the catalogue does. No new endpoint. |
| W2 | **Seam: new `fetch_watch_history()` trait method**, default-empty, Plex-only override (approach A); plus a `watch_history_source()` tag method (default `None`, Plex `Some("plex")`) that **gates** the apply. | Keeps `FetchedTitle` a pure source-agnostic catalogue row; isolated and testable via `FakeSource`; extensible to future sources. The tag gate is what prevents a successful *non-watch* source (e.g. MOTN, default no-op) from wiping `plex` rows — see W6. Cost: a 2nd Plex section sweep per sync — negligible for an infrequent background job. |
| W3 | **Keying: `imdb_id` only; skip no-imdb titles.** | Consistent with the app's current stance — no-imdb titles already cannot carry user-data (writes 422) nor surface `watched` (catalogue matches by `imdb_id`). A `plex:<guid>` fallback row would be dead today; deferred with the broader no-imdb gap. |
| W4 | **Watched determination:** movie ⇒ `viewCount > 0`; series ⇒ `viewedLeafCount > 0` (touched / in-progress). | For a discovery app "have I touched this" is more useful than "fully completed." |
| W5 | **Granularity: title-level only** — one `plex` row per watched title; `season`/`episode` left NULL. | Matches the catalogue's single per-title `watched` boolean. Per-episode history (using the `season`/`episode` cols + `/status/sessions/history/all`) is deferred. |
| W6 | **Apply: replace-by-source, scoped to success AND to sources that declare a watch tag.** For each *successful* source whose `watch_history_source()` is `Some(tag)`: in one transaction `DELETE FROM watch_history WHERE source = tag` then insert the new rows. **Skipped if the source failed OR declares no tag.** | Makes `plex` history a true mirror of Plex (un-watching in Plex clears it on next sync) while a Plex outage never wipes prior watched state — mirrors the existing scoped-prune philosophy. The tag gate stops a successful non-watch source (MOTN) from running an empty replace that would wipe `plex` rows. Manual rows untouched (delete is scoped to the tag, never `'manual'`). |
| W7 | **`watched_at`** = Plex `lastViewedAt` (unix epoch) → ISO at the DB layer via `datetime(?, 'unixepoch')`, falling back to `datetime('now')` when absent. | Mirrors the `COALESCE(?, datetime('now'))` pattern already used by `import_ratings`. |

## 4. Components & changes

### 4.1 `sync/mod.rs`
- New struct:
  ```rust
  /// One watched-title record emitted by a source's watch-history scan.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct WatchRecord {
      /// User-data key (currently always an `imdb_id`; see W3).
      pub key: String,
      /// Source's last-viewed unix epoch (seconds); `None` ⇒ DB default `now()`.
      pub watched_at: Option<i64>,
  }
  ```
- Two new trait methods on `CatalogueSource`, both with defaults:
  ```rust
  /// The `watch_history.source` tag this client writes, if any (default: none).
  /// `Some(tag)` opts the source into the watch-history replace; `None` skips it.
  fn watch_history_source(&self) -> Option<&'static str> {
      None
  }

  /// Fetch the user's watch history from this source (default: none).
  ///
  /// # Errors
  /// Returns an error if the upstream request fails or a body cannot be parsed.
  async fn fetch_watch_history(&self) -> anyhow::Result<Vec<WatchRecord>> {
      Ok(Vec::new())
  }
  ```
  `PlexClient` overrides both: `watch_history_source` → `Some("plex")`.
- `run_sync`: after title upserts + reconcile, for **each source that fetched
  successfully AND whose `watch_history_source()` is `Some(tag)`**, call
  `fetch_watch_history()` and apply via
  `db::user_data::replace_watch_history(pool, tag, &records)`. A watch-history
  fetch error is logged and does **not** fail the sync nor wipe prior rows (it
  leaves the existing tagged rows in place; the replace is only applied on a
  successful fetch).
  - Implementation note: the success/failure bookkeeping in `run_sync` is
    currently per-*service*. Watch history is a per-*source* concern, so capture
    the watch-apply step inside the existing per-source `match src.fetch()` arm's
    success branch (or a parallel `ok_sources` list), not the per-service
    reconcile loop.
  - The tag gate is essential: without it, a successful non-watch source
    (MOTN returns the default empty `Vec`) would run `replace_watch_history`
    with zero records and **delete all `plex` rows**.

### 4.2 `sync/plex.rs`
- Extend `Meta` with `#[serde(rename = "viewCount")] view_count: Option<i64>`,
  `#[serde(rename = "lastViewedAt")] last_viewed_at: Option<i64>`,
  `#[serde(rename = "viewedLeafCount")] viewed_leaf_count: Option<i64>`.
- New pure fn:
  ```rust
  /// Parse one `/library/sections/{key}/all` body into watched-title records.
  /// Movies: kept when `viewCount > 0`. Shows: kept when `viewedLeafCount > 0`.
  /// Items without an `imdb://` GUID are skipped (W3).
  pub fn parse_watch_history(json: &str) -> anyhow::Result<Vec<WatchRecord>>
  ```
- `PlexClient` overrides both trait methods: `watch_history_source()` →
  `Some("plex")`; `fetch_watch_history()` lists sections, sweeps each movie/show
  section's `/all`, runs `parse_watch_history`, and concatenates. (Same call
  pattern as `fetch`; a second sweep — see W2.)

### 4.3 `db/user_data.rs`
- New fn:
  ```rust
  /// Replace all watch_history rows of a given source with `records`
  /// (single transaction). Other sources' rows (incl. 'manual') are untouched.
  pub async fn replace_watch_history(
      pool: &SqlitePool,
      source: &str,
      records: &[WatchRecord],
  ) -> anyhow::Result<usize> // returns rows written
  ```
  - `DELETE FROM watch_history WHERE source = ?` then per record:
    `INSERT INTO watch_history (imdb_id, watched_at, source)
     VALUES (?, COALESCE(datetime(?, 'unixepoch'), datetime('now')), ?)`.

### 4.4 Frontend `views/SettingsView.vue`
- In `refresh()`, detect the running→finished transition (it already clears the
  poll there) and call `useCatalogueStore().load()` so newly-synced titles and
  imported watched flags surface without a manual reload — mirroring the IMDb
  card's post-import re-fetch. Guard the store access lazily (as the IMDb handler
  does) so non-Pinia test mounts are unaffected.

## 5. Data flow

```
sync trigger
  └─ run_sync
       ├─ for each source: fetch() titles            (existing)
       ├─ merge + upsert titles + reconcile services (existing)
       └─ for each SUCCESSFUL source with watch_history_source() == Some(tag):
            fetch_watch_history()  →  Vec<WatchRecord>   (Plex: viewCount/viewedLeafCount sweep)
            replace_watch_history(pool, tag, records)    (DELETE tag rows + INSERT; tx)
                                                          (manual rows preserved; skipped on failure
                                                           or when source declares no tag, e.g. MOTN)
catalogue read (existing): DISTINCT imdb_id FROM watch_history ⇒ per-title `watched`
frontend: sync finishes ⇒ catalogue store reloads ⇒ watched flags render
```

## 6. Error handling

- **Plex `fetch_watch_history` error:** logged via `tracing::error!`; the sync
  otherwise succeeds and **prior `plex` rows are left intact** (no destructive
  write on a failed scan). Never propagated as a sync failure.
- **Plex `fetch` (catalogue) failure:** the source is not "successful," so the
  watch-apply step is skipped — prior watched state survives a Plex outage (W6).
- **`replace_watch_history` DB error:** propagated from `run_sync` like other DB
  writes (transaction rolls back; no partial replace).

## 7. Testing

Backend (runtime SQLx; `tempfile::tempdir()` per project convention):
- `parse_watch_history` over a fixture covering: movie watched (`viewCount>0`),
  movie unwatched (`viewCount` 0/absent → dropped), show in-progress
  (`viewedLeafCount>0`), show untouched (dropped), watched item **without an
  imdb GUID** (skipped, W3), `lastViewedAt` carried through.
- `replace_watch_history`: replaces existing `plex` rows, **preserves a `manual`
  row** (D5.2), writes `watched_at` from epoch (and `now()` fallback when None),
  returns the written count.
- Orchestrator: a `FakeSource` with `watch_history_source() == Some("plex")`
  returning watch records proves the apply runs for a **successful** source; a
  failed source's watch records are **not** applied (prior `plex` rows survive).
- A successful source with `watch_history_source() == None` (MOTN-like) does
  **not** run the replace — prior `plex` rows survive (the wipe-guard, W6).

Frontend:
- `SettingsView` re-fetches the catalogue when a sync transitions running→done
  (extend the existing `SettingsView.spec.ts`).

## 8. Live-verify follow-up (post-merge)

Consistent with the MOTN/poster precedent — no live Plex in the build env:
- Confirm the real Plex `/library/sections/{key}/all` field names
  (`viewCount`, `lastViewedAt`, `viewedLeafCount`) against a real server and
  capture a `parse_watch_history` fixture from live data to lock the parser.
- Run one sync against a real Plex library and confirm watched titles light up
  in the grid/detail after the post-sync catalogue refresh, and that
  un-watching in Plex clears the cue flag on the next sync.

## 9. Out of scope / deferred

- Per-episode watch history (`season`/`episode` columns; `/status/sessions/
  history/all`) — title-level only for v1 (W5).
- `plex:<guid>` fallback keys for no-imdb titles — deferred with the broader
  no-imdb user-data gap (W3).
- Surfacing watch-history stats in the Settings UI.
- Single-fetch optimization (sharing one section sweep between `fetch` and
  `fetch_watch_history`) — only if the second sweep ever proves costly (W2).
