//! `AniList` enrichment: offline Fribb id-map (imdb/tmdb -> anilist) + batched
//! score fetch. Additive and wipe-guarded — any failure retains cached scores.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;
use sqlx::SqlitePool;

use crate::models::TitleKind;

/// Maps a title's external ids to its `AniList` id, built from Fribb's
/// `anime-list-full.json`. An id present here means the title IS anime.
///
/// TMDB movie and tv are SEPARATE id namespaces (`movie/123` != `tv/123`), so
/// they are kept in distinct maps and disambiguated by the title's kind — a
/// live-action movie whose tmdb id collides numerically with an anime's tv id
/// must NOT match. Keys that resolve to more than one `AniList` id (e.g. a
/// franchise's OVAs sharing one imdb id) are dropped so we never surface an
/// arbitrary installment's score.
#[derive(Debug, Default)]
pub struct AnimeIdMap {
    imdb: HashMap<String, i64>,
    tmdb_tv: HashMap<String, i64>,
    tmdb_movie: HashMap<String, i64>,
}

/// Keep only keys that map to exactly one `AniList` id; drop ambiguous keys.
fn unambiguous(m: HashMap<String, HashSet<i64>>) -> HashMap<String, i64> {
    m.into_iter()
        .filter(|(_, v)| v.len() == 1)
        .map(|(k, v)| (k, v.into_iter().next().expect("len == 1")))
        .collect()
}

impl AnimeIdMap {
    /// Build the map from Fribb's `anime-list-full.json`. The real file uses
    /// `imdb_id`: array-of-strings and `themoviedb_id`: `{tv, movie[]}` object;
    /// anything unexpected is skipped, not fatal.
    ///
    /// # Errors
    /// Returns an error only if the top-level JSON is not an array.
    pub fn parse(json: &str) -> anyhow::Result<Self> {
        let entries: Vec<serde_json::Value> = serde_json::from_str(json)?;
        // Accumulate candidate ids per key, then drop any key with >1 distinct id.
        let mut imdb: HashMap<String, HashSet<i64>> = HashMap::new();
        let mut tmdb_tv: HashMap<String, HashSet<i64>> = HashMap::new();
        let mut tmdb_movie: HashMap<String, HashSet<i64>> = HashMap::new();
        for e in entries {
            let Some(anilist) = e.get("anilist_id").and_then(serde_json::Value::as_i64) else {
                continue;
            };
            match e.get("imdb_id") {
                Some(serde_json::Value::String(s)) => {
                    imdb.entry(s.clone()).or_default().insert(anilist);
                }
                Some(serde_json::Value::Array(a)) => {
                    for v in a {
                        if let Some(s) = v.as_str() {
                            imdb.entry(s.to_string()).or_default().insert(anilist);
                        }
                    }
                }
                _ => {}
            }
            if let Some(serde_json::Value::Object(o)) = e.get("themoviedb_id") {
                if let Some(i) = o.get("tv").and_then(serde_json::Value::as_i64) {
                    tmdb_tv.entry(i.to_string()).or_default().insert(anilist);
                }
                if let Some(serde_json::Value::Array(a)) = o.get("movie") {
                    for v in a {
                        if let Some(i) = v.as_i64() {
                            tmdb_movie.entry(i.to_string()).or_default().insert(anilist);
                        }
                    }
                }
            }
        }
        Ok(Self {
            imdb: unambiguous(imdb),
            tmdb_tv: unambiguous(tmdb_tv),
            tmdb_movie: unambiguous(tmdb_movie),
        })
    }

    /// Resolve a title's `AniList` id from its external ids (imdb wins). cue's
    /// tmdb id may be prefixed (`"movie/9"`); the bare number is matched against
    /// the namespace selected by `kind` (series -> tv ids, movie -> movie ids).
    #[must_use]
    pub fn resolve(
        &self,
        imdb_id: Option<&str>,
        tmdb_id: Option<&str>,
        kind: TitleKind,
    ) -> Option<i64> {
        if let Some(i) = imdb_id {
            if let Some(a) = self.imdb.get(i) {
                return Some(*a);
            }
        }
        if let Some(t) = tmdb_id {
            let key = t.rsplit('/').next().unwrap_or(t);
            let map = match kind {
                TitleKind::Series => &self.tmdb_tv,
                TitleKind::Movie => &self.tmdb_movie,
            };
            if let Some(a) = map.get(key) {
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

    // (db_id, imdb_id, tmdb_id, type, existing_anilist_score)
    #[allow(clippy::type_complexity)] // 5-tuple from a raw query; a named struct would be overkill
    let titles: Vec<(i64, Option<String>, Option<String>, String, Option<f64>)> =
        sqlx::query_as("SELECT id, imdb_id, tmdb_id, type, anilist_score FROM titles")
            .fetch_all(pool)
            .await?;

    // (title_id, anilist_id) for matched anime that still need a score. `kind`
    // selects the tmdb namespace so a movie can't match an anime's tv id.
    let mut pending: Vec<(i64, i64)> = Vec::new();
    for (id, imdb, tmdb, kind_str, existing) in titles {
        if existing.is_some() {
            continue; // already scored — skip (refresh is a deferred follow-up)
        }
        let kind = TitleKind::parse(&kind_str).unwrap_or(TitleKind::Movie);
        if let Some(anilist) = map.resolve(imdb.as_deref(), tmdb.as_deref(), kind) {
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
            map.resolve(Some("tt0286390"), None, TitleKind::Series),
            Some(290),
            "TV entry resolved by imdb_id"
        );

        // MOVIE entry: anilist 164, imdb tt0119698, tmdb movie [128]
        // Test tmdb match with cue's "movie/<id>" prefix form
        assert_eq!(
            map.resolve(None, Some("movie/128"), TitleKind::Movie),
            Some(164),
            "MOVIE entry resolved by tmdb with prefix"
        );

        // Bare tmdb number also matches (kind selects the movie namespace)
        assert_eq!(
            map.resolve(None, Some("128"), TitleKind::Movie),
            Some(164),
            "MOVIE entry resolved by bare tmdb id"
        );

        // Entry with no imdb_id, only tmdb tv: anilist 1596, tmdb tv 29241
        assert_eq!(
            map.resolve(None, Some("29241"), TitleKind::Series),
            Some(1596),
            "OVA-no-imdb entry resolved by tmdb tv"
        );

        // imdb takes priority over tmdb when both are provided
        assert_eq!(
            map.resolve(Some("tt0286390"), Some("128"), TitleKind::Series),
            Some(290),
            "imdb wins over tmdb"
        );

        // Unknown id -> None
        assert_eq!(
            map.resolve(Some("tt0000000"), None, TitleKind::Movie),
            None,
            "unknown imdb_id returns None"
        );
        assert_eq!(
            map.resolve(None, Some("movie/9999999"), TitleKind::Movie),
            None,
            "unknown tmdb_id returns None"
        );
        assert_eq!(
            map.resolve(None, None, TitleKind::Movie),
            None,
            "both None returns None"
        );
    }

    #[test]
    fn tmdb_namespace_separates_tv_and_movie() {
        // The same integer is a TV id for one anime and a MOVIE id for another —
        // TMDB's movie/tv id spaces are distinct, so the kind must select which.
        let json = r#"[
            {"anilist_id":1,"themoviedb_id":{"tv":500}},
            {"anilist_id":2,"themoviedb_id":{"movie":[500]}}
        ]"#;
        let map = AnimeIdMap::parse(json).unwrap();
        assert_eq!(map.resolve(None, Some("500"), TitleKind::Series), Some(1));
        assert_eq!(map.resolve(None, Some("500"), TitleKind::Movie), Some(2));
        assert_eq!(
            map.resolve(None, Some("movie/500"), TitleKind::Movie),
            Some(2),
            "prefixed form resolves by kind, not by the prefix"
        );

        // The "Dude, Where's My Car?" bug: a live-action MOVIE whose tmdb id
        // numerically collides with an anime's TV id must NOT match.
        let tv_only =
            AnimeIdMap::parse(r#"[{"anilist_id":9,"themoviedb_id":{"tv":777}}]"#).unwrap();
        assert_eq!(
            tv_only.resolve(None, Some("777"), TitleKind::Movie),
            None,
            "a movie must not match an anime tv id"
        );
    }

    #[test]
    fn ambiguous_keys_are_dropped() {
        // The "Dominion" bug: one imdb id (and one shared tmdb tv id) maps to
        // three different OVAs — there is no right single score, so drop it.
        let json = r#"[
            {"anilist_id":1151,"imdb_id":["tt0158591"],"themoviedb_id":{"tv":45137}},
            {"anilist_id":1152,"imdb_id":["tt0158591"],"themoviedb_id":{"tv":45137}},
            {"anilist_id":2181,"imdb_id":["tt0158591"],"themoviedb_id":{"tv":45137}}
        ]"#;
        let map = AnimeIdMap::parse(json).unwrap();
        assert_eq!(
            map.resolve(Some("tt0158591"), None, TitleKind::Series),
            None,
            "imdb id mapping to multiple anilist ids is ambiguous -> dropped"
        );
        assert_eq!(
            map.resolve(None, Some("45137"), TitleKind::Series),
            None,
            "tmdb tv id shared by the same OVAs is also dropped"
        );

        // A key that maps to exactly one id still resolves.
        let single = AnimeIdMap::parse(
            r#"[{"anilist_id":5,"imdb_id":["tt1"],"themoviedb_id":{"movie":[7]}}]"#,
        )
        .unwrap();
        assert_eq!(single.resolve(Some("tt1"), None, TitleKind::Movie), Some(5));
        assert_eq!(single.resolve(None, Some("7"), TitleKind::Movie), Some(5));
    }
}
