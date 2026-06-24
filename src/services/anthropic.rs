//! Claude ranking client behind the `AskModel` trait so tests run offline.

use async_trait::async_trait;

pub const ASK_MODEL: &str = "claude-sonnet-4-6";
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// One retrieval candidate handed to Claude (compact — no description blurb).
#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: i64,
    pub title: String,
    pub year: i64,
    pub kind: String,
    pub genres: Vec<String>,
    pub score: Option<f64>,
}

/// The validated ask result. Field names match the frontend `AskResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskAnswer {
    pub ids: Vec<i64>,
    pub line: String,
    pub sub: String,
}

/// Ranks candidates against a natural-language query.
#[async_trait]
pub trait AskModel: Send + Sync {
    /// # Errors
    /// Returns an error if the upstream request fails or the reply is malformed.
    async fn rank(&self, query: &str, candidates: &[Candidate]) -> anyhow::Result<AskAnswer>;
}

/// The JSON schema constraining Claude's reply to `{ids, line, sub}`.
#[must_use]
pub fn ask_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "ids": { "type": "array", "items": { "type": "integer" } },
            "line": { "type": "string" },
            "sub": { "type": "string" }
        },
        "required": ["ids", "line", "sub"],
        "additionalProperties": false
    })
}

/// Parse the model's JSON text block into an `AskAnswer`.
///
/// # Errors
/// Returns an error if the text is not valid JSON of the expected shape.
pub fn parse_answer(text: &str) -> anyhow::Result<AskAnswer> {
    #[derive(serde::Deserialize)]
    struct Raw {
        ids: Vec<i64>,
        line: String,
        sub: String,
    }
    let raw: Raw = serde_json::from_str(text)?;
    Ok(AskAnswer {
        ids: raw.ids,
        line: raw.line,
        sub: raw.sub,
    })
}

fn candidate_lines(candidates: &[Candidate]) -> String {
    candidates
        .iter()
        .map(|c| {
            let score = c.score.map_or_else(|| "n/a".to_string(), |r| r.to_string());
            format!(
                "id={} | {} ({}) | {} | {} | score {}",
                c.id,
                c.title,
                c.year,
                c.kind,
                c.genres.join("/"),
                score
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn prompt(query: &str, candidates: &[Candidate]) -> String {
    format!(
        "You are a film/TV recommender for a personal library. From the catalogue \
         below, choose the titles that best answer the request, best first. Only use \
         ids that appear in the list. Write `line` as one warm sentence answering the \
         request, and `sub` as a short hint like \"{n} · refine or filter to narrow\". \
         Request: \"{query}\"\n\nCatalogue:\n{list}",
        n = candidates.len(),
        list = candidate_lines(candidates),
    )
}

/// Live Claude client (raw HTTP — there is no official Rust SDK).
pub struct ClaudeAskModel {
    client: reqwest::Client,
    api_key: String,
}

impl ClaudeAskModel {
    #[must_use]
    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
        }
    }
}

#[async_trait]
impl AskModel for ClaudeAskModel {
    async fn rank(&self, query: &str, candidates: &[Candidate]) -> anyhow::Result<AskAnswer> {
        let body = serde_json::json!({
            "model": ASK_MODEL,
            "max_tokens": 1024,
            "thinking": { "type": "disabled" },
            "output_config": {
                "effort": "low",
                "format": { "type": "json_schema", "schema": ask_schema() }
            },
            "messages": [{ "role": "user", "content": prompt(query, candidates) }]
        });
        let resp: serde_json::Value = self
            .client
            .post(ANTHROPIC_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let text = resp["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("no text block in Claude response"))?;
        parse_answer(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_answer_reads_schema_shape() {
        let a = parse_answer(r#"{"ids":[3,1],"line":"Cozy picks.","sub":"2 · refine"}"#).unwrap();
        assert_eq!(a.ids, vec![3, 1]);
        assert_eq!(a.line, "Cozy picks.");
        assert_eq!(a.sub, "2 · refine");
    }

    #[test]
    fn parse_answer_rejects_garbage() {
        assert!(parse_answer("not json").is_err());
    }

    #[test]
    fn ask_schema_is_closed() {
        assert_eq!(
            ask_schema()["additionalProperties"],
            serde_json::json!(false)
        );
    }
}
