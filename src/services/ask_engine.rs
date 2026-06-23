//! Orchestrates ask/similar/refine over the catalogue and external models.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sqlx::SqlitePool;

use crate::db::embeddings;
use crate::services::anthropic::{AskAnswer, AskModel, Candidate};
use crate::services::embeddings::{Embedder, EMBED_MODEL};
use crate::services::similarity;

const CANDIDATE_CAP: usize = 150;

/// Light-toned genres used as the fallback when no lightness axis/vector exists.
const LIGHT_GENRES: [&str; 5] = ["Comedy", "Animation", "Adventure", "Romance", "Musical"];

/// "164 min" -> 164; "28 eps" -> eps*24 so series sort sensibly among movies.
/// Mirrors the frontend stub's `lenMinutes`.
#[must_use]
pub fn len_minutes(len: &str) -> i64 {
    let n: i64 = len
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    if len.to_ascii_lowercase().contains("eps") {
        n * 24
    } else {
        n
    }
}

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

    /// "More like {title}": cosine over stored vectors, or shared-genre overlap
    /// when the anchor (or candidates) lack vectors.
    ///
    /// # Errors
    /// Returns `AskError::Other` on database or I/O failure.
    pub async fn similar(
        &self,
        anchor_id: i64,
        base_ids: Option<Vec<i64>>,
    ) -> Result<AskAnswer, AskError> {
        let pool_ids = match base_ids {
            Some(ids) if !ids.is_empty() => ids,
            _ => self.all_ids().await?,
        };
        let vectors = embeddings::load_all(&self.pool, EMBED_MODEL).await?;
        let vmap: HashMap<i64, Vec<f32>> = vectors.into_iter().collect();

        let ids = if let Some(anchor) = vmap.get(&anchor_id) {
            let mut scored: Vec<(i64, f32)> = pool_ids
                .iter()
                .filter(|&&id| id != anchor_id)
                .filter_map(|&id| vmap.get(&id).map(|v| (id, similarity::cosine(anchor, v))))
                .collect();
            scored.sort_by(|a, b| b.1.total_cmp(&a.1));
            scored.into_iter().take(20).map(|(id, _)| id).collect()
        } else {
            self.genre_overlap(anchor_id, &pool_ids).await?
        };

        let title = self.title_name(anchor_id).await?;
        Ok(AskAnswer {
            sub: format!("{} · refine or filter to narrow", ids.len()),
            line: format!("More like {title}."),
            ids,
        })
    }

    /// Deterministic refine chips — no Claude. `shorter` sorts by runtime;
    /// `lighter` projects onto the lightness axis (or filters light genres);
    /// `surprise` picks a high-rated outlier (or the highest-rated title).
    ///
    /// # Errors
    /// Returns `AskError::Other` on unknown `kind` or database/I/O failure.
    pub async fn refine(&self, kind: &str, ids: &[i64]) -> Result<AskAnswer, AskError> {
        let (line, out): (&str, Vec<i64>) = match kind {
            "shorter" => ("Shortest first.", self.sort_by_runtime(ids).await?),
            "lighter" => ("Lighter picks.", self.lighter(ids).await?),
            "surprise" => ("A wildcard you might've missed.", self.surprise(ids).await?),
            other => {
                return Err(AskError::Other(anyhow::anyhow!(
                    "unknown refine kind: {other}"
                )))
            }
        };
        Ok(AskAnswer {
            sub: format!("{} · refine or filter to narrow", out.len()),
            line: line.to_string(),
            ids: out,
        })
    }

    async fn all_ids(&self) -> anyhow::Result<Vec<i64>> {
        Ok(sqlx::query_scalar("SELECT id FROM titles ORDER BY id")
            .fetch_all(&self.pool)
            .await?)
    }

    async fn title_name(&self, id: i64) -> anyhow::Result<String> {
        Ok(sqlx::query_scalar("SELECT title FROM titles WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .unwrap_or_else(|| "this".to_string()))
    }

    async fn genre_overlap(&self, anchor_id: i64, pool_ids: &[i64]) -> anyhow::Result<Vec<i64>> {
        let anchor_genres: Vec<String> =
            sqlx::query_scalar("SELECT genre FROM title_genres WHERE title_id = ?")
                .bind(anchor_id)
                .fetch_all(&self.pool)
                .await?;
        let anchor_set: HashSet<String> = anchor_genres.into_iter().collect();
        let all = sqlx::query_as::<_, (i64, String)>("SELECT title_id, genre FROM title_genres")
            .fetch_all(&self.pool)
            .await?;
        let mut shared: HashMap<i64, i64> = HashMap::new();
        for (tid, g) in all {
            if anchor_set.contains(&g) {
                *shared.entry(tid).or_default() += 1;
            }
        }
        let want: HashSet<i64> = pool_ids.iter().copied().collect();
        let mut scored: Vec<(i64, i64)> = shared
            .into_iter()
            .filter(|(id, _)| *id != anchor_id && want.contains(id))
            .collect();
        scored.sort_by_key(|b| std::cmp::Reverse(b.1));
        Ok(scored.into_iter().take(20).map(|(id, _)| id).collect())
    }

    async fn sort_by_runtime(&self, ids: &[i64]) -> anyhow::Result<Vec<i64>> {
        let rows = sqlx::query_as::<_, (i64, String)>("SELECT id, length FROM titles")
            .fetch_all(&self.pool)
            .await?;
        let lens: HashMap<i64, i64> = rows
            .into_iter()
            .map(|(id, len)| (id, len_minutes(&len)))
            .collect();
        let mut out: Vec<i64> = ids.to_vec();
        out.sort_by_key(|id| lens.get(id).copied().unwrap_or(0));
        Ok(out)
    }

    async fn lighter(&self, ids: &[i64]) -> anyhow::Result<Vec<i64>> {
        if let Some(axis) = &self.light_axis {
            let vectors = embeddings::load_all(&self.pool, EMBED_MODEL).await?;
            let vmap: HashMap<i64, Vec<f32>> = vectors.into_iter().collect();
            if ids.iter().any(|id| vmap.contains_key(id)) {
                let mut scored: Vec<(i64, f32)> = ids
                    .iter()
                    .filter_map(|id| vmap.get(id).map(|v| (*id, similarity::dot(v, axis))))
                    .collect();
                scored.sort_by(|a, b| b.1.total_cmp(&a.1));
                return Ok(scored.into_iter().map(|(id, _)| id).collect());
            }
        }
        // Fallback: keep ids that have a light-toned genre.
        let all = sqlx::query_as::<_, (i64, String)>("SELECT title_id, genre FROM title_genres")
            .fetch_all(&self.pool)
            .await?;
        let mut light: HashSet<i64> = HashSet::new();
        for (tid, g) in all {
            if LIGHT_GENRES.contains(&g.as_str()) {
                light.insert(tid);
            }
        }
        Ok(ids
            .iter()
            .copied()
            .filter(|id| light.contains(id))
            .collect())
    }

    async fn surprise(&self, ids: &[i64]) -> anyhow::Result<Vec<i64>> {
        let vectors = embeddings::load_all(&self.pool, EMBED_MODEL).await?;
        let vmap: HashMap<i64, Vec<f32>> = vectors.into_iter().collect();
        let ratings = sqlx::query_as::<_, (i64, Option<f64>)>("SELECT id, imdb_rating FROM titles")
            .fetch_all(&self.pool)
            .await?;
        let rmap: HashMap<i64, f64> = ratings
            .into_iter()
            .map(|(id, r)| (id, r.unwrap_or(0.0)))
            .collect();

        // With vectors: farthest from the centroid of the set, weighted by rating.
        let present: Vec<Vec<f32>> = ids.iter().filter_map(|id| vmap.get(id).cloned()).collect();
        if !present.is_empty() {
            let center = similarity::centroid(&present);
            let pick = ids
                .iter()
                .filter_map(|id| vmap.get(id).map(|v| (*id, v)))
                .max_by(|(ia, va), (ib, vb)| {
                    // Compute score in f64 to avoid cast_possible_truncation on rmap values.
                    // `.get().unwrap_or(0.0)` (not indexing) so an embedding that
                    // outlived its title degrades instead of panicking — mirrors
                    // the fallback below.
                    let da = (1.0 - f64::from(similarity::cosine(va, &center)))
                        + rmap.get(ia).copied().unwrap_or(0.0) / 10.0;
                    let db = (1.0 - f64::from(similarity::cosine(vb, &center)))
                        + rmap.get(ib).copied().unwrap_or(0.0) / 10.0;
                    da.total_cmp(&db)
                })
                .map(|(id, _)| id);
            if let Some(id) = pick {
                return Ok(vec![id]);
            }
        }
        // Fallback: the highest-rated id in the set.
        let pick = ids.iter().copied().max_by(|a, b| {
            rmap.get(a)
                .unwrap_or(&0.0)
                .total_cmp(rmap.get(b).unwrap_or(&0.0))
        });
        Ok(pick.into_iter().collect())
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

    #[tokio::test]
    async fn refine_shorter_orders_by_runtime() {
        let (pool, _dir) = seeded().await;
        let engine = AskEngine::new(pool, None, None, None);
        // Seed ids 1..=3 exist; shorter must return them sorted by ascending length.
        let answer = engine.refine("shorter", &[1, 2, 3]).await.unwrap();
        assert_eq!(answer.ids.len(), 3);
        let mins: Vec<i64> = {
            let mut v = Vec::new();
            for id in &answer.ids {
                let len: String = sqlx::query_scalar("SELECT length FROM titles WHERE id = ?")
                    .bind(id)
                    .fetch_one(engine_pool(&engine))
                    .await
                    .unwrap();
                v.push(len_minutes(&len));
            }
            v
        };
        assert!(mins.windows(2).all(|w| w[0] <= w[1]), "ascending runtime");
    }

    #[tokio::test]
    async fn refine_lighter_falls_back_to_light_genres_without_axis() {
        let (pool, _dir) = seeded().await;
        let engine = AskEngine::new(pool, None, None, None);
        let answer = engine.refine("lighter", &[1, 2, 3]).await.unwrap();
        // Every returned id must have at least one light genre.
        for id in &answer.ids {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM title_genres WHERE title_id = ?
                 AND genre IN ('Comedy','Animation','Adventure','Romance','Musical')",
            )
            .bind(id)
            .fetch_one(engine_pool(&engine))
            .await
            .unwrap();
            assert!(count > 0, "id {id} kept without a light genre");
        }
    }

    #[tokio::test]
    async fn surprise_does_not_panic_on_embedding_without_title() {
        // Defensive: if an embedding row ever outlives its title (orphan), the
        // rating lookup in `surprise` must degrade, not panic. FK is normally
        // enforced, so insert the orphan with foreign_keys OFF on a dedicated
        // connection to reproduce the drift.
        let (pool, _dir) = seeded().await;
        embeddings::upsert(&pool, 1, &[1.0, 0.0, 0.0], EMBED_MODEL)
            .await
            .unwrap();
        let mut conn = pool.acquire().await.unwrap();
        sqlx::query("PRAGMA foreign_keys=OFF")
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO title_embeddings (title_id, vector, model, dims) VALUES (?, ?, ?, ?)",
        )
        .bind(9_999_999_i64)
        .bind(embeddings::encode(&[0.0, 1.0, 0.0]))
        .bind(EMBED_MODEL)
        .bind(3_i64)
        .execute(&mut *conn)
        .await
        .unwrap();
        sqlx::query("PRAGMA foreign_keys=ON")
            .execute(&mut *conn)
            .await
            .unwrap();
        drop(conn);

        let engine = AskEngine::new(pool, None, None, None);
        // Must not panic; returns exactly one pick from the input set.
        let answer = engine.refine("surprise", &[1, 9_999_999]).await.unwrap();
        assert_eq!(answer.ids.len(), 1);
        assert!(answer.ids[0] == 1 || answer.ids[0] == 9_999_999);
    }

    #[tokio::test]
    async fn similar_without_vectors_uses_genre_overlap() {
        let (pool, _dir) = seeded().await;
        let engine = AskEngine::new(pool, None, None, None);
        let answer = engine.similar(1, None).await.unwrap();
        assert!(!answer.ids.contains(&1), "anchor excluded");
        assert!(answer.line.contains("More like"));
    }

    #[test]
    fn len_minutes_parses_movies_and_series() {
        assert_eq!(len_minutes("164 min"), 164);
        assert_eq!(len_minutes("28 eps"), 28 * 24);
        assert_eq!(len_minutes(""), 0);
    }

    // Test-only accessor so the assertions above can re-query the pool.
    fn engine_pool(engine: &AskEngine) -> &SqlitePool {
        &engine.pool
    }
}
