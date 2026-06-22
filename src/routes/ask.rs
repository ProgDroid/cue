use std::sync::Arc;

use actix_web::{web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};

use crate::services::anthropic::AskAnswer;
use crate::services::ask_engine::{AskEngine, AskError};

#[derive(Deserialize)]
pub struct AskReq {
    pub query: String,
    #[serde(rename = "baseIds", default)]
    pub base_ids: Option<Vec<i64>>,
}

#[derive(Deserialize)]
pub struct SimilarReq {
    #[serde(rename = "anchorId")]
    pub anchor_id: i64,
    #[serde(rename = "baseIds", default)]
    pub base_ids: Option<Vec<i64>>,
}

#[derive(Deserialize)]
pub struct RefineReq {
    pub kind: String,
    pub ids: Vec<i64>,
}

#[derive(Serialize)]
pub struct AskRes {
    pub line: String,
    pub sub: String,
    pub ids: Vec<i64>,
}

impl From<AskAnswer> for AskRes {
    fn from(a: AskAnswer) -> Self {
        Self {
            line: a.line,
            sub: a.sub,
            ids: a.ids,
        }
    }
}

fn respond(result: Result<AskAnswer, AskError>) -> HttpResponse {
    match result {
        Ok(answer) => HttpResponse::Ok().json(AskRes::from(answer)),
        Err(AskError::Unavailable) => HttpResponse::ServiceUnavailable()
            .json(serde_json::json!({ "error": "ask is unavailable" })),
        Err(AskError::Other(e)) => {
            tracing::error!("ask failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

pub async fn ask(engine: web::Data<Arc<AskEngine>>, body: web::Json<AskReq>) -> impl Responder {
    let req = body.into_inner();
    respond(engine.ask(&req.query, req.base_ids).await)
}

pub async fn similar(
    engine: web::Data<Arc<AskEngine>>,
    body: web::Json<SimilarReq>,
) -> impl Responder {
    let req = body.into_inner();
    respond(engine.similar(req.anchor_id, req.base_ids).await)
}

pub async fn refine(
    engine: web::Data<Arc<AskEngine>>,
    body: web::Json<RefineReq>,
) -> impl Responder {
    let req = body.into_inner();
    respond(engine.refine(&req.kind, &req.ids).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{test, App};
    use serde_json::Value;

    use crate::db::{init_pool, seed::seed_if_empty};
    use crate::routes;

    async fn engine_no_models() -> (Arc<AskEngine>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        seed_if_empty(&pool).await.unwrap();
        (Arc::new(AskEngine::new(pool, None, None, None)), dir)
    }

    #[actix_web::test]
    async fn ask_without_keys_returns_503() {
        let (engine, _dir) = engine_no_models().await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(engine))
                .configure(routes::configure),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/api/ask")
            .set_json(serde_json::json!({ "query": "cozy" }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 503);
    }

    #[actix_web::test]
    async fn refine_shorter_returns_ids_without_keys() {
        let (engine, _dir) = engine_no_models().await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(engine))
                .configure(routes::configure),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/api/ask/refine")
            .set_json(serde_json::json!({ "kind": "shorter", "ids": [1, 2, 3] }))
            .to_request();
        let body: Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(body["ids"].as_array().unwrap().len(), 3);
        assert!(body["line"].is_string());
        assert!(body["sub"].is_string());
    }

    #[actix_web::test]
    async fn similar_returns_anchor_excluded_set() {
        let (engine, _dir) = engine_no_models().await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(engine))
                .configure(routes::configure),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/api/ask/similar")
            .set_json(serde_json::json!({ "anchorId": 1 }))
            .to_request();
        let body: Value = test::call_and_read_body_json(&app, req).await;

        assert!(!body["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .any(|x| x == 1));
    }
}
