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
///
/// # Errors
/// Returns an error if the database query fails or JSON cannot be parsed.
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
