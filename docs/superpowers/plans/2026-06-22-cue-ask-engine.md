# cue Ask Engine (Plan 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the frontend's client-side `StubAskService` with a server-backed ask engine — OpenAI embeddings + cosine retrieval → Claude structured output for `/api/ask`, plus pure-math endpoints for similar and the refine chips.

**Architecture:** A new `src/services/` layer wraps two external REST APIs behind `Embedder` and `AskModel` traits (so tests run offline). `ask_engine` orchestrates retrieval → ranking; `similarity` holds pure vector math; `db/embeddings` persists vectors. Only `/api/ask` reaches Claude; `similar` and refine are deterministic vector/attribute math. A startup backfill embeds the seeded catalogue.

**Tech Stack:** Rust + Actix-web + SQLx (SQLite) + `reqwest` (rustls) + `async-trait`; Vue 3 + TS + Pinia + Vitest.

**Spec:** `docs/superpowers/specs/2026-06-22-cue-ask-engine-design.md`

## Global Constraints

- **Crate is lib + bin.** Declare every new module in `src/lib.rs` (or a parent `mod.rs`), never in `main.rs`, so `cargo test --lib` sees it.
- **SQLx runtime queries only** — `sqlx::query` / `query_as` / `query_scalar`. NEVER the compile-time `query!` / `query_as!` macros (no live `DATABASE_URL`, no offline cache).
- **Test databases:** `tempfile::tempdir()` (NOT `NamedTempFile`); build the URL as `format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"))`; keep the `TempDir` guard bound as `_dir`.
- **Clippy:** canonical `[lints.clippy]` table is the single source of truth. Per-item `#[allow(clippy::…)]` with a one-line reason only; never widen the global table. Gate: `cargo clippy --all-targets -- -D warnings`.
- **Security boundary:** `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` are server-side `Config` only — NEVER serialized to the client.
- **Frontend on Windows:** use `npm install` / `npm run …` for local work (the lockfile is Linux-flavored for Docker `npm ci`). Tests: `npm run test:unit`.
- **Commit with the Bash tool, not PowerShell** (PowerShell prepends a UTF-8 BOM to commit subjects).
- **Embedding model string:** `text-embedding-3-small`. **Ask model string:** `claude-sonnet-4-6`. **Anthropic version header:** `2023-06-01`.

---

### Task 1: Dependencies + `services` module + `similarity` (pure vector math)

**Files:**
- Modify: `Cargo.toml` (add `reqwest`, `async-trait` to `[dependencies]`)
- Modify: `src/lib.rs` (add `pub mod services;`)
- Create: `src/services/mod.rs`
- Create: `src/services/similarity.rs` (impl + `#[cfg(test)]`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `cue::services::similarity::cosine(a: &[f32], b: &[f32]) -> f32`
  - `cue::services::similarity::dot(a: &[f32], b: &[f32]) -> f32`
  - `cue::services::similarity::axis(light: &[f32], dark: &[f32]) -> Vec<f32>`
  - `cue::services::similarity::centroid(vectors: &[Vec<f32>]) -> Vec<f32>`
  - `cue::services::similarity::rank_by_cosine(query: &[f32], items: &[(i64, Vec<f32>)], top_n: usize) -> Vec<i64>`

- [ ] **Step 1: Add dependencies**

In `Cargo.toml` under `[dependencies]`, add:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
async-trait = "0.1"
```

- [ ] **Step 2: Register the module**

In `src/lib.rs`, add the line (keep alphabetical with the others):

```rust
pub mod services;
```

- [ ] **Step 3: Create the services module file**

Create `src/services/mod.rs`:

```rust
pub mod similarity;
```

- [ ] **Step 4: Write the failing tests**

Create `src/services/similarity.rs`:

```rust
//! Pure vector math for retrieval and the refine chips. No I/O, no async.

/// Dot product of two equal-length vectors. Shorter length wins if they differ.
#[must_use]
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Cosine similarity in [-1, 1]; 0.0 if either vector has zero magnitude.
#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let na = dot(a, a).sqrt();
    let nb = dot(b, b).sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot(a, b) / (na * nb)
}

/// A direction vector = `light - dark`, elementwise. Used to score "lightness".
#[must_use]
pub fn axis(light: &[f32], dark: &[f32]) -> Vec<f32> {
    light.iter().zip(dark).map(|(l, d)| l - d).collect()
}

/// Mean vector of a set. Empty input yields an empty vector.
#[must_use]
pub fn centroid(vectors: &[Vec<f32>]) -> Vec<f32> {
    let Some(first) = vectors.first() else {
        return Vec::new();
    };
    let mut acc = vec![0.0_f32; first.len()];
    for v in vectors {
        for (a, x) in acc.iter_mut().zip(v) {
            *a += x;
        }
    }
    let n = vectors.len() as f32;
    for a in &mut acc {
        *a /= n;
    }
    acc
}

/// Top-`n` item ids by descending cosine similarity to `query`.
#[must_use]
pub fn rank_by_cosine(query: &[f32], items: &[(i64, Vec<f32>)], top_n: usize) -> Vec<i64> {
    let mut scored: Vec<(i64, f32)> = items
        .iter()
        .map(|(id, v)| (*id, cosine(query, v)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.into_iter().take(top_n).map(|(id, _)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_of_identical_vectors_is_one() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_handles_zero_vector() {
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn axis_is_elementwise_difference() {
        assert_eq!(axis(&[1.0, 1.0], &[0.0, 2.0]), vec![1.0, -1.0]);
    }

    #[test]
    fn centroid_averages_componentwise() {
        assert_eq!(centroid(&[vec![0.0, 0.0], vec![2.0, 4.0]]), vec![1.0, 2.0]);
    }

    #[test]
    fn rank_orders_by_similarity_and_caps() {
        let q = vec![1.0, 0.0];
        let items = vec![
            (1, vec![0.0, 1.0]), // orthogonal
            (2, vec![1.0, 0.0]), // identical
            (3, vec![1.0, 0.1]), // close
        ];
        assert_eq!(rank_by_cosine(&q, &items, 2), vec![2, 3]);
    }
}
```

- [ ] **Step 5: Run tests to verify they pass and clippy is clean**

Run: `cargo test --lib services::similarity && cargo clippy --all-targets -- -D warnings`
Expected: tests PASS; clippy clean.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/services/mod.rs src/services/similarity.rs
git commit -m "feat(ask): add reqwest/async-trait deps and pure similarity math"
```

---

### Task 2: `db/embeddings` — vector persistence

**Files:**
- Modify: `src/db/mod.rs` (add `pub mod embeddings;`)
- Create: `src/db/embeddings.rs` (impl + `#[cfg(test)]`)

**Interfaces:**
- Consumes: `sqlx::SqlitePool`; the `title_embeddings(title_id, vector BLOB, model, dims)` table from `migrations/0001_init.sql`.
- Produces:
  - `cue::db::embeddings::encode(v: &[f32]) -> Vec<u8>`
  - `cue::db::embeddings::decode(bytes: &[u8]) -> Vec<f32>`
  - `async fn cue::db::embeddings::upsert(pool: &SqlitePool, title_id: i64, vector: &[f32], model: &str) -> anyhow::Result<()>`
  - `async fn cue::db::embeddings::missing_for_model(pool: &SqlitePool, model: &str) -> anyhow::Result<Vec<i64>>` (title ids with no row for `model`)
  - `async fn cue::db::embeddings::load_all(pool: &SqlitePool, model: &str) -> anyhow::Result<Vec<(i64, Vec<f32>)>>`

- [ ] **Step 1: Register the module**

In `src/db/mod.rs`, add alongside the existing `pub mod` lines:

```rust
pub mod embeddings;
```

- [ ] **Step 2: Write the failing tests**

Create `src/db/embeddings.rs`:

```rust
//! Read/write `title_embeddings`. Vectors are stored as little-endian f32 bytes.

use sqlx::SqlitePool;

/// Serialize a vector to little-endian f32 bytes for a BLOB column.
#[must_use]
pub fn encode(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Deserialize little-endian f32 bytes back to a vector. Trailing partial
/// chunks (should never happen) are ignored.
#[must_use]
pub fn decode(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Insert or replace the embedding for a title under a given model.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn upsert(
    pool: &SqlitePool,
    title_id: i64,
    vector: &[f32],
    model: &str,
) -> anyhow::Result<()> {
    let dims = i64::try_from(vector.len()).unwrap_or(0);
    sqlx::query(
        "INSERT INTO title_embeddings (title_id, vector, model, dims)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(title_id) DO UPDATE SET vector = excluded.vector,
             model = excluded.model, dims = excluded.dims",
    )
    .bind(title_id)
    .bind(encode(vector))
    .bind(model)
    .bind(dims)
    .execute(pool)
    .await?;
    Ok(())
}

/// Title ids that have no embedding row for `model`.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn missing_for_model(pool: &SqlitePool, model: &str) -> anyhow::Result<Vec<i64>> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT t.id FROM titles t
         LEFT JOIN title_embeddings e ON e.title_id = t.id AND e.model = ?
         WHERE e.title_id IS NULL
         ORDER BY t.id",
    )
    .bind(model)
    .fetch_all(pool)
    .await?;
    Ok(ids)
}

/// Every `(title_id, vector)` stored for `model`.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn load_all(pool: &SqlitePool, model: &str) -> anyhow::Result<Vec<(i64, Vec<f32>)>> {
    let rows: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT title_id, vector FROM title_embeddings WHERE model = ? ORDER BY title_id")
            .bind(model)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(id, b)| (id, decode(&b))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{init_pool, seed::seed_if_empty};

    fn round_trip(v: &[f32]) -> Vec<f32> {
        decode(&encode(v))
    }

    #[test]
    fn encode_decode_round_trips() {
        let v = vec![0.0, -1.5, 3.25, 1024.0];
        assert_eq!(round_trip(&v), v);
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
    async fn missing_then_upsert_then_load() {
        let (pool, _dir) = seeded().await;
        let model = "test-model";

        let missing = missing_for_model(&pool, model).await.unwrap();
        assert_eq!(missing.len(), 28, "all seed titles start unembedded");

        upsert(&pool, missing[0], &[0.1, 0.2, 0.3], model).await.unwrap();

        let still_missing = missing_for_model(&pool, model).await.unwrap();
        assert_eq!(still_missing.len(), 27);

        let loaded = load_all(&pool, model).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0, missing[0]);
        assert_eq!(loaded[0].1, vec![0.1, 0.2, 0.3]);
    }

    #[tokio::test]
    async fn upsert_replaces_existing() {
        let (pool, _dir) = seeded().await;
        let id = missing_for_model(&pool, "m").await.unwrap()[0];
        upsert(&pool, id, &[1.0], "m").await.unwrap();
        upsert(&pool, id, &[2.0, 2.0], "m").await.unwrap();
        let loaded = load_all(&pool, "m").await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].1, vec![2.0, 2.0]);
    }
}
```

- [ ] **Step 3: Run tests and clippy**

Run: `cargo test --lib db::embeddings && cargo clippy --all-targets -- -D warnings`
Expected: PASS; clippy clean.

- [ ] **Step 4: Commit**

```bash
git add src/db/mod.rs src/db/embeddings.rs
git commit -m "feat(ask): persist title embeddings (blob codec + queries)"
```

---

### Task 3: `Embedder` trait + embedding-text builder + `OpenAiEmbedder`

**Files:**
- Modify: `src/services/mod.rs` (add `pub mod embeddings;`)
- Create: `src/services/embeddings.rs` (impl + `#[cfg(test)]`)

**Interfaces:**
- Consumes: `reqwest`, `async_trait`.
- Produces:
  - `cue::services::embeddings::EMBED_MODEL: &str` (= `"text-embedding-3-small"`)
  - trait `cue::services::embeddings::Embedder: Send + Sync` with `async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>>`
  - `cue::services::embeddings::embed_text(title: &str, year: i64, kind: &str, genres: &[String], description: &str) -> String`
  - `cue::services::embeddings::OpenAiEmbedder` with `OpenAiEmbedder::new(api_key: String) -> Self`

- [ ] **Step 1: Register the submodule**

In `src/services/mod.rs`, add:

```rust
pub mod embeddings;
```

(Result: `mod.rs` declares `pub mod similarity;` and `pub mod embeddings;`.)

- [ ] **Step 2: Write the failing test (text builder is the pure, unit-testable part)**

Create `src/services/embeddings.rs`:

```rust
//! OpenAI embeddings client behind the `Embedder` trait so tests run offline.

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

/// Live OpenAI embeddings client.
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
```

- [ ] **Step 3: Run test and clippy**

Run: `cargo test --lib services::embeddings && cargo clippy --all-targets -- -D warnings`
Expected: PASS; clippy clean. (`OpenAiEmbedder::embed` is exercised live in Task 9's manual smoke check, not in unit tests.)

- [ ] **Step 4: Commit**

```bash
git add src/services/mod.rs src/services/embeddings.rs
git commit -m "feat(ask): Embedder trait, embed-text builder, OpenAI client"
```

---

### Task 4: `AskModel` trait + ask schema + answer parsing + `ClaudeAskModel`

**Files:**
- Modify: `src/services/mod.rs` (add `pub mod anthropic;`)
- Create: `src/services/anthropic.rs` (impl + `#[cfg(test)]`)

**Interfaces:**
- Consumes: `reqwest`, `async_trait`.
- Produces:
  - `cue::services::anthropic::ASK_MODEL: &str` (= `"claude-sonnet-4-6"`)
  - struct `cue::services::anthropic::Candidate { pub id: i64, pub title: String, pub year: i64, pub kind: String, pub genres: Vec<String>, pub imdb: Option<f64> }`
  - struct `cue::services::anthropic::AskAnswer { pub ids: Vec<i64>, pub line: String, pub sub: String }`
  - trait `cue::services::anthropic::AskModel: Send + Sync` with `async fn rank(&self, query: &str, candidates: &[Candidate]) -> anyhow::Result<AskAnswer>`
  - `cue::services::anthropic::parse_answer(text: &str) -> anyhow::Result<AskAnswer>`
  - `cue::services::anthropic::ClaudeAskModel` with `ClaudeAskModel::new(api_key: String) -> Self`

- [ ] **Step 1: Register the submodule**

In `src/services/mod.rs`, add:

```rust
pub mod anthropic;
```

- [ ] **Step 2: Write the failing test (parse_answer is the pure, unit-testable part)**

Create `src/services/anthropic.rs`:

```rust
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
    pub imdb: Option<f64>,
}

/// The validated ask result. Field names match the frontend `AskResult`.
#[derive(Debug, Clone, PartialEq)]
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
            let imdb = c.imdb.map_or_else(|| "n/a".to_string(), |r| r.to_string());
            format!(
                "id={} | {} ({}) | {} | {} | imdb {}",
                c.id,
                c.title,
                c.year,
                c.kind,
                c.genres.join("/"),
                imdb
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
        assert_eq!(ask_schema()["additionalProperties"], serde_json::json!(false));
    }
}
```

- [ ] **Step 3: Run tests and clippy**

Run: `cargo test --lib services::anthropic && cargo clippy --all-targets -- -D warnings`
Expected: PASS; clippy clean.

> **Plan-time note:** `output_config` carrying both `effort` and `format` is asserted by the spec but should be confirmed against a live call during Task 9's smoke check. If the API rejects the pair, drop `"effort": "low"` (the default effort applies) and keep `format`.

- [ ] **Step 4: Commit**

```bash
git add src/services/mod.rs src/services/anthropic.rs
git commit -m "feat(ask): AskModel trait, ask schema, answer parsing, Claude client"
```

---

### Task 5: `ask_engine::ask` — retrieval → rank → validate

**Files:**
- Modify: `src/services/mod.rs` (add `pub mod ask_engine;`)
- Create: `src/services/ask_engine.rs` (impl + `#[cfg(test)]`)

**Interfaces:**
- Consumes: `db::embeddings::load_all`, `similarity::rank_by_cosine`, `Embedder`, `AskModel`, `Candidate`, `AskAnswer`, `EMBED_MODEL`.
- Produces:
  - enum `cue::services::ask_engine::AskError` (`Unavailable`, `Other(anyhow::Error)`)
  - struct `cue::services::ask_engine::AskEngine`
  - `AskEngine::new(pool: SqlitePool, embedder: Option<Arc<dyn Embedder>>, model: Option<Arc<dyn AskModel>>, light_axis: Option<Vec<f32>>) -> Self`
  - `async fn AskEngine::ask(&self, query: &str, base_ids: Option<Vec<i64>>) -> Result<AskAnswer, AskError>`
  - (private) `async fn candidates_for(&self, ids: &[i64]) -> anyhow::Result<Vec<Candidate>>`

- [ ] **Step 1: Register the submodule**

In `src/services/mod.rs`, add:

```rust
pub mod ask_engine;
```

- [ ] **Step 2: Write the failing tests**

Create `src/services/ask_engine.rs`:

```rust
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
        let answer = engine
            .ask("cozy", Some(vec![1, 2, 3]))
            .await
            .unwrap();
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
```

- [ ] **Step 3: Run tests and clippy**

Run: `cargo test --lib services::ask_engine && cargo clippy --all-targets -- -D warnings`
Expected: PASS; clippy clean.

- [ ] **Step 4: Commit**

```bash
git add src/services/mod.rs src/services/ask_engine.rs
git commit -m "feat(ask): ask_engine retrieval/rank/validate with offline fakes"
```

---

### Task 6: `ask_engine::similar` and `ask_engine::refine` (no Claude)

**Files:**
- Modify: `src/services/ask_engine.rs` (add two `pub async fn`, helper fns, and tests)

**Interfaces:**
- Consumes: same module's `AskEngine`, `embeddings::load_all`, `similarity`; `AskAnswer`.
- Produces:
  - `async fn AskEngine::similar(&self, anchor_id: i64, base_ids: Option<Vec<i64>>) -> Result<AskAnswer, AskError>`
  - `async fn AskEngine::refine(&self, kind: &str, ids: &[i64]) -> Result<AskAnswer, AskError>`
  - `cue::services::ask_engine::len_minutes(len: &str) -> i64` (pub, for tests)

- [ ] **Step 1: Add the runtime-length helper and a LIGHT genre set at module top**

In `src/services/ask_engine.rs`, below the existing `const CANDIDATE_CAP`, add:

```rust
/// Light-toned genres used as the fallback when no lightness axis/vector exists.
const LIGHT_GENRES: [&str; 5] = ["Comedy", "Animation", "Adventure", "Romance", "Musical"];

/// "164 min" -> 164; "28 eps" -> eps*24 so series sort sensibly among movies.
/// Mirrors the frontend stub's `lenMinutes`.
#[must_use]
pub fn len_minutes(len: &str) -> i64 {
    let n: i64 = len
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    if len.to_ascii_lowercase().contains("eps") {
        n * 24
    } else {
        n
    }
}
```

- [ ] **Step 2: Write the failing tests (append to the existing `#[cfg(test)] mod tests`)**

Add these tests inside the existing `mod tests` block in `src/services/ask_engine.rs`:

```rust
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
```

- [ ] **Step 3: Implement `similar` and `refine` (add as methods on `impl AskEngine`)**

Add inside `impl AskEngine` in `src/services/ask_engine.rs`:

```rust
    /// "More like {title}": cosine over stored vectors, or shared-genre overlap
    /// when the anchor (or candidates) lack vectors.
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
    pub async fn refine(&self, kind: &str, ids: &[i64]) -> Result<AskAnswer, AskError> {
        let (line, out): (&str, Vec<i64>) = match kind {
            "shorter" => ("Shortest first.", self.sort_by_runtime(ids).await?),
            "lighter" => ("Lighter picks.", self.lighter(ids).await?),
            "surprise" => ("A wildcard you might've missed.", self.surprise(ids).await?),
            other => {
                return Err(AskError::Other(anyhow::anyhow!("unknown refine kind: {other}")))
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
        scored.sort_by(|a, b| b.1.cmp(&a.1));
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
        Ok(ids.iter().copied().filter(|id| light.contains(id)).collect())
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
                    let da = (1.0 - similarity::cosine(va, &center)) + rmap[ia] as f32 / 10.0;
                    let db = (1.0 - similarity::cosine(vb, &center)) + rmap[ib] as f32 / 10.0;
                    da.total_cmp(&db)
                })
                .map(|(id, _)| id);
            if let Some(id) = pick {
                return Ok(vec![id]);
            }
        }
        // Fallback: the highest-rated id in the set.
        let pick = ids
            .iter()
            .copied()
            .max_by(|a, b| {
                rmap.get(a)
                    .unwrap_or(&0.0)
                    .total_cmp(rmap.get(b).unwrap_or(&0.0))
            });
        Ok(pick.into_iter().collect())
    }
```

- [ ] **Step 4: Run tests and clippy**

Run: `cargo test --lib services::ask_engine && cargo clippy --all-targets -- -D warnings`
Expected: PASS; clippy clean.

- [ ] **Step 5: Commit**

```bash
git add src/services/ask_engine.rs
git commit -m "feat(ask): similar + refine (cosine/axis/sort, no LLM)"
```

---

### Task 7: Embedding backfill function

**Files:**
- Modify: `src/services/embeddings.rs` (add `backfill` + `build_light_axis` + tests)

**Interfaces:**
- Consumes: `Embedder`, `db::embeddings`, `embed_text`, `EMBED_MODEL`.
- Produces:
  - `async fn cue::services::embeddings::backfill(pool: &SqlitePool, embedder: &dyn Embedder) -> anyhow::Result<usize>` (returns count embedded)
  - `async fn cue::services::embeddings::build_light_axis(embedder: &dyn Embedder) -> anyhow::Result<Vec<f32>>`

- [ ] **Step 1: Write the failing test**

Append to `src/services/embeddings.rs`. First add imports at the top of the file (below the existing `use async_trait::async_trait;`):

```rust
use sqlx::SqlitePool;

use crate::db::embeddings as embed_db;
```

Then add the two functions (before the `#[cfg(test)]` block):

```rust
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
```

Add these tests inside the existing `#[cfg(test)] mod tests` block:

```rust
    use crate::db::{init_pool, seed::seed_if_empty};
    use crate::db::embeddings as embed_db;

    struct SeqEmbedder;
    #[async_trait]
    impl Embedder for SeqEmbedder {
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
```

- [ ] **Step 2: Run tests and clippy**

Run: `cargo test --lib services::embeddings && cargo clippy --all-targets -- -D warnings`
Expected: PASS; clippy clean.

- [ ] **Step 3: Commit**

```bash
git add src/services/embeddings.rs
git commit -m "feat(ask): idempotent embedding backfill + lightness axis builder"
```

---

### Task 8: Routes `/api/ask`, `/api/ask/similar`, `/api/ask/refine`

**Files:**
- Modify: `src/routes/mod.rs` (add `pub mod ask;` and three routes in `configure`)
- Create: `src/routes/ask.rs` (handlers + `#[cfg(test)]`)

**Interfaces:**
- Consumes: `web::Data<Arc<AskEngine>>`, `AskEngine::{ask, similar, refine}`, `AskError`, `AskAnswer`.
- Produces:
  - `cue::routes::ask::ask` / `similar` / `refine` handlers, registered under `/api`.
  - Response JSON shape `{ "line": String, "sub": String, "ids": [i64] }` (matches frontend `AskResult`).

- [ ] **Step 1: Register the module and routes**

In `src/routes/mod.rs`, add `pub mod ask;` at the top and extend `configure`:

```rust
pub mod ask;
pub mod catalogue;

use actix_web::{web, HttpResponse};

async fn health() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({ "status": "ok" }))
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api")
            .route("/health", web::get().to(health))
            .route("/catalogue", web::get().to(catalogue::get_catalogue))
            .route("/ask", web::post().to(ask::ask))
            .route("/ask/similar", web::post().to(ask::similar))
            .route("/ask/refine", web::post().to(ask::refine)),
    );
}
```

- [ ] **Step 2: Write the failing tests**

Create `src/routes/ask.rs`:

```rust
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
        let ids: Vec<i64> = body["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
        assert!(!ids.contains(&1));
    }
}
```

- [ ] **Step 3: Run tests and clippy**

Run: `cargo test --lib routes::ask && cargo clippy --all-targets -- -D warnings`
Expected: PASS; clippy clean.

- [ ] **Step 4: Commit**

```bash
git add src/routes/mod.rs src/routes/ask.rs
git commit -m "feat(ask): /api/ask, /ask/similar, /ask/refine endpoints (503 when unavailable)"
```

---

### Task 9: Compose root — build clients, run backfill, register engine (`main.rs`)

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `Config`, `OpenAiEmbedder`, `ClaudeAskModel`, `services::embeddings::{backfill, build_light_axis}`, `AskEngine`.
- Produces: an `Arc<AskEngine>` registered via `app_data` so the Task 8 handlers resolve it.

- [ ] **Step 1: Build the engine and run the backfill (edit `main.rs`)**

In `src/main.rs`, after the seed block (`tracing::info!("seeded {inserted} titles");`) and before the `HttpServer::new` block, insert:

```rust
    use std::sync::Arc;
    use cue::services::anthropic::ClaudeAskModel;
    use cue::services::ask_engine::AskEngine;
    use cue::services::embeddings::{backfill, build_light_axis, OpenAiEmbedder};

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
        tracing::warn!("OPENAI_API_KEY unset — ask retrieval disabled, refine 'shorter' still works");
        None
    };

    let engine = Arc::new(AskEngine::new(pool.clone(), embedder, model, light_axis));
```

- [ ] **Step 2: Register the engine in the app factory**

In the `HttpServer::new(move || { App::new() ... })` closure in `src/main.rs`, add an `app_data` line for the engine (clone the `Arc` into the closure). The closure already moves `pool`; add `engine` to the moved set by referencing it inside:

```rust
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(engine.clone()))
            .app_data(web::Data::new(StaticDir(static_dir.clone())))
            .configure(cue::routes::configure)
            .default_service(web::route().to(serve_spa))
    })
```

- [ ] **Step 3: Build and verify the whole crate compiles + all tests + clippy**

Run: `cargo build && cargo test && cargo clippy --all-targets -- -D warnings`
Expected: builds; all tests PASS; clippy clean.

- [ ] **Step 4: Manual smoke check (requires real keys in `.env`)**

With `OPENAI_API_KEY` and `ANTHROPIC_API_KEY` set in `.env`, run `cargo run`, watch for `embedded 28 titles`, then in another shell:

```bash
curl -s -X POST localhost:8080/api/ask -H 'content-type: application/json' \
  -d '{"query":"cozy low-stakes animation"}'
```

Expected: `200` with `{"line":…,"sub":…,"ids":[…]}` where every id is a real catalogue id. (If the call 400s on `output_config`, apply the Task 4 plan-time note: drop `"effort":"low"`.) This step has no automated test — it exercises the live OpenAI + Anthropic calls.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat(ask): wire AskEngine, startup embedding backfill, lightness axis"
```

---

### Task 10: Frontend — make `similar()` async across interface, stub, store

**Files:**
- Modify: `frontend/src/services/askService.ts` (interface line 14; `StubAskService.similar` lines 44-52)
- Modify: `frontend/src/stores/catalogue.ts` (`moreLike` action, line 146)
- Modify: `frontend/src/services/__tests__/stub.test.ts` (the `similar()` test, lines 37-41)

**Interfaces:**
- Consumes: existing `AskResult`, `Title` types.
- Produces: `AskService.similar(title: Title, all: Title[]): Promise<AskResult>` (now async); `moreLike` awaits it.

- [ ] **Step 1: Update the failing test first (make the `similar` stub test async)**

In `frontend/src/services/__tests__/stub.test.ts`, change the `similar()` test to await:

```typescript
  it('similar() ranks by shared genre', async () => {
    const r = await svc.similar(cat[0], cat)
    expect(r.ids).toEqual([2])
    expect(r.line).toContain('Frieren')
  })
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cd frontend && npm run test:unit -- stub`
Expected: FAIL — `similar` currently returns a non-thenable `AskResult`, so `await` on a sync value passes, but TypeScript build/type-check will flag the interface once Step 3 lands. (If it still passes at runtime, proceed — the type change in Step 3 is the real driver.)

- [ ] **Step 3: Make the interface and stub async**

In `frontend/src/services/askService.ts`, change the interface method (line 14):

```typescript
  similar(title: Title, all: Title[]): Promise<AskResult>
```

And make the `StubAskService.similar` implementation `async` (it already returns an `AskResult`; just add `async`):

```typescript
  async similar(title: Title, all: Title[]): Promise<AskResult> {
    await Promise.resolve() // keep the await seam consistent with ask/refine
    const g = new Set(title.genres)
    const ids = all
      .filter(t => t.id !== title.id && t.genres.some(x => g.has(x)))
      .map(t => ({ id: t.id, shared: t.genres.filter(x => g.has(x)).length, imdb: t.imdb ?? -Infinity }))
      .sort((a, b) => b.shared - a.shared || b.imdb - a.imdb)
      .map(x => x.id)
    return { line: `More like ${title.title}.`, sub: `${ids.length} · refine or filter to narrow`, ids }
  }
```

- [ ] **Step 4: Await `similar()` in the store**

In `frontend/src/stores/catalogue.ts`, update `moreLike` (line 144-148) to await:

```typescript
    async moreLike(title: Title) {
      this.resolving = true
      try { this.applyResult(`≈ ${title.title}`, await askService.similar(title, this.catalogue)) }
      finally { this.resolving = false }
    },
```

- [ ] **Step 5: Run tests + type-check**

Run: `cd frontend && npm run test:unit && npm run build`
Expected: all Vitest PASS; `vue-tsc` build type-checks with no `similar`-related errors.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/services/askService.ts frontend/src/stores/catalogue.ts frontend/src/services/__tests__/stub.test.ts
git commit -m "refactor(frontend): make AskService.similar async"
```

---

### Task 11: Frontend — `ApiAskService` + swap the seam

**Files:**
- Modify: `frontend/src/services/askService.ts` (add `ApiAskService`)
- Modify: `frontend/src/services/index.ts` (swap the export)
- Create: `frontend/src/services/__tests__/api.test.ts`

**Interfaces:**
- Consumes: `AskService`, `AskResult`, `Title`; `fetch`; endpoints `/api/ask`, `/api/ask/similar`, `/api/ask/refine`.
- Produces: `ApiAskService implements AskService`; the `askService` singleton now backed by the API.

- [ ] **Step 1: Write the failing tests**

Create `frontend/src/services/__tests__/api.test.ts`:

```typescript
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { ApiAskService } from '../askService'
import type { Title } from '@/types'

const t = (id: number, title: string): Title => ({
  id, imdbId: null, title, year: 2020, services: ['plex'], type: 'movie',
  genres: ['Drama'], imdb: 7, len: '100 min', desc: '', cast: [], watched: false, rating: null,
})

describe('ApiAskService', () => {
  beforeEach(() => { vi.restoreAllMocks() })

  it('ask() posts the query and returns the parsed result', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ line: 'Here you go.', sub: '1 · narrow', ids: [2] }),
    })
    vi.stubGlobal('fetch', fetchMock)
    const svc = new ApiAskService()
    const r = await svc.ask('cozy', [t(1, 'A'), t(2, 'B')])
    expect(r.ids).toEqual([2])
    expect(fetchMock).toHaveBeenCalledWith('/api/ask', expect.objectContaining({ method: 'POST' }))
  })

  it('similar() posts anchorId and returns the parsed result', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ line: 'More like A.', sub: '1 · narrow', ids: [2] }),
    })
    vi.stubGlobal('fetch', fetchMock)
    const svc = new ApiAskService()
    const r = await svc.similar(t(1, 'A'), [t(1, 'A'), t(2, 'B')])
    expect(r.ids).toEqual([2])
    const [, opts] = fetchMock.mock.calls[0]
    expect(JSON.parse(opts.body)).toEqual({ anchorId: 1 })
  })

  it('throws a clear error on 503 so the store can surface "unavailable"', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status: 503 }))
    const svc = new ApiAskService()
    await expect(svc.ask('x', [])).rejects.toThrow(/unavailable|503/i)
  })

  it('rejects a malformed body (missing ids)', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true, json: async () => ({ line: 'x', sub: 'y' }),
    }))
    const svc = new ApiAskService()
    await expect(svc.ask('x', [])).rejects.toThrow(/ids/i)
  })
})
```

- [ ] **Step 2: Run to confirm failure**

Run: `cd frontend && npm run test:unit -- api`
Expected: FAIL — `ApiAskService` is not exported yet.

- [ ] **Step 3: Implement `ApiAskService`**

Append to `frontend/src/services/askService.ts`:

```typescript
function isAskResult(v: unknown): v is AskResult {
  if (typeof v !== 'object' || v === null) return false
  const r = v as Record<string, unknown>
  return Array.isArray(r.ids) && r.ids.every(n => typeof n === 'number')
    && typeof r.line === 'string' && typeof r.sub === 'string'
}

async function postAsk(path: string, payload: unknown): Promise<AskResult> {
  const res = await fetch(path, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(payload),
  })
  if (res.status === 503) throw new Error('ask is unavailable')
  if (!res.ok) throw new Error(`ask failed (HTTP ${res.status})`)
  const data: unknown = await res.json()
  if (!isAskResult(data)) throw new Error('malformed ask response: missing ids/line/sub')
  return data
}

export class ApiAskService implements AskService {
  ask(query: string, _base: Title[]): Promise<AskResult> {
    return postAsk('/api/ask', { query })
  }

  refine(kind: 'lighter' | 'shorter' | 'surprise', current: Title[]): Promise<AskResult> {
    return postAsk('/api/ask/refine', { kind, ids: current.map(t => t.id) })
  }

  similar(title: Title, _all: Title[]): Promise<AskResult> {
    return postAsk('/api/ask/similar', { anchorId: title.id })
  }
}
```

> Note: `ask` sends only `{query}` (no `baseIds`) for the v1 fresh-ask flow; thread-scoped refine routes through `/api/ask/refine`. The `_base`/`_all` params are kept to satisfy the interface signature.

- [ ] **Step 4: Swap the seam**

Replace the body of `frontend/src/services/index.ts`:

```typescript
import { ApiAskService } from './askService'
import type { AskService } from './askService'

export const askService: AskService = new ApiAskService()
```

- [ ] **Step 5: Run tests + type-check**

Run: `cd frontend && npm run test:unit && npm run build`
Expected: all PASS; build type-checks.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/services/askService.ts frontend/src/services/index.ts frontend/src/services/__tests__/api.test.ts
git commit -m "feat(frontend): ApiAskService backed by /api/ask*; swap the service seam"
```

---

### Task 12: Frontend — validate `/api/catalogue` response (drop the `as Title[]` cast)

**Files:**
- Modify: `frontend/src/api/client.ts` (line 8)
- Modify/Create: `frontend/src/api/__tests__/client.test.ts`

**Interfaces:**
- Consumes: `Title`, `ServiceKey`, `TitleKind` from `@/types`.
- Produces: `getCatalogue(): Promise<Title[]>` that validates the shape instead of casting.

- [ ] **Step 1: Write the failing test**

Create `frontend/src/api/__tests__/client.test.ts`:

```typescript
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { getCatalogue } from '../client'

const validTitle = {
  id: 1, imdbId: null, title: 'A', year: 2020, services: ['plex'], type: 'movie',
  genres: ['Drama'], imdb: 7, len: '100 min', desc: '', cast: [], watched: false, rating: null,
}

describe('getCatalogue', () => {
  beforeEach(() => { vi.restoreAllMocks() })

  it('returns validated titles on a well-formed response', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => [validTitle] }))
    const out = await getCatalogue()
    expect(out).toHaveLength(1)
    expect(out[0].id).toBe(1)
  })

  it('throws when the body is not an array', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => ({}) }))
    await expect(getCatalogue()).rejects.toThrow(/array|invalid/i)
  })

  it('throws when an item is missing required fields', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => [{ id: 1 }] }))
    await expect(getCatalogue()).rejects.toThrow(/invalid|title/i)
  })
})
```

- [ ] **Step 2: Run to confirm failure**

Run: `cd frontend && npm run test:unit -- client`
Expected: FAIL — the current `getCatalogue` casts and does not throw on bad shapes.

- [ ] **Step 3: Replace the cast with validation**

Replace the contents of `frontend/src/api/client.ts`:

```typescript
import type { Title } from '@/types'

function isTitle(v: unknown): v is Title {
  if (typeof v !== 'object' || v === null) return false
  const r = v as Record<string, unknown>
  return typeof r.id === 'number'
    && typeof r.title === 'string'
    && typeof r.year === 'number'
    && (r.type === 'movie' || r.type === 'series')
    && Array.isArray(r.services)
    && Array.isArray(r.genres)
    && Array.isArray(r.cast)
    && typeof r.len === 'string'
    && typeof r.desc === 'string'
    && typeof r.watched === 'boolean'
}

export async function getCatalogue(): Promise<Title[]> {
  const res = await fetch('/api/catalogue')
  if (!res.ok) {
    throw new Error(`Failed to load catalogue (HTTP ${res.status})`)
  }
  const data: unknown = await res.json()
  if (!Array.isArray(data)) {
    throw new Error('catalogue response is not an array')
  }
  if (!data.every(isTitle)) {
    throw new Error('catalogue response contains an invalid title')
  }
  return data
}
```

- [ ] **Step 4: Run all frontend tests + type-check**

Run: `cd frontend && npm run test:unit && npm run build`
Expected: all PASS; build type-checks.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/api/client.ts frontend/src/api/__tests__/client.test.ts
git commit -m "feat(frontend): validate /api/catalogue response instead of casting"
```

---

## Final verification (after all tasks)

- [ ] Backend: `cargo test && cargo clippy --all-targets -- -D warnings` — all green.
- [ ] Frontend: `cd frontend && npm run test:unit && npm run build` — all green.
- [ ] Manual: with keys set, `cargo run` logs `embedded 28 titles`; `POST /api/ask` returns in-catalogue ids; the Vue ask bar, per-card "More like", and refine chips all drive real endpoints.

## Self-review notes (coverage vs spec)

- Spec §4 modules → Tasks 1–7 (similarity, embeddings client+backfill, anthropic, ask_engine, db/embeddings). ✓
- Spec §5 endpoints + degraded 503 → Task 8 (503 tests) + Task 9 (key-gated wiring). ✓
- Spec §6 startup backfill → Tasks 7 + 9. ✓
- Spec §7 frontend (async `similar`, `ApiAskService`, seam, `as Title[]` removal) → Tasks 10–12. ✓
- Spec §8 testability (trait fakes, no network) → fakes in Tasks 5–7; mocked `fetch` in 11–12. ✓
- Spec §9 deps (`reqwest` rustls, no new env) → Task 1. ✓
- Deferred items (LLM chips, "not on your services") → intentionally not implemented.
