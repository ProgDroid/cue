//! `AniList` enrichment: offline Fribb id-map (imdb/tmdb -> anilist) + batched
//! score fetch. Additive and wipe-guarded — any failure retains cached scores.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use sqlx::SqlitePool;

/// Maps a title's external ids to its `AniList` id, built from Fribb's
/// `anime-list-full.json`. An id present here means the title IS anime.
#[derive(Debug, Default)]
pub struct AnimeIdMap {
    imdb_map: HashMap<String, i64>,
    tmdb_map: HashMap<String, i64>,
}

impl AnimeIdMap {
    /// Build the map from Fribb's `anime-list-full.json`. Defensive about the
    /// documented-but-irregular shapes (`imdb_id`: string|array; `themoviedb_id`:
    /// int|{tv,movie[]}) — anything unexpected is skipped, not fatal.
    ///
    /// # Errors
    /// Returns an error only if the top-level JSON is not an array.
    pub fn parse(json: &str) -> anyhow::Result<Self> {
        let entries: Vec<serde_json::Value> = serde_json::from_str(json)?;
        let mut imdb_map = HashMap::new();
        let mut tmdb_map = HashMap::new();
        for e in entries {
            let Some(anilist) = e.get("anilist_id").and_then(serde_json::Value::as_i64) else {
                continue;
            };
            match e.get("imdb_id") {
                Some(serde_json::Value::String(s)) => {
                    imdb_map.insert(s.clone(), anilist);
                }
                Some(serde_json::Value::Array(a)) => {
                    for v in a {
                        if let Some(s) = v.as_str() {
                            imdb_map.insert(s.to_string(), anilist);
                        }
                    }
                }
                _ => {}
            }
            match e.get("themoviedb_id") {
                Some(serde_json::Value::Number(n)) => {
                    if let Some(i) = n.as_i64() {
                        tmdb_map.insert(i.to_string(), anilist);
                    }
                }
                Some(serde_json::Value::Object(o)) => {
                    if let Some(i) = o.get("tv").and_then(serde_json::Value::as_i64) {
                        tmdb_map.insert(i.to_string(), anilist);
                    }
                    if let Some(serde_json::Value::Array(a)) = o.get("movie") {
                        for v in a {
                            if let Some(i) = v.as_i64() {
                                tmdb_map.insert(i.to_string(), anilist);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(Self { imdb_map, tmdb_map })
    }

    /// Resolve a title's `AniList` id from its external ids (imdb wins).
    /// cue's tmdb id may be prefixed (`"movie/9"`); the bare number is matched.
    #[must_use]
    pub fn resolve(&self, imdb_id: Option<&str>, tmdb_id: Option<&str>) -> Option<i64> {
        if let Some(i) = imdb_id {
            if let Some(a) = self.imdb_map.get(i) {
                return Some(*a);
            }
        }
        if let Some(t) = tmdb_id {
            let key = t.rsplit('/').next().unwrap_or(t);
            if let Some(a) = self.tmdb_map.get(key) {
                return Some(*a);
            }
        }
        None
    }
}

const ANILIST_URL: &str = "https://graphql.anilist.co";
const SCORES_QUERY: &str =
    "query($ids:[Int]){Page(perPage:50){media(id_in:$ids,type:ANIME){id averageScore}}}";

#[derive(Deserialize)]
struct GqlResp {
    data: Option<GqlData>,
}
#[derive(Deserialize)]
struct GqlData {
    #[serde(rename = "Page")]
    page: GqlPage,
}
#[derive(Deserialize)]
struct GqlPage {
    media: Vec<GqlMedia>,
}
#[derive(Deserialize)]
struct GqlMedia {
    id: i64,
    #[serde(rename = "averageScore")]
    average_score: Option<i64>,
}

/// Parse an `AniList` `Page` response into `(anilist_id, score/10)` pairs.
///
/// # Errors
/// Returns an error if the body is not the expected JSON shape.
pub fn parse_scores(body: &str) -> anyhow::Result<Vec<(i64, Option<f64>)>> {
    let r: GqlResp = serde_json::from_str(body)?;
    let media = r.data.map(|d| d.page.media).unwrap_or_default();
    Ok(media
        .into_iter()
        .map(|m| {
            #[allow(clippy::cast_precision_loss)] // scores are 0..=100, lossless in f64
            let s = m.average_score.map(|v| v as f64 / 10.0);
            (m.id, s)
        })
        .collect())
}

/// Fetch scores for up to 50 `AniList` ids in one request.
///
/// # Errors
/// Returns an error if the request fails or the body cannot be parsed.
pub async fn fetch_scores(
    client: &reqwest::Client,
    ids: &[i64],
) -> anyhow::Result<Vec<(i64, Option<f64>)>> {
    let body = serde_json::json!({ "query": SCORES_QUERY, "variables": { "ids": ids } });
    let text = client
        .post(ANILIST_URL)
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_scores(&text)
}

/// Write resolved `AniList` id + score for each `(title_id, anilist_id, score)`.
/// Returns the number of title rows updated.
///
/// # Errors
/// Returns an error if a write fails.
pub async fn persist_scores(
    pool: &SqlitePool,
    rows: &[(i64, i64, Option<f64>)],
) -> anyhow::Result<u64> {
    let mut updated = 0;
    let mut tx = pool.begin().await?;
    for (title_id, anilist_id, score) in rows {
        let r = sqlx::query("UPDATE titles SET anilist_id = ?, anilist_score = ? WHERE id = ?")
            .bind(anilist_id)
            .bind(score)
            .bind(title_id)
            .execute(&mut *tx)
            .await?;
        updated += r.rows_affected();
    }
    tx.commit().await?;
    Ok(updated)
}

/// Fribb mapping source + local cache TTL.
const FRIBB_URL: &str =
    "https://raw.githubusercontent.com/Fribb/anime-lists/master/anime-list-full.json";
const MAP_TTL_SECS: u64 = 7 * 24 * 60 * 60;

async fn load_map(client: &reqwest::Client, path: &Path) -> anyhow::Result<AnimeIdMap> {
    let fresh = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < MAP_TTL_SECS);
    if !fresh {
        let body = client
            .get(FRIBB_URL)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        AnimeIdMap::parse(&body)?; // validate before caching
        std::fs::write(path, &body)?;
    }
    AnimeIdMap::parse(&std::fs::read_to_string(path)?)
}

/// Enrich anime titles with `AniList` scores.
///
/// Resolves every title's external ids against the Fribb map; for matches still
/// missing a score, batch-fetches and persists. Wipe-guarded: any failure logs
/// and returns `Ok(0)` without clearing existing scores.
/// Returns the number of titles scored.
///
/// # Errors
/// Returns an error only if the initial titles query fails irrecoverably.
pub async fn enrich(pool: &SqlitePool, data_dir: &Path) -> anyhow::Result<u64> {
    if let Err(e) = std::fs::create_dir_all(data_dir) {
        tracing::error!("anilist: cannot create data dir: {e:#}; skipping");
        return Ok(0);
    }
    let client = reqwest::Client::new();
    let map = match load_map(&client, &data_dir.join("anime-list-full.json")).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("anilist: id-map load failed: {e:#}; retaining cached scores");
            return Ok(0);
        }
    };

    // (db_id, imdb_id, tmdb_id, existing_anilist_score)
    #[allow(clippy::type_complexity)] // 4-tuple from a raw query; a named struct would be overkill
    let titles: Vec<(i64, Option<String>, Option<String>, Option<f64>)> =
        sqlx::query_as("SELECT id, imdb_id, tmdb_id, anilist_score FROM titles")
            .fetch_all(pool)
            .await?;

    // (title_id, anilist_id) for matched anime that still need a score.
    let mut pending: Vec<(i64, i64)> = Vec::new();
    for (id, imdb, tmdb, existing) in titles {
        if existing.is_some() {
            continue; // already scored — skip (refresh is a deferred follow-up)
        }
        if let Some(anilist) = map.resolve(imdb.as_deref(), tmdb.as_deref()) {
            pending.push((id, anilist));
        }
    }
    if pending.is_empty() {
        return Ok(0);
    }

    let mut scored = 0;
    for chunk in pending.chunks(50) {
        let ids: Vec<i64> = chunk.iter().map(|(_, a)| *a).collect();
        let score_map = match fetch_scores(&client, &ids).await {
            Ok(s) => s.into_iter().collect::<HashMap<i64, Option<f64>>>(),
            Err(e) => {
                tracing::error!("anilist: score fetch failed: {e:#}; retaining cached scores");
                break; // keep whatever earlier chunks persisted; never wipe
            }
        };
        let rows: Vec<(i64, i64, Option<f64>)> = chunk
            .iter()
            .map(|(title_id, anilist)| {
                (
                    *title_id,
                    *anilist,
                    score_map.get(anilist).copied().flatten(),
                )
            })
            .collect();
        scored += persist_scores(pool, &rows).await?;
    }
    tracing::info!("anilist: scored {scored} anime titles");
    Ok(scored)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("testdata/fribb_sample.json");

    async fn pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (crate::db::init_pool(&url).await.unwrap(), dir)
    }

    #[tokio::test]
    async fn persist_writes_anilist_id_and_score() {
        let (p, _dir) = pool().await;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO titles (imdb_id, title, year, type) VALUES ('tt1','A',2020,'movie') RETURNING id",
        )
        .fetch_one(&p)
        .await
        .unwrap();
        let n = persist_scores(&p, &[(id, 101_759, Some(8.6))])
            .await
            .unwrap();
        assert_eq!(n, 1);
        let row: (Option<i64>, Option<f64>) =
            sqlx::query_as("SELECT anilist_id, anilist_score FROM titles WHERE id = ?")
                .bind(id)
                .fetch_one(&p)
                .await
                .unwrap();
        assert_eq!(row.0, Some(101_759));
        assert!((row.1.unwrap() - 8.6).abs() < 1e-9);
    }

    #[test]
    fn parses_anilist_page_scores_normalized_to_ten() {
        let body = r#"{"data":{"Page":{"media":[
        {"id":101759,"averageScore":86},
        {"id":21,"averageScore":null}
    ]}}}"#;
        let got = parse_scores(body).unwrap();
        assert_eq!(got.len(), 2);
        assert!((got[0].1.unwrap() - 8.6).abs() < 1e-9);
        assert_eq!(got[0].0, 101_759);
        assert_eq!(got[1].1, None);
    }

    #[test]
    fn resolves_anime_by_imdb_and_tmdb() {
        let map = AnimeIdMap::parse(SAMPLE).unwrap();

        // TV entry: anilist 290, imdb tt0286390, tmdb tv 26209
        assert_eq!(
            map.resolve(Some("tt0286390"), None),
            Some(290),
            "TV entry resolved by imdb_id"
        );

        // MOVIE entry: anilist 164, imdb tt0119698, tmdb movie [128]
        // Test tmdb match with cue's "movie/<id>" prefix form
        assert_eq!(
            map.resolve(None, Some("movie/128")),
            Some(164),
            "MOVIE entry resolved by tmdb with prefix"
        );

        // Bare tmdb number also matches
        assert_eq!(
            map.resolve(None, Some("128")),
            Some(164),
            "MOVIE entry resolved by bare tmdb id"
        );

        // Entry with no imdb_id, only tmdb tv: anilist 1596, tmdb tv 29241
        assert_eq!(
            map.resolve(None, Some("29241")),
            Some(1596),
            "OVA-no-imdb entry resolved by tmdb tv"
        );

        // imdb takes priority over tmdb when both are provided
        assert_eq!(
            map.resolve(Some("tt0286390"), Some("128")),
            Some(290),
            "imdb wins over tmdb"
        );

        // Unknown id -> None
        assert_eq!(
            map.resolve(Some("tt0000000"), None),
            None,
            "unknown imdb_id returns None"
        );
        assert_eq!(
            map.resolve(None, Some("movie/9999999")),
            None,
            "unknown tmdb_id returns None"
        );
        assert_eq!(map.resolve(None, None), None, "both None returns None");
    }
}
