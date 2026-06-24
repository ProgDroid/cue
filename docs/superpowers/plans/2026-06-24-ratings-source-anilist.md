# Rating provenance + AniList anime scores — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the mislabeled `imdb_rating` aggregate to `score` end-to-end, and enrich anime titles with real AniList scores via an exact offline id-map.

**Architecture:** A single migration renames the column and adds two AniList columns. A mechanical rename flows `score` through the Rust structs, DTOs, and the Vue type/fixtures. A new `src/sync/anilist.rs` builds an in-memory `imdb/tmdb → anilist` map from Fribb's offline JSON, batch-fetches `averageScore` from AniList by id, and persists it — invoked by `SyncRunner` after the core sync, wipe-guarded so any failure retains cached scores. The frontend shows the AniList pill on the grid (falling back to the generic Rating pill) and both on the detail view.

**Tech Stack:** Rust + Actix-web + SQLx (runtime queries) + SQLite; Vue 3 + TS + Pinia + Vitest. HTTP via `reqwest` (already a dep). No new crates.

## Global Constraints

- **SQLx runtime queries only** — `sqlx::query`/`query_as`/`query_scalar`; never the `query!` macros.
- **After adding migration `0008`, run `cargo clean -p cue` before `cargo test`** — `sqlx::migrate!` does not re-embed a new `.sql` on an incremental build, so the migration silently won't apply.
- **Test DBs:** `tempfile::tempdir()` + `format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"))`; keep the `TempDir` guard bound (`_dir`).
- **Clippy gate:** `cargo clippy --all-targets -- -D warnings`; per-item `#[allow(...)]` with a reason only, never widen the global table.
- **Commit through the Bash tool, not PowerShell** (PowerShell prepends a UTF-8 BOM to the commit subject).
- **Secrets:** AniList score fetch + Fribb download are unauthenticated; add no keys. `data/` is gitignored.
- **Aggregate field is named `score`** (not `rating` — `rating` is the user's personal 1–10 score). AniList field is `anilist_score` (Rust/SQL) / `anilistScore` (JSON/TS).
- **AniList score normalized to 0–10** on ingest (`averageScore / 10.0`), matching `score`.

---

### Task 1: Backend rename `imdb_rating → score` + AniList columns + DTO plumbing

**Files:**
- Create: `migrations/0008_score_rename_and_anilist.sql`
- Modify: `src/models.rs` (`TitleRow`, `TitleListRow`, `TitleDto`, `TitleListItem` + test)
- Modify: `src/db/catalogue.rs:14,65,80,139` (SELECTs + DTO mapping)
- Modify: `src/services/ask_engine.rs:247,295` (query strings)
- Modify: `src/sync/store.rs:113,123,136,145,274,482`
- Modify: `src/sync/merge.rs:50,120,140,314`
- Modify: `src/sync/mod.rs:48,267,317` (`FetchedTitle` + test constructors)
- Modify: `src/sync/motn.rs:202,558,714`
- Modify: `src/sync/plex.rs:106`
- Modify: `src/db/seed.rs:37`
- Modify: `src/db/motn_cache.rs:21,41,73,199` (`CachedTitle`, back-compat alias)

**Interfaces:**
- Produces: `FetchedTitle.score: Option<f64>`, `MergedTitle.score: Option<f64>`,
  `TitleRow { score: Option<f64>, anilist_score: Option<f64> }`,
  `TitleListRow { score: Option<f64>, anilist_score: Option<f64> }`,
  `TitleDto`/`TitleListItem` serialize `score` and `anilistScore`. DB column `titles.score`
  (renamed) plus `titles.anilist_id INTEGER`, `titles.anilist_score REAL`.

> This whole rename is one task because a partial rename does not compile (the field is referenced across 9 files).

- [ ] **Step 1: Write the migration**

Create `migrations/0008_score_rename_and_anilist.sql` (newline-terminated):

```sql
ALTER TABLE titles RENAME COLUMN imdb_rating TO score;
ALTER TABLE titles ADD COLUMN anilist_id    INTEGER;
ALTER TABLE titles ADD COLUMN anilist_score REAL;
```

- [ ] **Step 2: Rename the model structs + update the DTO test**

In `src/models.rs`:
- `TitleRow` (line 67): replace `pub imdb_rating: Option<f64>,` with:
```rust
    pub score: Option<f64>,
    pub anilist_score: Option<f64>,
```
- `TitleListRow` (line 81): same replacement (`score` + `anilist_score`).
- `TitleDto` (line 96): replace `pub imdb: Option<f64>,` with:
```rust
    pub score: Option<f64>,
    #[serde(rename = "anilistScore")]
    pub anilist_score: Option<f64>,
```
- `TitleListItem` (line 116): same replacement as `TitleDto`.
- In the `dto_serializes_with_frontend_field_names` test: change the literal `imdb: Some(8.0),` to:
```rust
            score: Some(8.0),
            anilist_score: None,
```
  and change `assert_eq!(v["imdb"], 8.0);` to:
```rust
        assert_eq!(v["score"], 8.0);
        assert_eq!(v["anilistScore"], serde_json::Value::Null);
```

- [ ] **Step 3: Update the catalogue queries + DTO mapping**

In `src/db/catalogue.rs`:
- Line 14 SELECT → `"SELECT id, imdb_id, title, year, type, score, anilist_score, length\n         FROM titles ORDER BY id"`.
- Line 65 `imdb: r.imdb_rating,` → `score: r.score,` and add `anilist_score: r.anilist_score,`.
- Line 80 SELECT → `"SELECT id, imdb_id, title, year, type, score, anilist_score, length, description\n         FROM titles WHERE id = ?"`.
- Line 139 `imdb: r.imdb_rating,` → `score: r.score,` and add `anilist_score: r.anilist_score,`.

- [ ] **Step 4: Update the ask-engine query strings**

In `src/services/ask_engine.rs`:
- Line 247: `"SELECT id, imdb_rating FROM titles"` → `"SELECT id, score FROM titles"`.
- Line 295: `"SELECT id, title, year, type, imdb_rating FROM titles"` → `"SELECT id, title, year, type, score FROM titles"`.

(The tuple types are positional — no other change.)

- [ ] **Step 5: Update the sync write/merge/source structs**

- `src/sync/mod.rs`: line 48 `pub imdb_rating: Option<f64>,` → `pub score: Option<f64>,`; lines 267 & 317 test constructors `imdb_rating: None,` → `score: None,`.
- `src/sync/merge.rs`: line 50 field → `pub score: Option<f64>,`; line 120 → `existing.score = existing.score.or(f.score);`; line 140 → `score: f.score,`; line 314 ft helper → `score: None,`.
- `src/sync/store.rs`: line 113 SQL `imdb_rating = ?` → `score = ?`; line 123 `.bind(t.imdb_rating)` → `.bind(t.score)`; line 136 INSERT column `imdb_rating` → `score`; line 145 `.bind(t.imdb_rating)` → `.bind(t.score)`; line 274 `imdb_rating: Some(7.5),` → `score: Some(7.5),`; line 482 `imdb_rating: None,` → `score: None,`.
- `src/sync/motn.rs`: line 202 `imdb_rating: s.rating.map(|r| r / 10.0),` → `score: s.rating.map(|r| r / 10.0),`; line 558 `(t.imdb_rating.unwrap() - 8.2)` → `(t.score.unwrap() - 8.2)`; line 714 `imdb_rating: None,` → `score: None,`.
- `src/sync/plex.rs`: line 106 `imdb_rating: m.rating,` → `score: m.rating,`.
- `src/db/seed.rs`: line 37 INSERT column list `imdb_rating` → `score` (the SeedTitle JSON key `imdb` is unchanged).

- [ ] **Step 6: Update the MOTN cache mirror (back-compat preserved)**

In `src/db/motn_cache.rs`:
- Line 21: replace `pub imdb_rating: Option<f64>,` with (alias keeps old cached payloads parseable):
```rust
    #[serde(alias = "imdb_rating")]
    pub score: Option<f64>,
```
- Line 41 (`From<&FetchedTitle>`): `imdb_rating: t.imdb_rating,` → `score: t.score,`.
- Line 73 (`into_fetched`): `imdb_rating: self.imdb_rating,` → `score: self.score,`.
- Line 199 (test `sample()`): `imdb_rating: Some(7.5),` → `score: Some(7.5),`.
- Leave the line-284 old-payload test JSON exactly as-is (its `"imdb_rating":null` now exercises the `alias`, proving production cache rows still load).

- [ ] **Step 7: Force a full rebuild so the new migration embeds, then test**

Run:
```bash
cargo clean -p cue && cargo test --lib
```
Expected: PASS. (If `score` errors as "no such column", the clean didn't take — re-run.)

- [ ] **Step 8: Clippy + commit**

Run: `cargo clippy --all-targets -- -D warnings` (expect clean).
```bash
git add migrations/0008_score_rename_and_anilist.sql src/models.rs src/db/catalogue.rs \
  src/services/ask_engine.rs src/sync/store.rs src/sync/merge.rs src/sync/mod.rs \
  src/sync/motn.rs src/sync/plex.rs src/db/seed.rs src/db/motn_cache.rs
git commit -m "refactor(rating): rename imdb_rating -> score; add anilist columns"
```

---

### Task 2: Frontend mechanical rename `imdb → score` (+ `anilistScore`)

**Files:**
- Modify: `frontend/src/types.ts:12`
- Modify: `frontend/src/services/askService.ts:48-49`
- Modify: `frontend/src/components/PosterCard.vue:65,67,70`
- Modify: `frontend/src/views/DetailView.vue:159,161`
- Modify (fixtures): `frontend/src/api/__tests__/client.test.ts`, `frontend/src/stores/__tests__/ask.test.ts`, `frontend/src/stores/__tests__/similar.test.ts`, `frontend/src/views/__tests__/BrowseView.test.ts`, `frontend/src/services/__tests__/stub.test.ts`, `frontend/src/stores/__tests__/catalogue.test.ts`, `frontend/src/views/__tests__/DetailView.test.ts`, `frontend/src/views/__tests__/DetailWatched.test.ts`, `frontend/src/services/__tests__/api.test.ts`, `frontend/src/components/__tests__/FilterBar.test.ts`, `frontend/src/components/__tests__/PosterCard.test.ts`, `frontend/src/components/__tests__/PosterGrid.test.ts`

**Interfaces:**
- Produces: `TitleListItem.score: number | null` and `TitleListItem.anilistScore: number | null` in `types.ts`. (`Title = TitleListItem`, `TitleDetail extends TitleListItem`.)

> Pills keep their current IMDb markup in this task — only the field name changes, so everything still compiles and renders. The visual redesign is Task 3.

- [ ] **Step 1: Update the type**

In `frontend/src/types.ts`, `TitleListItem` (line 12): replace `  imdb: number | null` with:
```ts
  score: number | null
  anilistScore: number | null
```

- [ ] **Step 2: Run the type-check / tests to see the failures**

Run: `cd frontend && npx vue-tsc --noEmit` (or `npm run test:unit`).
Expected: FAIL — many `Property 'imdb' does not exist` / fixtures missing `anilistScore`.

- [ ] **Step 3: Update the two component field reads**

- `PosterCard.vue`: line 65 `v-if="title.imdb !== null"` → `v-if="title.score !== null"`; line 67 `${title.imdb}` → `${title.score}`; line 70 `{{ title.imdb }}` → `{{ title.score }}`.
- `DetailView.vue`: line 159 `v-if="detail.imdb !== null"` → `v-if="detail.score !== null"`; line 161 `{{ detail.imdb }}` → `{{ detail.score }}`.

- [ ] **Step 4: Update the similar-ranking sort**

In `frontend/src/services/askService.ts`:
- Line 48: `.map(t => ({ id: t.id, shared: t.genres.filter(x => g.has(x)).length, imdb: t.imdb ?? -Infinity }))` → replace `imdb: t.imdb` with `score: t.score`.
- Line 49: `.sort((a, b) => b.shared - a.shared || b.imdb - a.imdb)` → `b.score - a.score`.

- [ ] **Step 5: Update every fixture**

In each fixtures file listed above, rename the property `imdb:` → `score:` and add `anilistScore: null` alongside it. Two concrete examples:
- `frontend/src/components/__tests__/PosterCard.test.ts:8` `type: 'series', genres: ['Animation'], imdb: 9.0, len: '28 eps',` → `type: 'series', genres: ['Animation'], score: 9.0, anilistScore: null, len: '28 eps',`
- `frontend/src/views/__tests__/BrowseView.test.ts:18` `... genres: [], imdb: null, year: 2000, ...` → `... genres: [], score: null, anilistScore: null, year: 2000, ...`

Apply the same transform to the `imdb:` occurrences in: `client.test.ts:7`, `ask.test.ts:20`, `similar.test.ts:8,16-19`, `stub.test.ts:7,11-13`, `catalogue.test.ts:9,15-17,50`, `DetailView.test.ts:12`, `DetailWatched.test.ts:19`, `api.test.ts:7`, `FilterBar.test.ts:20`, `PosterGrid.test.ts:10`. (Where a `t(...)` factory supplies defaults, set `score: null, anilistScore: null` in the factory's base object so per-case overrides still work.)

- [ ] **Step 6: Run type-check + tests**

Run: `cd frontend && npx vue-tsc --noEmit && npm run test:unit`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/types.ts frontend/src/services/askService.ts \
  frontend/src/components/PosterCard.vue frontend/src/views/DetailView.vue \
  frontend/src/**/__tests__/*.ts
git commit -m "refactor(web): rename title.imdb -> title.score; add anilistScore"
```

---

### Task 3: Frontend pill redesign — AniList pill + generic Rating pill

**Files:**
- Modify: `frontend/src/components/PosterCard.vue` (badge block + styles)
- Modify: `frontend/src/views/DetailView.vue` (badge row + styles)
- Modify: `frontend/src/components/__tests__/PosterCard.test.ts`
- Modify: `frontend/src/views/__tests__/DetailView.test.ts`

**Interfaces:**
- Consumes: `Title.score`, `Title.anilistScore` (Task 2).
- Grid pill rule: `anilistScore != null` → AniList pill; else `score != null` → generic Rating pill.
- Detail: render the AniList pill and the generic Rating pill independently, each when its value is present.

- [ ] **Step 1: Write the failing PosterCard test**

Add to `frontend/src/components/__tests__/PosterCard.test.ts`:
```ts
it('shows the AniList pill when anilistScore is present, hiding the generic pill', () => {
  const w = mount(PosterCard, { props: { title: { ...base, score: 7.4, anilistScore: 8.6 }, watched: false } })
  expect(w.find('[data-test="anilist-badge"]').exists()).toBe(true)
  expect(w.find('[data-test="score-badge"]').exists()).toBe(false)
  expect(w.text()).toContain('8.6')
})

it('falls back to the generic Rating pill when anilistScore is null', () => {
  const w = mount(PosterCard, { props: { title: { ...base, score: 7.4, anilistScore: null }, watched: false } })
  expect(w.find('[data-test="anilist-badge"]').exists()).toBe(false)
  expect(w.find('[data-test="score-badge"]').exists()).toBe(true)
  expect(w.text()).toContain('7.4')
})
```
(Define `base` from the file's existing fixture if not already in scope — reuse the top-of-file fixture object.)

- [ ] **Step 2: Run it — expect FAIL** (`data-test="anilist-badge"` not found).

Run: `cd frontend && npm run test:unit -- PosterCard`

- [ ] **Step 3: Implement the PosterCard pills**

Replace the badge block in `PosterCard.vue` (lines 63–71) with:
```html
      <!-- Rating badge (top-right): AniList preferred, else generic score -->
      <div
        v-if="title.anilistScore !== null"
        data-test="anilist-badge"
        class="poster-badge anilist"
        :aria-label="`AniList score ${title.anilistScore}`"
      >
        <span class="badge-mark anilist-mark" aria-hidden="true">AL</span>
        <span class="badge-score">{{ title.anilistScore }}</span>
      </div>
      <div
        v-else-if="title.score !== null"
        data-test="score-badge"
        class="poster-badge"
        :aria-label="`Rating ${title.score}`"
      >
        <span class="badge-mark" aria-hidden="true">★</span>
        <span class="badge-score">{{ title.score }}</span>
      </div>
```
Rename the `.poster-imdb-badge`/`.imdb-star`/`.imdb-score` style rules to `.poster-badge`/`.badge-mark`/`.badge-score` (same properties), and add the AniList accent:
```css
.anilist { background: rgba(2, 169, 255, 0.16); }
.anilist-mark {
  color: #02a9ff;
  font-size: 9px;
  font-weight: 700;
  letter-spacing: 0.02em;
}
```
(Keep the generic `.badge-mark` star using `var(--accent, #f5c518)` — a neutral star, no "IMDb" text.)

- [ ] **Step 4: Run PosterCard tests — expect PASS.** Run: `cd frontend && npm run test:unit -- PosterCard`

- [ ] **Step 5: Write the failing DetailView test**

Add to `frontend/src/views/__tests__/DetailView.test.ts` a case mounting a detail with `score: 7.4, anilistScore: 8.6` and assert both pills:
```ts
expect(wrapper.find('[data-test="detail-anilist"]').exists()).toBe(true)
expect(wrapper.find('[data-test="detail-score"]').exists()).toBe(true)
expect(wrapper.text()).toContain('8.6')
expect(wrapper.text()).toContain('7.4')
```
(Follow the file's existing mount/stub/`getTitle` mock pattern; set the mocked detail's `score`/`anilistScore`.)

- [ ] **Step 6: Run it — expect FAIL.** Run: `cd frontend && npm run test:unit -- DetailView`

- [ ] **Step 7: Implement the DetailView pills**

Replace the single `imdb-pill` span (`DetailView.vue` lines 159–163) with both pills:
```html
          <span v-if="detail.anilistScore !== null" data-test="detail-anilist" class="rating-pill anilist-pill">
            <span class="pill-mark">AL</span>
            <span class="pill-score">{{ detail.anilistScore }}</span>
            <span class="pill-label">AniList</span>
          </span>
          <span v-if="detail.score !== null" data-test="detail-score" class="rating-pill">
            <span class="pill-mark">★</span>
            <span class="pill-score">{{ detail.score }}</span>
            <span class="pill-label">Rating</span>
          </span>
```
Rename the `.imdb-pill`/`.imdb-star`/`.imdb-score`/`.imdb-label` rules to `.rating-pill`/`.pill-mark`/`.pill-score`/`.pill-label` (same properties), and add:
```css
.anilist-pill { background: rgba(2, 169, 255, 0.1); border-color: rgba(2, 169, 255, 0.25); }
.anilist-pill .pill-mark { color: #02a9ff; font-weight: 700; font-size: 10px; }
.anilist-pill .pill-label { color: #2b7fb0; }
```

- [ ] **Step 8: Run DetailView tests — expect PASS, then commit**

Run: `cd frontend && npm run test:unit`
```bash
git add frontend/src/components/PosterCard.vue frontend/src/views/DetailView.vue \
  frontend/src/components/__tests__/PosterCard.test.ts frontend/src/views/__tests__/DetailView.test.ts
git commit -m "feat(web): AniList pill on grid; AniList + Rating pills on detail"
```

---

### Task 4: Backend — Fribb offline id-map (`AnimeIdMap`)

**Files:**
- Create: `src/sync/anilist.rs`
- Modify: `src/sync/mod.rs` (add `pub mod anilist;` near the other `pub mod` lines, ~line 14–17)
- Create (test fixture): `src/sync/testdata/fribb_sample.json`

**Interfaces:**
- Produces: `AnimeIdMap` with `AnimeIdMap::parse(&str) -> anyhow::Result<AnimeIdMap>` and
  `AnimeIdMap::resolve(&self, imdb_id: Option<&str>, tmdb_id: Option<&str>) -> Option<i64>`.

> **Fribb shape is load-bearing — pin it with the real file.** Per the README, an entry's `imdb_id` is an array of strings (or absent) and `themoviedb_id` is `{ "tv": <int>, "movie": [<int>] }` (or absent); `anilist_id` is an int. Download a current copy and paste 2–3 real entries (one TV with imdb, one movie with `themoviedb_id.movie`) into `fribb_sample.json` so the test reflects reality. cue's `tmdb_id` may be prefixed (`"movie/9"`); `resolve` strips any `/`-prefix before matching.

- [ ] **Step 1: Write the failing parse/resolve test**

Create `src/sync/anilist.rs` with:
```rust
//! AniList enrichment: offline Fribb id-map (imdb/tmdb -> anilist) + batched
//! score fetch. Additive and wipe-guarded — any failure retains cached scores.

use std::collections::HashMap;

/// Maps a title's external ids to its AniList id, built from Fribb's
/// `anime-list-full.json`. An id present here means the title IS anime.
#[derive(Debug, Default)]
pub struct AnimeIdMap {
    by_imdb: HashMap<String, i64>,
    by_tmdb: HashMap<String, i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("testdata/fribb_sample.json");

    #[test]
    fn resolves_anime_by_imdb_and_tmdb() {
        let map = AnimeIdMap::parse(SAMPLE).unwrap();
        // Replace the ids below with the real ones you pasted into the fixture.
        assert_eq!(map.resolve(Some("tt9335498"), None), Some(101_759));
        assert_eq!(map.resolve(None, Some("movie/372058")), Some(101_759).map(|_| /* the movie entry's anilist id */ 0));
        assert_eq!(map.resolve(Some("tt0000000"), None), None);
    }
}
```
> Adjust the asserted ids to match the real entries you paste into `fribb_sample.json` (the `map(|_| 0)` line is a placeholder you will replace with the movie entry's actual AniList id once the fixture is real).

- [ ] **Step 2: Run it — expect FAIL** (`AnimeIdMap::parse` not found).

Run: `cargo test --lib anilist::tests::resolves_anime_by_imdb_and_tmdb`

- [ ] **Step 3: Implement `parse` + `resolve`**

Add to `src/sync/anilist.rs`:
```rust
impl AnimeIdMap {
    /// Build the map from Fribb's `anime-list-full.json`. Defensive about the
    /// documented-but-irregular shapes (`imdb_id`: string|array; `themoviedb_id`:
    /// int|{tv,movie[]}) — anything unexpected is skipped, not fatal.
    ///
    /// # Errors
    /// Returns an error only if the top-level JSON is not an array.
    pub fn parse(json: &str) -> anyhow::Result<Self> {
        let entries: Vec<serde_json::Value> = serde_json::from_str(json)?;
        let mut by_imdb = HashMap::new();
        let mut by_tmdb = HashMap::new();
        for e in entries {
            let Some(anilist) = e.get("anilist_id").and_then(serde_json::Value::as_i64) else {
                continue;
            };
            match e.get("imdb_id") {
                Some(serde_json::Value::String(s)) => {
                    by_imdb.insert(s.clone(), anilist);
                }
                Some(serde_json::Value::Array(a)) => {
                    for v in a {
                        if let Some(s) = v.as_str() {
                            by_imdb.insert(s.to_string(), anilist);
                        }
                    }
                }
                _ => {}
            }
            match e.get("themoviedb_id") {
                Some(serde_json::Value::Number(n)) => {
                    if let Some(i) = n.as_i64() {
                        by_tmdb.insert(i.to_string(), anilist);
                    }
                }
                Some(serde_json::Value::Object(o)) => {
                    if let Some(i) = o.get("tv").and_then(serde_json::Value::as_i64) {
                        by_tmdb.insert(i.to_string(), anilist);
                    }
                    if let Some(serde_json::Value::Array(a)) = o.get("movie") {
                        for v in a {
                            if let Some(i) = v.as_i64() {
                                by_tmdb.insert(i.to_string(), anilist);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(Self { by_imdb, by_tmdb })
    }

    /// Resolve a title's AniList id from its external ids (imdb wins).
    /// cue's tmdb id may be prefixed (`"movie/9"`); the bare number is matched.
    #[must_use]
    pub fn resolve(&self, imdb_id: Option<&str>, tmdb_id: Option<&str>) -> Option<i64> {
        if let Some(i) = imdb_id {
            if let Some(a) = self.by_imdb.get(i) {
                return Some(*a);
            }
        }
        if let Some(t) = tmdb_id {
            let key = t.rsplit('/').next().unwrap_or(t);
            if let Some(a) = self.by_tmdb.get(key) {
                return Some(*a);
            }
        }
        None
    }
}
```
Add `pub mod anilist;` to `src/sync/mod.rs` alongside the existing `pub mod merge; pub mod motn; ...`.

- [ ] **Step 4: Run the test — expect PASS** (after the fixture + asserted ids are real).

Run: `cargo test --lib anilist::`

- [ ] **Step 5: Clippy + commit**

Run: `cargo clippy --all-targets -- -D warnings`
```bash
git add src/sync/anilist.rs src/sync/mod.rs src/sync/testdata/fribb_sample.json
git commit -m "feat(sync): Fribb offline imdb/tmdb -> anilist id map"
```

---

### Task 5: Backend — AniList GraphQL score fetch

**Files:**
- Modify: `src/sync/anilist.rs` (add `parse_scores` + `fetch_scores`)

**Interfaces:**
- Produces: `parse_scores(body: &str) -> anyhow::Result<Vec<(i64, Option<f64>)>>` (anilist id → score/10),
  and `async fetch_scores(client: &reqwest::Client, ids: &[i64]) -> anyhow::Result<Vec<(i64, Option<f64>)>>`.

- [ ] **Step 1: Write the failing `parse_scores` test**

Add to the `tests` module in `src/sync/anilist.rs`:
```rust
#[test]
fn parses_anilist_page_scores_normalized_to_ten() {
    let body = r#"{"data":{"Page":{"media":[
        {"id":101759,"averageScore":86},
        {"id":21,"averageScore":null}
    ]}}}"#;
    let got = parse_scores(body).unwrap();
    assert_eq!(got.len(), 2);
    assert!((got[0].1.unwrap() - 8.6).abs() < 1e-9);
    assert_eq!(got[0].0, 101_759);
    assert_eq!(got[1].1, None);
}
```

- [ ] **Step 2: Run it — expect FAIL** (`parse_scores` not found).

Run: `cargo test --lib anilist::tests::parses_anilist_page_scores_normalized_to_ten`

- [ ] **Step 3: Implement `parse_scores` + `fetch_scores`**

Add to `src/sync/anilist.rs`:
```rust
use serde::Deserialize;

const ANILIST_URL: &str = "https://graphql.anilist.co";
const SCORES_QUERY: &str =
    "query($ids:[Int]){Page(perPage:50){media(id_in:$ids,type:ANIME){id averageScore}}}";

#[derive(Deserialize)]
struct GqlResp {
    data: Option<GqlData>,
}
#[derive(Deserialize)]
struct GqlData {
    #[serde(rename = "Page")]
    page: GqlPage,
}
#[derive(Deserialize)]
struct GqlPage {
    media: Vec<GqlMedia>,
}
#[derive(Deserialize)]
struct GqlMedia {
    id: i64,
    #[serde(rename = "averageScore")]
    average_score: Option<i64>,
}

/// Parse an AniList `Page` response into `(anilist_id, score/10)` pairs.
///
/// # Errors
/// Returns an error if the body is not the expected JSON shape.
pub fn parse_scores(body: &str) -> anyhow::Result<Vec<(i64, Option<f64>)>> {
    let r: GqlResp = serde_json::from_str(body)?;
    let media = r.data.map(|d| d.page.media).unwrap_or_default();
    Ok(media
        .into_iter()
        .map(|m| {
            #[allow(clippy::cast_precision_loss)] // scores are 0..=100, lossless in f64
            let s = m.average_score.map(|v| v as f64 / 10.0);
            (m.id, s)
        })
        .collect())
}

/// Fetch scores for up to 50 AniList ids in one request.
///
/// # Errors
/// Returns an error if the request fails or the body cannot be parsed.
pub async fn fetch_scores(
    client: &reqwest::Client,
    ids: &[i64],
) -> anyhow::Result<Vec<(i64, Option<f64>)>> {
    let body = serde_json::json!({ "query": SCORES_QUERY, "variables": { "ids": ids } });
    let text = client
        .post(ANILIST_URL)
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_scores(&text)
}
```

- [ ] **Step 4: Run the test — expect PASS.** Run: `cargo test --lib anilist::`

- [ ] **Step 5: Clippy + commit**

Run: `cargo clippy --all-targets -- -D warnings`
```bash
git add src/sync/anilist.rs
git commit -m "feat(sync): batched AniList averageScore fetch (normalized to 0-10)"
```

---

### Task 6: Backend — enrichment orchestration, persistence, and wiring

**Files:**
- Modify: `src/sync/anilist.rs` (add `persist_scores` + `enrich`)
- Modify: `src/config.rs` (add `data_dir()` helper)
- Modify: `src/sync/mod.rs` (`SyncRunner` gains a data dir; `try_start` calls `enrich`)
- Modify: `src/main.rs:89` (pass the data dir)
- Modify call sites: `src/sync/mod.rs:553`, `src/routes/sync.rs:105,126` (pass `None`)

**Interfaces:**
- Consumes: `AnimeIdMap` (Task 4), `fetch_scores` (Task 5).
- Produces: `async persist_scores(pool, &[(i64, i64, Option<f64>)]) -> anyhow::Result<u64>`
  (rows = `(title_id, anilist_id, score)`), and `async enrich(pool: &SqlitePool, data_dir: &Path) -> anyhow::Result<u64>`.
- `Config::data_dir(&self) -> std::path::PathBuf`.
- `SyncRunner::new(pool, sources, embedder, anilist_dir: Option<std::path::PathBuf>)`.

- [ ] **Step 1: Write the failing `persist_scores` test**

Add to the `tests` module in `src/sync/anilist.rs`:
```rust
async fn pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("t.db");
    let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
    (crate::db::init_pool(&url).await.unwrap(), dir)
}

#[tokio::test]
async fn persist_writes_anilist_id_and_score() {
    let (p, _dir) = pool().await;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO titles (imdb_id, title, year, type) VALUES ('tt1','A',2020,'movie') RETURNING id",
    )
    .fetch_one(&p)
    .await
    .unwrap();
    let n = persist_scores(&p, &[(id, 101_759, Some(8.6))]).await.unwrap();
    assert_eq!(n, 1);
    let row: (Option<i64>, Option<f64>) =
        sqlx::query_as("SELECT anilist_id, anilist_score FROM titles WHERE id = ?")
            .bind(id)
            .fetch_one(&p)
            .await
            .unwrap();
    assert_eq!(row.0, Some(101_759));
    assert!((row.1.unwrap() - 8.6).abs() < 1e-9);
}
```

- [ ] **Step 2: Run it — expect FAIL** (`persist_scores` not found).

Run: `cargo test --lib anilist::tests::persist_writes_anilist_id_and_score`

- [ ] **Step 3: Implement `persist_scores` + `enrich`**

Add to `src/sync/anilist.rs` (add `use std::path::Path;` and `use sqlx::SqlitePool;` to the imports):
```rust
/// Write resolved AniList id + score for each `(title_id, anilist_id, score)`.
/// Returns the number of title rows updated.
///
/// # Errors
/// Returns an error if a write fails.
pub async fn persist_scores(
    pool: &SqlitePool,
    rows: &[(i64, i64, Option<f64>)],
) -> anyhow::Result<u64> {
    let mut updated = 0;
    let mut tx = pool.begin().await?;
    for (title_id, anilist_id, score) in rows {
        let r = sqlx::query("UPDATE titles SET anilist_id = ?, anilist_score = ? WHERE id = ?")
            .bind(anilist_id)
            .bind(score)
            .bind(title_id)
            .execute(&mut *tx)
            .await?;
        updated += r.rows_affected();
    }
    tx.commit().await?;
    Ok(updated)
}

/// Fribb mapping source + local cache TTL.
const FRIBB_URL: &str =
    "https://raw.githubusercontent.com/Fribb/anime-lists/master/anime-list-full.json";
const MAP_TTL_SECS: u64 = 7 * 24 * 60 * 60;

async fn load_map(client: &reqwest::Client, path: &Path) -> anyhow::Result<AnimeIdMap> {
    let fresh = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < MAP_TTL_SECS);
    if !fresh {
        let body = client
            .get(FRIBB_URL)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        AnimeIdMap::parse(&body)?; // validate before caching
        std::fs::write(path, &body)?;
    }
    AnimeIdMap::parse(&std::fs::read_to_string(path)?)
}

/// Enrich anime titles with AniList scores. Resolves every title's external ids
/// against the Fribb map; for matches still missing a score, batch-fetches and
/// persists. Wipe-guarded: any failure logs and returns `Ok(0)` without clearing
/// existing scores. Returns the number of titles scored.
///
/// # Errors
/// Returns an error only if the initial titles query fails irrecoverably.
pub async fn enrich(pool: &SqlitePool, data_dir: &Path) -> anyhow::Result<u64> {
    if let Err(e) = std::fs::create_dir_all(data_dir) {
        tracing::error!("anilist: cannot create data dir: {e:#}; skipping");
        return Ok(0);
    }
    let client = reqwest::Client::new();
    let map = match load_map(&client, &data_dir.join("anime-list-full.json")).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("anilist: id-map load failed: {e:#}; retaining cached scores");
            return Ok(0);
        }
    };

    let titles: Vec<(i64, Option<String>, Option<String>, Option<f64>)> =
        sqlx::query_as("SELECT id, imdb_id, tmdb_id, anilist_score FROM titles")
            .fetch_all(pool)
            .await?;

    // (title_id, anilist_id) for matched anime that still need a score.
    let mut pending: Vec<(i64, i64)> = Vec::new();
    for (id, imdb, tmdb, existing) in titles {
        if existing.is_some() {
            continue; // already scored — skip (refresh is a deferred follow-up)
        }
        if let Some(anilist) = map.resolve(imdb.as_deref(), tmdb.as_deref()) {
            pending.push((id, anilist));
        }
    }
    if pending.is_empty() {
        return Ok(0);
    }

    let mut scored = 0;
    for chunk in pending.chunks(50) {
        let ids: Vec<i64> = chunk.iter().map(|(_, a)| *a).collect();
        let scores = match fetch_scores(&client, &ids).await {
            Ok(s) => s.into_iter().collect::<HashMap<i64, Option<f64>>>(),
            Err(e) => {
                tracing::error!("anilist: score fetch failed: {e:#}; retaining cached scores");
                break; // keep whatever earlier chunks persisted; never wipe
            }
        };
        let rows: Vec<(i64, i64, Option<f64>)> = chunk
            .iter()
            .map(|(title_id, anilist)| {
                (*title_id, *anilist, scores.get(anilist).copied().flatten())
            })
            .collect();
        scored += persist_scores(pool, &rows).await?;
    }
    tracing::info!("anilist: scored {scored} anime titles");
    Ok(scored)
}
```

- [ ] **Step 4: Run the persist test — expect PASS.** Run: `cargo test --lib anilist::`

- [ ] **Step 5: Add `Config::data_dir` + wire `SyncRunner`**

In `src/config.rs`, add a method on `Config` (near `sqlite_path`):
```rust
    /// Directory for runtime data files (Fribb cache, etc.). Parent of the
    /// sqlite file, or `./data` for in-memory / unparsable URLs.
    #[must_use]
    pub fn data_dir(&self) -> std::path::PathBuf {
        self.sqlite_path()
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_else(|| std::path::PathBuf::from("./data"))
    }
```
In `src/sync/mod.rs`, add a field and parameter to `SyncRunner`:
- struct: add `anilist_dir: Option<std::path::PathBuf>,`.
- `new(...)`: add a 4th param `anilist_dir: Option<std::path::PathBuf>` and set it in the struct literal.
- In `try_start`'s spawned task, after `run_sync(...)` completes, add:
```rust
            if let Some(dir) = me.anilist_dir.clone() {
                if let Err(e) = anilist::enrich(&me.pool, &dir).await {
                    tracing::error!("anilist enrich failed: {e:#}");
                }
            }
```

- [ ] **Step 6: Update the call sites**

- `src/main.rs:89`: `SyncRunner::new(pool.clone(), sources, embedder)` → `SyncRunner::new(pool.clone(), sources, embedder, Some(config.data_dir()))` (use the in-scope `Config` binding's name; it is `config`/`cfg` — match the file).
- `src/sync/mod.rs:553`: `SyncRunner::new(p, vec![src], None)` → `SyncRunner::new(p, vec![src], None, None)`.
- `src/routes/sync.rs:105` and `:126`: `SyncRunner::new(p.clone(), vec![], None)` → add a trailing `, None`.

- [ ] **Step 7: Full rebuild, test, clippy**

Run:
```bash
cargo clean -p cue && cargo test --lib && cargo clippy --all-targets -- -D warnings
```
Expected: PASS, clean.

- [ ] **Step 8: Commit**

```bash
git add src/sync/anilist.rs src/config.rs src/sync/mod.rs src/main.rs src/routes/sync.rs
git commit -m "feat(sync): wire AniList enrichment into SyncRunner (wipe-guarded)"
```

---

## Self-Review

**Spec coverage:**
- §3 "in the Fribb map = anime" → Task 4 `resolve` + Task 6 `enrich` (no genre/service pre-filter). ✓
- §4 data model (rename + 2 columns, `score` naming, `SeedTitle` keeps `imdb` JSON key) → Task 1. ✓
- §5.1 offline map + 7-day TTL + verify shape with fixture → Tasks 4 & 6 (`load_map`, `MAP_TTL_SECS`, fixture note). ✓
- §5.2 batched score fetch normalized /10 → Task 5. ✓
- §5.3 runs after merge/upsert (via `SyncRunner` after `run_sync`), cache-skip when scored → Task 6. ✓
- §5.4 wipe-guarded & non-fatal (download fail, fetch fail) → Task 6 `enrich` (returns `Ok(0)`, `break`). ✓
- §6 grid AniList-or-generic, detail both, IMDb branding removed → Tasks 2 & 3. ✓
- §7 tests (map parse, normalization, persist retains, pill selection) → Tasks 4–6 + Task 3. ✓
- §8 `data/` gitignored, no keys → reuses existing `data/`; `reqwest` unauthenticated. ✓

**Deferred (logged, not silently dropped):** periodic AniList score *refresh* — `enrich` currently scores only titles with `anilist_score IS NULL` (first-time populate). A scheduled full refresh is a follow-up (spec §9 TTL decision). Titles with neither imdb nor tmdb id cannot map. Note both in `docs/superpowers/deferred-followups.md` during execution.

**Placeholder scan:** the only intentional fill-in is `fribb_sample.json` + its asserted ids in Task 4 (must come from a real download — flagged loudly, cannot be invented). All other steps carry concrete code.

**Type consistency:** `score`/`anilist_score` (Rust), `score`/`anilistScore` (JSON/TS), `anilist_id` consistent across Tasks 1–6; `AnimeIdMap::parse`/`resolve`, `parse_scores`/`fetch_scores`, `persist_scores`/`enrich`, `Config::data_dir`, `SyncRunner::new`'s 4th param all match between producer and consumer tasks.
