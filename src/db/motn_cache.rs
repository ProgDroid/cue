//! MOTN's self-contained catalogue cache (showId-keyed), used to reconstruct a
//! full snapshot from `/changes` deltas.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::models::{Service, TitleKind};
use crate::sync::{FetchedTitle, ImageRef};

/// Serialized form of a MOTN title stored in `motn_catalog_cache.payload`.
///
/// Mirrors `FetchedTitle` but stores `kind`/`services` as strings so the JSON is
/// independent of the enum reprs. `plex_guid` is omitted — MOTN never sets it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedTitle {
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub title: String,
    pub year: Option<i64>,
    pub kind: String,
    #[serde(alias = "imdb_rating")]
    pub score: Option<f64>,
    pub length: Option<String>,
    pub description: Option<String>,
    pub genres: Vec<String>,
    pub cast: Vec<String>,
    pub services: Vec<String>,
    /// Per-service watch links, stored as `(service_str, url)` so the JSON is
    /// enum-repr-independent. `#[serde(default)]` keeps pre-existing cached rows
    /// (written before this field) deserializable.
    #[serde(default)]
    pub links: Vec<(String, String)>,
    #[serde(default)]
    pub poster_url: Option<String>,
    #[serde(default)]
    pub backdrop_url: Option<String>,
}

impl From<&FetchedTitle> for CachedTitle {
    fn from(t: &FetchedTitle) -> Self {
        Self {
            imdb_id: t.imdb_id.clone(),
            tmdb_id: t.tmdb_id.clone(),
            title: t.title.clone(),
            year: t.year,
            kind: t.kind.as_str().to_string(),
            score: t.score,
            length: t.length.clone(),
            description: t.description.clone(),
            genres: t.genres.clone(),
            cast: t.cast.clone(),
            services: t.services.iter().map(|s| s.as_str().to_string()).collect(),
            links: t
                .links
                .iter()
                .map(|(s, l)| (s.as_str().to_string(), l.clone()))
                .collect(),
            poster_url: t
                .poster
                .as_ref()
                .filter(|i| i.remote)
                .map(|i| i.value.clone()),
            backdrop_url: t
                .backdrop
                .as_ref()
                .filter(|i| i.remote)
                .map(|i| i.value.clone()),
        }
    }
}

impl CachedTitle {
    /// Rebuild a `FetchedTitle`. Unknown service/kind strings are dropped/defaulted
    /// defensively; in practice they always parse since we wrote them via `as_str`.
    #[must_use]
    pub fn into_fetched(self) -> FetchedTitle {
        FetchedTitle {
            imdb_id: self.imdb_id,
            tmdb_id: self.tmdb_id,
            plex_guid: None,
            title: self.title,
            year: self.year,
            kind: TitleKind::parse(&self.kind).unwrap_or(TitleKind::Movie),
            score: self.score,
            length: self.length,
            description: self.description,
            genres: self.genres,
            cast: self.cast,
            services: self
                .services
                .iter()
                .filter_map(|s| Service::parse(s))
                .collect(),
            plex_rating_key: None,
            links: self
                .links
                .iter()
                .filter_map(|(s, l)| Service::parse(s).map(|svc| (svc, l.clone())))
                .collect(),
            poster: self.poster_url.map(|value| ImageRef {
                value,
                remote: true,
            }),
            backdrop: self.backdrop_url.map(|value| ImageRef {
                value,
                remote: true,
            }),
        }
    }
}

/// Number of rows currently cached.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn count(pool: &SqlitePool) -> anyhow::Result<i64> {
    let n = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM motn_catalog_cache")
        .fetch_one(pool)
        .await?;
    Ok(n)
}

/// Insert or replace one cached title by `show_id`.
///
/// # Errors
/// Returns an error if serialization or the write fails.
pub async fn upsert(pool: &SqlitePool, show_id: &str, t: &CachedTitle) -> anyhow::Result<()> {
    let payload = serde_json::to_string(t)?;
    sqlx::query(
        "INSERT INTO motn_catalog_cache (show_id, payload, updated_at)
         VALUES (?, ?, datetime('now'))
         ON CONFLICT(show_id) DO UPDATE SET payload = excluded.payload,
                                            updated_at = excluded.updated_at",
    )
    .bind(show_id)
    .bind(payload)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete one cached title by `show_id` (no-op if absent).
///
/// # Errors
/// Returns an error if the write fails.
pub async fn delete(pool: &SqlitePool, show_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM motn_catalog_cache WHERE show_id = ?")
        .bind(show_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Atomically replace the entire cache with `entries` (used by full seed/re-seed).
///
/// # Errors
/// Returns an error if serialization or any write fails; the transaction rolls back.
pub async fn replace_all(
    pool: &SqlitePool,
    entries: &[(String, CachedTitle)],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM motn_catalog_cache")
        .execute(&mut *tx)
        .await?;
    for (show_id, t) in entries {
        let payload = serde_json::to_string(t)?;
        sqlx::query("INSERT INTO motn_catalog_cache (show_id, payload) VALUES (?, ?)")
            .bind(show_id)
            .bind(payload)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Load all cached titles as a reconstructed full snapshot.
///
/// # Errors
/// Returns an error if the query or any payload deserialization fails.
pub async fn load_all(pool: &SqlitePool) -> anyhow::Result<Vec<FetchedTitle>> {
    let rows =
        sqlx::query_scalar::<_, String>("SELECT payload FROM motn_catalog_cache ORDER BY show_id")
            .fetch_all(pool)
            .await?;
    let mut out = Vec::with_capacity(rows.len());
    for payload in rows {
        let ct: CachedTitle = serde_json::from_str(&payload)?;
        out.push(ct.into_fetched());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_pool;

    async fn pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let url = format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        (dir, pool)
    }

    fn sample() -> FetchedTitle {
        FetchedTitle {
            imdb_id: Some("tt1".into()),
            tmdb_id: Some("movie/9".into()),
            plex_guid: None,
            title: "Sample".into(),
            year: Some(2021),
            kind: TitleKind::Movie,
            score: Some(7.5),
            length: Some("90 min".into()),
            description: Some("desc".into()),
            genres: vec!["drama".into()],
            cast: vec!["A".into(), "B".into()],
            services: vec![Service::Disney],
            plex_rating_key: None,
            links: vec![],
            poster: None,
            backdrop: None,
        }
    }

    #[tokio::test]
    async fn roundtrips_a_title_through_the_cache() {
        let (_dir, pool) = pool().await;
        let ct = CachedTitle::from(&sample());
        upsert(&pool, "100", &ct).await.unwrap();
        assert_eq!(count(&pool).await.unwrap(), 1);
        let all = load_all(&pool).await.unwrap();
        assert_eq!(all, vec![sample()]);
    }

    #[tokio::test]
    async fn upsert_replaces_existing_show_id() {
        let (_dir, pool) = pool().await;
        upsert(&pool, "100", &CachedTitle::from(&sample()))
            .await
            .unwrap();
        let mut other = sample();
        other.title = "Renamed".into();
        upsert(&pool, "100", &CachedTitle::from(&other))
            .await
            .unwrap();
        assert_eq!(count(&pool).await.unwrap(), 1);
        assert_eq!(load_all(&pool).await.unwrap()[0].title, "Renamed");
    }

    #[tokio::test]
    async fn delete_removes_by_show_id() {
        let (_dir, pool) = pool().await;
        upsert(&pool, "100", &CachedTitle::from(&sample()))
            .await
            .unwrap();
        delete(&pool, "100").await.unwrap();
        assert_eq!(count(&pool).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn replace_all_wipes_then_inserts() {
        let (_dir, pool) = pool().await;
        upsert(&pool, "stale", &CachedTitle::from(&sample()))
            .await
            .unwrap();
        let entries = vec![
            ("a".to_string(), CachedTitle::from(&sample())),
            ("b".to_string(), CachedTitle::from(&sample())),
        ];
        replace_all(&pool, &entries).await.unwrap();
        assert_eq!(count(&pool).await.unwrap(), 2); // "stale" gone
    }

    #[test]
    fn cached_round_trip_preserves_remote_images() {
        use crate::sync::ImageRef;
        let mut ft = sample();
        ft.poster = Some(ImageRef {
            value: "https://cdn/p.jpg".into(),
            remote: true,
        });
        ft.backdrop = Some(ImageRef {
            value: "https://cdn/b.jpg".into(),
            remote: true,
        });

        let cached = CachedTitle::from(&ft);
        let json = serde_json::to_string(&cached).unwrap();
        let back: CachedTitle = serde_json::from_str(&json).unwrap();
        let rebuilt = back.into_fetched();

        assert_eq!(rebuilt.poster, ft.poster);
        assert_eq!(rebuilt.backdrop, ft.backdrop);
    }

    #[test]
    fn cached_defaults_images_for_old_payloads() {
        // A payload written before this feature has no image keys.
        let json = r#"{"imdb_id":"tt1","tmdb_id":null,"title":"X","year":2020,"kind":"movie","imdb_rating":null,"length":null,"description":null,"genres":[],"cast":[],"services":[]}"#;
        let back: CachedTitle = serde_json::from_str(json).unwrap();
        let rebuilt = back.into_fetched();
        assert!(rebuilt.poster.is_none());
        assert!(rebuilt.backdrop.is_none());
    }

    #[test]
    fn cached_title_roundtrips_links() {
        use crate::models::Service;
        let mut ft = sample();
        ft.links = vec![(Service::Crunchyroll, "https://crunchyroll.com/x".into())];
        let cached = CachedTitle::from(&ft);
        let back = cached.into_fetched();
        assert_eq!(
            back.links,
            vec![(Service::Crunchyroll, "https://crunchyroll.com/x".into())]
        );
    }
}
