# Red-team review: Sort Orders Phase 1 implementation plan

**Plan under review:** `docs/superpowers/plans/2026-10-03-sort-orders-phase1.md`
**Spec:** `docs/superpowers/specs/2026-10-02-cue-sort-orders-design.md` (revision 4, Phase 1 = §§1–8)
**Date:** 2026-10-03
**Stance:** hostile staff engineer. The question is why executing this plan fails.
**Code read:** `src/{models,main,lib}.rs`, `src/db/{catalogue,embeddings,app_meta,user_data,sync_runs}.rs`,
`src/sync/{mod,motn,store}.rs`, `src/routes/{mod,sync}.rs`, `src/services/{similarity,embeddings}.rs`,
`migrations/0001,0002,0005,0006`, `frontend/src/{types.ts,api/client.ts,api/sync.ts,stores/catalogue.ts}`,
`frontend/src/components/FilterBar.vue`, `frontend/src/views/SettingsView.vue`, and the existing
frontend tests (`api/__tests__/client.test.ts`, `views/__tests__/DetailView.test.ts`,
`views/SettingsView.spec.ts`, `stores/__tests__/*`).

Not re-raised here: For you's recompute cost and the fingerprint cache (r3 Objection 3, which the spec
now addresses), and the shape of the For you formula (r3 Objection 2).

---

## Objection 1: Task 6 breaks every detail page. `isTitle` is shared with `isTitleDetail`, and `/api/titles/{id}` never sends `newSince` *(category b: false assumption about the codebase; also a)*

**What the plan does.** Task 1 adds `new_since` to `TitleListItem` only (`src/models.rs:111-128`,
populated in `fetch_catalogue`, `src/db/catalogue.rs:56-69`). Task 6 adds
`(r.newSince === null || typeof r.newSince === 'number')` to `isTitle`, and adds `newSince` to the TS
`TitleListItem`.

**What the code does.**
- `isTitleDetail` begins with `if (!isTitle(v)) return false` (`frontend/src/api/client.ts:22-23`).
- `GET /api/titles/{id}` serialises `TitleDto` (`src/models.rs:87-108`, built at
  `src/db/catalogue.rs:153-169`). No task touches `TitleDto` or `fetch_title`, so the detail JSON has
  **no `newSince` key**. `r.newSince` is `undefined`, which is neither `null` nor a number.
- `getTitle` therefore throws `'title detail response is invalid'` for every title, and
  `DetailView.vue:29` shows its error state. The detail page breaks completely, including watch links
  and rating.

**Why the gates pass.**
- `TitleDetail extends TitleListItem` (`frontend/src/types.ts:22`). Once `newSince` is required on the
  list type, `vue-tsc` forces every `TitleDetail` fixture to carry it. Task 6 Step 3 tells the
  implementer to "add `newSince: null`" to those fixtures. That makes
  `client.test.ts` "getTitle returns the full detail" pass on a body the real backend never sends.
- `DetailView.test.ts:34` and `DetailWatched.test.ts:36` stub `client.getTitle`, so the validator is
  never exercised in the view tests.
- No test runs the Rust DTO against the TS validator, so nothing in either suite reaches this.

**A sign the plan mixed up the two DTOs.** Task 1 Step 3 says to "update its literal in `models.rs`
tests". The only struct literal there is a `TitleDto` (`src/models.rs:136`). No `TitleListItem`
literal exists.

**Fix (pick one, and test it).**
- Give `TitleDto` `newSince` too, computed the same way.
- Or give `TitleDetail` its own shape with no `newSince`, and validate list items and details
  separately.

Either way, add a `getTitle` test whose body is copied from a real `serde_json::to_value(TitleDto)`
and has no `newSince`. Spec §6 ("`Title` gains `newSince`") is where this ambiguity comes from. The
plan copied it without checking how `Title` and `TitleDetail` relate.

---

## Objection 2: The `motn: null` rule hides MOTN state in the case Settings exists for: a first seed that fails, then backs off *(category a: does not deliver §5.4 / §6 / D8; also b)*

**Spec.** §5.4 says `motn` is "`null` when MOTN is not configured". §6 says Settings shows the request
count, plus "seed paused until …" during back-off. D8 says Settings shows MOTN cache and request
state.

**Plan.** Task 5 returns `None` "when the cache is empty **and** `LAST_MODE` is unset (MOTN never
ran)". Task 4 sets `LAST_MODE` only on success ("a successful seed sets … `LAST_MODE = "seed"`; a
successful delta sets `LAST_MODE = "delta"`"). On a seed error it writes only `SEED_FAILED_AT`.

**Trace a fresh install, or any install whose first seed fails** (bad or expired key, 401, 429 from a
spent plan, or a parse error on page N):
1. `decide_mode` returns `Seed` (empty cache, `src/sync/motn.rs:391-399`). Pages are requested and
   counted, then the seed fails. `replace_all` (`motn.rs:582`) never runs, so the cache stays empty.
   `LAST_MODE` stays unset and `SEED_FAILED_AT = now`.
2. `/api/sync/status` returns `"motn": null`, so Settings hides the MOTN line. The user does not see
   the request count already spent, does not see "seed paused until …", and has no hint why nothing
   arrives.
3. For 3 days every "Sync now" fails without a network call. The only visible trace is the Sources
   card, which shows `disney — error (0)`. `SettingsView.vue:30` renders `s.status`, not the error
   text, and `SourceRun` has no error field. So the "MOTN seed back-off until 1759…" message (raw unix
   time in the plan's `bail!`) appears nowhere in the UI.

The rule also fails in the opposite direction. Remove `MOTN_API_KEY` after MOTN has run, and the
cache and `LAST_MODE` stay in the DB. Settings then shows a stale MOTN block, possibly with "seed
paused", forever.

**The right signal already exists.** `status()` already receives `runner: web::Data<Arc<SyncRunner>>`
(`src/routes/sync.rs:60-63`). The runner holds the configured sources, and `main.rs:81-88` adds MOTN
only when the key is set. Add `SyncRunner::has_source("motn")` and return `None` exactly when MOTN is
not configured, which is what the spec says. Add a Task 5 test: "seed failed, cache empty →
`motn.seedFailedAt` set and `requestsThisMonth > 0`, not null". Separately, `seed_failed_at` should
show in Settings even when `lastMode` is null.

---

## Objection 3: Catalog reuse covers the wrong failure. An `/countries` body that parses to nothing still records MOTN "ok" with zero rows, which wipes Disney+/Crunchyroll and prunes about 4.8k titles. Task 4 rewrites this branch and leaves it armed *(category c: the 6-month failure)*

**What the code does today.**
- `resolve_services` returns an empty `Vec` when the JSON does not parse (`src/sync/motn.rs:26-31`, it
  only warns), and also when the country entry is missing.
- `resolve_catalogs` turns that into `Ok(None)` (`motn.rs:472-479`).
- `fetch` turns `Ok(None)` into `return Ok(Vec::new())` (`motn.rs:574-576`), which counts as a
  **successful** fetch.
- `run_sync` then puts Disney and Crunchyroll in `ok_services` (`src/sync/mod.rs:122-127`) and calls
  `reconcile_service(svc, &[])` (`mod.rs:161`). That deletes every membership for those services
  (`src/sync/store.rs:184-191`). `prune_orphans_scoped` (`mod.rs:176-184`) then deletes every
  Disney/Crunchyroll-only title, and `ON DELETE CASCADE` deletes their embeddings.
- MOTN records `ok` with count 0, so `last_ok_unix` moves forward. The next day's delta rebuilds from
  the cache, which is intact, and re-inserts all ~4.8k titles under **new ids**. That breaks any
  `/title/:id` URL and re-embeds everything through OpenAI.

**This failure mode has happened before.** The comment on `CountryEntry` (`motn.rs:44-48`) records a
`/countries` schema mismatch that "silently fails the whole parse and resolves to no services". MOTN
schema drift is the most likely 6-month failure for this client.

**What the plan adds.** Task 4 adds a cached-catalogs fallback "on `/countries` failure", and the test
`cached_catalogs_used_when_countries_fails` uses HTTP 500. A 200 response with a drifted body is not
treated as a failure, so the new resilience path skips the one failure already seen in practice. The
plan also never says whether a `None` resolution is written to `motn.catalogs`. If the implementer
writes `""` with a fresh `catalogs_checked_at`, every delta for 7 days reuses "no services". The key
is also not scoped to `country`, so after a `REGION` change the old country's catalogs are reused for
up to 7 days.

**Fix, inside Task 4's own scope.**
- When the cache is non-empty, treat an empty resolution as an error: use the cached `motn.catalogs`
  and warn, or fail the source so its services stay protected. Never return `Ok(vec![])` with a full
  cache.
- Never cache an empty resolution.
- Store the country next to `motn.catalogs`, or include it in the key.
- Add tests "`/countries` 200 with unparseable body, cache present → cached catalogs used" and
  "… no cache → source error, not ok-with-zero".

---

## Runners-up (real, smaller)

1. **For you's failure states reduce to "basis 0", and the spec §7 retry can't be reached.** If the
   first `/api/for-you` fails, `basis` stays 0. `forYouAvailable` is then false and the option is
   disabled with "Rate 3+ titles you liked…", even for a user with 500 ratings. The
   "For you unavailable — retry" line only shows when `sort === 'foryou'`, which a disabled option
   can't reach. The same misleading hint shows permanently when `OPENAI_API_KEY` is unset, a supported
   config (`main.rs:60-64`). Use a separate `error` and `no embeddings` state in FilterBar, and show
   retry whenever the status is `error`.
2. **`fake_motn(handler: fn(&str) …)` gets only the path.** All seed pages hit the same
   `/shows/search/filters` path, distinguished only by the `cursor` query (`motn.rs:501-507`). A
   stateless fn pointer given the path alone can't do "page 1 ok, page 2 → 429" or "2-page seed".
   Pass `path_and_query`, and count hits per path so "never hits `/countries`" is a direct assertion.
3. **Task 2 rewrites `similarity::dot` in place.** That changes `cosine` and `rank_by_cosine` for the
   ask engine and the lightness axis. Spec §5.3 and §8 describe a separate chunked dot "equal to
   `similarity::dot` within tolerance". Keep `dot` as it is and add `dot_chunked`. Otherwise the Ask
   ranking changes and the spec's equality test has nothing to compare against.
4. **The watched fingerprint is `COUNT(*) FROM watch_history`.** Watching A and un-watching B
   (`user_data.rs:66-86`) leaves the count unchanged. The result cache then hits, and B stays out of
   `ids` (sunk) until something else changes. Add `MAX(id)` to the fingerprint.
5. **Task 9 test path.** Settings tests live at `frontend/src/views/SettingsView.spec.ts`, not
   `views/__tests__/`. "Create if absent" will create a second Settings suite. Its `status` fixture
   also needs `motn`.
6. **The spec's "warning includes the month's count" (§4) only reaches a log line.** The `sync_runs`
   error string is `format!("{e:#}")` = "MOTN rate limit (429)" and carries no count.
7. **The "first ready" flag in Task 7 is unspecified.** If `visibleTitles` gates the rank map on
   `status === 'ready'`, a failed reload (`status = 'error'`, old `ids` kept) jumps back to Trending.
   Add an explicit `hasLoaded` flag.
8. **`basis` before or after the 300 cap is unstated** (spec §5.3 and Task 2 both leave it open). It
   only matters for display, but it should be pinned by a test.
9. **Ordering in the 429 test.** "429 aborts and skips the rest" (§8) is not asserted. Add
   `hits == 2` after a page-2 429 on a 3-page fake.
