pub mod ask;
pub mod catalogue;
pub mod images;
pub mod import;
pub mod sync;
pub mod user_data;
pub mod watch;

use actix_web::{web, HttpResponse};

async fn health() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({ "status": "ok" }))
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api")
            .route("/health", web::get().to(health))
            .route("/catalogue", web::get().to(catalogue::get_catalogue))
            .route("/titles/{id}", web::get().to(catalogue::get_title))
            .route("/ask", web::post().to(ask::ask))
            .route("/ask/similar", web::post().to(ask::similar))
            .route("/ask/refine", web::post().to(ask::refine))
            .route("/sync", web::post().to(sync::trigger))
            .route("/sync/status", web::get().to(sync::status))
            .service(
                // Shared JsonConfig renders malformed bodies as {"error":…}
                // JSON (matching the domain-validation error shape) instead of
                // actix's default plain-text 400.
                web::resource("/titles/{id}/rating")
                    .app_data(user_data::rating_json_config())
                    .route(web::put().to(user_data::set_rating))
                    .route(web::delete().to(user_data::clear_rating)),
            )
            .service(
                web::resource("/titles/{id}/watched")
                    .app_data(user_data::rating_json_config())
                    .route(web::put().to(user_data::set_watched)),
            )
            .route(
                "/titles/{id}/watch/{service}",
                web::get().to(watch::redirect),
            )
            .route("/titles/{id}/poster", web::get().to(images::poster))
            .route("/titles/{id}/backdrop", web::get().to(images::backdrop))
            .service(
                // Raise the Bytes payload cap above actix's 256 KB default; an
                // IMDb export of several thousand rows can exceed it.
                web::resource("/import/ratings")
                    .app_data(web::PayloadConfig::new(8 * 1024 * 1024))
                    .route(web::post().to(import::import_ratings)),
            ),
    );
}
