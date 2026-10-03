# Red-team review: Sort Orders (Trending, For You, Leaving Soon)

**Spec under review:** `docs/superpowers/specs/2026-10-02-cue-sort-orders-design.md`
**Date:** 2026-10-02
**Stance:** hostile staff engineer. The question is whether this is the wrong thing to build, not whether the spec is tidy.
**Code read:** `src/sync/{motn,merge,store,mod}.rs`, `src/db/{motn_cache,catalogue,sync_runs}.rs`,
`src/services/{similarity,embeddings,ask_engine}.rs`, `src/routes/sync.rs`, `src/config.rs`,
`frontend/src/stores/catalogue.ts`, `frontend/src/components/FilterBar.vue`,
`frontend/src/views/BrowseView.vue`, plus the incremental-sync and IMDb-import specs.

---

## Objection 1: "Trending sorts by nothing" is load-bearing. The default sort is the Ask relevance order *(category b: assumed requirement)*

**The claim in the spec.** §1 calls the `'trending'` no-op a bug ("keeps the base order"). §6.2 replaces it with a
real reordering (trendRank, then newSince) and leaves it as the default.

**What the code actually does.** `visibleTitles` (`frontend/src/stores/catalogue.ts:88-113`) builds `base`
from `state.resultIds` whenever `answerActive` is true (line 89-91). `case 'trending': /* keep base order */`
(line 110) is the only thing that keeps those ids in the order the backend ranked them. Every answer mode
goes through this path, because `applyResult` (line 180-186) writes `resultIds` straight from:
- **Ask answers**: Claude's or the retrieval ranking.
- **"More like X"**: `AskEngine::similar` (`src/services/ask_engine.rs:106-135`) returns the top 20 by cosine
  similarity, already sorted.
- **Refine chips**: `refine("shorter")` returns "Shortest first.", and `lighter` projects onto the lightness
  axis. In both cases the order *is* the result.

`FilterBar` and the sort `<select>` stay mounted in answer mode (`BrowseView.vue`: `<FilterBar />` sits
outside the `answerActive` branch). The user never changes the sort from its default, so under §6.2 every
Ask answer, every "More like X", and "Shortest first" would be re-sorted: the 60 MOTN-popular titles first,
then newest-on-service. In practice that means "Shortest first" stops showing the shortest titles first,
and "More like Coco" shows whatever is trending before the closest match. The spec never mentions
`answerActive`. §9's frontend tests ("Store: Trending tiers + null handling") have no case for it, and no
existing test pins the current behaviour either (`grep trending frontend/src/**/__tests__` finds none). So
this would ship green.

**Why this is "wrong thing", not "bug".** The spec misreads what the problem is. Two different needs got
merged into one sort key:
1. a sensible *browse* order for the idle grid, which really is broken: `ORDER BY id` in `src/db/catalogue.rs:14`;
2. *relevance* order for answer sets, which works today *because* the default sort is a no-op.

Nobody established that answer sets should be re-ranked by global popularity. The spec assumed it by
reusing the default key.

**Cheaper shape.** Make the default key "Relevance / Default". In answer mode it keeps the base order. In
browse mode it uses whatever browse ranking you settle on. That ranking can live server-side as the
`ORDER BY` in `fetch_catalogue`, which also fixes §1's complaint for every consumer of `/api/catalogue`
and costs one SQL change. "Trending" then becomes an explicit opt-in sort, and nothing silently re-ranks
Ask.

---

## Objection 2: the ServiceOffer refactor is the expensive half of the spec and delivers the least. Side tables give most of the value *(category a: simpler design)*

**Cost of §5.1–5.2.** Swapping `links: Vec<(Service, String)>` for `offers: Vec<ServiceOffer>` touches
every layer of the sync pipeline:
- `FetchedTitle` (`src/sync/mod.rs:42-62`) and `show_to_fetched` (`src/sync/motn.rs:179-241`);
- Plex `parse_section` (`src/sync/plex.rs:83`), which does not read `addedAt` today;
- `MergedTitle` and the link-union in `merge` (`src/sync/merge.rs:43-60`, `132-136`);
- the `(id, services, links)` tuple and per-service `desired` build in `run_sync` (`src/sync/mod.rs:144-165`);
- the `reconcile_service` signature and upsert (`src/sync/store.rs:170-207`);
- `CachedTitle` with a *dual-format* legacy-`links` / new-`offers` deserialiser (`src/db/motn_cache.rs:28-32`,
  `39-110`);
- `apply_changes` single-service removal (`src/sync/motn.rs:413-424`);
- every hand-built `FetchedTitle` / `MergedTitle` / `CachedTitle` literal in tests (about 10 sites).

This is exactly the surface CLAUDE.md warns about. The MOTN cache round-trip has already silently dropped
`*_url`, `score`, and `links`. The spec adds a fourth field family to it, plus a back-compat shim.

**What it buys: tier 2 of Trending, which will mostly be NULL.** §5.2 says "no re-seed … dates fill in as
titles are refreshed by deltas or the next seed". But there is no "next seed" in steady state:
- `decide_mode` (`src/sync/motn.rs:391-399`) seeds only on an empty cache or no OK run in 25 days.
- The incremental-sync spec §4 says outright that "no periodic full re-fetch is needed".
- Deltas refresh only shows that had a `new`, `removed`, or `expiring` change.

So the large majority of the ~4.8k cached MOTN titles keep `available_since = NULL` indefinitely. Plex,
being a full fetch each run, gets real `addedAt` for every item on the first sync. Following §6.2 literally,
Trending becomes:
1. 60 popular titles;
2. **the user's own Plex library by date-added**. That is a list the user built, so it is the least
   "discovery" signal available;
3. a few recently-changed MOTN titles;
4. thousands of MOTN titles with null `newSince`, ordered by `id` ascending. That is §1's bug, kept and
   moved below the Plex block.

Even where it is populated, `availableSince` is defined in §3 as "when the option was **first detected**".
For back catalogue that is MOTN's crawl date, which bunches large parts of the catalogue onto the same day.
It does not mean "new on your services".

**Leaving soon doesn't need the refactor either.** §3 already states that `/changes?change_type=expiring`
with `from` omitted returns *the whole* today-to-+31-day window. That is a **snapshot**, not a delta. Fetch
it once per run and replace a side table `motn_expiring(show_id, imdb_id, service, expires_on)` in one
transaction. This is the same pattern §5.3 already uses for `motn_popularity`. `leavingOn` then becomes a
join in `fetch_catalogue` / `fetch_title`, with the "every service, Plex never leaves" rule done in SQL
against `title_services`. Running `expiring` through `parse_changes` as cache "additions" (§5.3 step 2) is
worse in two ways:
- it mixes a state snapshot into a delta pipeline;
- an expiry that gets extended or cancelled never receives a correcting change, so a stale `expires_on`
  stays in the cache JSON.

Note also that `changes_pages` always sends `from` (`src/sync/motn.rs:541-547`), so reusing it for
`expiring`, which §5.3 says must omit `from`, needs its own code path anyway.

**80% version.**
- `motn_popularity` side table (as specced).
- `motn_expiring` side table (replaced per run).
- Trending = trendRank, then the existing external-rating order. Drop `newSince`.
- For you as specced.
- No change to `FetchedTitle`, `merge`, `store`, the cache payload, or Plex.

This keeps roughly the whole user-visible feature set except a tier-2 order that, per above, mostly won't
have data behind it.

---

## Objection 3: D8's budget is an unenforced assertion, and the spec makes a cosmetic badge able to trigger the most expensive operation *(category c: the 6-month collision)*

**The arithmetic has no slack.** §5.4: steady state ~125–155, plus a re-seed of ~250–340, gives
**up to ~495 of 500**. That leaves about 5 requests of headroom in a re-seed month. The table also assumes
exactly one run per day. That assumption is not enforced:
- `SYNC_CRON` is user-configurable (`src/config.rs:17,36`);
- `POST /api/sync` (`src/routes/sync.rs:51-57`) runs the full pipeline on demand from Settings;
- every run costs at least 3 `/changes` requests after this spec, up from 2;
- nothing in the codebase counts MOTN requests or handles 429 (`grep -ri 'quota\|429\|rate.limit' src` is
  empty).

Two "Sync now" clicks a day, or a 12-hourly cron, push steady state to roughly 250–300 per month. At that
point a re-seed month cannot fit. The catalogue also grows over six months, so seed page count only goes up.

**The re-seed failure mode is a lock-in loop.**
- `seed_pages` (`src/sync/motn.rs:493-520`) is all-or-nothing. Pages pile up in memory and `replace_all`
  runs only after the last one (line 580-582).
- If the quota runs out at page 200, the run errors and `sync_runs` records `error` for disney/crunchyroll.
- `decide_mode` still says Seed the next day (`motn_recent_ok` is false, `src/db/sync_runs.rs:157-169`),
  so the retry starts again from page 1.
- Each attempt burns hundreds of requests and makes no progress until the monthly reset. Meanwhile MOTN
  data goes stale and every daily sync logs an error.

**This spec makes entering that loop more likely.** §8 row 2: "`expiring` page fails → … the MOTN source
fails as a `new`/`removed` failure does." So a nice-to-have badge is now on the critical path of the
delta. Delta failures are exactly what `decide_mode` counts toward the 25-day re-seed trigger. Consider a
malformed or changed `expiring` response, or the `from` mistake above (passing a past `from` where §3 says
the window starts today; whether MOTN rejects that is untested, since §9 uses only hand-built fixtures and
"no MOTN key in CI"). A persistent failure there means no OK runs for 25 days. That forces a full re-seed,
and the re-seed is the operation the budget can barely afford.

**What the spec should say instead.**
- Treat `expiring` like popularity: **non-fatal**, keep the old snapshot, warn.
- Persist a per-month MOTN request counter in `app_meta`, and have `fetch` refuse non-essential calls
  (popularity, `expiring`, `/countries`) above a soft cap so the re-seed reserve is real rather than
  arithmetic.
- Make the seed resumable (persist cursor plus partial entries), or at least back off for N days after a
  failed seed instead of retrying from page 1 daily.
- Spell out that D8 assumes one run a day, and either rate-limit manual triggers for MOTN or skip MOTN
  on a manual run that comes within X hours of the last OK one.

---

## Runners-up (real, but weaker than the three above)

- **For you's single weighted centroid will flatten taste for this user.** The IMDb import (I1 in
  `2026-06-23-cue-imdb-ratings-import-design.md`) brings in *everything*, "a few thousand rows". So `basis`
  will be dozens to hundreds of positives, not 3. The mean of many diverse `text-embedding-3-small`
  vectors (built from `"{title} ({year}) — {kind}. Genres: … {description}"`, `src/services/embeddings.rs:25-35`)
  drifts toward the shared component of the embedding space and ranks "generic" titles highest. A
  per-candidate max (or top-k mean) similarity against liked titles costs the same, reuses the loop in
  `AskEngine::similar`, and keeps separate tastes (anime vs Pixar) apart. The D5 gating (disabled option,
  hint, `loadForYou` on every load) is UI built for a cold-start user this install will not be.
- **D7 (Settings MOTN cache line, §6.4, §7) is unrelated to sort orders.** It adds two `app_meta` keys,
  a DTO block, and a Settings test to a spec that is already wide. Split it out.
- **§6.3's frontend contract repeats the Objection 1 mistake for For you.** "ids order, then every
  remaining visible title in Trending order" also re-ranks answer sets when For you is selected. That is
  defensible as an explicit choice, but it should be stated.
