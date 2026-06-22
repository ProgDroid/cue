# cue — Ask Engine Design Spec (Plan 3)

**Date:** 2026-06-22
**Status:** Approved (brainstorming) — pending implementation plan
**Author:** Fernando Ferreira (with Claude)
**Builds on:** `docs/superpowers/specs/2026-06-21-cue-design.md` (decisions D1–D9, §5 Ask engine, §7 API surface, §10 directory tree)

## 1. Overview

Plan 3 makes the ask bar real. Plans 1–2 shipped the backend foundation and the
full integrated-ask UI wired through an `AskService` interface backed by a
deterministic client-side `StubAskService`. This plan replaces that stub with a
server-backed implementation and builds the Rust ask engine behind it:
retrieval-augmented ranking over the personal catalogue (OpenAI embeddings →
cosine retrieval → Claude structured output), plus pure-math endpoints for
"similar" and the refine chips.

The architecture stays strictly in-catalogue: embeddings find candidate titles by
semantic similarity, then Claude is constrained (server-validated) to recommend only
from those candidates — it can never surface a title the user doesn't own.

This realises design decisions **D2** (retrieval → LLM ranks), **D3** (OpenAI
`text-embedding-3-small`), **D4** (Claude structured outputs), and **D5**
(`claude-sonnet-4-6`, thinking disabled, effort low) from the parent spec.

## 2. Scope decisions (this plan)

| # | Decision | Choice | Rationale |
|---|---|---|---|
| A1 | Which `AskService` methods go server-side | **All three** (`ask`, `refine`, `similar`) | User choice. Moving `similar` server-side gains true embedding cosine (the client only has genres). |
| A2 | Does Claude power `refine`/`similar`? | **No — only `ask` calls Claude** | Matches parent-spec §3 boundary principle ("only the natural-language ask bar calls Claude; similarity is pure vector math"). Refine/similar are deterministic, instant, free, and unit-testable. |
| A3 | `refine` chip logic | `shorter` = runtime sort; `lighter` = embedding lightness-axis; `surprise` = embedding outlier + high rating | `lighter` from embeddings beats genre-bucket filtering (reads tone from the description). No LLM cost on any chip. |
| A4 | Catalogue embedding trigger | **Startup backfill** — embed any title lacking a current-model vector; idempotent; skip+log if no key | Automatic, zero manual steps; the per-title embed routine is exactly what Plan 4's sync reuses. |
| A5 | HTTP client | **Raw `reqwest`** to the REST APIs | Rust has no official Anthropic/OpenAI SDK; raw HTTP is the sanctioned path. |
| A6 | External-call testability | **Trait seams** (`Embedder`, `AskModel`) injected like `Config` | Tests/CI run with no keys and never touch the network. |

## 3. Architecture & request flow

```
Vue SPA ──REST──> Actix /api/ask*  ──> ask_engine
                                         ├── embeddings (OpenAI)  ── query vector
                                         ├── similarity (Rust)    ── cosine rank → top ~150
                                         └── anthropic (Claude)   ── rank candidates → {ids,line,sub}
                  /api/ask/similar  ──> similarity (cosine over title_embeddings, NO Claude)
                  /api/ask/refine   ──> similarity / sort helpers (NO Claude)

main.rs startup ──> embeddings backfill ── embed titles missing a current-model vector
```

**Boundary principle (inherited):** Vue only ever talks to cue's own REST API and
never sees an external key. Only `POST /api/ask` reaches Claude. `similar` and the
refine chips are pure server-side math over the stored vectors.

## 4. Backend modules (declared in `src/lib.rs`)

Following the parent spec's §10 tree:

- **`src/services/embeddings.rs`** — `Embedder` trait + `OpenAiEmbedder` impl.
  - `OpenAiEmbedder` calls `POST https://api.openai.com/v1/embeddings` via `reqwest`
    with `{ "model": "text-embedding-3-small", "input": [<text>, ...] }`; reads
    `data[].embedding` (1536 `f32`).
  - Embedding text per title: a compact composition of `title`, `year`, `type`,
    `genres`, and `description` (the same fields shown to Claude, plus the blurb).
  - Trait shape (illustrative): `async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>`.
- **`src/services/anthropic.rs`** — `AskModel` trait + `ClaudeAskModel` impl.
  - Calls `POST https://api.anthropic.com/v1/messages` with headers
    `x-api-key`, `anthropic-version: 2023-06-01`, `content-type: application/json`.
  - Body: `model: "claude-sonnet-4-6"`, `max_tokens: 1024`,
    `thinking: {type: "disabled"}`,
    `output_config: {effort: "low", format: {type: "json_schema", schema: <ASK_SCHEMA>}}`,
    `messages: [{role:"user", content: <prompt with candidate list + query>}]`.
  - `ASK_SCHEMA` constrains the reply to
    `{ "type":"object", "properties": {"ids":{"type":"array","items":{"type":"integer"}}, "line":{"type":"string"}, "sub":{"type":"string"}}, "required":["ids","line","sub"], "additionalProperties": false }`.
  - The single text block of the response is parsed as JSON against that shape.
  - Trait shape (illustrative): `async fn rank(&self, query: &str, candidates: &[Candidate]) -> Result<AskAnswer>`.
- **`src/services/similarity.rs`** — pure functions, no I/O, no async:
  - `cosine(a: &[f32], b: &[f32]) -> f32`.
  - `lightness_axis() -> Vec<f32>` style helper: the difference of two fixed anchor
    embeddings ("light-hearted, feel-good, funny, warm" minus "dark, bleak, heavy,
    serious"). Anchors are produced once via the `Embedder` at startup and held in
    state (not recomputed per request); `similarity.rs` only does the projection math.
  - `rank_by_axis`, `surprise_pick` (highest-rated title farthest from the
    centroid of the current set), `top_n_by_cosine`.
- **`src/services/ask_engine.rs`** — orchestrator holding `Arc<dyn Embedder>` +
  `Arc<dyn AskModel>` + a DB handle:
  - `ask(query, base_ids)`:
    1. If `base_ids` is `Some` and non-empty → candidates = those titles (skip retrieval).
       Else embed `query`, cosine-rank the whole catalogue, take top ~150.
    2. Build compact `Candidate { id, title, year, type, genres, imdb }` payload.
    3. `AskModel::rank` → `{ids, line, sub}`.
    4. Drop any `id` not in the candidate set; return.
  - `similar(anchor_id, base_ids)`: cosine of the anchor's stored vector against the
    catalogue (or `base_ids` subset); no Claude. Fall back to shared-genre overlap if
    the anchor (or a candidate) has no vector.
  - `refine(kind, ids)`: `shorter` (parse runtime → sort), `lighter` (lightness-axis
    projection over `ids`), `surprise` (outlier pick over `ids`). Each returns
    `{ids, line, sub}` with a templated `line`.
- **`src/db/embeddings.rs`** — `title_embeddings` read/write:
  - `missing_for_model(model) -> Vec<title_id>`, `upsert(title_id, vector, model, dims)`,
    `load_all(model) -> Vec<(title_id, Vec<f32>)>`. Vectors stored as `BLOB` (little-endian
    `f32` bytes); `model`/`dims` columns already exist in `migrations/0001_init.sql`.

## 5. Endpoints (added to `src/routes/`)

All under the existing no-op auth slot. Title resources addressed by surrogate `id`.

| Method | Path | Request | Response | Claude? |
|---|---|---|---|---|
| POST | `/api/ask` | `{ query: string, baseIds?: number[] }` | `{ line, sub, ids }` | Yes |
| POST | `/api/ask/similar` | `{ anchorId: number, baseIds?: number[] }` | `{ line, sub, ids }` | No |
| POST | `/api/ask/refine` | `{ kind: "lighter"\|"shorter"\|"surprise", ids: number[] }` | `{ line, sub, ids }` | No |

`{ line, sub, ids }` matches the frontend `AskResult` type exactly (Claude's schema
field `line` carries the answer sentence; `sub` the count/hint line).

**Degraded behaviour:**
- `OPENAI_API_KEY` unset → startup backfill skips with a log line; `/api/ask` and
  `/api/ask/similar` return `503` with a clear message ("ask is unavailable — no
  embeddings"). `ANTHROPIC_API_KEY` unset → `/api/ask` returns `503`.
- Title missing a vector → `similar`/`lighter`/`surprise` fall back to shared-genre
  overlap for that title rather than erroring.
- `shorter` never needs embeddings or keys; always works.
- The frontend surfaces a `503` as an "ask unavailable" state in the existing answer
  context (no crash, no console error-swallow).

## 6. Startup backfill (`src/main.rs`)

After DB init, before serving: if an `Embedder` is configured (key present), call
`db::embeddings::missing_for_model("text-embedding-3-small")`, embed those titles in
batches, and `upsert` the vectors. Idempotent — a second boot with a fully-embedded
catalogue is a no-op query. Logged at info level (count embedded / skipped). The
lightness anchor vectors are computed here too and held in shared state. With no key,
the whole step is skipped with a single warning line; the server still serves the
catalogue and the non-LLM, non-embedding paths (`shorter`).

This per-title embed routine is the unit Plan 4's sync subsystem will call for
new/changed titles.

## 7. Frontend changes (`frontend/`)

- **`src/services/askService.ts`** — the `AskService` interface's `similar` becomes
  `async` (`similar(title, all): Promise<AskResult>`). `StubAskService.similar`
  gains `async`/`await Promise.resolve()` to match (kept for tests/offline).
- **New `ApiAskService implements AskService`** — `ask`/`refine`/`similar` each
  `fetch` the corresponding endpoint and return the parsed, validated `AskResult`.
- **`src/services/index.ts`** — swap the one-line seam:
  `export const askService: AskService = new ApiAskService()`.
- **Store + call sites** — wherever `similar()` is consumed (`DetailView`,
  `PosterCard` "More like {title}", and the store action), add `await`. This is the
  accepted ripple from moving `similar` server-side; the rest of the store/components
  stay untouched because `ask`/`refine` were already async.
- **`src/api/client.ts`** — replace the `as Title[]` cast on `GET /api/catalogue`
  with real response validation (shape-check the array and each `TitleDto`), folding
  in the deferred follow-up.

## 8. Testability

- **Backend:** `ask_engine` takes `Arc<dyn Embedder>` and `Arc<dyn AskModel>`; tests
  inject fakes (a fixed-vector embedder, a canned-answer model) and assert: candidate
  capping, `baseIds` short-circuit, `ids ⊆ candidates` validation, the three refine
  kinds, and the genre-overlap fallback. `similarity.rs` is pure and unit-tested
  directly (cosine, lightness projection, surprise pick). Test DBs follow the project
  Windows pattern (`tempfile::tempdir()`, backslash-fixed `sqlite:` URL). No test
  touches the network.
- **Frontend:** Vitest covers `ApiAskService` against a mocked `fetch` (success,
  `503` degraded state, and `ids` validation), plus the now-async `similar` wiring.

## 9. Config / dependencies

- No new env vars — `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` already exist in
  `Config` (Plan 1) and `.env.example`. Both remain server-side only, never
  serialised to the client.
- **`Cargo.toml`:** add `reqwest` with `rustls-tls` + `json` features (no native
  OpenSSL). Honour the canonical `[lints.clippy]` table; per-item `#[allow]` with a
  one-line reason only where unavoidable. Gate: `cargo clippy --all-targets -- -D warnings`.

## 10. Out of scope / deferred

- **LLM-generated dynamic refine chips** — chips synthesised by Claude from the
  on-screen result set instead of the three fixed kinds. Clean add-on on this same
  backbone; revisit after Plan 3. (Changes the frontend chip contract, so deferred.)
- **"Not on your services" discovery row** — already deferred in the parent spec;
  unchanged.
- **Plan 4 (catalogue sync)** reuses the per-title embed routine but is otherwise out
  of scope here.
- **Plan 5 (user-data writes)** — ratings/watched persistence unchanged by this plan.

## 11. Open items to confirm during planning

- Exact `reqwest` error → HTTP status mapping (timeout/4xx/5xx from OpenAI or
  Anthropic) and retry policy (likely none in v1; surface as `503`).
- Candidate prompt wording and token budget for the Claude `messages` content
  (compact candidate list; verify it fits comfortably under context for ~150 titles).
- Batch size / rate-limit handling for the startup backfill against the OpenAI
  embeddings endpoint.
