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

## Polish (cosmetic, non-blocking)

- Two `DetailView` colour literals remain bare because no design token matches
  their value: `#8a7a30` (dimmed-amber IMDb label) and `#d2d6dd` (sim-card
  title — nearest is `--text-secondary` `#c2c7d0`, which would shift the
  rendered colour). Add tokens for them if they should be themeable; otherwise
  leave as intentional one-offs.

---

### Cleared 2026-06-22 (in-repo follow-ups)

- ✅ **PR-triggered CI** — `.github/workflows/ci.yml` mirrors the backend
  (clippy + test) and frontend (build + test) gates on `pull_request` to `main`.
- ✅ **`parse_section` hardening** — unknown Plex `type` values are now skipped
  with a `tracing::warn!` instead of being coerced to `Movie`
  (`skips_items_with_unexpected_type` test added).
- ✅ **Tokenised `DetailView`/`SettingsView` hex literals** — exact-match hexes
  moved to `var(--token, #fallback)`; added a `--text-danger` token for the
  Settings error text.
- ✅ **Micro-opts** — `merge::merge` now sizes its maps with capacity hints and
  moves (not clones) first-seen genres; `catalogue_stats` collapsed 4 queries
  into 2 (one aggregate over `titles`, one count over `title_embeddings`).
