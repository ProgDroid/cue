//! Sync trigger + status endpoints.

use std::sync::Arc;

use actix_web::{web, HttpResponse, Responder};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::db::motn_meta::{self, MotnStatus};
use crate::db::sync_runs::{self, CatalogueStats, SourceRun};
use crate::sync::SyncRunner;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LastRun {
    status: String,
    item_count: i64,
    finished_at: Option<String>,
}

/// Country code the MOTN client queries; registered as app data so the status
/// endpoint can read the matching per-country bookkeeping keys.
#[derive(Debug, Clone)]
pub struct MotnCountry(pub String);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusBody {
    running: bool,
    last_run: Option<LastRun>,
    sources: Vec<SourceRun>,
    catalogue: CatalogueStats,
    motn: Option<MotnStatus>,
}

/// Reduce per-source statuses to one overall status.
fn overall(sources: &[SourceRun]) -> Option<LastRun> {
    if sources.is_empty() {
        return None;
    }
    let status = if sources.iter().all(|s| s.status == "ok") {
        "ok"
    } else if sources.iter().all(|s| s.status == "error") {
        "error"
    } else {
        "partial"
    };
    let item_count = sources.iter().map(|s| s.item_count).sum();
    let finished_at = sources.iter().filter_map(|s| s.last_run.clone()).max();
    Some(LastRun {
        status: status.to_string(),
        item_count,
        finished_at,
    })
}

/// `POST /api/sync` — start a background sync (202) or report one running (409).
pub async fn trigger(runner: web::Data<Arc<SyncRunner>>) -> impl Responder {
    if runner.try_start() {
        HttpResponse::Accepted().json(serde_json::json!({ "started": true }))
    } else {
        HttpResponse::Conflict().json(serde_json::json!({ "running": true }))
    }
}

/// `GET /api/sync/status` — latest per-source runs + catalogue stats.
pub async fn status(
    pool: web::Data<SqlitePool>,
    runner: web::Data<Arc<SyncRunner>>,
    country: Option<web::Data<MotnCountry>>,
) -> impl Responder {
    let sources = match sync_runs::latest_per_source(pool.get_ref()).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("sync status sources failed: {e:#}");
            return HttpResponse::InternalServerError().finish();
        }
    };
    let catalogue = match sync_runs::catalogue_stats(pool.get_ref()).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("sync status stats failed: {e:#}");
            return HttpResponse::InternalServerError().finish();
        }
    };
    // `None` only when MOTN is not configured; a configured-but-never-succeeded
    // MOTN still reports its counts.
    let motn = if runner.has_source("motn") {
        let country = country.as_ref().map(|c| c.0.as_str());
        match motn_meta::status(pool.get_ref(), country).await {
            Ok(m) => Some(m),
            Err(e) => {
                tracing::error!("sync status motn failed: {e:#}");
                return HttpResponse::InternalServerError().finish();
            }
        }
    } else {
        None
    };
    let last_run = overall(&sources);
    HttpResponse::Ok().json(StatusBody {
        running: runner.is_running(),
        last_run,
        sources,
        catalogue,
        motn,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{app_meta, init_pool, motn_meta, sync_runs};
    use crate::sync::SyncRunner;
    use actix_web::{test, web, App};

    async fn pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (init_pool(&url).await.unwrap(), dir)
    }

    #[actix_web::test]
    async fn status_reports_sources_and_stats() {
        let (p, _dir) = pool().await;
        sync_runs::record(&p, "plex", "ok", 3, None).await.unwrap();
        let runner = SyncRunner::new(p.clone(), vec![], None, None);
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(p.clone()))
                .app_data(web::Data::new(runner))
                .route("/api/sync/status", web::get().to(status)),
        )
        .await;
        let req = test::TestRequest::get()
            .uri("/api/sync/status")
            .to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["running"], false);
        assert_eq!(body["catalogue"]["titles"], 0);
        assert_eq!(body["sources"][0]["source"], "plex");
        assert_eq!(body["lastRun"]["status"], "ok");
    }

    struct FakeMotn;
    #[async_trait::async_trait]
    impl crate::sync::CatalogueSource for FakeMotn {
        fn name(&self) -> &'static str {
            "motn"
        }
        fn services(&self) -> &'static [crate::models::Service] {
            &[]
        }
        async fn fetch(&self) -> anyhow::Result<Vec<crate::sync::FetchedTitle>> {
            Ok(vec![])
        }
    }

    #[allow(clippy::future_not_send)] // actix test service is !Send
    async fn get_status(
        p: &sqlx::SqlitePool,
        with_motn: bool,
        country: Option<&str>,
    ) -> serde_json::Value {
        let sources: Vec<Arc<dyn crate::sync::CatalogueSource>> = if with_motn {
            vec![Arc::new(FakeMotn)]
        } else {
            vec![]
        };
        let runner = SyncRunner::new(p.clone(), sources, None, None);
        let mut app = App::new()
            .app_data(web::Data::new(p.clone()))
            .app_data(web::Data::new(runner))
            .route("/api/sync/status", web::get().to(status));
        if let Some(c) = country {
            app = app.app_data(web::Data::new(MotnCountry(c.to_string())));
        }
        let app = test::init_service(app).await;
        let req = test::TestRequest::get()
            .uri("/api/sync/status")
            .to_request();
        test::call_and_read_body_json(&app, req).await
    }

    async fn insert_cache_row(p: &sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO motn_catalog_cache (show_id, payload, updated_at)
             VALUES ('1', '{}', datetime('now'))",
        )
        .execute(p)
        .await
        .unwrap();
    }

    #[actix_web::test]
    async fn status_motn_is_null_when_not_configured() {
        let (p, _dir) = pool().await;
        let body = get_status(&p, false, None).await;
        assert!(body["motn"].is_null());
    }

    #[actix_web::test]
    async fn status_reports_motn_after_a_failed_first_seed() {
        let (p, _dir) = pool().await;
        motn_meta::set_i64(&p, motn_meta::SEED_FAILED_AT, motn_meta::now_unix() - 60)
            .await
            .unwrap();
        motn_meta::increment_requests(&p).await.unwrap();
        let body = get_status(&p, true, None).await;
        assert_eq!(body["motn"]["requestsThisMonth"], 1);
        assert!(body["motn"]["seedFailedAt"].is_number());
        assert!(body["motn"]["lastMode"].is_null());
        assert_eq!(body["motn"]["cacheSize"], 0);
    }

    #[actix_web::test]
    async fn status_reports_motn_state() {
        let (p, _dir) = pool().await;
        app_meta::set(&p, motn_meta::LAST_MODE, "delta")
            .await
            .unwrap();
        motn_meta::set_i64(&p, motn_meta::LAST_SEED_AT, 1_757_635_200)
            .await
            .unwrap();
        motn_meta::set_i64(&p, motn_meta::SEED_FAILED_AT, 0)
            .await
            .unwrap();
        motn_meta::set_i64(&p, &motn_meta::catalogs_checked_key("gb"), 1_757_000_000)
            .await
            .unwrap();
        motn_meta::increment_requests(&p).await.unwrap();
        motn_meta::increment_requests(&p).await.unwrap();
        insert_cache_row(&p).await;
        let body = get_status(&p, true, Some("gb")).await;
        let m = &body["motn"];
        assert_eq!(m["lastMode"], "delta");
        assert_eq!(m["lastSeedAt"], 1_757_635_200);
        assert_eq!(m["requestsThisMonth"], 2);
        assert_eq!(m["monthlyLimit"], 500);
        assert_eq!(m["cacheSize"], 1);
        assert!(m["seedFailedAt"].is_null());
        assert_eq!(m["catalogsCheckedAt"], 1_757_000_000);
    }

    #[actix_web::test]
    async fn status_catalogs_checked_at_is_null_without_registered_country() {
        let (p, _dir) = pool().await;
        motn_meta::set_i64(&p, &motn_meta::catalogs_checked_key("gb"), 1_757_000_000)
            .await
            .unwrap();
        let body = get_status(&p, true, None).await;
        assert!(body["motn"]["catalogsCheckedAt"].is_null());
    }

    #[actix_web::test]
    async fn trigger_starts_and_returns_202() {
        let (p, _dir) = pool().await;
        let runner = SyncRunner::new(p.clone(), vec![], None, None);
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(runner))
                .route("/api/sync", web::post().to(trigger)),
        )
        .await;
        let req = test::TestRequest::post().uri("/api/sync").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 202);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["started"], true);
    }

    mod overall_tests {
        use super::super::{overall, SourceRun};

        fn sr(status: &str, count: i64) -> SourceRun {
            SourceRun {
                source: "x".into(),
                last_run: Some("2026-06-22".into()),
                status: status.into(),
                item_count: count,
            }
        }

        #[test]
        fn overall_none_when_empty() {
            assert!(overall(&[]).is_none());
        }

        #[test]
        fn overall_all_ok_sums_counts() {
            let r = overall(&[sr("ok", 3), sr("ok", 2)]).unwrap();
            assert_eq!(r.status, "ok");
            assert_eq!(r.item_count, 5);
        }

        #[test]
        fn overall_all_error() {
            assert_eq!(
                overall(&[sr("error", 0), sr("error", 0)]).unwrap().status,
                "error"
            );
        }

        #[test]
        fn overall_mixed_is_partial() {
            assert_eq!(
                overall(&[sr("ok", 3), sr("error", 0)]).unwrap().status,
                "partial"
            );
        }

        #[test]
        fn overall_unknown_status_is_partial() {
            assert_eq!(overall(&[sr("running", 0)]).unwrap().status, "partial");
        }
    }
}
