# Sort Orders — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Real browse ordering (Trending = new-in-30-days → rating), a Relevance sort that keeps Ask answers in engine order, a personal For you sort, and MOTN request accounting surfaced in Settings — with no new MOTN calls.

**Architecture:** Backend adds a pure `newSince` computation to the catalogue DTO, a For you service (in-memory centred vector cache + top-k scoring off the request threads, behind `GET /api/for-you`), MOTN accounting in a new `db::motn_meta` module used by `MotnClient`, and a `motn` block on sync status. Frontend adds the new sort keys and ordering in the Pinia store, the sort-menu changes in `FilterBar`, and a MOTN line in Settings.

**Tech Stack:** Rust (Actix-web 4, SQLx runtime queries, SQLite, tokio), Vue 3 + TypeScript + Pinia, Vitest.

**Spec:** `docs/superpowers/specs/2026-10-02-cue-sort-orders-design.md` (Phase 1 = §§1–8).

## Global Constraints

- SQLx **runtime queries only** (`sqlx::query*`), never `query!` macros (CLAUDE.md). Phase 1 adds **no migration**.
- Declare new Rust modules in `src/lib.rs` / the parent `mod.rs`, not `main.rs`.
- Test DBs: `tempfile::tempdir()` + `format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"))`, keep `_dir` bound.
- In a test module that imports `actix_web::test`, sync `#[test]` fns don't compile — put pure tests in a sibling module.
- Clippy gate `cargo clippy --all-targets -- -D warnings`; per-item `#[allow(clippy::…)] // reason` only, never widen `[lints.clippy]`.
- No MOTN key exists in CI or this environment: MOTN tests use a local fake HTTP server, never the real API.
- External keys never reach the client; new DTO fields carry no secrets.
- Constants (spec §11), exact values: `NEW_WINDOW_SECS = 30 * 86_400`, `BULK_THRESHOLD = 300`, `BULK_WINDOW_SECS = 1_800`, `FOR_YOU_K = 5`, `FOR_YOU_LAMBDA = 0.5`, `MAX_POSITIVES = 300`, `MAX_NEGATIVES = 100`, `MIN_BASIS = 3`, `SEED_BACKOFF_SECS = 3 * 86_400`, `CATALOGS_MAX_AGE_SECS = 7 * 86_400`, `MOTN_MONTHLY_LIMIT = 500`.
- Exact UI copy: sort labels `Relevance`, `Trending`, `For you`, `Top rated`, `Newest`, `A–Z`; hint `Rate 3+ titles you liked to unlock For you`; error `For you unavailable — retry`; Settings suffix `requests this month (approx.)`.
- Gates before every commit: `cargo fmt --check`, clippy (above), `cargo test`; frontend `npx vitest run` and `npx vue-tsc -b` from `frontend/`. `cargo fmt` currently reorders `src/services/mod.rs` (pre-existing) — don't commit that hunk unless you touch the file.

## Review Focus

1. **Ratings on titles outside the catalogue or without an embedding** (most of a big IMDb import) must be ignored, not counted in `basis`, and must not error — Task 3 test `ratings_outside_catalogue_are_ignored`.
2. **Refining or stepping inside an answer must not override a sort the user picked during that answer** — only entering answer mode selects Relevance — Task 7 test `refine keeps an explicitly chosen sort`.
3. **A 429 in the middle of a full seed is a failed seed** and must start the 3-day back-off — Task 4 test `rate_limited_seed_records_backoff`.
4. **Embeddings of a different length** (a partially backfilled model change) must be skipped, not panic in the dot product — Task 2 test `vector_set_skips_mismatched_lengths`.
5. **Every candidate watched or rated** (basis ≥ 3, empty `ids`) must leave For you enabled and render the sunk list, not an empty grid — Task 7 test `for you with empty ids renders everything sunk`.

---

### Task 1: `newSince` on the catalogue list

**Files:**
- Modify: `src/models.rs` (`TitleListRow`, `TitleListItem`)
- Modify: `src/db/catalogue.rs` (`fetch_catalogue`, new pure fn + tests)

**Interfaces:**
- Produces: `TitleListItem.new_since: Option<i64>` serialised as `"newSince"` (unix secs or `null`); `pub fn new_since_map(added: &[(i64, i64)], now: i64) -> HashMap<i64, i64>` (input `(title_id, added_at_unix)`, output only the ids that are new).

- [ ] **Step 1: Write the failing tests** (sync `#[test]`s in a new `new_since_tests` module in `src/db/catalogue.rs` — the existing `tests` module uses `#[actix_web::test]`):

```rust
const NOW: i64 = 1_760_000_000;
#[test] fn lone_recent_addition_is_new() {
    let m = new_since_map(&[(1, NOW - 86_400)], NOW);
    assert_eq!(m.get(&1), Some(&(NOW - 86_400)));
}
#[test] fn older_than_30_days_is_not_new() {
    assert!(new_since_map(&[(1, NOW - 30 * 86_400 - 1)], NOW).is_empty());
}
#[test] fn burst_of_300_within_30_minutes_is_bulk() {
    let rows: Vec<(i64, i64)> = (0..300).map(|i| (i, NOW - 3_600 + i)).collect();
    assert!(new_since_map(&rows, NOW).is_empty());
}
#[test] fn burst_of_299_is_new() {
    let rows: Vec<(i64, i64)> = (0..299).map(|i| (i, NOW - 3_600 + i)).collect();
    assert_eq!(new_since_map(&rows, NOW).len(), 299);
}
#[test] fn window_is_plus_minus_1800_seconds_inclusive() {
    // 299 at t, plus one at t+1800 (inside) -> 300 within ±1800 of t -> bulk;
    // one at t+1801 would be outside.
    let t = NOW - 7_200;
    let mut rows: Vec<(i64, i64)> = (0..299).map(|i| (i, t)).collect();
    rows.push((999, t + 1_800));
    assert!(!new_since_map(&rows, NOW).contains_key(&0));
}
```

- [ ] **Step 2: Run** `cargo test --lib new_since` — expect a compile failure (`new_since_map` not found).

- [ ] **Step 3: Implement** `new_since_map` with the constants `NEW_WINDOW_SECS`, `BULK_THRESHOLD`, `BULK_WINDOW_SECS`: sort by timestamp, two-pointer count of rows within `[t − 1800, t + 1800]` (counting itself); keep ids with `now − t ≤ NEW_WINDOW_SECS` and count `< 300`. Add `added_at: i64` to `TitleListRow` and select it as `CAST(strftime('%s', added_at) AS INTEGER) AS added_at`; in `fetch_catalogue` build the map once with `now` from `SystemTime::now()` and set `new_since` per item. Add `#[serde(rename = "newSince")] pub new_since: Option<i64>` to `TitleListItem` and update its literal in `models.rs` tests.

- [ ] **Step 4: Run** `cargo test --lib catalogue` — PASS, then the full gates.

- [ ] **Step 5: Commit** `feat(catalogue): newSince with stateless bulk-insert rule`

---

### Task 2: For you scoring core (pure)

**Files:**
- Modify: `src/services/similarity.rs` (`dot` becomes chunked)
- Create: `src/services/for_you.rs` (pure types + scoring; declare `pub mod for_you;` in `src/services/mod.rs`)

**Interfaces:**
- Produces:
  - `pub struct VectorSet { pub ids: Vec<i64>, pub index: HashMap<i64, usize>, pub vecs: Vec<Vec<f32>> }` with `pub fn build(raw: Vec<(i64, Vec<f32>)>) -> VectorSet` — keeps only vectors whose length equals the most common length, subtracts the mean vector, re-normalises (zero-norm vectors dropped).
  - `pub struct Rated { pub title_id: i64, pub rating: i64, pub rated_at: String }`
  - `pub struct ForYouResult { pub ids: Vec<i64>, pub basis: usize }` (`Serialize`, fields `ids`, `basis`)
  - `pub fn rank(set: &VectorSet, ratings: &[Rated], excluded: &HashSet<i64>) -> ForYouResult`
  - `pub(crate) fn candidate_score(c: &[f32], positives: &[(&[f32], f32)], negatives: &[(&[f32], f32)]) -> f32` — `(vector, weight)` pairs, normalised vectors; used by `rank` and tested directly.
  - Constants `FOR_YOU_K`, `FOR_YOU_LAMBDA`, `MAX_POSITIVES`, `MAX_NEGATIVES`, `MIN_BASIS` (values in Global Constraints).

- [ ] **Step 1: Write the failing tests** (`#[cfg(test)] mod tests` in `for_you.rs`; plus one in `similarity.rs`):

```rust
// similarity.rs
#[test] fn chunked_dot_matches_naive() {
    let a: Vec<f32> = (0..1_539).map(|i| ((i * 7 % 13) as f32) - 6.0).collect();
    let b: Vec<f32> = (0..1_539).map(|i| ((i * 5 % 11) as f32) - 5.0).collect();
    let naive: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
    assert!((dot(&a, &b) - naive).abs() < 1e-2 * naive.abs().max(1.0));
}

// for_you.rs
#[test] fn vector_set_skips_mismatched_lengths() {
    let s = VectorSet::build(vec![(1, vec![1.0, 0.0]), (2, vec![0.0, 1.0]), (3, vec![1.0, 0.0, 0.0])]);
    assert!(!s.index.contains_key(&3));
}
#[test] fn centring_separates_vectors_that_share_a_common_component() {
    // Raw cosines all ≈ 0.99; after centring 1 and 2 point opposite ways.
    let s = VectorSet::build(vec![(1, vec![10.0, 1.0]), (2, vec![10.0, -1.0]), (3, vec![10.0, 0.0])]);
    let (a, b) = (&s.vecs[s.index[&1]], &s.vecs[s.index[&2]]);
    assert!(crate::services::similarity::cosine(a, b) < -0.9);
}
#[test] fn basis_below_three_returns_no_ids() {
    // two ratings ≥7 → basis 2 → empty
}
#[test] fn neighbours_are_selected_by_cosine_not_weight() {
    // candidate_score with 6 positives: five at weight 0.25 with cosine 0.9 to c,
    // one at weight 1.0 with cosine 0.3. k = 5 → the five close ones are picked
    // (weighted-cos selection would have picked the far one: 1.0*0.3 > 0.25*0.9),
    // so pos == 0.9 and, with no negatives, the score == 0.9 (± 1e-5).
}
#[test] fn disliked_neighbour_penalises() {
    // Two candidates equally close to the likes; one is also near a rating-1
    // title → it ranks second.
}
#[test] fn excluded_titles_never_appear() { /* watched/rated ids absent from result */ }
#[test] fn ties_break_by_id() { /* identical vectors → ascending id */ }
#[test] fn caps_positives_at_300_by_weight_then_recency() {
    // 301 positives: the dropped one is the lowest-weight, oldest rated_at.
}
```

The bodies marked with comments are the implementer's fixtures; each must assert exactly the stated ordering or membership.

- [ ] **Step 2: Run** `cargo test --lib for_you similarity` — compile failure / FAIL.

- [ ] **Step 3: Implement.** `dot`: iterate `chunks_exact(8)` on both slices with 8 independent `f32` accumulators, sum them, then add the remainder; keep the "shorter length wins" contract. `rank` follows spec §5.3 exactly: weights `(r−6)/4` for 7–10 and `(5−r)/4` for 1–4, ratings for titles not in `set.index` ignored, `basis = positives.len()`, return empty `ids` if `basis < MIN_BASIS`; caps applied after sorting by weight desc then `rated_at` desc; for each candidate (in `set`, not in `excluded`): the `k = min(FOR_YOU_K, |P|)` positives with the highest plain cosine (vectors are normalised, so cosine = `dot`), `pos = Σw·cos / Σw` over those, `neg = max(w_n · cos)` (0 if none), `score = pos − FOR_YOU_LAMBDA · neg`; sort by score desc then id asc.

- [ ] **Step 4: Run** the tests — PASS; full gates.

- [ ] **Step 5: Commit** `feat(for-you): centred vector set and top-k scoring`

---

### Task 3: For you service and `GET /api/for-you`

**Files:**
- Modify: `src/services/for_you.rs` (add the service)
- Create: `src/routes/for_you.rs` (declare in `src/routes/mod.rs`, route `.route("/for-you", web::get().to(for_you::get_for_you))` inside the `/api` scope)
- Modify: `src/main.rs` (create `Arc<ForYouService>` and register `web::Data`)

**Interfaces:**
- Consumes: Task 2's `VectorSet`, `Rated`, `rank`, `ForYouResult`; `db::embeddings::load_all(pool, EMBED_MODEL) -> Vec<(i64, Vec<f32>)>`.
- Produces: `pub struct ForYouService` with `pub fn new() -> Self` and `pub async fn get(&self, pool: &SqlitePool) -> anyhow::Result<ForYouResult>`; HTTP `GET /api/for-you` → `200 {"ids":[…],"basis":n}`, `500` on error.

- [ ] **Step 1: Write the failing tests** (route tests with `#[actix_web::test]` in `src/routes/for_you.rs`; seed titles + `title_embeddings` rows via `db::embeddings::upsert`, ratings via `user_data::set_rating`, watched via `user_data::set_watched`):

```rust
#[actix_web::test] async fn for_you_ranks_unwatched_unrated_titles() {
    // 3 liked titles + 2 candidates → 200, basis == 3, ids == [closer, farther],
    // and no rated/watched id in ids.
}
#[actix_web::test] async fn ratings_outside_catalogue_are_ignored() {
    // 5 ratings ≥7 on imdb ids with no title + 2 on embedded titles → basis == 2, ids == [].
}
#[actix_web::test] async fn no_embeddings_returns_basis_zero() {
    // titles + ratings but no title_embeddings → {"ids":[],"basis":0}
}
#[tokio::test] async fn result_cache_hits_until_a_rating_changes() {
    // svc.get twice → second call served from cache (expose a test-only
    // `computations()` counter == 1); set_rating → next get recomputes (== 2).
}
#[tokio::test] async fn vector_cache_rebuilds_after_new_embeddings() {
    // add an embedding → `vector_builds()` counter increments on next get.
}
```

- [ ] **Step 2: Run** `cargo test --lib for_you` — FAIL.

- [ ] **Step 3: Implement `ForYouService`.** State behind `tokio::sync::Mutex`: an `Arc<VectorSet>` with its key, and the last `ForYouResult` with its key. Keys come from runtime queries:
  - vector key = (`COUNT(*)`, `COALESCE(MAX(title_id),0)` of `title_embeddings WHERE model = EMBED_MODEL`; `COUNT(*) FROM titles`; `COALESCE(MAX(id),0) FROM sync_runs`);
  - result key = vector key + (`COUNT(*)`, `COALESCE(SUM(rating),0)`, `COALESCE(MAX(rated_at),'')` of `user_ratings`) + `COUNT(*) FROM watch_history`.

  On a vector-key miss, load embeddings and run `VectorSet::build` inside `tokio::task::spawn_blocking`. Ratings: `SELECT t.id, r.rating, r.rated_at FROM user_ratings r JOIN titles t ON t.imdb_id = r.imdb_id`. Excluded = rated title ids ∪ ids of titles whose `imdb_id` is in `watch_history`. Run `rank` in `spawn_blocking`. Test-only `computations()` / `vector_builds()` counters (`AtomicUsize`, `#[cfg(test)]` accessors). Handler `get_for_you(pool, svc: web::Data<Arc<ForYouService>>)` logs and 500s on error.

- [ ] **Step 4: Run** tests — PASS; full gates.

- [ ] **Step 5: Commit** `feat(for-you): cached service and GET /api/for-you`

---

### Task 4: MOTN request accounting

**Files:**
- Create: `src/db/motn_meta.rs` (declare in `src/db/mod.rs`)
- Modify: `src/sync/motn.rs` (`MotnClient`)

**Interfaces:**
- Produces (`db::motn_meta`):
  - key consts `CATALOGS = "motn.catalogs"`, `CATALOGS_CHECKED_AT = "motn.catalogs_checked_at"`, `LAST_SEED_AT = "motn.last_seed_at"`, `LAST_MODE = "motn.last_mode"`, `SEED_FAILED_AT = "motn.seed_failed_at"`; request keys are `"motn.requests." || strftime('%Y-%m','now')`.
  - `pub async fn increment_requests(pool) -> anyhow::Result<()>` (single upsert: insert `'1'` or `CAST(value AS INTEGER) + 1`)
  - `pub async fn requests_this_month(pool) -> anyhow::Result<i64>`
  - `pub async fn get_i64(pool, key) -> anyhow::Result<Option<i64>>`, `pub async fn set_i64(pool, key, v: i64) -> anyhow::Result<()>`
- Produces (`motn.rs`): `pub struct RateLimited;` (`Display` "MOTN rate limit (429)", implements `std::error::Error`) returned inside `anyhow::Error`; `#[cfg(test)] pub fn with_base(api_key, country, pool, base: String) -> Self` (reqwest client built with `.no_proxy()`).

- [ ] **Step 1: Write the failing tests.** Add a test helper in `motn.rs` tests: `async fn fake_motn(handler: fn(&str) -> (u16, String)) -> (String, Arc<AtomicUsize>)` that binds `std::net::TcpListener` on `127.0.0.1:0`, serves every path through an `actix_web::HttpServer` default service calling `handler(path)` and counting hits, spawns it, and returns `(base_url, hits)`. Tests (`#[actix_web::test]`):

```rust
async fn every_request_is_counted()                 // seed against a 2-page fake → requests_this_month == hits
async fn rate_limited_seed_records_backoff()        // page 1 ok, page 2 → 429: fetch errs, err.downcast_ref::<RateLimited>().is_some(), SEED_FAILED_AT set
async fn seed_backoff_errors_without_network()      // SEED_FAILED_AT = now - 3600, empty cache: fetch errs containing "seed back-off", hits == 0
async fn seed_backoff_expires_after_three_days()    // SEED_FAILED_AT = now - 3*86400 - 1 → seed runs (hits > 0)
async fn catalogs_reused_within_seven_days()        // CATALOGS="disney,crunchyroll", checked now-60: delta run never hits /countries
async fn catalogs_re_resolved_after_seven_days()    // checked now - 7*86400 - 1 → /countries hit once, CATALOGS_CHECKED_AT updated
async fn cached_catalogs_used_when_countries_fails()// stale cache + /countries → 500: run proceeds with cached catalogs
async fn modes_recorded()                           // after seed: LAST_MODE "seed", LAST_SEED_AT set, SEED_FAILED_AT == 0; after delta: LAST_MODE "delta"
```

Plus `motn_meta` unit tests: `increment_requests` three times → `requests_this_month == 3`; `get_i64` on a missing key → `None`.

- [ ] **Step 2: Run** `cargo test --lib motn` — FAIL.

- [ ] **Step 3: Implement.** Add `base: String` to `MotnClient` (`new` uses `MOTN_BASE`); route every request through `async fn get_text(&self, path: &str, query: &[(&str, &str)]) -> anyhow::Result<String>` which calls `increment_requests` **before** sending, maps HTTP 429 to `RateLimited`, otherwise `error_for_status` + `text()`. Reorder `fetch`: `decide_mode` → if `Seed` and `SEED_FAILED_AT` is within `SEED_BACKOFF_SECS`, `bail!("MOTN seed back-off until {unix}")` with no request → resolve catalogs (`force = true` on a seed; else reuse `CATALOGS` if `CATALOGS_CHECKED_AT` is within `CATALOGS_MAX_AGE_SECS`, mapping ids with `service_for_id`; on `/countries` failure fall back to a cached value with `tracing::warn!`) → run. A seed error sets `SEED_FAILED_AT = now` and returns the error; a successful seed sets `LAST_SEED_AT = now`, `LAST_MODE = "seed"`, `SEED_FAILED_AT = 0`; a successful delta sets `LAST_MODE = "delta"`. A 429 log line includes `requests_this_month`.

- [ ] **Step 4: Run** tests — PASS; full gates.

- [ ] **Step 5: Commit** `feat(motn): request counter, 429 abort, seed back-off, catalogs reuse`

---

### Task 5: `motn` block on sync status

**Files:**
- Modify: `src/db/motn_meta.rs` (status builder)
- Modify: `src/routes/sync.rs` (`StatusBody`)

**Interfaces:**
- Consumes: Task 4's keys, `requests_this_month`, `get_i64`; `motn_cache::count`.
- Produces: `#[derive(Serialize)] #[serde(rename_all = "camelCase")] pub struct MotnStatus { cache_size: i64, last_mode: Option<String>, last_seed_at: Option<i64>, seed_failed_at: Option<i64>, catalogs_checked_at: Option<i64>, requests_this_month: i64, monthly_limit: i64 }` and `pub async fn status(pool) -> anyhow::Result<Option<MotnStatus>>` — `None` when the cache is empty **and** `LAST_MODE` is unset (MOTN never ran); `seed_failed_at` is `None` when stored as `0`; `monthly_limit = MOTN_MONTHLY_LIMIT`. `StatusBody` gains `motn: Option<MotnStatus>` (JSON `"motn"`).

- [ ] **Step 1: Write the failing tests** in `src/routes/sync.rs` tests:

```rust
#[actix_web::test] async fn status_motn_is_null_when_never_run() { /* body["motn"].is_null() */ }
#[actix_web::test] async fn status_reports_motn_state() {
    // set LAST_MODE "delta", LAST_SEED_AT 1757635200, two increment_requests, one cache row →
    // body["motn"]["lastMode"] == "delta", ["lastSeedAt"] == 1757635200,
    // ["requestsThisMonth"] == 2, ["monthlyLimit"] == 500, ["cacheSize"] == 1, ["seedFailedAt"].is_null()
}
```

- [ ] **Step 2: Run** `cargo test --lib routes::sync` — FAIL.
- [ ] **Step 3: Implement** `motn_meta::status` and wire it into `status()` (500 + log on error, like the other parts).
- [ ] **Step 4: Run** — PASS; full gates.
- [ ] **Step 5: Commit** `feat(sync): MOTN cache and request state on /api/sync/status`

---

### Task 6: Frontend types and API clients

**Files:**
- Modify: `frontend/src/types.ts`, `frontend/src/api/client.ts` (`isTitle`)
- Create: `frontend/src/api/forYou.ts`
- Test: `frontend/src/api/__tests__/` (add `forYou.test.ts`; extend the existing client test for `newSince`)

**Interfaces:**
- Produces: `TitleListItem.newSince: number | null`; `interface MotnStatus { cacheSize: number; lastMode: string | null; lastSeedAt: number | null; seedFailedAt: number | null; catalogsCheckedAt: number | null; requestsThisMonth: number; monthlyLimit: number }`; `SyncStatus.motn: MotnStatus | null`; `interface ForYouResult { ids: number[]; basis: number }`; `export async function getForYou(): Promise<ForYouResult>` (throws `Error` on non-2xx or a malformed body).

- [ ] **Step 1: Write the failing tests:** `isTitle` rejects `newSince: "x"`, accepts `null` and a number; `getForYou` resolves `{ids:[3,1],basis:4}` from a mocked `fetch`, rejects on HTTP 500, rejects `{ids:"x"}`.
- [ ] **Step 2: Run** `npx vitest run src/api` — FAIL.
- [ ] **Step 3: Implement** the types, the `isTitle` clause `(r.newSince === null || typeof r.newSince === 'number')`, and `getForYou` mirroring `getCatalogue`'s error style. Update existing `Title` test fixtures that are now missing `newSince` (add `newSince: null`).
- [ ] **Step 4: Run** `npx vitest run` and `npx vue-tsc -b` — PASS.
- [ ] **Step 5: Commit** `feat(frontend): newSince, MotnStatus and getForYou types`

---

### Task 7: Store — Relevance, Trending, For you

**Files:**
- Modify: `frontend/src/stores/catalogue.ts`
- Test: `frontend/src/stores/__tests__/sortOrders.test.ts` (new)

**Interfaces:**
- Consumes: Task 6's `getForYou`, `ForYouResult`.
- Produces: `export type SortKey = 'relevance' | 'trending' | 'foryou' | 'rating' | 'year' | 'az'` (exported for FilterBar); state `browseSort: SortKey`, `forYou: { status: 'idle' | 'loading' | 'ready' | 'error'; ids: number[]; basis: number }`; actions `loadForYou(): Promise<void>`; getter `forYouAvailable: boolean` (`forYou.basis >= 3`).

- [ ] **Step 1: Write the failing tests** (`sortOrders.test.ts`; mock `@/api/forYou` and `@/services` with `vi.mock`):

```ts
it('trending: new titles first by newSince desc, then external rating desc, nulls last, ties by id')
it('an active answer keeps engine order under Relevance', /* applyResult ids [3,1,2] → visibleTitles ids [3,1,2] */)
it('entering an answer selects relevance and clearThread restores the browse sort')
it('refine keeps an explicitly chosen sort', /* answer → setSort('rating') → refine → sort stays 'rating' */)
it('for you: ids order first, then the rest in trending order')
it('for you sinks locally watched or rated titles even if in ids', /* toggleWatched optimistic */)
it('for you with empty ids renders everything sunk', /* basis 4, ids [] → all titles in trending order, forYouAvailable true */)
it('for you renders trending order until the first load is ready')
it('a reload keeps the previous ids until the new response arrives')
it('a superseded for-you response is ignored', /* two loads, first resolves last → ids from second */)
it('loadForYou runs after load(), setRating, clearRating and toggleWatched succeed')
it('forYouAvailable is false below basis 3')
```

- [ ] **Step 2: Run** `npx vitest run src/stores` — FAIL.

- [ ] **Step 3: Implement.**
  - Default `sort: 'trending'`, `browseSort: 'trending'`, `forYou: { status: 'idle', ids: [], basis: 0 }`.
  - A private helper `enterAnswer()`: if `!answerActive`, save `browseSort = sort` and set `sort = 'relevance'`; then set `answerActive = true`. Call it from `applyResult` and the three error branches (in place of `this.answerActive = true`). `clearThread` restores `sort = browseSort`. `stepThread` leaves `sort` alone.
  - `visibleTitles` sorting: `'relevance'` keeps base order; `'trending'` uses a comparator `(a, b)`: non-null `newSince` first, `newSince` desc, then `externalRating` desc (nulls last), then `id` asc; `'foryou'` builds a rank map from `forYou.ids` (skipped until the first `ready`), puts mapped titles that are not locally watched (`this.watched[id]`) or rated (`this.ratings[id]`) first by rank, then everything else with the trending comparator.
  - `loadForYou`: increment a module-level request sequence; set `status = 'loading'` only if not yet `ready` (keep old `ids` during reloads); on success apply only if the sequence matches; on failure set `status = 'error'` but keep the previous `ids`. Call it at the end of a successful `load()` and after successful `setRating` / `clearRating` / `toggleWatched` (not awaited by those actions).

- [ ] **Step 4: Run** `npx vitest run` and `npx vue-tsc -b` — PASS (existing store tests must stay green; update any that assumed `SortKey` had four values).

- [ ] **Step 5: Commit** `feat(frontend): relevance, trending and for-you ordering in the store`

---

### Task 8: Sort menu in `FilterBar`

**Files:**
- Modify: `frontend/src/components/FilterBar.vue`
- Test: `frontend/src/components/__tests__/FilterBar.test.ts`

**Interfaces:**
- Consumes: Task 7's `SortKey`, `answerActive`, `forYou`, `forYouAvailable`, `loadForYou`.

- [ ] **Step 1: Write the failing tests:**

```ts
it('browse mode lists Trending, For you, Top rated, Newest, A–Z (no Relevance)')
it('answer mode adds Relevance as the first option')
it('For you is disabled with the unlock hint below basis 3',
   /* option[value=foryou].disabled; text 'Rate 3+ titles you liked to unlock For you' visible */)
it('shows "For you unavailable — retry" when for-you errored with For you selected, and retry calls loadForYou')
```

- [ ] **Step 2: Run** `npx vitest run src/components/__tests__/FilterBar.test.ts` — FAIL.
- [ ] **Step 3: Implement:** `sortOptions` becomes a computed list from the exact labels; `selectedSort` setter typed `SortKey`; `:disabled` on the For you option when `!store.forYouAvailable`; the hint (`data-test="foryou-hint"`) shows while it is unavailable; the error line (`data-test="foryou-error"`, a button that calls `store.loadForYou()`) shows when `store.sort === 'foryou' && store.forYou.status === 'error'`. Match the existing muted caption styling with design tokens.
- [ ] **Step 4: Run** frontend gates — PASS.
- [ ] **Step 5: Commit** `feat(frontend): relevance and for-you in the sort menu`

---

### Task 9: Settings MOTN line + docs

**Files:**
- Modify: `frontend/src/views/SettingsView.vue`
- Test: `frontend/src/views/__tests__/SettingsView.test.ts` (create if absent; else extend the existing Settings test)
- Modify: `docs/superpowers/deferred-followups.md`

**Interfaces:**
- Consumes: Task 6's `SyncStatus.motn`.

- [ ] **Step 1: Write the failing tests:**

```ts
it('renders the MOTN line', /* motn {cacheSize:4812,lastMode:'delta',lastSeedAt:<12 Sep 2026 UTC>,
   requestsThisMonth:37,monthlyLimit:500,...} → text contains 'Cache 4,812 shows', 'last full seed 12 Sep',
   'delta', '37 / 500 requests this month (approx.)' */)
it('shows seed paused until … during back-off', /* seedFailedAt = now-3600 → 'seed paused until' + date 3 days later */)
it('hides the MOTN line when motn is null')
```

- [ ] **Step 2: Run** the test — FAIL.
- [ ] **Step 3: Implement** inside the Catalogue card (`data-test="motn-line"`): numbers via `toLocaleString('en-GB')`, dates via `toLocaleDateString('en-GB', { day: 'numeric', month: 'short', timeZone: 'UTC' })` on `unix * 1000`; omit "last full seed" when `lastSeedAt` is null; "seed paused until <date>" when `seedFailedAt` is set and `seedFailedAt + 3 days` is in the future.
- [ ] **Step 4: Run** frontend gates — PASS.
- [ ] **Step 5: Docs.** In `deferred-followups.md`, add a "Sort orders (2026-10-03)" section listing the homelab items: (a) measure the real cosine spread (raw and centred) on the live DB and tune `FOR_YOU_K` / `FOR_YOU_LAMBDA` / caps; (b) time a For you recompute after a rating (fallback: per-title top-50 neighbour lists); (c) after ~2 weeks, read `requestsThisMonth` to clear the Phase 2 popularity gate; (d) run the spec §10 `expiring` curl to clear the Leaving soon gate.
- [ ] **Step 6: Commit** `feat(frontend): MOTN state in Settings; docs: phase 2 gates`

---

## Final verification

- [ ] Backend: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` — all green.
- [ ] Frontend (in `frontend/`): `npx vitest run && npx vue-tsc -b && npx vite build` — all green.
- [ ] Push the branch.
