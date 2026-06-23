use std::collections::{HashMap, HashSet};

use sqlx::SqlitePool;

use crate::models::{Service, TitleDto, TitleKind, TitleListItem, TitleListRow, TitleRow};

/// Fetch every title in the slim list shape (services, genres, user-data; no
/// `desc`/`cast`).
///
/// # Errors
/// Returns an error if any database query fails.
pub async fn fetch_catalogue(pool: &SqlitePool) -> anyhow::Result<Vec<TitleListItem>> {
    let rows: Vec<TitleListRow> = sqlx::query_as(
        "SELECT id, imdb_id, title, year, type, imdb_rating, length
         FROM titles ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let services =
        sqlx::query_as::<_, (i64, String)>("SELECT title_id, service FROM title_services")
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

    let ratings = sqlx::query_as::<_, (String, i64)>("SELECT imdb_id, rating FROM user_ratings")
        .fetch_all(pool)
        .await?;
    let rating_map: HashMap<String, i64> = ratings.into_iter().collect();

    let watched = sqlx::query_scalar::<_, String>("SELECT DISTINCT imdb_id FROM watch_history")
        .fetch_all(pool)
        .await?;
    let watched_set: HashSet<String> = watched.into_iter().collect();

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let kind = TitleKind::parse(&r.kind).unwrap_or(TitleKind::Movie);
        let (watched, rating) = r.imdb_id.as_ref().map_or((false, None), |key| {
            (watched_set.contains(key), rating_map.get(key).copied())
        });
        out.push(TitleListItem {
            id: r.id,
            imdb_id: r.imdb_id,
            title: r.title,
            year: r.year,
            services: svc_map.remove(&r.id).unwrap_or_default(),
            kind,
            genres: genre_map.remove(&r.id).unwrap_or_default(),
            imdb: r.imdb_rating,
            len: r.length,
            watched,
            rating,
        });
    }
    Ok(out)
}

/// Fetch one fully-hydrated title (services, genres, cast, user-data).
///
/// # Errors
/// Returns an error if any database query fails.
pub async fn fetch_title(pool: &SqlitePool, id: i64) -> anyhow::Result<Option<TitleDto>> {
    let Some(r) = sqlx::query_as::<_, TitleRow>(
        "SELECT id, imdb_id, title, year, type, imdb_rating, length, description
         FROM titles WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    let services: Vec<Service> =
        sqlx::query_scalar::<_, String>("SELECT service FROM title_services WHERE title_id = ?")
            .bind(id)
            .fetch_all(pool)
            .await?
            .iter()
            .filter_map(|s| Service::parse(s))
            .collect();

    let genres = sqlx::query_scalar::<_, String>(
        "SELECT genre FROM title_genres WHERE title_id = ? ORDER BY genre",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let cast = sqlx::query_scalar::<_, String>(
        "SELECT person FROM title_cast WHERE title_id = ? ORDER BY ord",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let (watched, rating) = if let Some(key) = r.imdb_id.as_ref() {
        let rating =
            sqlx::query_scalar::<_, i64>("SELECT rating FROM user_ratings WHERE imdb_id = ?")
                .bind(key)
                .fetch_optional(pool)
                .await?;
        let watched =
            sqlx::query_scalar::<_, i64>("SELECT 1 FROM watch_history WHERE imdb_id = ? LIMIT 1")
                .bind(key)
                .fetch_optional(pool)
                .await?
                .is_some();
        (watched, rating)
    } else {
        (false, None)
    };

    let kind = TitleKind::parse(&r.kind).unwrap_or(TitleKind::Movie);
    Ok(Some(TitleDto {
        id: r.id,
        imdb_id: r.imdb_id,
        title: r.title,
        year: r.year,
        services,
        kind,
        genres,
        imdb: r.imdb_rating,
        len: r.length,
        desc: r.description,
        cast,
        watched,
        rating,
    }))
}
