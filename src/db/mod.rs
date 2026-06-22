pub mod catalogue;
pub mod embeddings;
pub mod seed;
pub mod sync_runs;

use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

/// Open the `SQLite` pool (creating the file if missing), enforce foreign keys,
/// and run embedded migrations.
///
/// # Errors
/// Returns an error if the URL cannot be parsed, the database cannot be opened,
/// or the migrations fail to apply.
pub async fn init_pool(database_url: &str) -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .foreign_keys(true);
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
}
