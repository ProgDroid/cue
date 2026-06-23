use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::db::user_data::{self, KeyLookup};

#[derive(Deserialize)]
pub struct RatingBody {
    pub rating: i64,
}

#[derive(Deserialize)]
pub struct WatchedBody {
    pub watched: bool,
}

/// Resolve the title id to its key, or return the appropriate early response
/// (404 unknown title, 422 no `imdb_id`, 500 on DB error).
async fn resolve_or_respond(pool: &SqlitePool, id: i64) -> Result<String, HttpResponse> {
    match user_data::resolve_key(pool, id).await {
        Ok(KeyLookup::Key(k)) => Ok(k),
        Ok(KeyLookup::NoImdbId) => {
            Err(HttpResponse::UnprocessableEntity()
                .json(serde_json::json!({ "error": "no_imdb_id" })))
        }
        Ok(KeyLookup::NotFound) => Err(HttpResponse::NotFound().finish()),
        Err(e) => {
            tracing::error!("resolve_key failed: {e:#}");
            Err(HttpResponse::InternalServerError().finish())
        }
    }
}

/// `PUT /api/titles/{id}/rating` — set the 1-10 rating.
pub async fn set_rating(
    pool: web::Data<SqlitePool>,
    path: web::Path<i64>,
    body: web::Json<RatingBody>,
) -> impl Responder {
    let rating = body.rating;
    if !(1..=10).contains(&rating) {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "error": "rating must be between 1 and 10" }));
    }
    let key = match resolve_or_respond(pool.get_ref(), path.into_inner()).await {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    match user_data::set_rating(pool.get_ref(), &key, rating).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "rating": rating })),
        Err(e) => {
            tracing::error!("set_rating failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

/// `DELETE /api/titles/{id}/rating` — clear the rating.
pub async fn clear_rating(pool: web::Data<SqlitePool>, path: web::Path<i64>) -> impl Responder {
    let key = match resolve_or_respond(pool.get_ref(), path.into_inner()).await {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    match user_data::clear_rating(pool.get_ref(), &key).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "rating": null })),
        Err(e) => {
            tracing::error!("clear_rating failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

/// `PUT /api/titles/{id}/watched` — set watched state.
pub async fn set_watched(
    pool: web::Data<SqlitePool>,
    path: web::Path<i64>,
    body: web::Json<WatchedBody>,
) -> impl Responder {
    let watched = body.watched;
    let key = match resolve_or_respond(pool.get_ref(), path.into_inner()).await {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    match user_data::set_watched(pool.get_ref(), &key, watched).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "watched": watched })),
        Err(e) => {
            tracing::error!("set_watched failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_pool;
    use actix_web::{test, App};

    async fn fresh_pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        (pool, dir)
    }

    async fn insert_title(pool: &SqlitePool, imdb: Option<&str>) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO titles (imdb_id, title, year, type) VALUES (?, 'T', 2020, 'movie') RETURNING id",
        )
        .bind(imdb).fetch_one(pool).await.unwrap()
    }

    // Register only the three user-data routes — avoids spelling the App<impl
    // ServiceFactory<...>> type and keeps the test app isolated from other
    // handlers' Data deps (SyncRunner, Embedder, AskModel).
    fn test_routes(cfg: &mut web::ServiceConfig) {
        cfg.route("/api/titles/{id}/rating", web::put().to(set_rating))
            .route("/api/titles/{id}/rating", web::delete().to(clear_rating))
            .route("/api/titles/{id}/watched", web::put().to(set_watched));
    }

    #[actix_web::test]
    async fn put_rating_ok() {
        let (pool, _dir) = fresh_pool().await;
        let id = insert_title(&pool, Some("tt100")).await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .configure(test_routes),
        )
        .await;
        let req = test::TestRequest::put()
            .uri(&format!("/api/titles/{id}/rating"))
            .set_json(serde_json::json!({ "rating": 7 }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["rating"], 7);
    }

    #[actix_web::test]
    async fn put_rating_out_of_range_is_400() {
        let (pool, _dir) = fresh_pool().await;
        let id = insert_title(&pool, Some("tt100")).await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .configure(test_routes),
        )
        .await;
        for bad in [0, 11] {
            let req = test::TestRequest::put()
                .uri(&format!("/api/titles/{id}/rating"))
                .set_json(serde_json::json!({ "rating": bad }))
                .to_request();
            assert_eq!(test::call_service(&app, req).await.status(), 400);
        }
    }

    #[actix_web::test]
    async fn put_rating_unknown_title_is_404() {
        let (pool, _dir) = fresh_pool().await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .configure(test_routes),
        )
        .await;
        let req = test::TestRequest::put()
            .uri("/api/titles/4242/rating")
            .set_json(serde_json::json!({ "rating": 5 }))
            .to_request();
        assert_eq!(test::call_service(&app, req).await.status(), 404);
    }

    #[actix_web::test]
    async fn put_rating_null_imdb_is_422() {
        let (pool, _dir) = fresh_pool().await;
        let id = insert_title(&pool, None).await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .configure(test_routes),
        )
        .await;
        let req = test::TestRequest::put()
            .uri(&format!("/api/titles/{id}/rating"))
            .set_json(serde_json::json!({ "rating": 5 }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 422);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["error"], "no_imdb_id");
    }

    #[actix_web::test]
    async fn delete_rating_ok() {
        let (pool, _dir) = fresh_pool().await;
        let id = insert_title(&pool, Some("tt100")).await;
        user_data::set_rating(&pool, "tt100", 8).await.unwrap();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool.clone()))
                .configure(test_routes),
        )
        .await;
        let req = test::TestRequest::delete()
            .uri(&format!("/api/titles/{id}/rating"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert!(body["rating"].is_null());
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_ratings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[actix_web::test]
    async fn put_watched_ok() {
        let (pool, _dir) = fresh_pool().await;
        let id = insert_title(&pool, Some("tt100")).await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .configure(test_routes),
        )
        .await;
        let req = test::TestRequest::put()
            .uri(&format!("/api/titles/{id}/watched"))
            .set_json(serde_json::json!({ "watched": true }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["watched"], true);
    }
}
