//! Read/write helpers for the `sync_runs` table + catalogue stats.

use serde::Serialize;
use sqlx::SqlitePool;

/// Latest run for one source, for `/api/sync/status`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRun {
    pub source: String,
    pub last_run: Option<String>,
    pub status: String,
    pub item_count: i64,
}

/// Aggregate catalogue counts for `/api/sync/status`.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogueStats {
    pub titles: i64,
    pub movies: i64,
    pub series: i64,
    pub embedded: i64,
}

/// Write one completed run row for a source.
///
/// # Errors
/// Returns an error if the insert fails.
pub async fn record(
    pool: &SqlitePool,
    source: &str,
    status: &str,
    item_count: i64,
    error: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sync_runs (source, finished_at, status, item_count, error)
         VALUES (?, datetime('now'), ?, ?, ?)",
    )
    .bind(source)
    .bind(status)
    .bind(item_count)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

/// The newest run per source (by row id).
///
/// # Errors
/// Returns an error if the query fails.
pub async fn latest_per_source(pool: &SqlitePool) -> anyhow::Result<Vec<SourceRun>> {
    let rows = sqlx::query_as::<_, (String, Option<String>, String, i64)>(
        "SELECT source, finished_at, status, item_count FROM sync_runs
         WHERE id IN (SELECT MAX(id) FROM sync_runs GROUP BY source)
         ORDER BY source",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(source, last_run, status, item_count)| SourceRun {
            source,
            last_run,
            status,
            item_count,
        })
        .collect())
}

/// Catalogue counts: total titles, movies, series, and how many are embedded.
///
/// # Errors
/// Returns an error if a query fails.
pub async fn catalogue_stats(pool: &SqlitePool) -> anyhow::Result<CatalogueStats> {
    // One aggregate over `titles` (SUM of a boolean predicate counts each kind;
    // COALESCE handles the empty-table NULL); `embedded` is a separate table.
    let (titles, movies, series): (i64, i64, i64) = sqlx::query_as(
        "SELECT
             COUNT(*),
             COALESCE(SUM(type = 'movie'), 0),
             COALESCE(SUM(type = 'series'), 0)
         FROM titles",
    )
    .fetch_one(pool)
    .await?;
    let embedded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_embeddings")
        .fetch_one(pool)
        .await?;
    Ok(CatalogueStats {
        titles,
        movies,
        series,
        embedded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{init_pool, seed::seed_if_empty};

    async fn pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (init_pool(&url).await.unwrap(), dir)
    }

    #[tokio::test]
    async fn record_then_latest_per_source_returns_newest_row() {
        let (p, _dir) = pool().await;
        record(&p, "plex", "ok", 10, None).await.unwrap();
        record(&p, "plex", "error", 0, Some("boom")).await.unwrap();
        record(&p, "disney", "ok", 5, None).await.unwrap();
        let rows = latest_per_source(&p).await.unwrap();
        let plex = rows.iter().find(|r| r.source == "plex").unwrap();
        assert_eq!(plex.status, "error");
        assert_eq!(
            rows.iter()
                .find(|r| r.source == "disney")
                .unwrap()
                .item_count,
            5
        );
    }

    #[tokio::test]
    async fn catalogue_stats_counts_seed() {
        let (p, _dir) = pool().await;
        seed_if_empty(&p).await.unwrap();
        let s = catalogue_stats(&p).await.unwrap();
        assert_eq!(s.titles, 28);
        assert_eq!(s.movies + s.series, 28);
        assert_eq!(s.embedded, 0);
    }
}
