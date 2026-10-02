# cue — Sort Orders: Trending, For You, Leaving Soon

**Date:** 2026-10-02
**Status:** Design — approved in brainstorming, pending spec review
**Scope:** Backend (MOTN + Plex sync, migration, catalogue DTO, new endpoint, sync
status) and frontend (sort menu, store, filter bar, poster card, detail, settings).
**Related specs:** `2026-06-23-motn-incremental-sync-design.md` (delta sync this
extends), `2026-06-22-cue-ask-engine-design.md` (embeddings For You reuses).

## 1. Problem

- **"Trending" sorts by nothing.** The `'trending'` case in `visibleTitles` keeps the
  base order, which is `ORDER BY id` (`src/db/catalogue.rs`) — first-insert order from
  the initial MOTN seed (MOTN's default `original_title` order), with later delta
  additions appended. Re-syncs keep ids, so it never changes. It is also the default.
- There is no personal ranking, even though title embeddings and the user's 1–10
  ratings both exist.
- Nothing tells the user a title is about to leave their services, although MOTN
  returns `expiresOn` in payloads we already fetch.

## 2. Decisions (from brainstorming)

| # | Decision |
|---|---|
| D1 | **Trending = tiers:** this week's popular titles (MOTN `popularity_1week`, top 60) first by rank, then everything else newest-on-your-services first. |
| D2 | Popularity is refreshed **every 3 days** (3 pages, ~30 requests/month). |
| D3 | **Leaving soon is a badge + a filter**, not a sort. A title is leaving only when it is leaving **every** service it is on (anything on Plex never leaves). Window: 30 days. |
| D4 | **For you** is a new sort: weighted ratings centroid over stored embeddings. 7–10 pull towards (weight `r − 6`), 1–4 push away (weight `r − 5`), 5–6 ignored. |
| D5 | For you needs **≥ 3 positive ratings** (on catalogue titles with an embedding); below that the option is disabled with a hint. |
| D6 | Watched and rated titles **sink to the bottom** under For you rather than being hidden — sorts reorder, filters remove. |
| D7 | The Settings sync card shows **MOTN cache state** (cache size, last seed, mode, popularity/catalogs freshness). |
| D8 | The MOTN request budget (500/month) must leave room for one full re-seed in any month. |

Already shipped on this branch during brainstorming (not part of this spec's plan):
the similar-titles strip now uses `/api/ask/similar` (embedding ranking), and MOTN
`removed` changes drop only the affected service (`72fafe0`).

## 3. MOTN API facts this relies on

From the official OpenAPI spec (`movieofthenight/streaming-availability-api`):

- `GET /shows/search/filters` accepts `order_by=popularity_1week` (also `_1month`,
  `_1year`, `_alltime`), default `desc`. Popularity is an **order only** — no numeric
  score is returned, so result position is the rank. With `series_granularity=show`,
  a page holds **20** items.
- Each `streamingOption` carries `availableSince` (required, unix secs — when the
  option was first detected) and optional `expiresOn` (unix secs, last day to watch)
  plus `expiresSoon` (within a month).
- `GET /changes` supports `change_type=expiring` (future window: `from` defaults to
  today, up to 31 days ahead). Every change has a required `service`, and an optional
  `timestamp` (omitted for expiring items whose exact date is unknown). Embedded
  `shows` are full, current show objects.

## 4. Data model (migration `0011_sort_orders.sql`)

```sql
ALTER TABLE title_services ADD COLUMN available_since INTEGER; -- unix secs, nullable
ALTER TABLE title_services ADD COLUMN expires_on      INTEGER; -- unix secs, nullable

CREATE TABLE motn_popularity (
    imdb_id    TEXT PRIMARY KEY,
    rank       INTEGER NOT NULL,   -- 1 = most popular
    fetched_at INTEGER NOT NULL    -- unix secs
);
```

`app_meta` keys (existing key/value table):

| Key | Value |
|---|---|
| `motn.catalogs` | resolved catalogs CSV (e.g. `disney,crunchyroll`) |
| `motn.catalogs_checked_at` | unix secs of the last `/countries` call |
| `motn.popularity_fetched_at` | unix secs of the last successful popularity refresh |
| `motn.last_seed_at` | unix secs of the last full seed |
| `motn.last_mode` | `seed` or `delta` |

Plex rows always have `expires_on = NULL`. After adding the migration run
`cargo clean -p cue` before testing (CLAUDE.md: embedded migrations).

## 5. Sync changes

### 5.1 Per-service offer replaces `links`

`FetchedTitle.links: Vec<(Service, String)>` becomes:

```rust
pub struct ServiceOffer {
    pub service: Service,
    pub link: Option<String>,
    pub available_since: Option<i64>,
    pub expires_on: Option<i64>,
}
pub offers: Vec<ServiceOffer>,
```

- **MOTN** (`show_to_fetched`): one offer per attributed service, from that service's
  `streamingOptions[country]` entry (`link`, `availableSince`, `expiresOn`). When a
  service has several options (e.g. subscription and addon), take the earliest
  `availableSince` and the latest `expiresOn`.
- **Plex** (`parse_section`): parse `addedAt` → one Plex offer
  `{ link: None, available_since: addedAt, expires_on: None }`.
- **Merge** (`sync::merge`): union offers by service. For a service present in both
  rows: keep the first-seen `link` (current behaviour), `min` of `available_since`,
  and `expires_on` from whichever row has it.
- **Store** (`reconcile_service`): `desired` carries the offer; the upsert writes
  `link`, `available_since`, `expires_on` (all refreshed on conflict).

### 5.2 Cache round-trip (CLAUDE.md MOTN rule)

`CachedTitle` gains `offers: Vec<CachedOffer>` (`service` as string + the three
fields) with `#[serde(default)]`. The legacy `links` field stays deserialisable
(`#[serde(default, skip_serializing)]`): `into_fetched` builds offers from `links`
when `offers` is empty, so the existing cache keeps working with no re-seed (dates
fill in as titles are refreshed by deltas or the next seed). `From<&FetchedTitle>`
writes only `offers`.

The single-service removal from `72fafe0` (`apply_changes`) now removes the matching
offer instead of the link.

### 5.3 Daily MOTN run

1. **Catalogs:** use `app_meta.motn.catalogs` if `catalogs_checked_at` is under
   **7 days** old; otherwise call `/countries` and store the result. A full seed always
   re-resolves.
2. **Seed or delta:** decided as today (`decide_mode`). The delta adds
   `change_type=expiring` to the existing `new` and `removed` passes (`from`/`to`
   omitted → today to +31 days). `parse_changes` treats `expiring` like the other two:
   refresh the show from its embedded detail (which carries `expiresOn`); without
   detail, set that offer's `expires_on` from the change `timestamp` if the show is
   cached (no timestamp → skip). Record `motn.last_mode`, and `motn.last_seed_at` on a
   seed.
3. **Popularity, when due:** if `popularity_fetched_at` is missing or ≥ **3 days**
   old, fetch **3 pages** of
   `/shows/search/filters?country&catalogs&order_by=popularity_1week&series_granularity=show`.
   Rank = position across pages (1–60), first occurrence wins, shows without an
   `imdbId` skipped. Replace `motn_popularity` in one transaction and set
   `popularity_fetched_at`. **Failure is non-fatal**: log a warning, keep the old
   ranks, do not fail the MOTN source.

### 5.4 Request budget (per 30 days)

| Item | Requests |
|---|---|
| Delta: `new` + `removed` + `expiring` (≥ 1 page each, daily) | ~90–120 |
| Popularity: 3 pages every 3 days | ~30 |
| `/countries`: weekly | ~5 |
| **Steady state** | **~125–155** |
| Full re-seed (fresh install / > 25 days without a sync) | +~250–340 |

A re-seed month stays under 500 (D8).

## 6. Ranking rules and API

### 6.1 Catalogue DTO (`TitleListItem`, `GET /api/catalogue`)

New fields, computed server-side from `title_services` and `motn_popularity`:

| Field | Rule |
|---|---|
| `trendRank: number \| null` | `motn_popularity.rank` joined on `imdb_id`, only if `fetched_at` is under **14 days** old. |
| `newSince: number \| null` | `MIN(available_since)` over the title's services (earliest you could watch it); null if none known. |
| `leavingOn: number \| null` | Set only if **every** service row has `expires_on > now`; value is the **latest** of them (last day to watch anywhere). Any Plex row (or any row without an expiry) → null. Past dates are ignored. |

`TitleDetail` (`GET /api/titles/:id`) carries `leavingOn` too, for the detail badge.

### 6.2 Trending (frontend, `visibleTitles`)

1. Titles with `trendRank`, ascending.
2. The rest by `newSince` descending; null `newSince` last; ties by `id` ascending.

### 6.3 For you — `GET /api/for-you` → `{ ids: number[], basis: number }`

1. Load the user's ratings joined to catalogue titles that have an embedding for the
   current `EMBED_MODEL`.
2. Weight each: `r ≥ 7 → r − 6` (1..4); `r ≤ 4 → r − 5` (−1..−4); 5–6 → skipped.
3. `basis` = number of positive weights. If `basis < 3` → `{ ids: [], basis }`.
4. Profile = Σ weight × (vector / ‖vector‖).
5. Candidates = titles with an embedding that are **not watched and not rated**;
   score = cosine(profile, vector); `ids` = candidates by score desc, ties by id.

No external calls (stored vectors only). Reuses `services::similarity`.

Frontend order under For you: `ids` order, then every remaining visible title
(watched, rated, or without an embedding) in Trending order (D6).

### 6.4 Sync status (`GET /api/sync/status`)

Adds:

```json
"motn": {
  "cacheSize": 4812,
  "lastMode": "delta",
  "lastSeedAt": 1757635200,
  "catalogsCheckedAt": 1759017600,
  "popularityFetchedAt": 1759190400
}
```

`null` when MOTN has never run (no `MOTN_API_KEY`).

## 7. Frontend

- **Types:** `Title` gains `trendRank`, `newSince`, `leavingOn` (`isTitle` validates
  them as number-or-null); `TitleDetail` gains `leavingOn`.
- **Sort menu** (`FilterBar`): Trending (default) · For you · Top rated · Newest ·
  A–Z. "Newest" stays release year. When For you is unavailable (`basis < 3`), its
  option is `disabled` and a hint reads "Rate 3+ titles you liked to unlock For you".
- **Store** (`catalogue.ts`):
  - `SortKey` adds `'foryou'`.
  - `forYou: { status: 'idle' | 'loading' | 'ready' | 'error', ids: number[], basis: number }`;
    `loadForYou()` fetches `/api/for-you`. It runs on first selection of For you and on
    load (so the disabled state is known), and is re-run after `setRating` /
    `clearRating` succeed and after the post-sync catalogue reload. Until `ready`,
    the For you sort renders in Trending order.
  - `leavingSoon: boolean` filter + `setLeavingSoon`. Keeps titles with `leavingOn`
    within `LEAVING_WINDOW_DAYS = 30` from now.
- **Filter bar:** a "Leaving soon" toggle alongside the rating filter, with an
  `aria-pressed` state.
- **Poster card:** a "Leaving 14 Oct" badge, bottom-left (the free corner), when
  `leavingOn` is within the window. Text label, not colour-only.
- **Detail view:** the same badge near the service pills.
- **Settings:** a MOTN line on the sync card, e.g. "Cache 4,812 shows · last full seed
  12 Sep · delta · popularity 30 Sep · catalogs 28 Sep". Hidden when `motn` is null.

## 8. Error handling

| Failure | Behaviour |
|---|---|
| Popularity fetch fails | Warn, keep old ranks; ranks older than 14 days stop counting, so Trending degrades to newest-first. MOTN source still `ok`. |
| `expiring` page fails | It is part of the delta, so the MOTN source fails as a `new`/`removed` failure does today (scoped: services untouched). |
| `/countries` fails but cached catalogs exist | Use the cached catalogs and warn. |
| `/api/for-you` fails | Grid stays in Trending order; inline "For you unavailable — retry"; never an empty grid. |
| No embeddings at all | `basis = 0` → option disabled with the same hint. |

## 9. Testing

All fixtures are hand-built JSON (no MOTN key in CI or the cloud environment).

**Rust**
- `show_to_fetched` reads `availableSince`/`expiresOn` per service; earliest/latest
  across multiple options for one service.
- `parse_section` reads Plex `addedAt`.
- `merge` unions offers (min `available_since`, keeps `expires_on`).
- `reconcile_service` writes and refreshes the three columns.
- `CachedTitle`: legacy JSON with only `links` deserialises and yields offers;
  round-trip keeps all offer fields.
- **Delta round-trip:** seed a cached title, run a delta, assert `available_since` /
  `expires_on` reach `title_services` (the CLAUDE.md MOTN failure mode).
- `parse_changes` `expiring`: detail refresh; timestamp fallback; no timestamp → skip.
- Popularity: page parsing, rank order across pages, de-dup, no-imdb skip; due/not-due
  by `popularity_fetched_at`; failure keeps the old table.
- Catalogs reuse within 7 days; re-resolve after.
- DTO: `trendRank` freshness cut-off; `newSince` = min; `leavingOn` rules (all
  services expiring → latest; any Plex → null; past dates ignored).
- `/api/for-you`: weights; `basis < 3` → empty; watched and rated excluded; ties by id.
- `/api/sync/status` `motn` block present / null.

**Frontend**
- Store: Trending tiers + null handling; For you order with sunk remainder; disabled
  state; reload after rating change; leaving filter including the 30-day edge.
- `FilterBar`: For you option disabled + hint; Leaving soon toggle.
- `PosterCard` / `DetailView`: badge shown inside the window, hidden outside.
- `SettingsView`: MOTN line renders; hidden when null.

Live checks against real MOTN/Plex data go on the homelab verification list.

## 10. Out of scope

- `change_type=upcoming` ("coming soon").
- A "new" badge.
- User-configurable windows (30-day leaving window, 3-day popularity refresh are
  constants).
- Explaining For you picks ("because you liked …").
