# cue — Sort Orders: Trending, For You, Leaving Soon

**Date:** 2026-10-02 (revised 2026-10-03 after red-team review)
**Status:** Design — approved in brainstorming, revised, pending spec review
**Scope:** Backend (migration, MOTN side tables + budget guard, catalogue DTO, new
endpoint, sync status) and frontend (sort menu, store, filter bar, poster card,
detail, settings).
**Related:** `2026-06-23-motn-incremental-sync-design.md` (delta sync this extends),
`2026-06-22-cue-ask-engine-design.md` (embeddings For you reuses),
`docs/superpowers/reviews/2026-10-02-cue-sort-orders-redteam.md` (review that drove
revision 2).

## 1. Problem

- **"Trending" sorts by nothing in browse mode.** With no answer active, the
  `'trending'` case in `visibleTitles` keeps the base order, which is `ORDER BY id`
  (`src/db/catalogue.rs`) — first-insert order from the initial MOTN seed (MOTN's
  default `original_title` order) with later additions appended.
- **But the same no-op is load-bearing in answer mode.** When an Ask / "More like" /
  refine answer is active, `visibleTitles` builds its base from `resultIds`
  (`catalogue.ts` `visibleTitles`, `applyResult`), and the no-op is what keeps the
  engine's ranking ("Shortest first", closest match first). Any real Trending order
  must not leak into answer sets.
- There is no personal ranking, though title embeddings and 1–10 ratings both exist.
- Nothing tells the user a title is about to leave their services.

## 2. Decisions

| # | Decision |
|---|---|
| D1 | **Answer sets keep engine order.** A **Relevance** sort exists only while an answer is active and is selected automatically when one starts; the user can still pick another sort. Dismissing the answer restores the previous browse sort. |
| D2 | **Trending (browse default) = tiers:** this week's popular titles (MOTN `popularity_1week`, top 60) by rank → titles new to the catalogue, newest first → everything else by external rating. |
| D3 | "New" = `titles.added_at` after a **catalogue baseline** (§4). Titles present at the baseline are not new. No per-service date pipeline. |
| D4 | **Leaving soon is a badge + filter**, not a sort, from a MOTN `expiring` **snapshot** side table. A title is leaving only if it is leaving **every** service it is on (anything on Plex never leaves). Window: 30 days. |
| D5 | Popularity and expiring are **optional passes**, refreshed together **every 3 days**, never fatal to a sync. |
| D6 | **For you** sort: weighted ratings centroid over stored embeddings. 7–10 pull towards (weight `r − 6`), 1–4 push away (weight `r − 5`), 5–6 ignored. Needs ≥ 3 positive ratings, else disabled with a hint. |
| D7 | Under For you, watched and rated titles **sink to the bottom** (sorts reorder, filters remove). |
| D8 | **The MOTN budget (500 requests/month) is enforced**, not assumed: monthly counter, soft cap for optional calls, 429 handling, a minimum interval between MOTN fetches, and back-off after a failed seed. |
| D9 | The Settings sync card shows MOTN cache and budget state. |

**Not changed by this spec:** `FetchedTitle`, `merge`, `store::reconcile_service`,
`CachedTitle` / the MOTN cache payload, and the Plex parser. (Revision 1's per-service
`ServiceOffer` refactor was dropped: existing cached titles would never get dates in
steady state because re-seeds only happen on recovery.)

Already shipped on this branch during brainstorming (not part of this plan): the
similar-titles strip uses `/api/ask/similar` (`d97acc4`), and MOTN `removed` changes
drop only the affected service (`72fafe0`).

## 3. MOTN API facts this relies on

From the official OpenAPI spec (`movieofthenight/streaming-availability-api`):

- `GET /shows/search/filters` accepts `order_by=popularity_1week` (also `_1month`,
  `_1year`, `_alltime`), default `desc`. Popularity is an **order only** — position
  is the rank. With `series_granularity=show`, a page holds **20** items.
- `GET /changes?change_type=expiring`: future changes; `from` defaults to today and
  the window extends up to 31 days ahead. Every change carries a required `service`
  and an optional `timestamp` (omitted when the exact date is unknown); embedded
  `shows` carry `imdbId`. 25 changes per page. Because `from`/`to` are omitted, each
  fetch is a **complete snapshot** of the next ~31 days, not a delta.

## 4. Data model (migration `0011_sort_orders.sql`)

```sql
CREATE TABLE motn_popularity (
    imdb_id    TEXT PRIMARY KEY,
    rank       INTEGER NOT NULL,   -- 1 = most popular
    fetched_at INTEGER NOT NULL    -- unix secs
);

CREATE TABLE motn_expiring (
    imdb_id    TEXT NOT NULL,
    service    TEXT NOT NULL,      -- 'disney' | 'crunchyroll'
    expires_on INTEGER NOT NULL,   -- unix secs, last day to watch
    PRIMARY KEY (imdb_id, service)
);

-- Existing installs: everything already in the catalogue predates the feature,
-- so none of it counts as "new".
INSERT INTO app_meta (key, value)
SELECT 'catalogue.baseline_at', CAST(strftime('%s', 'now') AS TEXT)
WHERE EXISTS (SELECT 1 FROM sync_runs WHERE status = 'ok');
```

On a fresh install (no ok sync yet) the baseline is unset; `run_sync` sets
`catalogue.baseline_at` to its finish time after a successful sync **only if the key
is still unset**, so the initial import is not "new". While unset, nothing is new.

`app_meta` keys:

| Key | Value |
|---|---|
| `catalogue.baseline_at` | unix secs; titles added after this are "new" |
| `motn.catalogs`, `motn.catalogs_checked_at` | cached `/countries` result + when |
| `motn.optional_fetched_at` | last successful popularity + expiring refresh |
| `motn.last_fetch_at` | last successful **network** seed or delta (see §5.3) |
| `motn.last_seed_at`, `motn.last_mode` | last full seed time; `seed` / `delta` / `skipped` |
| `motn.seed_failed_at` | last failed seed attempt (back-off) |
| `motn.requests.YYYY-MM` | MOTN HTTP requests made in that UTC month |

After adding the migration run `cargo clean -p cue` before testing (CLAUDE.md).

## 5. Sync changes (MOTN client only)

### 5.1 Request counter and 429

Every MOTN HTTP call goes through one helper that increments
`motn.requests.<current UTC month>` before sending. The MOTN plan's reset day is not
known; the calendar month is an approximation (stated in Settings).

A `429` aborts the MOTN run: the source fails as today (scoped — its services keep
their membership), the remaining calls are skipped, and the warning names the
month's count.

### 5.2 Run gating (before any network call)

1. **Minimum interval:** if `motn.last_fetch_at` is under **12 h** old (manual
   "Sync now" or a frequent `SYNC_CRON`), skip the network entirely and return the
   cache. `last_mode = skipped`.
2. **Seed back-off:** if a seed is due and `motn.seed_failed_at` is under **3 days**
   old, skip and return the cache (no daily retry-from-page-1 loop). A seed that
   fails records `seed_failed_at`.

A skipped run returns `motn_cache::load_all` (so membership is unchanged) and does
**not** move `motn.last_fetch_at`.

### 5.3 Delta window source

`decide_mode` and the delta `from` use `motn.last_fetch_at` instead of the last ok
`sync_runs` row, because a skipped run still records ok. When the key is absent
(upgrade), fall back to the current `sync_runs` logic once.

### 5.4 Core run (unchanged except gating and counting)

1. **Catalogs:** reuse `motn.catalogs` if checked under **7 days** ago; otherwise call
   `/countries`. A full seed always re-resolves. If `/countries` fails but a cached
   value exists, use it and warn.
2. **Seed or delta:** as today (`new` + `removed`). On success set
   `motn.last_fetch_at` (and `last_seed_at` on a seed).

### 5.5 Optional passes (popularity + expiring)

Run after a successful core run when `motn.optional_fetched_at` is missing or ≥
**3 days** old, **and** the month's request count is under the **soft cap of 400**.
Each pass is independent and **non-fatal**: on any error it logs a warning, keeps its
previous table, and the MOTN source still records ok.

- **Popularity:** 3 pages of
  `/shows/search/filters?country&catalogs&order_by=popularity_1week&series_granularity=show`.
  Rank = position across pages (1–60), first occurrence wins, shows without an
  `imdbId` skipped. Replace `motn_popularity` in one transaction.
- **Expiring:** `/changes?country&catalogs&change_type=expiring&item_type=show`
  (no `from`/`to`), paginated, **capped at 8 pages** (warn if truncated). Keep
  changes with a `timestamp` and a wanted `service` whose show has an `imdbId`;
  replace `motn_expiring` in one transaction (so extended or cancelled expiries
  disappear on the next refresh).

`motn.optional_fetched_at` is set when both passes succeed. With a 3-day refresh of a
31-day snapshot, a title expiring 28–30 days out can show its badge up to 3 days late.

### 5.6 Request budget (per 30 days, one sync a day)

| Item | Requests |
|---|---|
| Core delta (`new` + `removed`, ≥ 1 page each) | ~60–90 |
| Popularity (3 pages / 3 days) | ~30 |
| Expiring (1–8 pages / 3 days) | ~10–80 |
| `/countries` (weekly) | ~5 |
| **Steady state** | **~105–205** |
| Full re-seed (fresh install / > 25 days without a fetch) | +~250–340 |

Extra manual syncs within 12 h cost nothing (§5.2). Above 400 in a month the optional
passes stop, so a re-seed always has room in practice.

## 6. Ranking rules and API

### 6.1 Catalogue DTO (`TitleListItem`, `GET /api/catalogue`)

| Field | Rule |
|---|---|
| `trendRank: number \| null` | `motn_popularity.rank` joined on `imdb_id`, only if `fetched_at` is under **14 days** old. |
| `newSince: number \| null` | `strftime('%s', titles.added_at)` if `catalogue.baseline_at` is set and `added_at` is later than it; else null. |
| `leavingOn: number \| null` | Join `title_services` to `motn_expiring` on (`imdb_id`, service). Set only if **every** service row has an `expires_on > now`; value = the **latest** of them. Any Plex row, or any service without an expiry → null. |

`TitleDetail` (`GET /api/titles/:id`) carries `leavingOn` too. A title without an
`imdb_id` never has `trendRank` or `leavingOn`.

### 6.2 Sorts (frontend, `visibleTitles`)

- **Relevance** (only while `answerActive`): keep base (`resultIds`) order.
- **Trending:** (1) `trendRank` ascending; (2) `newSince` descending; (3) the rest by
  `externalRating` descending (nulls last); ties by `id` ascending.
- **For you:** `forYou.ids` order first, then every remaining visible title (watched,
  rated, no embedding) in Trending order. If picked during an answer, it reorders the
  answer set — an explicit user choice.
- Top rated, Newest (release year), A–Z: unchanged.

### 6.3 For you — `GET /api/for-you` → `{ ids: number[], basis: number }`

1. Load ratings joined to catalogue titles that have an embedding for the current
   `EMBED_MODEL`.
2. Weight: `r ≥ 7 → r − 6` (1..4); `r ≤ 4 → r − 5` (−1..−4); 5–6 skipped.
3. `basis` = number of positive weights; if `basis < 3` → `{ ids: [], basis }`.
4. Profile = Σ weight × (vector / ‖vector‖).
5. Candidates = titles with an embedding, **not watched and not rated**; ranked by
   cosine(profile, vector) desc, ties by id.

Stored vectors only (no external calls); reuses `services::similarity`.

### 6.4 Sync status (`GET /api/sync/status`)

```json
"motn": {
  "cacheSize": 4812,
  "lastMode": "delta",
  "lastSeedAt": 1757635200,
  "lastFetchAt": 1759449600,
  "catalogsCheckedAt": 1759017600,
  "optionalFetchedAt": 1759190400,
  "requestsThisMonth": 37,
  "softCap": 400
}
```

`null` when MOTN is not configured.

## 7. Frontend

- **Types:** `Title` gains `trendRank`, `newSince`, `leavingOn` (validated as
  number-or-null in `isTitle`); `TitleDetail` gains `leavingOn`.
- **Sort menu** (`FilterBar`): Relevance (answer mode only) · Trending (browse
  default) · For you · Top rated · Newest · A–Z. When For you is unavailable
  (`basis < 3`), its option is `disabled` with the hint "Rate 3+ titles you liked to
  unlock For you".
- **Store** (`catalogue.ts`):
  - `SortKey` adds `'relevance'` and `'foryou'`. `applyResult` saves the browse sort
    (if not already in an answer) and sets `'relevance'`; clearing the answer restores
    it. `stepThread` keeps the current sort.
  - `forYou: { status, ids, basis }`; `loadForYou()` runs on catalogue load and again
    after `setRating` / `clearRating` succeed and after the post-sync reload. Until
    `ready`, For you renders in Trending order.
  - `leavingSoon: boolean` filter + `setLeavingSoon`; keeps titles whose `leavingOn`
    is within `LEAVING_WINDOW_DAYS = 30`.
- **Filter bar:** "Leaving soon" toggle with `aria-pressed`.
- **Poster card:** "Leaving 14 Oct" badge, bottom-left, inside the window. Text, not
  colour-only.
- **Detail view:** the same badge near the service pills.
- **Settings:** a MOTN line, e.g. "Cache 4,812 shows · last full seed 12 Sep · delta ·
  popularity & expiring 30 Sep · 37 / 500 requests this month (approx.)".

## 8. Error handling

| Failure | Behaviour |
|---|---|
| Popularity or expiring pass fails | Warn, keep the previous table; MOTN source still ok. Ranks older than 14 days stop counting. |
| Month count ≥ 400 | Optional passes skipped (logged); core sync continues. |
| MOTN returns 429 | Abort the MOTN run (source error, services untouched); remaining calls skipped. |
| Seed fails | Record `seed_failed_at`; no seed retry for 3 days; cache keeps serving. |
| Manual sync within 12 h of the last fetch | MOTN skipped (cache served); Plex syncs normally. |
| `/countries` fails, cached catalogs exist | Use cache, warn. |
| `/api/for-you` fails | Trending order + inline "For you unavailable — retry"; never an empty grid. |
| No embeddings at all | `basis = 0` → option disabled with the hint. |

## 9. Testing

Hand-built JSON fixtures only (no MOTN key in CI or the cloud environment).

**Rust**
- Request counter increments per call and keys by UTC month; 429 aborts and skips
  remaining calls.
- Gating: skip within 12 h; seed back-off within 3 days; skipped runs return the cache
  and leave `last_fetch_at` alone; `decide_mode` uses `last_fetch_at` with the
  `sync_runs` fallback.
- Catalogs reuse within 7 days; re-resolve after; cached fallback on failure.
- Popularity: rank across pages, de-dup, no-imdb skip, replace-in-transaction, failure
  keeps the old table, soft cap skips it, due/not-due.
- Expiring: snapshot replace, timestamp-less and unwanted-service changes dropped,
  page cap, failure keeps the old table.
- Migration: baseline set on an install with an ok sync, unset on a fresh one;
  `run_sync` sets it after the first ok sync.
- DTO: `trendRank` freshness; `newSince` baseline rule; `leavingOn` (all services
  expiring → latest; any Plex → null; past dates ignored; no imdb → null).
- `/api/for-you`: weights, `basis < 3` → empty, watched/rated excluded, ties by id.
- `/api/sync/status` `motn` block present / null.

**Frontend**
- Relevance: an answer keeps engine order under the default; starting an answer
  selects Relevance; clearing restores the browse sort.
- Trending tiers and null handling; For you order with the sunk remainder; disabled
  state; reload after a rating change.
- Leaving filter including the 30-day edge; badge shown inside / hidden outside.
- `SettingsView` MOTN line renders; hidden when null.

Live checks against real MOTN/Plex data go on the homelab verification list.

## 10. Out of scope

- `change_type=upcoming` ("coming soon"); a "new" badge.
- Resumable seeds (persisting the cursor and partial pages); back-off covers the
  retry loop.
- User-configurable windows (30-day leaving, 3-day refresh, 12 h interval, 400 cap are
  constants).
- Alternative For you scoring (e.g. top-k similarity instead of a centroid).
