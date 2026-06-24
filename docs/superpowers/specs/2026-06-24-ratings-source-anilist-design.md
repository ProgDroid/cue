# Rating provenance + AniList anime scores — design

**Date:** 2026-06-24
**Status:** Draft for review
**Related:** D1 (bounded catalogue), D5 (user-data), D6 (IMDb-id-first identity), catalogue-sync plan (Plan 4), MOTN incremental-sync spec (`2026-06-23-motn-incremental-sync-design.md`)

## 1. Problem & motivation

The catalogue shows a single golden **"IMDb rating"** pill on every title. That label is inaccurate:

- **MOTN** titles (Disney+/Crunchyroll/seeded) store MOTN's `rating` field, which the
  [MOTN docs](https://docs.movieofthenight.com/resource/shows) define as *"the average of ratings
  found online from multiple sources"* (0–100) — a **blended aggregate**, not an IMDb score. MOTN
  exposes **no** separate `audienceRating`/`tmdbRating`/per-source field, so there is nothing
  distinct to fetch — the aggregate is all there is.
- **Plex** titles store Plex's `rating` field, which is **metadata-agent-dependent** (frequently the
  Rotten Tomatoes critic score, sometimes TMDB) — also not guaranteed IMDb.

So **no current source provides a genuine public IMDb rating**, yet the UI asserts one. We will not
add a dedicated IMDb feed (e.g. OMDb) — decided out of scope. Instead:

1. **Tell the truth about the existing value:** rename the rating to a neutral "Rating" and drop the
   IMDb branding.
2. **Add real anime scores from AniList**, which is the rating users actually care about for anime.

## 2. Goals / non-goals

**Goals**
- Rename the conflated `imdb_rating` to `rating` end-to-end (DB column, Rust structs, DTO field, UI).
- Drop IMDb-specific branding from the generic pill.
- Enrich anime titles with AniList `averageScore` via an **exact, offline id-mapping** join.
- Grid: show AniList score (fallback to the generic rating when unmatched). Detail: show **both**.

**Non-goals**
- No dedicated IMDb-score feed (OMDb etc.).
- No TMDB audience score.
- No fuzzy title matching (Approach A only — exact id map).
- No per-source provenance enum beyond what the two columns (`rating`, `anilist_score`) already imply.

## 3. Definition of "anime"

A title **is anime iff its `imdb_id` or `tmdb_id` appears in the Fribb anime id-map.** An id match in
an anime-only dataset is high-precision, so no genre/service pre-filter is needed (an in-memory map
lookup is O(1) and free — the earlier "Crunchyroll ∪ animation-genre" candidate gate was only
relevant to the rejected live-search approach). This catches anime on **any** service, including the
local Plex library.

## 4. Data model

Migration `migrations/0008_*.sql` (forward-only, newline-terminated; remember
`cargo clean -p cue` before tests — `sqlx::migrate!` won't pick up a new `.sql` on an incremental build):

```sql
ALTER TABLE titles RENAME COLUMN imdb_rating TO rating;
ALTER TABLE titles ADD COLUMN anilist_id   INTEGER;
ALTER TABLE titles ADD COLUMN anilist_score REAL;   -- normalized 0–10, like `rating`
```

(SQLite ≥ 3.25 supports `RENAME COLUMN`; the bundled SQLx driver is current.)

**Rust rename (mechanical, all `imdb_rating` → `rating`):** `FetchedTitle`, `MergedTitle`,
`SeedTitle`, the `motn.rs`/`plex.rs`/`seed.rs` constructors, `db/catalogue.rs` queries, and the
DTOs in `models.rs`. **DTO field rename `imdb` → `rating`** (the JSON key the frontend reads), plus a
new `anilistScore` field on both list-item and detail DTOs.

`anilist_score` is stored normalized to 0–10 (`averageScore / 10.0`) so it renders identically to
`rating`.

## 5. AniList enrichment pipeline

New module `src/sync/anilist.rs`.

### 5.1 Offline id-map (`AnimeIdMap`)
- Source: Fribb `anime-list-full.json` (GitHub raw URL), cached at `data/anime-list-full.json`
  (gitignored). Re-downloaded when missing or older than a TTL (default 7 days).
- Builds two `HashMap`s: `imdb_id → anilist_id` and `tmdb_id → anilist_id`.
- **JSON shape — VERIFY WITH A REAL FIXTURE before coding the parser.** Per the Fribb README the
  entry shape is non-obvious and load-bearing:
  - `imdb_id`: **array of strings** (e.g. `["tt1164545"]`) or absent
  - `themoviedb_id`: **object** `{ "tv": <int>, "movie": [<int>, …] }` (tv single, movie array)
  - `anilist_id`: integer; `type`: `"MOVIE" | "TV" | "OVA" | …`
  The implementation MUST download a current sample, assert these types in a parse test, and flatten
  the arrays/object into the flat id→anilist maps. If the real shape differs, the parser follows the
  real shape (API-response-validation rule: do not assume).

### 5.2 Score fetch (AniList GraphQL)
- Endpoint: `POST https://graphql.anilist.co`.
- Batched by id to keep volume trivial and respect the rate limit (~90 req/min, historically lower):
  ```graphql
  query ($ids: [Int]) {
    Page(perPage: 50) { media(id_in: $ids, type: ANIME) { id averageScore } }
  }
  ```
- Normalize `averageScore` (0–100) → `/10.0`. Titles with `averageScore == null` keep `anilist_score = NULL`.

### 5.3 Orchestration
- Runs **after `merge()`**, before/within the store step, as an additive enrichment.
- Resolve each merged title's `anilist_id` from the map. For titles newly matched or whose cached
  score is stale (reuse a reseed cadence consistent with the MOTN cache), batch-fetch and persist
  `anilist_id` + `anilist_score`. Already-cached titles skip the network entirely.

### 5.4 Failure handling — wipe-guarded & non-fatal
Mirrors the load-bearing watch-history sync guard (a flaky external source must never wipe good rows):
- Fribb **download failure** → skip enrichment this run; **retain** existing `anilist_id`/`anilist_score`.
- AniList **API failure** → skip; retain cached scores.
- Enrichment never blocks or fails the core catalogue sync.

## 6. Frontend

- **`PosterCard` (grid):** one pill.
  - `anilistScore != null` → **AniList pill** (AniList blue `#02A9FF`).
  - else `rating != null` → **generic "Rating" pill** (neutral star; IMDb gold `#f5c518`/branding removed).
- **`DetailView`:** render **both** the AniList pill and the generic Rating pill when each is present.
- Update `isTitle`/type guards and any `title.imdb` reads to `title.rating`; add `anilistScore`.
- A11y: pill `aria-label`s name their source ("AniList score 8.6", "Rating 7.4").

## 7. Testing (TDD, per repo convention)

**Backend**
- `AnimeIdMap` parse: from a real downloaded fixture, assert `imdb_id`/`themoviedb_id` shapes and that
  the flattened maps resolve a known title → its `anilist_id`.
- Score normalization (`86 → 8.6`; `null → None`).
- AniList GraphQL response parse from a sample body.
- Non-fatal failure: simulated download/API error retains existing cached scores (no wipe).
- `cargo clean -p cue` then `cargo test` after adding migration 0008.

**Frontend**
- `PosterCard` renders the AniList pill when `anilistScore` present, the generic pill otherwise.
- `DetailView` renders both pills when both present; each independently when only one is.

## 8. Rollout / ops notes
- First sync after deploy downloads the Fribb file (~few MB) and back-fills `anilist_score` for
  matched anime; subsequent syncs are cache-hits.
- `data/anime-list-full.json` is gitignored (lives under the already-ignored `data/`).
- No client secret exposure (AniList score fetch + Fribb download are unauthenticated; no keys added).

## 9. Open verification items (carried into the plan)
- Confirm the live Fribb entry JSON shape against a real download (§5.1) — pin with a fixture.
- Confirm AniList rate-limit headers / current limit and set the throttle accordingly.
- Decide the exact AniList-score reseed TTL (align with the MOTN cache cadence).
