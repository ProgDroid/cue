//! `OpenAI` embeddings client behind the `Embedder` trait so tests run offline.

use async_trait::async_trait;

pub const EMBED_MODEL: &str = "text-embedding-3-small";
const OPENAI_URL: &str = "https://api.openai.com/v1/embeddings";

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
        let body = serde_json::json!({ "model": EMBED_MODEL, "input": texts });
        let resp: Resp = self
            .client
            .post(OPENAI_URL)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp.data.into_iter().map(|i| i.embedding).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
