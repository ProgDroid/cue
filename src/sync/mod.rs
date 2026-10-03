//! Catalogue sync subsystem: external clients behind `CatalogueSource`,
//! pure merge logic, DB reconciliation, and the run orchestrator.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use sqlx::SqlitePool;

use crate::db::sync_runs;
use crate::models::{Service, TitleKind};
use crate::services::embeddings::{backfill, Embedder};

pub mod anilist;
pub mod merge;
pub mod motn;
pub mod plex;
pub mod store;

/// One artwork reference plus how the proxy endpoint must serve it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// Public absolute URL (when `remote`) or a relative Plex path (when not).
    pub value: String,
    /// `true` => public CDN URL (302 redirect); `false` => Plex path (token proxy).
    pub remote: bool,
}

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

/// A source-agnostic catalogue row emitted by every `CatalogueSource`.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedTitle {
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub plex_guid: Option<String>,
    pub title: String,
    pub year: Option<i64>,
    pub kind: TitleKind,
    pub score: Option<f64>,
    pub length: Option<String>,
    pub description: Option<String>,
    pub genres: Vec<String>,
    pub cast: Vec<String>,
    pub services: Vec<Service>,
    /// Plex per-item ratingKey (Plex source only); used to build a Plex web link.
    pub plex_rating_key: Option<String>,
    /// Per-service "watch here" web links (MOTN `link`); empty for Plex.
    pub links: Vec<(Service, String)>,
    pub poster: Option<ImageRef>,
    pub backdrop: Option<ImageRef>,
}

/// An external catalogue client. A *client* is the unit of fetching and failure;
/// the *services* it owns are the unit of membership reconciliation.
#[allow(clippy::double_must_use)] // async_trait expansion marks the boxed future #[must_use]
#[async_trait]
pub trait CatalogueSource: Send + Sync {
    /// Client name, used for logging/orchestration (e.g. "plex", "motn").
    fn name(&self) -> &'static str;
    /// The `title_services` membership(s) this client owns
    /// (Plex → `[Plex]`; MOTN → `[Disney, Crunchyroll]`).
    fn services(&self) -> &'static [Service];
    /// Fetch the client's current catalogue.
    ///
    /// # Errors
    /// Returns an error if the upstream request fails or a body cannot be parsed.
    async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>>;

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

    /// Server-global metadata to persist after a successful fetch (e.g. the Plex
    /// `machineIdentifier`, used to build watch deep links). Default: none.
    ///
    /// # Errors
    /// Returns an error if the upstream request fails or the body cannot be parsed.
    async fn server_meta(&self) -> anyhow::Result<Vec<(String, String)>> {
        Ok(Vec::new())
    }
}

/// Run one full sync: fetch every source, merge, reconcile each successfully
/// fetched service, prune orphans, embed new titles, and record `sync_runs`.
///
/// Never returns `Err` for a single source failure — a failed source is recorded
/// and its services are left untouched (scoped prune).
///
/// # Errors
/// Returns an error only if a DB write fails irrecoverably.
pub async fn run_sync(
    pool: &SqlitePool,
    sources: &[Arc<dyn CatalogueSource>],
    embedder: Option<&dyn Embedder>,
) -> anyhow::Result<()> {
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

    let merged = merge::merge(fetched);
    #[allow(clippy::type_complexity)] // three-tuple (id, services, links) is self-documenting here
    let mut id_services: Vec<(i64, Vec<Service>, Vec<(Service, String)>)> =
        Vec::with_capacity(merged.len());
    for m in &merged {
        let id = store::upsert_title(pool, m).await?;
        id_services.push((id, m.services.clone(), m.links.clone()));
    }

    // Reconcile + record only the services whose client succeeded.
    let mut unique_ok = ok_services.clone();
    unique_ok.sort_by_key(|s| s.as_str());
    unique_ok.dedup();
    for svc in &unique_ok {
        let desired: Vec<(i64, Option<String>)> = id_services
            .iter()
            .filter(|(_, services, _)| services.contains(svc))
            .map(|(id, _, links)| {
                let link = links.iter().find(|(s, _)| s == svc).map(|(_, l)| l.clone());
                (*id, link)
            })
            .collect();
        let count = i64::try_from(desired.len()).unwrap_or(i64::MAX);
        store::reconcile_service(pool, *svc, &desired).await?;
        sync_runs::record(pool, svc.as_str(), "ok", count, None).await?;
    }
    for (svc, err) in &failures {
        sync_runs::record(pool, svc.as_str(), "error", 0, Some(err)).await?;
    }

    // Prune titles that have no membership in any service touched this run
    // (either successfully reconciled or explicitly failed/protected).
    // This preserves titles exclusively owned by failed sources (scoped prune)
    // while removing stale seed/previous data from unrelated services.
    // Prune only when at least one source succeeded. If every source failed
    // (a total outage), skip pruning entirely so a transient outage never
    // empties the catalogue. `all_touched` includes failed sources' services
    // so their existing titles are never pruned when another source did sync.
    if !unique_ok.is_empty() {
        let all_touched: Vec<&str> = unique_ok
            .iter()
            .map(|s| s.as_str())
            .chain(failures.iter().map(|(s, _)| s.as_str()))
            .collect();
        let pruned = store::prune_orphans_scoped(pool, &all_touched).await?;
        tracing::info!("sync pruned {pruned} orphaned titles");
    }

    // Apply watch history for each successful source that owns a watch tag.
    // Secondary enrichment: a fetch or apply error is logged, never fatal, and
    // a failed (or tag-less) source never runs a replace — so a Plex outage or
    // a non-watch source (MOTN) cannot wipe existing plex rows.
    for src in &ok_sources {
        if let Some(tag) = src.watch_history_source() {
            match src.fetch_watch_history().await {
                Ok(records) => {
                    match crate::db::user_data::replace_watch_history(pool, tag, &records).await {
                        Ok(n) => tracing::info!("sync wrote {n} {tag} watch-history rows"),
                        Err(e) => tracing::error!("watch-history apply failed for {tag}: {e:#}"),
                    }
                }
                Err(e) => {
                    tracing::error!("watch-history fetch failed for {}: {e:#}", src.name());
                }
            }
        }
    }

    // Persist server-global metadata (e.g. Plex machineIdentifier) from each
    // successful source. Non-fatal: a failure logs and leaves the prior value,
    // so a stale-but-present machine id still builds working links.
    for src in &ok_sources {
        match src.server_meta().await {
            Ok(pairs) => {
                for (k, v) in pairs {
                    if let Err(e) = crate::db::app_meta::set(pool, &k, &v).await {
                        tracing::error!("app_meta set {k} failed: {e:#}");
                    }
                }
            }
            Err(e) => tracing::error!("server_meta fetch failed for {}: {e:#}", src.name()),
        }
    }

    if let Some(emb) = embedder {
        match backfill(pool, emb).await {
            Ok(n) => tracing::info!("sync embedded {n} titles"),
            Err(e) => tracing::error!("sync embedding backfill failed: {e:#}"),
        }
    } else {
        tracing::warn!("OPENAI_API_KEY unset — synced titles left unembedded");
    }

    // Bound sync_runs growth: keep the newest 100 rows per source. Non-fatal.
    match sync_runs::prune_old_runs(pool, 100).await {
        Ok(n) if n > 0 => tracing::info!("pruned {n} old sync_runs rows"),
        Ok(_) => {}
        Err(e) => tracing::warn!("sync_runs prune failed: {e:#}"),
    }
    Ok(())
}

/// Owns the sources + embedder and guards against concurrent runs.
pub struct SyncRunner {
    pool: SqlitePool,
    sources: Vec<Arc<dyn CatalogueSource>>,
    embedder: Option<Arc<dyn Embedder>>,
    anilist_dir: Option<std::path::PathBuf>,
    running: AtomicBool,
}

impl SyncRunner {
    #[must_use]
    pub fn new(
        pool: SqlitePool,
        sources: Vec<Arc<dyn CatalogueSource>>,
        embedder: Option<Arc<dyn Embedder>>,
        anilist_dir: Option<std::path::PathBuf>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            sources,
            embedder,
            anilist_dir,
            running: AtomicBool::new(false),
        })
    }

    /// Whether a source with this client name (e.g. `"motn"`) is configured.
    #[must_use]
    pub fn has_source(&self, name: &str) -> bool {
        self.sources.iter().any(|s| s.name() == name)
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Spawn a sync if none is in flight. Returns `false` if one is already running.
    pub fn try_start(self: &Arc<Self>) -> bool {
        if self
            .running
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return false;
        }
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let embedder = me.embedder.as_deref();
            if let Err(e) = run_sync(&me.pool, &me.sources, embedder).await {
                tracing::error!("sync run failed: {e:#}");
            }
            if let Some(dir) = me.anilist_dir.clone() {
                if let Err(e) = anilist::enrich(&me.pool, &dir).await {
                    tracing::error!("anilist enrich failed: {e:#}");
                }
            }
            me.running.store(false, Ordering::Release);
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetched_title_constructs() {
        let t = FetchedTitle {
            imdb_id: Some("tt1".into()),
            tmdb_id: None,
            plex_guid: None,
            title: "X".into(),
            year: Some(2020),
            kind: TitleKind::Movie,
            score: None,
            length: None,
            description: None,
            genres: vec![],
            cast: vec![],
            services: vec![Service::Plex],
            plex_rating_key: None,
            links: vec![],
            poster: None,
            backdrop: None,
        };
        assert_eq!(t.services, vec![Service::Plex]);
    }
}

#[cfg(test)]
mod orchestrator_tests {
    use super::*;
    use crate::db::{init_pool, seed::seed_if_empty};
    use crate::models::{Service, TitleKind};
    use std::sync::Arc;

    struct FakeSource {
        name: &'static str,
        services: Vec<Service>,
        result: anyhow::Result<Vec<FetchedTitle>>,
    }
    #[async_trait]
    impl CatalogueSource for FakeSource {
        fn name(&self) -> &'static str {
            self.name
        }
        fn services(&self) -> &'static [Service] {
            // leak a 'static slice for the test
            Box::leak(self.services.clone().into_boxed_slice())
        }
        async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>> {
            match &self.result {
                Ok(v) => Ok(v.clone()),
                Err(e) => Err(anyhow::anyhow!("{e}")),
            }
        }
    }

    fn title(imdb: &str, services: Vec<Service>) -> FetchedTitle {
        FetchedTitle {
            imdb_id: Some(imdb.into()),
            tmdb_id: None,
            plex_guid: None,
            title: imdb.into(),
            year: Some(2020),
            kind: TitleKind::Movie,
            score: None,
            length: None,
            description: None,
            genres: vec!["action".into()],
            cast: vec![],
            services,
            plex_rating_key: None,
            links: vec![],
            poster: None,
            backdrop: None,
        }
    }

    async fn pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (init_pool(&url).await.unwrap(), dir)
    }

    #[tokio::test]
    async fn run_sync_replaces_seed_with_fetched_titles() {
        let (p, _dir) = pool().await;
        seed_if_empty(&p).await.unwrap();
        let plex = Arc::new(FakeSource {
            name: "plex",
            services: vec![Service::Plex],
            result: Ok(vec![title("tt100", vec![Service::Plex])]),
        }) as Arc<dyn CatalogueSource>;
        run_sync(&p, &[plex], None).await.unwrap();
        let imdbs: Vec<String> = sqlx::query_scalar("SELECT imdb_id FROM titles")
            .fetch_all(&p)
            .await
            .unwrap();
        assert_eq!(
            imdbs,
            vec!["tt100".to_string()],
            "seed pruned, fetched title remains"
        );
    }

    #[tokio::test]
    async fn run_sync_with_empty_ok_source_completes() {
        let (p, _dir) = pool().await;
        let empty = Arc::new(FakeSource {
            name: "plex",
            services: vec![Service::Plex],
            result: Ok(vec![]),
        }) as Arc<dyn CatalogueSource>;
        // Must return Ok (not hang/panic) when an ok source yields zero titles.
        run_sync(&p, &[empty], None).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM titles")
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn failed_source_does_not_prune_its_services() {
        let (p, _dir) = pool().await;
        // Pre-populate a crunchyroll title via a successful MOTN-like run.
        let motn_ok = Arc::new(FakeSource {
            name: "motn",
            services: vec![Service::Disney, Service::Crunchyroll],
            result: Ok(vec![title("ttC", vec![Service::Crunchyroll])]),
        }) as Arc<dyn CatalogueSource>;
        run_sync(&p, &[motn_ok], None).await.unwrap();
        // Now run with a Plex success + MOTN failure.
        let plex = Arc::new(FakeSource {
            name: "plex",
            services: vec![Service::Plex],
            result: Ok(vec![title("ttP", vec![Service::Plex])]),
        }) as Arc<dyn CatalogueSource>;
        let motn_fail = Arc::new(FakeSource {
            name: "motn",
            services: vec![Service::Disney, Service::Crunchyroll],
            result: Err(anyhow::anyhow!("down")),
        }) as Arc<dyn CatalogueSource>;
        run_sync(&p, &[plex, motn_fail], None).await.unwrap();
        let imdbs: Vec<String> = sqlx::query_scalar("SELECT imdb_id FROM titles ORDER BY imdb_id")
            .fetch_all(&p)
            .await
            .unwrap();
        assert_eq!(
            imdbs,
            vec!["ttC".to_string(), "ttP".to_string()],
            "crunchyroll title survives MOTN outage"
        );
        // sync_runs records an error for the failed services.
        let errs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_runs WHERE status = 'error'")
            .fetch_one(&p)
            .await
            .unwrap();
        assert!(errs >= 1);
    }

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
            .execute(&p)
            .await
            .unwrap();
        let plex = Arc::new(WatchFake {
            name: "plex",
            services: vec![Service::Plex],
            fetch: Ok(vec![title("ttP", vec![Service::Plex])]),
            watch_source: Some("plex"),
            watch: vec![WatchRecord {
                key: "ttW".into(),
                watched_at: Some(1_600_000_000),
            }],
        }) as Arc<dyn CatalogueSource>;
        run_sync(&p, &[plex], None).await.unwrap();

        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT imdb_id FROM watch_history WHERE source = 'plex' ORDER BY imdb_id",
        )
        .fetch_all(&p)
        .await
        .unwrap();
        assert_eq!(rows, vec!["ttW".to_string()]); // stale replaced
    }

    #[tokio::test]
    async fn failed_source_does_not_wipe_watch_history() {
        let (p, _dir) = pool().await;
        sqlx::query("INSERT INTO watch_history (imdb_id, source) VALUES ('ttOLD','plex')")
            .execute(&p)
            .await
            .unwrap();
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
            .execute(&p)
            .await
            .unwrap();
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

    /// A fake source that returns links on fetch and key/value pairs from `server_meta`.
    struct MetaFake {
        name: &'static str,
        services: Vec<Service>,
        titles: Vec<FetchedTitle>,
        meta: anyhow::Result<Vec<(String, String)>>,
    }
    #[async_trait]
    impl CatalogueSource for MetaFake {
        fn name(&self) -> &'static str {
            self.name
        }
        fn services(&self) -> &'static [Service] {
            Box::leak(self.services.clone().into_boxed_slice())
        }
        async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>> {
            Ok(self.titles.clone())
        }
        async fn server_meta(&self) -> anyhow::Result<Vec<(String, String)>> {
            match &self.meta {
                Ok(v) => Ok(v.clone()),
                Err(e) => Err(anyhow::anyhow!("{e}")),
            }
        }
    }

    #[tokio::test]
    async fn run_sync_persists_service_link_end_to_end() {
        let (p, _dir) = pool().await;
        // A MOTN-like source that emits a Crunchyroll link for one title.
        let motn = Arc::new(MetaFake {
            name: "motn",
            services: vec![Service::Disney, Service::Crunchyroll],
            titles: vec![{
                let mut t = title("ttCR", vec![Service::Crunchyroll]);
                t.links = vec![(
                    Service::Crunchyroll,
                    "https://crunchyroll.com/watch/ttCR".into(),
                )];
                t
            }],
            meta: Ok(vec![]),
        }) as Arc<dyn CatalogueSource>;
        run_sync(&p, &[motn], None).await.unwrap();

        // The link must be persisted in title_services.link for Crunchyroll.
        let link: Option<String> = sqlx::query_scalar(
            "SELECT ts.link FROM title_services ts
             JOIN titles t ON t.id = ts.title_id
             WHERE t.imdb_id = 'ttCR' AND ts.service = 'crunchyroll'",
        )
        .fetch_one(&p)
        .await
        .unwrap();
        assert_eq!(
            link.as_deref(),
            Some("https://crunchyroll.com/watch/ttCR"),
            "Crunchyroll link must be persisted after sync"
        );
    }

    #[tokio::test]
    async fn run_sync_persists_server_meta() {
        let (p, _dir) = pool().await;
        // A source that returns a server_meta pair (simulates Plex machineIdentifier).
        let src = Arc::new(MetaFake {
            name: "plex",
            services: vec![Service::Plex],
            titles: vec![title("ttP2", vec![Service::Plex])],
            meta: Ok(vec![("plex_machine_id".into(), "ABC123".into())]),
        }) as Arc<dyn CatalogueSource>;
        run_sync(&p, &[src], None).await.unwrap();

        let val: Option<String> =
            sqlx::query_scalar("SELECT value FROM app_meta WHERE key = 'plex_machine_id'")
                .fetch_optional(&p)
                .await
                .unwrap();
        assert_eq!(
            val.as_deref(),
            Some("ABC123"),
            "server_meta pair must land in app_meta"
        );
    }

    #[tokio::test]
    async fn run_sync_server_meta_error_is_non_fatal() {
        let (p, _dir) = pool().await;
        // A source whose server_meta() fails — sync must still return Ok.
        let src = Arc::new(MetaFake {
            name: "plex",
            services: vec![Service::Plex],
            titles: vec![title("ttP3", vec![Service::Plex])],
            meta: Err(anyhow::anyhow!("identity endpoint down")),
        }) as Arc<dyn CatalogueSource>;
        // Must not propagate the server_meta error.
        run_sync(&p, &[src], None).await.unwrap();
        // Title still persisted — sync core was unaffected.
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM titles WHERE imdb_id = 'ttP3'")
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(n, 1, "title persisted despite server_meta failure");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runner_guard_rejects_concurrent_start() {
        use std::sync::atomic::{AtomicBool, Ordering};

        // A source whose fetch() blocks until released, so the first run is
        // GUARANTEED in flight when we attempt the second start. An empty-source
        // run can finish before the second `try_start` on a fast multi-thread
        // runtime (which made a timing-based version flaky in CI), so gate it.
        struct BlockingSource {
            gate: Arc<AtomicBool>,
        }
        #[async_trait]
        impl CatalogueSource for BlockingSource {
            fn name(&self) -> &'static str {
                "block"
            }
            fn services(&self) -> &'static [Service] {
                &[Service::Plex]
            }
            async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>> {
                while !self.gate.load(Ordering::Acquire) {
                    tokio::task::yield_now().await;
                }
                Ok(Vec::new())
            }
        }

        let (p, _dir) = pool().await;
        let gate = Arc::new(AtomicBool::new(false));
        let src = Arc::new(BlockingSource { gate: gate.clone() }) as Arc<dyn CatalogueSource>;
        let runner = SyncRunner::new(p, vec![src], None, None);

        // First start is accepted; its fetch() now blocks on the gate.
        assert!(runner.try_start(), "first start accepted");
        // A second start is rejected while the first run is in flight —
        // deterministic, since the first run cannot finish until we release.
        assert!(
            !runner.try_start(),
            "second start rejected while a run is in flight"
        );
        // Release the gate; the guard must clear once the run completes. Wait in
        // real time (not a yield-spin, which finishes far faster than the spawned
        // run's actual DB I/O) up to a generous bound.
        gate.store(true, Ordering::Release);
        for _ in 0..5_000 {
            if !runner.is_running() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        assert!(!runner.is_running(), "guard resets after the run completes");
    }
}
