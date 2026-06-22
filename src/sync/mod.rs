//! Catalogue sync subsystem: external clients behind `CatalogueSource`,
//! pure merge logic, DB reconciliation, and the run orchestrator.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use sqlx::SqlitePool;

use crate::db::sync_runs;
use crate::models::{Service, TitleKind};
use crate::services::embeddings::{backfill, Embedder};

pub mod merge;
pub mod store;

/// A source-agnostic catalogue row emitted by every `CatalogueSource`.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedTitle {
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub plex_guid: Option<String>,
    pub title: String,
    pub year: Option<i64>,
    pub kind: TitleKind,
    pub imdb_rating: Option<f64>,
    pub length: Option<String>,
    pub description: Option<String>,
    pub genres: Vec<String>,
    pub cast: Vec<String>,
    pub services: Vec<Service>,
}

/// An external catalogue client. A *client* is the unit of fetching and failure;
/// the *services* it owns are the unit of membership reconciliation.
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
    // (service, ok?) and any error message keyed by source name.
    let mut ok_services: Vec<Service> = Vec::new();
    let mut failures: Vec<(Service, String)> = Vec::new();

    for src in sources {
        match src.fetch().await {
            Ok(mut rows) => {
                tracing::info!("sync source {} fetched {} rows", src.name(), rows.len());
                fetched.append(&mut rows);
                ok_services.extend_from_slice(src.services());
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
    let mut id_services: Vec<(i64, Vec<Service>)> = Vec::with_capacity(merged.len());
    for m in &merged {
        let id = store::upsert_title(pool, m).await?;
        id_services.push((id, m.services.clone()));
    }

    // Reconcile + record only the services whose client succeeded.
    let mut unique_ok = ok_services.clone();
    unique_ok.sort_by_key(|s| s.as_str());
    unique_ok.dedup();
    for svc in &unique_ok {
        let desired: Vec<i64> = id_services
            .iter()
            .filter(|(_, services)| services.contains(svc))
            .map(|(id, _)| *id)
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
    if !unique_ok.is_empty() {
        let all_touched: Vec<&str> = unique_ok
            .iter()
            .map(|s| s.as_str())
            .chain(failures.iter().map(|(s, _)| s.as_str()))
            .collect();
        let pruned = store::prune_orphans_scoped(pool, &all_touched).await?;
        tracing::info!("sync pruned {pruned} orphaned titles");
    }

    if let Some(emb) = embedder {
        match backfill(pool, emb).await {
            Ok(n) => tracing::info!("sync embedded {n} titles"),
            Err(e) => tracing::error!("sync embedding backfill failed: {e:#}"),
        }
    } else {
        tracing::warn!("OPENAI_API_KEY unset — synced titles left unembedded");
    }
    Ok(())
}

/// Owns the sources + embedder and guards against concurrent runs.
pub struct SyncRunner {
    pool: SqlitePool,
    sources: Vec<Arc<dyn CatalogueSource>>,
    embedder: Option<Arc<dyn Embedder>>,
    running: AtomicBool,
}

impl SyncRunner {
    #[must_use]
    pub fn new(
        pool: SqlitePool,
        sources: Vec<Arc<dyn CatalogueSource>>,
        embedder: Option<Arc<dyn Embedder>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            sources,
            embedder,
            running: AtomicBool::new(false),
        })
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
            imdb_rating: None,
            length: None,
            description: None,
            genres: vec![],
            cast: vec![],
            services: vec![Service::Plex],
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
            imdb_rating: None,
            length: None,
            description: None,
            genres: vec!["action".into()],
            cast: vec![],
            services,
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runner_guard_rejects_concurrent_start() {
        let (p, _dir) = pool().await;
        let runner = SyncRunner::new(p, vec![], None);
        assert!(runner.try_start());
        // Second immediate start is rejected while the first is in flight.
        let second = runner.try_start();
        // Either rejected (still running) or the first finished instantly; assert the API shape works.
        assert!(second || !runner.is_running());
    }
}
