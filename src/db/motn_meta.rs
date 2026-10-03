//! Movie-of-the-Night request accounting and sync bookkeeping, kept in the
//! `app_meta` key/value table (no dedicated migration).
//!
//! Keys:
//! - `motn.requests.YYYY-MM` — MOTN HTTP requests made in that UTC month
//! - `motn.catalogs.<country>` / `motn.catalogs_checked_at.<country>` — cached
//!   `/countries` resolution (comma-separated catalog ids) and when it was checked
//! - [`LAST_SEED_AT`], [`LAST_MODE`], [`SEED_FAILED_AT`]

use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::SqlitePool;

use crate::db::app_meta;

/// Unix seconds of the last successful full seed.
pub const LAST_SEED_AT: &str = "motn.last_seed_at";
/// `"seed"` or `"delta"`: the mode of the last successful MOTN run.
pub const LAST_MODE: &str = "motn.last_mode";
/// Unix seconds of the last failed seed attempt (`0` once a seed succeeds).
pub const SEED_FAILED_AT: &str = "motn.seed_failed_at";
/// Prefix of the per-month request counter; the suffix is the UTC `YYYY-MM`.
pub const REQUESTS_PREFIX: &str = "motn.requests.";

/// Key holding the cached catalog ids (`"disney,crunchyroll"`) for `country`.
#[must_use]
pub fn catalogs_key(country: &str) -> String {
    format!("motn.catalogs.{country}")
}

/// Key holding when the catalogs for `country` were last resolved (unix seconds).
#[must_use]
pub fn catalogs_checked_key(country: &str) -> String {
    format!("motn.catalogs_checked_at.{country}")
}

/// Current unix time in seconds (0 if the clock is before the epoch).
#[must_use]
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

/// Count one MOTN request against the current UTC month (single upsert).
///
/// # Errors
/// Returns an error if the write fails.
pub async fn increment_requests(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO app_meta (key, value) VALUES (? || strftime('%Y-%m', 'now'), '1')
         ON CONFLICT(key) DO UPDATE SET value = CAST(app_meta.value AS INTEGER) + 1",
    )
    .bind(REQUESTS_PREFIX)
    .execute(pool)
    .await?;
    Ok(())
}

/// MOTN requests counted in the current UTC month (0 when none).
///
/// # Errors
/// Returns an error if the query fails.
pub async fn requests_this_month(pool: &SqlitePool) -> anyhow::Result<i64> {
    let n = sqlx::query_scalar::<_, i64>(
        "SELECT CAST(value AS INTEGER) FROM app_meta
         WHERE key = ? || strftime('%Y-%m', 'now')",
    )
    .bind(REQUESTS_PREFIX)
    .fetch_optional(pool)
    .await?;
    Ok(n.unwrap_or(0))
}

/// Read an integer value; `None` when the key is absent or not an integer.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn get_i64(pool: &SqlitePool, key: &str) -> anyhow::Result<Option<i64>> {
    Ok(app_meta::get(pool, key)
        .await?
        .and_then(|v| v.trim().parse().ok()))
}

/// Upsert an integer value.
///
/// # Errors
/// Returns an error if the write fails.
pub async fn set_i64(pool: &SqlitePool, key: &str, v: i64) -> anyhow::Result<()> {
    app_meta::set(pool, key, &v.to_string()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_pool;

    async fn pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        (dir, pool)
    }

    #[tokio::test]
    async fn increment_requests_counts_this_month() {
        let (_dir, p) = pool().await;
        assert_eq!(requests_this_month(&p).await.unwrap(), 0);
        for _ in 0..3 {
            increment_requests(&p).await.unwrap();
        }
        assert_eq!(requests_this_month(&p).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn get_i64_missing_key_is_none() {
        let (_dir, p) = pool().await;
        assert_eq!(get_i64(&p, "motn.nope").await.unwrap(), None);
        set_i64(&p, SEED_FAILED_AT, 42).await.unwrap();
        assert_eq!(get_i64(&p, SEED_FAILED_AT).await.unwrap(), Some(42));
    }

    #[test]
    fn keys_are_per_country() {
        assert_eq!(catalogs_key("gb"), "motn.catalogs.gb");
        assert_eq!(catalogs_checked_key("gb"), "motn.catalogs_checked_at.gb");
    }
}
