# Task 4 Report — Dev seed data + seeder

## Files Changed

| File | Action |
|------|--------|
| `seed/catalogue.json` | Created — 28-title JSON array transcribed from `design_handoff_cue/cue.dc.html` lines 476–503 |
| `src/db/seed.rs` | Created — `seed_if_empty` function + inline test |
| `src/db/mod.rs` | Modified — added `pub mod seed;` at top |

## Title Count Verification

- `seed/catalogue.json` contains exactly **28** objects (ids 1–28 from prototype).
- Verified with: `python -c "import json; data=json.load(open('seed/catalogue.json')); print(len(data))"` → `28`
- First: "Frieren: Beyond Journey's End" (crunchyroll, series)
- Last: "Fleabag" (plex, series)

## Test Results

Command: `cargo test --lib db::seed`

```
running 1 test
test db::seed::tests::seeds_all_titles_once ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.06s
```

**Status: PASS**

Assertions verified:
- `seed_if_empty` returns 28 on first call
- `SELECT COUNT(*) FROM titles` = 28
- Frieren has exactly 3 genres in `title_genres`
- Second call to `seed_if_empty` returns 0 (idempotent)

## Clippy Result

Command: `cargo clippy --all-targets -- -D warnings`

```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.52s
```

**Status: CLEAN** — zero warnings or errors.

## Commit

Message: `feat(backend): dev catalogue seed and idempotent seeder`

Files committed:
- `seed/catalogue.json`
- `src/db/seed.rs`
- `src/db/mod.rs`

## Self-Review Notes

- `imdb_id` is `null` for all 28 seed rows (correct — no personal data until a later plan).
- All accented characters preserved: "Stellan Skarsgård", "Genevieve O'Reilly", "Timothée Chalamet", "María Cecilia Botero", "protégé" — written via Write tool (not PowerShell here-string) to avoid encoding corruption.
- Used `sqlx::query_scalar` / `sqlx::query` throughout — no `query!` macros.
- Service values are exactly `plex`/`disney`/`crunchyroll` as specified.
- `i64::try_from(ord).unwrap_or(i64::MAX)` for cast ordering — acceptable in this context (ord is a loop index bounded by cast array length, never approaches i64::MAX in practice; `unwrap_or` not `unwrap`).
- No concerns.
