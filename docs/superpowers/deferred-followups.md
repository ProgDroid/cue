# cue — Deferred Follow-ups (backlog)

**Created:** 2026-06-22 (after Plan 4 catalogue-sync merged + first live verification)

Tracked home for work intentionally deferred out of Plans 1–4. Each item that
gets picked up runs through the normal brainstorming → writing-plans →
subagent-driven-development cycle (it is not "planned" until it has its own
spec/plan). Plan 5 (user-data writes) is the next committed plan and is **not**
in this backlog.

## Whole-project audit (2026-06-23)

Five parallel specialist agents reviewed security, code quality/tech-debt, the
data layer, the frontend, and architecture/ops/deps. Verdict: **no Critical
findings**; the codebase is in good shape. Tier-1 quick wins + Docker hardening
were fixed on branch `hardening/audit-tier1`; the rest are recorded here.

### ✅ Fixed in this pass (branch `hardening/audit-tier1`)
- **Image-proxy SSRF/open-redirect hardening** — `serve()` now only 302s a stored
  art URL when it is `https://` on the MOTN CDN (`is_allowed_redirect`), guards
  the Plex `plex_path` against traversal/scheme/userinfo smuggling
  (`is_safe_plex_path`), and fetches Plex art through a shared `reqwest::Client`
  with a 10s timeout and `redirect::Policy::none()` (was a one-off
  `reqwest::get` — no timeout, followed redirects). `src/routes/images.rs`.
- **Latent index panic in `surprise`** — `rmap[ia]`/`rmap[ib]` → `.get().copied().unwrap_or(0.0)`
  so an embedding that outlived its title degrades instead of panicking.
  `src/services/ask_engine.rs`.
- **Sync identity indexes** — migration `0006_identity_indexes.sql` adds
  non-UNIQUE indexes on `titles(tmdb_id)` + `titles(plex_guid)` (the
  `find_existing` fallback was full-scanning per upsert).
- **Silent rating/watched failures** — `DetailView` now renders
  `store.userDataError` near the controls (the store tracked it but no component
  showed it; failed writes rolled back with no feedback).
- **Docker hardening** — non-root `USER cue` owning `/data`, plus a `HEALTHCHECK`
  hitting `/api/health` so `restart: unless-stopped` can recover a wedged
  process. *(Image build not verified locally — Docker daemon down; CI builds it.)*

### ✅ Tier 2 + Tier 3 — DONE 2026-06-23 (branch `hardening/audit-tier1`)
All cleared except two items deliberately left (see "Deferred" below).
- ✅ Up-front config validation (`Config::validate` — bad `BIND_ADDR`/`SYNC_CRON`
  fail at boot naming the value).
- ✅ Magic `<= 28` seed sentinel → `sync_runs::any_sync_ok()`.
- ✅ `isTitle` validates `imdbId`/`imdb`/`rating`.
- ✅ DetailView distinguishes `NotFoundError` from transient errors (+ retry).
- ✅ A11y: focusable grid/sim cards + focus rings + Enter/Space; labelled service
  dots, IMDb/watched badges; `StarRating` group + per-star labels + `aria-pressed`.
- ✅ `find_existing` moved into the upsert transaction (TOCTOU closed).
- ✅ `cargo audit` job added (reusable `tests.yml`, advisory-visibility).
- ✅ Non-root volume caveat documented (README).
- ✅ CI deduped into reusable `tests.yml` (called by `ci.yml` + `docker-publish.yml`).
- ✅ Operator `README.md` written (env table, compose quickstart, GHCR swap, auth warning).
- ✅ `sync_runs(source)` indexed (migration 0007) + `prune_old_runs` keeps newest 100/source.
- ✅ Unused `prune_orphans` gated to `#[cfg(test)]`.
- ✅ Plex section-walk extracted to `for_each_section`.
- ✅ Image-proxy SQL made static (`Kind::select_sql`).
- ✅ `ShimmerGrid` uses 158px min column; `posterPlaceholder` precomputed per sim
  card; redundant `!` removed; `ResizeObserver entries[0]` guarded; `useVirtualGrid`
  now has direct tests; ask `ids`/`baseIds` capped at 1000.

**Deferred (deliberate):**
- **Faint-token WCAG-AA contrast** — raising `--text-faint`/`--text-faintest` to AA
  would collapse them into `--text-muted` and flatten the intended type hierarchy.
  Left as-is for these small decorative mono captions; revisit if AA compliance is
  required (a design call).
- **`--cols` cold-first-paint flash** — can momentarily show 1 column before the
  ResizeObserver corrects within a frame. Accepted (self-heals); gating render on
  `containerWidth > 0` would trade it for a blank frame.
- `Box::leak` in the sync test fakes — harmless test-only artifact; left.

### Cross-cutting (informational)
- All `/api` endpoints are unauthenticated by design (D7, single-user,
  `127.0.0.1`). The instant cue is exposed beyond localhost, every write/import/
  sync endpoint becomes an unauthenticated mutator — worth a README warning, not
  code today.

## Scale — now live-relevant (catalogue is ~5097 titles, no longer the 28-row seed)

These were noted during Plan 3 as "fine at seed scale, revisit at prod scale."
The first real MOTN sync brought the catalogue to ~5097 titles, so they are now
live but still working acceptably:

- ✅ **`GET /api/catalogue` slim list DTO + lazy detail + virtualization — DONE & merged 2026-06-23.**
  `GET /api/catalogue` now returns a `Vec<TitleListItem>` (no `desc`/`cast`),
  materially reducing the startup payload. Full detail (description, cast, etc.)
  is fetched on demand via `GET /api/titles/:id`. The frontend `PosterGrid` uses
  a window-scroll virtualizer (`useVirtualGrid`; JS-authoritative `--cols`,
  measured row height) so only the visible rows are rendered regardless of
  catalogue size. Server-side pagination was intentionally **not** done (design
  D1 — catalogue is bounded ~5k and a slim list DTO achieves the same goal).
  See spec `docs/superpowers/specs/2026-06-23-cue-catalogue-scale-design.md`
  and plan `docs/superpowers/plans/2026-06-23-catalogue-scale.md`.
  **Live-verify (post-merge):** on a real ~5k catalogue confirm (a) the
  `/api/catalogue` payload is materially smaller than before (no desc/cast
  fields); (b) opening a title still shows description + cast via the detail
  fetch; (c) the grid renders a constant-height window while scrolling (DOM
  row count stays stable).
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
  Non-blocking polish noted at merge: ~~result line has no singular/plural
  handling ("1 rows skipped")~~ (fixed 2026-06-23, see Polish section);
  `matched` scans the full `titles` table
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
  **Decision 2026-06-23: deferred by choice (won't-build for now; revisit if
  the need actually surfaces).** Rationale captured during brainstorming so a
  future session doesn't re-derive it:
  - It deliberately breaks the load-bearing **D2 boundary** — retrieval finds
    candidates *from the catalogue* and Claude is server-constrained so it can
    never surface a non-owned title. That hallucination-proof grounding is a
    core trust property; this row is the one feature that punches a hole in it.
  - **Grounded version is disproportionate.** "Not on your services" means real
    titles on Netflix/Prime/HBO/etc., but the catalogue is sourced only from
    Disney+/Crunchyroll (`WANTED`) + Plex. Doing it properly means syncing a
    broad MOTN dataset for all GB services → catalogue bloat (~5k → tens of
    thousands), MOTN quota burn (the whole incremental-sync feature exists to
    conserve quota), and OpenAI embedding cost for all of it.
  - **Cheap version is ungrounded.** The alternative — let Claude free-generate
    "acclaimed titles like this, not in your library" as an extra `beyond` array
    on `/api/ask` — can hallucinate and carries no real availability data (we'd
    only know a pick isn't in the catalogue, not which service it's actually on).
  - **Weak actionability.** Results are by definition things you can't play; for
    a personal "what can I watch now" app, surfacing "go buy another sub" is
    somewhat anti-aligned.
  - **If revived**, the only proportionate shape is the cheap one: a Claude
    open-knowledge `beyond` array on `/api/ask`, filtered against the catalogue,
    rendered as a muted read-only row (no detail page, posters best-effort).
- **Auth (token/login)** — no-op middleware slot reserved (D7); single-user /
  `127.0.0.1` for now.

## Polish (cosmetic, non-blocking)

- ✅ **Bare colour literals tokenized — DONE 2026-06-23.** Added two tokens to
  `frontend/src/design/tokens.css` at their exact values (no rendered shift):
  `--accent-dim: #8a7a30` (dimmed-amber IMDb label) and
  `--text-secondary-strong: #d2d6dd` (brighter secondary). Consumers now use the
  `var(--token, #literal)` form: `DetailView` `.imdb-label` + `.sim-title`,
  `ServicePill` `.label`, and `RefineChips` `.chip` (the chip previously read
  `var(--text-secondary, #d2d6dd)`, which actually rendered `#c2c7d0` — a latent
  mismatch vs the design handoff, now fixed to the intended `#d2d6dd`).
- ✅ **IMDb-import result singular/plural — DONE 2026-06-23.** Added a small
  `plural(n, word)` helper in `SettingsView.vue`; the summary now reads
  "1 rating"/"2 ratings" and "1 row skipped"/"2 rows skipped".

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

## Watch-at-source deep links (2026-06-24, branch feat/watch-at-source-links)

Whole-branch review verdict was **Ready to merge** (no Critical/Major). These are
non-blocking follow-ups surfaced during the review:

- **Route-level test for the non-allowlisted-link → 404 path** (`watch::redirect`):
  the `is_allowed_motn_link` predicate is unit-tested (http/wrong-host/Plex), but
  no integration test seeds a stored-but-non-allowlisted link and asserts the
  `tracing::warn` arm returns 404. The most security-adjacent of the deferred gaps.
- **Log the swallowed DB error in `watch::redirect`:** the `plex_rating_key`/`link`
  lookups use `.ok()`, turning a transient DB error into a silent 404 with no log
  line. Add a `tracing::warn` on the `Err` arm for debuggability (read-only path,
  not a correctness/security issue).
- **`watchable` vs `plex_web_url` residual edge:** `watchable` gates Plex on
  `plex_rating_key + machine_id` but not `plex_web_url`. Removing `PLEX_URL`/
  `PLEX_WEB_URL` after a successful Plex sync would render a "Watch on Plex" button
  whose redirect 404s. Pathological config change (can't sync Plex without
  `PLEX_URL`); document or add the guard if it ever bites.
- **Minor test/cosmetic niceties:** rename `run_sync_persists_server_meta_non_fatally`
  → `…_persists_server_meta` (the Err test is the real non-fatal guard); add an
  exclusion assertion to the `watchable` test; `color-mix` in WatchLinks.vue needs
  Baseline-2023 browsers (acceptable for self-hosted).
