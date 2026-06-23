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
- ✅ **IMDb ratings-export import — DONE & merged 2026-06-23** (branch
  `feat/imdb-ratings-import`). `POST /api/import/ratings` accepts the raw IMDb
  ratings CSV (text body, `csv` crate, header-named `Const`/`Your Rating`/
  `Date Rated`); `parse_ratings` skips invalid/unrated rows; `import_ratings`
  upserts in one transaction — **import everything** regardless of catalogue
  membership (no FK; unmatched ratings sit dormant until a matching title
  syncs), **overwrite on conflict**, **preserve `Date Rated`** into `rated_at`
  via `COALESCE(?, datetime('now'))`. 1–10 lossless (migration `0002`). Endpoint
  returns `{ imported, skipped, matched }`; `400 no_ratings_found` on empty.
  Frontend Settings card uploads the file (`file.text()` → `importRatings`),
  shows the summary, and re-fetches the catalogue so ratings surface without a
  reload. Payload cap raised 256 KB → 8 MB (endpoint-scoped). See spec
  `docs/superpowers/specs/2026-06-23-cue-imdb-ratings-import-design.md` and plan
  `docs/superpowers/plans/2026-06-23-imdb-ratings-import.md`.
- **IMDb ratings import — live-verify (post-merge):** run a real IMDb ratings
  export through the Settings card and confirm (a) the summary counts look sane
  vs the file (imported + skipped ≈ rated rows; `matched` ≈ titles you own);
  (b) ratings appear on the grid/detail for owned titles after the automatic
  re-fetch (no manual reload); (c) real-export quirks parse cleanly — a UTF-8
  BOM on the first header, CRLF line endings, and the full 14-column header.
  The automated suite proves the contract + persistence; only a real export
  exercises these CSV quirks.
  Non-blocking polish noted at merge: result line has no singular/plural
  handling ("1 rows skipped"); `matched` scans the full `titles` table
  (fine at ~5k, scales with catalogue not import); endpoint test harness
  registers via plain `cfg.route()` so it wouldn't catch loss of the
  production `PayloadConfig`; `invalid_utf8` branch untested;
  `looks_like_iso_date` accepts impossible dates (e.g. `2021-02-31`).
- ✅ **Plex watch-history import — DONE & merged 2026-06-23** (branch
  `feat/plex-watch-history-import`). Folded into catalogue sync: new default-empty
  `CatalogueSource::fetch_watch_history()` + `watch_history_source()` tag gate
  (Plex → `Some("plex")`); `run_sync` applies `db::user_data::replace_watch_history`
  per successful tagged source (replace-by-source; skipped on source failure or
  tag-less source, so a Plex outage / MOTN never wipes plex rows). Movie watched =
  `viewCount>0`, series = `viewedLeafCount>0`; `imdb_id` keying (no-imdb skipped);
  `lastViewedAt` epoch→ISO; manual rows preserved (D5.2). Frontend reloads the
  catalogue when a sync completes so watched flags surface. See spec
  `docs/superpowers/specs/2026-06-23-cue-plex-watch-history-import-design.md` and
  plan `docs/superpowers/plans/2026-06-23-plex-watch-history-import.md`.
- **Plex watch-history import — live-verify (post-merge):** confirm the real Plex
  `/library/sections/{key}/all` field names (`viewCount`, `lastViewedAt`,
  `viewedLeafCount`) against a live server and capture a `parse_watch_history`
  fixture from real data; run one sync against a real Plex library and confirm
  watched titles light up in grid/detail after the post-sync catalogue refresh,
  and that un-watching in Plex clears the cue flag on the next sync.
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

## Plan 5 deferred minors — ✅ ALL CLEARED 2026-06-23

(From per-task + whole-branch reviews; all non-blocking. Cleared in one
hardening pass — backend tests 129 passing, clippy `-D warnings` clean.)

- ✅ **`watch_history` manual-watch idempotency now a hard invariant** —
  migration `0005_manual_watch_unique.sql` adds a partial
  `UNIQUE INDEX idx_watch_history_manual_unique ON watch_history(imdb_id) WHERE source='manual'`.
  The `set_watched` `INSERT … WHERE NOT EXISTS` guard is retained (still the
  normal path); the index makes a duplicate manual row a structural error.
  Plex/other sources are outside the partial predicate, so their repeat-view
  rows stay unconstrained. Test
  `manual_watch_unique_index_rejects_duplicate` proves both halves.
- ✅ **`RatingBody.rating` narrowed `i64` → `u8`** — converted with
  `i64::from(rating)` at the `set_rating` call. Self-documents the small
  non-negative domain; negatives/over-255 are now rejected at extraction (see
  next item) rather than reaching the 1–10 range check.
- ✅ **Malformed rating/watched body returns the JSON `{"error":"invalid_body"}`
  shape** — new `rating_json_config()` (`web::JsonConfig` with an
  `error_handler`) attached to the rating/watched `web::resource`s in
  `routes::configure` and mirrored in the test harness. Tests
  `put_rating_malformed_body_is_json_400` + `put_rating_negative_is_json_400`.
- ✅ **Dedicated `DELETE /rating` 404 + 422 tests added** —
  `delete_rating_unknown_title_is_404`, `delete_rating_null_imdb_is_422`
  (coverage was previously only transitive via the shared `resolve_or_respond`).
- ✅ **Trailing-newline nit moot** — all `migrations/*.sql` (incl. `0002`)
  already end in `\n` (verified at byte level); nothing to change.

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
