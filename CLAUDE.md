# cue — project guide

Self-hosted personal media-discovery app. **Backend:** Rust + Actix-web + SQLx + SQLite. **Frontend:** Vue 3 + TypeScript + Pinia (Plan 2). A single Docker container serves the Vue build as static files alongside the REST API.

**Design & plans:** design spec at `docs/superpowers/specs/2026-06-21-cue-design.md` (architecture decisions D1–D9); implementation plans at `docs/superpowers/plans/`. Built as a 5-plan sequence: foundation → frontend → ask engine → catalogue sync → user-data writes.

## Backend conventions (non-obvious — follow these)

- **Crate is lib + bin.** Modules live in `src/lib.rs` as `pub mod …`; `src/main.rs` is a thin binary that consumes them via the `cue::` path. Declare new modules in `lib.rs`, not `main.rs`, so `cargo test --lib` works.
- **SQLx: runtime queries only** (`sqlx::query` / `query_as` / `query_scalar`). Do NOT use the compile-time `query!` / `query_as!` macros — the project must build with no live `DATABASE_URL` and no offline cache.
- **Migrations are embedded at COMPILE time** by `sqlx::migrate!("./migrations")`. After adding a `migrations/NNNN_*.sql` file, run `cargo clean -p cue` (or otherwise force a full rebuild) before `cargo test` — incremental builds don't re-run the macro when only a new `.sql` file is added (its token input is unchanged), so the migration silently won't apply and tests see the old schema. Migrations are forward-only; keep them sequential and newline-terminated.
- **Test databases:** use `tempfile::tempdir()` (NOT `NamedTempFile`, which holds a Windows file lock) and build the URL as `format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"))` — the backslash fixup is required for the `sqlite:` URL to parse on Windows. Keep the `TempDir` guard bound (`_dir`) for the test's lifetime.
- **`actix_web::test` shadows the built-in `#[test]` attribute.** In a `#[cfg(test)] mod` that does `use actix_web::{test, …}`, plain sync `#[test]` fns fail to compile with "the async keyword is missing from the function declaration". Put pure sync unit tests (helper-function tests) in a *separate* child module that does **not** import `actix_web::test` (see `routes/images.rs`: integration `tests` mod + a sibling `guard_tests` mod).
- **MOTN delta-sync round-trip:** any field you add to `FetchedTitle` that the MOTN source populates MUST also be carried through `CachedTitle` in `src/db/motn_cache.rs` (both `From<&FetchedTitle>` and `into_fetched`, with `#[serde(default)]` so pre-existing cache JSON still deserializes). The `/changes` delta path reconstructs the catalogue from `motn_catalog_cache`, not a fresh fetch, so a field that skips the cache is silently lost on every title that came via a delta sync. Unit tests on a fresh-fetch parser won't catch it. (Has bitten poster `*_url`, AniList `score`, and watch `links`.)
- **Clippy:** the canonical `[lints.clippy]` block (pedantic + nursery + restriction allows) in `Cargo.toml` is the single source of truth. Per-item exceptions use a local `#[allow(clippy::…)]` with a one-line reason — never widen the global table. Gate: `cargo clippy --all-targets -- -D warnings`.
- **Security boundary:** all external keys (Anthropic / OpenAI / Plex / Movie-of-the-Night) are server-side `Config` only and are NEVER serialized to the client. `BIND_ADDR` defaults to `127.0.0.1`.

## Secrets

`.env` and `data/` are gitignored — never commit them. Copy `.env.example` → `.env` for local and `docker compose` runs.
