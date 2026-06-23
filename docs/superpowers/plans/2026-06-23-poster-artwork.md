# Real Poster + Backdrop Artwork Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace generated oklch placeholder posters/backdrops with real artwork captured during sync, served through the backend so the Plex token never reaches the browser.

**Architecture:** Sync parsers emit a source-tagged `ImageRef` per image; `merge` carries them (preferring public CDN refs); `upsert_title` flattens them into 4 nullable `titles` columns. A new `GET /api/titles/{id}/{poster,backdrop}` endpoint 302-redirects to MOTN public URLs or proxies Plex images with the token injected server-side, else 404. The frontend renders an `<img>` overlaid on the existing placeholder that hides itself on load error.

**Tech Stack:** Rust + Actix-web + SQLx (runtime queries) + SQLite; Vue 3 + TypeScript + Vitest.

**Spec:** `docs/superpowers/specs/2026-06-23-cue-poster-artwork-design.md`

## Global Constraints

- **SQLx runtime queries only** — `sqlx::query` / `query_as` / `query_scalar`. NEVER the `query!` macros.
- **Test DBs:** `tempfile::tempdir()` (NOT `NamedTempFile`); URL = `format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"))`; keep the `TempDir` guard bound as `_dir`.
- **Clippy gate:** `cargo clippy --all-targets -- -D warnings` must pass. Per-item `#[allow(...)]` with a one-line reason only; never widen the global table.
- **Security boundary:** Plex token (`Config::plex_token`) is server-side only — it must NEVER appear in any JSON the client receives, nor in a redirect `Location`. Only the proxy-stream branch may use it, server-to-server.
- **Module declaration:** new backend modules are declared in `src/lib.rs` / their parent `mod.rs`, never only in `main.rs`.
- **Commits via the Bash tool**, not PowerShell (PowerShell prepends a BOM to commit subjects).

---

### Task 1: Migration `0004` — image columns on `titles`

**Files:**
- Create: `migrations/0004_title_artwork.sql`
- Test: covered by existing `init_pool` usage in `src/sync/store.rs` tests (every test runs all migrations on a fresh temp DB).

**Interfaces:**
- Produces: columns `titles.poster_url`, `titles.poster_plex`, `titles.backdrop_url`, `titles.backdrop_plex` (all `TEXT`, nullable).

- [ ] **Step 1: Create the migration**

`migrations/0004_title_artwork.sql` (ensure a trailing newline):

```sql
ALTER TABLE titles ADD COLUMN poster_url    TEXT;
ALTER TABLE titles ADD COLUMN poster_plex   TEXT;
ALTER TABLE titles ADD COLUMN backdrop_url  TEXT;
ALTER TABLE titles ADD COLUMN backdrop_plex TEXT;
```

- [ ] **Step 2: Verify migrations apply**

Run: `cargo test --lib sync::store`
Expected: PASS — the store tests build a fresh DB via `init_pool`, which now runs `0004`. A broken migration would fail every store test.

- [ ] **Step 3: Commit**

```bash
git add migrations/0004_title_artwork.sql
git commit -m "feat(db): migration 0004 — poster/backdrop artwork columns"
```

---

### Task 2: `ImageRef` type + `FetchedTitle`/`MergedTitle` image fields + merge carry-through

**Files:**
- Modify: `src/sync/mod.rs` (add `ImageRef`, add fields to `FetchedTitle`, fix test constructors at ~195 and ~243)
- Modify: `src/sync/merge.rs` (add fields to `MergedTitle`, carry + prefer in `merge`, fix `ft` helper at ~234)
- Modify: `src/sync/plex.rs` (constructor at ~80: emit `None` for now)
- Modify: `src/sync/motn.rs` (constructor at ~163: emit `None` for now)
- Modify: `src/db/motn_cache.rs` (`into_fetched` at ~52 and `sample` test at ~170: emit `None` for now)
- Modify: `src/sync/store.rs` (`merged` helper ~231 and literal ~374: emit `None` for now)

**Interfaces:**
- Produces:
  - `pub struct ImageRef { pub value: String, pub remote: bool }` (derives `Debug, Clone, PartialEq`).
  - `FetchedTitle.poster: Option<ImageRef>`, `FetchedTitle.backdrop: Option<ImageRef>`.
  - `MergedTitle.poster: Option<ImageRef>`, `MergedTitle.backdrop: Option<ImageRef>`.
  - `merge()` preference rule: a `remote: true` ref replaces a stored `remote: false` ref; otherwise first-seen wins.

- [ ] **Step 1: Write the failing test**

Add to `src/sync/merge.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn merge_prefers_remote_image_over_plex_path() {
    use crate::sync::ImageRef;
    let mut plex_first = ft(Some("tt1"), TitleKind::Movie, vec![], vec![Service::Plex]);
    plex_first.poster = Some(ImageRef { value: "/library/p.jpg".into(), remote: false });
    let mut motn_second = ft(Some("tt1"), TitleKind::Movie, vec![], vec![Service::Disney]);
    motn_second.poster = Some(ImageRef { value: "https://cdn/p.jpg".into(), remote: true });

    let out = merge(vec![plex_first, motn_second]);
    assert_eq!(out.len(), 1);
    let p = out[0].poster.as_ref().unwrap();
    assert!(p.remote, "remote CDN ref must win over a Plex path");
    assert_eq!(p.value, "https://cdn/p.jpg");
}

#[test]
fn merge_keeps_remote_when_plex_seen_second() {
    use crate::sync::ImageRef;
    let mut motn_first = ft(Some("tt1"), TitleKind::Movie, vec![], vec![Service::Disney]);
    motn_first.poster = Some(ImageRef { value: "https://cdn/p.jpg".into(), remote: true });
    let mut plex_second = ft(Some("tt1"), TitleKind::Movie, vec![], vec![Service::Plex]);
    plex_second.poster = Some(ImageRef { value: "/library/p.jpg".into(), remote: false });

    let out = merge(vec![motn_first, plex_second]);
    assert!(out[0].poster.as_ref().unwrap().remote);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib sync::merge`
Expected: FAIL — `ImageRef` undefined / `poster` field missing.

- [ ] **Step 3: Add `ImageRef` and `FetchedTitle` fields**

In `src/sync/mod.rs`, above `FetchedTitle`:

```rust
/// One artwork reference plus how the proxy endpoint must serve it.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRef {
    /// Public absolute URL (when `remote`) or a relative Plex path (when not).
    pub value: String,
    /// `true` => public CDN URL (302 redirect); `false` => Plex path (token proxy).
    pub remote: bool,
}
```

Add two fields to `FetchedTitle` (after `services`):

```rust
    pub poster: Option<ImageRef>,
    pub backdrop: Option<ImageRef>,
```

Fix the two `FetchedTitle { .. }` test constructors in `src/sync/mod.rs` (~195, ~243): add `poster: None, backdrop: None,`.

- [ ] **Step 4: Add `MergedTitle` fields + merge carry-through**

In `src/sync/merge.rs`:

Add to `MergedTitle` (after `services`):

```rust
    pub poster: Option<ImageRef>,
    pub backdrop: Option<ImageRef>,
```

Add the import at the top: `use crate::sync::{FetchedTitle, ImageRef};` (extend the existing `use crate::sync::FetchedTitle;`).

Add a free helper near the top of the file:

```rust
/// Prefer a remote (public CDN) ref over a Plex token-proxy ref; else keep `current`.
fn prefer_image(current: Option<ImageRef>, incoming: Option<ImageRef>) -> Option<ImageRef> {
    match (current, incoming) {
        (Some(c), Some(i)) if !c.remote && i.remote => Some(i),
        (Some(c), _) => Some(c),
        (None, x) => x,
    }
}
```

In `merge()`, in the `if let Some(existing) = by_key.get_mut(&key)` branch, after the existing scalar fills add:

```rust
            existing.poster = prefer_image(existing.poster.take(), f.poster);
            existing.backdrop = prefer_image(existing.backdrop.take(), f.backdrop);
```

In the `else` (first-seen) branch's `MergedTitle { .. }` literal (~120), add:

```rust
                    poster: f.poster,
                    backdrop: f.backdrop,
```

Fix the `ft` test helper (~234) `FetchedTitle { .. }`: add `poster: None, backdrop: None,`.

- [ ] **Step 5: Fix the remaining constructors to emit `None`**

- `src/sync/plex.rs` (~80) `FetchedTitle { .. }`: add `poster: None, backdrop: None,`.
- `src/sync/motn.rs` (~163) `FetchedTitle { .. }`: add `poster: None, backdrop: None,`.
- `src/db/motn_cache.rs` `into_fetched` (~52) and `sample` test (~170): add `poster: None, backdrop: None,`.
- `src/sync/store.rs` `merged` helper (~231) and the literal at ~374: add `poster: None, backdrop: None,`.

- [ ] **Step 6: Run tests + clippy**

Run: `cargo test --lib && cargo clippy --all-targets -- -D warnings`
Expected: PASS — all constructors compile; the two new merge tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/sync/mod.rs src/sync/merge.rs src/sync/plex.rs src/sync/motn.rs src/db/motn_cache.rs src/sync/store.rs
git commit -m "feat(sync): ImageRef + poster/backdrop on FetchedTitle/MergedTitle, merge prefers remote"
```

---

### Task 3: `upsert_title` writes image columns

**Files:**
- Modify: `src/sync/store.rs` (`upsert_title` INSERT + UPDATE; add a `split_ref` helper; add a test)

**Interfaces:**
- Consumes: `MergedTitle.poster`/`MergedTitle.backdrop` (Task 2), `titles` image columns (Task 1).
- Produces: persisted `poster_url`/`poster_plex`/`backdrop_url`/`backdrop_plex` per title.

- [ ] **Step 1: Write the failing test**

Add to `src/sync/store.rs` tests:

```rust
#[tokio::test]
async fn upsert_persists_image_columns() {
    use crate::sync::ImageRef;
    let (p, _dir) = pool().await;
    let mut m = merged("tt9", "Img", &[], &[Service::Plex]);
    m.poster = Some(ImageRef { value: "https://cdn/p.jpg".into(), remote: true });
    m.backdrop = Some(ImageRef { value: "/library/b.jpg".into(), remote: false });
    let id = upsert_title(&p, &m).await.unwrap();

    let row: (Option<String>, Option<String>, Option<String>, Option<String>) =
        sqlx::query_as("SELECT poster_url, poster_plex, backdrop_url, backdrop_plex FROM titles WHERE id = ?")
            .bind(id)
            .fetch_one(&p)
            .await
            .unwrap();
    assert_eq!(row.0.as_deref(), Some("https://cdn/p.jpg")); // remote poster -> _url
    assert_eq!(row.1, None);                                  // not the plex column
    assert_eq!(row.2, None);                                  // backdrop not remote
    assert_eq!(row.3.as_deref(), Some("/library/b.jpg"));     // plex backdrop -> _plex
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib sync::store::tests::upsert_persists_image_columns`
Expected: FAIL — columns written as `NULL` (assertions on `row.0`/`row.3` fail).

- [ ] **Step 3: Implement**

Add a helper above `upsert_title` in `src/sync/store.rs`:

```rust
use crate::sync::merge::MergedTitle;
use crate::sync::ImageRef;

/// Split an image ref into `(url_column, plex_column)` values.
fn split_ref(r: &Option<ImageRef>) -> (Option<&str>, Option<&str>) {
    match r {
        Some(i) if i.remote => (Some(i.value.as_str()), None),
        Some(i) => (None, Some(i.value.as_str())),
        None => (None, None),
    }
}
```

(The `MergedTitle` import already exists — only add the `ImageRef` import.)

In `upsert_title`, before the `tx` block compute:

```rust
    let (poster_url, poster_plex) = split_ref(&t.poster);
    let (backdrop_url, backdrop_plex) = split_ref(&t.backdrop);
```

Change the UPDATE SQL to include the columns:

```rust
        sqlx::query(
            "UPDATE titles SET imdb_id = ?, tmdb_id = ?, plex_guid = ?, title = ?, year = ?,
             type = ?, imdb_rating = ?, length = ?, description = ?,
             poster_url = ?, poster_plex = ?, backdrop_url = ?, backdrop_plex = ?,
             updated_at = datetime('now') WHERE id = ?",
        )
```

and add the binds **after** `.bind(&t.description)` and **before** `.bind(id)`:

```rust
        .bind(poster_url)
        .bind(poster_plex)
        .bind(backdrop_url)
        .bind(backdrop_plex)
```

Change the INSERT SQL + binds the same way:

```rust
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO titles (imdb_id, tmdb_id, plex_guid, title, year, type, imdb_rating, length, description, poster_url, poster_plex, backdrop_url, backdrop_plex)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
```

and add the four binds after `.bind(&t.description)`:

```rust
        .bind(poster_url)
        .bind(poster_plex)
        .bind(backdrop_url)
        .bind(backdrop_plex)
```

- [ ] **Step 4: Run tests + clippy**

Run: `cargo test --lib sync::store && cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/sync/store.rs
git commit -m "feat(sync): upsert_title persists poster/backdrop image columns"
```

---

### Task 4: MOTN `imageSet` parsing → remote poster/backdrop refs

**Files:**
- Modify: `src/sync/motn.rs` (`ImageSet` struct, `Show.image_set`, size-pick helper, `show_to_fetched`, extend a test)

**Interfaces:**
- Consumes: `ImageRef` (Task 2).
- Produces: MOTN-sourced titles carry `poster`/`backdrop` as `remote: true`.

> **Size-key note:** MOTN's documented `imageSet` shape is `{ verticalPoster: {wNNN: url…}, horizontalPoster: {wNNN: url…}, … }`. The preference lists below reflect the documented vertical keys (`w240,w360,w480,w600,w720`). **Before finishing this task, capture one real MOTN response** (a `search/filters` page or `/changes` body via the configured key) and confirm the exact key names; adjust the lists if they differ, and save the captured body as a fixture under the test for `parse_page`.

- [ ] **Step 1: Write the failing test**

Add to `src/sync/motn.rs` tests (alongside `parse_page_reads_movie_fields_and_cursor`):

```rust
#[test]
fn parse_page_extracts_image_set() {
    let json = r#"{
      "shows": [{
        "id": "1", "imdbId": "tt1", "title": "X", "showType": "movie",
        "releaseYear": 2020,
        "imageSet": {
          "verticalPoster": { "w240": "https://cdn/v240.jpg", "w480": "https://cdn/v480.jpg" },
          "horizontalPoster": { "w1080": "https://cdn/h1080.jpg" }
        },
        "streamingOptions": {}
      }],
      "hasMore": false
    }"#;
    let (titles, _) = parse_page(json, "gb", &[Service::Disney]).unwrap();
    let t = &titles[0];
    let p = t.poster.as_ref().unwrap();
    assert!(p.remote);
    assert_eq!(p.value, "https://cdn/v480.jpg"); // w480 preferred
    let b = t.backdrop.as_ref().unwrap();
    assert!(b.remote);
    assert_eq!(b.value, "https://cdn/h1080.jpg");
}

#[test]
fn parse_page_handles_missing_image_set() {
    let json = r#"{"shows":[{"id":"1","imdbId":"tt1","title":"X","showType":"movie","releaseYear":2020,"streamingOptions":{}}],"hasMore":false}"#;
    let (titles, _) = parse_page(json, "gb", &[Service::Disney]).unwrap();
    assert!(titles[0].poster.is_none());
    assert!(titles[0].backdrop.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib sync::motn::tests::parse_page_extracts_image_set`
Expected: FAIL — `poster` is `None` (field not parsed yet).

- [ ] **Step 3: Implement**

Add the `ImageRef` import to `src/sync/motn.rs` (extend the existing `use crate::sync::...`): `use crate::sync::{FetchedTitle, ImageRef};` (match the actual current import line).

Add the deserialize struct + helpers (near the other structs):

```rust
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ImageSet {
    #[serde(default)]
    vertical_poster: HashMap<String, String>,
    #[serde(default)]
    horizontal_poster: HashMap<String, String>,
}

/// Documented MOTN vertical poster sizes, most-preferred first.
const POSTER_SIZES: &[&str] = &["w480", "w360", "w600", "w240", "w720"];
/// Horizontal sizes for the backdrop, most-preferred first.
const BACKDROP_SIZES: &[&str] = &["w1080", "w720", "w600", "w480", "w360", "w240"];

/// Pick the first available size from `prefer`, as a remote `ImageRef`.
fn pick_size(m: &HashMap<String, String>, prefer: &[&str]) -> Option<ImageRef> {
    prefer
        .iter()
        .find_map(|k| m.get(*k))
        .map(|value| ImageRef { value: value.clone(), remote: true })
}
```

Add to `Show` (after `streaming_options`):

```rust
    #[serde(default)]
    image_set: Option<ImageSet>,
```

In `show_to_fetched`, before constructing `FetchedTitle`, compute (borrow before the moves):

```rust
    let poster = s.image_set.as_ref().and_then(|set| pick_size(&set.vertical_poster, POSTER_SIZES));
    let backdrop = s.image_set.as_ref().and_then(|set| pick_size(&set.horizontal_poster, BACKDROP_SIZES));
```

and set them in the `FetchedTitle { .. }` literal (replace the `poster: None, backdrop: None,` from Task 2):

```rust
        poster,
        backdrop,
```

- [ ] **Step 4: Run tests + clippy**

Run: `cargo test --lib sync::motn && cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: (when a key is configured) capture a fixture & confirm keys**

If `MOTN_API_KEY` is available, capture one real response and confirm the `verticalPoster`/`horizontalPoster` key names match `POSTER_SIZES`/`BACKDROP_SIZES`; adjust if needed and add the captured body as a `parse_page` fixture. If no key is available, leave a note in the PR/commit body — this becomes part of the post-merge live-verify.

- [ ] **Step 6: Commit**

```bash
git add src/sync/motn.rs
git commit -m "feat(sync): extract MOTN imageSet into remote poster/backdrop refs"
```

---

### Task 5: Plex `thumb`/`art` parsing → Plex poster/backdrop refs

**Files:**
- Modify: `src/sync/plex.rs` (`Meta.thumb`/`Meta.art`, `parse_section`, extend a test)

**Interfaces:**
- Consumes: `ImageRef` (Task 2).
- Produces: Plex-sourced titles carry `poster`/`backdrop` as `remote: false` (relative paths).

- [ ] **Step 1: Write the failing test**

Find the existing `parses_metadata_into_fetched_titles` test fixture in `src/sync/plex.rs` and add `"thumb"`/`"art"` to one item, then assert. Add a focused test:

```rust
#[test]
fn parse_section_extracts_thumb_and_art_as_plex_refs() {
    let json = r#"{"MediaContainer":{"Metadata":[
      {"type":"movie","title":"M","year":2020,
       "thumb":"/library/metadata/1/thumb/9","art":"/library/metadata/1/art/9",
       "Guid":[{"id":"imdb://tt1"}]}
    ]}}"#;
    let out = parse_section(json).unwrap();
    let p = out[0].poster.as_ref().unwrap();
    assert!(!p.remote);
    assert_eq!(p.value, "/library/metadata/1/thumb/9");
    let b = out[0].backdrop.as_ref().unwrap();
    assert!(!b.remote);
    assert_eq!(b.value, "/library/metadata/1/art/9");
}

#[test]
fn parse_section_handles_missing_thumb_art() {
    let json = r#"{"MediaContainer":{"Metadata":[
      {"type":"movie","title":"M","year":2020,"Guid":[{"id":"imdb://tt1"}]}
    ]}}"#;
    let out = parse_section(json).unwrap();
    assert!(out[0].poster.is_none());
    assert!(out[0].backdrop.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib sync::plex::tests::parse_section_extracts_thumb_and_art_as_plex_refs`
Expected: FAIL — `poster` is `None`.

- [ ] **Step 3: Implement**

Add the `ImageRef` import to `src/sync/plex.rs` (extend the existing `use crate::sync::...` line).

Add to `Meta` (after `summary`/before `guid` is fine):

```rust
    thumb: Option<String>,
    art: Option<String>,
```

In `parse_section`'s `FetchedTitle { .. }`, replace the `poster: None, backdrop: None,` (Task 2) with:

```rust
                poster: m.thumb.map(|value| ImageRef { value, remote: false }),
                backdrop: m.art.map(|value| ImageRef { value, remote: false }),
```

- [ ] **Step 4: Run tests + clippy**

Run: `cargo test --lib sync::plex && cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/sync/plex.rs
git commit -m "feat(sync): map Plex thumb/art to relative poster/backdrop refs"
```

---

### Task 6: MOTN delta cache preserves artwork

**Files:**
- Modify: `src/db/motn_cache.rs` (`CachedTitle` fields, `From<&FetchedTitle>`, `into_fetched`, extend a test)

**Interfaces:**
- Consumes: `ImageRef` (Task 2).
- Produces: `/changes`-reconstructed MOTN titles keep their (remote) poster/backdrop across delta syncs.

- [ ] **Step 1: Write the failing test**

Add to `src/db/motn_cache.rs` tests (the module already has a `sample()` helper):

```rust
#[test]
fn cached_round_trip_preserves_remote_images() {
    use crate::sync::ImageRef;
    let mut ft = sample();
    ft.poster = Some(ImageRef { value: "https://cdn/p.jpg".into(), remote: true });
    ft.backdrop = Some(ImageRef { value: "https://cdn/b.jpg".into(), remote: true });

    let cached = CachedTitle::from(&ft);
    let json = serde_json::to_string(&cached).unwrap();
    let back: CachedTitle = serde_json::from_str(&json).unwrap();
    let rebuilt = back.into_fetched();

    assert_eq!(rebuilt.poster, ft.poster);
    assert_eq!(rebuilt.backdrop, ft.backdrop);
}

#[test]
fn cached_defaults_images_for_old_payloads() {
    // A payload written before this feature has no image keys.
    let json = r#"{"imdb_id":"tt1","tmdb_id":null,"title":"X","year":2020,"kind":"movie","imdb_rating":null,"length":null,"description":null,"genres":[],"cast":[],"services":[]}"#;
    let back: CachedTitle = serde_json::from_str(json).unwrap();
    let rebuilt = back.into_fetched();
    assert!(rebuilt.poster.is_none());
    assert!(rebuilt.backdrop.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib db::motn_cache`
Expected: FAIL — `CachedTitle` has no image fields; round-trip drops them.

- [ ] **Step 3: Implement**

Add the `ImageRef` import to `src/db/motn_cache.rs` (extend the existing `use crate::sync::FetchedTitle;`).

Add to `CachedTitle` (after `services`):

```rust
    #[serde(default)]
    pub poster_url: Option<String>,
    #[serde(default)]
    pub backdrop_url: Option<String>,
```

In `From<&FetchedTitle>`, add:

```rust
            poster_url: t.poster.as_ref().filter(|i| i.remote).map(|i| i.value.clone()),
            backdrop_url: t.backdrop.as_ref().filter(|i| i.remote).map(|i| i.value.clone()),
```

In `into_fetched`, replace the `poster: None, backdrop: None,` (Task 2) with:

```rust
            poster: self.poster_url.map(|value| ImageRef { value, remote: true }),
            backdrop: self.backdrop_url.map(|value| ImageRef { value, remote: true }),
```

- [ ] **Step 4: Run tests + clippy**

Run: `cargo test --lib db::motn_cache && cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/db/motn_cache.rs
git commit -m "feat(sync): preserve MOTN poster/backdrop across /changes delta cache"
```

---

### Task 7: Image proxy endpoint + app wiring

**Files:**
- Create: `src/routes/images.rs`
- Modify: `src/routes/mod.rs` (declare `pub mod images;`, register two routes)
- Modify: `src/main.rs` (register `PlexArt` app data)

**Interfaces:**
- Consumes: `titles` image columns (Tasks 1/3); `Config::plex_url`/`plex_token`.
- Produces: `GET /api/titles/{id}/poster`, `GET /api/titles/{id}/backdrop`; `pub struct PlexArt { pub base_url: Option<String>, pub token: Option<String> }` (derives `Clone`).

- [ ] **Step 1: Write the failing test**

Create `src/routes/images.rs` with only the test module first (it won't compile until Step 3 — that's the failing state):

```rust
#[cfg(test)]
mod tests {
    use actix_web::{test, web, App};
    use sqlx::SqlitePool;

    use crate::db::init_pool;
    use crate::routes::{self, images::PlexArt};

    async fn pool_with_title(poster_url: Option<&str>, poster_plex: Option<&str>) -> (SqlitePool, tempfile::TempDir, i64) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO titles (title, year, type, poster_url, poster_plex) VALUES ('T', 2020, 'movie', ?, ?) RETURNING id",
        )
        .bind(poster_url)
        .bind(poster_plex)
        .fetch_one(&pool)
        .await
        .unwrap();
        (pool, dir, id)
    }

    fn app_with(pool: SqlitePool) -> App<impl actix_web::dev::ServiceFactory<actix_web::dev::ServiceRequest, Config = (), Response = actix_web::dev::ServiceResponse, Error = actix_web::Error, InitError = ()>> {
        App::new()
            .app_data(web::Data::new(pool))
            .app_data(web::Data::new(PlexArt { base_url: None, token: None }))
            .configure(routes::configure)
    }

    #[actix_web::test]
    async fn poster_redirects_to_public_url() {
        let (pool, _dir, id) = pool_with_title(Some("https://cdn/p.jpg"), None).await;
        let app = test::init_service(app_with(pool)).await;
        let req = test::TestRequest::get().uri(&format!("/api/titles/{id}/poster")).to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 302);
        assert_eq!(resp.headers().get("location").unwrap().to_str().unwrap(), "https://cdn/p.jpg");
    }

    #[actix_web::test]
    async fn missing_title_is_404() {
        let (pool, _dir, _id) = pool_with_title(None, None).await;
        let app = test::init_service(app_with(pool)).await;
        let req = test::TestRequest::get().uri("/api/titles/999999/poster").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 404);
    }

    #[actix_web::test]
    async fn title_without_art_is_404() {
        let (pool, _dir, id) = pool_with_title(None, None).await;
        let app = test::init_service(app_with(pool)).await;
        let req = test::TestRequest::get().uri(&format!("/api/titles/{id}/poster")).to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 404);
    }
}
```

> If the `app_with` return-type signature is awkward, inline the `App::new()...` builder in each test instead of the helper — the three tests are short.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib routes::images`
Expected: FAIL to compile — `PlexArt` / handlers not defined, routes not registered.

- [ ] **Step 3: Implement the handlers**

Prepend to `src/routes/images.rs` (above the test module):

```rust
//! Artwork proxy: 302-redirect to public CDN URLs (MOTN) or stream Plex images
//! with the token injected server-side. The Plex token never reaches the client.

use actix_web::{web, HttpResponse, Responder};
use sqlx::SqlitePool;

/// Plex base URL + token for the proxy-stream branch. Held in app data so the
/// token stays server-side (never serialized into any response).
#[derive(Clone)]
pub struct PlexArt {
    pub base_url: Option<String>,
    pub token: Option<String>,
}

#[derive(Clone, Copy)]
enum Kind {
    Poster,
    Backdrop,
}

impl Kind {
    const fn columns(self) -> (&'static str, &'static str) {
        match self {
            Self::Poster => ("poster_url", "poster_plex"),
            Self::Backdrop => ("backdrop_url", "backdrop_plex"),
        }
    }
}

const CACHE: (&str, &str) = ("Cache-Control", "public, max-age=86400");

async fn serve(id: i64, kind: Kind, pool: &SqlitePool, plex: &PlexArt) -> HttpResponse {
    let (url_col, plex_col) = kind.columns();
    let sql = format!("SELECT {url_col}, {plex_col} FROM titles WHERE id = ?");
    let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await;
    let Ok(Some((url, plex_path))) = row else {
        return HttpResponse::NotFound().finish();
    };

    if let Some(u) = url {
        return HttpResponse::Found()
            .insert_header(("Location", u))
            .insert_header(CACHE)
            .finish();
    }

    let Some(path) = plex_path else {
        return HttpResponse::NotFound().finish();
    };
    let (Some(base), Some(token)) = (plex.base_url.as_ref(), plex.token.as_ref()) else {
        return HttpResponse::NotFound().finish();
    };
    let upstream = format!("{}{}?X-Plex-Token={}", base.trim_end_matches('/'), path, token);
    match reqwest::get(&upstream).await {
        Ok(resp) if resp.status().is_success() => {
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("image/jpeg")
                .to_owned();
            match resp.bytes().await {
                Ok(bytes) => HttpResponse::Ok()
                    .insert_header(CACHE)
                    .content_type(content_type)
                    .body(bytes),
                Err(_) => HttpResponse::BadGateway().finish(),
            }
        }
        _ => HttpResponse::BadGateway().finish(),
    }
}

pub async fn poster(
    id: web::Path<i64>,
    pool: web::Data<SqlitePool>,
    plex: web::Data<PlexArt>,
) -> impl Responder {
    serve(id.into_inner(), Kind::Poster, pool.get_ref(), plex.get_ref()).await
}

pub async fn backdrop(
    id: web::Path<i64>,
    pool: web::Data<SqlitePool>,
    plex: web::Data<PlexArt>,
) -> impl Responder {
    serve(id.into_inner(), Kind::Backdrop, pool.get_ref(), plex.get_ref()).await
}
```

In `src/routes/mod.rs`: add `pub mod images;` to the module list and register the routes inside the `/api` scope (after the existing `/titles/{id}/watched` route):

```rust
            .route("/titles/{id}/poster", web::get().to(images::poster))
            .route("/titles/{id}/backdrop", web::get().to(images::backdrop)),
```

(Move the trailing `,`/`)` so the scope still closes correctly.)

In `src/main.rs`, before the `HttpServer::new(move || {` closure, add:

```rust
    let plex_art = cue::routes::images::PlexArt {
        base_url: cfg.plex_url.clone(),
        token: cfg.plex_token.clone(),
    };
```

and inside the closure's `App::new()` chain (next to the other `.app_data`):

```rust
            .app_data(web::Data::new(plex_art.clone()))
```

(`cfg` is in scope where sources are assembled; if it has been moved by then, clone the two strings into locals before the move.)

- [ ] **Step 4: Run tests + clippy**

Run: `cargo test --lib routes::images && cargo clippy --all-targets -- -D warnings`
Expected: PASS (redirect + both 404 branches). The Plex proxy-stream branch is exercised by live-verify (D-PA6).

- [ ] **Step 5: Verify the full backend build/test**

Run: `cargo build && cargo test --lib`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/routes/images.rs src/routes/mod.rs src/main.rs
git commit -m "feat(api): poster/backdrop proxy endpoint (302 CDN or token-streamed Plex)"
```

---

### Task 8: Frontend — `<img>` overlays with placeholder fallback

**Files:**
- Modify: `frontend/src/components/PosterCard.vue`
- Modify: `frontend/src/views/DetailView.vue`
- Modify: `frontend/src/components/__tests__/PosterCard.test.ts`

**Interfaces:**
- Consumes: `GET /api/titles/{id}/poster` and `/backdrop` (Task 7).
- Produces: real art rendered over placeholders at the grid card, detail poster, detail backdrop, and similar-row thumbs; placeholder shown on 404/load error.

- [ ] **Step 1: Update the failing test (PosterCard)**

Replace the first test in `frontend/src/components/__tests__/PosterCard.test.ts` (the one asserting no `<img>`) with:

```ts
  it('renders a lazy <img> pointing at the poster endpoint', () => {
    const w = mount(PosterCard, { props: { title, watched: false } })
    const img = w.find('img')
    expect(img.exists()).toBe(true)
    expect(img.attributes('src')).toBe('/api/titles/7/poster')
    expect(img.attributes('loading')).toBe('lazy')
  })

  it('hides the image (revealing placeholder) when it fails to load', async () => {
    const w = mount(PosterCard, { props: { title, watched: false } })
    await w.find('img').trigger('error')
    expect(w.find('img').isVisible()).toBe(false)
  })
```

- [ ] **Step 2: Run test to verify it fails**

Run (from `frontend/`): `npm run test -- PosterCard`
Expected: FAIL — no `<img>` rendered yet.

- [ ] **Step 3: Implement PosterCard**

In `frontend/src/components/PosterCard.vue` `<script setup>`: change the import to include `ref` and add state:

```ts
import { computed, ref } from 'vue'
```

```ts
const imgFailed = ref(false)
const posterSrc = computed(() => `/api/titles/${props.title.id}/poster`)
```

In the template, inside `<div class="poster" …>`, add the `<img>` immediately after the `poster-mono` div (so it paints over the motif/monogram but the badges/vignette declared later still paint on top):

```html
      <img
        v-show="!imgFailed"
        :src="posterSrc"
        loading="lazy"
        alt=""
        class="poster-img"
        @error="imgFailed = true"
      />
```

In `<style scoped>` add:

```css
.poster-img {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
}
```

- [ ] **Step 4: Run PosterCard test**

Run (from `frontend/`): `npm run test -- PosterCard`
Expected: PASS.

- [ ] **Step 5: Implement DetailView (poster, backdrop, similar thumbs)**

In `frontend/src/views/DetailView.vue` `<script setup>`, add reactive failure flags and the similar-thumb helper:

```ts
import { computed, ref } from 'vue' // ensure `ref` is imported (merge with existing import)
```

```ts
const posterFailed = ref(false)
const backdropFailed = ref(false)
const simFailed = ref<Record<number, boolean>>({})
```

Backdrop — inside `<div class="backdrop" …>`, after `backdrop-mono`:

```html
      <img
        v-show="title && !backdropFailed"
        :src="`/api/titles/${title.id}/backdrop`"
        alt=""
        class="backdrop-img"
        @error="backdropFailed = true"
      />
```

Detail poster — inside `<div class="poster" …>`, after `poster-mono`:

```html
          <img
            v-show="title && !posterFailed"
            :src="`/api/titles/${title.id}/poster`"
            loading="lazy"
            alt=""
            class="poster-img"
            @error="posterFailed = true"
          />
```

Similar thumbs — inside `<div class="sim-poster" …>`, after `sim-poster-mono`:

```html
                <img
                  v-show="!simFailed[sim.id]"
                  :src="`/api/titles/${sim.id}/poster`"
                  loading="lazy"
                  alt=""
                  class="sim-poster-img"
                  @error="simFailed[sim.id] = true"
                />
```

In `<style scoped>` add (all three share `object-fit: cover` over their absolutely-positioned parents — `.backdrop`, `.poster`, `.sim-poster` are already `position: relative`/absolute containers):

```css
.backdrop-img,
.poster-img,
.sim-poster-img {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
}
```

> Verify `.backdrop`, `.poster`, and `.sim-poster` establish a positioning context (they use `position: relative` / `absolute` already in this file). If `.backdrop` is not positioned, add `position: relative;` to it so the absolute `<img>` is contained.

- [ ] **Step 6: Add a DetailView render assertion**

In `frontend/src/views/__tests__/DetailView.test.ts`, locate the existing test that mounts `DetailView` with a title rendered and add assertions (reuse that test's existing mount/harness — do not build a new one):

```ts
    expect(wrapper.find('.poster img').attributes('src')).toContain('/poster')
    expect(wrapper.find('.backdrop img').attributes('src')).toContain('/backdrop')
```

(If the variable is named differently than `wrapper`, match the existing test.)

- [ ] **Step 7: Run the frontend suite + build**

Run (from `frontend/`): `npm run test && npm run build`
Expected: PASS — all tests green, type-check/build clean.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/components/PosterCard.vue frontend/src/views/DetailView.vue frontend/src/components/__tests__/PosterCard.test.ts frontend/src/views/__tests__/DetailView.test.ts
git commit -m "feat(ui): render real poster/backdrop art with placeholder fallback"
```

---

### Task 9: Backlog update + full verification gate

**Files:**
- Modify: `docs/superpowers/deferred-followups.md`

- [ ] **Step 1: Run the full gates**

Run (backend): `cargo test --lib && cargo clippy --all-targets -- -D warnings`
Run (frontend, from `frontend/`): `npm run test && npm run build`
Expected: all PASS.

- [ ] **Step 2: Mark the backlog item done**

In `docs/superpowers/deferred-followups.md`, under "Features", change the "Real poster artwork — now actionable" bullet to a ✅ DONE entry referencing the spec/plan dates and the new endpoint, mirroring the existing ✅ Plan 5 entry style. Add a short "Live-verify (post-merge)" note carrying over the three checks from the spec's Live-verify section (MOTN 302, Plex stream, grid/detail spot-check) and the MOTN size-key/fixture confirmation if it wasn't done in Task 4.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/deferred-followups.md
git commit -m "docs: mark real poster artwork done; record poster live-verify follow-ups"
```

---

## Notes for the implementer

- **Why `<img>` overlay + `v-show`, not `v-if`:** the placeholder is the always-rendered base layer; the image overlays it and removes itself from the visual layer on `error`, so art-less titles and network failures degrade to exactly today's UI with zero placeholder changes.
- **Why fetch art by id, not via the DTO:** `TitleDto`/`fetch_catalogue`/the `Title` type are untouched, so the embeddings path, Ask engine, and catalogue tests carry zero risk — the feature is purely additive.
- **Why a buffered read (not chunked stream) in the proxy:** poster/backdrop images are small; `resp.bytes()` then `.body(bytes)` is simpler than a chunked stream and adequate. Revisit only if large images appear.
- **Plex proxy-stream branch (D-PA6):** unit-tested only for URL construction implicitly; full byte-path verified manually post-merge. The token must never appear in client-visible output.
```
