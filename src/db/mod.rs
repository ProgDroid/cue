pub mod catalogue;
pub mod embeddings;
pub mod seed;
pub mod sync_runs;
pub mod user_data;

use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqliteSynchronous};

/// Open the `SQLite` pool (creating the file if missing), enforce foreign keys,
/// and run embedded migrations.
///
/// Uses WAL journaling so a long-running catalogue sync (many write
/// transactions + embedding round-trips) does not block concurrent reads —
/// e.g. the settings page polling `/api/sync/status`. Without WAL the default
/// rollback journal makes a writer block all readers, which surfaced as
/// `database is locked` (`SQLITE_BUSY`) on the status endpoint during a sync.
/// `synchronous = NORMAL` is the standard, durable-enough companion to WAL, and
/// a generous `busy_timeout` lets the rare writer-vs-writer wait resolve instead
/// of erroring immediately.
///
/// # Errors
/// Returns an error if the URL cannot be parsed, the database cannot be opened,
/// or the migrations fail to apply.
pub async fn init_pool(database_url: &str) -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(15));
    let pool = SqlitePool::connect_with(opts).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

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

        let names: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .fetch_all(&pool)
                .await
                .unwrap();

        for expected in [
            "title_cast",
            "title_embeddings",
            "title_genres",
            "title_services",
            "titles",
            "user_ratings",
            "watch_history",
            "sync_runs",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "missing table {expected}"
            );
        }
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        // SQLite only honours REFERENCES clauses when `PRAGMA foreign_keys` is
        // ON, and that pragma is per-connection. `init_pool` sets it on every
        // pooled connection; assert both the pragma value and the actual
        // behaviour (an orphaned child insert must be rejected).
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();

        let fk_on: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(fk_on, 1, "foreign_keys pragma should be enabled");

        // title_genres.title_id REFERENCES titles(id); id 999999 has no parent.
        let orphan =
            sqlx::query("INSERT INTO title_genres (title_id, genre) VALUES (999999, 'Comedy')")
                .execute(&pool)
                .await;
        assert!(
            orphan.is_err(),
            "inserting a title_genres row with no parent title should violate the FK"
        );
    }

    #[tokio::test]
    async fn wal_mode_is_enabled() {
        // WAL lets readers (status polling) proceed while a sync writes; the
        // default rollback journal would block them and surface SQLITE_BUSY.
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();

        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal", "pool should open in WAL mode");
    }

    #[tokio::test]
    async fn user_ratings_accepts_one_to_ten() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();

        // 10 is now in range.
        sqlx::query("INSERT INTO user_ratings (imdb_id, rating) VALUES ('tt1', 10)")
            .execute(&pool)
            .await
            .expect("rating 10 should be accepted after migration 0002");

        // 11 is still rejected by the CHECK.
        let too_high = sqlx::query("INSERT INTO user_ratings (imdb_id, rating) VALUES ('tt2', 11)")
            .execute(&pool)
            .await;
        assert!(too_high.is_err(), "rating 11 must violate the CHECK");
    }
}
