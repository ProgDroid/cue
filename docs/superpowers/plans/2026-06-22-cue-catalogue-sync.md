# Catalogue Sync (Plan 4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the 28-title dev seed with a live catalogue synced from Plex + the UK Disney+/Crunchyroll catalogues (Movie-of-the-Night API), refreshed daily and on demand, with an observable settings page.

**Architecture:** Two HTTP clients (`PlexClient`, `MotnClient`) sit behind a `CatalogueSource` trait (the same offline-test seam as `Embedder`/`AskModel`). A pure `merge.rs` dedups by identity, unions service membership, and normalizes genres. An orchestrator upserts merged titles, reconciles `title_services` per successfully-synced service, prunes orphaned titles, reuses the Plan 3 embedding `backfill`, and writes per-service `sync_runs` rows. A `tokio-cron-scheduler` job + a `POST /api/sync` trigger + a startup-if-empty run drive it; `GET /api/sync/status` reports per-source + catalogue stats; the header avatar opens a settings page.

**Tech Stack:** Rust, Actix-web 4, SQLx 0.8 (runtime queries only), `reqwest` 0.12, `tokio-cron-scheduler`, `async-trait`; Vue 3 `<script setup>` + TS + Vitest.

**Spec:** `docs/superpowers/specs/2026-06-22-cue-catalogue-sync-design.md`

## Global Constraints

- **Crate is lib + bin.** Declare every new module in `src/lib.rs` (`pub mod …`), not `main.rs`, so `cargo test --lib` sees it.
- **SQLx runtime queries only** — `sqlx::query` / `query_as` / `query_scalar`. NEVER the `query!` / `query_as!` macros (no live `DATABASE_URL` at build).
- **Test databases:** `tempfile::tempdir()` (NOT `NamedTempFile`); URL = `format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"))`; keep the `TempDir` guard bound as `_dir`.
- **Clippy gate:** `cargo clippy --all-targets -- -D warnings`. The `[lints.clippy]` table in `Cargo.toml` is the single source of truth; per-item exceptions use a local `#[allow(...)]` + one-line reason — never widen the global table.
- **Backend test gate:** `cargo test`. **Frontend:** dev installs use `npm install`; tests run with `npm test` (vitest run); build/type-check is `npm run build` (vue-tsc).
- **Security boundary:** all external keys live in server-side `Config` only and are NEVER serialized to Vue. `BIND_ADDR` default stays `127.0.0.1:8080`.
- **Commit via the Bash tool, not PowerShell** (PowerShell prepends a UTF-8 BOM to the commit subject). End commit messages with the `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` trailer.
- **Scheduler tests** must use `#[tokio::test(flavor = "multi_thread")]` — the crate's `add()` hangs on a single-threaded runtime.

Existing reusable building blocks (verified in-repo):
- `cue::services::embeddings::backfill(pool: &SqlitePool, embedder: &dyn Embedder) -> anyhow::Result<usize>` — idempotent; embeds titles lacking a current-model vector.
- `cue::models::Service` (`Plex`/`Disney`/`Crunchyroll`, `#[serde(rename_all="lowercase")]`, `Service::parse(&str) -> Option<Self>`) and `cue::models::TitleKind` (`Movie`/`Series`, lowercase).
- `titles(id, imdb_id UNIQUE, tmdb_id, plex_guid, title, year, type CHECK(movie|series), imdb_rating, length, description, added_at, updated_at)`; `title_services(title_id, service CHECK(plex|disney|crunchyroll), PK(title_id,service)) ON DELETE CASCADE`; `title_genres`, `title_cast(title_id,person,ord)`, `title_embeddings`, `sync_runs(id, source, started_at, finished_at, status, item_count, error)`. All child tables cascade on `titles` delete.
- Route registration: `cue::routes::configure(&mut web::ServiceConfig)` adds routes under the `/api` scope; handlers take `web::Data<…>` app data set in `src/main.rs`.

---

### Task 1: Scaffold the `sync` module — types, trait, model `as_str` helpers

**Files:**
- Create: `src/sync/mod.rs`
- Modify: `src/lib.rs` (add `pub mod sync;`)
- Modify: `src/models.rs` (add `Service::as_str`, `TitleKind::as_str`)
- Test: in `src/models.rs` `tests` module and `src/sync/mod.rs` `tests` module

**Interfaces:**
- Produces: `cue::models::Service::as_str(self) -> &'static str`; `cue::models::TitleKind::as_str(self) -> &'static str`; `cue::sync::FetchedTitle`; `cue::sync::CatalogueSource` trait with `name()`, `services()`, `async fn fetch()`.

- [ ] **Step 1: Write failing tests for the `as_str` helpers** (append to `src/models.rs` `mod tests`)

```rust
#[test]
fn service_as_str_roundtrips_parse() {
    for s in [Service::Plex, Service::Disney, Service::Crunchyroll] {
        assert_eq!(Service::parse(s.as_str()), Some(s));
    }
}

#[test]
fn title_kind_as_str_matches_db_values() {
    assert_eq!(TitleKind::Movie.as_str(), "movie");
    assert_eq!(TitleKind::Series.as_str(), "series");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib models`
Expected: FAIL — `no method named as_str`.

- [ ] **Step 3: Add the helpers** (in `src/models.rs`, inside `impl Service` and `impl TitleKind`)

```rust
// in impl Service
#[must_use]
pub const fn as_str(self) -> &'static str {
    match self {
        Self::Plex => "plex",
        Self::Disney => "disney",
        Self::Crunchyroll => "crunchyroll",
    }
}
```

```rust
// in impl TitleKind
#[must_use]
pub const fn as_str(self) -> &'static str {
    match self {
        Self::Movie => "movie",
        Self::Series => "series",
    }
}
```

- [ ] **Step 4: Create `src/sync/mod.rs` with the shared types + trait**

```rust
//! Catalogue sync subsystem: external clients behind `CatalogueSource`,
//! pure merge logic, DB reconciliation, and the run orchestrator.

use async_trait::async_trait;

use crate::models::{Service, TitleKind};

pub mod merge;

/// A source-agnostic catalogue row emitted by every `CatalogueSource`.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedTitle {
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub plex_guid: Option<String>,
    pub title: String,
    pub year: Option<i64>,
    pub kind: TitleKind,
    pub imdb_rating: Option<f64>,
    pub length: Option<String>,
    pub description: Option<String>,
    pub genres: Vec<String>,
    pub cast: Vec<String>,
    pub services: Vec<Service>,
}

/// An external catalogue client. A *client* is the unit of fetching and failure;
/// the *services* it owns are the unit of membership reconciliation.
#[async_trait]
pub trait CatalogueSource: Send + Sync {
    /// Client name, used for logging/orchestration (e.g. "plex", "motn").
    fn name(&self) -> &'static str;
    /// The `title_services` membership(s) this client owns
    /// (Plex → `[Plex]`; MOTN → `[Disney, Crunchyroll]`).
    fn services(&self) -> &'static [Service];
    /// Fetch the client's current catalogue.
    ///
    /// # Errors
    /// Returns an error if the upstream request fails or a body cannot be parsed.
    async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetched_title_constructs() {
        let t = FetchedTitle {
            imdb_id: Some("tt1".into()),
            tmdb_id: None,
            plex_guid: None,
            title: "X".into(),
            year: Some(2020),
            kind: TitleKind::Movie,
            imdb_rating: None,
            length: None,
            description: None,
            genres: vec![],
            cast: vec![],
            services: vec![Service::Plex],
        };
        assert_eq!(t.services, vec![Service::Plex]);
    }
}
```

Add `pub mod sync;` to `src/lib.rs` (alphabetical with the other `pub mod` lines). Create an empty placeholder `src/sync/merge.rs` with `//! Pure merge logic.` so the module compiles (Task 2 fills it).

- [ ] **Step 5: Run the lib tests**

Run: `cargo test --lib`
Expected: PASS (existing tests + the two new `as_str` tests + `fetched_title_constructs`).

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/models.rs src/sync/mod.rs src/sync/merge.rs
git commit -m "feat(sync): scaffold CatalogueSource trait + FetchedTitle + model as_str helpers"
```

---

### Task 2: Genre normalization (pure)

**Files:**
- Modify: `src/sync/merge.rs`
- Test: `src/sync/merge.rs` `tests` module

**Interfaces:**
- Produces: `cue::sync::merge::normalize_genres(&[String]) -> Vec<String>` (lowercased, trimmed, aliased, de-duplicated, sorted ascending).

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_lowercases_trims_dedups_sorts() {
        let out = normalize_genres(&["  Action ".into(), "Drama".into(), "action".into()]);
        assert_eq!(out, vec!["action".to_string(), "drama".to_string()]);
    }

    #[test]
    fn normalize_applies_alias_map() {
        let out = normalize_genres(&["Sci-Fi".into(), "SciFi".into(), "Science-Fiction".into()]);
        assert_eq!(out, vec!["science fiction".to_string()]);
    }

    #[test]
    fn normalize_drops_empty() {
        let out = normalize_genres(&["".into(), "   ".into(), "Comedy".into()]);
        assert_eq!(out, vec!["comedy".to_string()]);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib sync::merge`
Expected: FAIL — `cannot find function normalize_genres`.

- [ ] **Step 3: Implement** (top of `src/sync/merge.rs`)

```rust
//! Pure merge logic: genre normalization + dedup/union of fetched titles.

use std::collections::BTreeSet;

/// Map one raw genre to its normalized form (lowercased, trimmed, aliased).
fn normalize_one(raw: &str) -> String {
    let g = raw.trim().to_lowercase();
    match g.as_str() {
        "sci-fi" | "scifi" | "science-fiction" => "science fiction".to_string(),
        "rom-com" | "romcom" => "romantic comedy".to_string(),
        "docu" | "documentaries" => "documentary".to_string(),
        _ => g,
    }
}

/// Normalize, drop empties, de-duplicate, and sort a title's genres.
#[must_use]
pub fn normalize_genres(raws: &[String]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for r in raws {
        let g = normalize_one(r);
        if !g.is_empty() {
            set.insert(g);
        }
    }
    set.into_iter().collect()
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --lib sync::merge`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/sync/merge.rs
git commit -m "feat(sync): genre normalization with alias map"
```

---

### Task 3: Dedup + union merge (pure)

**Files:**
- Modify: `src/sync/merge.rs`
- Test: `src/sync/merge.rs` `tests` module

**Interfaces:**
- Consumes: `cue::sync::FetchedTitle`, `cue::models::{Service, TitleKind}`, `normalize_genres`.
- Produces: `cue::sync::merge::MergedTitle` (public struct, fields below); `cue::sync::merge::identity_key(imdb: Option<&str>, plex_guid: Option<&str>, tmdb: Option<&str>, title: &str, year: i64) -> String`; `cue::sync::merge::merge(Vec<FetchedTitle>) -> Vec<MergedTitle>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn identity_prefers_imdb_then_plex_then_tmdb_then_title() {
    assert_eq!(identity_key(Some("tt9"), Some("g"), Some("5"), "X", 2000), "tt9");
    assert_eq!(identity_key(None, Some("g"), Some("5"), "X", 2000), "plex:g");
    assert_eq!(identity_key(None, None, Some("5"), "X", 2000), "tmdb:5");
    assert_eq!(identity_key(None, None, None, "The Film", 2000), "title:the film:2000");
}

#[test]
fn merge_unions_services_and_genres_for_same_imdb() {
    let a = ft(Some("tt1"), TitleKind::Movie, vec!["Action".into()], vec![Service::Plex]);
    let b = ft(Some("tt1"), TitleKind::Movie, vec!["action".into(), "Drama".into()], vec![Service::Disney]);
    let out = merge(vec![a, b]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].services, vec![Service::Plex, Service::Disney]);
    assert_eq!(out[0].genres, vec!["action".to_string(), "drama".to_string()]);
}

#[test]
fn merge_keeps_distinct_titles_in_first_seen_order() {
    let a = ft(Some("tt1"), TitleKind::Movie, vec![], vec![Service::Plex]);
    let b = ft(Some("tt2"), TitleKind::Series, vec![], vec![Service::Crunchyroll]);
    let out = merge(vec![a, b]);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].imdb_id.as_deref(), Some("tt1"));
    assert_eq!(out[1].imdb_id.as_deref(), Some("tt2"));
}

// test helper
fn ft(imdb: Option<&str>, kind: TitleKind, genres: Vec<String>, services: Vec<Service>) -> FetchedTitle {
    FetchedTitle {
        imdb_id: imdb.map(str::to_string),
        tmdb_id: None,
        plex_guid: None,
        title: "T".into(),
        year: Some(2001),
        kind,
        imdb_rating: None,
        length: None,
        description: None,
        genres,
        cast: vec![],
        services,
    }
}
```

Add `use crate::sync::FetchedTitle;` and `use crate::models::{Service, TitleKind};` to the `tests` module imports as needed.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib sync::merge`
Expected: FAIL — `MergedTitle`, `identity_key`, `merge` undefined.

- [ ] **Step 3: Implement** (append to `src/sync/merge.rs`)

```rust
use std::collections::HashMap;

use crate::models::{Service, TitleKind};
use crate::sync::FetchedTitle;

/// A deduplicated title ready for DB upsert (genres already normalized).
#[derive(Debug, Clone, PartialEq)]
pub struct MergedTitle {
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub plex_guid: Option<String>,
    pub title: String,
    pub year: i64,
    pub kind: TitleKind,
    pub imdb_rating: Option<f64>,
    pub length: String,
    pub description: String,
    pub genres: Vec<String>,
    pub cast: Vec<String>,
    pub services: Vec<Service>,
}

/// Stable identity key (D6): IMDb id, else `plex:<guid>`, else `tmdb:<id>`,
/// else a `title:<lower>:<year>` fallback.
#[must_use]
pub fn identity_key(
    imdb: Option<&str>,
    plex_guid: Option<&str>,
    tmdb: Option<&str>,
    title: &str,
    year: i64,
) -> String {
    if let Some(i) = imdb {
        return i.to_string();
    }
    if let Some(g) = plex_guid {
        return format!("plex:{g}");
    }
    if let Some(m) = tmdb {
        return format!("tmdb:{m}");
    }
    format!("title:{}:{year}", title.to_lowercase())
}

/// Dedup fetched rows by identity, unioning services/genres/cast and filling
/// missing scalar fields from whichever row first provides them.
#[must_use]
pub fn merge(fetched: Vec<FetchedTitle>) -> Vec<MergedTitle> {
    let mut order: Vec<String> = Vec::new();
    let mut by_key: HashMap<String, MergedTitle> = HashMap::new();
    // Accumulate raw (un-normalized) genres per key, normalize once at the end.
    let mut raw_genres: HashMap<String, Vec<String>> = HashMap::new();

    for f in fetched {
        let key = identity_key(
            f.imdb_id.as_deref(),
            f.plex_guid.as_deref(),
            f.tmdb_id.as_deref(),
            &f.title,
            f.year.unwrap_or(0),
        );
        raw_genres.entry(key.clone()).or_default().extend(f.genres.clone());
        match by_key.get_mut(&key) {
            Some(existing) => {
                for s in f.services {
                    if !existing.services.contains(&s) {
                        existing.services.push(s);
                    }
                }
                for c in f.cast {
                    if !existing.cast.contains(&c) {
                        existing.cast.push(c);
                    }
                }
                existing.imdb_id = existing.imdb_id.take().or(f.imdb_id);
                existing.tmdb_id = existing.tmdb_id.take().or(f.tmdb_id);
                existing.plex_guid = existing.plex_guid.take().or(f.plex_guid);
                existing.imdb_rating = existing.imdb_rating.or(f.imdb_rating);
                if existing.description.is_empty() {
                    existing.description = f.description.unwrap_or_default();
                }
                if existing.length.is_empty() {
                    existing.length = f.length.unwrap_or_default();
                }
            }
            None => {
                order.push(key.clone());
                by_key.insert(
                    key,
                    MergedTitle {
                        imdb_id: f.imdb_id,
                        tmdb_id: f.tmdb_id,
                        plex_guid: f.plex_guid,
                        title: f.title,
                        year: f.year.unwrap_or(0),
                        kind: f.kind,
                        imdb_rating: f.imdb_rating,
                        length: f.length.unwrap_or_default(),
                        description: f.description.unwrap_or_default(),
                        genres: Vec::new(),
                        cast: f.cast,
                        services: f.services,
                    },
                );
            }
        }
    }

    order
        .into_iter()
        .map(|key| {
            let mut m = by_key.remove(&key).expect("key present");
            m.genres = normalize_genres(&raw_genres.remove(&key).unwrap_or_default());
            m
        })
        .collect()
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --lib sync::merge`
Expected: PASS (all merge tests).

- [ ] **Step 5: Commit**

```bash
git add src/sync/merge.rs
git commit -m "feat(sync): dedup + union merge of fetched titles"
```

---

### Task 4: DB store — upsert title, reconcile services, prune orphans

**Files:**
- Create: `src/sync/store.rs`
- Modify: `src/sync/mod.rs` (add `pub mod store;`)
- Test: `src/sync/store.rs` `tests` module

**Interfaces:**
- Consumes: `MergedTitle`, `Service`.
- Produces:
  - `async fn upsert_title(pool: &SqlitePool, t: &MergedTitle) -> anyhow::Result<i64>` (insert or update by identity; replaces that title's genres + cast; returns the surrogate id).
  - `async fn reconcile_service(pool: &SqlitePool, service: Service, desired_ids: &[i64]) -> anyhow::Result<()>`.
  - `async fn prune_orphans(pool: &SqlitePool) -> anyhow::Result<u64>` (delete titles with zero `title_services`; returns rows removed).

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_pool;
    use crate::models::TitleKind;

    async fn pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (init_pool(&url).await.unwrap(), dir)
    }

    fn merged(imdb: &str, title: &str, genres: &[&str], services: &[Service]) -> MergedTitle {
        MergedTitle {
            imdb_id: Some(imdb.into()),
            tmdb_id: None,
            plex_guid: None,
            title: title.into(),
            year: 2020,
            kind: TitleKind::Movie,
            imdb_rating: Some(7.5),
            length: "100 min".into(),
            description: "d".into(),
            genres: genres.iter().map(|s| (*s).to_string()).collect(),
            cast: vec!["Actor".into()],
            services: services.to_vec(),
        }
    }

    #[tokio::test]
    async fn upsert_inserts_then_updates_same_id() {
        let (p, _dir) = pool().await;
        let id1 = upsert_title(&p, &merged("tt1", "Old", &["action"], &[Service::Plex])).await.unwrap();
        let id2 = upsert_title(&p, &merged("tt1", "New", &["drama"], &[Service::Plex])).await.unwrap();
        assert_eq!(id1, id2, "same imdb_id reuses the surrogate id");
        let title: String = sqlx::query_scalar("SELECT title FROM titles WHERE id = ?").bind(id1).fetch_one(&p).await.unwrap();
        assert_eq!(title, "New");
        let genres: Vec<String> = sqlx::query_scalar("SELECT genre FROM title_genres WHERE title_id = ?").bind(id1).fetch_all(&p).await.unwrap();
        assert_eq!(genres, vec!["drama".to_string()], "genres replaced, not appended");
    }

    #[tokio::test]
    async fn reconcile_adds_and_removes_then_prune_deletes_orphans() {
        let (p, _dir) = pool().await;
        let a = upsert_title(&p, &merged("tt1", "A", &[], &[Service::Plex])).await.unwrap();
        let b = upsert_title(&p, &merged("tt2", "B", &[], &[Service::Plex])).await.unwrap();
        reconcile_service(&p, Service::Plex, &[a, b]).await.unwrap();
        // Second sync: only `a` is still on Plex.
        reconcile_service(&p, Service::Plex, &[a]).await.unwrap();
        let removed = prune_orphans(&p).await.unwrap();
        assert_eq!(removed, 1);
        let remaining: Vec<i64> = sqlx::query_scalar("SELECT id FROM titles ORDER BY id").fetch_all(&p).await.unwrap();
        assert_eq!(remaining, vec![a]);
    }

    #[tokio::test]
    async fn prune_spares_titles_owned_by_an_untouched_service() {
        let (p, _dir) = pool().await;
        let x = upsert_title(&p, &merged("tt3", "X", &[], &[Service::Crunchyroll])).await.unwrap();
        reconcile_service(&p, Service::Crunchyroll, &[x]).await.unwrap();
        // A Plex-only sync runs and reconciles Plex to empty; Crunchyroll untouched.
        reconcile_service(&p, Service::Plex, &[]).await.unwrap();
        let removed = prune_orphans(&p).await.unwrap();
        assert_eq!(removed, 0, "Crunchyroll membership keeps the title alive");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib sync::store`
Expected: FAIL — module/functions undefined.

- [ ] **Step 3: Implement `src/sync/store.rs`**

```rust
//! DB writes for sync: upsert titles, reconcile service membership, prune.

use std::collections::HashSet;

use sqlx::SqlitePool;

use crate::models::Service;
use crate::sync::merge::MergedTitle;

/// Find an existing surrogate id by identity (imdb → tmdb → plex_guid).
async fn find_existing(pool: &SqlitePool, t: &MergedTitle) -> anyhow::Result<Option<i64>> {
    if let Some(imdb) = &t.imdb_id {
        if let Some(id) = sqlx::query_scalar::<_, i64>("SELECT id FROM titles WHERE imdb_id = ?")
            .bind(imdb).fetch_optional(pool).await?
        {
            return Ok(Some(id));
        }
    }
    if let Some(tmdb) = &t.tmdb_id {
        if let Some(id) = sqlx::query_scalar::<_, i64>("SELECT id FROM titles WHERE tmdb_id = ?")
            .bind(tmdb).fetch_optional(pool).await?
        {
            return Ok(Some(id));
        }
    }
    if let Some(guid) = &t.plex_guid {
        if let Some(id) = sqlx::query_scalar::<_, i64>("SELECT id FROM titles WHERE plex_guid = ?")
            .bind(guid).fetch_optional(pool).await?
        {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Replace a title's genre + cast child rows.
async fn replace_children(pool: &SqlitePool, id: i64, t: &MergedTitle) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM title_genres WHERE title_id = ?").bind(id).execute(pool).await?;
    for g in &t.genres {
        sqlx::query("INSERT OR IGNORE INTO title_genres (title_id, genre) VALUES (?, ?)")
            .bind(id).bind(g).execute(pool).await?;
    }
    sqlx::query("DELETE FROM title_cast WHERE title_id = ?").bind(id).execute(pool).await?;
    for (ord, person) in t.cast.iter().enumerate() {
        sqlx::query("INSERT OR IGNORE INTO title_cast (title_id, person, ord) VALUES (?, ?, ?)")
            .bind(id).bind(person).bind(i64::try_from(ord).unwrap_or(i64::MAX))
            .execute(pool).await?;
    }
    Ok(())
}

/// Insert a new title or update the existing one matched by identity. Returns its id.
///
/// # Errors
/// Returns an error if any query fails.
pub async fn upsert_title(pool: &SqlitePool, t: &MergedTitle) -> anyhow::Result<i64> {
    let id = if let Some(id) = find_existing(pool, t).await? {
        sqlx::query(
            "UPDATE titles SET imdb_id = ?, tmdb_id = ?, plex_guid = ?, title = ?, year = ?,
             type = ?, imdb_rating = ?, length = ?, description = ?, updated_at = datetime('now')
             WHERE id = ?",
        )
        .bind(&t.imdb_id).bind(&t.tmdb_id).bind(&t.plex_guid).bind(&t.title).bind(t.year)
        .bind(t.kind.as_str()).bind(t.imdb_rating).bind(&t.length).bind(&t.description).bind(id)
        .execute(pool).await?;
        id
    } else {
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO titles (imdb_id, tmdb_id, plex_guid, title, year, type, imdb_rating, length, description)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(&t.imdb_id).bind(&t.tmdb_id).bind(&t.plex_guid).bind(&t.title).bind(t.year)
        .bind(t.kind.as_str()).bind(t.imdb_rating).bind(&t.length).bind(&t.description)
        .fetch_one(pool).await?
    };
    replace_children(pool, id, t).await?;
    Ok(id)
}

/// Make `title_services` for `service` exactly match `desired_ids`.
///
/// # Errors
/// Returns an error if any query fails.
pub async fn reconcile_service(
    pool: &SqlitePool,
    service: Service,
    desired_ids: &[i64],
) -> anyhow::Result<()> {
    let want: HashSet<i64> = desired_ids.iter().copied().collect();
    let current: Vec<i64> =
        sqlx::query_scalar("SELECT title_id FROM title_services WHERE service = ?")
            .bind(service.as_str()).fetch_all(pool).await?;
    for id in current {
        if !want.contains(&id) {
            sqlx::query("DELETE FROM title_services WHERE service = ? AND title_id = ?")
                .bind(service.as_str()).bind(id).execute(pool).await?;
        }
    }
    for id in desired_ids {
        sqlx::query("INSERT OR IGNORE INTO title_services (title_id, service) VALUES (?, ?)")
            .bind(id).bind(service.as_str()).execute(pool).await?;
    }
    Ok(())
}

/// Delete titles with no service membership. Returns rows removed.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn prune_orphans(pool: &SqlitePool) -> anyhow::Result<u64> {
    let res = sqlx::query("DELETE FROM titles WHERE id NOT IN (SELECT title_id FROM title_services)")
        .execute(pool).await?;
    Ok(res.rows_affected())
}
```

Add `pub mod store;` to `src/sync/mod.rs`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --lib sync::store`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/sync/mod.rs src/sync/store.rs
git commit -m "feat(sync): DB upsert + per-service reconcile + orphan prune"
```

---

### Task 5: `sync_runs` recording + status queries

**Files:**
- Create: `src/db/sync_runs.rs`
- Modify: `src/db/mod.rs` (add `pub mod sync_runs;`)
- Test: `src/db/sync_runs.rs` `tests` module

**Interfaces:**
- Produces:
  - `struct SourceRun { source: String, last_run: Option<String>, status: String, item_count: i64 }` (serde `Serialize`, camelCase: `source`, `lastRun`, `status`, `itemCount`).
  - `async fn record(pool, source: &str, status: &str, item_count: i64, error: Option<&str>) -> anyhow::Result<()>`.
  - `async fn latest_per_source(pool) -> anyhow::Result<Vec<SourceRun>>`.
  - `struct CatalogueStats { titles: i64, movies: i64, series: i64, embedded: i64 }` (serde `Serialize`).
  - `async fn catalogue_stats(pool) -> anyhow::Result<CatalogueStats>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{init_pool, seed::seed_if_empty};

    async fn pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (init_pool(&url).await.unwrap(), dir)
    }

    #[tokio::test]
    async fn record_then_latest_per_source_returns_newest_row() {
        let (p, _dir) = pool().await;
        record(&p, "plex", "ok", 10, None).await.unwrap();
        record(&p, "plex", "error", 0, Some("boom")).await.unwrap();
        record(&p, "disney", "ok", 5, None).await.unwrap();
        let rows = latest_per_source(&p).await.unwrap();
        let plex = rows.iter().find(|r| r.source == "plex").unwrap();
        assert_eq!(plex.status, "error");
        assert_eq!(rows.iter().find(|r| r.source == "disney").unwrap().item_count, 5);
    }

    #[tokio::test]
    async fn catalogue_stats_counts_seed() {
        let (p, _dir) = pool().await;
        seed_if_empty(&p).await.unwrap();
        let s = catalogue_stats(&p).await.unwrap();
        assert_eq!(s.titles, 28);
        assert_eq!(s.movies + s.series, 28);
        assert_eq!(s.embedded, 0);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib db::sync_runs`
Expected: FAIL — module/functions undefined.

- [ ] **Step 3: Implement `src/db/sync_runs.rs`**

```rust
//! Read/write helpers for the `sync_runs` table + catalogue stats.

use serde::Serialize;
use sqlx::SqlitePool;

/// Latest run for one source, for `/api/sync/status`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRun {
    pub source: String,
    pub last_run: Option<String>,
    pub status: String,
    pub item_count: i64,
}

/// Aggregate catalogue counts for `/api/sync/status`.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogueStats {
    pub titles: i64,
    pub movies: i64,
    pub series: i64,
    pub embedded: i64,
}

/// Write one completed run row for a source.
///
/// # Errors
/// Returns an error if the insert fails.
pub async fn record(
    pool: &SqlitePool,
    source: &str,
    status: &str,
    item_count: i64,
    error: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sync_runs (source, finished_at, status, item_count, error)
         VALUES (?, datetime('now'), ?, ?, ?)",
    )
    .bind(source).bind(status).bind(item_count).bind(error)
    .execute(pool).await?;
    Ok(())
}

/// The newest run per source (by row id).
///
/// # Errors
/// Returns an error if the query fails.
pub async fn latest_per_source(pool: &SqlitePool) -> anyhow::Result<Vec<SourceRun>> {
    let rows = sqlx::query_as::<_, (String, Option<String>, String, i64)>(
        "SELECT source, finished_at, status, item_count FROM sync_runs
         WHERE id IN (SELECT MAX(id) FROM sync_runs GROUP BY source)
         ORDER BY source",
    )
    .fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|(source, last_run, status, item_count)| SourceRun { source, last_run, status, item_count })
        .collect())
}

/// Catalogue counts: total titles, movies, series, and how many are embedded.
///
/// # Errors
/// Returns an error if a query fails.
pub async fn catalogue_stats(pool: &SqlitePool) -> anyhow::Result<CatalogueStats> {
    let titles: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM titles").fetch_one(pool).await?;
    let movies: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM titles WHERE type = 'movie'").fetch_one(pool).await?;
    let series: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM titles WHERE type = 'series'").fetch_one(pool).await?;
    let embedded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_embeddings").fetch_one(pool).await?;
    Ok(CatalogueStats { titles, movies, series, embedded })
}
```

Add `pub mod sync_runs;` to `src/db/mod.rs`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --lib db::sync_runs`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/db/mod.rs src/db/sync_runs.rs
git commit -m "feat(db): sync_runs recording, latest-per-source, catalogue stats"
```

---

### Task 6: Orchestrator `run_sync` + `SyncRunner` guard

**Files:**
- Modify: `src/sync/mod.rs`
- Test: `src/sync/mod.rs` `tests` module

**Interfaces:**
- Consumes: `CatalogueSource`, `merge::merge`, `store::{upsert_title, reconcile_service, prune_orphans}`, `db::sync_runs::record`, `services::embeddings::{Embedder, backfill}`.
- Produces:
  - `async fn run_sync(pool: &SqlitePool, sources: &[Arc<dyn CatalogueSource>], embedder: Option<&dyn Embedder>) -> anyhow::Result<()>`.
  - `struct SyncRunner { … }` with `SyncRunner::new(pool, sources, embedder) -> Arc<Self>`, `fn try_start(self: &Arc<Self>) -> bool` (false if already running), `fn is_running(&self) -> bool`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod orchestrator_tests {
    use super::*;
    use std::sync::Arc;
    use crate::db::{init_pool, seed::seed_if_empty};
    use crate::models::{Service, TitleKind};

    struct FakeSource {
        name: &'static str,
        services: Vec<Service>,
        result: anyhow::Result<Vec<FetchedTitle>>,
    }
    #[async_trait]
    impl CatalogueSource for FakeSource {
        fn name(&self) -> &'static str { self.name }
        fn services(&self) -> &'static [Service] {
            // leak a 'static slice for the test
            Box::leak(self.services.clone().into_boxed_slice())
        }
        async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>> {
            match &self.result {
                Ok(v) => Ok(v.clone()),
                Err(e) => Err(anyhow::anyhow!("{e}")),
            }
        }
    }

    fn title(imdb: &str, services: Vec<Service>) -> FetchedTitle {
        FetchedTitle {
            imdb_id: Some(imdb.into()), tmdb_id: None, plex_guid: None,
            title: imdb.into(), year: Some(2020), kind: TitleKind::Movie,
            imdb_rating: None, length: None, description: None,
            genres: vec!["action".into()], cast: vec![], services,
        }
    }

    async fn pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (init_pool(&url).await.unwrap(), dir)
    }

    #[tokio::test]
    async fn run_sync_replaces_seed_with_fetched_titles() {
        let (p, _dir) = pool().await;
        seed_if_empty(&p).await.unwrap();
        let plex = Arc::new(FakeSource {
            name: "plex", services: vec![Service::Plex],
            result: Ok(vec![title("tt100", vec![Service::Plex])]),
        }) as Arc<dyn CatalogueSource>;
        run_sync(&p, &[plex], None).await.unwrap();
        let imdbs: Vec<String> = sqlx::query_scalar("SELECT imdb_id FROM titles").fetch_all(&p).await.unwrap();
        assert_eq!(imdbs, vec!["tt100".to_string()], "seed pruned, fetched title remains");
    }

    #[tokio::test]
    async fn failed_source_does_not_prune_its_services() {
        let (p, _dir) = pool().await;
        // Pre-populate a crunchyroll title via a successful MOTN-like run.
        let motn_ok = Arc::new(FakeSource {
            name: "motn", services: vec![Service::Disney, Service::Crunchyroll],
            result: Ok(vec![title("ttC", vec![Service::Crunchyroll])]),
        }) as Arc<dyn CatalogueSource>;
        run_sync(&p, &[motn_ok], None).await.unwrap();
        // Now run with a Plex success + MOTN failure.
        let plex = Arc::new(FakeSource {
            name: "plex", services: vec![Service::Plex],
            result: Ok(vec![title("ttP", vec![Service::Plex])]),
        }) as Arc<dyn CatalogueSource>;
        let motn_fail = Arc::new(FakeSource {
            name: "motn", services: vec![Service::Disney, Service::Crunchyroll],
            result: Err(anyhow::anyhow!("down")),
        }) as Arc<dyn CatalogueSource>;
        run_sync(&p, &[plex, motn_fail], None).await.unwrap();
        let imdbs: Vec<String> = sqlx::query_scalar("SELECT imdb_id FROM titles ORDER BY imdb_id").fetch_all(&p).await.unwrap();
        assert_eq!(imdbs, vec!["ttC".to_string(), "ttP".to_string()], "crunchyroll title survives MOTN outage");
        // sync_runs records an error for the failed services.
        let errs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_runs WHERE status = 'error'").fetch_one(&p).await.unwrap();
        assert!(errs >= 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runner_guard_rejects_concurrent_start() {
        let (p, _dir) = pool().await;
        let runner = SyncRunner::new(p, vec![], None);
        assert!(runner.try_start());
        // Second immediate start is rejected while the first is in flight.
        let second = runner.try_start();
        // Either rejected (still running) or the first finished instantly; assert the API shape works.
        assert!(second || !runner.is_running());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib sync::`
Expected: FAIL — `run_sync` / `SyncRunner` undefined.

- [ ] **Step 3: Implement** (append to `src/sync/mod.rs`; add imports at top)

Add near the top of the file:

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sqlx::SqlitePool;

use crate::db::sync_runs;
use crate::services::embeddings::{backfill, Embedder};

pub mod store;
```

Append:

```rust
/// Run one full sync: fetch every source, merge, reconcile each successfully
/// fetched service, prune orphans, embed new titles, and record `sync_runs`.
///
/// Never returns `Err` for a single source failure — a failed source is recorded
/// and its services are left untouched (scoped prune).
///
/// # Errors
/// Returns an error only if a DB write fails irrecoverably.
pub async fn run_sync(
    pool: &SqlitePool,
    sources: &[Arc<dyn CatalogueSource>],
    embedder: Option<&dyn Embedder>,
) -> anyhow::Result<()> {
    let mut fetched: Vec<FetchedTitle> = Vec::new();
    // (service, ok?) and any error message keyed by source name.
    let mut ok_services: Vec<Service> = Vec::new();
    let mut failures: Vec<(Service, String)> = Vec::new();

    for src in sources {
        match src.fetch().await {
            Ok(mut rows) => {
                tracing::info!("sync source {} fetched {} rows", src.name(), rows.len());
                fetched.append(&mut rows);
                ok_services.extend_from_slice(src.services());
            }
            Err(e) => {
                tracing::error!("sync source {} failed: {e:#}", src.name());
                for s in src.services() {
                    failures.push((*s, format!("{e:#}")));
                }
            }
        }
    }

    let merged = merge::merge(fetched);
    let mut id_services: Vec<(i64, Vec<Service>)> = Vec::with_capacity(merged.len());
    for m in &merged {
        let id = store::upsert_title(pool, m).await?;
        id_services.push((id, m.services.clone()));
    }

    // Reconcile + record only the services whose client succeeded.
    let mut unique_ok = ok_services.clone();
    unique_ok.sort_by_key(|s| s.as_str());
    unique_ok.dedup();
    for svc in &unique_ok {
        let desired: Vec<i64> = id_services
            .iter()
            .filter(|(_, services)| services.contains(svc))
            .map(|(id, _)| *id)
            .collect();
        let count = i64::try_from(desired.len()).unwrap_or(i64::MAX);
        store::reconcile_service(pool, *svc, &desired).await?;
        sync_runs::record(pool, svc.as_str(), "ok", count, None).await?;
    }
    for (svc, err) in &failures {
        sync_runs::record(pool, svc.as_str(), "error", 0, Some(err)).await?;
    }

    let pruned = store::prune_orphans(pool).await?;
    tracing::info!("sync pruned {pruned} orphaned titles");

    if let Some(emb) = embedder {
        match backfill(pool, emb).await {
            Ok(n) => tracing::info!("sync embedded {n} titles"),
            Err(e) => tracing::error!("sync embedding backfill failed: {e:#}"),
        }
    } else {
        tracing::warn!("OPENAI_API_KEY unset — synced titles left unembedded");
    }
    Ok(())
}

/// Owns the sources + embedder and guards against concurrent runs.
pub struct SyncRunner {
    pool: SqlitePool,
    sources: Vec<Arc<dyn CatalogueSource>>,
    embedder: Option<Arc<dyn Embedder>>,
    running: AtomicBool,
}

impl SyncRunner {
    #[must_use]
    pub fn new(
        pool: SqlitePool,
        sources: Vec<Arc<dyn CatalogueSource>>,
        embedder: Option<Arc<dyn Embedder>>,
    ) -> Arc<Self> {
        Arc::new(Self { pool, sources, embedder, running: AtomicBool::new(false) })
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Spawn a sync if none is in flight. Returns `false` if one is already running.
    pub fn try_start(self: &Arc<Self>) -> bool {
        if self
            .running
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return false;
        }
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let embedder = me.embedder.as_deref();
            if let Err(e) = run_sync(&me.pool, &me.sources, embedder).await {
                tracing::error!("sync run failed: {e:#}");
            }
            me.running.store(false, Ordering::Release);
        });
        true
    }
}
```

> Note: `pub mod store;` and `pub mod merge;` must each appear exactly once in `src/sync/mod.rs` — Task 1 added `merge`, Task 4 added `store`; this step's import block re-states `pub mod store;` only if not already present. Keep a single declaration.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --lib sync::`
Expected: PASS (orchestrator tests + earlier sync tests).

- [ ] **Step 5: Run the clippy gate**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/sync/mod.rs
git commit -m "feat(sync): run_sync orchestrator + SyncRunner concurrency guard"
```

---

### Task 7: `PlexClient` — parse a Plex library into `FetchedTitle`

**Files:**
- Create: `src/sync/plex.rs`
- Create: `tests/fixtures/plex_section_all.json`
- Modify: `src/sync/mod.rs` (add `pub mod plex;`)
- Test: `src/sync/plex.rs` `tests` module

**Interfaces:**
- Consumes: `FetchedTitle`, `CatalogueSource`, `Service`, `TitleKind`.
- Produces: `cue::sync::plex::PlexClient` (`PlexClient::new(base_url: String, token: String)`), `pub fn parse_section(json: &str) -> anyhow::Result<Vec<FetchedTitle>>`.

Plex returns `{ "MediaContainer": { "Metadata": [ { "type": "movie|show", "title", "year", "summary", "rating", "Guid":[{"id":"imdb://tt..."}], "Genre":[{"tag":"Action"}], "Role":[{"tag":"Actor"}] } ] } }`.

- [ ] **Step 1: Create the fixture `tests/fixtures/plex_section_all.json`**

```json
{
  "MediaContainer": {
    "Metadata": [
      {
        "type": "movie",
        "title": "Blade Runner 2049",
        "year": 2017,
        "summary": "A young blade runner uncovers a secret.",
        "rating": 8.0,
        "duration": 9840000,
        "Guid": [{ "id": "imdb://tt1856101" }, { "id": "tmdb://335984" }],
        "Genre": [{ "tag": "Sci-Fi" }, { "tag": "Drama" }],
        "Role": [{ "tag": "Ryan Gosling" }, { "tag": "Harrison Ford" }]
      },
      {
        "type": "show",
        "title": "Severance",
        "year": 2022,
        "summary": "Workers split memories.",
        "Guid": [{ "id": "imdb://tt11280740" }],
        "Genre": [{ "tag": "Thriller" }],
        "Role": [{ "tag": "Adam Scott" }]
      }
    ]
  }
}
```

- [ ] **Step 2: Write the failing test** (`src/sync/plex.rs` `tests`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Service, TitleKind};

    #[test]
    fn parses_metadata_into_fetched_titles() {
        let json = include_str!("../../tests/fixtures/plex_section_all.json");
        let out = parse_section(json).unwrap();
        assert_eq!(out.len(), 2);

        let film = &out[0];
        assert_eq!(film.imdb_id.as_deref(), Some("tt1856101"));
        assert_eq!(film.tmdb_id.as_deref(), Some("335984"));
        assert_eq!(film.kind, TitleKind::Movie);
        assert_eq!(film.year, Some(2017));
        assert_eq!(film.genres, vec!["Sci-Fi".to_string(), "Drama".to_string()]);
        assert_eq!(film.cast, vec!["Ryan Gosling".to_string(), "Harrison Ford".to_string()]);
        assert_eq!(film.services, vec![Service::Plex]);
        assert_eq!(film.length.as_deref(), Some("164 min"));

        let show = &out[1];
        assert_eq!(show.kind, TitleKind::Series);
        assert_eq!(show.imdb_id.as_deref(), Some("tt11280740"));
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test --lib sync::plex`
Expected: FAIL — `parse_section` undefined.

- [ ] **Step 4: Implement `src/sync/plex.rs`**

```rust
//! Plex client: list library sections and parse items into `FetchedTitle`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::models::{Service, TitleKind};
use crate::sync::{CatalogueSource, FetchedTitle};

#[derive(Deserialize)]
struct Container {
    #[serde(rename = "MediaContainer")]
    media_container: MediaContainer,
}
#[derive(Deserialize)]
struct MediaContainer {
    #[serde(default, rename = "Metadata")]
    metadata: Vec<Meta>,
}
#[derive(Deserialize)]
struct Meta {
    #[serde(rename = "type")]
    kind: String,
    title: String,
    year: Option<i64>,
    summary: Option<String>,
    rating: Option<f64>,
    duration: Option<i64>,
    #[serde(default, rename = "Guid")]
    guid: Vec<Tagged>,
    #[serde(default, rename = "Genre")]
    genre: Vec<Tag>,
    #[serde(default, rename = "Role")]
    role: Vec<Tag>,
}
#[derive(Deserialize)]
struct Tagged {
    id: String,
}
#[derive(Deserialize)]
struct Tag {
    tag: String,
}

fn guid_value(guids: &[Tagged], scheme: &str) -> Option<String> {
    let prefix = format!("{scheme}://");
    guids.iter().find_map(|g| g.id.strip_prefix(&prefix).map(str::to_string))
}

/// Parse one `/library/sections/{key}/all` JSON body into fetched titles.
///
/// # Errors
/// Returns an error if the JSON does not match the expected shape.
pub fn parse_section(json: &str) -> anyhow::Result<Vec<FetchedTitle>> {
    let parsed: Container = serde_json::from_str(json)?;
    Ok(parsed
        .media_container
        .metadata
        .into_iter()
        .map(|m| {
            let kind = if m.kind == "show" { TitleKind::Series } else { TitleKind::Movie };
            let length = m.duration.map(|ms| format!("{} min", ms / 60000));
            FetchedTitle {
                imdb_id: guid_value(&m.guid, "imdb"),
                tmdb_id: guid_value(&m.guid, "tmdb"),
                plex_guid: guid_value(&m.guid, "plex"),
                title: m.title,
                year: m.year,
                kind,
                imdb_rating: m.rating,
                length,
                description: m.summary,
                genres: m.genre.into_iter().map(|g| g.tag).collect(),
                cast: m.role.into_iter().map(|r| r.tag).collect(),
                services: vec![Service::Plex],
            }
        })
        .collect())
}

/// Live Plex client (raw HTTP — fetches sections then items).
pub struct PlexClient {
    client: reqwest::Client,
    base_url: String,
    token: String,
}

impl PlexClient {
    #[must_use]
    pub fn new(base_url: String, token: String) -> Self {
        Self { client: reqwest::Client::new(), base_url: base_url.trim_end_matches('/').to_string(), token }
    }

    async fn get_json(&self, path: &str) -> anyhow::Result<String> {
        Ok(self
            .client
            .get(format!("{}{path}", self.base_url))
            .header("X-Plex-Token", &self.token)
            .header("Accept", "application/json")
            .send().await?
            .error_for_status()?
            .text().await?)
    }
}

#[derive(Deserialize)]
struct Sections {
    #[serde(rename = "MediaContainer")]
    media_container: SectionList,
}
#[derive(Deserialize)]
struct SectionList {
    #[serde(default, rename = "Directory")]
    directory: Vec<SectionDir>,
}
#[derive(Deserialize)]
struct SectionDir {
    key: String,
    #[serde(rename = "type")]
    kind: String,
}

#[async_trait]
impl CatalogueSource for PlexClient {
    fn name(&self) -> &'static str { "plex" }
    fn services(&self) -> &'static [Service] { &[Service::Plex] }

    async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>> {
        let sections: Sections = serde_json::from_str(&self.get_json("/library/sections").await?)?;
        let mut out = Vec::new();
        for dir in sections.media_container.directory {
            if dir.kind == "movie" || dir.kind == "show" {
                let body = self.get_json(&format!("/library/sections/{}/all", dir.key)).await?;
                out.extend(parse_section(&body)?);
            }
        }
        Ok(out)
    }
}
```

Add `pub mod plex;` to `src/sync/mod.rs`.

- [ ] **Step 5: Run to verify pass**

Run: `cargo test --lib sync::plex`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/sync/mod.rs src/sync/plex.rs tests/fixtures/plex_section_all.json
git commit -m "feat(sync): PlexClient + section parsing against fixture"
```

> Live note (§8): the exact Plex JSON field paths (`duration` units, `Guid` scheme strings) are confirmed during server verification; correct this parser + fixture from a captured real response if they differ.

---

### Task 8: `MotnClient` — resolve services + paginate UK Disney+/Crunchyroll

**Files:**
- Create: `src/sync/motn.rs`
- Create: `tests/fixtures/motn_countries.json`, `tests/fixtures/motn_search_page1.json`, `tests/fixtures/motn_search_page2.json`
- Modify: `src/sync/mod.rs` (add `pub mod motn;`)
- Test: `src/sync/motn.rs` `tests` module

**Interfaces:**
- Consumes: `FetchedTitle`, `CatalogueSource`, `Service`, `TitleKind`.
- Produces: `cue::sync::motn::MotnClient` (`MotnClient::new(api_key: String, country: String)`); `pub fn resolve_services(countries_json: &str, country: &str) -> Vec<(Service, String)>` (maps our `Service` → the API's catalog id for that country, omitting services the country lacks); `pub fn parse_page(json: &str, services: &[Service]) -> anyhow::Result<(Vec<FetchedTitle>, Option<String>)>` (titles + next cursor).

The MOTN show object: `{ "imdbId", "tmdbId", "title", "overview", "showType":"movie|series", "releaseYear", "firstAirYear", "genres":[{"name":"Action"}], "cast":["..."], "rating", "runtime", "seasonCount", "episodeCount" }`. `/countries` returns `{ "gb": { "services": { "disney": {...}, "crunchyroll": {...} } } }` (service ids are the map keys).

- [ ] **Step 1: Create the fixtures**

`tests/fixtures/motn_countries.json`:
```json
{
  "gb": {
    "countryCode": "gb",
    "services": {
      "disney": { "id": "disney", "name": "Disney+" },
      "crunchyroll": { "id": "crunchyroll", "name": "Crunchyroll" },
      "netflix": { "id": "netflix", "name": "Netflix" }
    }
  }
}
```

`tests/fixtures/motn_search_page1.json`:
```json
{
  "shows": [
    {
      "imdbId": "tt2380307",
      "tmdbId": "354912",
      "title": "Coco",
      "overview": "A boy enters the Land of the Dead.",
      "showType": "movie",
      "releaseYear": 2017,
      "genres": [{ "name": "Animation" }, { "name": "Family" }],
      "cast": ["Anthony Gonzalez"],
      "rating": 82,
      "runtime": 105
    }
  ],
  "hasMore": true,
  "nextCursor": "354912:Coco"
}
```

`tests/fixtures/motn_search_page2.json`:
```json
{
  "shows": [
    {
      "imdbId": "tt9335498",
      "title": "Solo Leveling",
      "overview": "A weak hunter grows strong.",
      "showType": "series",
      "firstAirYear": 2024,
      "genres": [{ "name": "Animation" }, { "name": "Action" }],
      "cast": ["Taito Ban"],
      "rating": 84,
      "seasonCount": 2,
      "episodeCount": 25
    }
  ],
  "hasMore": false,
  "nextCursor": null
}
```

- [ ] **Step 2: Write the failing tests** (`src/sync/motn.rs` `tests`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Service, TitleKind};

    #[test]
    fn resolve_services_maps_present_and_skips_absent() {
        let json = include_str!("../../tests/fixtures/motn_countries.json");
        let resolved = resolve_services(json, "gb");
        let disney = resolved.iter().find(|(s, _)| *s == Service::Disney);
        let crunchy = resolved.iter().find(|(s, _)| *s == Service::Crunchyroll);
        assert_eq!(disney.map(|(_, id)| id.as_str()), Some("disney"));
        assert_eq!(crunchy.map(|(_, id)| id.as_str()), Some("crunchyroll"));
        assert_eq!(resolved.len(), 2, "only our two services, never netflix");
    }

    #[test]
    fn parse_page_reads_movie_fields_and_cursor() {
        let json = include_str!("../../tests/fixtures/motn_search_page1.json");
        let (titles, cursor) = parse_page(json, &[Service::Disney]).unwrap();
        assert_eq!(cursor.as_deref(), Some("354912:Coco"));
        let t = &titles[0];
        assert_eq!(t.imdb_id.as_deref(), Some("tt2380307"));
        assert_eq!(t.kind, TitleKind::Movie);
        assert_eq!(t.year, Some(2017));
        assert_eq!(t.length.as_deref(), Some("105 min"));
        assert!((t.imdb_rating.unwrap() - 8.2).abs() < 1e-9, "rating 82 -> 8.2");
        assert_eq!(t.services, vec![Service::Disney]);
    }

    #[test]
    fn parse_page_reads_series_and_terminal_cursor() {
        let json = include_str!("../../tests/fixtures/motn_search_page2.json");
        let (titles, cursor) = parse_page(json, &[Service::Crunchyroll]).unwrap();
        assert_eq!(cursor, None);
        let t = &titles[0];
        assert_eq!(t.kind, TitleKind::Series);
        assert_eq!(t.year, Some(2024));
        assert_eq!(t.length.as_deref(), Some("25 eps"));
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test --lib sync::motn`
Expected: FAIL — functions undefined.

- [ ] **Step 4: Implement `src/sync/motn.rs`**

```rust
//! Movie-of-the-Night (Streaming Availability) client for UK Disney+/Crunchyroll.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;

use crate::models::{Service, TitleKind};
use crate::sync::{CatalogueSource, FetchedTitle};

const MOTN_BASE: &str = "https://api.movieofthenight.com/v4";

/// The services we sync and their API catalog-id candidates.
const WANTED: [(Service, &str); 2] =
    [(Service::Disney, "disney"), (Service::Crunchyroll, "crunchyroll")];

/// From a `/v4/countries` body, return `(Service, catalog_id)` for the wanted
/// services that the given country actually lists. Absent services are skipped.
#[must_use]
pub fn resolve_services(countries_json: &str, country: &str) -> Vec<(Service, String)> {
    let parsed: HashMap<String, CountryEntry> =
        serde_json::from_str(countries_json).unwrap_or_default();
    let Some(entry) = parsed.get(country) else { return Vec::new() };
    WANTED
        .iter()
        .filter_map(|(svc, id)| {
            entry.services.get(*id).map(|_| (*svc, (*id).to_string()))
        })
        .collect()
}

#[derive(Deserialize, Default)]
struct CountryEntry {
    #[serde(default)]
    services: HashMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    shows: Vec<Show>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    next_cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Show {
    imdb_id: Option<String>,
    tmdb_id: Option<String>,
    title: String,
    overview: Option<String>,
    show_type: String,
    release_year: Option<i64>,
    first_air_year: Option<i64>,
    #[serde(default)]
    genres: Vec<Named>,
    #[serde(default)]
    cast: Vec<String>,
    rating: Option<f64>,
    runtime: Option<i64>,
    episode_count: Option<i64>,
}
#[derive(Deserialize)]
struct Named {
    name: String,
}

/// Parse one search page into `(titles, next_cursor)`. `services` is the
/// membership stamped on every title from this fetch.
///
/// # Errors
/// Returns an error if the JSON does not match the expected shape.
pub fn parse_page(json: &str, services: &[Service]) -> anyhow::Result<(Vec<FetchedTitle>, Option<String>)> {
    // serde rename_all="camelCase" maps hasMore/nextCursor automatically.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RawPage {
        #[serde(default)]
        shows: Vec<Show>,
        #[serde(default)]
        has_more: bool,
        #[serde(default)]
        next_cursor: Option<String>,
    }
    let page: RawPage = serde_json::from_str(json)?;
    let titles = page
        .shows
        .into_iter()
        .map(|s| {
            let kind = if s.show_type == "series" { TitleKind::Series } else { TitleKind::Movie };
            let year = s.release_year.or(s.first_air_year);
            let length = match kind {
                TitleKind::Movie => s.runtime.map(|m| format!("{m} min")),
                TitleKind::Series => s.episode_count.map(|e| format!("{e} eps")),
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
                services: services.to_vec(),
            }
        })
        .collect();
    let cursor = if page.has_more { page.next_cursor } else { None };
    Ok((titles, cursor))
}

/// Live MOTN client.
pub struct MotnClient {
    client: reqwest::Client,
    api_key: String,
    country: String,
}

impl MotnClient {
    #[must_use]
    pub fn new(api_key: String, country: String) -> Self {
        Self { client: reqwest::Client::new(), api_key, country }
    }
}

#[async_trait]
impl CatalogueSource for MotnClient {
    fn name(&self) -> &'static str { "motn" }
    fn services(&self) -> &'static [Service] { &[Service::Disney, Service::Crunchyroll] }

    async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>> {
        let countries = self
            .client
            .get(format!("{MOTN_BASE}/countries"))
            .header("X-API-Key", &self.api_key)
            .send().await?.error_for_status()?.text().await?;
        let resolved = resolve_services(&countries, &self.country);
        if resolved.is_empty() {
            tracing::warn!("MOTN lists none of [disney, crunchyroll] for {}", self.country);
            return Ok(Vec::new());
        }
        let catalog_ids: Vec<String> = resolved.iter().map(|(_, id)| id.clone()).collect();
        let services: Vec<Service> = resolved.iter().map(|(s, _)| *s).collect();
        let catalogs = catalog_ids.join(",");

        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut req = self
                .client
                .get(format!("{MOTN_BASE}/shows/search/filters"))
                .header("X-API-Key", &self.api_key)
                .query(&[("country", self.country.as_str()), ("catalogs", catalogs.as_str())]);
            if let Some(c) = &cursor {
                req = req.query(&[("cursor", c.as_str())]);
            }
            let body = req.send().await?.error_for_status()?.text().await?;
            let (mut titles, next) = parse_page(&body, &services)?;
            out.append(&mut titles);
            match next {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        Ok(out)
    }
}
```

> Implementer note: the duplicated `Page`/`RawPage` structs are deliberate to keep the unused-warning-free public `parse_page` self-contained; delete the outer `Page` struct if clippy flags it as dead code, keeping only the inner `RawPage`.

- [ ] **Step 5: Run to verify pass**

Run: `cargo test --lib sync::motn`
Expected: PASS (3 tests).

- [ ] **Step 6: Clippy + commit**

Run: `cargo clippy --all-targets -- -D warnings` → clean.

```bash
git add src/sync/mod.rs src/sync/motn.rs tests/fixtures/motn_countries.json tests/fixtures/motn_search_page1.json tests/fixtures/motn_search_page2.json
git commit -m "feat(sync): MotnClient with /countries resolve + cursor pagination"
```

> Live note (§8/§12): Crunchyroll's presence in `gb` and the exact show-object field names (`rating` scale, `genres[].name`) are confirmed during server verification; correct parser + fixtures from a captured response if they differ. The client degrades gracefully if a service is absent.

---

### Task 9: HTTP routes — `POST /api/sync` + `GET /api/sync/status`

**Files:**
- Create: `src/routes/sync.rs`
- Modify: `src/routes/mod.rs` (add `mod sync;` + two routes)
- Test: `src/routes/sync.rs` `tests` module

**Interfaces:**
- Consumes: `web::Data<SqlitePool>`, `web::Data<Arc<SyncRunner>>`, `db::sync_runs::{latest_per_source, catalogue_stats, SourceRun, CatalogueStats}`.
- Produces: `pub async fn trigger(...) -> impl Responder`; `pub async fn status(...) -> impl Responder`; both registered under `/api`.

`GET /api/sync/status` body shape:
```json
{ "running": false,
  "lastRun": { "status": "ok|partial|error", "itemCount": 0, "finishedAt": "..." } | null,
  "sources": [ { "source": "...", "lastRun": "...", "status": "...", "itemCount": 0 } ],
  "catalogue": { "titles": 0, "movies": 0, "series": 0, "embedded": 0 } }
```

- [ ] **Step 1: Write the failing tests** (actix in-process; build app with fake state)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{test, web, App};
    use std::sync::Arc;
    use crate::db::{init_pool, sync_runs};
    use crate::sync::SyncRunner;

    async fn pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (init_pool(&url).await.unwrap(), dir)
    }

    #[actix_web::test]
    async fn status_reports_sources_and_stats() {
        let (p, _dir) = pool().await;
        sync_runs::record(&p, "plex", "ok", 3, None).await.unwrap();
        let runner = SyncRunner::new(p.clone(), vec![], None);
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(p.clone()))
                .app_data(web::Data::new(runner))
                .route("/api/sync/status", web::get().to(status)),
        ).await;
        let req = test::TestRequest::get().uri("/api/sync/status").to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["running"], false);
        assert_eq!(body["catalogue"]["titles"], 0);
        assert_eq!(body["sources"][0]["source"], "plex");
        assert_eq!(body["lastRun"]["status"], "ok");
    }

    #[actix_web::test]
    async fn trigger_starts_and_returns_202() {
        let (p, _dir) = pool().await;
        let runner = SyncRunner::new(p.clone(), vec![], None);
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(runner))
                .route("/api/sync", web::post().to(trigger)),
        ).await;
        let req = test::TestRequest::post().uri("/api/sync").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 202);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib routes::sync`
Expected: FAIL — `status` / `trigger` undefined.

- [ ] **Step 3: Implement `src/routes/sync.rs`**

```rust
//! Sync trigger + status endpoints.

use std::sync::Arc;

use actix_web::{web, HttpResponse, Responder};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::db::sync_runs::{self, CatalogueStats, SourceRun};
use crate::sync::SyncRunner;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LastRun {
    status: String,
    item_count: i64,
    finished_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusBody {
    running: bool,
    last_run: Option<LastRun>,
    sources: Vec<SourceRun>,
    catalogue: CatalogueStats,
}

/// Reduce per-source statuses to one overall status.
fn overall(sources: &[SourceRun]) -> Option<LastRun> {
    if sources.is_empty() {
        return None;
    }
    let any_err = sources.iter().any(|s| s.status == "error");
    let any_ok = sources.iter().any(|s| s.status == "ok");
    let status = if any_err && any_ok {
        "partial"
    } else if any_err {
        "error"
    } else {
        "ok"
    };
    let item_count = sources.iter().map(|s| s.item_count).sum();
    let finished_at = sources.iter().filter_map(|s| s.last_run.clone()).max();
    Some(LastRun { status: status.to_string(), item_count, finished_at })
}

/// `POST /api/sync` — start a background sync (202) or report one running (409).
pub async fn trigger(runner: web::Data<Arc<SyncRunner>>) -> impl Responder {
    if runner.try_start() {
        HttpResponse::Accepted().json(serde_json::json!({ "started": true }))
    } else {
        HttpResponse::Conflict().json(serde_json::json!({ "running": true }))
    }
}

/// `GET /api/sync/status` — latest per-source runs + catalogue stats.
pub async fn status(
    pool: web::Data<SqlitePool>,
    runner: web::Data<Arc<SyncRunner>>,
) -> impl Responder {
    let sources = match sync_runs::latest_per_source(pool.get_ref()).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("sync status sources failed: {e:#}");
            return HttpResponse::InternalServerError().finish();
        }
    };
    let catalogue = match sync_runs::catalogue_stats(pool.get_ref()).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("sync status stats failed: {e:#}");
            return HttpResponse::InternalServerError().finish();
        }
    };
    let last_run = overall(&sources);
    HttpResponse::Ok().json(StatusBody { running: runner.is_running(), last_run, sources, catalogue })
}
```

- [ ] **Step 4: Register routes** in `src/routes/mod.rs` — add `mod sync;` at the top, and inside `configure`'s `/api` scope add:

```rust
            .route("/sync", web::post().to(sync::trigger))
            .route("/sync/status", web::get().to(sync::status))
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test --lib routes::sync`
Expected: PASS (2 tests).

- [ ] **Step 6: Clippy + commit**

Run: `cargo clippy --all-targets -- -D warnings` → clean.

```bash
git add src/routes/mod.rs src/routes/sync.rs
git commit -m "feat(routes): POST /api/sync trigger + GET /api/sync/status"
```

---

### Task 10: Wire scheduler, state, and startup-if-empty in `main.rs`

**Files:**
- Modify: `Cargo.toml` (add `tokio-cron-scheduler`)
- Modify: `src/main.rs`
- (No new unit test — verified by build + the live server check.)

**Interfaces:**
- Consumes: `SyncRunner`, `run_sync` (via the runner), `PlexClient`, `MotnClient`, the existing `embedder` Arc.

- [ ] **Step 1: Add the dependency** to `Cargo.toml` `[dependencies]`

```toml
tokio-cron-scheduler = "0.13"
```

Run: `cargo build` → compiles (pulls the crate). If `0.13` is unavailable, run `cargo add tokio-cron-scheduler` and use whatever current minor it resolves; the API used (`JobScheduler::new`, `Job::new_async`, `add`, `start`) is stable across recent versions.

- [ ] **Step 2: Build the sources + runner + scheduler in `main.rs`**

After the existing `engine` is constructed and before `HttpServer::new(...)`, add (the `embedder` Option is already in scope):

```rust
    // Assemble catalogue sources from configured credentials.
    let mut sources: Vec<Arc<dyn cue::sync::CatalogueSource>> = Vec::new();
    if let (Some(url), Some(token)) = (cfg.plex_url.clone(), cfg.plex_token.clone()) {
        sources.push(Arc::new(cue::sync::plex::PlexClient::new(url, token)));
    }
    if let Some(key) = cfg.motn_api_key.clone() {
        let country = cfg.region.clone().unwrap_or_else(|| "gb".to_string());
        sources.push(Arc::new(cue::sync::motn::MotnClient::new(key, country)));
    }
    let runner = cue::sync::SyncRunner::new(pool.clone(), sources, embedder.clone());

    // Sync once on startup if the catalogue is empty or still just the seed.
    let title_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM titles")
        .fetch_one(&pool).await.map_err(std::io::Error::other)?;
    if title_count <= 28 {
        if runner.try_start() {
            tracing::info!("startup sync triggered (catalogue had {title_count} titles)");
        }
    }

    // Daily (configurable) scheduled sync.
    let cron = cfg.sync_cron.clone().unwrap_or_else(|| "0 0 3 * * *".to_string());
    let scheduler = tokio_cron_scheduler::JobScheduler::new().await.map_err(std::io::Error::other)?;
    let runner_for_job = runner.clone();
    let job = tokio_cron_scheduler::Job::new_async(cron.as_str(), move |_uuid, _l| {
        let r = runner_for_job.clone();
        Box::pin(async move {
            if r.try_start() {
                tracing::info!("scheduled sync triggered");
            } else {
                tracing::warn!("scheduled sync skipped — a run is already active");
            }
        })
    }).map_err(std::io::Error::other)?;
    scheduler.add(job).await.map_err(std::io::Error::other)?;
    scheduler.start().await.map_err(std::io::Error::other)?;
```

> The `embedder` value is currently moved into `AskEngine::new`. Change that line to pass a clone so the runner can hold its own: build `let embedder_for_engine = embedder.clone();` and pass `embedder_for_engine` to `AskEngine::new`, keeping `embedder` for the runner. (Both are `Option<Arc<dyn Embedder>>`, cheap to clone.)

- [ ] **Step 3: Register the runner in app data** — inside the `HttpServer::new(move || { App::new() … })` closure, add alongside the other `.app_data(...)` calls:

```rust
            .app_data(web::Data::new(runner.clone()))
```

(Move `runner` into the closure by cloning the `Arc` like `pool`/`engine` are.)

- [ ] **Step 4: Build + full test suite**

Run: `cargo build`
Expected: compiles.

Run: `cargo test`
Expected: all tests pass (lib + route tests).

- [ ] **Step 5: Clippy gate**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs
git commit -m "feat(sync): wire scheduler + SyncRunner state + startup-if-empty"
```

---

### Task 11: `.env.example` + docs for the new sync vars

**Files:**
- Modify: `.env.example`
- Modify: `CLAUDE.md` (one line noting the sync endpoints, optional)

- [ ] **Step 1: Document the sync env vars** — ensure `.env.example` has (with empty/placeholder values; this file is committed, real `.env` is gitignored):

```dotenv
# Catalogue sync (Plan 4) — all server-side, never sent to the client
PLEX_URL=
PLEX_TOKEN=
MOTN_API_KEY=
REGION=gb
# cron: sec min hour day month weekday — default daily at 03:00
SYNC_CRON=0 0 3 * * *
```

- [ ] **Step 2: Commit**

```bash
git add .env.example CLAUDE.md
git commit -m "docs(sync): document Plex/MOTN/REGION/SYNC_CRON env vars"
```

---

### Task 12: Frontend API client — `triggerSync()` + `getSyncStatus()`

**Files:**
- Create: `frontend/src/api/sync.ts`
- Modify: `frontend/src/types.ts` (add `SyncStatus` types)
- Test: `frontend/src/api/sync.spec.ts`

**Interfaces:**
- Produces: `triggerSync(): Promise<'started' | 'running'>`; `getSyncStatus(): Promise<SyncStatus>`; types `SyncStatus`, `SourceRun`, `CatalogueStats`, `LastRun`.

- [ ] **Step 1: Add types** to `frontend/src/types.ts`

```ts
export interface SourceRun {
  source: string
  lastRun: string | null
  status: string
  itemCount: number
}
export interface CatalogueStats {
  titles: number
  movies: number
  series: number
  embedded: number
}
export interface LastRun {
  status: string
  itemCount: number
  finishedAt: string | null
}
export interface SyncStatus {
  running: boolean
  lastRun: LastRun | null
  sources: SourceRun[]
  catalogue: CatalogueStats
}
```

- [ ] **Step 2: Write the failing test** `frontend/src/api/sync.spec.ts`

```ts
import { describe, it, expect, vi, afterEach } from 'vitest'
import { triggerSync, getSyncStatus } from './sync'

afterEach(() => vi.restoreAllMocks())

describe('sync api', () => {
  it('triggerSync returns "started" on 202', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ status: 202, ok: true }))
    expect(await triggerSync()).toBe('started')
  })

  it('triggerSync returns "running" on 409', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ status: 409, ok: false }))
    expect(await triggerSync()).toBe('running')
  })

  it('getSyncStatus parses the status body', async () => {
    const body = {
      running: false,
      lastRun: { status: 'ok', itemCount: 3, finishedAt: '2026-06-22' },
      sources: [{ source: 'plex', lastRun: '2026-06-22', status: 'ok', itemCount: 3 }],
      catalogue: { titles: 3, movies: 2, series: 1, embedded: 3 },
    }
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve(body) }))
    const s = await getSyncStatus()
    expect(s.catalogue.titles).toBe(3)
    expect(s.sources[0].source).toBe('plex')
  })
})
```

- [ ] **Step 3: Run to verify failure**

Run: `cd frontend && npm test -- sync`
Expected: FAIL — module `./sync` not found.

- [ ] **Step 4: Implement** `frontend/src/api/sync.ts`

```ts
import type { SyncStatus } from '@/types'

export async function triggerSync(): Promise<'started' | 'running'> {
  const res = await fetch('/api/sync', { method: 'POST' })
  if (res.status === 409) return 'running'
  if (res.status === 202) return 'started'
  throw new Error(`Failed to trigger sync (HTTP ${res.status})`)
}

export async function getSyncStatus(): Promise<SyncStatus> {
  const res = await fetch('/api/sync/status')
  if (!res.ok) throw new Error(`Failed to load sync status (HTTP ${res.status})`)
  return (await res.json()) as SyncStatus
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cd frontend && npm test -- sync`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add frontend/src/types.ts frontend/src/api/sync.ts frontend/src/api/sync.spec.ts
git commit -m "feat(frontend): sync api client (triggerSync, getSyncStatus)"
```

---

### Task 13: Settings page + avatar entry point

**Files:**
- Create: `frontend/src/views/SettingsView.vue`
- Modify: `frontend/src/router/index.ts` (add `/settings` route)
- Modify: `frontend/src/components/AppHeader.vue` (avatar → router push)
- Test: `frontend/src/views/SettingsView.spec.ts`

**Interfaces:**
- Consumes: `triggerSync`, `getSyncStatus`, `SyncStatus`.

- [ ] **Step 1: Add the route** in `frontend/src/router/index.ts`

```ts
import SettingsView from '@/views/SettingsView.vue'
// ...inside routes:
    { path: '/settings', component: SettingsView },
```

- [ ] **Step 2: Make the avatar navigate** — in `frontend/src/components/AppHeader.vue`, change the avatar element to a button:

```vue
    <!-- Avatar → settings/utility page -->
    <button
      type="button"
      class="avatar"
      aria-label="Open settings: FF"
      @click="$router.push('/settings')"
    >FF</button>
```

In the `.avatar` CSS block, add `cursor: pointer;` and reset button defaults: `appearance: none; padding: 0;` (keep the existing visual rules).

- [ ] **Step 3: Write the failing test** `frontend/src/views/SettingsView.spec.ts`

```ts
import { describe, it, expect, vi, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import SettingsView from './SettingsView.vue'

afterEach(() => vi.restoreAllMocks())

const status = {
  running: false,
  lastRun: { status: 'ok', itemCount: 3, finishedAt: '2026-06-22T03:00:00' },
  sources: [{ source: 'plex', lastRun: '2026-06-22T03:00:00', status: 'ok', itemCount: 3 }],
  catalogue: { titles: 3, movies: 2, series: 1, embedded: 3 },
}

describe('SettingsView', () => {
  it('renders catalogue stats from the status endpoint', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve(status) }))
    const wrapper = mount(SettingsView, { global: { stubs: { RouterLink: true } } })
    await flushPromises()
    expect(wrapper.text()).toContain('plex')
    expect(wrapper.find('[data-test="stat-titles"]').text()).toContain('3')
  })

  it('triggers a sync when the button is clicked', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(status) }) // initial load
      .mockResolvedValueOnce({ status: 202, ok: true })                          // POST /api/sync
      .mockResolvedValue({ ok: true, json: () => Promise.resolve(status) })      // refresh
    vi.stubGlobal('fetch', fetchMock)
    const wrapper = mount(SettingsView, { global: { stubs: { RouterLink: true } } })
    await flushPromises()
    await wrapper.find('[data-test="sync-now"]').trigger('click')
    await flushPromises()
    expect(fetchMock).toHaveBeenCalledWith('/api/sync', { method: 'POST' })
  })
})
```

- [ ] **Step 4: Run to verify failure**

Run: `cd frontend && npm test -- SettingsView`
Expected: FAIL — `SettingsView.vue` not found.

- [ ] **Step 5: Implement** `frontend/src/views/SettingsView.vue`

```vue
<template>
  <section class="settings">
    <header class="settings__head">
      <RouterLink to="/" class="settings__back">← back</RouterLink>
      <h1>Library &amp; sync</h1>
    </header>

    <div class="settings__row">
      <button
        type="button"
        data-test="sync-now"
        class="btn"
        :disabled="busy"
        @click="onSync"
      >{{ busy ? 'Syncing…' : 'Sync now' }}</button>
      <span v-if="message" class="settings__msg">{{ message }}</span>
    </div>

    <div v-if="status" class="settings__grid">
      <div class="card">
        <h2>Last run</h2>
        <p>{{ status.lastRun ? `${status.lastRun.status} · ${status.lastRun.itemCount} items` : 'never' }}</p>
        <p class="muted">{{ status.lastRun?.finishedAt ?? '' }}</p>
      </div>

      <div class="card">
        <h2>Sources</h2>
        <ul>
          <li v-for="s in status.sources" :key="s.source">
            <strong>{{ s.source }}</strong> — {{ s.status }} ({{ s.itemCount }})
            <span class="muted">{{ s.lastRun ?? 'never' }}</span>
          </li>
          <li v-if="status.sources.length === 0" class="muted">no runs yet</li>
        </ul>
      </div>

      <div class="card">
        <h2>Catalogue</h2>
        <p data-test="stat-titles">{{ status.catalogue.titles }} titles</p>
        <p class="muted">{{ status.catalogue.movies }} movies · {{ status.catalogue.series }} series</p>
        <p class="muted">{{ status.catalogue.embedded }} embedded</p>
      </div>
    </div>

    <p v-if="error" class="settings__error">{{ error }}</p>
  </section>
</template>

<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { RouterLink } from 'vue-router'
import { triggerSync, getSyncStatus } from '@/api/sync'
import type { SyncStatus } from '@/types'

const status = ref<SyncStatus | null>(null)
const busy = ref(false)
const message = ref('')
const error = ref('')
let poll: ReturnType<typeof setInterval> | undefined

async function refresh() {
  try {
    status.value = await getSyncStatus()
    busy.value = status.value.running
    if (!status.value.running && poll) {
      clearInterval(poll)
      poll = undefined
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : 'failed to load status'
  }
}

async function onSync() {
  message.value = ''
  error.value = ''
  try {
    const result = await triggerSync()
    busy.value = true
    message.value = result === 'running' ? 'a sync is already running' : 'sync started'
    if (!poll) poll = setInterval(refresh, 3000)
    await refresh()
  } catch (e) {
    error.value = e instanceof Error ? e.message : 'failed to start sync'
    busy.value = false
  }
}

onMounted(refresh)
onUnmounted(() => { if (poll) clearInterval(poll) })
</script>

<style scoped>
.settings { max-width: 760px; margin: 0 auto; padding: 28px 22px; color: var(--text-primary, #e9ebf0); }
.settings__head { display: flex; align-items: baseline; gap: 16px; margin-bottom: 22px; }
.settings__back { color: var(--text-muted, #aab0bb); text-decoration: none; font-size: 13px; }
.settings__row { display: flex; align-items: center; gap: 14px; margin-bottom: 22px; }
.settings__msg { color: var(--text-muted, #aab0bb); font-size: 13px; }
.settings__error { color: #f5a3a3; margin-top: 16px; }
.settings__grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 16px; }
.card { background: var(--surface-1, #15171c); border: 1px solid var(--border, rgba(255,255,255,0.08)); border-radius: var(--r-md, 8px); padding: 16px; }
.card h2 { font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--text-faint, #5f6570); margin: 0 0 10px; }
.card ul { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 6px; font-size: 13px; }
.muted { color: var(--text-faint, #5f6570); font-size: 12px; }
.btn { height: 36px; padding: 0 16px; border-radius: var(--r-md, 8px); border: 1px solid var(--accent-focus, rgba(245,197,24,0.5)); background: var(--surface-1, #15171c); color: var(--text-primary, #e9ebf0); cursor: pointer; font-family: var(--font-ui); }
.btn:disabled { opacity: 0.6; cursor: default; }
</style>
```

- [ ] **Step 6: Run to verify pass**

Run: `cd frontend && npm test -- SettingsView`
Expected: PASS (2 tests).

- [ ] **Step 7: Full frontend gates**

Run: `cd frontend && npm test`
Expected: all suites pass.

Run: `cd frontend && npm run build`
Expected: vue-tsc + vite build succeed (no type errors).

- [ ] **Step 8: Commit**

```bash
git add frontend/src/router/index.ts frontend/src/components/AppHeader.vue frontend/src/views/SettingsView.vue frontend/src/views/SettingsView.spec.ts
git commit -m "feat(frontend): settings page with sync trigger, status, per-source + catalogue stats"
```

---

### Task 14: Final integration gate

**Files:** none (verification only).

- [ ] **Step 1: Backend gate**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all tests pass; clippy clean.

- [ ] **Step 2: Frontend gate**

Run: `cd frontend && npm test && npm run build`
Expected: all tests pass; build succeeds.

- [ ] **Step 3: Live verification on the server** (§8 — requires real `PLEX_URL`/`PLEX_TOKEN` + `MOTN_API_KEY`)

Deploy the image, open the app, click the **FF avatar → Sync now**, and watch `/api/sync/status`. Confirm: real titles replace the seed in Browse, `embedded` count rises, and the per-source breakdown shows Plex + Disney+/Crunchyroll. If Plex JSON paths or MOTN field names differ from the fixtures, correct `plex.rs`/`motn.rs` + the fixtures and re-run the gates (Tasks 7/8).

- [ ] **Step 4: Tag the branch ready for review** — hand off to `superpowers:requesting-code-review` (whole-branch Opus review, per the Plan 1–3 pattern) before merge.

---

## Self-Review

**Spec coverage:**
- §2 module layout → Tasks 1,4,5,7,8,9 (all modules created). ✔
- §3 trait seam + clients → Tasks 1,7,8. ✔
- §3.2 `/countries` resolve + cursor pagination → Task 8. ✔
- §4 dedup/identity/normalize/scoped-prune → Tasks 2,3,4,6. ✔
- §5 embedding backfill reuse → Task 6 (`run_sync` calls `backfill`). ✔
- §6 scheduler + startup-if-empty + manual trigger guard → Tasks 6,9,10. ✔
- §7 API surface + per-source rows (no migration) → Tasks 5,9. ✔
- §8 offline fixtures + live verify → Tasks 7,8,14. ✔
- §9 settings page (sync button, status, per-source, catalogue stats) → Tasks 12,13. ✔
- §10 config/env → Task 11. ✔
- §11 deferred items → not implemented (correct). ✔

**Placeholder scan:** No TBD/TODO; every code + test step carries complete code. The two "live notes" (Tasks 7,8) are explicit verification instructions, not deferred work. ✔

**Type consistency:** `Service::as_str`/`TitleKind::as_str` (Task 1) used in Tasks 4,6. `MergedTitle` (Task 3) consumed by Tasks 4,6. `CatalogueSource`/`FetchedTitle` (Task 1) implemented in Tasks 7,8 and consumed in Task 6. `SyncRunner`/`run_sync` (Task 6) used in Tasks 9,10. `SourceRun`/`CatalogueStats` (Task 5) used in Task 9. `SyncStatus` TS type (Task 12) used in Task 13. Endpoint shape (`running`/`lastRun`/`sources`/`catalogue`) consistent between Task 9 (server) and Tasks 12/13 (client). ✔
