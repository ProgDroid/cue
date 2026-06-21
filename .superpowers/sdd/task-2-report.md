# Task 2 Report — SQLite Schema Migration + Pool Initialization

## Files Changed

- **Created**: `migrations/0001_init.sql` — full schema (titles, title_services, title_genres, title_cast, title_embeddings, user_ratings, watch_history, sync_runs) with indexes.
- **Created**: `src/db/mod.rs` — `pub async fn init_pool(database_url: &str) -> anyhow::Result<SqlitePool>` using `SqliteConnectOptions` with `create_if_missing(true)` + `foreign_keys(true)` + embedded migration runner. Inline `#[cfg(test)]` module with `migrations_create_expected_tables` test.
- **Modified**: `src/lib.rs` — added `pub mod db;` (now `pub mod config;` + `pub mod db;`).
- **Modified**: `Cargo.toml` — completed the `[lints.clippy]` table to include the canonical `all` group and all allow overrides per rust.md project standards (was missing `all`, allow overrides).

## Test Results

Command: `cargo test --lib db`

```
running 1 test
test db::tests::migrations_create_expected_tables ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.04s
```

**Result: PASS**

## Clippy Result

Command: `cargo clippy --all-targets -- -D warnings`

Initial run surfaced one lint: `doc_markdown` — `SQLite` in the doc comment needed backticks (`\`SQLite\``). Fixed inline. Second run:

```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.14s
```

**Result: CLEAN (zero warnings/errors)**

## Commit

Message: `feat(backend): sqlite schema migration and pool init`

Files staged: `migrations/0001_init.sql`, `src/db/mod.rs`, `src/lib.rs`, `Cargo.toml`, `.superpowers/sdd/task-2-report.md`

## Self-Review Notes / Concerns

- The `[lints.clippy]` block in `Cargo.toml` was incomplete from Task 1 (missing `all` group and allow overrides). Added the canonical block from rust.md. This is a Task 1 artifact fix, not a Task 2 scope creep concern — it was needed for `cargo clippy -- -D warnings` to behave correctly.
- Used `sqlx::query_scalar` (runtime) in the test per the Global Constraint forbidding compile-time `query!` macros.
- The `doc_markdown` clippy lint on "SQLite" was a one-line fix; no structural concern.
- No concerns about correctness: migration runs cleanly against a temp file DB on Windows with the backslash-to-forward-slash URL normalisation.
