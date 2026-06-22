//! Sync trigger + status endpoints.

use std::sync::Arc;

use actix_web::{web, HttpResponse, Responder};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::db::sync_runs::{self, CatalogueStats, SourceRun};
use crate::sync::SyncRunner;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LastRun {
    status: String,
    item_count: i64,
    finished_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusBody {
    running: bool,
    last_run: Option<LastRun>,
    sources: Vec<SourceRun>,
    catalogue: CatalogueStats,
}

/// Reduce per-source statuses to one overall status.
fn overall(sources: &[SourceRun]) -> Option<LastRun> {
    if sources.is_empty() {
        return None;
    }
    let any_err = sources.iter().any(|s| s.status == "error");
    let any_ok = sources.iter().any(|s| s.status == "ok");
    let status = if any_err && any_ok {
        "partial"
    } else if any_err {
        "error"
    } else {
        "ok"
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
    let last_run = overall(&sources);
    HttpResponse::Ok().json(StatusBody {
        running: runner.is_running(),
        last_run,
        sources,
        catalogue,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{init_pool, sync_runs};
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
        let runner = SyncRunner::new(p.clone(), vec![], None);
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

    #[actix_web::test]
    async fn trigger_starts_and_returns_202() {
        let (p, _dir) = pool().await;
        let runner = SyncRunner::new(p.clone(), vec![], None);
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(runner))
                .route("/api/sync", web::post().to(trigger)),
        )
        .await;
        let req = test::TestRequest::post().uri("/api/sync").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 202);
    }
}
