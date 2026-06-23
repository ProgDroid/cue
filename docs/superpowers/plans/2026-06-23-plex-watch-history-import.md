# Plex Watch-History Import Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Populate `watch_history` with `source='plex'` rows during catalogue sync so titles watched on Plex surface as watched in cue.

**Architecture:** Watch capture is folded into `run_sync`. A new default-empty `CatalogueSource::fetch_watch_history()` plus a `watch_history_source()` tag method (approach A) keep `FetchedTitle` a pure catalogue DTO; only `PlexClient` overrides them. The orchestrator applies a replace-by-source write per *successful* source that declares a tag. The catalogue read already derives the per-title `watched` boolean from `watch_history`, so no read-path changes are needed.

**Tech Stack:** Rust, Actix-web, SQLx (runtime queries), SQLite, async-trait, serde; Vue 3 + TypeScript + Pinia + Vitest (frontend).

## Global Constraints

- **SQLx runtime queries only** — `sqlx::query` / `query_as` / `query_scalar`. Never the compile-time `query!` / `query_as!` macros (project must build with no live `DATABASE_URL`).
- **Test DBs** — use `tempfile::tempdir()` (NOT `NamedTempFile`); build the URL as `format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"))`; keep the `TempDir` guard bound (`_dir`) for the test's lifetime.
- **Clippy gate** — `cargo clippy --all-targets -- -D warnings` must pass. Per-item exceptions use a local `#[allow(clippy::…)]` with a one-line reason; never widen the global `[lints.clippy]` table.
- **Modules live in `src/lib.rs`** as `pub mod …`; new public items must be reachable so `cargo test --lib` works.
- **Keying** — user-data keys are `imdb_id` (D6). Titles without an `imdb_id` are skipped.
- **D5.2** — `manual` watch rows must never be deleted by an import; only `source='plex'` rows are replaced.
- **Commit via the Bash tool**, not PowerShell (PowerShell prepends a UTF-8 BOM to commit subjects on this machine).

---

### Task 1: Plex watch-history parser (`parse_watch_history`) + `WatchRecord`

**Files:**
- Modify: `src/sync/mod.rs` (add `WatchRecord` struct near `FetchedTitle`, ~line 28)
- Modify: `src/sync/plex.rs` (extend `Meta`; add `parse_watch_history`)
- Test: `src/sync/plex.rs` (in the existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `pub struct WatchRecord { pub key: String, pub watched_at: Option<i64> }` (in `crate::sync`), deriving `Debug, Clone, PartialEq, Eq`.
  - `pub fn parse_watch_history(json: &str) -> anyhow::Result<Vec<WatchRecord>>` (in `crate::sync::plex`).
- Consumes: the existing `Container` / `MediaContainer` / `Meta` / `Tagged` deserialize types and `guid_value` helper already in `plex.rs`.

- [ ] **Step 1: Add the `WatchRecord` struct to `src/sync/mod.rs`**

Insert directly after the `ImageRef` struct (before `FetchedTitle`):

```rust
/// One watched-title record emitted by a source's watch-history scan.
///
/// `key` is the user-data key (currently always an `imdb_id`, per D6).
/// `watched_at` is the source's last-viewed unix epoch (seconds); `None`
/// falls back to the DB default `datetime('now')`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchRecord {
    pub key: String,
    pub watched_at: Option<i64>,
}
```

- [ ] **Step 2: Extend the `Meta` deserializer in `src/sync/plex.rs`**

Add three fields to the existing `struct Meta` (alongside `rating`, `duration`):

```rust
    #[serde(rename = "viewCount")]
    view_count: Option<i64>,
    #[serde(rename = "lastViewedAt")]
    last_viewed_at: Option<i64>,
    #[serde(rename = "viewedLeafCount")]
    viewed_leaf_count: Option<i64>,
```

- [ ] **Step 3: Write the failing test**

Add to `src/sync/plex.rs`'s `mod tests`:

```rust
#[test]
fn parse_watch_history_keeps_watched_movies_and_inprogress_shows() {
    let json = r#"{"MediaContainer":{"Metadata":[
      {"type":"movie","title":"Watched Film","viewCount":1,"lastViewedAt":1600000000,
       "Guid":[{"id":"imdb://tt1"}]},
      {"type":"movie","title":"Unwatched Film","viewCount":0,
       "Guid":[{"id":"imdb://tt2"}]},
      {"type":"movie","title":"Never-Played Film",
       "Guid":[{"id":"imdb://tt3"}]},
      {"type":"show","title":"In-Progress Show","viewedLeafCount":4,"lastViewedAt":1700000000,
       "Guid":[{"id":"imdb://tt4"}]},
      {"type":"show","title":"Untouched Show","viewedLeafCount":0,
       "Guid":[{"id":"imdb://tt5"}]},
      {"type":"movie","title":"Watched No IMDb","viewCount":2,
       "Guid":[{"id":"tmdb://999"}]}
    ]}}"#;
    let out = parse_watch_history(json).unwrap();
    // Watched movie (tt1) + in-progress show (tt4) only. tt2/tt3/tt5 not watched;
    // the no-imdb watched item is skipped (W3).
    let keys: Vec<&str> = out.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(keys, vec!["tt1", "tt4"]);
    assert_eq!(out[0].watched_at, Some(1600000000));
    assert_eq!(out[1].watched_at, Some(1700000000));
}
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test --lib parse_watch_history_keeps_watched -- --nocapture`
Expected: FAIL — `cannot find function parse_watch_history in this scope`.

- [ ] **Step 5: Implement `parse_watch_history`**

Add to `src/sync/plex.rs` (after `parse_section`). Import `WatchRecord`:
update the existing `use crate::sync::{...}` line to include `WatchRecord`.

```rust
/// Parse one `/library/sections/{key}/all` body into watched-title records.
///
/// A movie is "watched" when `viewCount > 0`; a show is "watched" (touched /
/// in-progress) when `viewedLeafCount > 0`. Items without an `imdb://` GUID are
/// skipped — user-data is keyed on `imdb_id` (D6), so a non-imdb watch row
/// would never surface in the catalogue read.
///
/// # Errors
/// Returns an error if the JSON does not match the expected shape.
pub fn parse_watch_history(json: &str) -> anyhow::Result<Vec<WatchRecord>> {
    let parsed: Container = serde_json::from_str(json)?;
    Ok(parsed
        .media_container
        .metadata
        .into_iter()
        .filter_map(|m| {
            let watched = match m.kind.as_str() {
                "movie" => m.view_count.unwrap_or(0) > 0,
                "show" => m.viewed_leaf_count.unwrap_or(0) > 0,
                _ => false,
            };
            if !watched {
                return None;
            }
            let key = guid_value(&m.guid, "imdb")?; // skip no-imdb items (W3)
            Some(WatchRecord {
                key,
                watched_at: m.last_viewed_at,
            })
        })
        .collect())
}
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test --lib parse_watch_history_keeps_watched`
Expected: PASS.

- [ ] **Step 7: Run clippy and the full lib test suite**

Run: `cargo clippy --all-targets -- -D warnings && cargo test --lib`
Expected: no clippy warnings; all tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/sync/mod.rs src/sync/plex.rs
git commit -m "feat(sync): parse_watch_history + WatchRecord (Plex watched-title extraction)"
```

---

### Task 2: `replace_watch_history` DB write (replace-by-source)

**Files:**
- Modify: `src/db/user_data.rs` (add `replace_watch_history`; add tests in its `mod tests`)

**Interfaces:**
- Consumes: `crate::sync::WatchRecord` (from Task 1).
- Produces:
  - `pub async fn replace_watch_history(pool: &SqlitePool, source: &str, records: &[WatchRecord]) -> anyhow::Result<usize>` — deletes all rows with the given `source`, inserts one row per record (`watched_at` from epoch via `datetime(?, 'unixepoch')`, falling back to `datetime('now')`), returns the number of rows written. Runs in a single transaction.

- [ ] **Step 1: Write the failing test**

Add to `src/db/user_data.rs`'s `mod tests`:

```rust
#[tokio::test]
async fn replace_watch_history_replaces_plex_and_preserves_manual() {
    use crate::sync::WatchRecord;
    let (pool, _dir) = fresh_pool().await;

    // Pre-existing state: a stale plex row + a manual row that must survive.
    sqlx::query("INSERT INTO watch_history (imdb_id, source) VALUES ('ttOLD', 'plex')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO watch_history (imdb_id, source) VALUES ('ttMAN', 'manual')")
        .execute(&pool).await.unwrap();

    let records = vec![
        WatchRecord { key: "ttNEW".into(), watched_at: Some(1_600_000_000) },
        WatchRecord { key: "ttNODATE".into(), watched_at: None },
    ];
    let written = replace_watch_history(&pool, "plex", &records).await.unwrap();
    assert_eq!(written, 2);

    // Stale plex row gone; both new plex rows present; manual row preserved.
    let plex: Vec<String> = sqlx::query_scalar(
        "SELECT imdb_id FROM watch_history WHERE source = 'plex' ORDER BY imdb_id",
    ).fetch_all(&pool).await.unwrap();
    assert_eq!(plex, vec!["ttNEW".to_string(), "ttNODATE".to_string()]);

    let manual: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM watch_history WHERE source = 'manual' AND imdb_id = 'ttMAN'",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(manual, 1);

    // Epoch converted to ISO; None fell back to a non-empty now() timestamp.
    let dated: String = sqlx::query_scalar(
        "SELECT watched_at FROM watch_history WHERE imdb_id = 'ttNEW'",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(dated, "2020-09-13 12:26:40"); // datetime(1600000000,'unixepoch')
    let nodate: String = sqlx::query_scalar(
        "SELECT watched_at FROM watch_history WHERE imdb_id = 'ttNODATE'",
    ).fetch_one(&pool).await.unwrap();
    assert!(!nodate.is_empty());
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib replace_watch_history_replaces_plex`
Expected: FAIL — `cannot find function replace_watch_history`.

- [ ] **Step 3: Implement `replace_watch_history`**

Add to `src/db/user_data.rs` (and add `use crate::sync::WatchRecord;` to the file's imports):

```rust
/// Replace all `watch_history` rows of a given `source` with `records`,
/// in one transaction. Rows of other sources (including `'manual'`) are
/// untouched (D5.2). Returns the number of rows written.
///
/// `watched_at` is taken from each record's unix-epoch `watched_at` via
/// `datetime(?, 'unixepoch')`, falling back to `datetime('now')` when absent.
///
/// # Errors
/// Returns an error if any query or the transaction fails (rolls back; no
/// partial replace).
pub async fn replace_watch_history(
    pool: &SqlitePool,
    source: &str,
    records: &[WatchRecord],
) -> anyhow::Result<usize> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM watch_history WHERE source = ?")
        .bind(source)
        .execute(&mut *tx)
        .await?;
    for r in records {
        sqlx::query(
            "INSERT INTO watch_history (imdb_id, watched_at, source)
             VALUES (?, COALESCE(datetime(?, 'unixepoch'), datetime('now')), ?)",
        )
        .bind(&r.key)
        .bind(r.watched_at)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(records.len())
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib replace_watch_history_replaces_plex`
Expected: PASS.

- [ ] **Step 5: Run clippy and the full lib test suite**

Run: `cargo clippy --all-targets -- -D warnings && cargo test --lib`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/db/user_data.rs
git commit -m "feat(db): replace_watch_history (replace-by-source, preserves manual rows)"
```

---

### Task 3: Trait methods, Plex override, and orchestrator wiring

**Files:**
- Modify: `src/sync/mod.rs` (two new trait methods with defaults; `run_sync` apply step; new orchestrator test)
- Modify: `src/sync/plex.rs` (override both methods on `PlexClient`)

**Interfaces:**
- Consumes: `WatchRecord` (Task 1), `replace_watch_history` (Task 2).
- Produces (on the `CatalogueSource` trait):
  - `fn watch_history_source(&self) -> Option<&'static str>` (default `None`).
  - `async fn fetch_watch_history(&self) -> anyhow::Result<Vec<WatchRecord>>` (default `Ok(Vec::new())`).
  - `PlexClient` overrides both: tag `Some("plex")`; fetch sweeps sections and concatenates `parse_watch_history` output.

- [ ] **Step 1: Add the two trait methods (with defaults) to `CatalogueSource` in `src/sync/mod.rs`**

Inside the `pub trait CatalogueSource` block, after the existing `fetch` method:

```rust
    /// The `watch_history.source` tag this client writes, if any.
    /// `Some(tag)` opts the source into the watch-history replace; the default
    /// `None` skips it (so a non-watch source never wipes another's rows).
    fn watch_history_source(&self) -> Option<&'static str> {
        None
    }

    /// Fetch the user's watch history from this source (default: none).
    ///
    /// # Errors
    /// Returns an error if the upstream request fails or a body cannot be parsed.
    async fn fetch_watch_history(&self) -> anyhow::Result<Vec<WatchRecord>> {
        Ok(Vec::new())
    }
```

- [ ] **Step 2: Override both methods on `PlexClient` in `src/sync/plex.rs`**

Inside `impl CatalogueSource for PlexClient`, after the existing `fetch`:

```rust
    fn watch_history_source(&self) -> Option<&'static str> {
        Some("plex")
    }

    async fn fetch_watch_history(&self) -> anyhow::Result<Vec<WatchRecord>> {
        let sections: Sections = serde_json::from_str(&self.get_json("/library/sections").await?)?;
        let mut out = Vec::new();
        for dir in sections.media_container.directory {
            if dir.kind == "movie" || dir.kind == "show" {
                let body = self
                    .get_json(&format!("/library/sections/{}/all", dir.key))
                    .await?;
                out.extend(parse_watch_history(&body)?);
            }
        }
        Ok(out)
    }
```

- [ ] **Step 3: Track successful sources and apply watch history in `run_sync` (`src/sync/mod.rs`)**

In `run_sync`, add an `ok_sources` accumulator. Change the fetch loop's success arm to also record the source reference:

```rust
    let mut fetched: Vec<FetchedTitle> = Vec::new();
    let mut ok_services: Vec<Service> = Vec::new();
    let mut ok_sources: Vec<&Arc<dyn CatalogueSource>> = Vec::new();
    let mut failures: Vec<(Service, String)> = Vec::new();

    for src in sources {
        match src.fetch().await {
            Ok(mut rows) => {
                tracing::info!("sync source {} fetched {} rows", src.name(), rows.len());
                fetched.append(&mut rows);
                ok_services.extend_from_slice(src.services());
                ok_sources.push(src);
            }
            Err(e) => {
                tracing::error!("sync source {} failed: {e:#}", src.name());
                for s in src.services() {
                    failures.push((*s, format!("{e:#}")));
                }
            }
        }
    }
```

Then, after the prune block and before the embedding backfill, add the watch-history apply step:

```rust
    // Apply watch history for each successful source that owns a watch tag.
    // Secondary enrichment: a fetch or apply error is logged, never fatal, and
    // a failed (or tag-less) source never runs a replace — so a Plex outage or
    // a non-watch source (MOTN) cannot wipe existing plex rows.
    for src in &ok_sources {
        if let Some(tag) = src.watch_history_source() {
            match src.fetch_watch_history().await {
                Ok(records) => match crate::db::user_data::replace_watch_history(
                    pool, tag, &records,
                )
                .await
                {
                    Ok(n) => tracing::info!("sync wrote {n} {tag} watch-history rows"),
                    Err(e) => tracing::error!("watch-history apply failed for {tag}: {e:#}"),
                },
                Err(e) => {
                    tracing::error!("watch-history fetch failed for {}: {e:#}", src.name());
                }
            }
        }
    }
```

- [ ] **Step 4: Write the failing orchestrator tests**

Add a dedicated fake + three tests to `mod orchestrator_tests` in `src/sync/mod.rs`. The fake supports a fetch result, a watch tag, and watch records:

```rust
    struct WatchFake {
        name: &'static str,
        services: Vec<Service>,
        fetch: anyhow::Result<Vec<FetchedTitle>>,
        watch_source: Option<&'static str>,
        watch: Vec<WatchRecord>,
    }
    #[async_trait]
    impl CatalogueSource for WatchFake {
        fn name(&self) -> &'static str {
            self.name
        }
        fn services(&self) -> &'static [Service] {
            Box::leak(self.services.clone().into_boxed_slice())
        }
        async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>> {
            match &self.fetch {
                Ok(v) => Ok(v.clone()),
                Err(e) => Err(anyhow::anyhow!("{e}")),
            }
        }
        fn watch_history_source(&self) -> Option<&'static str> {
            self.watch_source
        }
        async fn fetch_watch_history(&self) -> anyhow::Result<Vec<WatchRecord>> {
            Ok(self.watch.clone())
        }
    }

    async fn plex_watch_count(pool: &sqlx::SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM watch_history WHERE source = 'plex'")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn successful_source_applies_watch_history() {
        let (p, _dir) = pool().await;
        // A stale plex row should be replaced by the new scan.
        sqlx::query("INSERT INTO watch_history (imdb_id, source) VALUES ('ttSTALE','plex')")
            .execute(&p).await.unwrap();
        let plex = Arc::new(WatchFake {
            name: "plex",
            services: vec![Service::Plex],
            fetch: Ok(vec![title("ttP", vec![Service::Plex])]),
            watch_source: Some("plex"),
            watch: vec![WatchRecord { key: "ttW".into(), watched_at: Some(1_600_000_000) }],
        }) as Arc<dyn CatalogueSource>;
        run_sync(&p, &[plex], None).await.unwrap();

        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT imdb_id FROM watch_history WHERE source = 'plex' ORDER BY imdb_id",
        ).fetch_all(&p).await.unwrap();
        assert_eq!(rows, vec!["ttW".to_string()]); // stale replaced
    }

    #[tokio::test]
    async fn failed_source_does_not_wipe_watch_history() {
        let (p, _dir) = pool().await;
        sqlx::query("INSERT INTO watch_history (imdb_id, source) VALUES ('ttOLD','plex')")
            .execute(&p).await.unwrap();
        // Plex fetch fails -> source not "successful" -> apply skipped.
        let plex_fail = Arc::new(WatchFake {
            name: "plex",
            services: vec![Service::Plex],
            fetch: Err(anyhow::anyhow!("down")),
            watch_source: Some("plex"),
            watch: vec![],
        }) as Arc<dyn CatalogueSource>;
        // A MOTN success so the run still does real work.
        let motn = Arc::new(WatchFake {
            name: "motn",
            services: vec![Service::Disney, Service::Crunchyroll],
            fetch: Ok(vec![title("ttC", vec![Service::Crunchyroll])]),
            watch_source: None,
            watch: vec![],
        }) as Arc<dyn CatalogueSource>;
        run_sync(&p, &[plex_fail, motn], None).await.unwrap();
        assert_eq!(plex_watch_count(&p).await, 1); // ttOLD survives the outage
    }

    #[tokio::test]
    async fn tagless_source_does_not_touch_watch_history() {
        let (p, _dir) = pool().await;
        sqlx::query("INSERT INTO watch_history (imdb_id, source) VALUES ('ttOLD','plex')")
            .execute(&p).await.unwrap();
        // A successful source with no watch tag must not run a replace.
        let motn = Arc::new(WatchFake {
            name: "motn",
            services: vec![Service::Disney, Service::Crunchyroll],
            fetch: Ok(vec![title("ttC", vec![Service::Crunchyroll])]),
            watch_source: None,
            watch: vec![],
        }) as Arc<dyn CatalogueSource>;
        run_sync(&p, &[motn], None).await.unwrap();
        assert_eq!(plex_watch_count(&p).await, 1); // ttOLD untouched (wipe-guard)
    }
```

- [ ] **Step 5: Run the tests to verify they fail**

Run: `cargo test --lib applies_watch_history does_not_wipe does_not_touch`
Expected: FAIL initially if implementation steps were skipped; after Steps 1-3 they should compile. If Step 4 is written before 1-3, expect compile errors about missing methods. (Write 4 last among 1-4, then run.)

Note: write Steps 1-3 first, then Step 4, then run here.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib successful_source_applies_watch_history failed_source_does_not_wipe_watch_history tagless_source_does_not_touch_watch_history`
Expected: PASS (all three).

- [ ] **Step 7: Run clippy and the full lib test suite**

Run: `cargo clippy --all-targets -- -D warnings && cargo test --lib`
Expected: clean. (Existing `FakeSource`-based tests still compile because the new trait methods have defaults.)

- [ ] **Step 8: Commit**

```bash
git add src/sync/mod.rs src/sync/plex.rs
git commit -m "feat(sync): fold Plex watch-history capture into run_sync (tag-gated replace)"
```

---

### Task 4: Frontend — re-fetch catalogue when a sync completes

**Files:**
- Modify: `frontend/src/views/SettingsView.vue` (the `refresh` function, ~lines 79-90)
- Test: `frontend/src/views/SettingsView.spec.ts`

**Interfaces:**
- Consumes: `useCatalogueStore()` (already imported in `SettingsView.vue`) with its `load()` action.
- Produces: on a running→finished transition during polling, the catalogue store reloads so newly-synced titles and imported watched flags surface without a manual reload.

- [ ] **Step 1: Inspect the existing test file and catalogue store action**

Read `frontend/src/views/SettingsView.spec.ts` and `frontend/src/stores/catalogue.ts` to confirm the `load` action name and the existing mocking style (how `getSyncStatus` / `triggerSync` are mocked). Mirror that style in Step 3.

- [ ] **Step 2: Modify `refresh()` in `SettingsView.vue` to reload the catalogue on completion**

Replace the body of `refresh()` so it remembers the prior running state and reloads when a run finishes:

```ts
async function refresh() {
  try {
    const wasRunning = busy.value
    status.value = await getSyncStatus()
    busy.value = status.value.running
    if (!status.value.running && poll) {
      clearInterval(poll)
      poll = undefined
    }
    // A sync just finished (running -> not running): reload the catalogue so
    // newly-synced titles and imported watched flags surface without a reload.
    // Store accessed lazily so non-Pinia test mounts are unaffected.
    if (wasRunning && !status.value.running) {
      await useCatalogueStore().load()
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : 'failed to load status'
  }
}
```

- [ ] **Step 3: Write the failing test**

Add to `frontend/src/views/SettingsView.spec.ts` a test that drives a running→done transition and asserts `catalogueStore.load` is called. Use the file's existing mock setup as the template; the essential assertions:

```ts
it('reloads the catalogue when a sync transitions running -> done', async () => {
  // getSyncStatus: first call running:true, second call running:false.
  const getSyncStatus = vi
    .fn()
    .mockResolvedValueOnce({ running: true, sources: [], catalogue: { titles: 0, movies: 0, series: 0, embedded: 0 }, lastRun: null })
    .mockResolvedValueOnce({ running: false, sources: [], catalogue: { titles: 0, movies: 0, series: 0, embedded: 0 }, lastRun: null })
  // (wire this mock through the same module-mock mechanism the file already uses)

  const load = vi.fn().mockResolvedValue(undefined)
  // (stub useCatalogueStore to return an object exposing `load`, matching the
  //  store-mocking approach already used in the suite)

  // mount, run onMounted refresh (running:true), then invoke a second refresh
  // (running:false) and flush promises.
  // assert:
  expect(load).toHaveBeenCalledTimes(1)
})
```

Implement the test concretely using the suite's existing mocks (do not invent a new mocking style — match `SettingsView.spec.ts`).

- [ ] **Step 4: Run the test to verify it fails (then passes after Step 2)**

Run: `cd frontend && npm run test -- SettingsView`
Expected: the new test FAILs before Step 2's change is in place; PASS after. (If Step 2 is already applied, confirm the assertion fails when you temporarily revert it, then restore.)

- [ ] **Step 5: Run the frontend build + full test suite**

Run: `cd frontend && npm run test && npm run build`
Expected: all tests pass; build succeeds.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/views/SettingsView.vue frontend/src/views/SettingsView.spec.ts
git commit -m "feat(web): reload catalogue when a sync completes (surface watched flags)"
```

---

### Task 5: Update the deferred-followups backlog

**Files:**
- Modify: `docs/superpowers/deferred-followups.md`

- [ ] **Step 1: Mark the feature done and add a live-verify follow-up**

In the "Features" section, replace the `Plex watch-history import` bullet with a
done entry, and add a live-verify follow-up bullet beneath it:

```markdown
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
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/deferred-followups.md
git commit -m "docs: mark Plex watch-history import done; record live-verify follow-up"
```

---

## Self-Review

**1. Spec coverage:**
- W1 (folded into sync) → Task 3 (`run_sync` apply). ✓
- W2 (seam: `fetch_watch_history` + `watch_history_source` gate) → Task 3 Steps 1-2. ✓
- W3 (imdb_id keying, skip no-imdb) → Task 1 Step 5 (`guid_value(...,"imdb")?`) + test. ✓
- W4 (movie `viewCount>0`, series `viewedLeafCount>0`) → Task 1 Step 5 + test. ✓
- W5 (title-level granularity, season/episode NULL) → Task 2 insert omits season/episode (default NULL). ✓
- W6 (replace-by-source; scoped to success AND tag) → Task 2 (`replace_watch_history`) + Task 3 apply loop + tests (`failed_source_does_not_wipe`, `tagless_source_does_not_touch`). ✓
- W7 (`watched_at` epoch→ISO, now() fallback) → Task 2 `COALESCE(datetime(?, 'unixepoch'), datetime('now'))` + test. ✓
- §4.4 (frontend reload on completion) → Task 4. ✓
- §6 (errors logged non-fatal) → Task 3 Step 3 apply loop (log, no `?`). ✓
- §7 (testing) → Tasks 1-4 tests. ✓
- §8 (live-verify follow-up) → Task 5. ✓

**2. Placeholder scan:** Task 4 Step 3 intentionally points the implementer at the suite's existing mock style rather than hardcoding a mock mechanism that may not match — the concrete assertions and the transition being tested are spelled out. All Rust steps contain complete code. No TBD/TODO. ✓

**3. Type consistency:**
- `WatchRecord { key: String, watched_at: Option<i64> }` — used identically in Tasks 1, 2, 3. ✓
- `replace_watch_history(pool, source: &str, records: &[WatchRecord]) -> Result<usize>` — defined Task 2, called Task 3 with `(pool, tag, &records)`. ✓
- `watch_history_source() -> Option<&'static str>` / `fetch_watch_history() -> Result<Vec<WatchRecord>>` — defined Task 3 Step 1, overridden Task 3 Step 2, faked in Task 3 Step 4. ✓
- `parse_watch_history(&str) -> Result<Vec<WatchRecord>>` — Task 1, consumed by `PlexClient::fetch_watch_history` Task 3 Step 2. ✓
