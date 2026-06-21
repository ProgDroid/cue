# Task 6 Report: Static SPA Serving + Server Bootstrap

## Files Changed

- **Created**: `src/static_files.rs` — `StaticDir` newtype, `serve_spa` catch-all handler, inline test.
- **Modified**: `src/lib.rs` — added `pub mod static_files;`.
- **Rewritten**: `src/main.rs` — full lib+bin bootstrap (tracing, dotenv, config, DB init, seed, HttpServer).

## Automated Tests

```
cargo test --lib static_files
```
**PASS** — 1 test: `fallback_serves_placeholder_when_no_build`

```
cargo test
```
**PASS** — 10 tests (all lib tests; 0 binary, 0 doc-tests):
- config: 4 tests
- models: 2 tests
- static_files: 1 test
- db: 2 tests
- routes::catalogue: 1 test

## Clippy

```
cargo clippy --all-targets -- -D warnings
```
**CLEAN** — no warnings or errors.

Three issues were fixed relative to the brief's verbatim code:
1. `clippy::too_long_first_doc_paragraph` — split doc comment into a short first sentence + longer second paragraph.
2. `clippy::future_not_send` — added `#[allow(clippy::future_not_send)]` with justification (Actix handlers are single-threaded).
3. `clippy::option_if_let_else` — rewrote `match` on `Result` as `.map_or_else(...)`.

## Manual Smoke Test

Run with `DATABASE_URL=sqlite::memory:` (no `.env` present in repo; `.env` is gitignored):

| Endpoint | Expected | Result |
|----------|----------|--------|
| `GET /api/health` | `{"status":"ok"}` | PASS |
| `GET /api/catalogue` | JSON array of titles | PASS (first title: "Frieren: Beyond Journey's End") |
| `GET /` | placeholder HTML | PASS (`cue backend running` in body) |

Server was started in background and stopped cleanly after curl checks.

## Commit

See git log for commit hash and message `feat(backend): SPA static serving and server bootstrap`.

## Self-Review Notes

- The `#[allow(clippy::future_not_send)]` diverges from the brief's verbatim code but is required to satisfy the clippy gate (`cargo clippy --all-targets -- -D warnings`). The suppression is correct: Actix-web handlers run on a single-threaded Tokio runtime and `HttpRequest` is intentionally `!Send`.
- `src/main.rs` binary declares no `mod` of its own — all module access is via `cue::` as specified.
- The smoke test used an in-memory SQLite DB because no `.env` file is present (gitignored by design); the seed ran and returned real data confirming end-to-end DB path works.
