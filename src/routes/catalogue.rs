use actix_web::{web, HttpResponse, Responder};
use sqlx::SqlitePool;

use crate::db::catalogue::{fetch_catalogue, fetch_title};
use crate::routes::watch::WatchConfig;

pub async fn get_catalogue(pool: web::Data<SqlitePool>) -> impl Responder {
    match fetch_catalogue(pool.get_ref()).await {
        Ok(titles) => HttpResponse::Ok().json(titles),
        Err(e) => {
            tracing::error!("catalogue fetch failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

pub async fn get_title(
    pool: web::Data<SqlitePool>,
    path: web::Path<i64>,
    watch_cfg: Option<web::Data<WatchConfig>>,
) -> impl Responder {
    let id = path.into_inner();
    match fetch_title(pool.get_ref(), id).await {
        Ok(Some(mut dto)) => {
            // The Plex redirect needs PLEX_WEB_URL; without it the button would 404.
            if watch_cfg.and_then(|c| c.plex_web_url.clone()).is_none() {
                dto.watchable.retain(|s| s != "plex");
            }
            HttpResponse::Ok().json(dto)
        }
        Ok(None) => HttpResponse::NotFound().finish(),
        Err(e) => {
            tracing::error!("title detail fetch failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{test, web, App};
    use serde_json::Value;
    use sqlx::SqlitePool;

    use crate::db::{init_pool, seed::seed_if_empty};
    use crate::routes;

    async fn seeded_pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        seed_if_empty(&pool).await.unwrap();
        (pool, dir)
    }

    #[actix_web::test]
    async fn title_detail_returns_full_record() {
        let (pool, _dir) = seeded_pool().await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .configure(routes::configure),
        )
        .await;

        // id 1 exists in the seed
        let req = test::TestRequest::get().uri("/api/titles/1").to_request();
        let body: Value = test::call_and_read_body_json(&app, req).await;

        assert_eq!(body["id"], 1);
        assert!(body["desc"].is_string(), "detail must carry desc");
        assert!(body["cast"].is_array(), "detail must carry cast");
        assert!(body["services"].is_array());
    }

    #[actix_web::test]
    async fn title_detail_unknown_id_is_404() {
        let (pool, _dir) = seeded_pool().await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .configure(routes::configure),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/api/titles/999999")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn catalogue_endpoint_returns_seed() {
        let (pool, _dir) = seeded_pool().await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .configure(routes::configure),
        )
        .await;

        let req = test::TestRequest::get().uri("/api/catalogue").to_request();
        let body: Value = test::call_and_read_body_json(&app, req).await;

        let arr = body.as_array().unwrap();
        assert_eq!(arr.len(), 28);
        let first = &arr[0];
        assert!(first["id"].is_number());
        assert!(first["type"] == "movie" || first["type"] == "series");
        assert!(first["services"].is_array());
        assert!(first["genres"].is_array());
        assert!(
            first.get("desc").is_none(),
            "list payload must not carry desc"
        );
        assert!(
            first.get("cast").is_none(),
            "list payload must not carry cast"
        );
    }
}
