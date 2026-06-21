# cue — Backend Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the Rust/Actix backend so it boots, applies the SQLite schema, seeds a 28-title dev catalogue, serves it at `GET /api/catalogue`, serves the (future) Vue SPA as static files, and runs in Docker.

**Architecture:** A single Actix-web service owns a SQLite database via an SQLx pool. Schema lives in an embedded migration applied at startup. A seeder populates dev data when the catalogue is empty. Domain rows are assembled into a frontend-shaped `TitleDto` JSON. Actix serves `/api/*` and falls back to the SPA's `index.html` for all other routes. Everything is wrapped in a multi-stage Docker image (the Vue build stage is added in the frontend plan).

**Tech Stack:** Rust, Actix-web 4, actix-files 0.6, SQLx 0.8 (SQLite, runtime-tokio), Tokio, Serde, tracing, Docker.

## Global Constraints

- **Backend stack:** Rust + Actix-web + SQLx + SQLite. No ORM beyond SQLx.
- **SQLx usage:** Use **runtime** query functions (`sqlx::query`, `sqlx::query_as`) — **not** the compile-time `query!`/`query_as!` macros — so the project builds without a live `DATABASE_URL` or offline cache.
- **Migrations:** embedded via `sqlx::migrate!("./migrations")`; applied at startup.
- **Personal-data key:** `user_ratings` / `watch_history` are keyed on `imdb_id` (canonical key; `imdb_id` is `NULL` for the dev seed — those rows simply have no rating/watched until the user-data plan adds fallback keys).
- **Secrets:** all external keys come from env; `.env` and `data/` are gitignored and never committed; no secret is ever sent to the frontend.
- **Service identifiers** (must match `design_handoff_cue/tokens.ts`): `plex`, `disney`, `crunchyroll`.
- **Title type values:** `movie`, `series`.
- **Clippy:** `Cargo.toml` carries the canonical `[lints.clippy]` table (pedantic + nursery) per project Rust standards; code must pass `cargo clippy --all-targets -- -D warnings`.
- **Bind address:** default `127.0.0.1:8080` (localhost-only); Docker overrides to `0.0.0.0:8080`.

---

### Task 1: Cargo scaffold + config module

**Files:**
- Create: `Cargo.toml`
- Create: `.gitignore`
- Create: `.env.example`
- Create: `src/config.rs`
- Create: `src/lib.rs` (library crate root; declares `pub mod config;`)
- Create: `src/main.rs` (thin binary consuming the lib crate; replaced in Task 6)
- Test: inline `#[cfg(test)]` module in `src/config.rs`

The crate is a **library + binary**: modules live in `src/lib.rs` (so `cargo test --lib` works), and `src/main.rs` consumes them via the `cue::` crate path.

**Interfaces:**
- Produces: `Config` struct with public fields `bind_addr: String`, `database_url: String`, `static_dir: String`, and `Option<String>` fields `anthropic_api_key`, `openai_api_key`, `motn_api_key`, `plex_url`, `plex_token`, `region`, `sync_cron`.
- Produces: `Config::load(get: impl Fn(&str) -> Option<String>) -> Config` (pure, testable) and `Config::from_env() -> Config` (wraps `std::env::var`).
- Produces: `Config::sqlite_path(&self) -> Option<std::path::PathBuf>` — the on-disk DB file path (`None` for `sqlite::memory:`).

- [ ] **Step 1: Create `Cargo.toml`**

```toml
[package]
name = "cue"
version = "0.1.0"
edition = "2021"

[dependencies]
actix-web = "4"
actix-files = "0.6"
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "migrate"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
thiserror = "2"
dotenvy = "0.15"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
tempfile = "3"

[lints.clippy]
pedantic = { level = "warn", priority = -1 }
nursery = { level = "warn", priority = -1 }
```

- [ ] **Step 2: Create `.gitignore`**

```gitignore
/target
/data
.env
node_modules
/frontend/dist
*.db
*.db-journal
*.db-wal
```

- [ ] **Step 3: Create `.env.example`**

```dotenv
# cue configuration — copy to .env and fill in. NEVER commit .env.
BIND_ADDR=127.0.0.1:8080
DATABASE_URL=sqlite:./data/cue.db
STATIC_DIR=frontend/dist

# External services (consumed by later plans)
ANTHROPIC_API_KEY=
OPENAI_API_KEY=
MOTN_API_KEY=
PLEX_URL=
PLEX_TOKEN=
REGION=uk
# 6-field cron (sec min hour dom mon dow) for tokio-cron-scheduler — daily 04:00
SYNC_CRON=0 0 4 * * *
```

- [ ] **Step 4: Write the failing test in `src/config.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn getter(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |k: &str| map.get(k).map(|v| (*v).to_string())
    }

    #[test]
    fn defaults_apply_when_unset() {
        let cfg = Config::load(getter(HashMap::new()));
        assert_eq!(cfg.bind_addr, "127.0.0.1:8080");
        assert_eq!(cfg.database_url, "sqlite:./data/cue.db");
        assert_eq!(cfg.static_dir, "frontend/dist");
        assert!(cfg.anthropic_api_key.is_none());
    }

    #[test]
    fn env_overrides_defaults() {
        let mut m = HashMap::new();
        m.insert("BIND_ADDR", "0.0.0.0:9000");
        m.insert("ANTHROPIC_API_KEY", "sk-test");
        let cfg = Config::load(getter(m));
        assert_eq!(cfg.bind_addr, "0.0.0.0:9000");
        assert_eq!(cfg.anthropic_api_key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn sqlite_path_strips_scheme_and_query() {
        let cfg = Config::load(getter(HashMap::from([("DATABASE_URL", "sqlite:./data/cue.db?mode=rwc")])));
        assert_eq!(cfg.sqlite_path(), Some(std::path::PathBuf::from("./data/cue.db")));
    }

    #[test]
    fn sqlite_path_is_none_for_memory() {
        let cfg = Config::load(getter(HashMap::from([("DATABASE_URL", "sqlite::memory:")])));
        assert_eq!(cfg.sqlite_path(), None);
    }
}
```

- [ ] **Step 5: Run the test to verify it fails**

Run: `cargo test --lib config`
Expected: FAIL to compile — `Config` / `Config::load` not defined.

- [ ] **Step 6: Implement `Config` in `src/config.rs` (above the test module)**

```rust
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: String,
    pub database_url: String,
    pub static_dir: String,
    pub anthropic_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub motn_api_key: Option<String>,
    pub plex_url: Option<String>,
    pub plex_token: Option<String>,
    pub region: Option<String>,
    pub sync_cron: Option<String>,
}

impl Config {
    pub fn load(get: impl Fn(&str) -> Option<String>) -> Self {
        let required = |key: &str, default: &str| get(key).unwrap_or_else(|| default.to_string());
        Self {
            bind_addr: required("BIND_ADDR", "127.0.0.1:8080"),
            database_url: required("DATABASE_URL", "sqlite:./data/cue.db"),
            static_dir: required("STATIC_DIR", "frontend/dist"),
            anthropic_api_key: get("ANTHROPIC_API_KEY"),
            openai_api_key: get("OPENAI_API_KEY"),
            motn_api_key: get("MOTN_API_KEY"),
            plex_url: get("PLEX_URL"),
            plex_token: get("PLEX_TOKEN"),
            region: get("REGION"),
            sync_cron: get("SYNC_CRON"),
        }
    }

    pub fn from_env() -> Self {
        Self::load(|k| std::env::var(k).ok())
    }

    /// On-disk path of the SQLite file, or `None` for an in-memory database.
    pub fn sqlite_path(&self) -> Option<PathBuf> {
        let rest = self.database_url.strip_prefix("sqlite:")?;
        if rest.starts_with(":memory:") || rest.is_empty() {
            return None;
        }
        let path = rest.split('?').next().unwrap_or(rest);
        Some(PathBuf::from(path))
    }
}
```

- [ ] **Step 7: Create `src/lib.rs` and a thin `src/main.rs`**

`src/lib.rs`:

```rust
pub mod config;
```

`src/main.rs` (consumes the lib crate so it compiles and links):

```rust
use cue::config::Config;

fn main() {
    let cfg = Config::from_env();
    println!("cue config loaded: bind={}", cfg.bind_addr);
}
```

- [ ] **Step 8: Run tests and clippy**

Run: `cargo test --lib config` → Expected: PASS (4 tests).
Run: `cargo clippy --all-targets -- -D warnings` → Expected: no warnings.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore .env.example src/config.rs src/lib.rs src/main.rs
git commit -m "feat(backend): cargo scaffold and env-driven config"
```

---

### Task 2: SQLite schema migration + pool initialization

**Files:**
- Create: `migrations/0001_init.sql`
- Create: `src/db/mod.rs`
- Modify: `src/main.rs` (add `mod db;`)
- Test: inline `#[cfg(test)]` module in `src/db/mod.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks (takes a `&str` URL).
- Produces: `db::init_pool(database_url: &str) -> anyhow::Result<sqlx::SqlitePool>` — opens (creating the file if missing), enforces foreign keys, runs embedded migrations, returns the pool.

- [ ] **Step 1: Create `migrations/0001_init.sql`**

```sql
CREATE TABLE titles (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    imdb_id      TEXT UNIQUE,
    tmdb_id      TEXT,
    plex_guid    TEXT,
    title        TEXT NOT NULL,
    year         INTEGER NOT NULL,
    type         TEXT NOT NULL CHECK (type IN ('movie','series')),
    imdb_rating  REAL,
    length       TEXT NOT NULL DEFAULT '',
    description  TEXT NOT NULL DEFAULT '',
    added_at     TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE title_services (
    title_id  INTEGER NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    service   TEXT NOT NULL CHECK (service IN ('plex','disney','crunchyroll')),
    PRIMARY KEY (title_id, service)
);

CREATE TABLE title_genres (
    title_id  INTEGER NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    genre     TEXT NOT NULL,
    PRIMARY KEY (title_id, genre)
);

CREATE TABLE title_cast (
    title_id  INTEGER NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    person    TEXT NOT NULL,
    ord       INTEGER NOT NULL,
    PRIMARY KEY (title_id, ord)
);

CREATE TABLE title_embeddings (
    title_id  INTEGER PRIMARY KEY REFERENCES titles(id) ON DELETE CASCADE,
    vector    BLOB NOT NULL,
    model     TEXT NOT NULL,
    dims      INTEGER NOT NULL
);

CREATE TABLE user_ratings (
    imdb_id   TEXT PRIMARY KEY,
    rating    INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 5),
    rated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE watch_history (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    imdb_id     TEXT NOT NULL,
    watched_at  TEXT NOT NULL DEFAULT (datetime('now')),
    source      TEXT NOT NULL CHECK (source IN ('manual','plex')),
    season      INTEGER,
    episode     INTEGER
);

CREATE TABLE sync_runs (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    source       TEXT NOT NULL,
    started_at   TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at  TEXT,
    status       TEXT NOT NULL,
    item_count   INTEGER NOT NULL DEFAULT 0,
    error        TEXT
);

CREATE INDEX idx_title_services_service ON title_services(service);
CREATE INDEX idx_title_genres_genre ON title_genres(genre);
CREATE INDEX idx_watch_history_imdb ON watch_history(imdb_id);
```

- [ ] **Step 2: Write the failing test in `src/db/mod.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_create_expected_tables() {
        // tempdir (not NamedTempFile) so no handle is held open on the db file —
        // required on Windows. Swap backslashes so the sqlite: URL parses.
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();

        let names: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        for expected in [
            "title_cast", "title_embeddings", "title_genres", "title_services",
            "titles", "user_ratings", "watch_history", "sync_runs",
        ] {
            assert!(names.contains(&expected.to_string()), "missing table {expected}");
        }
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --lib db`
Expected: FAIL to compile — `init_pool` not defined.

- [ ] **Step 4: Implement `src/db/mod.rs`**

```rust
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

/// Open the SQLite pool (creating the file if missing), enforce foreign keys,
/// and run embedded migrations.
pub async fn init_pool(database_url: &str) -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePool::connect_with(opts).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
```

- [ ] **Step 5: Add `pub mod db;` to `src/lib.rs`**

The crate is a library (`src/lib.rs`) plus a thin binary (`src/main.rs`); modules are declared in `src/lib.rs`. Add:

```rust
pub mod config;
pub mod db;
```

- [ ] **Step 6: Run the test and clippy**

Run: `cargo test --lib db` → Expected: PASS.
Run: `cargo clippy --all-targets -- -D warnings` → Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add migrations/0001_init.sql src/db/mod.rs src/lib.rs
git commit -m "feat(backend): sqlite schema migration and pool init"
```

---

### Task 3: Domain models + DTO serialization

**Files:**
- Create: `src/models.rs`
- Modify: `src/main.rs` (add `mod models;`)
- Test: inline `#[cfg(test)]` module in `src/models.rs`

**Interfaces:**
- Produces: `Service` enum (`Plex`/`Disney`/`Crunchyroll`, serde `lowercase`) with `Service::parse(&str) -> Option<Service>`.
- Produces: `TitleKind` enum (`Movie`/`Series`, serde `lowercase`) with `TitleKind::parse(&str) -> Option<TitleKind>`.
- Produces: `TitleRow` (`sqlx::FromRow`) mirroring the `titles` columns `id, imdb_id, title, year, type→kind, imdb_rating, length, description`.
- Produces: `TitleDto` (serde `Serialize`) with the exact frontend field names: `id, imdbId, title, year, services, type, genres, imdb, len, desc, cast, watched, rating`.

- [ ] **Step 1: Write the failing test in `src/models.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dto_serializes_with_frontend_field_names() {
        let dto = TitleDto {
            id: 7,
            imdb_id: Some("tt0096895".to_string()),
            title: "Blade Runner 2049".to_string(),
            year: 2017,
            services: vec![Service::Plex],
            kind: TitleKind::Movie,
            genres: vec!["Sci-Fi".to_string(), "Drama".to_string()],
            imdb: Some(8.0),
            len: "164 min".to_string(),
            desc: "A replicant blade runner...".to_string(),
            cast: vec!["Ryan Gosling".to_string()],
            watched: false,
            rating: None,
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["type"], "movie");
        assert_eq!(v["imdbId"], "tt0096895");
        assert_eq!(v["services"], serde_json::json!(["plex"]));
        assert_eq!(v["imdb"], 8.0);
        assert_eq!(v["rating"], serde_json::Value::Null);
    }

    #[test]
    fn enum_parsing_round_trips() {
        assert_eq!(Service::parse("crunchyroll"), Some(Service::Crunchyroll));
        assert_eq!(Service::parse("nope"), None);
        assert_eq!(TitleKind::parse("series"), Some(TitleKind::Series));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib models`
Expected: FAIL to compile — types not defined.

- [ ] **Step 3: Implement `src/models.rs` (above the test module)**

```rust
use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    Plex,
    Disney,
    Crunchyroll,
}

impl Service {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "plex" => Some(Self::Plex),
            "disney" => Some(Self::Disney),
            "crunchyroll" => Some(Self::Crunchyroll),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TitleKind {
    Movie,
    Series,
}

impl TitleKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "movie" => Some(Self::Movie),
            "series" => Some(Self::Series),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct TitleRow {
    pub id: i64,
    pub imdb_id: Option<String>,
    pub title: String,
    pub year: i64,
    #[sqlx(rename = "type")]
    pub kind: String,
    pub imdb_rating: Option<f64>,
    pub length: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TitleDto {
    pub id: i64,
    #[serde(rename = "imdbId")]
    pub imdb_id: Option<String>,
    pub title: String,
    pub year: i64,
    pub services: Vec<Service>,
    #[serde(rename = "type")]
    pub kind: TitleKind,
    pub genres: Vec<String>,
    pub imdb: Option<f64>,
    pub len: String,
    pub desc: String,
    pub cast: Vec<String>,
    pub watched: bool,
    pub rating: Option<i64>,
}
```

- [ ] **Step 4: Add `pub mod models;` to `src/lib.rs`**

```rust
pub mod config;
pub mod db;
pub mod models;
```

- [ ] **Step 5: Run the test and clippy**

Run: `cargo test --lib models` → Expected: PASS (2 tests).
Run: `cargo clippy --all-targets -- -D warnings` → Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/models.rs src/lib.rs
git commit -m "feat(backend): domain enums, TitleRow, and TitleDto"
```

---

### Task 4: Dev seed data + seeder

**Files:**
- Create: `seed/catalogue.json`
- Create: `src/db/seed.rs`
- Modify: `src/db/mod.rs` (add `pub mod seed;`)
- Test: inline `#[cfg(test)]` module in `src/db/seed.rs`

**Interfaces:**
- Consumes: `db::init_pool` (Task 2).
- Produces: `db::seed::seed_if_empty(pool: &sqlx::SqlitePool) -> anyhow::Result<usize>` — inserts all seed titles (+ services/genres/cast) only when `titles` is empty; returns the number of titles inserted (0 if already populated).

- [ ] **Step 1: Create `seed/catalogue.json`**

Transcribe the 28-title array from the prototype at `design_handoff_cue/cue.dc.html` lines **476–503**. For each prototype object, drop the numeric `id`, keep `title`/`year`/`type`/`genres`/`imdb`/`len`/`desc`/`cast`, rename `service` (a single string) — `imdb_id` stays `null` (the prototype has none). Two example entries showing the exact target shape (transcribe the remaining 26 the same way):

```json
[
  {
    "imdb_id": null,
    "title": "Frieren: Beyond Journey's End",
    "year": 2023,
    "type": "series",
    "service": "crunchyroll",
    "genres": ["Fantasy", "Adventure", "Drama"],
    "imdb": 8.9,
    "len": "28 eps",
    "desc": "An elf mage outlives her party and sets out to understand the human lives she once overlooked.",
    "cast": ["Atsumi Tanezaki", "Kana Ichinose", "Nobuhiko Okamoto"]
  },
  {
    "imdb_id": null,
    "title": "Andor",
    "year": 2022,
    "type": "series",
    "service": "disney",
    "genres": ["Sci-Fi", "Drama", "Thriller"],
    "imdb": 8.4,
    "len": "12 eps",
    "desc": "Before the rebellion had hope, it had a thief learning what resistance really costs.",
    "cast": ["Diego Luna", "Stellan Skarsgård", "Genevieve O'Reilly"]
  }
]
```

The final array must contain all 28 titles (ids 1–28 in the prototype).

- [ ] **Step 2: Write the failing test in `src/db/seed.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_pool;

    // Returns the pool plus the TempDir guard — keep the guard bound (`_dir`)
    // for the test's lifetime so the directory isn't cleaned up early.
    async fn fresh_pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        (pool, dir)
    }

    #[tokio::test]
    async fn seeds_all_titles_once() {
        let (pool, _dir) = fresh_pool().await;

        let inserted = seed_if_empty(&pool).await.unwrap();
        assert_eq!(inserted, 28);

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM titles")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 28);

        // Frieren must have its three genres and the crunchyroll service.
        let genres: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM title_genres tg JOIN titles t ON t.id = tg.title_id WHERE t.title = ?",
        )
        .bind("Frieren: Beyond Journey's End")
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(genres, 3);

        // Running again is a no-op.
        let again = seed_if_empty(&pool).await.unwrap();
        assert_eq!(again, 0);
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --lib db::seed`
Expected: FAIL to compile — `seed_if_empty` not defined.

- [ ] **Step 4: Implement `src/db/seed.rs`**

```rust
use serde::Deserialize;
use sqlx::SqlitePool;

#[derive(Debug, Deserialize)]
struct SeedTitle {
    imdb_id: Option<String>,
    title: String,
    year: i64,
    #[serde(rename = "type")]
    kind: String,
    service: String,
    genres: Vec<String>,
    imdb: Option<f64>,
    len: String,
    desc: String,
    cast: Vec<String>,
}

const SEED_JSON: &str = include_str!("../../seed/catalogue.json");

/// Insert the dev catalogue when `titles` is empty. Returns the number inserted.
pub async fn seed_if_empty(pool: &SqlitePool) -> anyhow::Result<usize> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM titles")
        .fetch_one(pool)
        .await?;
    if count > 0 {
        return Ok(0);
    }

    let seeds: Vec<SeedTitle> = serde_json::from_str(SEED_JSON)?;
    let mut tx = pool.begin().await?;
    for s in &seeds {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO titles (imdb_id, title, year, type, imdb_rating, length, description)
             VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(&s.imdb_id)
        .bind(&s.title)
        .bind(s.year)
        .bind(&s.kind)
        .bind(s.imdb)
        .bind(&s.len)
        .bind(&s.desc)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query("INSERT INTO title_services (title_id, service) VALUES (?, ?)")
            .bind(id)
            .bind(&s.service)
            .execute(&mut *tx)
            .await?;

        for g in &s.genres {
            sqlx::query("INSERT INTO title_genres (title_id, genre) VALUES (?, ?)")
                .bind(id)
                .bind(g)
                .execute(&mut *tx)
                .await?;
        }

        for (ord, person) in s.cast.iter().enumerate() {
            sqlx::query("INSERT INTO title_cast (title_id, person, ord) VALUES (?, ?, ?)")
                .bind(id)
                .bind(person)
                .bind(i64::try_from(ord).unwrap_or(i64::MAX))
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    Ok(seeds.len())
}
```

- [ ] **Step 5: Add `pub mod seed;` to `src/db/mod.rs`**

Add at the top of `src/db/mod.rs`:

```rust
pub mod seed;
```

- [ ] **Step 6: Run the test and clippy**

Run: `cargo test --lib db::seed` → Expected: PASS.
Run: `cargo clippy --all-targets -- -D warnings` → Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add seed/catalogue.json src/db/seed.rs src/db/mod.rs
git commit -m "feat(backend): dev catalogue seed and idempotent seeder"
```

---

### Task 5: Catalogue assembly + `GET /api/catalogue`

**Files:**
- Create: `src/db/catalogue.rs`
- Create: `src/routes/mod.rs`
- Create: `src/routes/catalogue.rs`
- Modify: `src/main.rs` (add `mod routes;`)
- Modify: `src/db/mod.rs` (add `pub mod catalogue;`)
- Test: inline `#[cfg(test)]` module in `src/routes/catalogue.rs`

**Interfaces:**
- Consumes: `models::{TitleDto, TitleRow, Service, TitleKind}`, `db::init_pool`, `db::seed::seed_if_empty`.
- Produces: `db::catalogue::fetch_catalogue(pool: &SqlitePool) -> anyhow::Result<Vec<TitleDto>>` — fully hydrated, ordered by `titles.id`.
- Produces: `routes::configure(cfg: &mut actix_web::web::ServiceConfig)` registering `/api/health` and `/api/catalogue`. The pool is shared via `web::Data<SqlitePool>`.

- [ ] **Step 1: Implement `src/db/catalogue.rs`**

```rust
use std::collections::{HashMap, HashSet};

use sqlx::SqlitePool;

use crate::models::{Service, TitleDto, TitleKind, TitleRow};

/// Fetch every title fully hydrated with services, genres, cast, and user data.
pub async fn fetch_catalogue(pool: &SqlitePool) -> anyhow::Result<Vec<TitleDto>> {
    let rows: Vec<TitleRow> = sqlx::query_as(
        "SELECT id, imdb_id, title, year, type, imdb_rating, length, description
         FROM titles ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let services = sqlx::query_as::<_, (i64, String)>(
        "SELECT title_id, service FROM title_services",
    )
    .fetch_all(pool)
    .await?;
    let mut svc_map: HashMap<i64, Vec<Service>> = HashMap::new();
    for (tid, s) in services {
        if let Some(service) = Service::parse(&s) {
            svc_map.entry(tid).or_default().push(service);
        }
    }

    let genres = sqlx::query_as::<_, (i64, String)>(
        "SELECT title_id, genre FROM title_genres ORDER BY title_id, genre",
    )
    .fetch_all(pool)
    .await?;
    let mut genre_map: HashMap<i64, Vec<String>> = HashMap::new();
    for (tid, g) in genres {
        genre_map.entry(tid).or_default().push(g);
    }

    let cast = sqlx::query_as::<_, (i64, String)>(
        "SELECT title_id, person FROM title_cast ORDER BY title_id, ord",
    )
    .fetch_all(pool)
    .await?;
    let mut cast_map: HashMap<i64, Vec<String>> = HashMap::new();
    for (tid, person) in cast {
        cast_map.entry(tid).or_default().push(person);
    }

    let ratings = sqlx::query_as::<_, (String, i64)>(
        "SELECT imdb_id, rating FROM user_ratings",
    )
    .fetch_all(pool)
    .await?;
    let rating_map: HashMap<String, i64> = ratings.into_iter().collect();

    let watched = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT imdb_id FROM watch_history",
    )
    .fetch_all(pool)
    .await?;
    let watched_set: HashSet<String> = watched.into_iter().collect();

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let kind = TitleKind::parse(&r.kind).unwrap_or(TitleKind::Movie);
        let (watched, rating) = match &r.imdb_id {
            Some(key) => (watched_set.contains(key), rating_map.get(key).copied()),
            None => (false, None),
        };
        out.push(TitleDto {
            id: r.id,
            imdb_id: r.imdb_id,
            title: r.title,
            year: r.year,
            services: svc_map.remove(&r.id).unwrap_or_default(),
            kind,
            genres: genre_map.remove(&r.id).unwrap_or_default(),
            imdb: r.imdb_rating,
            len: r.length,
            desc: r.description,
            cast: cast_map.remove(&r.id).unwrap_or_default(),
            watched,
            rating,
        });
    }
    Ok(out)
}
```

- [ ] **Step 2: Add `pub mod catalogue;` to `src/db/mod.rs`**

```rust
pub mod catalogue;
```

- [ ] **Step 3: Create `src/routes/catalogue.rs` with the failing test**

```rust
use actix_web::{web, HttpResponse, Responder};
use sqlx::SqlitePool;

use crate::db::catalogue::fetch_catalogue;

pub async fn get_catalogue(pool: web::Data<SqlitePool>) -> impl Responder {
    match fetch_catalogue(pool.get_ref()).await {
        Ok(titles) => HttpResponse::Ok().json(titles),
        Err(e) => {
            tracing::error!("catalogue fetch failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{test, web, App};
    use serde_json::Value;
    use sqlx::SqlitePool;

    use crate::db::{init_pool, seed::seed_if_empty};
    use crate::routes;

    async fn seeded_pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        seed_if_empty(&pool).await.unwrap();
        (pool, dir)
    }

    #[actix_web::test]
    async fn catalogue_endpoint_returns_seed() {
        let (pool, _dir) = seeded_pool().await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .configure(routes::configure),
        )
        .await;

        let req = test::TestRequest::get().uri("/api/catalogue").to_request();
        let body: Value = test::call_and_read_body_json(&app, req).await;

        let arr = body.as_array().unwrap();
        assert_eq!(arr.len(), 28);
        let first = &arr[0];
        assert!(first["id"].is_number());
        assert!(first["type"] == "movie" || first["type"] == "series");
        assert!(first["services"].is_array());
        assert!(first["genres"].is_array());
    }
}
```

- [ ] **Step 4: Create `src/routes/mod.rs`**

```rust
pub mod catalogue;

use actix_web::{web, HttpResponse};

async fn health() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({ "status": "ok" }))
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api")
            .route("/health", web::get().to(health))
            .route("/catalogue", web::get().to(catalogue::get_catalogue)),
    );
}
```

- [ ] **Step 5: Add `pub mod routes;` to `src/lib.rs`**

```rust
pub mod config;
pub mod db;
pub mod models;
pub mod routes;
```

- [ ] **Step 6: Run the test to verify it fails, then passes**

Run: `cargo test --lib routes::catalogue` → Expected: first FAILS to compile until Steps 1–5 are all in place, then PASSES.
Run: `cargo clippy --all-targets -- -D warnings` → Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/db/catalogue.rs src/db/mod.rs src/routes/mod.rs src/routes/catalogue.rs src/lib.rs
git commit -m "feat(backend): catalogue assembly and GET /api/catalogue"
```

---

### Task 6: Static SPA serving + server bootstrap

**Files:**
- Create: `src/static_files.rs`
- Modify: `src/main.rs` (full bootstrap)
- Test: inline `#[cfg(test)]` module in `src/static_files.rs`

**Interfaces:**
- Consumes: `config::Config`, `db::{init_pool, seed::seed_if_empty}`, `routes::configure`.
- Produces: `static_files::StaticDir(pub String)` (app-data newtype) and `static_files::serve_spa(req, dir) -> HttpResponse` — a single catch-all that serves the requested file from `static_dir` when it exists, otherwise `index.html` (SPA fallback), otherwise an inline placeholder. Rejects `..` traversal.

**Why one catch-all instead of `actix_files::Files`:** `Files` returns 404 for missing paths rather than falling through, which would break client-side routes (e.g. `/detail/5`). The catch-all serves real assets *and* falls back to `index.html` for unknown routes.

- [ ] **Step 1: Create `src/static_files.rs` with the failing test**

```rust
use actix_files::NamedFile;
use actix_web::{web, HttpRequest, HttpResponse, Responder};

#[derive(Clone)]
pub struct StaticDir(pub String);

const PLACEHOLDER: &str = "<!doctype html><meta charset=utf-8><title>cue</title>\
    <h1>cue backend running</h1><p>Frontend not built yet.</p>";

/// Catch-all static handler: serve the requested file when it exists under the
/// static dir; otherwise fall back to `index.html` (SPA client routing);
/// otherwise an inline placeholder (frontend not built yet).
pub async fn serve_spa(req: HttpRequest, dir: web::Data<StaticDir>) -> impl Responder {
    let base = std::path::Path::new(&dir.0);
    let rel = req.path().trim_start_matches('/');

    // Reject path traversal before touching the filesystem.
    if !rel.is_empty() && !rel.contains("..") {
        if let Ok(file) = NamedFile::open_async(base.join(rel)).await {
            return file.into_response(&req);
        }
    }

    match NamedFile::open_async(base.join("index.html")).await {
        Ok(file) => file.into_response(&req),
        Err(_) => HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(PLACEHOLDER),
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{test, web, App};

    use super::{serve_spa, StaticDir};

    #[actix_web::test]
    async fn fallback_serves_placeholder_when_no_build() {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(StaticDir("does/not/exist".to_string())))
                .default_service(web::route().to(serve_spa)),
        )
        .await;

        let req = test::TestRequest::get().uri("/some/client/route").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        let body = test::read_body(resp).await;
        assert!(String::from_utf8_lossy(&body).contains("cue backend running"));
    }
}
```

- [ ] **Step 2: Register the module in `src/lib.rs` and run the test**

Add `pub mod static_files;` to `src/lib.rs`:

```rust
pub mod config;
pub mod db;
pub mod models;
pub mod routes;
pub mod static_files;
```

Run: `cargo test --lib static_files`
Expected: PASS (the `serve_spa` placeholder test is self-contained).

- [ ] **Step 3: Write the full `src/main.rs` bootstrap**

The binary consumes the library crate (`cue::…`); it declares no modules of its own.

```rust
use actix_web::{web, App, HttpServer};

use cue::config::Config;
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

    let static_dir = cfg.static_dir.clone();
    let bind_addr = cfg.bind_addr.clone();
    tracing::info!("listening on {bind_addr}");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
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
```

- [ ] **Step 4: Run the test and clippy**

Run: `cargo test --lib static_files` → Expected: PASS.
Run: `cargo test` → Expected: all tests PASS.
Run: `cargo clippy --all-targets -- -D warnings` → Expected: no warnings.

- [ ] **Step 5: Manual smoke test**

Run: `cargo run`
Then in another shell:
- `curl -s http://127.0.0.1:8080/api/health` → Expected: `{"status":"ok"}`
- `curl -s http://127.0.0.1:8080/api/catalogue | head -c 200` → Expected: a JSON array beginning with a title object.
- `curl -s http://127.0.0.1:8080/` → Expected: the "cue backend running" placeholder HTML.

Stop the server (Ctrl-C).

- [ ] **Step 6: Commit**

```bash
git add src/static_files.rs src/main.rs
git commit -m "feat(backend): SPA static serving and server bootstrap"
```

---

### Task 7: Docker packaging

**Files:**
- Create: `Dockerfile`
- Create: `.dockerignore`
- Create: `docker-compose.yml`

**Interfaces:**
- Consumes: the built `cue` binary and embedded migrations.
- Produces: a runnable image; `docker compose up` serves the API on `127.0.0.1:8080` with SQLite persisted to a named volume.

- [ ] **Step 1: Create `.dockerignore`**

```dockerignore
/target
/data
.env
node_modules
/frontend/node_modules
/frontend/dist
.git
```

- [ ] **Step 2: Create `Dockerfile`**

The Vue build stage is commented out and enabled by the frontend plan; this plan ships a backend-only image that serves the API plus the inline placeholder.

```dockerfile
# syntax=docker/dockerfile:1

# --- Stage (enabled in the frontend plan): build the Vue SPA ---
# FROM node:22-alpine AS frontend
# WORKDIR /app/frontend
# COPY frontend/package*.json ./
# RUN npm ci
# COPY frontend/ ./
# RUN npm run build            # outputs /app/frontend/dist

# --- Build the Rust backend ---
FROM rust:1-bookworm AS backend
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY seed ./seed
RUN cargo build --release

# --- Runtime ---
FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=backend /app/target/release/cue /usr/local/bin/cue
# COPY --from=frontend /app/frontend/dist ./frontend/dist   # enabled in the frontend plan
ENV BIND_ADDR=0.0.0.0:8080
ENV DATABASE_URL=sqlite:/data/cue.db
ENV STATIC_DIR=/app/frontend/dist
EXPOSE 8080
CMD ["cue"]
```

- [ ] **Step 3: Create `docker-compose.yml`**

```yaml
services:
  cue:
    build: .
    ports:
      - "127.0.0.1:8080:8080"
    volumes:
      - cue-data:/data
    env_file:
      - .env
    restart: unless-stopped

volumes:
  cue-data:
```

- [ ] **Step 4: Verify the image builds and runs**

Run: `docker compose build` → Expected: build succeeds.
Run: `docker compose up -d` then `curl -s http://127.0.0.1:8080/api/catalogue | head -c 100` → Expected: JSON array of titles.
Run: `docker compose down` to stop.

> Note: `.env` must exist for `env_file` (copy from `.env.example`). The SQLite file lives on the `cue-data` volume at `/data/cue.db`.

- [ ] **Step 5: Commit**

```bash
git add Dockerfile .dockerignore docker-compose.yml
git commit -m "feat(backend): docker image and compose setup"
```

---

## Self-Review

**Spec coverage (against `2026-06-21-cue-design.md`):**
- §4 schema (all 8 tables) → Task 2. ✓
- §6 D6 `imdb_id` canonical key + null-for-seed → Task 2 schema + Task 5 hydration. ✓
- §7 `GET /api/catalogue` → Task 5. ✓ (`/api/ask`, rating/watched, sync status are later plans — listed in §11/plan sequence, intentionally out of scope here.)
- §8 frontend `Title` shape → mirrored exactly by `TitleDto` (Task 3). ✓
- §9 config/secrets/Docker/clippy table → Tasks 1 & 7. ✓
- §3 static SPA serving + boundary (Vue → own API only) → Task 6. ✓
- D8 multi-service set → `title_services` + `services: Vec<Service>`. ✓ (Seed titles have one service each; the model supports many.)
- D9 seed retained as dev fixture → Task 4. ✓

**Placeholder scan:** the only non-literal content is the seed transcription (Task 4 Step 1), which cites an exact in-repo source (`cue.dc.html:476–503`) and shows the precise target shape with two complete examples — concrete data transcription, not a code placeholder.

**Type consistency:** `TitleDto`/`Service`/`TitleKind` defined in Task 3 are consumed unchanged in Task 5; `init_pool`/`seed_if_empty`/`fetch_catalogue`/`configure`/`StaticDir`/`spa_fallback` names match across all tasks and tests.

## Notes for later plans
- The frontend plan enables the two commented Docker stages and replaces the inline placeholder with the real `index.html`.
- The ask-engine plan adds `OPENAI_API_KEY`/`ANTHROPIC_API_KEY` usage (already parsed into `Config`) and writes to `title_embeddings`.
- The user-data plan defines the fallback personal-data key for titles without `imdb_id` (e.g. `local:<id>`), and the rating/watched write endpoints.
