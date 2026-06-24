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

    // Fail fast on a bad BIND_ADDR / SYNC_CRON, naming the value — before the
    // pool opens and a startup sync may kick off.
    if let Err(e) = cfg.validate() {
        tracing::error!("invalid configuration: {e}");
        return Err(std::io::Error::other(e));
    }

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

    // Clone before move into AskEngine so the runner can also hold a reference.
    let embedder_for_engine = embedder.clone();
    let engine = Arc::new(AskEngine::new(
        pool.clone(),
        embedder_for_engine,
        model,
        light_axis,
    ));

    // Assemble catalogue sources from configured credentials.
    let mut sources: Vec<Arc<dyn cue::sync::CatalogueSource>> = Vec::new();
    if let (Some(url), Some(token)) = (cfg.plex_url.clone(), cfg.plex_token.clone()) {
        sources.push(Arc::new(cue::sync::plex::PlexClient::new(url, token)));
    }
    if let Some(key) = cfg.motn_api_key.clone() {
        let country = cfg.region.clone().unwrap_or_else(|| "gb".to_string());
        sources.push(Arc::new(cue::sync::motn::MotnClient::new(
            key,
            country,
            pool.clone(),
        )));
    }
    let runner = cue::sync::SyncRunner::new(pool.clone(), sources, embedder, Some(cfg.data_dir()));

    // Sync once on startup until a real sync has ever succeeded (fresh DB or
    // seed-only catalogue). Keys off sync history, not a hard-coded seed size.
    let ever_synced = cue::db::sync_runs::any_sync_ok(&pool)
        .await
        .map_err(std::io::Error::other)?;
    if !ever_synced && runner.try_start() {
        tracing::info!("startup sync triggered (no prior successful sync)");
    }

    // Daily (configurable) scheduled sync.
    let cron = cfg
        .sync_cron
        .clone()
        .unwrap_or_else(|| "0 0 3 * * *".to_string());
    let scheduler = tokio_cron_scheduler::JobScheduler::new()
        .await
        .map_err(std::io::Error::other)?;
    let runner_for_job = runner.clone();
    #[allow(clippy::redundant_pub_crate)] // macro-generated future in closure
    let job = tokio_cron_scheduler::Job::new_async(cron.as_str(), move |_uuid, _l| {
        let r = runner_for_job.clone();
        Box::pin(async move {
            if r.try_start() {
                tracing::info!("scheduled sync triggered");
            } else {
                tracing::warn!("scheduled sync skipped — a run is already active");
            }
        })
    })
    .map_err(std::io::Error::other)?;
    scheduler.add(job).await.map_err(std::io::Error::other)?;
    scheduler.start().await.map_err(std::io::Error::other)?;

    let plex_art = cue::routes::images::PlexArt {
        base_url: cfg.plex_url.clone(),
        token: cfg.plex_token.clone(),
    };
    let watch_cfg = cue::routes::watch::WatchConfig {
        plex_web_url: cfg.plex_web_url.clone(),
    };

    let static_dir = cfg.static_dir.clone();
    let bind_addr = cfg.bind_addr.clone();
    tracing::info!("listening on {bind_addr}");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(engine.clone()))
            .app_data(web::Data::new(runner.clone()))
            .app_data(web::Data::new(plex_art.clone()))
            .app_data(web::Data::new(watch_cfg.clone()))
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
