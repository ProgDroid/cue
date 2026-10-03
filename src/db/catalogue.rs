use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::SqlitePool;

use crate::models::{Service, TitleDto, TitleKind, TitleListItem, TitleListRow, TitleRow};

/// A title is "new" for this long after it was added.
const NEW_WINDOW_SECS: i64 = 30 * 86_400;
/// This many additions inside one `BULK_WINDOW_SECS` neighbourhood is a bulk
/// insert (initial import / re-sync), not genuinely new releases.
const BULK_THRESHOLD: usize = 300;
/// Half-width of the neighbourhood used to detect a bulk insert.
const BULK_WINDOW_SECS: i64 = 1_800;

/// Which titles are "new", as `title_id -> added_at` (unix secs).
///
/// Stateless rule: a title is new when it was added within `NEW_WINDOW_SECS`
/// of `now` and was not part of a bulk insert, i.e. fewer than
/// `BULK_THRESHOLD` titles (itself included) were added within
/// `±BULK_WINDOW_SECS` of it. `added` is `(title_id, added_at_unix)`.
#[must_use]
pub fn new_since_map(added: &[(i64, i64)], now: i64) -> HashMap<i64, i64> {
    let mut sorted: Vec<(i64, i64)> = added.to_vec();
    sorted.sort_unstable_by_key(|&(_, t)| t);

    let mut out = HashMap::new();
    // Two pointers over the sorted timestamps: `lo..hi` is the slice of rows
    // within `[t - BULK_WINDOW_SECS, t + BULK_WINDOW_SECS]` of the current row.
    let (mut lo, mut hi) = (0_usize, 0_usize);
    for &(id, t) in &sorted {
        while sorted[lo].1 < t - BULK_WINDOW_SECS {
            lo += 1;
        }
        while hi < sorted.len() && sorted[hi].1 <= t + BULK_WINDOW_SECS {
            hi += 1;
        }
        if now - t <= NEW_WINDOW_SECS && hi - lo < BULK_THRESHOLD {
            out.insert(id, t);
        }
    }
    out
}

/// Load `added_at` for every title and apply [`new_since_map`]. A row whose
/// `added_at` does not parse (NULL timestamp) is skipped, never new.
async fn load_new_since(pool: &SqlitePool, now: i64) -> anyhow::Result<HashMap<i64, i64>> {
    let rows = sqlx::query_as::<_, (i64, Option<i64>)>(
        "SELECT id, CAST(strftime('%s', added_at) AS INTEGER) FROM titles",
    )
    .fetch_all(pool)
    .await?;
    let added: Vec<(i64, i64)> = rows
        .into_iter()
        .filter_map(|(id, t)| t.map(|t| (id, t)))
        .collect();
    Ok(new_since_map(&added, now))
}

/// Current unix time in seconds (0 if the clock is before the epoch).
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

/// Fetch every title in the slim list shape (services, genres, user-data; no
/// `desc`/`cast`).
///
/// # Errors
/// Returns an error if any database query fails.
pub async fn fetch_catalogue(pool: &SqlitePool) -> anyhow::Result<Vec<TitleListItem>> {
    let rows: Vec<TitleListRow> = sqlx::query_as(
        "SELECT id, imdb_id, title, year, type, score, anilist_score, length\n         FROM titles ORDER BY id",
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

    let new_since = load_new_since(pool, unix_now()).await?;

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
            score: r.score,
            anilist_score: r.anilist_score,
            len: r.length,
            watched,
            rating,
            new_since: new_since.get(&r.id).copied(),
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
        "SELECT id, imdb_id, title, year, type, score, anilist_score, length, description\n         FROM titles WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    let plex_rating_key: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT plex_rating_key FROM titles WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .flatten();

    let service_rows = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT service, link FROM title_services WHERE title_id = ?",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let services: Vec<Service> = service_rows
        .iter()
        .filter_map(|(s, _)| Service::parse(s))
        .collect();

    let machine_id = crate::db::app_meta::get(pool, "plex_machine_id").await?;
    let mut watchable: Vec<String> = Vec::new();
    for (svc, link) in &service_rows {
        match Service::parse(svc) {
            Some(Service::Plex) if plex_rating_key.is_some() && machine_id.is_some() => {
                watchable.push("plex".to_string());
            }
            Some(Service::Crunchyroll | Service::Disney) if link.is_some() => {
                watchable.push(svc.clone());
            }
            _ => {}
        }
    }

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

    let new_since = load_new_since(pool, unix_now()).await?.get(&id).copied();

    let kind = TitleKind::parse(&r.kind).unwrap_or(TitleKind::Movie);
    Ok(Some(TitleDto {
        id: r.id,
        imdb_id: r.imdb_id,
        title: r.title,
        year: r.year,
        services,
        kind,
        genres,
        score: r.score,
        anilist_score: r.anilist_score,
        len: r.length,
        desc: r.description,
        cast,
        watched,
        rating,
        watchable,
        new_since,
    }))
}

#[cfg(test)]
mod tests {
    use sqlx::SqlitePool;

    use super::fetch_title;
    use crate::db::{init_pool, seed::seed_if_empty};

    async fn seeded_pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        seed_if_empty(&pool).await.unwrap();
        (pool, dir)
    }

    #[actix_web::test]
    async fn watchable_lists_only_resolvable_sources() {
        let (pool, _dir) = seeded_pool().await;
        // Give title 1 a plex_rating_key so the Plex watch link can be resolved.
        sqlx::query("UPDATE titles SET plex_rating_key = '49518' WHERE id = 1")
            .execute(&pool)
            .await
            .unwrap();
        // Plex membership with no link (link comes from rating key + machine id).
        sqlx::query(
            "INSERT OR REPLACE INTO title_services (title_id, service, link) VALUES (1, 'plex', NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        // Crunchyroll membership with a direct link.
        sqlx::query(
            "INSERT OR REPLACE INTO title_services (title_id, service, link) VALUES (1, 'crunchyroll', 'https://www.crunchyroll.com/x')",
        )
        .execute(&pool)
        .await
        .unwrap();
        // Disney membership with no link: not resolvable, must be excluded.
        sqlx::query(
            "INSERT OR REPLACE INTO title_services (title_id, service, link) VALUES (1, 'disney', NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        // Write the machine id so Plex is considered resolvable.
        crate::db::app_meta::set(&pool, "plex_machine_id", "MID")
            .await
            .unwrap();

        let dto = fetch_title(&pool, 1).await.unwrap().unwrap();
        assert!(
            dto.watchable.contains(&"plex".to_string()),
            "watchable should include plex; got {:?}",
            dto.watchable
        );
        assert!(
            dto.watchable.contains(&"crunchyroll".to_string()),
            "watchable should include crunchyroll; got {:?}",
            dto.watchable
        );
        assert!(
            !dto.watchable.contains(&"disney".to_string()),
            "a membership without a link must not be watchable; got {:?}",
            dto.watchable
        );
    }

    #[actix_web::test]
    async fn malformed_added_at_does_not_fail_the_catalogue() {
        let (pool, _dir) = seeded_pool().await;
        sqlx::query("UPDATE titles SET added_at = 'garbage' WHERE id = 1")
            .execute(&pool)
            .await
            .unwrap();
        let list = super::fetch_catalogue(&pool).await.unwrap();
        let bad = list.iter().find(|t| t.id == 1).unwrap();
        assert_eq!(bad.new_since, None, "unparseable added_at is never new");
        assert!(list.len() > 1, "the rest of the catalogue still loads");
        let dto = fetch_title(&pool, 1).await.unwrap().unwrap();
        assert_eq!(dto.new_since, None);
        // The NULL is skipped, not read as epoch 0 (which a `now` near the
        // epoch would otherwise report as "new since 0").
        let map = super::load_new_since(&pool, 1_000).await.unwrap();
        assert!(!map.contains_key(&1), "NULL added_at must be skipped");
    }
}

#[cfg(test)]
mod new_since_tests {
    use super::new_since_map;

    const NOW: i64 = 1_760_000_000;

    #[test]
    fn lone_recent_addition_is_new() {
        let m = new_since_map(&[(1, NOW - 86_400)], NOW);
        assert_eq!(m.get(&1), Some(&(NOW - 86_400)));
    }

    #[test]
    fn exactly_30_days_old_is_still_new() {
        let m = new_since_map(&[(1, NOW - 30 * 86_400)], NOW);
        assert!(m.contains_key(&1));
    }

    #[test]
    fn older_than_30_days_is_not_new() {
        assert!(new_since_map(&[(1, NOW - 30 * 86_400 - 1)], NOW).is_empty());
    }

    #[test]
    fn burst_of_300_within_30_minutes_is_bulk() {
        let rows: Vec<(i64, i64)> = (0..300).map(|i| (i, NOW - 3_600 + i)).collect();
        assert!(new_since_map(&rows, NOW).is_empty());
    }

    #[test]
    fn burst_of_299_is_new() {
        let rows: Vec<(i64, i64)> = (0..299).map(|i| (i, NOW - 3_600 + i)).collect();
        assert_eq!(new_since_map(&rows, NOW).len(), 299);
    }

    #[test]
    fn window_is_plus_minus_1800_seconds_inclusive() {
        // 299 at t, plus one at t+1800 (inside) -> 300 within ±1800 of t -> bulk;
        // one at t+1801 would be outside.
        let t = NOW - 7_200;
        let mut rows: Vec<(i64, i64)> = (0..299).map(|i| (i, t)).collect();
        rows.push((999, t + 1_800));
        assert!(!new_since_map(&rows, NOW).contains_key(&0));
    }

    #[test]
    fn row_just_outside_the_window_does_not_make_a_bulk() {
        let t = NOW - 7_200;
        let mut rows: Vec<(i64, i64)> = (0..299).map(|i| (i, t)).collect();
        rows.push((999, t + 1_801));
        let m = new_since_map(&rows, NOW);
        assert!(m.contains_key(&0));
        assert!(m.contains_key(&999));
    }

    #[test]
    fn old_bulk_does_not_hide_a_lone_recent_addition() {
        // 300 rows long ago (bulk, and also stale) plus one fresh row far away.
        let mut rows: Vec<(i64, i64)> = (0..300).map(|i| (i, NOW - 90 * 86_400 + i)).collect();
        rows.push((999, NOW - 86_400));
        let m = new_since_map(&rows, NOW);
        assert_eq!(m.len(), 1);
        assert!(m.contains_key(&999));
    }
}
