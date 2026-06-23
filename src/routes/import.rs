use actix_web::{web, HttpResponse, Responder};
use sqlx::SqlitePool;

use crate::db::user_data;
use crate::import::imdb_ratings;

/// `POST /api/import/ratings` — import an `IMDb` ratings-export CSV (raw text body).
pub async fn import_ratings(pool: web::Data<SqlitePool>, body: web::Bytes) -> impl Responder {
    let Ok(csv) = std::str::from_utf8(&body) else {
        return HttpResponse::BadRequest().json(serde_json::json!({ "error": "invalid_utf8" }));
    };
    let parsed = imdb_ratings::parse_ratings(csv);
    if parsed.rows.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({ "error": "no_ratings_found" }));
    }
    match user_data::import_ratings(pool.get_ref(), &parsed.rows).await {
        Ok(outcome) => HttpResponse::Ok().json(serde_json::json!({
            "imported": outcome.imported,
            "skipped": parsed.skipped,
            "matched": outcome.matched,
        })),
        Err(e) => {
            tracing::error!("import_ratings failed: {e:#}");
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

    fn test_routes(cfg: &mut web::ServiceConfig) {
        cfg.route("/api/import/ratings", web::post().to(import_ratings));
    }

    #[actix_web::test]
    async fn import_happy_path_returns_summary() {
        let (pool, _dir) = fresh_pool().await;
        sqlx::query(
            "INSERT INTO titles (imdb_id, title, year, type) VALUES ('tt0111161','T',1994,'movie')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .configure(test_routes),
        )
        .await;

        let csv = "Const,Your Rating,Date Rated,Title\n\
tt0111161,10,2019-03-14,The Shawshank Redemption\n\
tt0137523,9,2020-01-02,Fight Club\n\
tt0000002,11,2021-05-06,Bad High\n";
        let req = test::TestRequest::post()
            .uri("/api/import/ratings")
            .insert_header(("content-type", "text/csv"))
            .set_payload(csv)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["imported"], 2); // 10 and 9 valid; 11 skipped
        assert_eq!(body["skipped"], 1);
        assert_eq!(body["matched"], 1); // only tt0111161 is in titles
    }

    #[actix_web::test]
    async fn empty_body_is_400_no_ratings_found() {
        let (pool, _dir) = fresh_pool().await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .configure(test_routes),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/api/import/ratings")
            .insert_header(("content-type", "text/csv"))
            .set_payload("")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["error"], "no_ratings_found");
    }
}
