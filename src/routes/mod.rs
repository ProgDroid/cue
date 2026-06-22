pub mod ask;
pub mod catalogue;

use actix_web::{web, HttpResponse};

async fn health() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({ "status": "ok" }))
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api")
            .route("/health", web::get().to(health))
            .route("/catalogue", web::get().to(catalogue::get_catalogue))
            .route("/ask", web::post().to(ask::ask))
            .route("/ask/similar", web::post().to(ask::similar))
            .route("/ask/refine", web::post().to(ask::refine)),
    );
}
