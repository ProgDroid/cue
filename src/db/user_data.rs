use std::collections::HashSet;

use sqlx::SqlitePool;

use crate::import::imdb_ratings::RatingImport;

/// Outcome of resolving a numeric title id to its `imdb_id` write key (D6).
pub enum KeyLookup {
    Key(String),
    NoImdbId,
    NotFound,
}

/// Resolve a numeric title id to the `imdb_id` used as the user-data key (D6).
///
/// # Errors
/// Returns an error if the database query fails.
pub async fn resolve_key(pool: &SqlitePool, title_id: i64) -> anyhow::Result<KeyLookup> {
    let row: Option<(Option<String>,)> = sqlx::query_as("SELECT imdb_id FROM titles WHERE id = ?")
        .bind(title_id)
        .fetch_optional(pool)
        .await?;
    Ok(match row {
        None => KeyLookup::NotFound,
        Some((None,)) => KeyLookup::NoImdbId,
        Some((Some(key),)) => KeyLookup::Key(key),
    })
}

/// Upsert the user's 1-10 rating for a title key.
///
/// # Errors
/// Returns an error if the database query fails.
pub async fn set_rating(pool: &SqlitePool, key: &str, rating: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_ratings (imdb_id, rating) VALUES (?, ?)
         ON CONFLICT(imdb_id) DO UPDATE SET rating = excluded.rating, rated_at = datetime('now')",
    )
    .bind(key)
    .bind(rating)
    .execute(pool)
    .await?;
    Ok(())
}

/// Remove the user's rating for a title key (no-op if absent).
///
/// # Errors
/// Returns an error if the database query fails.
pub async fn clear_rating(pool: &SqlitePool, key: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM user_ratings WHERE imdb_id = ?")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

/// Set watched state via a single `manual` `watch_history` row.
///
/// Watching is idempotent (at most one manual row); un-watching deletes only
/// `source = 'manual'` rows, leaving any imported `plex` history intact (D5.2).
///
/// # Errors
/// Returns an error if the database query fails.
pub async fn set_watched(pool: &SqlitePool, key: &str, watched: bool) -> anyhow::Result<()> {
    if watched {
        sqlx::query(
            "INSERT INTO watch_history (imdb_id, source)
             SELECT ?, 'manual'
             WHERE NOT EXISTS (
                 SELECT 1 FROM watch_history WHERE imdb_id = ? AND source = 'manual'
             )",
        )
        .bind(key)
        .bind(key)
        .execute(pool)
        .await?;
    } else {
        sqlx::query("DELETE FROM watch_history WHERE imdb_id = ? AND source = 'manual'")
            .bind(key)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Summary of an import: rows written and how many resolve to a catalogue title.
pub struct ImportOutcome {
    pub imported: usize,
    pub matched: usize,
}

/// Bulk-upsert imported ratings, returning counts of written rows and catalogue matches.
///
/// Runs in a single transaction; import overwrites on conflict, preserving
/// `rated_at` when supplied. `matched` is how many imported `imdb_id`s exist
/// in `titles`.
///
/// # Errors
/// Returns an error if any query or the transaction fails.
pub async fn import_ratings(
    pool: &SqlitePool,
    rows: &[RatingImport],
) -> anyhow::Result<ImportOutcome> {
    let mut tx = pool.begin().await?;
    for row in rows {
        sqlx::query(
            "INSERT INTO user_ratings (imdb_id, rating, rated_at)
             VALUES (?, ?, COALESCE(?, datetime('now')))
             ON CONFLICT(imdb_id) DO UPDATE SET
                 rating = excluded.rating,
                 rated_at = excluded.rated_at",
        )
        .bind(&row.imdb_id)
        .bind(row.rating)
        .bind(row.rated_at.as_deref())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    // `matched` = imported ids that exist in the catalogue. titles is small
    // (~5k); intersect in memory to avoid a large dynamic IN clause.
    let imported_ids: HashSet<&str> = rows.iter().map(|r| r.imdb_id.as_str()).collect();
    let title_ids: Vec<String> =
        sqlx::query_scalar("SELECT imdb_id FROM titles WHERE imdb_id IS NOT NULL")
            .fetch_all(pool)
            .await?;
    let matched = title_ids
        .iter()
        .filter(|id| imported_ids.contains(id.as_str()))
        .count();

    Ok(ImportOutcome {
        imported: rows.len(),
        matched,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_pool;

    async fn fresh_pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        (pool, dir)
    }

    async fn insert_title(pool: &SqlitePool, imdb: Option<&str>) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO titles (imdb_id, title, year, type) VALUES (?, 'T', 2020, 'movie') RETURNING id",
        )
        .bind(imdb)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn resolve_key_distinguishes_missing_null_and_present() {
        let (pool, _dir) = fresh_pool().await;
        let with = insert_title(&pool, Some("tt100")).await;
        let without = insert_title(&pool, None).await;

        assert!(
            matches!(resolve_key(&pool, with).await.unwrap(), KeyLookup::Key(k) if k == "tt100")
        );
        assert!(matches!(
            resolve_key(&pool, without).await.unwrap(),
            KeyLookup::NoImdbId
        ));
        assert!(matches!(
            resolve_key(&pool, 99999).await.unwrap(),
            KeyLookup::NotFound
        ));
    }

    #[tokio::test]
    async fn set_rating_upserts() {
        let (pool, _dir) = fresh_pool().await;
        set_rating(&pool, "tt100", 7).await.unwrap();
        set_rating(&pool, "tt100", 9).await.unwrap();

        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM user_ratings WHERE imdb_id = 'tt100'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let val: i64 =
            sqlx::query_scalar("SELECT rating FROM user_ratings WHERE imdb_id = 'tt100'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!((rows, val), (1, 9));
    }

    #[tokio::test]
    async fn clear_rating_removes_and_is_noop_when_absent() {
        let (pool, _dir) = fresh_pool().await;
        set_rating(&pool, "tt100", 5).await.unwrap();
        clear_rating(&pool, "tt100").await.unwrap();
        clear_rating(&pool, "tt100").await.unwrap(); // no-op, no error

        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM user_ratings WHERE imdb_id = 'tt100'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn set_watched_is_idempotent_and_manual_only() {
        let (pool, _dir) = fresh_pool().await;
        // A pre-existing imported plex row must survive an un-watch.
        sqlx::query("INSERT INTO watch_history (imdb_id, source) VALUES ('tt100', 'plex')")
            .execute(&pool)
            .await
            .unwrap();

        set_watched(&pool, "tt100", true).await.unwrap();
        set_watched(&pool, "tt100", true).await.unwrap(); // idempotent: no second manual row

        let manual: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM watch_history WHERE imdb_id = 'tt100' AND source = 'manual'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(manual, 1);

        set_watched(&pool, "tt100", false).await.unwrap();
        let manual_after: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM watch_history WHERE imdb_id = 'tt100' AND source = 'manual'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let plex_after: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM watch_history WHERE imdb_id = 'tt100' AND source = 'plex'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((manual_after, plex_after), (0, 1));
    }

    #[tokio::test]
    async fn import_ratings_upserts_overwrites_and_counts_matched() {
        use crate::import::imdb_ratings::RatingImport;
        let (pool, _dir) = fresh_pool().await;
        // tt100 is in the catalogue; tt900 is not.
        insert_title(&pool, Some("tt100")).await;

        // First import sets tt100 -> 5 (with a date) and tt900 -> 7 (no date).
        let first = vec![
            RatingImport {
                imdb_id: "tt100".into(),
                rating: 5,
                rated_at: Some("2018-01-01".into()),
            },
            RatingImport {
                imdb_id: "tt900".into(),
                rating: 7,
                rated_at: None,
            },
        ];
        let out = import_ratings(&pool, &first).await.unwrap();
        assert_eq!((out.imported, out.matched), (2, 1)); // only tt100 is a catalogue title

        // Re-import overwrites tt100 -> 9 and updates its date.
        let second = vec![RatingImport {
            imdb_id: "tt100".into(),
            rating: 9,
            rated_at: Some("2020-02-02".into()),
        }];
        import_ratings(&pool, &second).await.unwrap();

        let (rating, rated_at): (i64, String) =
            sqlx::query_as("SELECT rating, rated_at FROM user_ratings WHERE imdb_id = 'tt100'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(rating, 9);
        assert_eq!(rated_at, "2020-02-02");

        // tt900 (no date) got the now() default — a non-empty timestamp.
        let t900: String =
            sqlx::query_scalar("SELECT rated_at FROM user_ratings WHERE imdb_id = 'tt900'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!t900.is_empty());
    }
}
