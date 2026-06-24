# Watch-at-source Deep Links Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a per-source "Watch on…" affordance to the title detail page that opens the title's page on the user's self-hosted Plex web UI or on Crunchyroll/Disney+ in a new tab, via a backend 302 redirect that keeps real URLs out of the client.

**Architecture:** Sync captures the Plex per-item `ratingKey` + the server `machineIdentifier`, and the MOTN per-service `link`. These are stored (new `titles.plex_rating_key`, `title_services.link`, `app_meta` kv). A unified `GET /api/titles/{id}/watch/{service}` endpoint resolves and 302-redirects to the real URL server-side. The detail payload exposes only a URL-free `watchable: string[]`; the frontend renders one branded button per entry.

**Tech Stack:** Rust + Actix-web + SQLx (runtime queries) + SQLite; Vue 3 + TypeScript + Vitest.

## Global Constraints

- SQLx **runtime** queries only (`sqlx::query`/`query_as`/`query_scalar`) — never the `query!` macros. (CLAUDE.md)
- After adding a migration file, run `cargo clean -p cue` before `cargo test` so `sqlx::migrate!` re-embeds it. (CLAUDE.md)
- Test DBs: `tempfile::tempdir()`, URL `format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"))`, keep the `TempDir` guard bound. (CLAUDE.md)
- Modules are declared in `src/lib.rs`; `main.rs` is thin. New `db`/`routes` submodules are declared in their parent `mod.rs`.
- Clippy gate: `cargo clippy --all-targets -- -D warnings`; per-item `#[allow]` with a one-line reason only, never widen the global table.
- Security boundary: external infra (Plex URL/token, `machineIdentifier`, MOTN link strings) is server-side only — NEVER serialized to the client. The detail payload exposes availability (`watchable`), never URLs.
- Git commits via the **Bash tool** (PowerShell prepends a BOM to commit subjects). Service enum string values are exactly `plex` / `disney` / `crunchyroll`.
- `PLEX_WEB_URL` defaults to `PLEX_URL`; the deployment uses them identically.

---

## File Structure

**Backend — create:**
- `migrations/0010_watch_links.sql` — `title_services.link`, `titles.plex_rating_key`, `app_meta` table.
- `src/db/app_meta.rs` — `get`/`set` for the kv table.
- `src/routes/watch.rs` — the `/watch/{service}` redirect handler + URL builders.

**Backend — modify:**
- `src/sync/mod.rs` — `FetchedTitle` gains `plex_rating_key` + `links`; `CatalogueSource::server_meta` default method; `run_sync` threads links into reconcile and writes `app_meta`.
- `src/sync/merge.rs` — `MergedTitle` gains the two fields; `merge()` unions them.
- `src/sync/store.rs` — `upsert_title` binds `plex_rating_key`; `reconcile_service` takes `&[(i64, Option<String>)]` and upserts `link`.
- `src/sync/plex.rs` — parse `ratingKey`; implement `server_meta` (`/identity`).
- `src/sync/motn.rs` — parse `StreamOption.link`; attribute per-service links.
- `src/db/motn_cache.rs` — `CachedTitle` round-trips `links`.
- `src/db/mod.rs` — declare `pub mod app_meta;`.
- `src/db/catalogue.rs` — `fetch_title` derives `watchable`.
- `src/models.rs` — `TitleDto` gains `watchable`.
- `src/config.rs` — `plex_web_url` field.
- `src/routes/mod.rs` — declare `pub mod watch;` + register the route.
- `src/main.rs` — build `WatchConfig` app_data.

**Frontend — create:**
- `frontend/src/components/WatchLinks.vue` — the button block.
- `frontend/src/components/__tests__/WatchLinks.test.ts`.

**Frontend — modify:**
- `frontend/src/types.ts` — `TitleDetail.watchable`.
- `frontend/src/api/client.ts` — validate `watchable`.
- `frontend/src/views/DetailView.vue` — mount `WatchLinks` in the action column.

---

## Task 1: Migration + `app_meta` kv module

**Files:**
- Create: `migrations/0010_watch_links.sql`
- Create: `src/db/app_meta.rs`
- Modify: `src/db/mod.rs:1-6`

**Interfaces:**
- Produces: `cue::db::app_meta::get(pool: &SqlitePool, key: &str) -> anyhow::Result<Option<String>>` and `set(pool: &SqlitePool, key: &str, value: &str) -> anyhow::Result<()>`.
- Produces schema: `title_services.link TEXT`, `titles.plex_rating_key TEXT`, `app_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL)`.

- [ ] **Step 1: Write the migration**

Create `migrations/0010_watch_links.sql`:

```sql
-- Watch-at-source deep links: per-service MOTN link, Plex per-item ratingKey,
-- and a kv table for the server-global Plex machineIdentifier.
ALTER TABLE title_services ADD COLUMN link TEXT;
ALTER TABLE titles ADD COLUMN plex_rating_key TEXT;

CREATE TABLE app_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

- [ ] **Step 2: Declare the module**

In `src/db/mod.rs`, add to the module list (keep alphabetical-ish with the others):

```rust
pub mod app_meta;
```

- [ ] **Step 3: Write `src/db/app_meta.rs` with a failing test**

```rust
//! Tiny key/value store for server-global, runtime-discovered singletons
//! (currently just the Plex `machineIdentifier` used to build watch links).

use sqlx::SqlitePool;

/// Read one value by key, or `None` if absent.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn get(pool: &SqlitePool, key: &str) -> anyhow::Result<Option<String>> {
    let v = sqlx::query_scalar::<_, String>("SELECT value FROM app_meta WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(v)
}

/// Upsert one key/value pair.
///
/// # Errors
/// Returns an error if the write fails.
pub async fn set(pool: &SqlitePool, key: &str, value: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO app_meta (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_pool;

    async fn pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (init_pool(&url).await.unwrap(), dir)
    }

    #[tokio::test]
    async fn set_then_get_roundtrips_and_upserts() {
        let (p, _dir) = pool().await;
        assert_eq!(get(&p, "plex_machine_id").await.unwrap(), None);
        set(&p, "plex_machine_id", "ABC123").await.unwrap();
        assert_eq!(get(&p, "plex_machine_id").await.unwrap(), Some("ABC123".into()));
        set(&p, "plex_machine_id", "DEF456").await.unwrap();
        assert_eq!(get(&p, "plex_machine_id").await.unwrap(), Some("DEF456".into()));
    }
}
```

- [ ] **Step 4: Force migration re-embed, then run the test (expect FAIL → PASS)**

Run:
```bash
cargo clean -p cue && cargo test --lib db::app_meta -- --nocapture
```
Expected: compiles and the test PASSES (the migration creates `app_meta`). If `init_pool` does not run migrations, confirm it calls `sqlx::migrate!`.

- [ ] **Step 5: Commit**

```bash
git add migrations/0010_watch_links.sql src/db/app_meta.rs src/db/mod.rs
git commit -F - <<'EOF'
feat(db): watch-link schema + app_meta kv store

migration 0010 adds title_services.link, titles.plex_rating_key, and an
app_meta(key,value) table; db::app_meta get/set upserts the kv pairs.
EOF
```

---

## Task 2: Extend `FetchedTitle`/`MergedTitle`, merge union, cache round-trip

**Files:**
- Modify: `src/sync/mod.rs:42-58` (struct), construction sites `:269`, `:319`
- Modify: `src/sync/merge.rs:43-58` (struct), `:133` (merge output), `:307` (test helper), merge body
- Modify: `src/sync/store.rs:267`, `:475` (test helpers — add fields)
- Modify: `src/db/motn_cache.rs` (CachedTitle + From + into_fetched + `:193` sample)

**Interfaces:**
- Produces: `FetchedTitle.plex_rating_key: Option<String>`, `FetchedTitle.links: Vec<(Service, String)>`; same two fields on `MergedTitle`. `merge()` unions `links` (first link per service wins) and fills `plex_rating_key` from the first row that has it.

- [ ] **Step 1: Add fields to `FetchedTitle`**

In `src/sync/mod.rs`, in `pub struct FetchedTitle` (after `pub services: Vec<Service>,`):

```rust
    /// Plex per-item ratingKey (Plex source only); used to build a Plex web link.
    pub plex_rating_key: Option<String>,
    /// Per-service "watch here" web links (MOTN `link`); empty for Plex.
    pub links: Vec<(Service, String)>,
```

- [ ] **Step 2: Add the same fields to `MergedTitle`**

In `src/sync/merge.rs`, in `pub struct MergedTitle` (after `pub services: Vec<Service>,`):

```rust
    pub plex_rating_key: Option<String>,
    pub links: Vec<(Service, String)>,
```

- [ ] **Step 3: Initialize the fields in `merge()` output and union them**

In `src/sync/merge.rs`, the `MergedTitle { … }` literal (around `:133`) — its construction is from the first row seen for a key. Add to that literal:

```rust
                    plex_rating_key: f.plex_rating_key.clone(),
                    links: f.links.clone(),
```

Then in the `if let Some(existing) = by_key.get_mut(&key)` merge branch (where `services`/`cast` are unioned and scalars filled with `.or()`), add:

```rust
            existing.plex_rating_key = existing.plex_rating_key.take().or(f.plex_rating_key);
            for (svc, link) in f.links {
                if !existing.links.iter().any(|(s, _)| *s == svc) {
                    existing.links.push((svc, link));
                }
            }
```

> Note: `f` is consumed in the merge branch; if `f.plex_rating_key`/`f.links` were already moved by the `MergedTitle { … }` literal in the same scope, use the branch ordering already present (literal runs only for the *first* occurrence via `by_key.entry`/insert; the `get_mut` branch is the subsequent-occurrence path — they are mutually exclusive, so both may move from `f`).

- [ ] **Step 4: Update every other construction site to compile**

The compiler will flag each `FetchedTitle { … }` / `MergedTitle { … }` missing the new fields. Add `plex_rating_key: None,` and `links: vec![],` (or `Vec::new()`) to each:
- `src/sync/mod.rs:269` (test) and `:319` (`title()` helper)
- `src/sync/merge.rs:307` (test helper `fetched(...)`)
- `src/sync/store.rs:267` (`merged()` helper) and `:475` (test `MergedTitle`)
- `src/sync/plex.rs:101` and `src/sync/motn.rs:195` — add `plex_rating_key: None,` and `links: vec![],` for now (populated in Tasks 4/5).
- `src/db/motn_cache.rs:67` (`into_fetched`) and `:193` (`sample`) — handled in Step 6.

- [ ] **Step 5: Write a failing merge test for link/ratingKey union**

In `src/sync/merge.rs` `#[cfg(test)] mod tests`, add (adapt the existing `fetched(...)` helper — it now accepts the new fields as `None`/`vec![]`; build rows inline):

```rust
    #[test]
    fn merge_unions_links_and_fills_rating_key() {
        use crate::models::{Service, TitleKind};
        let plex = FetchedTitle {
            imdb_id: Some("tt9".into()), tmdb_id: None, plex_guid: None,
            title: "X".into(), year: Some(2020), kind: TitleKind::Movie,
            score: None, length: None, description: None, genres: vec![], cast: vec![],
            services: vec![Service::Plex],
            plex_rating_key: Some("777".into()), links: vec![],
            poster: None, backdrop: None,
        };
        let motn = FetchedTitle {
            imdb_id: Some("tt9".into()), tmdb_id: None, plex_guid: None,
            title: "X".into(), year: Some(2020), kind: TitleKind::Movie,
            score: None, length: None, description: None, genres: vec![], cast: vec![],
            services: vec![Service::Crunchyroll],
            plex_rating_key: None,
            links: vec![(Service::Crunchyroll, "https://crunchyroll.com/x".into())],
            poster: None, backdrop: None,
        };
        let out = merge(vec![plex, motn]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].plex_rating_key.as_deref(), Some("777"));
        assert_eq!(out[0].links, vec![(Service::Crunchyroll, "https://crunchyroll.com/x".into())]);
    }
```

- [ ] **Step 6: Round-trip `links` through `CachedTitle`**

In `src/db/motn_cache.rs`, add to `struct CachedTitle` (after `pub services: Vec<String>,`):

```rust
    /// Per-service watch links, stored as `(service_str, url)` so the JSON is
    /// enum-repr-independent. `#[serde(default)]` keeps pre-existing cached rows
    /// (written before this field) deserializable.
    #[serde(default)]
    pub links: Vec<(String, String)>,
```

In `impl From<&FetchedTitle> for CachedTitle`, add to the literal:

```rust
            links: t.links.iter().map(|(s, l)| (s.as_str().to_string(), l.clone())).collect(),
```

In `into_fetched`, add to the `FetchedTitle` literal:

```rust
            plex_rating_key: None,
            links: self
                .links
                .iter()
                .filter_map(|(s, l)| Service::parse(s).map(|svc| (svc, l.clone())))
                .collect(),
```

In the `sample()` test helper (`:193`), add `plex_rating_key: None,` and `links: vec![],` (or a sample link for a round-trip assertion).

- [ ] **Step 7: Write a failing cache round-trip test**

In `src/db/motn_cache.rs` tests, add:

```rust
    #[test]
    fn cached_title_roundtrips_links() {
        use crate::models::Service;
        let mut ft = sample();
        ft.links = vec![(Service::Crunchyroll, "https://crunchyroll.com/x".into())];
        let cached = CachedTitle::from(&ft);
        let back = cached.into_fetched();
        assert_eq!(back.links, vec![(Service::Crunchyroll, "https://crunchyroll.com/x".into())]);
    }
```

- [ ] **Step 8: Run tests (expect PASS)**

Run:
```bash
cargo test --lib sync::merge db::motn_cache
```
Expected: PASS. Fix any remaining missing-field compile errors flagged by the compiler.

- [ ] **Step 9: Commit**

```bash
git add src/sync/mod.rs src/sync/merge.rs src/sync/store.rs src/sync/plex.rs src/sync/motn.rs src/db/motn_cache.rs
git commit -F - <<'EOF'
feat(sync): carry plex_rating_key + per-service links through merge/cache

FetchedTitle/MergedTitle gain plex_rating_key and links; merge unions links
(first per service wins) and fills rating_key; CachedTitle round-trips links
so delta-synced MOTN titles keep them.
EOF
```

---

## Task 3: Persist `plex_rating_key` + per-service `link` in store

**Files:**
- Modify: `src/sync/store.rs` (`upsert_title` SQL, `reconcile_service` signature/body, test callers `:323-392`, `:475`)

**Interfaces:**
- Consumes: `MergedTitle.plex_rating_key`, `MergedTitle.links` (Task 2).
- Produces: `reconcile_service(pool, service, desired: &[(i64, Option<String>)])` — upserts `title_services(title_id, service, link)`, deletes stale ids. `upsert_title` writes `plex_rating_key`.

- [ ] **Step 1: Add `plex_rating_key` to both `upsert_title` SQL statements**

In `src/sync/store.rs::upsert_title`, the UPDATE: add `plex_rating_key = ?,` to the SET list (e.g. after `description = ?,`) and add `.bind(&t.plex_rating_key)` in the matching position (after `.bind(&t.description)`).

For the INSERT: add `plex_rating_key` to the column list and one more `?` to VALUES, and the matching `.bind(&t.plex_rating_key)` after `.bind(&t.description)`. Resulting INSERT columns:

```sql
INSERT INTO titles (imdb_id, tmdb_id, plex_guid, title, year, type, score, length, description, plex_rating_key, poster_url, poster_plex, backdrop_url, backdrop_plex)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id
```
(Bind order must match: …`.bind(&t.description).bind(&t.plex_rating_key).bind(poster_url)`…)

- [ ] **Step 2: Change `reconcile_service` to carry links**

Replace the signature and body of `reconcile_service`:

```rust
pub async fn reconcile_service(
    pool: &SqlitePool,
    service: Service,
    desired: &[(i64, Option<String>)],
) -> anyhow::Result<()> {
    let want: HashSet<i64> = desired.iter().map(|(id, _)| *id).collect();
    let current: Vec<i64> =
        sqlx::query_scalar("SELECT title_id FROM title_services WHERE service = ?")
            .bind(service.as_str())
            .fetch_all(pool)
            .await?;

    let mut tx = pool.begin().await?;
    for id in current {
        if !want.contains(&id) {
            sqlx::query("DELETE FROM title_services WHERE service = ? AND title_id = ?")
                .bind(service.as_str())
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }
    for (id, link) in desired {
        // Upsert so an existing membership row also refreshes its link.
        sqlx::query(
            "INSERT INTO title_services (title_id, service, link) VALUES (?, ?, ?)
             ON CONFLICT(title_id, service) DO UPDATE SET link = excluded.link",
        )
        .bind(id)
        .bind(service.as_str())
        .bind(link)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}
```

- [ ] **Step 3: Update existing `reconcile_service` test callers**

In `src/sync/store.rs` tests, change every `&[a, b]`-style id slice to tuples with `None` link, e.g.:
- `reconcile_service(&p, Service::Plex, &[a, b])` → `&[(a, None), (b, None)]`
- `&[a]` → `&[(a, None)]`, `&[c]` → `&[(c, None)]`, `&[]` → `&[]` (stays empty)
- the `:475` `MergedTitle` test literal already got `plex_rating_key`/`links` in Task 2.

- [ ] **Step 4: Write failing tests for link + rating_key persistence**

Add to `src/sync/store.rs` tests:

```rust
    #[tokio::test]
    async fn reconcile_service_persists_link() {
        let (p, _dir) = pool().await; // use the test pool helper already in this module
        let id = upsert_title(&p, &merged("tt1", "T", &[], &[Service::Crunchyroll]))
            .await
            .unwrap();
        reconcile_service(
            &p,
            Service::Crunchyroll,
            &[(id, Some("https://crunchyroll.com/t".into()))],
        )
        .await
        .unwrap();
        let link: Option<String> = sqlx::query_scalar(
            "SELECT link FROM title_services WHERE title_id = ? AND service = 'crunchyroll'",
        )
        .bind(id)
        .fetch_one(&p)
        .await
        .unwrap();
        assert_eq!(link.as_deref(), Some("https://crunchyroll.com/t"));
    }

    #[tokio::test]
    async fn upsert_persists_plex_rating_key() {
        let (p, _dir) = pool().await;
        let mut m = merged("tt2", "P", &[], &[Service::Plex]);
        m.plex_rating_key = Some("49518".into());
        let id = upsert_title(&p, &m).await.unwrap();
        let rk: Option<String> = sqlx::query_scalar("SELECT plex_rating_key FROM titles WHERE id = ?")
            .bind(id)
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(rk.as_deref(), Some("49518"));
    }
```

> If the store tests don't already have a `pool()` helper, reuse the exact pattern from the existing tests in this module (tempdir + `init_pool`). Match what's there.

- [ ] **Step 5: Update the `run_sync` caller (compile fix only)**

`src/sync/mod.rs:143` calls `reconcile_service(pool, *svc, &desired)` with `desired: Vec<i64>`. This will now fail to compile. Leave the full wiring for Task 6, but to keep the tree compiling after *this* task, temporarily map ids to `(id, None)`:

```rust
        let desired: Vec<(i64, Option<String>)> = id_services
            .iter()
            .filter(|(_, services)| services.contains(svc))
            .map(|(id, _)| (*id, None))
            .collect();
```
(Task 6 replaces the `None` with the real link.)

- [ ] **Step 6: Run tests (expect PASS)**

Run:
```bash
cargo test --lib sync::store
```
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/sync/store.rs src/sync/mod.rs
git commit -F - <<'EOF'
feat(store): persist plex_rating_key and per-service title_services.link

reconcile_service now upserts a link per (title, service); upsert_title
writes plex_rating_key.
EOF
```

---

## Task 4: Plex sync — capture `ratingKey` + server `machineIdentifier`

**Files:**
- Modify: `src/sync/plex.rs` (`Meta` struct, `parse_section`, `server_meta` impl), `src/sync/mod.rs` (`CatalogueSource::server_meta` default)
- Modify: `tests/fixtures/plex_section_all.json` (add `ratingKey`)

**Interfaces:**
- Produces: `CatalogueSource::server_meta(&self) -> anyhow::Result<Vec<(String, String)>>` (default `Ok(vec![])`); Plex returns `vec![("plex_machine_id", <id>)]`. `parse_section` sets `FetchedTitle.plex_rating_key` from Plex `ratingKey`.

- [ ] **Step 1: Add the default trait method**

In `src/sync/mod.rs`, inside `trait CatalogueSource`, after `fetch_watch_history`:

```rust
    /// Server-global metadata to persist after a successful fetch (e.g. the Plex
    /// `machineIdentifier`, used to build watch deep links). Default: none.
    ///
    /// # Errors
    /// Returns an error if the upstream request fails or the body cannot be parsed.
    async fn server_meta(&self) -> anyhow::Result<Vec<(String, String)>> {
        Ok(Vec::new())
    }
```

- [ ] **Step 2: Capture `ratingKey` on `Meta` and set it (failing test first)**

In `src/sync/plex.rs`, add to `struct Meta` (after `title: String,` is fine):

```rust
    #[serde(rename = "ratingKey")]
    rating_key: Option<String>,
```

In `parse_section`'s `FetchedTitle { … }` literal, replace the placeholder `plex_rating_key: None,` (from Task 2) with:

```rust
                plex_rating_key: m.rating_key,
```
(Keep `links: vec![],` — Plex has no MOTN links.)

Add a fixture field: in `tests/fixtures/plex_section_all.json`, add `"ratingKey":"49518"` to the first movie item's object.

Add a test:

```rust
    #[test]
    fn parses_rating_key() {
        let json = include_str!("../../tests/fixtures/plex_section_all.json");
        let out = parse_section(json).unwrap();
        assert_eq!(out[0].plex_rating_key.as_deref(), Some("49518"));
    }
```

- [ ] **Step 3: Implement `server_meta` for `PlexClient` (failing test first)**

Add a parse helper + a unit test in `src/sync/plex.rs`:

```rust
#[derive(Deserialize)]
struct Identity {
    #[serde(rename = "MediaContainer")]
    media_container: IdentityBody,
}
#[derive(Deserialize)]
struct IdentityBody {
    #[serde(rename = "machineIdentifier")]
    machine_identifier: Option<String>,
}

/// Extract the server `machineIdentifier` from a Plex `/identity` body.
///
/// # Errors
/// Returns an error if the JSON does not match the expected shape.
pub fn parse_machine_id(json: &str) -> anyhow::Result<Option<String>> {
    let parsed: Identity = serde_json::from_str(json)?;
    Ok(parsed.media_container.machine_identifier)
}
```

Test:

```rust
    #[test]
    fn parses_machine_identifier() {
        let json = r#"{"MediaContainer":{"machineIdentifier":"abc123def","version":"1.40"}}"#;
        assert_eq!(parse_machine_id(json).unwrap().as_deref(), Some("abc123def"));
    }
```

Then implement the trait method on `PlexClient` (in the `impl CatalogueSource for PlexClient` block):

```rust
    async fn server_meta(&self) -> anyhow::Result<Vec<(String, String)>> {
        let body = self.get_json("/identity").await?;
        Ok(parse_machine_id(&body)?
            .map(|id| vec![("plex_machine_id".to_string(), id)])
            .unwrap_or_default())
    }
```

- [ ] **Step 4: Run tests (expect PASS)**

Run:
```bash
cargo test --lib sync::plex
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/sync/plex.rs src/sync/mod.rs tests/fixtures/plex_section_all.json
git commit -F - <<'EOF'
feat(plex): capture per-item ratingKey and server machineIdentifier

parse_section sets plex_rating_key; new server_meta() reads /identity and
returns ("plex_machine_id", id) for run_sync to persist.
EOF
```

---

## Task 5: MOTN sync — parse per-service `link`

**Files:**
- Modify: `src/sync/motn.rs` (`StreamOption`, `show_to_fetched`)
- Modify: a MOTN fixture (add `link` to a `streamingOptions[]` entry)

**Interfaces:**
- Produces: `show_to_fetched` populates `FetchedTitle.links` with `(Service, link)` for each wanted service whose `streamingOptions[country]` entry carries a `link`.

- [ ] **Step 1: Add `link` to `StreamOption`**

In `src/sync/motn.rs`:

```rust
#[derive(Deserialize)]
struct StreamOption {
    service: ServiceRef,
    #[serde(default)]
    link: Option<String>,
}
```

- [ ] **Step 2: Build per-service links in `show_to_fetched` (failing test first)**

In `show_to_fetched`, after the `svcs` block, derive links from the same per-country options and map MOTN catalog ids back to `Service` via the existing `wanted_id`. Add:

```rust
    let links: Vec<(Service, String)> = s
        .streaming_options
        .get(country)
        .map(|opts| {
            services
                .iter()
                .filter_map(|svc| {
                    let id = wanted_id(*svc)?;
                    let link = opts
                        .iter()
                        .find(|o| o.service.id == id)
                        .and_then(|o| o.link.clone())?;
                    Some((*svc, link))
                })
                .collect()
        })
        .unwrap_or_default();
```

Replace the placeholder `links: vec![],` in the `FetchedTitle { … }` literal with `links,`.

> `s.streaming_options` is borrowed for `svcs` (via `.get(country)`) and again here — both are shared `.get`, so no borrow conflict. If the earlier `svcs` block moved `s.streaming_options`, reorder so both reads happen before any move. (Current code only `.get`s it, so this is fine.)

- [ ] **Step 3: Add `link` to a fixture and assert**

In `tests/fixtures/motn_search_page1.json`, add `"link":"https://www.crunchyroll.com/series/abc"` to a `streamingOptions[<country>][]` entry whose `service.id` is `crunchyroll` (add such an entry if the fixture only has `disney`). Then add a test in `motn.rs`:

```rust
    #[test]
    fn parses_streaming_link_per_service() {
        let json = include_str!("../../tests/fixtures/motn_search_page1.json");
        let (titles, _) = parse_page(json, "gb", &[Service::Crunchyroll, Service::Disney]).unwrap();
        let with_link = titles
            .iter()
            .find(|t| t.links.iter().any(|(s, _)| *s == Service::Crunchyroll));
        assert!(with_link.is_some(), "a crunchyroll link should be attributed");
        let (_, url) = with_link
            .unwrap()
            .links
            .iter()
            .find(|(s, _)| *s == Service::Crunchyroll)
            .unwrap();
        assert!(url.contains("crunchyroll.com"));
    }
```

> Match the fixture's actual country key and `streamingOptions` shape; adjust the `"gb"` arg and the injected entry to be consistent with the existing fixture (the design notes `streamingOptions` is keyed by country).

- [ ] **Step 4: Run tests (expect PASS)**

Run:
```bash
cargo test --lib sync::motn
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/sync/motn.rs tests/fixtures/motn_search_page1.json
git commit -F - <<'EOF'
feat(motn): capture per-service streamingOptions link

show_to_fetched attributes each wanted service's `link` onto FetchedTitle.links.
EOF
```

---

## Task 6: Wire links + machineId into `run_sync`

**Files:**
- Modify: `src/sync/mod.rs` (`run_sync`: `id_services` carries links; reconcile passes real links; persist `server_meta`)

**Interfaces:**
- Consumes: `MergedTitle.links` (Task 2), `reconcile_service(&[(i64, Option<String>)])` (Task 3), `CatalogueSource::server_meta` (Task 4), `db::app_meta::set` (Task 1).

- [ ] **Step 1: Carry links on `id_services`**

In `run_sync`, change the `id_services` accumulation:

```rust
    let mut id_services: Vec<(i64, Vec<Service>, Vec<(Service, String)>)> =
        Vec::with_capacity(merged.len());
    for m in &merged {
        let id = store::upsert_title(pool, m).await?;
        id_services.push((id, m.services.clone(), m.links.clone()));
    }
```

- [ ] **Step 2: Resolve the real link per (id, service) in the reconcile loop**

Replace the temporary `desired` builder (from Task 3 Step 5) with:

```rust
        let desired: Vec<(i64, Option<String>)> = id_services
            .iter()
            .filter(|(_, services, _)| services.contains(svc))
            .map(|(id, _, links)| {
                let link = links.iter().find(|(s, _)| s == svc).map(|(_, l)| l.clone());
                (*id, link)
            })
            .collect();
```

- [ ] **Step 3: Persist `server_meta` from each successful source (non-fatal)**

After the watch-history apply loop (or alongside it, iterating `ok_sources`), add:

```rust
    // Persist server-global metadata (e.g. Plex machineIdentifier) from each
    // successful source. Non-fatal: a failure logs and leaves the prior value,
    // so a stale-but-present machine id still builds working links.
    for src in &ok_sources {
        match src.server_meta().await {
            Ok(pairs) => {
                for (k, v) in pairs {
                    if let Err(e) = crate::db::app_meta::set(pool, &k, &v).await {
                        tracing::error!("app_meta set {k} failed: {e:#}");
                    }
                }
            }
            Err(e) => tracing::error!("server_meta fetch failed for {}: {e:#}", src.name()),
        }
    }
```

- [ ] **Step 4: Build and run the full sync suite**

Run:
```bash
cargo test --lib sync::
```
Expected: PASS (existing `run_sync` integration tests still green; the reconcile now passes `(id, link)` tuples).

- [ ] **Step 5: Commit**

```bash
git add src/sync/mod.rs
git commit -F - <<'EOF'
feat(sync): reconcile per-service links and persist server machineId

run_sync threads MergedTitle.links into reconcile_service and upserts each
source's server_meta (Plex machineIdentifier) into app_meta, non-fatally.
EOF
```

---

## Task 7: `PLEX_WEB_URL` config + `WatchConfig` app data

**Files:**
- Modify: `src/config.rs` (struct field + `load`)
- Modify: `src/main.rs` (build + register `WatchConfig`)
- Create: the `WatchConfig` type in `src/routes/watch.rs` (declared here, used in Task 8 — define it now)

**Interfaces:**
- Produces: `Config.plex_web_url: Option<String>` (= `PLEX_WEB_URL` else `PLEX_URL`). `cue::routes::watch::WatchConfig { plex_web_url: Option<String> }` registered as `web::Data`.

- [ ] **Step 1: Add the config field (failing test first)**

In `src/config.rs`, add to `struct Config` (after `plex_token`):

```rust
    pub plex_web_url: Option<String>,
```

In `load`, after `plex_token: get("PLEX_TOKEN"),`:

```rust
            // Browser-facing Plex base for watch links; defaults to PLEX_URL
            // (identical on a single-host self-hosted deployment).
            plex_web_url: get("PLEX_WEB_URL").or_else(|| get("PLEX_URL")),
```

Add a test in `src/config.rs` tests:

```rust
    #[test]
    fn plex_web_url_defaults_to_plex_url() {
        let m = std::collections::HashMap::from([("PLEX_URL".to_string(), "http://lan:32400".to_string())]);
        let c = Config::load(getter(m));
        assert_eq!(c.plex_web_url.as_deref(), Some("http://lan:32400"));
    }
```

> Use the same `getter(...)` test helper the existing config tests use. If `PLEX_WEB_URL` is set, it must win — add a second assertion if a helper makes it cheap.

- [ ] **Step 2: Create `src/routes/watch.rs` with `WatchConfig` (minimal, expands in Task 8)**

```rust
//! Watch-at-source redirect: GET /api/titles/{id}/watch/{service} 302s to the
//! title's page on Plex (self-hosted /web) or Crunchyroll/Disney+ (MOTN link).
//! Real URLs (machineId, internal Plex base, MOTN links) never enter any payload.

/// Browser-facing Plex base URL for building watch links. Held in app data so
/// the internal Plex address never reaches the client.
#[derive(Clone)]
pub struct WatchConfig {
    pub plex_web_url: Option<String>,
}
```

- [ ] **Step 3: Declare the module**

In `src/routes/mod.rs`, add `pub mod watch;` to the module list.

- [ ] **Step 4: Register `WatchConfig` in `main.rs`**

In `src/main.rs`, near the `plex_art` construction (~`:124`), add:

```rust
    let watch_cfg = cue::routes::watch::WatchConfig {
        plex_web_url: cfg.plex_web_url.clone(),
    };
```

And in the `App::new()` builder (alongside `.app_data(web::Data::new(plex_art.clone()))`):

```rust
            .app_data(web::Data::new(watch_cfg.clone()))
```

- [ ] **Step 5: Build + test**

Run:
```bash
cargo test --lib config:: && cargo build
```
Expected: PASS / builds.

- [ ] **Step 6: Commit**

```bash
git add src/config.rs src/routes/watch.rs src/routes/mod.rs src/main.rs
git commit -F - <<'EOF'
feat(config): PLEX_WEB_URL (defaults to PLEX_URL) + WatchConfig app data
EOF
```

---

## Task 8: Watch redirect route

**Files:**
- Modify: `src/routes/watch.rs` (handler + URL builders + tests)
- Modify: `src/routes/mod.rs` (register the route)

**Interfaces:**
- Consumes: `WatchConfig` (Task 7), `db::app_meta::get` (Task 1), `titles.plex_rating_key` + `title_services.link` (Tasks 3–5), `Service::parse`.
- Produces: `GET /api/titles/{id}/watch/{service}` → `302` Location, `404` (no link / unknown id), `400` (unknown service).

- [ ] **Step 1: Add the URL builders + handler to `src/routes/watch.rs`**

Append to `src/routes/watch.rs`:

```rust
use actix_web::{web, HttpResponse, Responder};
use sqlx::SqlitePool;

use crate::models::Service;

/// Build the Plex web deep link for an item on the self-hosted server.
#[must_use]
pub fn plex_web_url(base: &str, machine_id: &str, rating_key: &str) -> String {
    format!(
        "{}/web/index.html#!/server/{}/details?key=%2Flibrary%2Fmetadata%2F{}",
        base.trim_end_matches('/'),
        machine_id,
        rating_key
    )
}

/// Defense-in-depth: a MOTN link comes from an external feed, so only redirect
/// to `https://` on the service's own domain (prevents an open-redirect pivot).
#[must_use]
pub fn is_allowed_motn_link(service: Service, url: &str) -> bool {
    let expected = match service {
        Service::Crunchyroll => "crunchyroll.com",
        Service::Disney => "disneyplus.com",
        Service::Plex => return false,
    };
    reqwest::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str()
                .is_some_and(|h| h == expected || h.ends_with(&format!(".{expected}")))
    })
}

pub async fn redirect(
    path: web::Path<(i64, String)>,
    pool: web::Data<SqlitePool>,
    cfg: web::Data<WatchConfig>,
) -> impl Responder {
    let (id, service_str) = path.into_inner();
    let Some(service) = Service::parse(&service_str) else {
        return HttpResponse::BadRequest().finish();
    };

    let location = match service {
        Service::Plex => {
            let rating_key = sqlx::query_scalar::<_, Option<String>>(
                "SELECT plex_rating_key FROM titles WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(pool.get_ref())
            .await
            .ok()
            .flatten()
            .flatten();
            let machine_id = crate::db::app_meta::get(pool.get_ref(), "plex_machine_id")
                .await
                .ok()
                .flatten();
            match (rating_key, machine_id, cfg.plex_web_url.as_ref()) {
                (Some(rk), Some(mid), Some(base)) => plex_web_url(base, &mid, &rk),
                _ => return HttpResponse::NotFound().finish(),
            }
        }
        Service::Crunchyroll | Service::Disney => {
            let link = sqlx::query_scalar::<_, Option<String>>(
                "SELECT link FROM title_services WHERE title_id = ? AND service = ?",
            )
            .bind(id)
            .bind(service.as_str())
            .fetch_optional(pool.get_ref())
            .await
            .ok()
            .flatten()
            .flatten();
            match link {
                Some(u) if is_allowed_motn_link(service, &u) => u,
                Some(_) => {
                    tracing::warn!("refusing non-allowlisted watch link for {}", service.as_str());
                    return HttpResponse::NotFound().finish();
                }
                None => return HttpResponse::NotFound().finish(),
            }
        }
    };

    HttpResponse::Found()
        .insert_header(("Location", location))
        .finish()
}
```

> The double `.flatten()` on `plex_rating_key`/`link`: `fetch_optional` of a `SELECT col` typed as `Option<String>` yields `Result<Option<Option<String>>>` — outer = row exists, inner = column NULL. `.ok().flatten().flatten()` collapses both to `Option<String>`.

- [ ] **Step 2: Register the route**

In `src/routes/mod.rs`, inside the `/api` scope, add:

```rust
            .route("/titles/{id}/watch/{service}", web::get().to(watch::redirect))
```

- [ ] **Step 3: Write failing route tests**

Add to `src/routes/watch.rs`:

```rust
#[cfg(test)]
mod tests {
    use actix_web::{test, web, App};
    use sqlx::SqlitePool;

    use super::{is_allowed_motn_link, plex_web_url, WatchConfig};
    use crate::db::{app_meta, init_pool};
    use crate::models::Service;
    use crate::routes;

    async fn pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (init_pool(&url).await.unwrap(), dir)
    }

    #[test]
    fn builds_plex_web_url() {
        assert_eq!(
            plex_web_url("http://lan:32400/", "MID", "49518"),
            "http://lan:32400/web/index.html#!/server/MID/details?key=%2Flibrary%2Fmetadata%2F49518"
        );
    }

    #[test]
    fn motn_link_allowlist() {
        assert!(is_allowed_motn_link(Service::Crunchyroll, "https://www.crunchyroll.com/x"));
        assert!(!is_allowed_motn_link(Service::Crunchyroll, "http://www.crunchyroll.com/x"));
        assert!(!is_allowed_motn_link(Service::Crunchyroll, "https://evil.example/x"));
        assert!(!is_allowed_motn_link(Service::Plex, "https://crunchyroll.com/x"));
    }

    async fn app_with(pool: SqlitePool, cfg: WatchConfig) -> impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse,
        Error = actix_web::Error,
    > {
        test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .app_data(web::Data::new(cfg))
                .configure(routes::configure),
        )
        .await
    }

    #[actix_web::test]
    async fn plex_redirects_when_resolvable() {
        let (p, _dir) = pool().await;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO titles (title, year, type, plex_rating_key) VALUES ('T', 2020, 'movie', '49518') RETURNING id",
        )
        .fetch_one(&p)
        .await
        .unwrap();
        app_meta::set(&p, "plex_machine_id", "MID").await.unwrap();
        let app = app_with(p, WatchConfig { plex_web_url: Some("http://lan:32400".into()) }).await;

        let req = test::TestRequest::get()
            .uri(&format!("/api/titles/{id}/watch/plex"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 302);
        let loc = resp.headers().get("Location").unwrap().to_str().unwrap();
        assert!(loc.contains("/server/MID/details"));
        assert!(loc.contains("49518"));
    }

    #[actix_web::test]
    async fn crunchyroll_redirects_to_link() {
        let (p, _dir) = pool().await;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO titles (title, year, type) VALUES ('A', 2021, 'series') RETURNING id",
        )
        .fetch_one(&p)
        .await
        .unwrap();
        sqlx::query("INSERT INTO title_services (title_id, service, link) VALUES (?, 'crunchyroll', 'https://www.crunchyroll.com/series/x')")
            .bind(id)
            .execute(&p)
            .await
            .unwrap();
        let app = app_with(p, WatchConfig { plex_web_url: None }).await;

        let req = test::TestRequest::get()
            .uri(&format!("/api/titles/{id}/watch/crunchyroll"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 302);
        assert_eq!(
            resp.headers().get("Location").unwrap().to_str().unwrap(),
            "https://www.crunchyroll.com/series/x"
        );
    }

    #[actix_web::test]
    async fn missing_link_is_404_and_bad_service_is_400() {
        let (p, _dir) = pool().await;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO titles (title, year, type) VALUES ('B', 2021, 'movie') RETURNING id",
        )
        .fetch_one(&p)
        .await
        .unwrap();
        let app = app_with(p, WatchConfig { plex_web_url: None }).await;

        let r404 = test::call_service(
            &app,
            test::TestRequest::get().uri(&format!("/api/titles/{id}/watch/plex")).to_request(),
        )
        .await;
        assert_eq!(r404.status(), 404);

        let r400 = test::call_service(
            &app,
            test::TestRequest::get().uri(&format!("/api/titles/{id}/watch/netflix")).to_request(),
        )
        .await;
        assert_eq!(r400.status(), 400);
    }
}
```

> If the `app_with` helper's explicit return type is awkward in this codebase, inline `test::init_service(...)` in each test instead (matches the pattern in `catalogue.rs`/`images.rs` tests). Keep whichever compiles cleanly with the project's actix version. The `actix_http::Request` type may need `use actix_web::dev` adjustments — prefer inlining if unsure.

- [ ] **Step 4: Run tests (expect PASS)**

Run:
```bash
cargo test --lib routes::watch
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/routes/watch.rs src/routes/mod.rs
git commit -F - <<'EOF'
feat(routes): unified /api/titles/{id}/watch/{service} 302 redirect

Resolves Plex web URL (machineId + ratingKey) or the allowlisted MOTN link
server-side; 404 on no link/unknown id, 400 on unknown service.
EOF
```

---

## Task 9: Detail payload `watchable`

**Files:**
- Modify: `src/models.rs` (`TitleDto` field)
- Modify: `src/db/catalogue.rs` (`fetch_title` derivation + query)

**Interfaces:**
- Consumes: `titles.plex_rating_key`, `title_services.link`, `app_meta.plex_machine_id`.
- Produces: `TitleDto.watchable: Vec<String>` — services that resolve to a link (Plex iff `plex_rating_key` set **and** `plex_machine_id` known; Crunchyroll/Disney iff their `title_services.link` is non-NULL).

- [ ] **Step 1: Add the field to `TitleDto`**

In `src/models.rs`, in `struct TitleDto` (after `pub services: Vec<Service>,` or near the end before `watched`):

```rust
    /// Services that resolve to a working watch link (availability only — no URLs).
    pub watchable: Vec<String>,
```

The serde tests in `models.rs` build a `TitleDto`; add `watchable: vec![],` to that literal so they compile.

- [ ] **Step 2: Derive `watchable` in `fetch_title` (failing test first)**

In `src/db/catalogue.rs::fetch_title`:

1. Extend the title row query to include `plex_rating_key`. The function uses `TitleRow`; rather than alter `TitleRow`, fetch the rating key separately to keep `TitleRow` shared with the list path:

```rust
    let plex_rating_key: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT plex_rating_key FROM titles WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .flatten();
```

2. Fetch per-service links (replace the existing services scalar query with a row query that also returns `link`):

```rust
    let service_rows = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT service, link FROM title_services WHERE title_id = ?",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let services: Vec<Service> = service_rows
        .iter()
        .filter_map(|(s, _)| Service::parse(s))
        .collect();
```

3. Read the machine id and build `watchable`:

```rust
    let machine_id = crate::db::app_meta::get(pool, "plex_machine_id").await?;
    let mut watchable: Vec<String> = Vec::new();
    for (svc, link) in &service_rows {
        match Service::parse(svc) {
            Some(Service::Plex) if plex_rating_key.is_some() && machine_id.is_some() => {
                watchable.push("plex".to_string());
            }
            Some(Service::Crunchyroll | Service::Disney) if link.is_some() => {
                watchable.push(svc.clone());
            }
            _ => {}
        }
    }
```

4. Add `watchable,` to the returned `TitleDto { … }` literal.

Add a test:

```rust
    #[actix_web::test]
    async fn watchable_lists_only_resolvable_sources() {
        let (pool, _dir) = seeded_pool().await; // reuse the catalogue.rs test helper
        // Make title 1 a Plex item with a rating key + a crunchyroll link, and set machine id.
        sqlx::query("UPDATE titles SET plex_rating_key = '49518' WHERE id = 1").execute(&pool).await.unwrap();
        sqlx::query("INSERT OR REPLACE INTO title_services (title_id, service, link) VALUES (1, 'plex', NULL)").execute(&pool).await.unwrap();
        sqlx::query("INSERT OR REPLACE INTO title_services (title_id, service, link) VALUES (1, 'crunchyroll', 'https://www.crunchyroll.com/x')").execute(&pool).await.unwrap();
        crate::db::app_meta::set(&pool, "plex_machine_id", "MID").await.unwrap();

        let dto = fetch_title(&pool, 1).await.unwrap().unwrap();
        assert!(dto.watchable.contains(&"plex".to_string()));
        assert!(dto.watchable.contains(&"crunchyroll".to_string()));
    }
```

> Put this test where it can see `fetch_title` (a `#[cfg(test)] mod tests` in `db/catalogue.rs` using the tempdir+seed pattern). If `db/catalogue.rs` has no test module yet, add one mirroring the tempdir/seed helper used in `routes/catalogue.rs`.

- [ ] **Step 3: Run tests (expect PASS)**

Run:
```bash
cargo test --lib db::catalogue && cargo test --lib models
```
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/models.rs src/db/catalogue.rs
git commit -F - <<'EOF'
feat(catalogue): detail payload exposes URL-free watchable[] list

watchable lists Plex (when ratingKey + machineId known) and Crunchyroll/Disney
(when a title_services.link exists); no URLs are serialized.
EOF
```

---

## Task 10: Frontend types + client validation

**Files:**
- Modify: `frontend/src/types.ts` (`TitleDetail`)
- Modify: `frontend/src/api/client.ts` (`isTitleDetail`)
- Modify: `frontend/src/api/__tests__/client.test.ts`

**Interfaces:**
- Produces: `TitleDetail.watchable: ServiceKey[]`; `getTitle` rejects a detail whose `watchable` is not an array of `ServiceKey`.

- [ ] **Step 1: Add `watchable` to `TitleDetail`**

In `frontend/src/types.ts`:

```ts
export interface TitleDetail extends TitleListItem {
  desc: string
  cast: string[]
  watchable: ServiceKey[]
}
```

- [ ] **Step 2: Validate it in `isTitleDetail` (failing test first)**

In `frontend/src/api/client.ts`, extend `isTitleDetail`:

```ts
function isTitleDetail(v: unknown): v is TitleDetail {
  if (!isTitle(v)) return false
  const r = v as unknown as Record<string, unknown>
  const watchable = r.watchable
  const services: ReadonlyArray<string> = ['plex', 'disney', 'crunchyroll']
  return typeof r.desc === 'string'
    && Array.isArray(r.cast)
    && Array.isArray(watchable)
    && watchable.every((s) => typeof s === 'string' && services.includes(s))
}
```

Add a test in `frontend/src/api/__tests__/client.test.ts` (mirror the existing detail-validation tests): a valid detail with `watchable: ['plex']` passes; one with `watchable: ['netflix']` or a missing `watchable` is rejected by `getTitle` (mock `fetch`). Follow the file's existing mocking style.

- [ ] **Step 3: Run tests (expect PASS)**

Run:
```bash
cd frontend && npx vitest run src/api/__tests__/client.test.ts
```
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/types.ts frontend/src/api/client.ts frontend/src/api/__tests__/client.test.ts
git commit -F - <<'EOF'
feat(web): TitleDetail.watchable type + client validation
EOF
```

---

## Task 11: `WatchLinks.vue` + DetailView integration

**Files:**
- Create: `frontend/src/components/WatchLinks.vue`
- Create: `frontend/src/components/__tests__/WatchLinks.test.ts`
- Modify: `frontend/src/views/DetailView.vue` (mount under "Mark as watched")

**Interfaces:**
- Consumes: `TitleDetail.watchable` (Task 10), `services` tokens (`frontend/src/design/tokens.ts`).
- Produces: `<WatchLinks :id="number" :watchable="ServiceKey[]" />` rendering one `<a href="/api/titles/{id}/watch/{svc}" target="_blank" rel="noopener noreferrer">` per entry, branded with each service's dot color; renders nothing when empty.

- [ ] **Step 1: Write `WatchLinks.vue`**

```vue
<script setup lang="ts">
import type { ServiceKey } from '@/types'
import { services } from '@/design/tokens'

const props = defineProps<{ id: number; watchable: ServiceKey[] }>()
</script>

<template>
  <div v-if="props.watchable.length" class="watch-links" data-test="watch-links">
    <div class="watch-eyebrow">Watch on</div>
    <a
      v-for="svc in props.watchable"
      :key="svc"
      class="watch-btn"
      :href="`/api/titles/${props.id}/watch/${svc}`"
      target="_blank"
      rel="noopener noreferrer"
      :data-test="`watch-${svc}`"
      :style="{ '--svc-dot': services[svc].dot }"
    >
      <span class="dot" />
      {{ services[svc].label }}
    </a>
  </div>
</template>

<style scoped>
.watch-links {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 12px;
}
.watch-eyebrow {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 10px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-secondary, #9aa0aa);
}
.watch-btn {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 8px 12px;
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, var(--svc-dot) 45%, transparent);
  background: color-mix(in srgb, var(--svc-dot) 12%, transparent);
  color: var(--text-primary, #f2f4f8);
  font-size: 12px;
  text-decoration: none;
  transition: background 120ms ease;
}
.watch-btn:hover {
  background: color-mix(in srgb, var(--svc-dot) 22%, transparent);
}
.watch-btn .dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--svc-dot);
  flex: none;
}
</style>
```

- [ ] **Step 2: Write the failing component test**

`frontend/src/components/__tests__/WatchLinks.test.ts`:

```ts
import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import WatchLinks from '../WatchLinks.vue'

describe('WatchLinks', () => {
  it('renders a branded link per watchable service with the right href', () => {
    const w = mount(WatchLinks, { props: { id: 42, watchable: ['plex', 'crunchyroll'] } })
    const plex = w.get('[data-test="watch-plex"]')
    expect(plex.attributes('href')).toBe('/api/titles/42/watch/plex')
    expect(plex.attributes('target')).toBe('_blank')
    expect(plex.attributes('rel')).toBe('noopener noreferrer')
    expect(w.get('[data-test="watch-crunchyroll"]').attributes('href')).toBe('/api/titles/42/watch/crunchyroll')
  })

  it('renders nothing when watchable is empty', () => {
    const w = mount(WatchLinks, { props: { id: 1, watchable: [] } })
    expect(w.find('[data-test="watch-links"]').exists()).toBe(false)
  })
})
```

- [ ] **Step 3: Run the component test (expect PASS)**

Run:
```bash
cd frontend && npx vitest run src/components/__tests__/WatchLinks.test.ts
```
Expected: PASS.

- [ ] **Step 4: Mount it in `DetailView.vue`**

In `frontend/src/views/DetailView.vue`:

1. Import it with the other component imports (near `import ServicePill from '@/components/ServicePill.vue'`):

```ts
import WatchLinks from '@/components/WatchLinks.vue'
```

2. Place it in the left action column, immediately after the "Mark as watched" `<button>` block (after line ~128, before the "Your rating" well):

```vue
        <WatchLinks :id="detail.id" :watchable="detail.watchable" />
```

> `detail` is the loaded `TitleDetail`; it now has `watchable` (Task 10). If `detail` may be loosely typed in this file, no cast is needed — the prop accepts `ServiceKey[]`.

- [ ] **Step 5: Run the DetailView tests + full frontend suite**

Run:
```bash
cd frontend && npx vitest run
```
Expected: PASS. If existing `DetailView` tests build a detail fixture object, add `watchable: []` to those fixtures so they satisfy the type (and to assert no buttons render by default).

- [ ] **Step 6: Commit**

```bash
git add frontend/src/components/WatchLinks.vue frontend/src/components/__tests__/WatchLinks.test.ts frontend/src/views/DetailView.vue
git commit -F - <<'EOF'
feat(web): WatchLinks block in the detail action column

Renders one branded "Watch on X" link per watchable service, opening the
backend redirect in a new tab; renders nothing when none are available.
EOF
```

---

## Final verification

- [ ] **Backend:** `cargo clean -p cue && cargo test` → all pass.
- [ ] **Clippy:** `cargo clippy --all-targets -- -D warnings` → clean (add per-item `#[allow]` with a one-line reason only if truly needed).
- [ ] **Frontend:** `cd frontend && npx vitest run` → all pass; `npx vue-tsc --noEmit` (or the project's typecheck script) → clean.
- [ ] **Manual smoke (optional, needs live Plex/MOTN):** run a sync, open a Plex-backed title's detail, confirm a "Watch on Plex" button appears and the redirect opens the server's web UI on that item; confirm a Crunchyroll/Disney title opens the service page.

---

## Self-Review Notes (coverage map)

- Spec §1 unified redirect → Task 8. §2 link construction → Task 8 (`plex_web_url`) + Task 5 (MOTN link). §3 `PLEX_WEB_URL` → Task 7. §4 data capture → Task 4 (Plex) + Task 5 (MOTN). §5 storage → Task 1 (schema/app_meta) + Task 3 (persist) + Task 2 (cache round-trip, the delta-sync wrinkle). §6 frontend → Tasks 9–11. §7 errors/fallbacks → Task 8 (404/400, allowlist) + Task 6 (non-fatal machineId) + Task 9 (watchable gating). §8 testing → per-task tests. §Security → no URLs in payload (Task 9), allowlist (Task 8), `rel="noopener noreferrer"` (Task 11).
