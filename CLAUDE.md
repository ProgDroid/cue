# cue — project guide

Self-hosted personal media-discovery app. **Backend:** Rust + Actix-web + SQLx + SQLite. **Frontend:** Vue 3 + TypeScript + Pinia (Plan 2). A single Docker container serves the Vue build as static files alongside the REST API.

**Design & plans:** design spec at `docs/superpowers/specs/2026-06-21-cue-design.md` (architecture decisions D1–D9); implementation plans at `docs/superpowers/plans/`. Built as a 5-plan sequence: foundation → frontend → ask engine → catalogue sync → user-data writes.

## Backend conventions (non-obvious — follow these)

- **Crate is lib + bin.** Modules live in `src/lib.rs` as `pub mod …`; `src/main.rs` is a thin binary that consumes them via the `cue::` path. Declare new modules in `lib.rs`, not `main.rs`, so `cargo test --lib` works.
- **SQLx: runtime queries only** (`sqlx::query` / `query_as` / `query_scalar`). Do NOT use the compile-time `query!` / `query_as!` macros — the project must build with no live `DATABASE_URL` and no offline cache.
- **Test databases:** use `tempfile::tempdir()` (NOT `NamedTempFile`, which holds a Windows file lock) and build the URL as `format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"))` — the backslash fixup is required for the `sqlite:` URL to parse on Windows. Keep the `TempDir` guard bound (`_dir`) for the test's lifetime.
- **Clippy:** the canonical `[lints.clippy]` block (pedantic + nursery + restriction allows) in `Cargo.toml` is the single source of truth. Per-item exceptions use a local `#[allow(clippy::…)]` with a one-line reason — never widen the global table. Gate: `cargo clippy --all-targets -- -D warnings`.
- **Security boundary:** all external keys (Anthropic / OpenAI / Plex / Movie-of-the-Night) are server-side `Config` only and are NEVER serialized to the client. `BIND_ADDR` defaults to `127.0.0.1`.

## Secrets

`.env` and `data/` are gitignored — never commit them. Copy `.env.example` → `.env` for local and `docker compose` runs.
