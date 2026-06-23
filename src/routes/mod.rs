pub mod ask;
pub mod catalogue;
pub mod images;
pub mod sync;
pub mod user_data;

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
            .route("/ask/refine", web::post().to(ask::refine))
            .route("/sync", web::post().to(sync::trigger))
            .route("/sync/status", web::get().to(sync::status))
            .route("/titles/{id}/rating", web::put().to(user_data::set_rating))
            .route(
                "/titles/{id}/rating",
                web::delete().to(user_data::clear_rating),
            )
            .route(
                "/titles/{id}/watched",
                web::put().to(user_data::set_watched),
            )
            .route("/titles/{id}/poster", web::get().to(images::poster))
            .route("/titles/{id}/backdrop", web::get().to(images::backdrop)),
    );
}
