use std::collections::{HashMap, HashSet};

use sqlx::SqlitePool;

use crate::models::{Service, TitleDto, TitleKind, TitleRow};

/// Fetch every title fully hydrated with services, genres, cast, and user data.
///
/// # Errors
/// Returns an error if any database query fails.
pub async fn fetch_catalogue(pool: &SqlitePool) -> anyhow::Result<Vec<TitleDto>> {
    let rows: Vec<TitleRow> = sqlx::query_as(
        "SELECT id, imdb_id, title, year, type, imdb_rating, length, description
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

    let cast = sqlx::query_as::<_, (i64, String)>(
        "SELECT title_id, person FROM title_cast ORDER BY title_id, ord",
    )
    .fetch_all(pool)
    .await?;
    let mut cast_map: HashMap<i64, Vec<String>> = HashMap::new();
    for (tid, person) in cast {
        cast_map.entry(tid).or_default().push(person);
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
