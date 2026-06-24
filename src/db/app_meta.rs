//! Tiny key/value store for server-global, runtime-discovered singletons
//! (currently just the Plex `machineIdentifier` used to build watch links).

use sqlx::SqlitePool;

/// Read one value by key, or `None` if absent.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn get(pool: &SqlitePool, key: &str) -> anyhow::Result<Option<String>> {
    let v = sqlx::query_scalar::<_, String>("SELECT value FROM app_meta WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(v)
}

/// Upsert one key/value pair.
///
/// # Errors
/// Returns an error if the write fails.
pub async fn set(pool: &SqlitePool, key: &str, value: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO app_meta (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_pool;

    async fn pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (init_pool(&url).await.unwrap(), dir)
    }

    #[tokio::test]
    async fn set_then_get_roundtrips_and_upserts() {
        let (p, _dir) = pool().await;
        assert_eq!(get(&p, "plex_machine_id").await.unwrap(), None);
        set(&p, "plex_machine_id", "ABC123").await.unwrap();
        assert_eq!(
            get(&p, "plex_machine_id").await.unwrap(),
            Some("ABC123".into())
        );
        set(&p, "plex_machine_id", "DEF456").await.unwrap();
        assert_eq!(
            get(&p, "plex_machine_id").await.unwrap(),
            Some("DEF456".into())
        );
    }
}
