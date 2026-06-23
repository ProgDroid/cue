# cue — User-Data Writes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the write path for personal ratings (1–10) and watched state, persisting to `user_ratings`/`watch_history` keyed on `imdb_id`, and wire the frontend's currently local-only `toggleWatched`/`setRating` to the API.

**Architecture:** Three new REST endpoints (`PUT`/`DELETE /api/titles/{id}/rating`, `PUT /api/titles/{id}/watched`) backed by a new pure-SQLx `db::user_data` module. A migration widens the rating `CHECK` to 1–10. The Pinia store's two stub mutators become async optimistic-with-rollback actions (plus a new `clearRating`); `StarRating` grows to 10 pips with a clear gesture and a disabled state for titles lacking an `imdb_id`.

**Tech Stack:** Rust + Actix-web + SQLx (SQLite, runtime queries) backend; Vue 3 `<script setup>` + TypeScript + Pinia + Vitest frontend.

## Global Constraints

- **SQLx runtime queries only** — `sqlx::query` / `query_as` / `query_scalar`. NEVER the compile-time `query!` / `query_as!` macros (project builds with no live `DATABASE_URL`).
- **Test databases** use `tempfile::tempdir()` (NOT `NamedTempFile`); build the URL as `format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"))`; keep the `TempDir` guard bound as `_dir` for the test's lifetime.
- **User data is keyed on `imdb_id`** (decision D6), resolved from the numeric `titles.id` in the path.
- **Rating range is 1–10 inclusive.**
- **Modules are declared in their parent `mod.rs`** (`src/db/mod.rs`, `src/routes/mod.rs`), not in `main.rs` — the crate is lib + bin.
- **Backend gate (must pass before each backend commit):** `cargo test && cargo clippy --all-targets -- -D warnings`. Per-item exceptions use a local `#[allow(clippy::…)]` with a one-line reason; never widen the global `[lints.clippy]` table.
- **Frontend gate (must pass before each frontend commit):** `npm test` (vitest run) AND `npm run build` (vue-tsc). For local installs use `npm install` (the lockfile is Linux-flavored; `npm ci` is the Docker/CI consumer).
- **Commit via the Bash tool, not PowerShell** (PowerShell prepends a UTF-8 BOM to the commit subject). Keep commit subjects free of double quotes.

---

### Task 1: Migration — widen `user_ratings.rating` to 1–10

**Files:**
- Create: `migrations/0002_widen_rating_to_10.sql`
- Test: `src/db/mod.rs` (add a test to the existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: the existing `init_pool` (`src/db/mod.rs`) which runs `sqlx::migrate!("./migrations")`.
- Produces: a `user_ratings` table whose `CHECK` accepts 1–10. No Rust API change.

**Context:** SQLite cannot `ALTER` a `CHECK` constraint, so the table is rebuilt. The seed (`src/db/seed.rs`) inserts **no** `user_ratings` rows and no live writes shipped before this plan, so the data copy is empty in practice — but written to be correct if rows exist.

- [ ] **Step 1: Write the migration**

Create `migrations/0002_widen_rating_to_10.sql`:

```sql
-- SQLite can't ALTER a CHECK constraint, so rebuild user_ratings with 1-10.
CREATE TABLE user_ratings_new (
    imdb_id   TEXT PRIMARY KEY,
    rating    INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 10),
    rated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
INSERT INTO user_ratings_new (imdb_id, rating, rated_at)
    SELECT imdb_id, rating, rated_at FROM user_ratings;
DROP TABLE user_ratings;
ALTER TABLE user_ratings_new RENAME TO user_ratings;
```

- [ ] **Step 2: Write the failing test**

Add to the `tests` module in `src/db/mod.rs` (it already has a `fresh_pool`-style helper using `init_pool`; reuse the same tempdir pattern — check the top of the existing test module for the exact helper name and signature, then mirror it):

```rust
#[tokio::test]
async fn user_ratings_accepts_one_to_ten() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");
    let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
    let pool = init_pool(&url).await.unwrap();

    // 10 is now in range.
    sqlx::query("INSERT INTO user_ratings (imdb_id, rating) VALUES ('tt1', 10)")
        .execute(&pool)
        .await
        .expect("rating 10 should be accepted after migration 0002");

    // 11 is still rejected by the CHECK.
    let too_high = sqlx::query("INSERT INTO user_ratings (imdb_id, rating) VALUES ('tt2', 11)")
        .execute(&pool)
        .await;
    assert!(too_high.is_err(), "rating 11 must violate the CHECK");
}
```

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test --lib user_ratings_accepts_one_to_ten`
Expected: PASS (the migration is applied by `init_pool`). If it fails with a CHECK error on `rating 10`, the migration did not apply — verify the filename prefix is `0002_`.

- [ ] **Step 4: Run the full backend gate**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all tests pass, clippy clean.

- [ ] **Step 5: Commit**

```bash
git add migrations/0002_widen_rating_to_10.sql src/db/mod.rs
git commit -m "feat(db): widen user_ratings to 1-10 (migration 0002)"
```

---

### Task 2: `db::user_data` — persistence functions

**Files:**
- Create: `src/db/user_data.rs`
- Modify: `src/db/mod.rs:1-4` (add `pub mod user_data;` to the module declarations)

**Interfaces:**
- Consumes: `sqlx::SqlitePool`; the `user_ratings` (1–10) and `watch_history` tables.
- Produces (used by Task 3):
  - `pub enum KeyLookup { Key(String), NoImdbId, NotFound }`
  - `pub async fn resolve_key(pool: &SqlitePool, title_id: i64) -> anyhow::Result<KeyLookup>`
  - `pub async fn set_rating(pool: &SqlitePool, key: &str, rating: i64) -> anyhow::Result<()>`
  - `pub async fn clear_rating(pool: &SqlitePool, key: &str) -> anyhow::Result<()>`
  - `pub async fn set_watched(pool: &SqlitePool, key: &str, watched: bool) -> anyhow::Result<()>`

- [ ] **Step 1: Declare the module**

In `src/db/mod.rs`, add after line 4 (`pub mod sync_runs;`):

```rust
pub mod user_data;
```

- [ ] **Step 2: Write the failing tests**

Create `src/db/user_data.rs` with the test module first (and an empty body that won't compile yet — that's the failing state):

```rust
use sqlx::SqlitePool;

/// Outcome of resolving a numeric title id to its `imdb_id` write key (D6).
pub enum KeyLookup {
    Key(String),
    NoImdbId,
    NotFound,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_pool;

    async fn fresh_pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        (pool, dir)
    }

    async fn insert_title(pool: &SqlitePool, imdb: Option<&str>) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO titles (imdb_id, title, year, type) VALUES (?, 'T', 2020, 'movie') RETURNING id",
        )
        .bind(imdb)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn resolve_key_distinguishes_missing_null_and_present() {
        let (pool, _dir) = fresh_pool().await;
        let with = insert_title(&pool, Some("tt100")).await;
        let without = insert_title(&pool, None).await;

        assert!(matches!(resolve_key(&pool, with).await.unwrap(), KeyLookup::Key(k) if k == "tt100"));
        assert!(matches!(resolve_key(&pool, without).await.unwrap(), KeyLookup::NoImdbId));
        assert!(matches!(resolve_key(&pool, 99999).await.unwrap(), KeyLookup::NotFound));
    }

    #[tokio::test]
    async fn set_rating_upserts() {
        let (pool, _dir) = fresh_pool().await;
        set_rating(&pool, "tt100", 7).await.unwrap();
        set_rating(&pool, "tt100", 9).await.unwrap();

        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_ratings WHERE imdb_id = 'tt100'")
            .fetch_one(&pool).await.unwrap();
        let val: i64 = sqlx::query_scalar("SELECT rating FROM user_ratings WHERE imdb_id = 'tt100'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!((rows, val), (1, 9));
    }

    #[tokio::test]
    async fn clear_rating_removes_and_is_noop_when_absent() {
        let (pool, _dir) = fresh_pool().await;
        set_rating(&pool, "tt100", 5).await.unwrap();
        clear_rating(&pool, "tt100").await.unwrap();
        clear_rating(&pool, "tt100").await.unwrap(); // no-op, no error

        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_ratings WHERE imdb_id = 'tt100'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn set_watched_is_idempotent_and_manual_only() {
        let (pool, _dir) = fresh_pool().await;
        // A pre-existing imported plex row must survive an un-watch.
        sqlx::query("INSERT INTO watch_history (imdb_id, source) VALUES ('tt100', 'plex')")
            .execute(&pool).await.unwrap();

        set_watched(&pool, "tt100", true).await.unwrap();
        set_watched(&pool, "tt100", true).await.unwrap(); // idempotent: no second manual row

        let manual: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM watch_history WHERE imdb_id = 'tt100' AND source = 'manual'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(manual, 1);

        set_watched(&pool, "tt100", false).await.unwrap();
        let manual_after: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM watch_history WHERE imdb_id = 'tt100' AND source = 'manual'")
            .fetch_one(&pool).await.unwrap();
        let plex_after: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM watch_history WHERE imdb_id = 'tt100' AND source = 'plex'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!((manual_after, plex_after), (0, 1));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib db::user_data`
Expected: FAIL to compile — `resolve_key`, `set_rating`, `clear_rating`, `set_watched` not found.

- [ ] **Step 4: Write the implementation**

Insert the function bodies above the `#[cfg(test)]` block in `src/db/user_data.rs`:

```rust
/// Resolve a numeric title id to the `imdb_id` used as the user-data key (D6).
///
/// # Errors
/// Returns an error if the database query fails.
pub async fn resolve_key(pool: &SqlitePool, title_id: i64) -> anyhow::Result<KeyLookup> {
    let row: Option<(Option<String>,)> = sqlx::query_as("SELECT imdb_id FROM titles WHERE id = ?")
        .bind(title_id)
        .fetch_optional(pool)
        .await?;
    Ok(match row {
        None => KeyLookup::NotFound,
        Some((None,)) => KeyLookup::NoImdbId,
        Some((Some(key),)) => KeyLookup::Key(key),
    })
}

/// Upsert the user's 1-10 rating for a title key.
///
/// # Errors
/// Returns an error if the database query fails.
pub async fn set_rating(pool: &SqlitePool, key: &str, rating: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_ratings (imdb_id, rating) VALUES (?, ?)
         ON CONFLICT(imdb_id) DO UPDATE SET rating = excluded.rating, rated_at = datetime('now')",
    )
    .bind(key)
    .bind(rating)
    .execute(pool)
    .await?;
    Ok(())
}

/// Remove the user's rating for a title key (no-op if absent).
///
/// # Errors
/// Returns an error if the database query fails.
pub async fn clear_rating(pool: &SqlitePool, key: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM user_ratings WHERE imdb_id = ?")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

/// Set watched state via a single `manual` `watch_history` row.
///
/// Watching is idempotent (at most one manual row); un-watching deletes only
/// `source = 'manual'` rows, leaving any imported `plex` history intact (D5.2).
///
/// # Errors
/// Returns an error if the database query fails.
pub async fn set_watched(pool: &SqlitePool, key: &str, watched: bool) -> anyhow::Result<()> {
    if watched {
        sqlx::query(
            "INSERT INTO watch_history (imdb_id, source)
             SELECT ?, 'manual'
             WHERE NOT EXISTS (
                 SELECT 1 FROM watch_history WHERE imdb_id = ? AND source = 'manual'
             )",
        )
        .bind(key)
        .bind(key)
        .execute(pool)
        .await?;
    } else {
        sqlx::query("DELETE FROM watch_history WHERE imdb_id = ? AND source = 'manual'")
            .bind(key)
            .execute(pool)
            .await?;
    }
    Ok(())
}
```

- [ ] **Step 5: Run tests + gate**

Run: `cargo test --lib db::user_data && cargo clippy --all-targets -- -D warnings`
Expected: 4 tests pass, clippy clean.

- [ ] **Step 6: Commit**

```bash
git add src/db/mod.rs src/db/user_data.rs
git commit -m "feat(db): add user_data persistence (rating/watched, imdb_id keyed)"
```

---

### Task 3: Routes — rating/watched endpoints

**Files:**
- Create: `src/routes/user_data.rs`
- Modify: `src/routes/mod.rs:1-3` (add `pub mod user_data;`) and `src/routes/mod.rs:10-21` (`configure`: add 3 routes)

**Interfaces:**
- Consumes: `crate::db::user_data::{self, KeyLookup}`; `web::Data<SqlitePool>`.
- Produces: handlers `set_rating`, `clear_rating`, `set_watched` (referenced in `configure`).

**Context:** Existing handlers (e.g. `catalogue::get_catalogue`) take `pool: web::Data<SqlitePool>`. Verbs are explicit-desired-state (idempotent). Response carries the canonical field so the client reconciles to truth.

- [ ] **Step 1: Declare the module**

In `src/routes/mod.rs`, add after line 3 (`pub mod sync;`):

```rust
pub mod user_data;
```

- [ ] **Step 2: Write the failing route tests**

Create `src/routes/user_data.rs`. Start with imports + the test module (won't compile until handlers exist):

```rust
use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::db::user_data::{self, KeyLookup};

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{test, App};
    use crate::db::init_pool;

    async fn fresh_pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        (pool, dir)
    }

    async fn insert_title(pool: &SqlitePool, imdb: Option<&str>) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO titles (imdb_id, title, year, type) VALUES (?, 'T', 2020, 'movie') RETURNING id",
        )
        .bind(imdb).fetch_one(pool).await.unwrap()
    }

    // Register only the three user-data routes — avoids spelling the App<impl
    // ServiceFactory<...>> type and keeps the test app isolated from other
    // handlers' Data deps (SyncRunner, Embedder, AskModel).
    fn test_routes(cfg: &mut web::ServiceConfig) {
        cfg.route("/api/titles/{id}/rating", web::put().to(set_rating))
            .route("/api/titles/{id}/rating", web::delete().to(clear_rating))
            .route("/api/titles/{id}/watched", web::put().to(set_watched));
    }

    #[actix_web::test]
    async fn put_rating_ok() {
        let (pool, _dir) = fresh_pool().await;
        let id = insert_title(&pool, Some("tt100")).await;
        let app = test::init_service(
            App::new().app_data(web::Data::new(pool)).configure(test_routes),
        ).await;
        let req = test::TestRequest::put()
            .uri(&format!("/api/titles/{id}/rating"))
            .set_json(serde_json::json!({ "rating": 7 }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["rating"], 7);
    }

    #[actix_web::test]
    async fn put_rating_out_of_range_is_400() {
        let (pool, _dir) = fresh_pool().await;
        let id = insert_title(&pool, Some("tt100")).await;
        let app = test::init_service(
            App::new().app_data(web::Data::new(pool)).configure(test_routes),
        ).await;
        for bad in [0, 11] {
            let req = test::TestRequest::put()
                .uri(&format!("/api/titles/{id}/rating"))
                .set_json(serde_json::json!({ "rating": bad }))
                .to_request();
            assert_eq!(test::call_service(&app, req).await.status(), 400);
        }
    }

    #[actix_web::test]
    async fn put_rating_unknown_title_is_404() {
        let (pool, _dir) = fresh_pool().await;
        let app = test::init_service(
            App::new().app_data(web::Data::new(pool)).configure(test_routes),
        ).await;
        let req = test::TestRequest::put()
            .uri("/api/titles/4242/rating")
            .set_json(serde_json::json!({ "rating": 5 }))
            .to_request();
        assert_eq!(test::call_service(&app, req).await.status(), 404);
    }

    #[actix_web::test]
    async fn put_rating_null_imdb_is_422() {
        let (pool, _dir) = fresh_pool().await;
        let id = insert_title(&pool, None).await;
        let app = test::init_service(
            App::new().app_data(web::Data::new(pool)).configure(test_routes),
        ).await;
        let req = test::TestRequest::put()
            .uri(&format!("/api/titles/{id}/rating"))
            .set_json(serde_json::json!({ "rating": 5 }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 422);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["error"], "no_imdb_id");
    }

    #[actix_web::test]
    async fn delete_rating_ok() {
        let (pool, _dir) = fresh_pool().await;
        let id = insert_title(&pool, Some("tt100")).await;
        user_data::set_rating(&pool, "tt100", 8).await.unwrap();
        let app = test::init_service(
            App::new().app_data(web::Data::new(pool.clone())).configure(test_routes),
        ).await;
        let req = test::TestRequest::delete()
            .uri(&format!("/api/titles/{id}/rating"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert!(body["rating"].is_null());
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_ratings")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 0);
    }

    #[actix_web::test]
    async fn put_watched_ok() {
        let (pool, _dir) = fresh_pool().await;
        let id = insert_title(&pool, Some("tt100")).await;
        let app = test::init_service(
            App::new().app_data(web::Data::new(pool)).configure(test_routes),
        ).await;
        let req = test::TestRequest::put()
            .uri(&format!("/api/titles/{id}/watched"))
            .set_json(serde_json::json!({ "watched": true }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["watched"], true);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib routes::user_data`
Expected: FAIL to compile — handlers `set_rating`/`clear_rating`/`set_watched` not found.

- [ ] **Step 4: Write the handlers**

Insert above the `#[cfg(test)]` block in `src/routes/user_data.rs`:

```rust
#[derive(Deserialize)]
pub struct RatingBody {
    pub rating: i64,
}

#[derive(Deserialize)]
pub struct WatchedBody {
    pub watched: bool,
}

/// Resolve the title id to its key, or return the appropriate early response
/// (404 unknown title, 422 no `imdb_id`, 500 on DB error).
async fn resolve_or_respond(pool: &SqlitePool, id: i64) -> Result<String, HttpResponse> {
    match user_data::resolve_key(pool, id).await {
        Ok(KeyLookup::Key(k)) => Ok(k),
        Ok(KeyLookup::NoImdbId) => Err(HttpResponse::UnprocessableEntity()
            .json(serde_json::json!({ "error": "no_imdb_id" }))),
        Ok(KeyLookup::NotFound) => Err(HttpResponse::NotFound().finish()),
        Err(e) => {
            tracing::error!("resolve_key failed: {e:#}");
            Err(HttpResponse::InternalServerError().finish())
        }
    }
}

/// `PUT /api/titles/{id}/rating` — set the 1-10 rating.
pub async fn set_rating(
    pool: web::Data<SqlitePool>,
    path: web::Path<i64>,
    body: web::Json<RatingBody>,
) -> impl Responder {
    let rating = body.rating;
    if !(1..=10).contains(&rating) {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "error": "rating must be between 1 and 10" }));
    }
    let key = match resolve_or_respond(pool.get_ref(), path.into_inner()).await {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    match user_data::set_rating(pool.get_ref(), &key, rating).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "rating": rating })),
        Err(e) => {
            tracing::error!("set_rating failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

/// `DELETE /api/titles/{id}/rating` — clear the rating.
pub async fn clear_rating(pool: web::Data<SqlitePool>, path: web::Path<i64>) -> impl Responder {
    let key = match resolve_or_respond(pool.get_ref(), path.into_inner()).await {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    match user_data::clear_rating(pool.get_ref(), &key).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "rating": null })),
        Err(e) => {
            tracing::error!("clear_rating failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

/// `PUT /api/titles/{id}/watched` — set watched state.
pub async fn set_watched(
    pool: web::Data<SqlitePool>,
    path: web::Path<i64>,
    body: web::Json<WatchedBody>,
) -> impl Responder {
    let watched = body.watched;
    let key = match resolve_or_respond(pool.get_ref(), path.into_inner()).await {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    match user_data::set_watched(pool.get_ref(), &key, watched).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "watched": watched })),
        Err(e) => {
            tracing::error!("set_watched failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}
```

- [ ] **Step 5: Wire the routes into `configure`**

In `src/routes/mod.rs`, inside the `web::scope("/api")` chain in `configure` (after the `/sync/status` route), add:

```rust
            .route("/titles/{id}/rating", web::put().to(user_data::set_rating))
            .route("/titles/{id}/rating", web::delete().to(user_data::clear_rating))
            .route("/titles/{id}/watched", web::put().to(user_data::set_watched))
```

- [ ] **Step 6: Run tests + gate**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all backend tests pass (including the 7 new route tests), clippy clean.

- [ ] **Step 7: Commit**

```bash
git add src/routes/mod.rs src/routes/user_data.rs
git commit -m "feat(api): add rating/watched write endpoints"
```

---

### Task 4: Frontend API client

**Files:**
- Create: `frontend/src/api/userData.ts`

**Interfaces:**
- Produces (used by Task 5):
  - `setRating(id: number, rating: number): Promise<{ rating: number | null }>`
  - `clearRating(id: number): Promise<{ rating: null }>`
  - `setWatched(id: number, watched: boolean): Promise<{ watched: boolean }>`

**Context:** Mirrors `frontend/src/api/sync.ts` (plain `fetch`, throw on non-2xx).

- [ ] **Step 1: Write the client**

Create `frontend/src/api/userData.ts`:

```ts
function errMsg(status: number): string {
  if (status === 422) return 'No IMDb match — can’t save for this title.'
  return `Request failed (HTTP ${status})`
}

export async function setRating(id: number, rating: number): Promise<{ rating: number | null }> {
  const res = await fetch(`/api/titles/${id}/rating`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ rating }),
  })
  if (!res.ok) throw new Error(errMsg(res.status))
  return (await res.json()) as { rating: number | null }
}

export async function clearRating(id: number): Promise<{ rating: null }> {
  const res = await fetch(`/api/titles/${id}/rating`, { method: 'DELETE' })
  if (!res.ok) throw new Error(errMsg(res.status))
  return (await res.json()) as { rating: null }
}

export async function setWatched(id: number, watched: boolean): Promise<{ watched: boolean }> {
  const res = await fetch(`/api/titles/${id}/watched`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ watched }),
  })
  if (!res.ok) throw new Error(errMsg(res.status))
  return (await res.json()) as { watched: boolean }
}
```

- [ ] **Step 2: Type-check**

Run (in `frontend/`): `npm run build`
Expected: build succeeds (no vue-tsc errors). No test yet — this module is exercised via the store tests in Task 5.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/api/userData.ts
git commit -m "feat(frontend): add userData API client"
```

---

### Task 5: Store — async optimistic actions

**Files:**
- Modify: `frontend/src/stores/catalogue.ts` (State interface ~line 11-29; initial state ~line 32-50; actions `toggleWatched`/`setRating` at lines 105-107)
- Test: `frontend/src/stores/__tests__/userData.test.ts` (create)

**Interfaces:**
- Consumes: `setRating`, `clearRating`, `setWatched` from `@/api/userData` (Task 4).
- Produces: `toggleWatched(id)`, `setRating(id, n)`, `clearRating(id)` (all async); `userDataError: string | null` state.

- [ ] **Step 1: Write the failing tests**

Create `frontend/src/stores/__tests__/userData.test.ts`:

```ts
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useCatalogueStore } from '@/stores/catalogue'

vi.mock('@/api/userData', () => ({
  setRating: vi.fn(),
  clearRating: vi.fn(),
  setWatched: vi.fn(),
}))
import { setRating, clearRating, setWatched } from '@/api/userData'

beforeEach(() => {
  setActivePinia(createPinia())
  vi.clearAllMocks()
})

describe('catalogue store — user-data actions', () => {
  it('toggleWatched optimistically sets and reconciles on success', async () => {
    vi.mocked(setWatched).mockResolvedValue({ watched: true })
    const store = useCatalogueStore()
    await store.toggleWatched(5)
    expect(setWatched).toHaveBeenCalledWith(5, true)
    expect(store.isWatched(5)).toBe(true)
    expect(store.userDataError).toBeNull()
  })

  it('toggleWatched rolls back and records error on failure', async () => {
    vi.mocked(setWatched).mockRejectedValue(new Error('boom'))
    const store = useCatalogueStore()
    await store.toggleWatched(5)
    expect(store.isWatched(5)).toBe(false) // reverted
    expect(store.userDataError).toBe('boom')
  })

  it('setRating optimistically sets and rolls back on failure', async () => {
    vi.mocked(setRating).mockRejectedValue(new Error('nope'))
    const store = useCatalogueStore()
    store.ratings[5] = 3
    await store.setRating(5, 8)
    expect(store.ratingOf(5)).toBe(3) // rolled back to previous
    expect(store.userDataError).toBe('nope')
  })

  it('clearRating removes the rating on success', async () => {
    vi.mocked(clearRating).mockResolvedValue({ rating: null })
    const store = useCatalogueStore()
    store.ratings[5] = 9
    await store.clearRating(5)
    expect(store.ratingOf(5)).toBeNull()
    expect(clearRating).toHaveBeenCalledWith(5)
  })
})
```

- [ ] **Step 2: Run tests to verify they fail**

Run (in `frontend/`): `npm test -- userData`
Expected: FAIL — `clearRating` undefined / `userDataError` undefined / actions not async.

- [ ] **Step 3: Add the import + state field**

In `frontend/src/stores/catalogue.ts`:

Add the import near the top (after the existing `import { askService } from '@/services'`):

```ts
import { setRating as apiSetRating, clearRating as apiClearRating, setWatched as apiSetWatched } from '@/api/userData'
```

Add to the `State` interface (after `askError: string | null`):

```ts
  userDataError: string | null
```

Add to the initial state object (after `askError: null,`):

```ts
    userDataError: null,
```

- [ ] **Step 4: Replace the stub actions**

Replace lines 105-107 (`toggleWatched`, `setRating`, and the comment) with:

```ts
    async toggleWatched(id: number) {
      this.userDataError = null
      const prev = !!this.watched[id]
      const next = !prev
      this.watched[id] = next // optimistic
      try {
        const r = await apiSetWatched(id, next)
        this.watched[id] = r.watched // reconcile to server truth
      } catch (e) {
        this.watched[id] = prev // rollback
        this.userDataError = e instanceof Error ? e.message : 'Could not update watched state.'
      }
    },

    async setRating(id: number, n: number) {
      this.userDataError = null
      const prev = this.ratings[id]
      this.ratings[id] = n // optimistic
      try {
        const r = await apiSetRating(id, n)
        if (r.rating != null) this.ratings[id] = r.rating
      } catch (e) {
        if (prev == null) delete this.ratings[id]
        else this.ratings[id] = prev
        this.userDataError = e instanceof Error ? e.message : 'Could not save rating.'
      }
    },

    async clearRating(id: number) {
      this.userDataError = null
      const prev = this.ratings[id]
      delete this.ratings[id] // optimistic
      try {
        await apiClearRating(id)
      } catch (e) {
        if (prev != null) this.ratings[id] = prev
        this.userDataError = e instanceof Error ? e.message : 'Could not clear rating.'
      }
    },
```

- [ ] **Step 5: Run tests + gate**

Run (in `frontend/`): `npm test -- userData && npm run build`
Expected: 4 tests pass, build clean.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/stores/catalogue.ts frontend/src/stores/__tests__/userData.test.ts
git commit -m "feat(frontend): persist rating/watched via optimistic store actions"
```

---

### Task 6: `StarRating` — 1–10 pips, clear gesture, disabled

**Files:**
- Modify: `frontend/src/components/StarRating.vue`
- Test: `frontend/src/components/__tests__/StarRating.test.ts` (rewrite)

**Interfaces:**
- Produces: `StarRating` props `{ value: number | null; disabled?: boolean }`; emits `set: [n]` and `clear: []`.

- [ ] **Step 1: Rewrite the test**

Replace the contents of `frontend/src/components/__tests__/StarRating.test.ts`:

```ts
import { mount } from '@vue/test-utils'
import { describe, it, expect } from 'vitest'
import StarRating from '../StarRating.vue'

describe('StarRating', () => {
  it('renders 10 pips and emits set with the clicked index (1-based)', async () => {
    const w = mount(StarRating, { props: { value: null } })
    const stars = w.findAll('[data-test="star"]')
    expect(stars).toHaveLength(10)
    await stars[6].trigger('click')
    expect(w.emitted('set')?.[0]).toEqual([7])
  })

  it('marks pips up to value as filled', () => {
    const w = mount(StarRating, { props: { value: 7 } })
    expect(w.findAll('[data-test="star"].filled')).toHaveLength(7)
  })

  it('clicking the current value emits clear instead of set', async () => {
    const w = mount(StarRating, { props: { value: 7 } })
    await w.findAll('[data-test="star"]')[6].trigger('click') // the 7th pip
    expect(w.emitted('clear')).toBeTruthy()
    expect(w.emitted('set')).toBeUndefined()
  })

  it('disabled blocks all emits', async () => {
    const w = mount(StarRating, { props: { value: null, disabled: true } })
    await w.findAll('[data-test="star"]')[3].trigger('click')
    expect(w.emitted('set')).toBeUndefined()
    expect(w.emitted('clear')).toBeUndefined()
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run (in `frontend/`): `npm test -- StarRating`
Expected: FAIL — currently renders 5 stars, no `clear`/`disabled`.

- [ ] **Step 3: Rewrite the component**

Replace `frontend/src/components/StarRating.vue`:

```vue
<script setup lang="ts">
const props = defineProps<{ value: number | null; disabled?: boolean }>()
const emit = defineEmits<{ set: [n: number]; clear: [] }>()

const STARS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

function click(n: number) {
  if (props.disabled) return
  if (n === props.value) emit('clear')
  else emit('set', n)
}
</script>

<template>
  <div class="star-row" :class="{ 'star-row--disabled': props.disabled }">
    <button
      v-for="n in STARS"
      :key="n"
      data-test="star"
      :disabled="props.disabled"
      :class="['star', { filled: props.value !== null && n <= props.value }]"
      @click="click(n)"
    >★</button>
  </div>
</template>

<style scoped>
.star-row {
  display: flex;
  gap: 3px;
}

.star {
  background: none;
  border: none;
  padding: 0;
  cursor: pointer;
  font-size: 20px;
  line-height: 1;
  color: var(--star-empty, #3a3f4a);
  transition: color 0.1s ease;
}

.star.filled {
  color: var(--star-filled, #f5c518);
}

.star:not(:disabled):hover {
  color: var(--star-filled, #f5c518);
}

.star-row--disabled .star {
  cursor: default;
  opacity: 0.5;
}
</style>
```

- [ ] **Step 4: Run test + gate**

Run (in `frontend/`): `npm test -- StarRating && npm run build`
Expected: 4 tests pass, build clean.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/StarRating.vue frontend/src/components/__tests__/StarRating.test.ts
git commit -m "feat(frontend): StarRating 1-10 pips with clear and disabled"
```

---

### Task 7: DetailView — wire async actions + disabled state

**Files:**
- Modify: `frontend/src/views/DetailView.vue` (script lines 29-34; template watched button lines 70-77 and StarRating lines 82-85)
- Test: `frontend/src/views/__tests__/DetailWatched.test.ts` (rewrite for async + 10 pips + disabled)

**Interfaces:**
- Consumes: store `toggleWatched`/`setRating`/`clearRating` (Task 5); `StarRating` `@clear` + `:disabled` (Task 6); `Title.imdbId`.

**Context:** `Title` already carries `imdbId: string | null` (`frontend/src/types.ts`) — no type change needed. `PosterGrid` only reads `isWatched`, so DetailView is the only mutator call site.

- [ ] **Step 1: Rewrite the test**

Replace `frontend/src/views/__tests__/DetailWatched.test.ts`:

```ts
import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import DetailView from '../DetailView.vue'
import { useCatalogueStore } from '@/stores/catalogue'
import type { Title } from '@/types'

vi.mock('@/api/userData', () => ({
  setRating: vi.fn().mockResolvedValue({ rating: 7 }),
  clearRating: vi.fn().mockResolvedValue({ rating: null }),
  setWatched: vi.fn().mockResolvedValue({ watched: true }),
}))

function makeTitle(over: Partial<Title> = {}): Title {
  return {
    id: 5, imdbId: 'tt0000005', title: 'Coco', year: 2017, services: ['disney'],
    type: 'movie', genres: ['Animation'], imdb: 8.4, len: '105 min',
    desc: 'A boy and music.', cast: ['Anthony Gonzalez'], watched: false, rating: null,
    ...over,
  }
}

beforeEach(() => setActivePinia(createPinia()))

async function mountDetail(title: Title) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/', component: { template: '<div>home</div>' } },
      { path: '/title/:id', component: DetailView },
    ],
  })
  const store = useCatalogueStore()
  store.catalogue = [title]
  router.push(`/title/${title.id}`)
  await router.isReady()
  const wrapper = mount(DetailView, { global: { plugins: [router] } })
  await flushPromises()
  return { wrapper, store }
}

describe('DetailView — watched/rating store wiring', () => {
  it('mark-watched persists and updates the store', async () => {
    const { wrapper, store } = await mountDetail(makeTitle())
    await wrapper.find('[data-test="mark-watched"]').trigger('click')
    await flushPromises()
    expect(store.isWatched(5)).toBe(true)
    expect(Object.keys(store.watched)).not.toContain('[object Object]')
  })

  it('clicking a star persists the rating', async () => {
    const { wrapper, store } = await mountDetail(makeTitle())
    const stars = wrapper.findAll('[data-test="star"]')
    expect(stars).toHaveLength(10)
    await stars[6].trigger('click') // 7th pip
    await flushPromises()
    expect(store.ratingOf(5)).toBe(7)
    expect(Object.keys(store.ratings)).not.toContain('[object Object]')
  })

  it('disables controls when the title has no imdbId', async () => {
    const { wrapper } = await mountDetail(makeTitle({ imdbId: null }))
    expect(wrapper.find('[data-test="mark-watched"]').attributes('disabled')).toBeDefined()
    expect(wrapper.find('[data-test="star"]').attributes('disabled')).toBeDefined()
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run (in `frontend/`): `npm test -- DetailWatched`
Expected: FAIL — 10-pip assertion fails (still 5), disabled attribute absent.

- [ ] **Step 3: Update the script**

In `frontend/src/views/DetailView.vue`, replace lines 29-34 (the wrapper block) with:

```ts
// Wrappers that resolve id.value in script scope so the template never
// passes the ComputedRef object itself into store methods.
const idIsWatched = computed(() => store.isWatched(id.value))
const idRating = computed(() => store.ratingOf(id.value))
const canRate = computed(() => !!title.value?.imdbId)
function toggleWatched() { store.toggleWatched(id.value) }
function setRating(n: number) { store.setRating(id.value, n) }
function clearRating() { store.clearRating(id.value) }
```

- [ ] **Step 4: Update the template**

Replace the watched button (lines 70-77) with a disabled-aware version:

```vue
        <button
          data-test="mark-watched"
          class="watched-btn"
          :class="{ 'watched-btn--active': idIsWatched }"
          :disabled="!canRate"
          @click="toggleWatched"
        >
          {{ idIsWatched ? '✓ Watched' : 'Mark as watched' }}
        </button>
```

Replace the rating well (lines 79-86) with the wired StarRating + a hint:

```vue
        <!-- Your rating well -->
        <div class="rating-well">
          <div class="rating-eyebrow">Your rating</div>
          <StarRating
            :value="idRating"
            :disabled="!canRate"
            @set="setRating"
            @clear="clearRating"
          />
          <p v-if="!canRate" class="no-imdb-hint">No IMDb match — can’t save ratings.</p>
        </div>
```

Add to the `<style scoped>` block (after the `.rating-eyebrow` rule):

```css
.no-imdb-hint {
  margin: 9px 0 0;
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 10.5px;
  color: var(--text-faint, #5f6570);
}

.watched-btn:disabled {
  cursor: default;
  opacity: 0.5;
}
```

- [ ] **Step 5: Run test + full frontend gate**

Run (in `frontend/`): `npm test && npm run build`
Expected: all frontend tests pass (including the rewritten DetailWatched + StarRating + store userData), build clean.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/views/DetailView.vue frontend/src/views/__tests__/DetailWatched.test.ts
git commit -m "feat(frontend): wire DetailView rating/watched to persistence; disable for no-imdb titles"
```

---

## Final verification

- [ ] Backend: `cargo test && cargo clippy --all-targets -- -D warnings` — all green.
- [ ] Frontend: `npm test && npm run build` (in `frontend/`) — all green.
- [ ] Manual smoke (optional, needs a running server): rate a title 1–10, reload → rating persists; click the current rating → clears; mark watched → reload persists; un-mark → clears; a title with no IMDb match shows disabled controls + hint.
- [ ] Update `docs/superpowers/deferred-followups.md` if any sub-item was touched (the 1–10 migration makes the deferred IMDb/Plex ratings imports lossless — note that the schema is now import-ready).

## Self-review notes (coverage vs spec)

- Spec §2 migration → Task 1. §3 `db::user_data` → Task 2. §4 routes → Task 3. §5.1 API client → Task 4. §5.2 store → Task 5. §6.4 `StarRating` → Task 6. §6.3 DetailView wiring + disable → Task 7.
- Spec §6.3 "`Title` gains `imdbId`" is **already satisfied** (`frontend/src/types.ts` line 6) — no task; consumed directly in Task 7.
- Spec §3 "update seed rating values" is a **no-op** — `src/db/seed.rs` inserts no `user_ratings` rows; called out in Task 1 context so the implementer doesn't go hunting.
- Decisions D5.1 (422 + disabled) → Tasks 3 & 7; D5.2 (manual-only unwatch) → Task 2 `set_watched` + its test; D5.3 (1–10, 10 pips) → Tasks 1 & 6; D5.4 (clear path) → Tasks 2/3/5/6/7.
