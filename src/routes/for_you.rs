//! `GET /api/for-you`: ranked "For you" title ids.

use std::sync::Arc;

use actix_web::{web, HttpResponse};
use sqlx::SqlitePool;

use crate::services::for_you::ForYouService;

pub async fn get_for_you(
    pool: web::Data<SqlitePool>,
    svc: web::Data<Arc<ForYouService>>,
) -> HttpResponse {
    match svc.get(&pool).await {
        Ok(result) => HttpResponse::Ok().json(result),
        Err(e) => {
            tracing::error!("for-you failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use actix_web::{test, web, App};
    use sqlx::SqlitePool;

    use crate::db::{embeddings, init_pool, user_data};
    use crate::services::embeddings::EMBED_MODEL;
    use crate::services::for_you::ForYouService;

    async fn fresh_pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (init_pool(&url).await.unwrap(), dir)
    }

    async fn insert_title(pool: &SqlitePool, imdb: &str, vector: Option<&[f32]>) -> i64 {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO titles (imdb_id, title, year, type) VALUES (?, 'T', 2020, 'movie') RETURNING id",
        )
        .bind(imdb)
        .fetch_one(pool)
        .await
        .unwrap();
        if let Some(v) = vector {
            embeddings::upsert(pool, id, v, EMBED_MODEL).await.unwrap();
        }
        id
    }

    #[allow(clippy::future_not_send)] // actix test service is !Send
    async fn get_for_you(pool: SqlitePool) -> serde_json::Value {
        let svc = Arc::new(ForYouService::new());
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .app_data(web::Data::new(svc))
                .route("/api/for-you", web::get().to(super::get_for_you)),
        )
        .await;
        let resp = test::call_service(
            &app,
            test::TestRequest::get().uri("/api/for-you").to_request(),
        )
        .await;
        assert_eq!(resp.status(), 200);
        test::read_body_json(resp).await
    }

    #[actix_web::test]
    async fn for_you_ranks_unwatched_unrated_titles() {
        let (pool, _dir) = fresh_pool().await;
        for (imdb, v) in [
            ("tt1", [1.0, 0.0, 0.0]),
            ("tt2", [0.9, 0.1, 0.0]),
            ("tt3", [0.9, -0.1, 0.0]),
        ] {
            insert_title(&pool, imdb, Some(&v)).await;
            user_data::set_rating(&pool, imdb, 9).await.unwrap();
        }
        let closer = insert_title(&pool, "tt4", Some(&[0.8, 0.05, 0.0])).await;
        let farther = insert_title(&pool, "tt5", Some(&[0.0, 1.0, 0.0])).await;
        let watched = insert_title(&pool, "tt6", Some(&[0.85, 0.0, 0.0])).await;
        user_data::set_watched(&pool, "tt6", true).await.unwrap();

        let body = get_for_you(pool).await;
        assert_eq!(body["basis"], 3);
        assert_eq!(body["ids"], serde_json::json!([closer, farther]));
        assert!(!body["ids"].as_array().unwrap().contains(&watched.into()));
    }

    #[actix_web::test]
    async fn ratings_outside_catalogue_are_ignored() {
        let (pool, _dir) = fresh_pool().await;
        for i in 0..5 {
            user_data::set_rating(&pool, &format!("tt9{i}"), 9)
                .await
                .unwrap();
        }
        for (imdb, v) in [("tt1", [1.0, 0.0]), ("tt2", [0.0, 1.0])] {
            insert_title(&pool, imdb, Some(&v)).await;
            user_data::set_rating(&pool, imdb, 8).await.unwrap();
        }
        let body = get_for_you(pool).await;
        assert_eq!(body["basis"], 2);
        assert_eq!(body["ids"], serde_json::json!([]));
    }

    #[actix_web::test]
    async fn no_embeddings_returns_basis_zero() {
        let (pool, _dir) = fresh_pool().await;
        for imdb in ["tt1", "tt2", "tt3"] {
            insert_title(&pool, imdb, None).await;
            user_data::set_rating(&pool, imdb, 9).await.unwrap();
        }
        let body = get_for_you(pool).await;
        assert_eq!(body, serde_json::json!({ "ids": [], "basis": 0 }));
    }
}
