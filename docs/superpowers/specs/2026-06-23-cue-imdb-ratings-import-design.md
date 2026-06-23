# cue — IMDb Ratings Import — Design

**Created:** 2026-06-23
**Status:** Approved (brainstorming complete; ready for implementation plan)
**Backlog origin:** `docs/superpowers/deferred-followups.md` → "IMDb ratings-export
import → `user_ratings`".

## Summary

Let the user import their IMDb ratings export (a CSV) into `user_ratings` so that
ratings made on IMDb appear in cue. The import is **lossless**: `Your Rating` is
already on IMDb's 1–10 scale and migration `0002` widened `user_ratings.rating` to
1–10, so no rescaling is needed.

`user_ratings.imdb_id` is a free-standing `TEXT PRIMARY KEY` with **no foreign key**
to `titles`. Ratings can therefore exist for IMDb IDs not (yet) in the catalogue;
the frontend simply doesn't surface them until a matching title syncs in. This makes
an "import everything" strategy viable and lossless.

## Decisions (settled during brainstorming)

| # | Decision | Choice |
|---|----------|--------|
| I1 | Scope — ratings for IMDb IDs not in the catalogue | **Import everything.** All valid rows are written regardless of catalogue membership; unmatched ratings sit dormant until a matching title syncs in, then surface automatically. |
| I2 | Conflict policy — IMDb ID already rated in `user_ratings` | **Import overwrites (upsert).** CSV value wins via `ON CONFLICT … DO UPDATE`. Makes re-import idempotent (last import is the truth). |
| I3 | `rated_at` — preserve IMDb `Date Rated`? | **Preserve it.** Parse `Date Rated` (`YYYY-MM-DD`) into `rated_at`; fall back to DB default `now()` when missing/unparseable. |
| I4 | Upload/parse mechanism | **Raw CSV text body + `csv` crate.** Frontend reads the file via `FileReader` and POSTs the text; backend parses with header-named deserialization. No `actix-multipart`. |

### Why raw-text body over multipart

IMDb exports are tiny (a few thousand rows, well under 1 MB), so streaming and
multipart machinery buy nothing. The `csv` crate gives robust RFC-4180 parsing
(handles commas/quotes inside title fields such as
`Lock, Stock and Two Smoking Barrels`) and header-named field mapping, so column
reordering or extra columns don't break the import. Hand-rolled comma-splitting was
rejected as fragile.

## IMDb export format (reference)

The IMDb "Export" produces a CSV whose header includes (order not relied upon):

```
Const, Your Rating, Date Rated, Title, Original Title, URL, Title Type,
IMDb Rating, Runtime (mins), Year, Genres, Num Votes, Release Date, Directors
```

Only three columns are consumed:

- **`Const`** — the IMDb tconst, e.g. `tt0111161` → `user_ratings.imdb_id`.
- **`Your Rating`** — integer 1–10 → `user_ratings.rating`.
- **`Date Rated`** — `YYYY-MM-DD` → `user_ratings.rated_at` (optional).

## Architecture

### Backend — parsing (`src/import/imdb_ratings.rs`, new module)

Add the `csv` crate to `[dependencies]`.

```rust
#[derive(serde::Deserialize)]
struct ImdbRow {
    #[serde(rename = "Const")]       const_id: String,
    #[serde(rename = "Your Rating")] your_rating: Option<String>,
    #[serde(rename = "Date Rated")]  date_rated: Option<String>,
}

pub struct RatingImport { pub imdb_id: String, pub rating: i64, pub rated_at: Option<String> }
pub struct ParsedImport { pub rows: Vec<RatingImport>, pub skipped: usize }

pub fn parse_ratings(csv: &str) -> ParsedImport; // pure, no DB
```

Per-row rules:

- `Const` blank/whitespace → **skip** (does not count as `skipped` — malformed, not a
  rateable row). *(Open nuance: blank-Const rows are not expected in a real export;
  counting them as skipped vs ignoring them is immaterial. Treat as skip-and-ignore.)*
- `Your Rating` missing, non-integer, or outside 1–10 → **skip**, increment `skipped`.
- `Date Rated` present and parseable as `YYYY-MM-DD` → keep as `Some(date)`;
  otherwise `None`.

The function is pure (string in, struct out) and unit-testable without a DB.

### Backend — persistence (`src/db/user_data.rs`)

```rust
pub struct ImportOutcome { pub imported: usize, pub matched: usize }

pub async fn import_ratings(pool: &SqlitePool, rows: &[RatingImport])
    -> anyhow::Result<ImportOutcome>;
```

- Runs in a **single transaction**. For each row, upsert:

  ```sql
  INSERT INTO user_ratings (imdb_id, rating, rated_at)
  VALUES (?, ?, COALESCE(?, datetime('now')))
  ON CONFLICT(imdb_id) DO UPDATE SET
      rating   = excluded.rating,
      rated_at = excluded.rated_at;
  ```

  Binding `rated_at` as `Option<String>`; `COALESCE` applies the DB default when the
  import date is absent.
- `imported` = number of rows written.
- After commit, one query computes `matched` — how many imported IDs exist in
  `titles`: `SELECT COUNT(*) FROM user_ratings WHERE imdb_id IN (SELECT imdb_id FROM
  titles WHERE imdb_id IS NOT NULL)` scoped to the imported set, or equivalently a
  join. (Implementation detail; the figure is informational only.)

### Backend — endpoint (`src/routes/import.rs`, new; wired in `src/routes/mod.rs`)

- `POST /api/import/ratings`
- Body: raw CSV via the `String` extractor (`Content-Type: text/csv`).
- Flow: `parse_ratings` → if `rows` is empty → `400 { "error": "no_ratings_found" }`;
  else `import_ratings` → `200 { "imported": N, "skipped": M, "matched": K }`.
- DB error → `500` (logged via `tracing::error!`, consistent with existing handlers).

Route registration sits alongside the other `/api` routes in `routes/mod.rs`.

### Frontend

- `frontend/src/api/userData.ts`: add

  ```ts
  export async function importRatings(csv: string):
    Promise<{ imported: number; skipped: number; matched: number }>
  ```

  POSTs the raw text with `Content-Type: text/csv`; reuses the existing `errMsg`
  pattern for failures.
- `frontend/src/views/SettingsView.vue`: a new card **"Import IMDb ratings"**:
  - `<input type="file" accept=".csv">`; on change, read the file via `FileReader`
    (`readAsText`), call `importRatings`.
  - Show a result line, e.g. `Imported 482 ratings · 310 in your library · 3 rows
    skipped`, or an error message (reusing the `.settings__error` style).
  - On success, trigger a **catalogue re-fetch** (via the catalogue store's load
    action) so imported ratings surface without a manual page reload.
  - Disable the input / show a busy state while the request is in flight.

## Data flow

```
IMDb CSV file (browser)
  → FileReader.readAsText → POST /api/import/ratings (text/csv)
  → parse_ratings (skip invalid/unrated rows)
  → import_ratings (single txn, upsert, preserve Date Rated)
  → 200 { imported, skipped, matched }
  → SettingsView shows summary + re-fetches catalogue
  → GET /api/catalogue (existing left-join on user_ratings) surfaces new ratings
```

## Error handling

| Condition | Result |
|-----------|--------|
| Body parses to zero data rows (empty/garbage/header-only) | `400 { "error": "no_ratings_found" }` |
| Row with blank `Const` | silently skipped |
| Row with missing/non-integer/out-of-range `Your Rating` | skipped, counted in `skipped` |
| Row with unparseable `Date Rated` | imported with `rated_at` = `now()` |
| DB/transaction failure | `500`, logged |
| Frontend fetch non-OK | error line in the card (`errMsg`) |

## Testing

**Rust — `parse_ratings` unit tests** (no DB): a representative CSV containing a
comma-in-title row, a blank/unrated row, an out-of-range value (e.g. `0`, `11`), and a
malformed date — assert parsed-row count, `skipped` count, and `Date Rated`
passthrough (preserved when valid, `None` when malformed).

**Rust — `import_ratings` test** (tempdir pool per project convention): overwrite-on-
conflict (re-import changes the stored rating and `rated_at`), and the `matched` count
against a seeded `titles` row.

**Rust — endpoint test** (actix `test`): happy path returns 200 with the expected
`{ imported, skipped, matched }`; empty body returns `400 no_ratings_found`.

**Frontend — Vitest**: `importRatings` client (request shape + parsed response;
error on non-OK); a `SettingsView` test covering the file-read → POST → result-message
flow (FileReader/fetch mocked) and that a successful import triggers the catalogue
re-fetch.

## Out of scope (remains deferred)

- Plex watch-history import (separate backlog item).
- Any `source` column on `user_ratings` to distinguish manual vs imported ratings.
- Progress/streaming UI for very large files (not needed at IMDb export sizes).

## Conventions to honour (from `CLAUDE.md`)

- Runtime SQLx queries only (`sqlx::query` / `query_as` / `query_scalar`) — no
  compile-time macros.
- New backend modules declared in `src/lib.rs` (`pub mod import;`), consumed via the
  `cue::` path from the binary.
- Test DBs via `tempfile::tempdir()` with the `sqlite:` URL backslash fixup.
- Canonical `[lints.clippy]` table is the source of truth; per-item `#[allow]` only,
  with a one-line reason. Gate: `cargo clippy --all-targets -- -D warnings`.
- No secrets touched; this feature reads no external keys.
