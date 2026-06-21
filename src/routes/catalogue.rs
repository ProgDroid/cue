use actix_web::{web, HttpResponse, Responder};
use sqlx::SqlitePool;

use crate::db::catalogue::fetch_catalogue;

pub async fn get_catalogue(pool: web::Data<SqlitePool>) -> impl Responder {
    match fetch_catalogue(pool.get_ref()).await {
        Ok(titles) => HttpResponse::Ok().json(titles),
        Err(e) => {
            tracing::error!("catalogue fetch failed: {e:#}");
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
    }
}
