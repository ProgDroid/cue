# IMDb Ratings Import Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user import an IMDb ratings-export CSV into `user_ratings` so IMDb ratings appear in cue.

**Architecture:** A pure Rust parser turns the CSV into validated `RatingImport` rows; a single-transaction DB function upserts them (overwrite-on-conflict, preserving the IMDb `Date Rated`); a `POST /api/import/ratings` endpoint accepts the raw CSV body and returns a `{ imported, skipped, matched }` summary; the Settings page gains a file-upload card that POSTs the file text and re-fetches the catalogue so new ratings surface.

**Tech Stack:** Rust + Actix-web + SQLx (SQLite) + the `csv` crate; Vue 3 + TypeScript + Pinia; Vitest.

**Spec:** `docs/superpowers/specs/2026-06-23-cue-imdb-ratings-import-design.md`

## Global Constraints

- **SQLx runtime queries only** — `sqlx::query` / `query_as` / `query_scalar`. No compile-time `query!` macros (must build with no live `DATABASE_URL`).
- **New backend modules declared in `src/lib.rs`** as `pub mod …`, consumed via the `cue::` path from `src/main.rs`.
- **Test DBs** use `tempfile::tempdir()` (not `NamedTempFile`) with the URL built as `format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"))`; keep the `TempDir` guard bound (`_dir`).
- **Clippy gate:** `cargo clippy --all-targets -- -D warnings`. The `[lints.clippy]` table in `Cargo.toml` is the single source of truth; per-item `#[allow(...)]` only, each with a one-line reason. Never widen the global table.
- **Rating domain is 1–10** (migration `0002`). IMDb `Your Rating` is already 1–10 — no rescaling.
- **No secrets** are touched; this feature reads no external keys.
- **Commits via the Bash tool**, never PowerShell (PowerShell prepends a UTF-8 BOM to commit subjects on this machine).

## File Structure

| File | Responsibility | Action |
|------|----------------|--------|
| `Cargo.toml` | add `csv = "1"` dependency | Modify |
| `src/import/mod.rs` | module root: `pub mod imdb_ratings;` | Create |
| `src/import/imdb_ratings.rs` | pure `parse_ratings` + types + unit tests | Create |
| `src/lib.rs` | declare `pub mod import;` | Modify |
| `src/db/user_data.rs` | `import_ratings` transaction + `ImportOutcome` + tests | Modify |
| `src/routes/import.rs` | `POST /api/import/ratings` handler + tests | Create |
| `src/routes/mod.rs` | declare `pub mod import;`, wire the route with a raised `PayloadConfig` | Modify |
| `frontend/src/api/userData.ts` | `importRatings(csv)` client | Modify |
| `frontend/src/api/__tests__/userData.import.test.ts` | client test | Create |
| `frontend/src/views/SettingsView.vue` | "Import IMDb ratings" card + handler | Modify |
| `frontend/src/views/SettingsView.spec.ts` | import-flow test | Modify |

---

### Task 1: CSV parser (`parse_ratings`)

**Files:**
- Modify: `Cargo.toml` (add `csv = "1"`)
- Create: `src/import/mod.rs`
- Create: `src/import/imdb_ratings.rs`
- Modify: `src/lib.rs` (add `pub mod import;`)

**Interfaces:**
- Produces:
  - `pub struct RatingImport { pub imdb_id: String, pub rating: i64, pub rated_at: Option<String> }`
  - `pub struct ParsedImport { pub rows: Vec<RatingImport>, pub skipped: usize }`
  - `pub fn parse_ratings(csv: &str) -> ParsedImport`

- [ ] **Step 1: Add the `csv` dependency**

In `Cargo.toml`, under `[dependencies]` (after the `reqwest` line), add:

```toml
csv = "1"
```

- [ ] **Step 2: Create the module root**

Create `src/import/mod.rs`:

```rust
pub mod imdb_ratings;
```

- [ ] **Step 3: Declare the module in `lib.rs`**

In `src/lib.rs`, add the line (keep the list alphabetical — insert after `pub mod db;`):

```rust
pub mod import;
```

- [ ] **Step 4: Write the failing parser tests**

Create `src/import/imdb_ratings.rs` with ONLY the types, a stub, and the tests:

```rust
use serde::Deserialize;

/// One importable rating row distilled from the IMDb export.
pub struct RatingImport {
    pub imdb_id: String,
    pub rating: i64,
    /// IMDb `Date Rated` (`YYYY-MM-DD`) when valid; `None` falls back to the DB default.
    pub rated_at: Option<String>,
}

/// Result of parsing an IMDb ratings CSV.
pub struct ParsedImport {
    pub rows: Vec<RatingImport>,
    pub skipped: usize,
}

/// Columns consumed from the IMDb export; header-named so order/extra columns don't matter.
#[derive(Deserialize)]
struct ImdbRow {
    #[serde(rename = "Const")]
    const_id: String,
    #[serde(rename = "Your Rating")]
    your_rating: Option<String>,
    #[serde(rename = "Date Rated")]
    date_rated: Option<String>,
}

/// Parse an IMDb ratings-export CSV into validated rows.
///
/// Rows with a blank `Const` are ignored. Rows whose `Your Rating` is missing,
/// non-integer, or outside 1–10 are counted in `skipped`. A `Date Rated` that
/// is not `YYYY-MM-DD` is dropped to `None`.
#[must_use]
pub fn parse_ratings(_csv: &str) -> ParsedImport {
    ParsedImport { rows: Vec::new(), skipped: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Const,Your Rating,Date Rated,Title,Genres\n\
tt0111161,10,2019-03-14,The Shawshank Redemption,Drama\n\
tt0137523,9,2020-01-02,\"Fight Club\",Drama\n\
tt0110912,8,not-a-date,\"Pulp Fiction, a film\",\"Crime, Drama\"\n\
tt0000001,0,2021-05-05,Bad Low,Drama\n\
tt0000002,11,2021-05-06,Bad High,Drama\n\
tt0000003,,2021-05-07,Unrated,Drama\n";

    #[test]
    fn parses_valid_rows_and_counts_skips() {
        let parsed = parse_ratings(SAMPLE);
        // 3 valid (10, 9, 8); 3 skipped (0, 11, blank).
        assert_eq!(parsed.rows.len(), 3);
        assert_eq!(parsed.skipped, 3);
    }

    #[test]
    fn preserves_quoted_title_and_maps_fields() {
        let parsed = parse_ratings(SAMPLE);
        let pulp = parsed
            .rows
            .iter()
            .find(|r| r.imdb_id == "tt0110912")
            .expect("pulp row present");
        assert_eq!(pulp.rating, 8);
    }

    #[test]
    fn keeps_valid_date_and_drops_invalid() {
        let parsed = parse_ratings(SAMPLE);
        let shawshank = parsed.rows.iter().find(|r| r.imdb_id == "tt0111161").unwrap();
        let pulp = parsed.rows.iter().find(|r| r.imdb_id == "tt0110912").unwrap();
        assert_eq!(shawshank.rated_at.as_deref(), Some("2019-03-14"));
        assert_eq!(pulp.rated_at, None); // "not-a-date" dropped
    }

    #[test]
    fn blank_const_is_ignored_not_skipped() {
        let csv = "Const,Your Rating,Date Rated\n\
,7,2021-01-01\n\
tt0111161,7,2021-01-01\n";
        let parsed = parse_ratings(csv);
        assert_eq!(parsed.rows.len(), 1);
        assert_eq!(parsed.skipped, 0); // blank Const ignored, not counted
    }

    #[test]
    fn empty_or_headerless_yields_no_rows() {
        assert_eq!(parse_ratings("").rows.len(), 0);
        // No recognizable Const column -> every row fails -> no rows.
        assert_eq!(parse_ratings("a,b,c\n1,2,3\n").rows.len(), 0);
    }
}
```

- [ ] **Step 5: Run the tests to verify they fail**

Run: `cargo test --lib import::imdb_ratings`
Expected: FAIL (the stub returns empty; assertions on counts/fields fail).

- [ ] **Step 6: Implement `parse_ratings`**

Replace the stub body and add the date helper:

```rust
/// True when `s` is exactly `YYYY-MM-DD` with plausible month/day values.
fn looks_like_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |range: std::ops::Range<usize>| s[range].bytes().all(|c| c.is_ascii_digit());
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let month: u8 = s[5..7].parse().unwrap_or(0);
    let day: u8 = s[8..10].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

#[must_use]
pub fn parse_ratings(csv: &str) -> ParsedImport {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(csv.as_bytes());

    let mut rows = Vec::new();
    let mut skipped = 0_usize;

    for result in reader.deserialize::<ImdbRow>() {
        let Ok(row) = result else {
            skipped += 1;
            continue;
        };
        let const_id = row.const_id.trim();
        if const_id.is_empty() {
            continue; // ignored, not skipped
        }
        let rating = match row
            .your_rating
            .as_deref()
            .map(str::trim)
            .and_then(|s| s.parse::<i64>().ok())
        {
            Some(r) if (1..=10).contains(&r) => r,
            _ => {
                skipped += 1;
                continue;
            }
        };
        let rated_at = row
            .date_rated
            .as_deref()
            .map(str::trim)
            .filter(|s| looks_like_iso_date(s))
            .map(ToString::to_string);

        rows.push(RatingImport {
            imdb_id: const_id.to_string(),
            rating,
            rated_at,
        });
    }

    ParsedImport { rows, skipped }
}
```

> Note: a row that fails to deserialize (e.g. no `Const` column at all) is counted as `skipped`. For the `empty_or_headerless` test the file has no `Const` header, so the single data row is skipped and `rows` is empty — the assertion only checks `rows.len() == 0`, which holds.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --lib import::imdb_ratings`
Expected: PASS (5 tests).

- [ ] **Step 8: Clippy + commit**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

```bash
git add Cargo.toml Cargo.lock src/import/ src/lib.rs
git commit -m "feat(import): IMDb ratings CSV parser

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Persist imported ratings (`import_ratings`)

**Files:**
- Modify: `src/db/user_data.rs`

**Interfaces:**
- Consumes: `cue::import::imdb_ratings::RatingImport` (from Task 1).
- Produces:
  - `pub struct ImportOutcome { pub imported: usize, pub matched: usize }`
  - `pub async fn import_ratings(pool: &SqlitePool, rows: &[RatingImport]) -> anyhow::Result<ImportOutcome>`

- [ ] **Step 1: Add imports and the outcome type**

At the top of `src/db/user_data.rs`, below `use sqlx::SqlitePool;`, add:

```rust
use std::collections::HashSet;

use crate::import::imdb_ratings::RatingImport;
```

Above the `#[cfg(test)]` module, add:

```rust
/// Summary of an import: rows written and how many resolve to a catalogue title.
pub struct ImportOutcome {
    pub imported: usize,
    pub matched: usize,
}
```

- [ ] **Step 2: Write the failing test**

In the existing `#[cfg(test)] mod tests` of `src/db/user_data.rs`, add (the helpers `fresh_pool` / `insert_title` already exist in this module):

```rust
#[tokio::test]
async fn import_ratings_upserts_overwrites_and_counts_matched() {
    use crate::import::imdb_ratings::RatingImport;
    let (pool, _dir) = fresh_pool().await;
    // tt100 is in the catalogue; tt900 is not.
    insert_title(&pool, Some("tt100")).await;

    // First import sets tt100 -> 5 (with a date) and tt900 -> 7 (no date).
    let first = vec![
        RatingImport { imdb_id: "tt100".into(), rating: 5, rated_at: Some("2018-01-01".into()) },
        RatingImport { imdb_id: "tt900".into(), rating: 7, rated_at: None },
    ];
    let out = import_ratings(&pool, &first).await.unwrap();
    assert_eq!((out.imported, out.matched), (2, 1)); // only tt100 is a catalogue title

    // Re-import overwrites tt100 -> 9 and updates its date.
    let second = vec![RatingImport { imdb_id: "tt100".into(), rating: 9, rated_at: Some("2020-02-02".into()) }];
    import_ratings(&pool, &second).await.unwrap();

    let (rating, rated_at): (i64, String) =
        sqlx::query_as("SELECT rating, rated_at FROM user_ratings WHERE imdb_id = 'tt100'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rating, 9);
    assert_eq!(rated_at, "2020-02-02");

    // tt900 (no date) got the now() default — a non-empty timestamp.
    let t900: String = sqlx::query_scalar("SELECT rated_at FROM user_ratings WHERE imdb_id = 'tt900'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!t900.is_empty());
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --lib db::user_data::tests::import_ratings_upserts_overwrites_and_counts_matched`
Expected: FAIL with "cannot find function `import_ratings`".

- [ ] **Step 4: Implement `import_ratings`**

Add this function above the `#[cfg(test)]` module in `src/db/user_data.rs`:

```rust
/// Bulk-upsert imported ratings in a single transaction (import overwrites on
/// conflict), preserving `rated_at` when supplied. Returns the number written
/// and how many of the imported `imdb_id`s exist in `titles`.
///
/// # Errors
/// Returns an error if any query or the transaction fails.
pub async fn import_ratings(
    pool: &SqlitePool,
    rows: &[RatingImport],
) -> anyhow::Result<ImportOutcome> {
    let mut tx = pool.begin().await?;
    for row in rows {
        sqlx::query(
            "INSERT INTO user_ratings (imdb_id, rating, rated_at)
             VALUES (?, ?, COALESCE(?, datetime('now')))
             ON CONFLICT(imdb_id) DO UPDATE SET
                 rating = excluded.rating,
                 rated_at = excluded.rated_at",
        )
        .bind(&row.imdb_id)
        .bind(row.rating)
        .bind(row.rated_at.as_deref())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    // `matched` = imported ids that exist in the catalogue. titles is small
    // (~5k); intersect in memory to avoid a large dynamic IN clause.
    let imported_ids: HashSet<&str> = rows.iter().map(|r| r.imdb_id.as_str()).collect();
    let title_ids: Vec<String> =
        sqlx::query_scalar("SELECT imdb_id FROM titles WHERE imdb_id IS NOT NULL")
            .fetch_all(pool)
            .await?;
    let matched = title_ids
        .iter()
        .filter(|id| imported_ids.contains(id.as_str()))
        .count();

    Ok(ImportOutcome {
        imported: rows.len(),
        matched,
    })
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --lib db::user_data::tests::import_ratings_upserts_overwrites_and_counts_matched`
Expected: PASS.

- [ ] **Step 6: Clippy + commit**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

```bash
git add src/db/user_data.rs
git commit -m "feat(import): transactional import_ratings upsert + matched count

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: HTTP endpoint `POST /api/import/ratings`

**Files:**
- Create: `src/routes/import.rs`
- Modify: `src/routes/mod.rs`

**Interfaces:**
- Consumes: `cue::import::imdb_ratings::parse_ratings` (Task 1), `cue::db::user_data::import_ratings` (Task 2).
- Produces: `pub async fn import_ratings(pool: web::Data<SqlitePool>, body: web::Bytes) -> impl Responder` mounted at `POST /api/import/ratings`.

- [ ] **Step 1: Write the failing endpoint tests**

Create `src/routes/import.rs` with a stub handler and the tests:

```rust
use actix_web::{web, HttpResponse, Responder};
use sqlx::SqlitePool;

use crate::db::user_data;
use crate::import::imdb_ratings;

/// `POST /api/import/ratings` — import an IMDb ratings-export CSV (raw text body).
pub async fn import_ratings(
    _pool: web::Data<SqlitePool>,
    _body: web::Bytes,
) -> impl Responder {
    HttpResponse::InternalServerError().finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_pool;
    use actix_web::{test, App};

    async fn fresh_pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        (pool, dir)
    }

    fn test_routes(cfg: &mut web::ServiceConfig) {
        cfg.route("/api/import/ratings", web::post().to(import_ratings));
    }

    #[actix_web::test]
    async fn import_happy_path_returns_summary() {
        let (pool, _dir) = fresh_pool().await;
        sqlx::query("INSERT INTO titles (imdb_id, title, year, type) VALUES ('tt0111161','T',1994,'movie')")
            .execute(&pool)
            .await
            .unwrap();
        let app = test::init_service(
            App::new().app_data(web::Data::new(pool)).configure(test_routes),
        )
        .await;

        let csv = "Const,Your Rating,Date Rated,Title\n\
tt0111161,10,2019-03-14,The Shawshank Redemption\n\
tt0137523,9,2020-01-02,Fight Club\n\
tt0000002,11,2021-05-06,Bad High\n";
        let req = test::TestRequest::post()
            .uri("/api/import/ratings")
            .insert_header(("content-type", "text/csv"))
            .set_payload(csv)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["imported"], 2); // 10 and 9 valid; 11 skipped
        assert_eq!(body["skipped"], 1);
        assert_eq!(body["matched"], 1); // only tt0111161 is in titles
    }

    #[actix_web::test]
    async fn empty_body_is_400_no_ratings_found() {
        let (pool, _dir) = fresh_pool().await;
        let app = test::init_service(
            App::new().app_data(web::Data::new(pool)).configure(test_routes),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/api/import/ratings")
            .insert_header(("content-type", "text/csv"))
            .set_payload("")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["error"], "no_ratings_found");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib routes::import`
Expected: FAIL (stub returns 500 for both cases).

- [ ] **Step 3: Implement the handler**

Replace the stub `import_ratings` body in `src/routes/import.rs`:

```rust
/// `POST /api/import/ratings` — import an IMDb ratings-export CSV (raw text body).
pub async fn import_ratings(
    pool: web::Data<SqlitePool>,
    body: web::Bytes,
) -> impl Responder {
    let Ok(csv) = std::str::from_utf8(&body) else {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "error": "invalid_utf8" }));
    };
    let parsed = imdb_ratings::parse_ratings(csv);
    if parsed.rows.is_empty() {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "error": "no_ratings_found" }));
    }
    match user_data::import_ratings(pool.get_ref(), &parsed.rows).await {
        Ok(outcome) => HttpResponse::Ok().json(serde_json::json!({
            "imported": outcome.imported,
            "skipped": parsed.skipped,
            "matched": outcome.matched,
        })),
        Err(e) => {
            tracing::error!("import_ratings failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib routes::import`
Expected: PASS (2 tests).

- [ ] **Step 5: Wire the route into the API scope**

In `src/routes/mod.rs`:

1. Add the module declaration with the others (after `pub mod images;`):

```rust
pub mod import;
```

2. Inside `configure`, register the route on the `/api` scope. Add a `.service(...)` after the `/titles/{id}/backdrop` route, BEFORE the closing `,` of the scope. Replace:

```rust
            .route("/titles/{id}/poster", web::get().to(images::poster))
            .route("/titles/{id}/backdrop", web::get().to(images::backdrop)),
    );
```

with:

```rust
            .route("/titles/{id}/poster", web::get().to(images::poster))
            .route("/titles/{id}/backdrop", web::get().to(images::backdrop))
            .service(
                // Raise the Bytes payload cap above actix's 256 KB default; an
                // IMDb export of several thousand rows can exceed it.
                web::resource("/import/ratings")
                    .app_data(web::PayloadConfig::new(8 * 1024 * 1024))
                    .route(web::post().to(import::import_ratings)),
            ),
    );
```

- [ ] **Step 6: Run the full backend suite**

Run: `cargo test`
Expected: PASS (existing tests + the new parser/db/route tests).

- [ ] **Step 7: Clippy + commit**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

```bash
git add src/routes/import.rs src/routes/mod.rs
git commit -m "feat(import): POST /api/import/ratings endpoint (raised payload cap)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Frontend API client `importRatings`

**Files:**
- Modify: `frontend/src/api/userData.ts`
- Create: `frontend/src/api/__tests__/userData.import.test.ts`

**Interfaces:**
- Produces: `export async function importRatings(csv: string): Promise<{ imported: number; skipped: number; matched: number }>`

- [ ] **Step 1: Write the failing client test**

Create `frontend/src/api/__tests__/userData.import.test.ts`:

```ts
import { describe, it, expect, vi, afterEach } from 'vitest'
import { importRatings } from '@/api/userData'

afterEach(() => vi.restoreAllMocks())

describe('importRatings', () => {
  it('POSTs the CSV as text/csv and returns the summary', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ imported: 2, skipped: 1, matched: 1 }),
    })
    vi.stubGlobal('fetch', fetchMock)

    const result = await importRatings('Const,Your Rating\ntt1,9\n')

    expect(fetchMock).toHaveBeenCalledWith('/api/import/ratings', {
      method: 'POST',
      headers: { 'Content-Type': 'text/csv' },
      body: 'Const,Your Rating\ntt1,9\n',
    })
    expect(result).toEqual({ imported: 2, skipped: 1, matched: 1 })
  })

  it('throws on a non-OK response', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status: 400 }))
    await expect(importRatings('garbage')).rejects.toThrow()
  })
})
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `frontend/`): `npm run test -- userData.import`
Expected: FAIL ("importRatings is not a function" / not exported).

- [ ] **Step 3: Implement `importRatings`**

Append to `frontend/src/api/userData.ts`:

```ts
export async function importRatings(
  csv: string,
): Promise<{ imported: number; skipped: number; matched: number }> {
  const res = await fetch('/api/import/ratings', {
    method: 'POST',
    headers: { 'Content-Type': 'text/csv' },
    body: csv,
  })
  if (!res.ok) throw new Error(errMsg(res.status))
  return (await res.json()) as { imported: number; skipped: number; matched: number }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (from `frontend/`): `npm run test -- userData.import`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/api/userData.ts frontend/src/api/__tests__/userData.import.test.ts
git commit -m "feat(import): importRatings API client

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Settings "Import IMDb ratings" card

**Files:**
- Modify: `frontend/src/views/SettingsView.vue`
- Modify: `frontend/src/views/SettingsView.spec.ts`

**Interfaces:**
- Consumes: `importRatings` (Task 4); `useCatalogueStore().load()` for the post-import re-fetch.

- [ ] **Step 1: Write the failing import-flow test**

Append a new test to `frontend/src/views/SettingsView.spec.ts`. First, add imports at the top of the file (uses real Pinia + a spy — no new dependency):

```ts
import { createPinia, setActivePinia } from 'pinia'
import { useCatalogueStore } from '@/stores/catalogue'
import * as userDataApi from '@/api/userData'
```

Then add the test inside the `describe('SettingsView', …)` block:

```ts
it('imports a ratings file and re-fetches the catalogue', async () => {
  // Initial status load for onMounted.
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve(status) }))
  const importSpy = vi
    .spyOn(userDataApi, 'importRatings')
    .mockResolvedValue({ imported: 5, skipped: 1, matched: 3 })

  const pinia = createPinia()
  setActivePinia(pinia)
  // Spy on the same store instance the component will resolve (same pinia).
  const loadSpy = vi.spyOn(useCatalogueStore(), 'load').mockResolvedValue()

  const wrapper = mount(SettingsView, {
    global: { plugins: [pinia], stubs: { RouterLink: true } },
  })
  await flushPromises()

  const input = wrapper.find('[data-test="imdb-file"]')
  const file = new File(['Const,Your Rating\ntt1,9\n'], 'ratings.csv', { type: 'text/csv' })
  // jsdom's File lacks .text(); stub it.
  Object.defineProperty(file, 'text', { value: () => Promise.resolve('Const,Your Rating\ntt1,9\n') })
  Object.defineProperty(input.element, 'files', { value: [file], configurable: true })
  await input.trigger('change')
  await flushPromises()

  expect(importSpy).toHaveBeenCalledWith('Const,Your Rating\ntt1,9\n')
  expect(wrapper.find('[data-test="imdb-result"]').text()).toContain('Imported 5')
  expect(loadSpy).toHaveBeenCalled()
})
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `frontend/`): `npm run test -- SettingsView`
Expected: FAIL (no `[data-test="imdb-file"]` element).

- [ ] **Step 3: Add the card to the template**

In `frontend/src/views/SettingsView.vue`, inside `<div v-if="status" class="settings__grid">`, after the Catalogue `<div class="card">…</div>`, add a new card:

```vue
      <div class="card">
        <h2>Import IMDb ratings</h2>
        <input
          type="file"
          accept=".csv"
          data-test="imdb-file"
          :disabled="importing"
          @change="onImportFile"
        />
        <p v-if="importMsg" class="muted" data-test="imdb-result">{{ importMsg }}</p>
        <p v-if="importError" class="settings__error">{{ importError }}</p>
      </div>
```

- [ ] **Step 4: Add the handler to the script**

In the `<script setup lang="ts">` block of `SettingsView.vue`:

1. Add imports below the existing ones:

```ts
import { importRatings } from '@/api/userData'
import { useCatalogueStore } from '@/stores/catalogue'
```

2. Add reactive state next to the other `ref`s:

```ts
const importing = ref(false)
const importMsg = ref('')
const importError = ref('')
```

3. Add the handler (above `onMounted(refresh)`):

```ts
async function onImportFile(e: Event) {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  importing.value = true
  importMsg.value = ''
  importError.value = ''
  try {
    const csv = await file.text()
    const r = await importRatings(csv)
    importMsg.value = `Imported ${r.imported} ratings · ${r.matched} in your library · ${r.skipped} rows skipped`
    // Re-fetch so imported ratings surface without a manual reload.
    // Store accessed lazily here (not at setup) so tests that mount without
    // Pinia are unaffected.
    await useCatalogueStore().load()
  } catch (err) {
    importError.value = err instanceof Error ? err.message : 'Import failed.'
  } finally {
    importing.value = false
    input.value = '' // allow re-selecting the same file
  }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run (from `frontend/`): `npm run test -- SettingsView`
Expected: PASS (existing 2 tests + the new import test).

- [ ] **Step 6: Run the full frontend suite + build**

Run (from `frontend/`): `npm run test && npm run build`
Expected: all tests PASS; build succeeds.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/views/SettingsView.vue frontend/src/views/SettingsView.spec.ts
git commit -m "feat(import): Settings card to upload IMDb ratings CSV

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Decision I1 (import everything) → Task 2 writes all rows regardless of catalogue membership; `matched` is informational only. ✓
- Decision I2 (overwrite on conflict) → Task 2 `ON CONFLICT DO UPDATE` + test asserts re-import changes the value. ✓
- Decision I3 (preserve Date Rated) → Task 1 `looks_like_iso_date` + Task 2 `COALESCE(?, datetime('now'))` + tests for valid/invalid date. ✓
- Decision I4 (raw text body + csv crate) → Task 1 adds `csv`; Task 3 uses `web::Bytes`. ✓
- Endpoint `POST /api/import/ratings` returning `{ imported, skipped, matched }` → Task 3. ✓
- 400 `no_ratings_found` on empty → Task 3 test. ✓
- Skip rules (blank Const ignored; bad rating skipped) → Task 1 tests. ✓
- Frontend client + Settings card + catalogue re-fetch → Tasks 4 & 5. ✓
- Testing matrix (parser unit, import_ratings, endpoint, client, SettingsView) → covered across tasks. ✓
- Payload-cap gotcha (actix 256 KB default) → Task 3 Step 5 raises `PayloadConfig`. ✓

**Placeholder scan:** No TBD/TODO; every code step has complete code. ✓

**Type consistency:** `RatingImport { imdb_id, rating, rated_at }`, `ParsedImport { rows, skipped }`, `ImportOutcome { imported, matched }`, and `parse_ratings` / `import_ratings` / `importRatings` signatures are used identically across Tasks 1–5. ✓

## Notes / risks

- **No new test dependency:** Task 5's test uses real Pinia (`createPinia` + `setActivePinia`) and `vi.spyOn(store, 'load')` — no `@pinia/testing` needed.
- **`file.text()` in jsdom:** the test stubs `text()` on the `File` because jsdom may not implement it; the runtime relies on the browser's native `Blob.text()`.
- **Live-verify (post-merge, deferred):** exercise the real flow with an actual IMDb export — confirm the summary counts look right and ratings appear in the grid/detail after the catalogue re-fetch. Add to `deferred-followups.md` when merging.
