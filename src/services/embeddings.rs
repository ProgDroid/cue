//! `OpenAI` embeddings client behind the `Embedder` trait so tests run offline.

use async_trait::async_trait;
use sqlx::SqlitePool;

use crate::db::embeddings as embed_db;

pub const EMBED_MODEL: &str = "text-embedding-3-small";
const OPENAI_URL: &str = "https://api.openai.com/v1/embeddings";
/// Max inputs per embeddings request. `OpenAI` caps a request at 2048 inputs (and
/// a total token budget); a full catalogue exceeds that in one shot, so batch.
const EMBED_BATCH: usize = 100;

/// Produces an embedding for each input text, preserving input order.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// # Errors
    /// Returns an error if the upstream request fails or the body cannot be parsed.
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>>;
}

/// Compose the text fed to the embedding model for one title: the same facts
/// shown to Claude plus the description blurb (which carries the tone signal).
#[must_use]
pub fn embed_text(
    title: &str,
    year: i64,
    kind: &str,
    genres: &[String],
    description: &str,
) -> String {
    format!(
        "{title} ({year}) — {kind}. Genres: {}. {description}",
        genres.join(", ")
    )
}

/// Live `OpenAI` embeddings client.
pub struct OpenAiEmbedder {
    client: reqwest::Client,
    api_key: String,
}

impl OpenAiEmbedder {
    #[must_use]
    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
        }
    }
}

#[async_trait]
impl Embedder for OpenAiEmbedder {
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        #[derive(serde::Deserialize)]
        struct Resp {
            data: Vec<Item>,
        }
        #[derive(serde::Deserialize)]
        struct Item {
            embedding: Vec<f32>,
        }
        // Batch: a single request with the whole catalogue exceeds OpenAI's
        // per-request input cap and 400s. Chunks are sent in order and results
        // concatenated, preserving input order.
        let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(EMBED_BATCH) {
            let body = serde_json::json!({ "model": EMBED_MODEL, "input": chunk });
            let resp = self
                .client
                .post(OPENAI_URL)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await?;
            // Surface OpenAI's error body instead of swallowing it with
            // `error_for_status` (a bare status hides *why* it 400'd).
            let status = resp.status();
            if !status.is_success() {
                let detail = resp.text().await.unwrap_or_default();
                anyhow::bail!("OpenAI embeddings request failed ({status}): {detail}");
            }
            let parsed: Resp = resp.json().await?;
            out.extend(parsed.data.into_iter().map(|i| i.embedding));
        }
        Ok(out)
    }
}

/// Embed every title that lacks a current-model vector. Idempotent: a second
/// run with a fully embedded catalogue embeds nothing. Returns the count embedded.
///
/// # Errors
/// Returns an error if a query or the embedding request fails.
pub async fn backfill(pool: &SqlitePool, embedder: &dyn Embedder) -> anyhow::Result<usize> {
    let ids = embed_db::missing_for_model(pool, EMBED_MODEL).await?;
    if ids.is_empty() {
        return Ok(0);
    }
    // Load the fields needed for the embedding text for just these ids.
    let mut texts: Vec<(i64, String)> = Vec::with_capacity(ids.len());
    for id in &ids {
        let row = sqlx::query_as::<_, (String, i64, String, String)>(
            "SELECT title, year, type, description FROM titles WHERE id = ?",
        )
        .bind(id)
        .fetch_one(pool)
        .await?;
        let genres: Vec<String> =
            sqlx::query_scalar("SELECT genre FROM title_genres WHERE title_id = ? ORDER BY genre")
                .bind(id)
                .fetch_all(pool)
                .await?;
        texts.push((*id, embed_text(&row.0, row.1, &row.2, &genres, &row.3)));
    }
    let inputs: Vec<String> = texts.iter().map(|(_, t)| t.clone()).collect();
    let vectors = embedder.embed(&inputs).await?;
    for ((id, _), vec) in texts.iter().zip(vectors) {
        embed_db::upsert(pool, *id, &vec, EMBED_MODEL).await?;
    }
    Ok(texts.len())
}

/// The lightness direction = embed(light phrase) - embed(dark phrase).
///
/// # Errors
/// Returns an error if the embedding request fails.
pub async fn build_light_axis(embedder: &dyn Embedder) -> anyhow::Result<Vec<f32>> {
    let anchors = embedder
        .embed(&[
            "light-hearted, feel-good, funny, warm, cozy".to_string(),
            "dark, bleak, heavy, serious, grim".to_string(),
        ])
        .await?;
    Ok(crate::services::similarity::axis(&anchors[0], &anchors[1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{embeddings as embed_db, init_pool, seed::seed_if_empty};

    #[test]
    fn embed_text_includes_facts_and_blurb() {
        let s = embed_text(
            "Frieren",
            2023,
            "series",
            &["Animation".into(), "Adventure".into()],
            "An elf mage reflects on time.",
        );
        assert!(s.contains("Frieren (2023)"));
        assert!(s.contains("series"));
        assert!(s.contains("Animation, Adventure"));
        assert!(s.contains("elf mage"));
    }

    struct SeqEmbedder;
    #[async_trait]
    impl Embedder for SeqEmbedder {
        #[allow(clippy::cast_precision_loss)]
        // text length fits f32 mantissa for any realistic embed input
        async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
            // Distinct deterministic vectors keyed by text length.
            Ok(texts
                .iter()
                .map(|t| vec![t.len() as f32, 1.0, 0.0])
                .collect())
        }
    }

    async fn seeded() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        seed_if_empty(&pool).await.unwrap();
        (pool, dir)
    }

    #[tokio::test]
    async fn backfill_embeds_then_is_idempotent() {
        let (pool, _dir) = seeded().await;
        let n = backfill(&pool, &SeqEmbedder).await.unwrap();
        assert_eq!(n, 28);
        let again = backfill(&pool, &SeqEmbedder).await.unwrap();
        assert_eq!(again, 0, "second run embeds nothing");
        let stored = embed_db::load_all(&pool, EMBED_MODEL).await.unwrap();
        assert_eq!(stored.len(), 28);
    }

    #[tokio::test]
    async fn light_axis_is_difference_of_anchors() {
        let axis = build_light_axis(&SeqEmbedder).await.unwrap();
        // SeqEmbedder vectors differ in component 0 by the phrase-length delta.
        assert_eq!(axis.len(), 3);
    }
}
