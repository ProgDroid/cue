use actix_web::{web, App, HttpServer};

use cue::config::Config;
use cue::static_files::{serve_spa, StaticDir};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let _ = dotenvy::dotenv();

    let cfg = Config::from_env();

    if let Some(path) = cfg.sqlite_path() {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
    }

    let pool = cue::db::init_pool(&cfg.database_url)
        .await
        .map_err(std::io::Error::other)?;
    let inserted = cue::db::seed::seed_if_empty(&pool)
        .await
        .map_err(std::io::Error::other)?;
    tracing::info!("seeded {inserted} titles");

    let static_dir = cfg.static_dir.clone();
    let bind_addr = cfg.bind_addr.clone();
    tracing::info!("listening on {bind_addr}");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(StaticDir(static_dir.clone())))
            .configure(cue::routes::configure)
            // The `/api` scope is matched first; everything else (real assets
            // and client routes) falls to the catch-all SPA handler.
            .default_service(web::route().to(serve_spa))
    })
    .bind(bind_addr)?
    .run()
    .await
}
