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

- ✅ **Plan 5 — user-data writes — DONE & merged 2026-06-23** (branch
  `feat/user-data-writes`). `PUT`/`DELETE /api/titles/:id/rating` +
  `PUT /api/titles/:id/watched` + persistence to `user_ratings`/`watch_history`
  (keyed on `imdb_id`, D6); frontend `toggleWatched`/`setRating`/`clearRating`
  now persist (optimistic + rollback). Rating scale widened 1–5 → **1–10**
  (migration `0002`), so the IMDb/Plex ratings imports below are now **lossless**
  (no downscaling). See spec `2026-06-23-cue-user-data-writes-design.md`.
- ✅ **Real poster artwork — DONE & merged 2026-06-23** (branch
  `feat/poster-artwork`). 4 nullable image columns added to `titles` via
  migration `0004` (`poster_url`, `backdrop_url`, `poster_path`, `backdrop_path`);
  MOTN `imageSet` (`verticalPoster.*`, `horizontalBackdrop.*`) and Plex
  `thumb`/`art` captured during sync via `parse_page` / `parse_section`;
  `GET /api/titles/:id/poster` and `GET /api/titles/:id/backdrop` proxy endpoints
  (302 to MOTN CDN for public URLs; buffered token-streamed proxy for Plex paths;
  404 when art is absent so the frontend falls back to the oklch placeholder);
  `<img>` overlay + `v-show` in grid cards, `DetailView` hero + similar-title
  thumbs — placeholder is always the base layer and degrades gracefully on
  network error. See spec `docs/superpowers/specs/2026-06-23-cue-poster-artwork-design.md`
  and plan `docs/superpowers/plans/2026-06-23-poster-artwork.md`.
- **Real poster artwork — live-verify (post-merge):** (a) pick a MOTN title and
  confirm `GET /api/titles/:id/poster` returns a 302 that resolves to a working
  `cdn.movieofthenight.com` URL; (b) pick a Plex-only title and confirm its
  poster streams through the backend with no Plex token visible in the
  client-side network request URL; (c) spot-check the grid and a detail page —
  real art should render for MOTN/Plex-backed titles; oklch placeholder should
  appear only where art is genuinely absent; (d) confirm the exact size-key
  names used by MOTN `imageSet` against a real `/shows` response and capture a
  `parse_page` fixture to lock the parser to live data (deferred from Task 4 —
  no MOTN key in the build env).
- **IMDb ratings-export import** → `user_ratings` (schema ready & now 1–10 so
  imports are lossless; importer UI deferred — master spec §11).
- **Plex watch-history import** → `watch_history` (schema ready — master spec
  §11). When built, note that manual un-watch deletes only `source='manual'`
  rows (D5.2), so imported `plex` rows are preserved.
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

## Plan 5 deferred minors (from per-task + whole-branch reviews, 2026-06-23)

All non-blocking; the whole-branch review verdict was "Ready to merge".

- **`watch_history` manual-watch idempotency is guard-only** (`set_watched`
  uses `INSERT … WHERE NOT EXISTS`, no constraint). Effectively unreachable for
  a single-user app. Future hardening: a partial
  `UNIQUE INDEX ON watch_history(imdb_id) WHERE source='manual'` to make it a
  hard invariant.
- `RatingBody.rating` is typed `i64` (wider than the 1–10 domain it's range-
  checked into); `u8`/`i32` would be more self-documenting. Harmless.
- A malformed rating body returns actix's default 400 shape rather than the
  JSON `{"error":…}` shape used for range errors (spec only requires the 400
  status).
- No dedicated `DELETE /rating` 404/422 test — the path is shared with `PUT`
  via `resolve_or_respond`, which is tested, so coverage is transitive.
- `migrations/0002_widen_rating_to_10.sql` has no trailing newline (cosmetic).

---

### Cleared 2026-06-23 (Plan 5 final fix wave)

- ✅ Added store rollback tests (`clearRating` failure; `setRating` no-prior-
  rating delete-branch) — closes the spec §6.5 rollback-coverage intent.
- ✅ Added a `.no-imdb-hint` render assertion to the disabled-controls test.
- ✅ Removed the stale `Writes below are local only — persistence is Plan 5`
  comment from `DetailView.vue`.

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

## MOTN incremental sync — post-merge verification (Plan Feat/motn-incremental-sync)

- **MOTN incremental sync — spec §12 assumption RESOLVED via docs (2026-06-23):**
  Validated against https://docs.movieofthenight.com/resource/changes — the `/changes`
  `200` response is `{ changes[] (25 items), shows{} (25 keys, map keyed by showId),
  hasMore, nextCursor }`. So `shows` IS a map (our `ChangesPage.shows: HashMap<String,
  Show>` is correct — no `Vec<Show>` change needed). The Show object (per
  /resource/shows) carries `id`, `imdbId`, and `streamingOptions`, and the embedded
  `/changes` shows are full Show objects — so `show_to_fetched` attributes a `new`
  title from its real `streamingOptions[gb]`; the both-services fallback only triggers
  if availability is entirely absent, which does not occur for a real embedded show.
  The final-review attribution concern is therefore moot in practice. No code change.
- **MOTN incremental sync — operational live-verify (post-merge, when convenient):**
  Only an operational confirmation remains (no correctness risk): on the server, run one
  sync with a non-empty cache and confirm logs show `MOTN delta: +N -M` (not a full
  seed) and that the monthly request counter rises by only a few. Optionally capture a
  real `/changes` response as a `parse_changes` fixture to lock the parser to live data.
- **MOTN cache state in settings UI (deferred):** `/api/sync/status` could surface
  cache size + last seed vs delta mode. Out of scope for the incremental-sync plan.
