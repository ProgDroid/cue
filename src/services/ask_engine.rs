//! Orchestrates ask/similar/refine over the catalogue and external models.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sqlx::SqlitePool;

use crate::db::embeddings;
use crate::services::anthropic::{AskAnswer, AskModel, Candidate};
use crate::services::embeddings::{Embedder, EMBED_MODEL};
use crate::services::similarity;

const CANDIDATE_CAP: usize = 150;

/// Errors the routes translate to HTTP: `Unavailable` -> 503, `Other` -> 500.
#[derive(Debug, thiserror::Error)]
pub enum AskError {
    #[error("ask is unavailable (missing API key or embeddings)")]
    Unavailable,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Holds the DB plus the (optional) external clients and lightness axis.
pub struct AskEngine {
    pool: SqlitePool,
    embedder: Option<Arc<dyn Embedder>>,
    model: Option<Arc<dyn AskModel>>,
    #[allow(dead_code)] // used by the future `similar`/`refine` methods
    light_axis: Option<Vec<f32>>,
}

impl AskEngine {
    #[must_use]
    pub fn new(
        pool: SqlitePool,
        embedder: Option<Arc<dyn Embedder>>,
        model: Option<Arc<dyn AskModel>>,
        light_axis: Option<Vec<f32>>,
    ) -> Self {
        Self {
            pool,
            embedder,
            model,
            light_axis,
        }
    }

    /// Natural-language ask. With `base_ids`, ranks that set directly (refine /
    /// follow-up within a thread); otherwise embeds the query and retrieves the
    /// top candidates from the whole catalogue.
    ///
    /// # Errors
    /// Returns `AskError::Unavailable` if no model, embedder, or stored vectors
    /// are available. Returns `AskError::Other` on upstream I/O or parse failure.
    pub async fn ask(
        &self,
        query: &str,
        base_ids: Option<Vec<i64>>,
    ) -> Result<AskAnswer, AskError> {
        let model = self.model.as_ref().ok_or(AskError::Unavailable)?;
        let candidates = match base_ids {
            Some(ids) if !ids.is_empty() => self.candidates_for(&ids).await?,
            _ => {
                let embedder = self.embedder.as_ref().ok_or(AskError::Unavailable)?;
                let mut qvecs = embedder.embed(&[query.to_string()]).await?;
                let qvec = qvecs.pop().ok_or(AskError::Unavailable)?;
                let vectors = embeddings::load_all(&self.pool, EMBED_MODEL).await?;
                if vectors.is_empty() {
                    return Err(AskError::Unavailable);
                }
                let top = similarity::rank_by_cosine(&qvec, &vectors, CANDIDATE_CAP);
                self.candidates_for(&top).await?
            }
        };
        let mut answer = model.rank(query, &candidates).await?;
        let allowed: HashSet<i64> = candidates.iter().map(|c| c.id).collect();
        answer.ids.retain(|id| allowed.contains(id));
        Ok(answer)
    }

    /// Load compact candidates for the given ids, preserving the input order.
    async fn candidates_for(&self, ids: &[i64]) -> anyhow::Result<Vec<Candidate>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        // Build one row per id; genres joined in a second query.
        let mut by_id: HashMap<i64, Candidate> = HashMap::new();
        let rows = sqlx::query_as::<_, (i64, String, i64, String, Option<f64>)>(
            "SELECT id, title, year, type, imdb_rating FROM titles",
        )
        .fetch_all(&self.pool)
        .await?;
        let want: HashSet<i64> = ids.iter().copied().collect();
        for (id, title, year, kind, imdb) in rows {
            if want.contains(&id) {
                by_id.insert(
                    id,
                    Candidate {
                        id,
                        title,
                        year,
                        kind,
                        genres: Vec::new(),
                        imdb,
                    },
                );
            }
        }
        let genres = sqlx::query_as::<_, (i64, String)>(
            "SELECT title_id, genre FROM title_genres ORDER BY title_id, genre",
        )
        .fetch_all(&self.pool)
        .await?;
        for (tid, g) in genres {
            if let Some(c) = by_id.get_mut(&tid) {
                c.genres.push(g);
            }
        }
        Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{init_pool, seed::seed_if_empty};
    use crate::services::anthropic::AskAnswer;
    use async_trait::async_trait;

    async fn seeded() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        seed_if_empty(&pool).await.unwrap();
        (pool, dir)
    }

    /// Embeds every text to the same constant vector (retrieval is order-only here).
    struct ConstEmbedder;
    #[async_trait]
    impl Embedder for ConstEmbedder {
        async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|_| vec![1.0, 0.0, 0.0]).collect())
        }
    }

    /// Returns whatever ids it is told to, plus one bogus id to prove validation.
    struct EchoModel;
    #[async_trait]
    impl AskModel for EchoModel {
        async fn rank(&self, _q: &str, candidates: &[Candidate]) -> anyhow::Result<AskAnswer> {
            let mut ids: Vec<i64> = candidates.iter().take(2).map(|c| c.id).collect();
            ids.push(9_999_999); // not a real candidate — must be dropped
            Ok(AskAnswer {
                ids,
                line: "ok".into(),
                sub: "ok".into(),
            })
        }
    }

    #[tokio::test]
    async fn ask_without_model_is_unavailable() {
        let (pool, _dir) = seeded().await;
        let engine = AskEngine::new(pool, None, None, None);
        assert!(matches!(
            engine.ask("anything", None).await,
            Err(AskError::Unavailable)
        ));
    }

    #[tokio::test]
    async fn ask_with_base_ids_skips_retrieval_and_validates() {
        let (pool, _dir) = seeded().await;
        // No embedder needed because base_ids short-circuits retrieval.
        let engine = AskEngine::new(pool, None, Some(Arc::new(EchoModel)), None);
        let answer = engine.ask("cozy", Some(vec![1, 2, 3])).await.unwrap();
        // EchoModel returned [1, 2, 9999999]; the bogus id is dropped.
        assert_eq!(answer.ids, vec![1, 2]);
    }

    #[tokio::test]
    async fn ask_freeform_requires_embeddings() {
        let (pool, _dir) = seeded().await;
        // Embedder present, but no vectors stored yet -> Unavailable.
        let engine = AskEngine::new(
            pool,
            Some(Arc::new(ConstEmbedder)),
            Some(Arc::new(EchoModel)),
            None,
        );
        assert!(matches!(
            engine.ask("cozy", None).await,
            Err(AskError::Unavailable)
        ));
    }
}
