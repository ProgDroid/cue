# cue — Plan 5: User-Data Writes (design)

**Date:** 2026-06-23
**Status:** Approved (brainstorming) — ready for implementation plan
**Plan position:** 5 of 5 (final committed plan; Plans 1–4 merged to `main`)

## 1. Goal & scope

Add the **write path** for personal ratings and watched state, persist them, and
wire the frontend's currently local-only `toggleWatched` / `setRating` to the API.
Also widen the rating scale from 1–5 to **1–10** so future IMDb/Plex ratings
imports are lossless.

The **read path already exists** and is unchanged: `fetch_catalogue`
(`src/db/catalogue.rs`) joins `user_ratings` + `watch_history` into
`TitleDto.rating` / `.watched`, and the Pinia store hydrates `watched`/`ratings`
from the catalogue payload on `load()`. Plan 5 only adds writes + the migration.

Per master design decision **D6**, user data is keyed on **`imdb_id`**, not the
numeric `titles.id`.

### Out of scope (stays in `deferred-followups.md`)
IMDb ratings-export import, Plex watch-history/ratings import, poster artwork,
the "not on your services" row, auth. This plan only makes the schema
import-ready (1–10); the importers themselves remain deferred.

## 2. Decisions (locked during brainstorming, 2026-06-23)

| # | Decision | Choice |
|---|----------|--------|
| D5.1 | Title whose `imdb_id` is NULL (possible for Plex items with no imdb GUID) | **Reject the write with 422**; frontend **disables** the rating/watched controls for that title. A fallback synthetic key was rejected — it would orphan the row once a later sync fills in the real `imdb_id`. |
| D5.2 | Un-watch semantics on the append-only `watch_history` (`source` ∈ `manual`/`plex`) | Delete **only `source='manual'`** rows for that `imdb_id`. Leaves any future imported Plex history intact. (Today only manual rows exist, so identical behaviour now — forward-compatible later.) |
| D5.3 | Rating scale + control | **1–10**, rendered as **10 stars/pips** (`StarRating` `STARS` extended to `1..10`). Matches IMDb (1–10) and Plex's internal 0–10 scale → lossless imports. |
| D5.4 | Clearing a rating | **Supported.** `DELETE /api/titles/{id}/rating`; clicking the **current** rating value again clears it (title returns to unrated). |

## 3. Data model — migration `0002_widen_rating_to_10.sql`

SQLite cannot `ALTER` a `CHECK` constraint, so the table is rebuilt:

```sql
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

- Applied automatically on startup by the existing `sqlx::migrate!("./migrations")`
  call in `init_pool` (`src/db/mod.rs`).
- The server's live `user_ratings` is **empty** (no writes shipped before Plan 5),
  so the copy carries no mis-scaled real data.
- If the 28-row **dev seed** (`src/db/seed.rs`) inserts demo ratings on the old
  1–5 scale, those values are updated to the 1–10 scale as part of this work.
- `watch_history` is **unchanged**.

## 4. Backend — `src/db/user_data.rs`

Pure SQLx runtime queries (no compile-time macros, per project convention). No
trait seam — these touch only the local DB, so tests use the standard
`tempfile::tempdir()` pattern. Functions:

- `resolve_key(pool, title_id) -> KeyLookup`
  `SELECT imdb_id FROM titles WHERE id = ?` via `fetch_optional` of
  `(Option<String>,)`:
  - row absent → **`NotFound`** (→ HTTP 404)
  - `Some(None)` → **`NoImdbId`** (→ HTTP 422)
  - `Some(Some(key))` → **`Key(String)`** → proceed.
- `set_rating(pool, key, rating)` →
  `INSERT INTO user_ratings (imdb_id, rating) VALUES (?, ?)
   ON CONFLICT(imdb_id) DO UPDATE SET rating = excluded.rating,
   rated_at = datetime('now')`.
- `clear_rating(pool, key)` → `DELETE FROM user_ratings WHERE imdb_id = ?`.
- `set_watched(pool, key, true)` → insert one `source='manual'` row **only if
  absent** (idempotent), e.g.
  `INSERT INTO watch_history (imdb_id, source) SELECT ?, 'manual'
   WHERE NOT EXISTS (SELECT 1 FROM watch_history WHERE imdb_id = ? AND source = 'manual')`.
- `set_watched(pool, key, false)` →
  `DELETE FROM watch_history WHERE imdb_id = ? AND source = 'manual'` (D5.2).

### Backend tests
- `set_rating` upserts (insert then update same key keeps one row, new value).
- `clear_rating` removes the row; clearing a missing row is a no-op.
- invalid rating rejected at the route layer (see §5).
- `set_watched(true)` twice → still exactly one manual row (idempotent).
- `set_watched(false)` deletes manual rows but **leaves a seeded `source='plex'`
  row intact** and the title still reads `watched = true`.
- `resolve_key`: unknown id → `NotFound`; NULL-imdb title → `NoImdbId`.
- round-trip: rate/watch then `fetch_catalogue` reflects `rating`/`watched`.

## 5. Backend — `src/routes/user_data.rs`

Handlers take `web::Data<SqlitePool>` + `web::Path<i64>` (+ `web::Json<…>` where a
body applies), matching the existing handler style (e.g. `get_catalogue`). Verbs
are **explicit desired-state** (idempotent; no toggle/double-fire races). Wired
into `routes/mod.rs::configure`.

| Method + path | Body | Success | Errors |
|---|---|---|---|
| `PUT /api/titles/{id}/rating` | `{ "rating": 1..10 }` | `200 { "rating": n }` | `400` rating out of 1–10 / malformed; `404` no such title; `422 {"error":"no_imdb_id"}` |
| `DELETE /api/titles/{id}/rating` | — | `200 { "rating": null }` | `404`; `422` |
| `PUT /api/titles/{id}/watched` | `{ "watched": bool }` | `200 { "watched": b }` | `404`; `422` |

Each response returns the **canonical** field after the write so the client
reconciles to server truth rather than trusting its optimistic guess.
Validation order: parse body (→400) → `resolve_key` (→404/422) → write (→200).

### Route tests
Happy path for each endpoint + 400 (rating 0 / 11 / non-integer) + 404 (unknown
id) + 422 (NULL-imdb title), asserting status codes and JSON bodies.

## 6. Frontend

### 6.1 API client — `frontend/src/api/userData.ts`
Mirrors `frontend/src/api/sync.ts` fetch style:
- `setRating(id, n): Promise<{ rating: number | null }>` → `PUT`.
- `clearRating(id): Promise<{ rating: null }>` → `DELETE`.
- `setWatched(id, watched): Promise<{ watched: boolean }>` → `PUT`.
Non-2xx → throw with a useful message (422 surfaces "no IMDb match").

### 6.2 Store — `frontend/src/stores/catalogue.ts`
Replace the two local-only mutators with **async, optimistic-with-rollback**
actions, add `clearRating`, and add a `userDataError: string | null` state
(cleared at the start of each action):
- `async toggleWatched(id)`: compute `desired = !isWatched(id)`; optimistically
  set `watched[id]=desired`; call `setWatched`; on success reconcile from the
  response; on error revert + set `userDataError`.
- `async setRating(id, n)`: optimistic `ratings[id]=n`; call `setRating`; rollback
  on error.
- `async clearRating(id)`: optimistically delete `ratings[id]`; call
  `clearRating`; rollback on error.

### 6.3 Disable controls for un-keyable titles (D5.1)
- `Title` type (`frontend/src/types`) gains `imdbId: string | null` — already
  serialized as `imdbId` on `TitleDto`; the catalogue mapper just carries it
  through.
- A derived `canRate = !!title.imdbId` gates the controls.

### 6.4 Components
- `StarRating.vue`: `STARS` → `1..10`; add a `disabled` prop; clicking the value
  equal to the current `value` emits **`clear`** (otherwise `set`). Layout stays
  a compact pip row.
- `DetailView.vue` (and any grid-card watched control): wire the async store
  actions; when `!canRate`, disable the star row + watched button and show a
  hint ("No IMDb match — can't save rating").

### 6.5 Frontend tests (Vitest)
- store: optimistic set then rollback when the mocked API rejects; `clearRating`;
  `toggleWatched` both directions; `userDataError` set on failure.
- `StarRating`: renders 10 pips; `set(n)` on a new value; `clear` when clicking the
  current value; `disabled` blocks emits.
- `DetailView`: controls disabled when `imdbId` is null.
- Update `frontend/src/views/__tests__/DetailWatched.test.ts` (currently asserts
  the local toggle) to mock `api/userData`.

## 7. Verification gates
- Backend: `cargo test` (all green) + `cargo clippy --all-targets -- -D warnings`.
- Frontend: `npm test` (vitest run) + `npm run build` (vue-tsc).
- Migration applies cleanly on a fresh DB and on a DB created by `0001`.

## 8. Implementation order (for the plan)
1. Migration `0002` + seed rating values → 1–10.
2. `db/user_data.rs` + tests.
3. `routes/user_data.rs` + wire `configure` + route tests.
4. Frontend `Title.imdbId` carry-through.
5. `api/userData.ts`.
6. Store async actions + `userDataError`.
7. `StarRating` 1–10 + `clear` + `disabled`.
8. `DetailView` (+ card) wiring + disabled state.
9. Frontend tests + update `DetailWatched.test.ts`.
