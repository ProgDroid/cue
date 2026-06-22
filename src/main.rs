use std::sync::Arc;

use actix_web::{web, App, HttpServer};

use cue::config::Config;
use cue::services::anthropic::ClaudeAskModel;
use cue::services::ask_engine::AskEngine;
use cue::services::embeddings::{backfill, build_light_axis, OpenAiEmbedder};
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

    // Build optional external clients from server-side keys (never sent to Vue).
    let embedder: Option<Arc<dyn cue::services::embeddings::Embedder>> = cfg
        .openai_api_key
        .clone()
        .map(|k| Arc::new(OpenAiEmbedder::new(k)) as Arc<dyn cue::services::embeddings::Embedder>);
    let model: Option<Arc<dyn cue::services::anthropic::AskModel>> = cfg
        .anthropic_api_key
        .clone()
        .map(|k| Arc::new(ClaudeAskModel::new(k)) as Arc<dyn cue::services::anthropic::AskModel>);

    // Backfill embeddings + compute the lightness axis when an embedder exists.
    let light_axis = if let Some(emb) = embedder.as_deref() {
        match backfill(&pool, emb).await {
            Ok(n) => tracing::info!("embedded {n} titles"),
            Err(e) => tracing::error!("embedding backfill failed: {e:#}"),
        }
        build_light_axis(emb).await.ok()
    } else {
        tracing::warn!(
            "OPENAI_API_KEY unset — ask retrieval disabled, refine 'shorter' still works"
        );
        None
    };

    let engine = Arc::new(AskEngine::new(pool.clone(), embedder, model, light_axis));

    let static_dir = cfg.static_dir.clone();
    let bind_addr = cfg.bind_addr.clone();
    tracing::info!("listening on {bind_addr}");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(engine.clone()))
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
