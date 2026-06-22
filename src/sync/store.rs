//! DB writes for sync: upsert titles, reconcile service membership, prune.

use std::collections::HashSet;

use sqlx::{SqliteConnection, SqlitePool};

use crate::models::Service;
use crate::sync::merge::MergedTitle;

/// Find an existing surrogate id by identity (imdb → tmdb → `plex_guid`).
async fn find_existing(pool: &SqlitePool, t: &MergedTitle) -> anyhow::Result<Option<i64>> {
    if let Some(imdb) = &t.imdb_id {
        if let Some(id) = sqlx::query_scalar::<_, i64>("SELECT id FROM titles WHERE imdb_id = ?")
            .bind(imdb)
            .fetch_optional(pool)
            .await?
        {
            return Ok(Some(id));
        }
    }
    if let Some(tmdb) = &t.tmdb_id {
        if let Some(id) = sqlx::query_scalar::<_, i64>("SELECT id FROM titles WHERE tmdb_id = ?")
            .bind(tmdb)
            .fetch_optional(pool)
            .await?
        {
            return Ok(Some(id));
        }
    }
    if let Some(guid) = &t.plex_guid {
        if let Some(id) = sqlx::query_scalar::<_, i64>("SELECT id FROM titles WHERE plex_guid = ?")
            .bind(guid)
            .fetch_optional(pool)
            .await?
        {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Replace a title's genre + cast child rows inside an open transaction.
async fn replace_children(
    conn: &mut SqliteConnection,
    id: i64,
    t: &MergedTitle,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM title_genres WHERE title_id = ?")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    for g in &t.genres {
        sqlx::query("INSERT OR IGNORE INTO title_genres (title_id, genre) VALUES (?, ?)")
            .bind(id)
            .bind(g)
            .execute(&mut *conn)
            .await?;
    }
    sqlx::query("DELETE FROM title_cast WHERE title_id = ?")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    for (ord, person) in t.cast.iter().enumerate() {
        sqlx::query("INSERT OR IGNORE INTO title_cast (title_id, person, ord) VALUES (?, ?, ?)")
            .bind(id)
            .bind(person)
            .bind(i64::try_from(ord).unwrap_or(i64::MAX))
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Insert a new title or update the existing one matched by identity. Returns its id.
///
/// All writes (title row + genres + cast) are wrapped in a single transaction so a
/// crash mid-operation cannot leave the title without its child rows.
///
/// # Errors
/// Returns an error if any query fails.
pub async fn upsert_title(pool: &SqlitePool, t: &MergedTitle) -> anyhow::Result<i64> {
    // Identity lookup runs outside the tx — read-only, no consistency risk.
    let existing = find_existing(pool, t).await?;

    let mut tx = pool.begin().await?;
    let id = if let Some(id) = existing {
        sqlx::query(
            "UPDATE titles SET imdb_id = ?, tmdb_id = ?, plex_guid = ?, title = ?, year = ?,
             type = ?, imdb_rating = ?, length = ?, description = ?, updated_at = datetime('now')
             WHERE id = ?",
        )
        .bind(&t.imdb_id)
        .bind(&t.tmdb_id)
        .bind(&t.plex_guid)
        .bind(&t.title)
        .bind(t.year)
        .bind(t.kind.as_str())
        .bind(t.imdb_rating)
        .bind(&t.length)
        .bind(&t.description)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        id
    } else {
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO titles (imdb_id, tmdb_id, plex_guid, title, year, type, imdb_rating, length, description)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(&t.imdb_id)
        .bind(&t.tmdb_id)
        .bind(&t.plex_guid)
        .bind(&t.title)
        .bind(t.year)
        .bind(t.kind.as_str())
        .bind(t.imdb_rating)
        .bind(&t.length)
        .bind(&t.description)
        .fetch_one(&mut *tx)
        .await?
    };
    replace_children(&mut tx, id, t).await?;
    tx.commit().await?;
    Ok(id)
}

/// Make `title_services` for `service` exactly match `desired_ids`.
///
/// The delete-stale and insert-missing passes run inside a single transaction so
/// the result is always exactly `desired_ids`, even if interrupted.
///
/// # Errors
/// Returns an error if any query fails.
pub async fn reconcile_service(
    pool: &SqlitePool,
    service: Service,
    desired_ids: &[i64],
) -> anyhow::Result<()> {
    let want: HashSet<i64> = desired_ids.iter().copied().collect();
    // Read current membership outside the tx — consistent snapshot for the diff.
    let current: Vec<i64> =
        sqlx::query_scalar("SELECT title_id FROM title_services WHERE service = ?")
            .bind(service.as_str())
            .fetch_all(pool)
            .await?;

    let mut tx = pool.begin().await?;
    for id in current {
        if !want.contains(&id) {
            sqlx::query("DELETE FROM title_services WHERE service = ? AND title_id = ?")
                .bind(service.as_str())
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }
    for id in desired_ids {
        sqlx::query("INSERT OR IGNORE INTO title_services (title_id, service) VALUES (?, ?)")
            .bind(id)
            .bind(service.as_str())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Delete titles with no service membership. Returns rows removed.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn prune_orphans(pool: &SqlitePool) -> anyhow::Result<u64> {
    let res =
        sqlx::query("DELETE FROM titles WHERE id NOT IN (SELECT title_id FROM title_services)")
            .execute(pool)
            .await?;
    Ok(res.rows_affected())
}

/// Delete titles that have no service membership in any of the `touched_services`
/// (the union of successfully-reconciled and explicitly-failed services for this run).
///
/// This is the "scoped prune": titles owned exclusively by protected (failed) services
/// are preserved; stale titles from services not mentioned in this run are removed.
/// Returns the number of rows deleted.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn prune_orphans_scoped(
    pool: &SqlitePool,
    touched_services: &[&str],
) -> anyhow::Result<u64> {
    if touched_services.is_empty() {
        return Ok(0);
    }
    // Build "?, ?, ..." placeholder string for the IN clause.
    let placeholders = touched_services
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "DELETE FROM titles WHERE id NOT IN \
         (SELECT title_id FROM title_services WHERE service IN ({placeholders}))"
    );
    let mut q = sqlx::query(&sql);
    for svc in touched_services {
        q = q.bind(*svc);
    }
    let res = q.execute(pool).await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_pool;
    use crate::models::TitleKind;

    async fn pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (init_pool(&url).await.unwrap(), dir)
    }

    fn merged(imdb: &str, title: &str, genres: &[&str], services: &[Service]) -> MergedTitle {
        MergedTitle {
            imdb_id: Some(imdb.into()),
            tmdb_id: None,
            plex_guid: None,
            title: title.into(),
            year: 2020,
            kind: TitleKind::Movie,
            imdb_rating: Some(7.5),
            length: "100 min".into(),
            description: "d".into(),
            genres: genres.iter().map(|s| (*s).to_string()).collect(),
            cast: vec!["Actor".into()],
            services: services.to_vec(),
        }
    }

    #[tokio::test]
    async fn upsert_inserts_then_updates_same_id() {
        let (p, _dir) = pool().await;
        let id1 = upsert_title(&p, &merged("tt1", "Old", &["action"], &[Service::Plex]))
            .await
            .unwrap();
        let id2 = upsert_title(&p, &merged("tt1", "New", &["drama"], &[Service::Plex]))
            .await
            .unwrap();
        assert_eq!(id1, id2, "same imdb_id reuses the surrogate id");
        let title: String = sqlx::query_scalar("SELECT title FROM titles WHERE id = ?")
            .bind(id1)
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(title, "New");
        let genres: Vec<String> =
            sqlx::query_scalar("SELECT genre FROM title_genres WHERE title_id = ?")
                .bind(id1)
                .fetch_all(&p)
                .await
                .unwrap();
        assert_eq!(
            genres,
            vec!["drama".to_string()],
            "genres replaced, not appended"
        );
    }

    #[tokio::test]
    async fn reconcile_adds_and_removes_then_prune_deletes_orphans() {
        let (p, _dir) = pool().await;
        let a = upsert_title(&p, &merged("tt1", "A", &[], &[Service::Plex]))
            .await
            .unwrap();
        let b = upsert_title(&p, &merged("tt2", "B", &[], &[Service::Plex]))
            .await
            .unwrap();
        reconcile_service(&p, Service::Plex, &[a, b]).await.unwrap();
        // Second sync: only `a` is still on Plex.
        reconcile_service(&p, Service::Plex, &[a]).await.unwrap();
        let removed = prune_orphans(&p).await.unwrap();
        assert_eq!(removed, 1);
        let remaining: Vec<i64> = sqlx::query_scalar("SELECT id FROM titles ORDER BY id")
            .fetch_all(&p)
            .await
            .unwrap();
        assert_eq!(remaining, vec![a]);
    }

    #[tokio::test]
    async fn prune_spares_titles_owned_by_an_untouched_service() {
        let (p, _dir) = pool().await;
        let x = upsert_title(&p, &merged("tt3", "X", &[], &[Service::Crunchyroll]))
            .await
            .unwrap();
        reconcile_service(&p, Service::Crunchyroll, &[x])
            .await
            .unwrap();
        // A Plex-only sync runs and reconciles Plex to empty; Crunchyroll untouched.
        reconcile_service(&p, Service::Plex, &[]).await.unwrap();
        let removed = prune_orphans(&p).await.unwrap();
        assert_eq!(removed, 0, "Crunchyroll membership keeps the title alive");
    }
}
