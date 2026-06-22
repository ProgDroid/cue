# cue — Deferred Follow-ups (backlog)

**Created:** 2026-06-22 (after Plan 4 catalogue-sync merged + first live verification)

Tracked home for work intentionally deferred out of Plans 1–4. Each item that
gets picked up runs through the normal brainstorming → writing-plans →
subagent-driven-development cycle (it is not "planned" until it has its own
spec/plan). Plan 5 (user-data writes) is the next committed plan and is **not**
in this backlog.

## Scale — now live-relevant (catalogue is ~5097 titles, no longer the 28-row seed)

These were noted during Plan 3 as "fine at seed scale, revisit at prod scale."
The first real MOTN sync brought the catalogue to ~5097 titles, so they are now
live but still working acceptably:

- **`GET /api/catalogue` returns the entire library in one response** — a full
  table scan server-side plus a large JSON payload hydrated to Vue on startup.
  Works at ~5k; revisit with pagination / a lighter list endpoint (or
  server-side filtering) as the library grows.
- **Ask retrieval brute-forces cosine** over all title vectors in Rust
  (`similarity` / `candidates_for`). ~5k × 1536-dim f32 is fast; revisit with a
  vector index (e.g. sqlite-vss / HNSW) only if latency becomes noticeable.
- Minor full-table fetches in `candidates_for` / `sort_by_runtime` — same story.

## Features (spec'd / intended, not yet built)

- **Plan 5 — user-data writes (NEXT, has its own plan ahead):** `PUT
  /api/titles/:id/rating` + `/watched` endpoints + persistence; wire the
  frontend's currently local `toggleWatched`/`setRating` to
  `user_ratings`/`watch_history` (keyed on `imdb_id`, D6).
- **Real poster artwork — now actionable.** MOTN show objects carry an
  `imageSet` with poster URLs (`verticalPoster.w240..w720`,
  `horizontalPoster`, backdrops); Plex items carry `thumb`/`art`. Capture an
  artwork URL during sync and wire `<img>` in the grid/detail, replacing the
  generated oklch placeholder (master spec §11). Needs a `titles` column (or a
  small `title_images` table) + sync mapping + frontend.
- **IMDb ratings-export import** → `user_ratings` (schema ready; importer UI
  deferred — master spec §11).
- **Plex watch-history import** → `watch_history` (schema ready — master spec
  §11).
- **"Not on your services" discovery row** — open recommendations on the
  retrieval backbone (master spec §11).
- **Auth (token/login)** — no-op middleware slot reserved (D7); single-user /
  `127.0.0.1` for now.

## Sync hardening (confirm against real data)

- **`parse_section` defaults an unknown Plex `type` to `Movie`.** In practice
  `fetch()` only feeds `movie`/`show` sections, so it is not triggered; confirm
  real Plex `type` strings during live verification and harden (skip/log) if
  other top-level types appear.

## Ops / CI

- **PR-triggered `ci.yml`.** Only push-to-`main` is gated (by
  `docker-publish.yml`); PRs currently run no CI. Add a PR workflow mirroring
  the backend + frontend gates (cargo test + clippy; npm test + build).

## Polish (cosmetic, non-blocking)

- `DetailView` hardcodes some hex colours instead of token vars; `SettingsView`
  error text uses a hardcoded `#f5a3a3`. Move to design tokens.
- Micro-opts surfaced in review and deferred: `merge::merge` capacity hints +
  an avoidable first-seen `genres.clone()`; `db::sync_runs::catalogue_stats`
  uses 4 queries where 1–2 aggregates would do.
