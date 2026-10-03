# cue — Sort Orders: Trending, For You, Leaving Soon

**Date:** 2026-10-02 (revision 4, 2026-10-03, after three red-team reviews)
**Status:** Design — approved in brainstorming, revised, pending spec review
**Scope:** Backend (catalogue DTO, For you endpoint, MOTN request
accounting, sync status) and frontend (sort menu, store, settings). Phase 2 adds MOTN
popularity and expiring side tables, gated on evidence.
**Related:** `2026-06-23-motn-incremental-sync-design.md`,
`2026-06-22-cue-ask-engine-design.md`; reviews
`docs/superpowers/reviews/2026-10-02-cue-sort-orders-redteam.md`,
`…-r2-redteam.md` and `…-r3-redteam.md`.

## 1. Problem

- **"Trending" sorts by nothing in browse mode.** With no answer active, the
  `'trending'` case in `visibleTitles` keeps the base order, `ORDER BY id`
  (`src/db/catalogue.rs`) — first-insert order from the initial MOTN seed.
- **The same no-op is load-bearing in answer mode.** With an Ask / "More like" /
  refine answer active, `visibleTitles` builds its base from `resultIds`, and the no-op
  is what keeps the engine's ranking. A real browse order must not leak into answers.
- There is no personal ranking, though embeddings and 1–10 ratings exist.
- Nothing warns that a title is about to leave the user's services.
- MOTN usage (500 requests/month) is unmeasured: there is no request counter, no 429
  handling, and a failed seed retries from page 1 every day.

## 2. Decisions

| # | Decision |
|---|---|
| D1 | **Answer sets keep engine order:** a **Relevance** sort exists only while an answer is active, is selected automatically when one starts, and the previous browse sort is restored when it is cleared. |
| D2 | **Trending (browse default):** Phase 1 = titles **new in the last 30 days**, newest first → everything else by external rating. Phase 2 prepends this week's MOTN popular titles (top 60) by rank. |
| D3 | "New" = `titles.added_at` within 30 days **and not part of a bulk insert** (fewer than 300 titles added within ±30 minutes of it). Stateless: no baseline, migration or flag, and it covers seeds, a MOTN key added later and large Plex imports alike. |
| D4 | **For you** sort, scored per candidate against its **closest liked titles** (top-k by plain cosine on mean-centred vectors, then rating-weighted), minus similarity to disliked ones — not a single centroid, so distinct tastes stay distinct. Constants are provisional until measured on the real library. |
| D5 | Under For you, watched and rated titles **sink to the bottom** (sorts reorder, filters remove). |
| D6 | **MOTN accounting first:** request counter, 429 abort, failed-seed back-off and `/countries` reuse ship in Phase 1, with no new MOTN calls. |
| D7 | **Phase 2 is gated on evidence:** popularity ships once ~2 weeks of real counts show headroom; Leaving soon ships only if a live `expiring` request for UK Disney+/Crunchyroll returns dated entries. |
| D8 | The Settings sync card shows MOTN cache and request state. |

**Not changed by this spec:** `FetchedTitle`, `merge`, `store::reconcile_service`,
`CachedTitle` / the MOTN cache payload, the Plex parser. No 12-hour gate: a manual
"Sync now" always syncs MOTN, and its cost is visible in Settings.

Already shipped on this branch during brainstorming (not in this plan): the
similar-titles strip uses `/api/ask/similar` (`d97acc4`); MOTN `removed` changes drop
only the affected service (`72fafe0`).

---

# Phase 1 — no new MOTN calls

## 3. Data model

**No migration in Phase 1.** State lives in the existing `app_meta` table:

| Key | Value |
|---|---|
| `motn.requests.YYYY-MM` | MOTN HTTP requests made in that UTC month |
| `motn.catalogs`, `motn.catalogs_checked_at` | cached `/countries` result + when |
| `motn.last_seed_at`, `motn.last_mode` | last full seed time; `seed` / `delta` |
| `motn.seed_failed_at` | last failed seed attempt |

(Revision 3 had a `catalogue.baseline_at` key set at seed finish. That was wrong: the
seed finishes inside `MotnClient::fetch`, but `added_at` is stamped later, when
`run_sync` upserts the rows, so every seeded title landed after the baseline. The
stateless bulk rule in §5.1 replaces it.)

## 4. MOTN accounting (MOTN client only)

- **Counter:** every MOTN HTTP call goes through one helper that increments
  `motn.requests.<UTC month>` before sending. The plan's reset day is unknown; the
  calendar month is an approximation and Settings says so.
- **429:** aborts the MOTN run — the source fails as any MOTN error does today
  (scoped: its services keep their membership), remaining calls are skipped, and the
  warning includes the month's count.
- **Seed back-off:** a failed seed records `motn.seed_failed_at` — but only when the
  failure is a 429 or happened after seed pages were requested; a catalog-resolution
  failure (≤ 1 request) does not back off, so a network blip on a fresh install can retry
  on the next run (final-review revision). While a seed is due
  and that is under **3 days** old, `fetch` returns an error ("seed back-off until …")
  **without any network call**. It deliberately does not return the cache: that would
  record an ok run and make the next delta start from the wrong time.
- **Catalogs reuse:** use `motn.catalogs` if checked under **7 days** ago; else call
  `/countries` and store it. A full seed always re-resolves. If `/countries` fails and a
  cached value exists, use it and warn.
- Record `motn.last_mode`, and `motn.last_seed_at` on a successful seed.

Steady state stays the current ~60–90 requests/month (`new` + `removed` daily) minus
~25 for `/countries` reuse.

## 5. Ranking rules and API

### 5.1 Catalogue DTO (`TitleListItem`, `GET /api/catalogue`)

`newSince: number | null` = `strftime('%s', titles.added_at)` when both hold, else
null:

- `added_at` is within the last **30 days**;
- **fewer than 300** titles (`BULK_THRESHOLD`) have an `added_at` within **±30
  minutes** (`BULK_WINDOW`) of it, counting itself.

Computed in `fetch_catalogue` in Rust: sort the `added_at` values and count each
title's neighbours with a two-pointer sweep, O(n log n) over ~5k rows. Seeds,
recovery seeds that re-insert many titles, a MOTN key added after Plex, and large Plex
imports are all bulk; daily deltas (even a big monthly Disney+ drop, tens of titles)
and individual Plex additions are not.

### 5.2 Sorts (frontend, `visibleTitles`)

- **Relevance** (only while `answerActive`): keep base (`resultIds`) order.
- **Trending:** (1) `newSince` descending; (2) the rest by `externalRating`
  descending, nulls last; ties by `id` ascending. (Phase 2 inserts `trendRank` first.)
- **For you:** `forYou.ids` order, then every remaining visible title in Trending
  order. If picked during an answer it reorders the answer set — an explicit choice.
- Top rated, Newest (release year), A–Z: unchanged.

### 5.3 For you — `GET /api/for-you` → `{ ids: number[], basis: number }`

**Vectors.** A process-wide `VectorCache` holds every catalogue embedding for the
current `EMBED_MODEL`, decoded once, **mean-centred** (the catalogue's average vector
subtracted, which widens the otherwise narrow cosine band produced by one shared
embedding template) and re-normalised. It is rebuilt only after a sync that changed
titles or embeddings, not per request.

**Ratings.** Ratings joined to catalogue titles present in the cache.

- Positives `P`: ratings 7–10, weight `w = (r − 6) / 4` (0.25..1).
- Negatives `N`: ratings 1–4, weight `w = (5 − r) / 4` (0.25..1). 5–6 ignored.
- `basis = |P|`; if `basis < 3` → `{ ids: [], basis }`.
- Caps: at most 300 positives (highest weight, then most recent) and 100 negatives.

**Score** for each candidate `c` (in the cache, **not watched, not rated**):

1. Take the `k = min(5, |P|)` positives with the **highest plain cosine** to `c`
   (selection ignores weights, so it is driven by similarity).
2. `pos(c)` = weighted mean of those `k` cosines, `Σ w·cos / Σ w`.
3. `neg(c)` = the highest plain cosine to any negative, times that negative's weight
   (0 if `N` is empty).
4. `score(c) = pos(c) − λ · neg(c)`, `λ = 0.5`.

`ids` = candidates by score desc, ties by id. `k`, `λ` and the caps are provisional:
before tuning, measure the real cosine spread on the homelab database (centred and
not) and record it in the plan.

**Performance.**

- The dot product loops over fixed-width chunks (`chunks_exact(8)` with separate
  accumulators) so the compiler can vectorise it. Same results as `similarity::dot`
  up to float rounding.
- The computation runs in `web::block` / `spawn_blocking`, never on an actix worker.
- Results are cached by fingerprint (`COUNT(*)`, `MAX(rated_at)` of `user_ratings`;
  watch-history count; `VectorCache` generation), so page reloads are free. Every
  rating or watched change misses by design; with vectors already decoded, such a
  recompute is ≤ ~5k × 400 dot products.
- **Fallback if this measures too slow on the homelab:** precompute each title's
  top-50 nearest neighbours after a sync and score from those lists, which costs
  O(|P| × 50) per request.

No external calls.

### 5.4 Sync status (`GET /api/sync/status`)

```json
"motn": {
  "cacheSize": 4812,
  "lastMode": "delta",
  "lastSeedAt": 1757635200,
  "seedFailedAt": null,
  "catalogsCheckedAt": 1759017600,
  "requestsThisMonth": 37,
  "monthlyLimit": 500
}
```

`null` when MOTN is not configured.

## 6. Frontend (Phase 1)

- **Types:** `Title` gains `newSince` (number-or-null in `isTitle`).
- **Sort menu** (`FilterBar`): Relevance (answer mode only) · Trending (browse
  default) · For you · Top rated · Newest · A–Z. When `basis < 3`, For you is `disabled`
  with the hint "Rate 3+ titles you liked to unlock For you".
- **Store** (`catalogue.ts`): `SortKey` adds `'relevance'` and `'foryou'`.
  `applyResult` saves the browse sort (if not already in an answer) and sets
  `'relevance'`; clearing the answer restores it; `stepThread` keeps the current sort.
  `forYou: { status, ids, basis }`; `loadForYou()` runs on catalogue load, after
  `setRating` / `clearRating` / `toggleWatched` succeed, and after the post-sync
  reload. On the first load, For you renders in Trending order until `ready`. On a
  **reload** the previous `ids` stay in use until the new ones arrive (no flash back to
  Trending), and a superseded response is ignored. The client always sinks watched and
  rated titles locally (they are not in `ids`), so an optimistic rating moves its title
  immediately.
- **Settings:** a MOTN line, e.g. "Cache 4,812 shows · last full seed 12 Sep · delta ·
  37 / 500 requests this month (approx.)", plus "seed paused until …" during back-off.

## 7. Error handling (Phase 1)

| Failure | Behaviour |
|---|---|
| MOTN returns 429 | Abort the MOTN run (source error, services untouched). |
| Seed fails | Record `seed_failed_at`; MOTN source errors without network calls for 3 days. |
| `/countries` fails, cached catalogs exist | Use cache, warn. |
| `/api/for-you` fails | Trending order + inline "For you unavailable — retry"; never an empty grid. |
| No embeddings | `basis = 0` → option disabled with the hint. |

## 8. Testing (Phase 1)

Hand-built JSON fixtures only (no MOTN key in CI or the cloud environment).

**Rust:** counter increments per call, keyed by UTC month; 429 aborts and skips the
rest; seed back-off errors with no network call (fake client asserts zero requests) and
expires after 3 days; catalogs reuse within 7 days, re-resolve after, cached fallback
on failure; `newSince`: 30-day window, a 300-title burst within ±30 min is excluded,
a 299-title burst and a lone addition are included, window edges; For you: centring
(fixture where raw cosines are all close but centred ones separate), top-k selected by
plain cosine not weight (a weight-1 distant like must not beat a weight-0.25 close
like), weighted mean, negative penalty, `basis < 3`, caps, watched/rated excluded,
ties; chunked dot equals `similarity::dot` within tolerance; `VectorCache` rebuilt
after a sync and reused otherwise; result cache hit / miss by fingerprint; `motn`
status block present / null.

**Frontend:** an active answer keeps engine order under the default sort; starting an
answer selects Relevance and clearing restores the browse sort; Trending tiers and
nulls; For you order with the sunk remainder, disabled state, reload after a rating or
watched change keeps the previous order until the new response, a stale response is
ignored; Settings MOTN line renders / hidden when null.

---

# Phase 2 — gated MOTN additions

Each part ships only after its gate passes; each gets its own plan.

## 9. Popularity tier for Trending

**Gate:** ~2 weeks of Phase 1 request counts on the homelab show steady state leaves
room for +~30/month and a re-seed (~250–340).

- Table `motn_popularity(imdb_id TEXT PRIMARY KEY, rank INTEGER NOT NULL,
  fetched_at INTEGER NOT NULL)`; `app_meta.motn.popularity_fetched_at`.
- After a successful core run, if the last refresh is ≥ **3 days** old and the month's
  count is under a **soft cap of 400**: 3 pages of
  `/shows/search/filters?country&catalogs&order_by=popularity_1week&series_granularity=show`
  (20 per page). Rank = position (1–60), first occurrence wins, no-imdb skipped;
  replace the table in one transaction.
- **Non-fatal:** on error, warn and keep the old table; the MOTN source still records
  ok.
- DTO `trendRank: number | null` (rank if `fetched_at` under 14 days old). Trending
  becomes `trendRank` → `newSince` → rating.

## 10. Leaving soon

**Gate:** in the homelab session, one live request

```sh
curl -s -H "X-API-Key: $MOTN_API_KEY" \
  "https://api.movieofthenight.com/v4/changes?country=gb&catalogs=disney,crunchyroll&change_type=expiring&item_type=show" \
  | jq '{n: (.changes|length), dated: ([.changes[]|select(.timestamp)]|length), hasMore}'
```

returns a meaningful number of dated entries. MOTN's spec puts no per-service limit on
`expiring` (only `upcoming` is restricted), but notes some services do not state exact
expiry dates, and `timestamp` is optional. If `dated` is ~0, drop this section.

- Table `motn_expiring(imdb_id, service, expires_on, PRIMARY KEY (imdb_id, service))`,
  replaced in one transaction from `/changes?change_type=expiring&item_type=show`
  (no `from`/`to` → a snapshot of the next ~31 days), paginated and **capped at 8
  pages**. Keep dated changes on a wanted service whose show has an `imdbId`. Runs
  with the popularity refresh (every 3 days, same soft cap), non-fatal.
- DTO `leavingOn: number | null`: set only if **every** service the title is on has an
  `expires_on > now` in the table; value = the latest. Any Plex row → null.
  `TitleDetail` carries it too.
- Frontend: "Leaving soon" filter toggle (`aria-pressed`) and a "Leaving 14 Oct"
  badge (poster card bottom-left; detail view near the service pills), both for
  `leavingOn` within 30 days. A title expiring 28–30 days out can appear up to 3 days
  late.

## 11. Out of scope

- `change_type=upcoming`; a "new" badge; resumable seeds (the back-off covers the
  daily retry loop, not the requests a failed seed already spent).
- User-configurable windows and caps (30-day new and leaving windows, 3-day refresh,
  3-day back-off, 400 soft cap, 300-title / ±30 min bulk rule are constants; For you's
  `k`, `λ` and caps are constants too, tuned once from homelab measurements).
