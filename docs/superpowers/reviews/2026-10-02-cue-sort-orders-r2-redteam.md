# Red-team review, round 2: Sort Orders (Trending, For You, Leaving Soon)

**Spec under review:** `docs/superpowers/specs/2026-10-02-cue-sort-orders-design.md` (revision 2, 2026-10-03)
**Prior review:** `docs/superpowers/reviews/2026-10-02-cue-sort-orders-redteam.md`
**Date:** 2026-10-03
**Stance:** hostile staff engineer. The question is whether this is the wrong thing to build.
**Code read:** `src/sync/{mod,motn,store}.rs`, `src/db/{sync_runs,app_meta,catalogue}.rs`,
`src/routes/sync.rs`, `src/main.rs`, `src/services/similarity.rs`, `migrations/0001_init.sql`,
`frontend/src/stores/catalogue.ts`, `frontend/src/components/FilterBar.vue`, and the
incremental-sync spec.

## What revision 2 fixed (not re-raised here)

- Relevance as its own sort key, so answer sets keep engine order (prior Objection 1, now D1 / §6.2).
- The `ServiceOffer` refactor is gone. Leaving soon is a replaced snapshot side table (prior Objection 2, now §4 / §5.5).
- `expiring` is non-fatal. There is a request counter, a soft cap, 429 handling, seed back-off and a 12 h
  manual-sync gate (prior Objection 3, now D8 / §5.1–5.6).
- Prior runners-up (centroid flattening, the Settings card scope, For you inside an answer) are not repeated.

The three objections below are new, or they show where a revision-2 fix is itself wrong.

---

## Objection 1: the new `newSince` tier has no expiry, and its one-shot global baseline misfires whenever a source joins late. Within months Trending is mostly "everything added since launch, by insert order" *(category c: 6-month collision; the revision's fix is wrong)*

**What revision 2 did.** The prior review showed that `available_since` would mostly be NULL and proposed
dropping tier 2. Revision 2 kept the tier and re-sourced it:
- D3 / §6.1: `newSince = strftime('%s', titles.added_at)` if `catalogue.baseline_at` is set and `added_at` is later.
- §4: the baseline is a single global key. The migration sets it if *any* `sync_runs` row is ok. On a fresh
  install, `run_sync` sets it after the first successful sync, only if it is still unset.

**Problem A: nothing ages out of "new".** §6.1 puts a 14-day freshness limit on `trendRank` but no window on
`newSince`. Once a title is added after the baseline, it is new for good. §6.2 ranks tier 2 (all `newSince`,
descending) above tier 3 (everything else by rating).

Use the spec's own budget to size it. §5.6 puts the core delta at 60–90 requests a month at one sync a day.
That is the 60-request floor (≥ 1 page each for `new` and `removed`) plus up to ~30 extra pages at 25 changes
a page, so up to ~750 changes a month. Crunchyroll UK adds titles constantly. Even if only a few hundred net
additions a month reach `titles`, tier 2 holds about 1–2k titles after six months, against a ~4.8k catalogue.
Trending then shows:
1. the top 60;
2. a growing block of everything added since the feature shipped, newest first;
3. the original catalogue by rating, pushed further down every week.

"Newest first" is not a ranking. A title added five months ago still beats an 8.9-rated film from the
baseline catalogue. That is the §1 complaint ("sorts by nothing") back again, with the date as the key.

**Problem B: one global baseline is wrong when sources join at different times.** `titles.added_at` is the DB
default `datetime('now')` (`migrations/0001_init.sql:12`). It is stamped on every first insert
(`src/sync/store.rs:137`), whatever the cause of the insert. The baseline is global, but sources are optional
and independent: `src/main.rs` adds Plex and MOTN only if their keys are set, and `run_sync` records each
source separately (`src/sync/mod.rs:160-165`). So the first-import exemption covers only sources that
succeeded before the baseline was set. Any bulk import after that becomes "new":
- **MOTN key added later.** The user starts Plex-only (a supported setup) and Plex's first ok run sets the
  baseline. Later they add `MOTN_API_KEY`, and the first seed inserts ~4.8k titles, all of them "new".
- **First MOTN seed fails. Revision 2 makes this case worse.** The startup sync (`src/main.rs:91-97`) runs
  Plex, then MOTN. If Plex succeeds and the seed fails (one 5xx at page 200 is enough, since `seed_pages`
  at `src/sync/motn.rs:493` is all-or-nothing), §4 sets the baseline because the sync was "successful". The
  new §5.2 back-off then holds the seed off for 3 days. When it lands, the whole MOTN catalogue is "new".
  §4 never says what "a successful sync" means when some sources fail.
- **A new service is added to `WANTED`** (`src/sync/motn.rs:17`). Its whole catalogue becomes "new". This
  is the obvious next feature for a two-service app.

In each case all the new titles share an `added_at` to within seconds of one run. So §6.2's tiebreak
("ties by `id` ascending") orders tier 2 by insert order, which is exactly the `ORDER BY id`
(`src/db/catalogue.rs:14`) that §1 calls the bug. The user's own Plex additions also count as "new" and
outrank everything rated. The prior review already noted that a discovery sort should not promote titles
the user put there themselves. Revision 2 changed the data source but kept that effect.

**What to do instead.** Either drop tier 2, as the prior review proposed (the top 60 then rating is a
complete, honest order), or make it bounded and per-source:
- **Window:** `added_at` within the last 14–30 days, matching the `trendRank` freshness rule.
- **Per-source exemption:** don't count a title as new if a seed or a source's first ok run inserted it.
  Key on `motn.last_mode = seed` for that run, or keep `baseline_at.<source>` per source.

Add a test that pins the late-joining-source case. §9's only baseline tests cover the migration and the
first sync.

---

## Objection 2: Leaving soon assumes MOTN has dated expiry data for Disney+ and Crunchyroll in the UK. Nobody has checked, and checking costs one request *(category b: assumed requirement)*

**What the spec relies on.** §3 cites only the OpenAPI schema, and that schema says `timestamp` is optional
("omitted when the exact date is unknown"). §5.5 then drops every change with no timestamp. D4 drops every
title that is on Plex or on a second service with no expiry. §9 says "hand-built JSON fixtures only (no MOTN
key in CI or the cloud environment)", and live checks are deferred to the homelab list. So the whole feature
is designed and tested against data no one has looked at:
- the `motn_expiring` table and migration;
- the 8-page cap with a truncation warning (§5.5);
- the `leavingOn` join on both DTOs (§6.1);
- the filter toggle, the card badge and the detail badge (§7);
- four test groups (§9);
- 10–80 requests a month in the budget (§5.6).

**Why the doubt is reasonable, not pedantic.** Both services in `WANTED` are the ones least likely to have
this data:
- **Disney+** is mostly a first-party library. It does not run a "leaving soon" row the way Netflix does.
- **Crunchyroll** licences do lapse, but usually at the season level. `item_type=show` in §5.5 filters season
  changes out.

If MOTN mostly returns undated or season-level changes for `gb` / `disney,crunchyroll`, then after the §5.5
filters and the D4 every-service rule the table holds a handful of rows, or none. The "Leaving soon" toggle
then shows an empty grid, and §7 / §8 define no empty state for that filter. The 8-page cap and "~10–80
requests" assume a volume nobody has observed.

The same blind spot applies to `popularity_1week` (§3). The spec does not say what drives MOTN's
popularity number, or whether a weekly top 60 over a ~4.8k Disney+/Crunchyroll catalogue moves enough
between 3-day refreshes to be worth refreshing.

**Cost to establish it: 2–4 requests, against a 500 budget.** Make one `/changes?country=gb&catalogs=disney,crunchyroll&change_type=expiring&item_type=show`
call with no `from`, and one popularity page. Count the changes, the share that have a `timestamp`, and the
split by service. Save the responses as the §9 fixtures, so the tests pin real shapes rather than imagined
ones. Do this before the plan is written. If the dated show-level expiries are only a handful, cut Leaving
soon (or switch to `item_type=season` with a different badge rule), and the spec loses a table, a pass, two
DTO fields and a quarter of its budget line.

---

## Objection 3: the budget machinery added in revision 2 is now the biggest and riskiest part of the spec, and it exists to pay for the optional passes. Several of its fixes are leaky. The 80% version spends no MOTN budget on sort orders *(category a: simpler design; the revision's fix is wrong)*

**Where the weight moved.** Revision 2 dropped the `ServiceOffer` refactor but added a rate-limiting
subsystem inside `MotnClient::fetch` (`src/sync/motn.rs:573`), the part of the codebase CLAUDE.md warns about
most:
- a counting wrapper around every HTTP call;
- two pre-network gates;
- a `decide_mode` rewrite (`src/sync/motn.rs:391`) with an upgrade fallback;
- a 7-day `/countries` cache with a failure fallback;
- seed back-off;
- nine-plus `app_meta` keys (§4).

Its only job, beyond the 429 abort that any client should have, is to make room for popularity plus
expiring: 40–110 requests a month, which is 20–55% of the spec's own steady state (§5.6). The guard is the
cost of the feature, not a separate improvement.

**The fixes leak in four ways.**

1. **Skipped runs record `ok`, so sync state now has two sources of truth.**
   - §5.2 returns `motn_cache::load_all` without a network call, and `run_sync` records `ok` with
     `item_count = cache size` for `disney` and `crunchyroll` (`src/sync/mod.rs:160-162`). §5.3 admits this
     ("because a skipped run still records ok") and moves the real state into `motn.last_fetch_at`.
   - Everything that reads `sync_runs` now describes runs that never happened: `latest_per_source`
     (`src/db/sync_runs.rs:53`), `overall()` (`src/routes/sync.rs:30`), `last_ok_unix` and `motn_recent_ok`.
   - A MOTN source in seed back-off for 3 days shows green "ok" in Settings every day, with the old count.
   - Revision 2 patches this by adding a §6.4 block that reports the truth next to the `sources` list,
     which still says ok.
   - The simple fix is a `skipped` status in `sync_runs`, which needs `overall()` to treat it as ok. A
     parallel key store is not needed.

2. **The soft cap is not a reserve.** §5.1 counts by UTC calendar month and admits that "the MOTN plan's reset
   day is not known". If the real window resets mid-month, the counter can read 50 while MOTN has 450
   counted. The optional passes then run and push the account into 429. §5.6's "a re-seed always has room in
   practice" depends on a counter alignment the spec says it does not have.

3. **Seed back-off limits how often a seed is retried, not how much a failed seed wastes.** `seed_pages`
   still collects every page in memory and writes only at the end (`src/sync/motn.rs:493`, `:582`). §10
   leaves resumable seeds out of scope ("back-off covers the retry loop"). A transient failure at page ~200,
   then one retry 3 days later, costs about 200 + 340 = 540 requests. That is over 500 even with no other
   traffic. The retry hits 429 near the end, backs off again, and MOTN stays dark until the window resets.
   §5.6's "+~250–340" assumes a seed succeeds on the first attempt.

4. **The 12 h gate hides MOTN from the "Sync now" button.** In a single-user app, the usual reason to press
   "Sync now" is to check that MOTN is working after changing a key, region or plan. Under §5.2 that press
   skips MOTN silently and records `ok` (point 1). The gate guards against a cost the counter already
   measures.

**80% version.** Split by MOTN cost, not by screen:
- **Ship now (zero MOTN requests):** Relevance (D1), a server-side default browse order (`ORDER BY` external
  rating in `fetch_catalogue`, which fixes §1 for every client of `/api/catalogue`), For you (§6.3, stored
  vectors only) and D7. The MOTN changes are just the 429 abort and the request counter, shown read-only in
  Settings, as instruments.
- **After a month of real counts:** add popularity alone. Weekly is enough for a "this week" list: 3 pages,
  ~13 requests a month. Gate it on the measured counter. Add expiring only if Objection 2's spike shows data
  worth it.
- **Drop:** the 12 h gate (or limit it to scheduled runs), the calendar-month soft cap as a "reserve", and
  the `decide_mode` rewrite. That rewrite is only needed because skipped runs write `ok`, so fix point 1
  instead.

This version delivers what the user mainly asked for: a real browse order, a personal ranking, and answer
sets that are no longer re-sorted. It does not rewrite the sync core around a budget the spec has only
estimated (§5.6 is arithmetic, and the incremental-sync spec's "confirm request count drop" is still on the
homelab list).

---

## Runners-up

- **`loadForYou` on every rating change (§7) re-reads every embedding blob.** The endpoint loads all vectors
  for the current `EMBED_MODEL` on each call (~5k × 1536 f32, about 30 MB from SQLite) to answer a question
  whose inputs change by one rating. Rating several titles quickly from the grid triggers back-to-back full
  scans. Debounce it, or cache the normalised vectors in process and invalidate them on sync.
- **For you's `basis` counts only ratings that land on catalogue titles with embeddings (§6.3 step 1).** Most
  IMDb-imported ratings are for titles outside a Disney+/Crunchyroll/Plex catalogue, and the ones inside it
  are mostly the user's own Plex library. So the profile is close to "titles like my Plex library", and the
  catalogue's Plex titles are excluded as candidates because they are watched or rated. That may be fine,
  but the spec should say so instead of implying "your taste".
- **`applyResult` re-selects Relevance on every new answer (§7).** "Saves the browse sort (if not already in
  an answer) and sets `'relevance'`". If the user picked Top rated inside an answer and then refines,
  their explicit choice is overwritten. Keep the user's choice for later steps of the same thread.
