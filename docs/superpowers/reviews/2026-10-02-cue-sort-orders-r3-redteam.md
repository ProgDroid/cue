# Red-team review, round 3: Sort Orders (Trending, For You, Leaving Soon)

**Spec under review:** `docs/superpowers/specs/2026-10-02-cue-sort-orders-design.md` (revision 3, 2026-10-03)
**Prior reviews:** `docs/superpowers/reviews/2026-10-02-cue-sort-orders-redteam.md` (r1),
`docs/superpowers/reviews/2026-10-02-cue-sort-orders-r2-redteam.md` (r2)
**Date:** 2026-10-03
**Stance:** hostile staff engineer. The question is whether this is the wrong thing to build.
**Code read:** `src/sync/{mod,motn,store}.rs`, `src/db/{sync_runs,app_meta,catalogue,embeddings,user_data}.rs`,
`src/services/{similarity,embeddings,ask_engine}.rs`, `src/routes/sync.rs`, `src/main.rs`,
`migrations/0001_init.sql`, `frontend/src/stores/catalogue.ts`, `frontend/src/views/SettingsView.vue`,
`frontend/src/components/FilterBar.vue`, `Cargo.toml`.

## Settled in revision 3 (not re-raised)

- Phasing. Phase 1 makes no new MOTN calls. Popularity and Leaving soon are gated on evidence, including the
  one live `expiring` request (r2 Objections 2 and 3).
- `newSince` now has a 30-day window, and the baseline moves after each full seed (r2 Objection 1, problem A
  and the MOTN-late part of problem B).
- The 12 h gate is gone. Seed back-off returns an error, not the cache, so `sync_runs` stays truthful (r2
  Objection 3, points 1 and 4).
- Top-k For you replaces the centroid (r1 runner-up). The Settings MOTN line stays. Both were the user's
  choices. I attack only how they are specified.

Each objection below is new, or shows that a revision-3 fix is itself wrong.

---

## Objection 1: the baseline fix fails on the main path. "Seed finish time" comes before the titles are stamped, so a fresh MOTN install marks its whole catalogue "new". A stateless rule would do the job with no baseline at all *(category a: simpler design; the revision's fix is wrong)*

**What revision 3 specifies.**
- D3 / §3: `newSince` needs `added_at` to be after `catalogue.baseline_at`.
- "after any successful sync, set it if unset; after a successful MOTN full seed, set it to **the seed's
  finish time**". The writer is given only as "in `run_sync` / `MotnClient`".

**Where the time comes from in the code.**
- `titles.added_at` is the column default `datetime('now')` (`migrations/0001_init.sql:12`). It is stamped
  by the `INSERT` in `store::upsert_title` (`src/sync/store.rs`, the `else` branch).
- That upsert runs in `run_sync` *after every source has returned from `fetch`* (`src/sync/mod.rs:121-145`).
- The seed finishes inside `MotnClient::fetch`, at `seed_pages` → `motn_cache::replace_all`
  (`src/sync/motn.rs:573-583`). MOTN is the last source (`src/main.rs`: Plex is pushed first). Its
  `load_all`, the merge, and about 5k upserts in separate transactions all happen after that point.

So whichever component writes it, a baseline equal to "the seed's finish time" is earlier than every
`added_at` that run stamps. Every title the seed inserted counts as "new".

**The fresh-install case is the worst one.** Both rules fire in the same run. If the seed rule sits in
`MotnClient`, where the spec puts it and the only place that knows a seed happened, it writes T₀ during
`fetch`. The end-of-run "set if unset" rule then finds the key already set and does nothing. Result:
- The baseline is T₀. The whole initial catalogue (about 4.8k titles) is "new" for 30 days.
- Trending tier 1 is everything, sorted by `added_at` descending. Ties fall back to `id` ascending.
- That is chunked insertion order: the `ORDER BY id` that §1 calls the bug. It runs for the first month on
  the default sort.

A re-seed after a 25-day MOTN gap does the same thing. The rule meant to make bulk imports "never count as
new" makes them count as new.

**`run_sync` cannot do it correctly either, as the spec stands.** `CatalogueSource::fetch` returns only
`Vec<FetchedTitle>` (`src/sync/mod.rs:76`). The orchestrator cannot tell that a seed happened. To do it
right you would need one of these:
- a trait change that reports the mode;
- the generic orchestrator reading `motn.last_mode` / `motn.last_seed_at` out of `app_meta`, which couples
  it to one source's keys;
- `MotnClient` writing a time that is deliberately later than its own finish, which is not a finish time.

**The tests will not catch it.**
- `MotnClient::fetch` is never run in tests. `MotnClient::new` does not appear in any test, and
  `Cargo.toml` has no HTTP mock crate. The orchestrator tests use `FakeSource`, which has no idea of a seed.
- §8's tests ("baseline set after the first ok sync and moved after a seed") check that the key gets
  written. None checks that a title inserted by the seed is *not* new.

**It still misses the Plex-late case.** r2 problem B was "sources join at different times". Revision 3
fixed only MOTN-late, because only MOTN has a "seed". Suppose a user runs MOTN first and adds `PLEX_URL` /
`PLEX_TOKEN` later. Plex's first fetch inserts the user's whole library, minus any overlap with
Disney+/Crunchyroll. That is a bulk import, and the baseline is not moved. So the user's own library is
"new" and tops the discovery default for 30 days. The coupling also works the other way. A MOTN re-seed
moves the one global baseline forward and strips "new" from Plex titles that really were added last week.

**The simpler design, with no state.** "Bulk" can be read straight from the data. Every bulk insert lands
thousands of rows within a minute or two. Every legitimate "new" arrives in a daily delta of tens.
- Define: `newSince` is set when `added_at` is within 30 days **and** fewer than N titles (say 200) have an
  `added_at` within ±30 minutes of it.
- That is one window query, or a sort plus a sliding window in `fetch_catalogue`, over at most 5k rows.
- It handles all of these the same way: fresh install, re-seed, MOTN-late, Plex-late, a new service in
  `WANTED`, a restore from backup.
- It removes migration `0011`, the `catalogue.baseline_at` key, both update rules, the question of who
  writes the key, and the migration test row in §8.

Pin it with one test that inserts 500 titles in one burst plus 3 titles a day later, and expects exactly 3
to be new.

---

## Objection 2: For you's formula lets rating weight drown out similarity. Ratings of 7 and 8 have almost no effect, the negative term is close to a constant, and §8's toy-vector tests cannot show any of this *(category b: assumption not established)*

The user chose top-k, so this is not an argument for the centroid. It is about how §5.3 builds top-k, and
that formula decides whether the feature's headline promise holds: "distinct tastes stay distinct" (D4).

**The assumption.** §5.3 multiplies first and selects second:
- `pos(c)` = mean of the k largest values of `w_p · cos(c, p)`, where `w ∈ {0.25, 0.5, 0.75, 1}` for
  ratings 7, 8, 9, 10;
- `neg(c)` = max over N of `w_n · cos(c, n)`.

That only works if `cos` varies across a range wide enough for weight to act as a tie-break. With these
embeddings it does not.
- Every vector embeds the same template, `"{title} ({year}) — {kind}. Genres: …. {description}"`
  (`src/services/embeddings.rs:25-35`), with `text-embedding-3-small`.
- That model's cosines for same-domain text sit in a narrow, raised band. Unrelated movie blurbs often
  score around 0.3, and a strong match around 0.6.
- Weight varies by 4×. Within the band, cosine varies by about 2×.

**What follows.**
1. **Weight picks the neighbours.**
   - A 10-rated title the candidate barely resembles (1.0 × 0.30 = 0.30) beats a 7-rated near-match
     (0.25 × 0.70 = 0.175).
   - With the 300-positive cap filled by an IMDb import, which brings in thousands of rows per the import
     spec, every candidate's top-5 comes from the user's 9s and 10s, whatever they are similar to.
   - The profile reduces to "near my 10s, then my 9s". Suppose the user's anime is rated 7–8 and their
     Pixar is rated 9–10. Anime candidates then lose to middling Pixar-adjacent titles. That is the
     taste-merging D4 was meant to stop.
2. **`neg` is an extreme value over up to 100 items.** In a raised band, almost every candidate has *some*
   disliked title at around 0.5. So `0.5 · neg(c)` is close to the same offset for everyone. It reorders
   candidates by noise more than by dislike, and the offset rises as the user rates more 1–4s.
   - Meanwhile `pos` is a smoothed mean of 5. The two terms are not on comparable scales, so the 0.5
     coefficient has nothing to calibrate against.
3. **§8 cannot find this.** "For you top-k scoring, negative penalty" will be tested with hand-built
   vectors, where cosines are 0 or 1 and the band problem never appears. The suite passes with the feature
   ranking by the user's 10s.

**How to establish it first.** On the homelab DB, one throwaway script that reads `title_embeddings` (as
`embeddings::load_all` does) can print:
- the distribution of `cos(c, p)` over candidates × positives;
- how often the top-5 under `w · cos` differs from the top-5 under raw `cos`.

That costs an afternoon, uses no external calls, and decides the constants.

**The likely fixes are no more work than the spec.**
- **Select by similarity, then weight:** take the k nearest positives by raw `cos`, and score
  `Σ wᵢ cosᵢ / Σ wᵢ`. That is a standard weighted kNN. Weight then means "how much I liked my nearest
  neighbours" and cannot pull in strangers.
- **Mean-centre the vectors:** subtract the catalogue centroid (`similarity::centroid` exists), then
  re-normalise. This removes the shared template component and spreads the cosines. It is one pass per
  sync.
- **Match `neg` to `pos`:** use a mean of the top-k (k = 1–3), or require `cos(c, n)` to beat `c`'s best
  positive cosine before any penalty applies.

---

## Objection 3: For you's recompute sits in the rating loop, and the fingerprint cache misses on exactly the events that trigger it. The cost grows with the feature's success, on a server that blocks a worker while it computes *(category c: the 6-month collision)*

r2 flagged that every `loadForYou` re-reads all embedding blobs (runner-up). Revision 3 answers with
"cached in memory, keyed by a fingerprint … so repeat loads are free" (§5.3). That fix aims at the wrong
loads.

**The cache cannot hit on the loads that matter.** §6 fires `loadForYou()` after `setRating`,
`clearRating` and `toggleWatched` succeed. Each of these changes the fingerprint by construction:
- `COUNT(*)` / `MAX(rated_at)`: `set_rating` sets `rated_at = datetime('now')`, `src/db/user_data.rs:38`;
- the watch-history count: `set_watched` inserts or deletes a `manual` row, `src/db/user_data.rs:66-84`.

So every post-action reload is a full recompute. The cache helps only with page reloads, which were never
the problem.

**What a recompute costs.**
- `embeddings::load_all` decodes about 5k × 1536 f32 (about 30 MB of BLOBs) on every call
  (`src/db/embeddings.rs:76-85`). Nothing caches the decoded vectors.
- Then |candidates| × (|P| + |N|) dot products. That is up to about 5k × 400 × 1536 ≈ 3 × 10⁹
  multiply-adds, by the spec's own bound.
- `similarity::dot` is `a.iter().zip(b).map(..).sum()` on f32 (`src/services/similarity.rs:5-7`). LLVM
  cannot vectorise that strict-order float reduction, so it runs at the latency of one add per step.
- Estimate: seconds per call on a desktop core, and several times that on typical homelab hardware. It
  should be measured, but it is not milliseconds.

**Where it runs.** The established pattern (`AskEngine::similar`, `src/services/ask_engine.rs:106-135`)
does vector work directly in an async handler. There is no `spawn_blocking` anywhere in `src/`. A For you
handler built the same way holds an actix worker thread for the whole computation. A catalogue load plus a
rating reload arriving together are two concurrent full computations, because §5.3 specifies no
single-flight. Image-proxy and catalogue requests queue behind them.

**The UI makes the cost visible.** §6 says "Until `ready`, For you renders in Trending order". If
`loadForYou` drops back to `loading` while it recomputes (the spec does not say otherwise), then every
rating made under For you visibly reshuffles the grid to Trending and back several seconds later. The
just-rated title also stays in its old For you slot until the server replies, even though the client
already knows it is rated (`ratings` / `watched` are in the store).

**Why this is the 6-month collision.**
- The For you loop is "rate titles, get better picks". |P| rises toward the 300 cap as the user rates in
  the app.
- The catalogue only grows. MOTN deltas add titles and prunes are rare.
- The obvious next feature, another service in `WANTED`, multiplies |candidates|.
- Cost scales with the product of these. The feature gets slower as it gets used, in the one place the
  user expects an instant response.

**Fix it in the design, not in the plan.**
1. **Take watch state out of the server's job.** Watch changes never change P or N. They only change which
   candidates are excluded.
   - Score every embedded title, and let the client sink watched and rated titles from state it already
     has (D5).
   - Then `toggleWatched` needs no reload, the watch count leaves the fingerprint, and a just-rated title
     sinks at once.
2. **Cache the decoded, normalised (and, per Objection 2, centred) vectors in process.** Invalidate them on
   sync only.
3. **Make a rating change cheap.**
   - Keep each candidate's top-k positive scores.
   - Adding or re-weighting one positive is one pass of |candidates| dot products, about 8M multiply-adds:
     milliseconds.
   - Do a full recompute only on sync or when a rating is cleared. At minimum, run the full computation in
     `spawn_blocking` behind a single-flight lock.
4. **Stale-while-revalidate on the frontend.** Keep `status: 'ready'` with the old `ids` while a reload is
   in flight, so the grid never falls back to Trending order after a rating.

---

## Runners-up

- **The fingerprint can serve a stale result after a sync.** `run_sync` records `sync_runs` (the
  fingerprint's "latest `sync_runs.id`") at `src/sync/mod.rs:162`, before the embedding backfill at `:222`.
  A For you computed mid-sync, for example a rating made while "Sync now" runs, is cached under the final
  fingerprint. The post-sync reload then hits that cache and misses every title embedded afterwards, until
  the next rating or sync. Add `COUNT(*)` of `title_embeddings` for the current model to the fingerprint.
- **Trending, the default, does not sink seen titles.** D5 applies only to For you. Tier 2 is "the rest by
  external rating". This user has an IMDb ratings import and Plex watch history, so the top-rated Disney+
  and Crunchyroll titles are mostly ones they have already seen. The default grid's first screens become
  their own history. `FilterBar` has no hide-watched filter. Applying the D5 sink to Trending costs one
  comparator line in `visibleTitles`.
- **§8's MOTN tests need a transport seam that does not exist.** "Counter increments per call", "429 aborts
  and skips the rest" and "fake client asserts zero requests" all require MOTN HTTP calls to be
  interceptable. `MotnClient` holds a concrete `reqwest::Client` (`src/sync/motn.rs`), `fetch` has no test
  coverage, and `Cargo.toml` has no HTTP mock crate. "MOTN client only" (§4) leaves out the refactor that
  every one of those tests depends on. Name it, a transport trait or a mock-server dev-dependency, so the
  plan sizes it.
- **The Phase 2 popularity gate measures the cheap months.** "~2 weeks of Phase 1 request counts" will
  almost never include a full seed, which happens only on an empty cache or a 25-day gap. So the gate
  confirms the ~35–65/month steady state that §4 already estimates, and says nothing about whether a seed
  month (+250–340) leaves room for +30. State the gate as "steady state + measured seed cost ≤ soft cap".
  The seed's page count is exactly what the new counter can record the next time a seed runs.
- **`monthlyLimit: 500` is hard-coded in the status DTO (§5.4).** It is a property of the user's MOTN plan,
  not of the code. Make it a `Config` value with a default, or leave it out and show only the count.
