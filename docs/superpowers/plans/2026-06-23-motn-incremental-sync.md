# Incremental MOTN Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace MOTN's full-catalogue fetch with a showId-keyed local cache updated from the `/changes` endpoint, slashing monthly API usage while keeping the catalogue accurate.

**Architecture:** `MotnClient::fetch()` keeps returning MOTN's complete current catalogue (so the orchestrator's full-snapshot reconcile/prune is untouched), but reconstructs that snapshot from a local `motn_catalog_cache` table. On each run it decides between a **full seed** (empty cache or >25-day gap) and a **delta** (`GET /changes?change_type=new|removed`), applies the result to the cache, and returns all cache rows.

**Tech Stack:** Rust, Actix-web, SQLx (runtime queries only), SQLite, `serde`, `reqwest`, `async-trait`.

**Spec:** `docs/superpowers/specs/2026-06-23-motn-incremental-sync-design.md`

## Global Constraints

- **Crate is lib + bin.** New modules declared in `src/lib.rs` / `src/db/mod.rs` as `pub mod …`; binary consumes via `cue::` path.
- **SQLx runtime queries only** — `sqlx::query` / `query_as` / `query_scalar`. No `query!` macros, no `DATABASE_URL` at build, no offline cache.
- **Test databases:** `tempfile::tempdir()` (NOT `NamedTempFile`); URL = `format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"))`; keep the `TempDir` guard bound as `_dir`.
- **Clippy:** pedantic + nursery via the `[lints.clippy]` table in `Cargo.toml`. Per-item `#[allow(...)]` with a one-line reason only. Gate: `cargo clippy --all-targets -- -D warnings`.
- **Security boundary:** the MOTN api key stays server-side `Config` only; never serialized to the client.
- **Commits:** via the Bash tool (PowerShell prepends a BOM to commit subjects). End commit messages with `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- **`fetch()` is not unit-tested** (it does live HTTP) — same posture as today. All new logic lives in pure/DB-only functions that ARE tested; `fetch()` is thin glue gated by build + clippy + existing tests + a live verify.

---

### Task 1: `motn_catalog_cache` table + cache CRUD module

**Files:**
- Create: `migrations/0003_motn_catalog_cache.sql`
- Create: `src/db/motn_cache.rs`
- Modify: `src/db/mod.rs:1-5` (add `pub mod motn_cache;`)

**Interfaces:**
- Consumes: `crate::sync::FetchedTitle`, `crate::models::{Service, TitleKind}`, `crate::db::init_pool` (tests).
- Produces:
  - `pub struct CachedTitle` (serde) with `impl From<&FetchedTitle> for CachedTitle` and `pub fn into_fetched(self) -> FetchedTitle`.
  - `pub async fn count(pool: &SqlitePool) -> anyhow::Result<i64>`
  - `pub async fn load_all(pool: &SqlitePool) -> anyhow::Result<Vec<FetchedTitle>>`
  - `pub async fn replace_all(pool: &SqlitePool, entries: &[(String, CachedTitle)]) -> anyhow::Result<()>`
  - `pub async fn upsert(pool: &SqlitePool, show_id: &str, t: &CachedTitle) -> anyhow::Result<()>`
  - `pub async fn delete(pool: &SqlitePool, show_id: &str) -> anyhow::Result<()>`

- [ ] **Step 1: Create the migration**

`migrations/0003_motn_catalog_cache.sql`:

```sql
-- MOTN keeps its own showId-keyed snapshot here so it can rebuild its full
-- catalogue from /changes deltas instead of re-fetching everything each sync.
CREATE TABLE motn_catalog_cache (
    show_id    TEXT PRIMARY KEY,   -- MOTN internal show id (stable correlation key)
    payload    TEXT NOT NULL,      -- JSON: CachedTitle (title fields + attributed services)
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

- [ ] **Step 2: Declare the module**

In `src/db/mod.rs`, add to the `pub mod` block (keep alphabetical with the existing list):

```rust
pub mod catalogue;
pub mod embeddings;
pub mod motn_cache;
pub mod seed;
pub mod sync_runs;
pub mod user_data;
```

- [ ] **Step 3: Write the failing tests**

Create `src/db/motn_cache.rs` with ONLY the test module first so it fails to compile (drives the impl):

```rust
//! MOTN's self-contained catalogue cache (showId-keyed), used to reconstruct a
//! full snapshot from `/changes` deltas.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::models::{Service, TitleKind};
use crate::sync::FetchedTitle;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_pool;

    async fn pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let url = format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        (dir, pool)
    }

    fn sample() -> FetchedTitle {
        FetchedTitle {
            imdb_id: Some("tt1".into()),
            tmdb_id: Some("movie/9".into()),
            plex_guid: None,
            title: "Sample".into(),
            year: Some(2021),
            kind: TitleKind::Movie,
            imdb_rating: Some(7.5),
            length: Some("90 min".into()),
            description: Some("desc".into()),
            genres: vec!["drama".into()],
            cast: vec!["A".into(), "B".into()],
            services: vec![Service::Disney],
        }
    }

    #[tokio::test]
    async fn roundtrips_a_title_through_the_cache() {
        let (_dir, pool) = pool().await;
        let ct = CachedTitle::from(&sample());
        upsert(&pool, "100", &ct).await.unwrap();
        assert_eq!(count(&pool).await.unwrap(), 1);
        let all = load_all(&pool).await.unwrap();
        assert_eq!(all, vec![sample()]);
    }

    #[tokio::test]
    async fn upsert_replaces_existing_show_id() {
        let (_dir, pool) = pool().await;
        upsert(&pool, "100", &CachedTitle::from(&sample())).await.unwrap();
        let mut other = sample();
        other.title = "Renamed".into();
        upsert(&pool, "100", &CachedTitle::from(&other)).await.unwrap();
        assert_eq!(count(&pool).await.unwrap(), 1);
        assert_eq!(load_all(&pool).await.unwrap()[0].title, "Renamed");
    }

    #[tokio::test]
    async fn delete_removes_by_show_id() {
        let (_dir, pool) = pool().await;
        upsert(&pool, "100", &CachedTitle::from(&sample())).await.unwrap();
        delete(&pool, "100").await.unwrap();
        assert_eq!(count(&pool).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn replace_all_wipes_then_inserts() {
        let (_dir, pool) = pool().await;
        upsert(&pool, "stale", &CachedTitle::from(&sample())).await.unwrap();
        let entries = vec![
            ("a".to_string(), CachedTitle::from(&sample())),
            ("b".to_string(), CachedTitle::from(&sample())),
        ];
        replace_all(&pool, &entries).await.unwrap();
        assert_eq!(count(&pool).await.unwrap(), 2); // "stale" gone
    }
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test --lib db::motn_cache`
Expected: FAIL — compile error (`CachedTitle`, `count`, etc. not found).

- [ ] **Step 5: Implement the module**

Insert above the `#[cfg(test)]` block in `src/db/motn_cache.rs`:

```rust
/// Serialized form of a MOTN title stored in `motn_catalog_cache.payload`.
///
/// Mirrors `FetchedTitle` but stores `kind`/`services` as strings so the JSON is
/// independent of the enum reprs. `plex_guid` is omitted — MOTN never sets it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedTitle {
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub title: String,
    pub year: Option<i64>,
    pub kind: String,
    pub imdb_rating: Option<f64>,
    pub length: Option<String>,
    pub description: Option<String>,
    pub genres: Vec<String>,
    pub cast: Vec<String>,
    pub services: Vec<String>,
}

impl From<&FetchedTitle> for CachedTitle {
    fn from(t: &FetchedTitle) -> Self {
        Self {
            imdb_id: t.imdb_id.clone(),
            tmdb_id: t.tmdb_id.clone(),
            title: t.title.clone(),
            year: t.year,
            kind: t.kind.as_str().to_string(),
            imdb_rating: t.imdb_rating,
            length: t.length.clone(),
            description: t.description.clone(),
            genres: t.genres.clone(),
            cast: t.cast.clone(),
            services: t.services.iter().map(|s| s.as_str().to_string()).collect(),
        }
    }
}

impl CachedTitle {
    /// Rebuild a `FetchedTitle`. Unknown service/kind strings are dropped/defaulted
    /// defensively; in practice they always parse since we wrote them via `as_str`.
    #[must_use]
    pub fn into_fetched(self) -> FetchedTitle {
        FetchedTitle {
            imdb_id: self.imdb_id,
            tmdb_id: self.tmdb_id,
            plex_guid: None,
            title: self.title,
            year: self.year,
            kind: TitleKind::parse(&self.kind).unwrap_or(TitleKind::Movie),
            imdb_rating: self.imdb_rating,
            length: self.length,
            description: self.description,
            genres: self.genres,
            cast: self.cast,
            services: self.services.iter().filter_map(|s| Service::parse(s)).collect(),
        }
    }
}

/// Number of rows currently cached.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn count(pool: &SqlitePool) -> anyhow::Result<i64> {
    let n = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM motn_catalog_cache")
        .fetch_one(pool)
        .await?;
    Ok(n)
}

/// Insert or replace one cached title by `show_id`.
///
/// # Errors
/// Returns an error if serialization or the write fails.
pub async fn upsert(pool: &SqlitePool, show_id: &str, t: &CachedTitle) -> anyhow::Result<()> {
    let payload = serde_json::to_string(t)?;
    sqlx::query(
        "INSERT INTO motn_catalog_cache (show_id, payload, updated_at)
         VALUES (?, ?, datetime('now'))
         ON CONFLICT(show_id) DO UPDATE SET payload = excluded.payload,
                                            updated_at = excluded.updated_at",
    )
    .bind(show_id)
    .bind(payload)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete one cached title by `show_id` (no-op if absent).
///
/// # Errors
/// Returns an error if the write fails.
pub async fn delete(pool: &SqlitePool, show_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM motn_catalog_cache WHERE show_id = ?")
        .bind(show_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Atomically replace the entire cache with `entries` (used by full seed/re-seed).
///
/// # Errors
/// Returns an error if serialization or any write fails; the transaction rolls back.
pub async fn replace_all(pool: &SqlitePool, entries: &[(String, CachedTitle)]) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM motn_catalog_cache").execute(&mut *tx).await?;
    for (show_id, t) in entries {
        let payload = serde_json::to_string(t)?;
        sqlx::query(
            "INSERT INTO motn_catalog_cache (show_id, payload) VALUES (?, ?)",
        )
        .bind(show_id)
        .bind(payload)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Load all cached titles as a reconstructed full snapshot.
///
/// # Errors
/// Returns an error if the query or any payload deserialization fails.
pub async fn load_all(pool: &SqlitePool) -> anyhow::Result<Vec<FetchedTitle>> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT payload FROM motn_catalog_cache ORDER BY show_id",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for payload in rows {
        let ct: CachedTitle = serde_json::from_str(&payload)?;
        out.push(ct.into_fetched());
    }
    Ok(out)
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --lib db::motn_cache`
Expected: PASS (4 tests).

- [ ] **Step 7: Clippy gate**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add migrations/0003_motn_catalog_cache.sql src/db/motn_cache.rs src/db/mod.rs
git commit -m "feat(sync): add showId-keyed motn_catalog_cache table + CRUD

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `sync_runs` last-ok helpers

**Files:**
- Modify: `src/db/sync_runs.rs` (append two functions + tests)

**Interfaces:**
- Consumes: `sqlx::SqlitePool`, `crate::db::init_pool` + `crate::db::sync_runs::record` (tests).
- Produces:
  - `pub async fn last_ok_unix(pool: &SqlitePool) -> anyhow::Result<Option<i64>>` — Unix seconds of the newest `ok` Disney/Crunchyroll run, or `None` if there is none.
  - `pub async fn motn_recent_ok(pool: &SqlitePool) -> anyhow::Result<bool>` — true iff such a run finished within the last 25 days.

- [ ] **Step 1: Write the failing tests**

Append to the existing `#[cfg(test)] mod tests` in `src/db/sync_runs.rs` (or add one if absent — match the file's current test setup; it already builds pools via the project convention):

```rust
#[tokio::test]
async fn last_ok_and_recent_reflect_recorded_runs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let url = format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"));
    let pool = crate::db::init_pool(&url).await.unwrap();

    // No runs yet.
    assert_eq!(last_ok_unix(&pool).await.unwrap(), None);
    assert!(!motn_recent_ok(&pool).await.unwrap());

    // A failed run does not count.
    record(&pool, "disney", "error", 0, Some("boom")).await.unwrap();
    assert_eq!(last_ok_unix(&pool).await.unwrap(), None);
    assert!(!motn_recent_ok(&pool).await.unwrap());

    // A successful run counts and is recent.
    record(&pool, "crunchyroll", "ok", 10, None).await.unwrap();
    assert!(last_ok_unix(&pool).await.unwrap().is_some());
    assert!(motn_recent_ok(&pool).await.unwrap());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib db::sync_runs::tests::last_ok_and_recent`
Expected: FAIL — `last_ok_unix` / `motn_recent_ok` not found.

- [ ] **Step 3: Implement the helpers**

Append to `src/db/sync_runs.rs` (above the test module):

```rust
/// Unix-seconds timestamp of the newest successful MOTN-owned run
/// (`disney`/`crunchyroll`), or `None` if none has succeeded. Drives the
/// `/changes` `from` parameter.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn last_ok_unix(pool: &SqlitePool) -> anyhow::Result<Option<i64>> {
    let ts = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT CAST(strftime('%s', MAX(finished_at)) AS INTEGER)
         FROM sync_runs
         WHERE source IN ('disney', 'crunchyroll') AND status = 'ok'",
    )
    .fetch_one(pool)
    .await?;
    Ok(ts)
}

/// Whether a successful MOTN-owned run finished within the last 25 days — under
/// MOTN's 31-day `/changes` window, so a delta sync would not miss changes.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn motn_recent_ok(pool: &SqlitePool) -> anyhow::Result<bool> {
    let recent = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(
            SELECT 1 FROM sync_runs
            WHERE source IN ('disney', 'crunchyroll')
              AND status = 'ok'
              AND finished_at >= datetime('now', '-25 days')
        )",
    )
    .fetch_one(pool)
    .await?;
    Ok(recent != 0)
}
```

> Note: `MAX(finished_at)` over an empty/all-error set yields SQL `NULL`, which `strftime` passes through as `NULL` → `Option::None`. The outer `fetch_one` returns exactly one row, so use `query_scalar::<_, Option<i64>>`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib db::sync_runs`
Expected: PASS.

- [ ] **Step 5: Clippy gate**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/db/sync_runs.rs
git commit -m "feat(sync): last_ok_unix + motn_recent_ok queries for delta decisions

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Parse refactor + `/changes` parser in `motn.rs`

**Files:**
- Modify: `src/sync/motn.rs` (add `id` to `Show`; extract `show_to_fetched`; add `parse_page_entries`; add changes structs + `parse_changes`; tests)

**Interfaces:**
- Consumes: `crate::db::motn_cache::CachedTitle`, existing `Service`, `wanted_id`, `FetchedTitle`, `TitleKind`.
- Produces:
  - `fn show_to_fetched(s: Show, country: &str, services: &[Service]) -> FetchedTitle` (private; the current per-show mapping logic).
  - `pub fn parse_page_entries(json: &str, country: &str, services: &[Service]) -> anyhow::Result<(Vec<(String, CachedTitle)>, Option<String>)>` — seed path (showId + CachedTitle).
  - `pub struct ParsedChanges { pub additions: Vec<(String, CachedTitle)>, pub removals: Vec<String> }` with `Default` + `fn merge(&mut self, other: ParsedChanges)`.
  - `pub fn parse_changes(json: &str, country: &str, services: &[Service]) -> anyhow::Result<(ParsedChanges, Option<String>)>`.
- `parse_page` keeps its exact existing signature/behavior (now expressed via `parse_page_entries`).

- [ ] **Step 1: Add `id` to `Show` and the changes structs**

In `src/sync/motn.rs`, add `id` to the `Show` struct (defaulted, so the `/changes` map form — where the key is the id — still parses):

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Show {
    #[serde(default)]
    id: String,
    imdb_id: Option<String>,
    // ...unchanged fields...
}
```

Add near the other deserialize structs:

```rust
/// One `/changes` response page. `shows` is a map keyed by showId.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangesPage {
    #[serde(default)]
    changes: Vec<ChangeEntry>,
    #[serde(default)]
    shows: HashMap<String, Show>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangeEntry {
    change_type: String,
    #[serde(default)]
    item_type: Option<String>,
    show_id: String,
}
```

- [ ] **Step 2: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `src/sync/motn.rs`:

```rust
#[test]
fn parse_page_entries_pairs_show_id_with_title() {
    let json = r#"{
        "shows": [
            {"id":"100","imdbId":"tt1","title":"A","showType":"movie","releaseYear":2020,
             "streamingOptions":{"gb":[{"service":{"id":"disney"}}]}}
        ],
        "hasMore": false
    }"#;
    let (entries, cursor) =
        parse_page_entries(json, "gb", &[Service::Disney]).unwrap();
    assert_eq!(cursor, None);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "100");
    assert_eq!(entries[0].1.title, "A");
    assert_eq!(entries[0].1.services, vec!["disney".to_string()]);
}

#[test]
fn parse_changes_collects_new_and_removed() {
    let json = r#"{
        "changes": [
            {"changeType":"new","itemType":"show","showId":"100"},
            {"changeType":"removed","itemType":"show","showId":"200"}
        ],
        "shows": {
            "100": {"id":"100","imdbId":"tt1","title":"A","showType":"movie",
                    "streamingOptions":{"gb":[{"service":{"id":"disney"}}]}}
        },
        "hasMore": false
    }"#;
    let (parsed, cursor) = parse_changes(json, "gb", &[Service::Disney]).unwrap();
    assert_eq!(cursor, None);
    assert_eq!(parsed.additions.len(), 1);
    assert_eq!(parsed.additions[0].0, "100");
    assert_eq!(parsed.removals, vec!["200".to_string()]);
}

#[test]
fn parse_changes_skips_new_without_show_detail_or_imdb() {
    // "new" 300 has no entry in `shows`; "new" 400 has detail but no imdbId.
    let json = r#"{
        "changes": [
            {"changeType":"new","itemType":"show","showId":"300"},
            {"changeType":"new","itemType":"show","showId":"400"}
        ],
        "shows": {
            "400": {"id":"400","title":"NoImdb","showType":"movie",
                    "streamingOptions":{"gb":[{"service":{"id":"disney"}}]}}
        },
        "hasMore": false
    }"#;
    let (parsed, _) = parse_changes(json, "gb", &[Service::Disney]).unwrap();
    assert!(parsed.additions.is_empty());
    assert!(parsed.removals.is_empty());
}

#[test]
fn parse_changes_propagates_cursor() {
    let json = r#"{"changes":[],"shows":{},"hasMore":true,"nextCursor":"abc"}"#;
    let (_, cursor) = parse_changes(json, "gb", &[Service::Disney]).unwrap();
    assert_eq!(cursor, Some("abc".to_string()));
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib sync::motn`
Expected: FAIL — `parse_page_entries` / `parse_changes` / `ParsedChanges` not found.

- [ ] **Step 4: Refactor `parse_page` and implement the new functions**

Add the import at the top of `src/sync/motn.rs`:

```rust
use crate::db::motn_cache::CachedTitle;
```

Extract the per-show mapping currently inside `parse_page`'s closure into a private fn (move the body verbatim — kind/year/length/attribution/`FetchedTitle` build):

```rust
/// Map one MOTN show into a `FetchedTitle`, attributing only the wanted services
/// its per-country `streamingOptions` actually lists (falling back to the searched
/// set when availability is missing). Shared by the seed and `/changes` paths.
fn show_to_fetched(s: Show, country: &str, services: &[Service]) -> FetchedTitle {
    let kind = if s.show_type == "series" {
        TitleKind::Series
    } else {
        TitleKind::Movie
    };
    let year = s.release_year.or(s.first_air_year);
    let length = match kind {
        TitleKind::Movie => s.runtime.map(|m| format!("{m} min")),
        TitleKind::Series => s.episode_count.map(|e| format!("{e} eps")),
    };
    let svcs = {
        let available: HashSet<&str> = s
            .streaming_options
            .get(country)
            .map(|opts| opts.iter().map(|o| o.service.id.as_str()).collect())
            .unwrap_or_default();
        let attributed: Vec<Service> = services
            .iter()
            .copied()
            .filter(|svc| wanted_id(*svc).is_some_and(|id| available.contains(id)))
            .collect();
        if attributed.is_empty() {
            services.to_vec()
        } else {
            attributed
        }
    };
    FetchedTitle {
        imdb_id: s.imdb_id,
        tmdb_id: s.tmdb_id,
        plex_guid: None,
        title: s.title,
        year,
        kind,
        imdb_rating: s.rating.map(|r| r / 10.0),
        length,
        description: s.overview,
        genres: s.genres.into_iter().map(|g| g.name).collect(),
        cast: s.cast,
        services: svcs,
    }
}
```

Replace `parse_page`'s body so it reuses `parse_page_entries` (keeps its public signature + existing tests green):

```rust
/// Parse one search page into `(titles, next_cursor)`. (Kept for callers/tests;
/// delegates to `parse_page_entries` and drops the show ids.)
///
/// # Errors
/// Returns an error if the JSON does not match the expected shape.
pub fn parse_page(
    json: &str,
    country: &str,
    services: &[Service],
) -> anyhow::Result<(Vec<FetchedTitle>, Option<String>)> {
    let (entries, cursor) = parse_page_entries(json, country, services)?;
    let titles = entries.into_iter().map(|(_, ct)| ct.into_fetched()).collect();
    Ok((titles, cursor))
}

/// Parse one search page into `((show_id, CachedTitle), next_cursor)` for seeding
/// the cache.
///
/// # Errors
/// Returns an error if the JSON does not match the expected shape.
pub fn parse_page_entries(
    json: &str,
    country: &str,
    services: &[Service],
) -> anyhow::Result<(Vec<(String, CachedTitle)>, Option<String>)> {
    let page: Page = serde_json::from_str(json)?;
    let entries = page
        .shows
        .into_iter()
        .map(|s| {
            let id = s.id.clone();
            let ft = show_to_fetched(s, country, services);
            (id, CachedTitle::from(&ft))
        })
        .collect();
    let cursor = if page.has_more { page.next_cursor } else { None };
    Ok((entries, cursor))
}
```

Add the changes parser + `ParsedChanges`:

```rust
/// Additions/removals distilled from one or more `/changes` pages.
#[derive(Debug, Default)]
pub struct ParsedChanges {
    pub additions: Vec<(String, CachedTitle)>, // (show_id, title)
    pub removals: Vec<String>,                 // show_ids
}

impl ParsedChanges {
    fn merge(&mut self, mut other: ParsedChanges) {
        self.additions.append(&mut other.additions);
        self.removals.append(&mut other.removals);
    }
}

/// Parse one `/changes` page into `(ParsedChanges, next_cursor)`.
///
/// `new` changes are turned into cache upserts using the embedded `shows` detail;
/// a `new` change whose show is missing from `shows` or lacks an `imdbId` is
/// skipped with a warning rather than aborting the sync. `removed` changes need
/// only the `showId`. Non-show item types are ignored.
///
/// # Errors
/// Returns an error only if the page JSON itself does not parse.
pub fn parse_changes(
    json: &str,
    country: &str,
    services: &[Service],
) -> anyhow::Result<(ParsedChanges, Option<String>)> {
    let mut page: ChangesPage = serde_json::from_str(json)?;
    let mut out = ParsedChanges::default();
    for ch in &page.changes {
        if ch.item_type.as_deref().is_some_and(|t| t != "show") {
            continue;
        }
        match ch.change_type.as_str() {
            "new" => {
                let Some(show) = page.shows.remove(&ch.show_id) else {
                    tracing::warn!("MOTN /changes 'new' {} missing show detail", ch.show_id);
                    continue;
                };
                let ft = show_to_fetched(show, country, services);
                if ft.imdb_id.is_none() {
                    tracing::warn!("MOTN /changes 'new' {} has no imdbId; skipping", ch.show_id);
                    continue;
                }
                out.additions.push((ch.show_id.clone(), CachedTitle::from(&ft)));
            }
            "removed" => out.removals.push(ch.show_id.clone()),
            _ => {}
        }
    }
    let cursor = if page.has_more { page.next_cursor } else { None };
    Ok((out, cursor))
}
```

> `ParsedChanges::merge` is used by `fetch()` in Task 5; keep it even though Task 3's tests don't call it (the `Produces` block documents it). If clippy flags it as unused before Task 5 lands, add `#[allow(dead_code)] // used by fetch() in the delta loop` with the reason, then remove the allow in Task 5.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib sync::motn`
Expected: PASS (existing `parse_page`/`resolve_services` tests + the 4 new ones).

- [ ] **Step 6: Clippy gate**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src/sync/motn.rs
git commit -m "refactor(sync): share show->title mapping; add /changes parser

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Mode decision + delta apply (DB logic)

**Files:**
- Modify: `src/sync/motn.rs` (add `SyncMode`, `decide_mode`, `apply_changes`; tests)

**Interfaces:**
- Consumes: `crate::db::{motn_cache, sync_runs}`, `ParsedChanges` (Task 3), `SqlitePool`.
- Produces:
  - `enum SyncMode { Seed, Delta { from: i64 } }`
  - `async fn decide_mode(pool: &SqlitePool) -> anyhow::Result<SyncMode>`
  - `async fn apply_changes(pool: &SqlitePool, parsed: &ParsedChanges) -> anyhow::Result<()>`
  - `const CHANGES_OVERLAP_SECS: i64 = 6 * 3600;`

- [ ] **Step 1: Write the failing tests**

Add to `#[cfg(test)] mod tests` in `src/sync/motn.rs`:

```rust
use crate::db::{init_pool, motn_cache, sync_runs};

async fn test_pool() -> (tempfile::TempDir, sqlx::SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let url = format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"));
    let pool = init_pool(&url).await.unwrap();
    (dir, pool)
}

fn cached(title: &str) -> CachedTitle {
    CachedTitle {
        imdb_id: Some(format!("tt-{title}")),
        tmdb_id: None,
        title: title.to_string(),
        year: None,
        kind: "movie".into(),
        imdb_rating: None,
        length: None,
        description: None,
        genres: vec![],
        cast: vec![],
        services: vec!["disney".into()],
    }
}

#[tokio::test]
async fn decide_mode_seeds_on_empty_cache() {
    let (_dir, pool) = test_pool().await;
    assert!(matches!(decide_mode(&pool).await.unwrap(), SyncMode::Seed));
}

#[tokio::test]
async fn decide_mode_seeds_when_no_recent_ok_run() {
    let (_dir, pool) = test_pool().await;
    motn_cache::upsert(&pool, "1", &cached("A")).await.unwrap();
    // Cache non-empty but no ok run within 25 days -> Seed (recovery re-seed).
    assert!(matches!(decide_mode(&pool).await.unwrap(), SyncMode::Seed));
}

#[tokio::test]
async fn decide_mode_deltas_when_cache_and_recent_ok() {
    let (_dir, pool) = test_pool().await;
    motn_cache::upsert(&pool, "1", &cached("A")).await.unwrap();
    sync_runs::record(&pool, "disney", "ok", 1, None).await.unwrap();
    match decide_mode(&pool).await.unwrap() {
        SyncMode::Delta { from } => assert!(from > 0),
        SyncMode::Seed => panic!("expected delta"),
    }
}

#[tokio::test]
async fn apply_changes_adds_and_removes() {
    let (_dir, pool) = test_pool().await;
    motn_cache::upsert(&pool, "keep", &cached("Keep")).await.unwrap();
    motn_cache::upsert(&pool, "drop", &cached("Drop")).await.unwrap();
    let parsed = ParsedChanges {
        additions: vec![("new1".to_string(), cached("New"))],
        removals: vec!["drop".to_string()],
    };
    apply_changes(&pool, &parsed).await.unwrap();
    let titles: Vec<String> =
        motn_cache::load_all(&pool).await.unwrap().into_iter().map(|t| t.title).collect();
    assert!(titles.contains(&"Keep".to_string()));
    assert!(titles.contains(&"New".to_string()));
    assert!(!titles.contains(&"Drop".to_string()));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib sync::motn`
Expected: FAIL — `decide_mode` / `apply_changes` / `SyncMode` not found.

- [ ] **Step 3: Implement**

Add to `src/sync/motn.rs`:

```rust
use crate::db::{motn_cache, sync_runs};

/// Safety overlap subtracted from the `from` timestamp so a change straddling the
/// previous run's boundary is never missed. Idempotent: re-applying a `new`/`removed`
/// for the same `showId` is a no-op.
const CHANGES_OVERLAP_SECS: i64 = 6 * 3600;

/// Which fetch strategy this run uses.
enum SyncMode {
    /// Full pagination of `/shows/search/filters`, replacing the whole cache.
    Seed,
    /// `/changes` since `from` (Unix seconds), applied to the existing cache.
    Delta { from: i64 },
}

/// Decide between a full seed and an incremental delta:
/// - empty cache (fresh install) → `Seed`
/// - no successful MOTN run within 25 days (gap exceeds the 31-day window) → `Seed`
/// - otherwise → `Delta` from the last-ok timestamp minus the overlap buffer.
async fn decide_mode(pool: &SqlitePool) -> anyhow::Result<SyncMode> {
    if motn_cache::count(pool).await? == 0 || !sync_runs::motn_recent_ok(pool).await? {
        return Ok(SyncMode::Seed);
    }
    let last_ok = sync_runs::last_ok_unix(pool).await?.unwrap_or(0);
    Ok(SyncMode::Delta {
        from: last_ok.saturating_sub(CHANGES_OVERLAP_SECS).max(0),
    })
}

/// Apply parsed `/changes` to the cache: upsert additions, delete removals.
///
/// # Errors
/// Returns an error if any cache write fails.
async fn apply_changes(pool: &SqlitePool, parsed: &ParsedChanges) -> anyhow::Result<()> {
    for (show_id, ct) in &parsed.additions {
        motn_cache::upsert(pool, show_id, ct).await?;
    }
    for show_id in &parsed.removals {
        motn_cache::delete(pool, show_id).await?;
    }
    Ok(())
}
```

Add the `SqlitePool` import to the top of `motn.rs` if not already present:

```rust
use sqlx::SqlitePool;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib sync::motn`
Expected: PASS.

- [ ] **Step 5: Clippy gate**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/sync/motn.rs
git commit -m "feat(sync): decide_mode + apply_changes for incremental MOTN

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Wire `fetch()` to seed/delta + thread the pool through

**Files:**
- Modify: `src/sync/motn.rs` (`MotnClient` struct + `new` signature; rewrite `fetch()`; add `/changes` request helpers)
- Modify: `src/main.rs:74-77` (pass the pool to `MotnClient::new`)

**Interfaces:**
- Consumes: everything from Tasks 1–4 (`decide_mode`, `apply_changes`, `parse_page_entries`, `parse_changes`, `motn_cache::{replace_all, load_all}`).
- Produces: `pub fn new(api_key: String, country: String, pool: SqlitePool) -> Self` (new signature). No other call sites exist (verified: only `main.rs:76`).

- [ ] **Step 1: Add the pool to `MotnClient` and update `new`**

In `src/sync/motn.rs`, change the struct + constructor:

```rust
pub struct MotnClient {
    client: reqwest::Client,
    api_key: String,
    country: String,
    pool: SqlitePool,
}

impl MotnClient {
    #[must_use]
    pub fn new(api_key: String, country: String, pool: SqlitePool) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            country,
            pool,
        }
    }
}
```

- [ ] **Step 2: Rewrite `fetch()` to seed-or-delta**

Replace the body of `impl CatalogueSource for MotnClient`'s `fetch` (keep `name`/`services` unchanged). Factor the catalog-resolve + the two HTTP loops into small private helpers on `MotnClient`:

```rust
async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>> {
    let Some((catalogs, services)) = self.resolve_catalogs().await? else {
        return Ok(Vec::new()); // none of [disney, crunchyroll] in this country
    };

    match decide_mode(&self.pool).await? {
        SyncMode::Seed => {
            let entries = self.seed_pages(&catalogs, &services).await?;
            tracing::info!("MOTN full seed: {} shows", entries.len());
            motn_cache::replace_all(&self.pool, &entries).await?;
        }
        SyncMode::Delta { from } => {
            let mut parsed = ParsedChanges::default();
            for change_type in ["new", "removed"] {
                let page = self
                    .changes_pages(&catalogs, &services, change_type, from)
                    .await?;
                parsed.merge(page);
            }
            tracing::info!(
                "MOTN delta: +{} -{}",
                parsed.additions.len(),
                parsed.removals.len()
            );
            apply_changes(&self.pool, &parsed).await?;
        }
    }

    motn_cache::load_all(&self.pool).await
}
```

Add the helpers in `impl MotnClient`:

```rust
/// Resolve `(catalogs_csv, services)` from `/countries`, or `None` if this
/// country lists none of the wanted services.
async fn resolve_catalogs(&self) -> anyhow::Result<Option<(String, Vec<Service>)>> {
    let countries = self
        .client
        .get(format!("{MOTN_BASE}/countries"))
        .header("X-API-Key", &self.api_key)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let resolved = resolve_services(&countries, &self.country);
    if resolved.is_empty() {
        tracing::warn!("MOTN lists none of [disney, crunchyroll] for {}", self.country);
        return Ok(None);
    }
    let catalogs = resolved.iter().map(|(_, id)| id.clone()).collect::<Vec<_>>().join(",");
    let services = resolved.iter().map(|(s, _)| *s).collect();
    Ok(Some((catalogs, services)))
}

/// Full pagination of `/shows/search/filters` → `(show_id, CachedTitle)` entries.
async fn seed_pages(
    &self,
    catalogs: &str,
    services: &[Service],
) -> anyhow::Result<Vec<(String, CachedTitle)>> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut req = self
            .client
            .get(format!("{MOTN_BASE}/shows/search/filters"))
            .header("X-API-Key", &self.api_key)
            .query(&[("country", self.country.as_str()), ("catalogs", catalogs)]);
        if let Some(c) = &cursor {
            req = req.query(&[("cursor", c.as_str())]);
        }
        let body = req.send().await?.error_for_status()?.text().await?;
        let (mut entries, next) = parse_page_entries(&body, &self.country, services)?;
        out.append(&mut entries);
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    Ok(out)
}

/// Paginate `/changes` for one `change_type` since `from` (Unix seconds).
async fn changes_pages(
    &self,
    catalogs: &str,
    services: &[Service],
    change_type: &str,
    from: i64,
) -> anyhow::Result<ParsedChanges> {
    let mut out = ParsedChanges::default();
    let from_str = from.to_string();
    let mut cursor: Option<String> = None;
    loop {
        let mut req = self
            .client
            .get(format!("{MOTN_BASE}/changes"))
            .header("X-API-Key", &self.api_key)
            .query(&[
                ("country", self.country.as_str()),
                ("catalogs", catalogs),
                ("item_type", "show"),
                ("change_type", change_type),
                ("from", from_str.as_str()),
            ]);
        if let Some(c) = &cursor {
            req = req.query(&[("cursor", c.as_str())]);
        }
        let body = req.send().await?.error_for_status()?.text().await?;
        let (parsed, next) = parse_changes(&body, &self.country, services)?;
        out.merge(parsed);
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    Ok(out)
}
```

> If Task 3 added an `#[allow(dead_code)]` to `ParsedChanges::merge`, remove it now — `merge` is used here.

- [ ] **Step 3: Update the call site in `main.rs`**

In `src/main.rs`, change the MOTN source construction (around line 74-77):

```rust
if let Some(key) = cfg.motn_api_key.clone() {
    let country = cfg.region.clone().unwrap_or_else(|| "gb".to_string());
    sources.push(Arc::new(cue::sync::motn::MotnClient::new(
        key,
        country,
        pool.clone(),
    )));
}
```

- [ ] **Step 4: Build + full test suite + clippy**

Run: `cargo build`
Expected: compiles.

Run: `cargo test`
Expected: all pass (no live network — `fetch()` isn't unit-tested).

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/sync/motn.rs src/main.rs
git commit -m "feat(sync): MOTN fetch() seeds or applies /changes deltas

Reconstructs the full snapshot from motn_catalog_cache so the orchestrator's
reconcile/prune contract is unchanged. Cuts monthly API usage from full daily
pagination to a /countries resolve + a couple of /changes pages.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Docs + live-verify checklist

**Files:**
- Modify: `CLAUDE.md` (one line under backend conventions about the MOTN cache, if a natural spot exists)
- Modify: `docs/superpowers/deferred-followups.md` (note: surfacing cache/seed state in `/api/sync/status` UI is out of scope; capture the §12 fixture-verification as a live-verify item)

**Interfaces:** none (docs only).

- [ ] **Step 1: Record the live-verify steps**

Append to `docs/superpowers/deferred-followups.md` (create the bullet under the appropriate section):

```markdown
- **MOTN incremental sync — live verify (post-merge):** On the server, run one sync
  with a non-empty cache and confirm logs show `MOTN delta: +N -M` (not a full seed),
  and that the monthly request counter increments by only a few. Capture a real
  `/changes` response as a fixture and confirm the embedded `shows` map carries
  `id`/`imdbId`/`streamingOptions` (spec §12 assumption). If `shows` is an array, not
  a map, adjust `ChangesPage.shows` to `Vec<Show>` keyed via each show's `id`.
- **MOTN cache state in settings UI (deferred):** `/api/sync/status` could surface
  cache size + last seed vs delta mode. Out of scope for the incremental-sync plan.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/deferred-followups.md CLAUDE.md
git commit -m "docs: MOTN incremental sync live-verify + deferred UI follow-up

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- §2 full-snapshot contract preserved → Task 5 `fetch()` returns `load_all` (full snapshot); orchestrator untouched. ✓
- §3/§4 `/changes` new+removed, item_type=show, from window → Task 5 `changes_pages`. ✓
- §5 `motn_catalog_cache` migration + last-ok via `sync_runs` → Tasks 1, 2. ✓
- §6 decide-then-act (seed/re-seed/delta) → Task 4 `decide_mode`, Task 5 `fetch()`. ✓
- §7 `id` on `Show`, shared attribution, fail-safe skip → Task 3. ✓
- §8 pool via new `db/motn_cache.rs` + `MotnClient` holds pool → Tasks 1, 5. ✓
- §9 idempotency (overlap buffer; upsert/delete by id), atomic re-seed (`replace_all` tx), failure leaves catalogue intact (errors propagate; orchestrator scoped-prune unchanged) → Tasks 1, 4, 5. ✓
- §11 testing posture → tests in Tasks 1–4; live verify Task 6. ✓
- §12 assumption + §13 out-of-scope → Task 6 deferred-followups. ✓

**Placeholder scan:** No TBD/TODO; every code step shows full code; commands have expected output. ✓

**Type consistency:** `CachedTitle` (fields, `From<&FetchedTitle>`, `into_fetched`), `ParsedChanges { additions: Vec<(String, CachedTitle)>, removals: Vec<String> }`, `SyncMode { Seed, Delta { from: i64 } }`, `decide_mode`/`apply_changes`/`parse_changes`/`parse_page_entries` signatures, and `MotnClient::new(_, _, SqlitePool)` are used identically across Tasks 1–5. ✓
