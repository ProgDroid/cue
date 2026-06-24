//! Plex client: list library sections and parse items into `FetchedTitle`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::models::{Service, TitleKind};
use crate::sync::{CatalogueSource, FetchedTitle, ImageRef, WatchRecord};

#[derive(Deserialize)]
struct Container {
    #[serde(rename = "MediaContainer")]
    media_container: MediaContainer,
}
#[derive(Deserialize)]
struct MediaContainer {
    #[serde(default, rename = "Metadata")]
    metadata: Vec<Meta>,
}
#[derive(Deserialize)]
struct Meta {
    #[serde(rename = "type")]
    kind: String,
    title: String,
    year: Option<i64>,
    summary: Option<String>,
    thumb: Option<String>,
    art: Option<String>,
    rating: Option<f64>,
    #[serde(rename = "audienceRating")]
    audience_rating: Option<f64>,
    duration: Option<i64>,
    #[serde(rename = "viewCount")]
    view_count: Option<i64>,
    #[serde(rename = "lastViewedAt")]
    last_viewed_at: Option<i64>,
    #[serde(rename = "viewedLeafCount")]
    viewed_leaf_count: Option<i64>,
    #[serde(default, rename = "Guid")]
    guid: Vec<Tagged>,
    #[serde(default, rename = "Genre")]
    genre: Vec<Tag>,
    #[serde(default, rename = "Role")]
    role: Vec<Tag>,
}
#[derive(Deserialize)]
struct Tagged {
    id: String,
}
#[derive(Deserialize)]
struct Tag {
    tag: String,
}

fn guid_value(guids: &[Tagged], scheme: &str) -> Option<String> {
    let prefix = format!("{scheme}://");
    guids
        .iter()
        .find_map(|g| g.id.strip_prefix(&prefix).map(str::to_string))
}

/// Path for a section's bulk listing, including `includeGuids=1`.
///
/// Plex does NOT return the per-item `<Guid>` external-ID array (`imdb://`,
/// `tmdb://`, `plex://`) in a bulk `/library/sections/{key}/all` response
/// unless `includeGuids=1` is requested. Without it, `parse_section` /
/// `parse_watch_history` see an empty `guid` vec and every title resolves to a
/// NULL `imdb_id` (so user-data writes 422 and watch-history rows are dropped).
fn section_all_path(key: &str) -> String {
    format!("/library/sections/{key}/all?includeGuids=1")
}

/// Parse one `/library/sections/{key}/all` JSON body into fetched titles.
///
/// Only `movie` and `show` items are kept. Any other top-level `type` is
/// skipped with a warning rather than silently coerced to a movie — in
/// practice `fetch()` only feeds movie/show sections, so this guards against
/// an unexpected section shape rather than a routine case.
///
/// # Errors
/// Returns an error if the JSON does not match the expected shape.
pub fn parse_section(json: &str) -> anyhow::Result<Vec<FetchedTitle>> {
    let parsed: Container = serde_json::from_str(json)?;
    Ok(parsed
        .media_container
        .metadata
        .into_iter()
        .filter_map(|m| {
            let kind = match m.kind.as_str() {
                "movie" => TitleKind::Movie,
                "show" => TitleKind::Series,
                other => {
                    tracing::warn!(
                        "skipping Plex item {:?} with unexpected type {:?}",
                        m.title,
                        other
                    );
                    return None;
                }
            };
            let length = m.duration.map(|ms| format!("{} min", ms / 60000));
            Some(FetchedTitle {
                imdb_id: guid_value(&m.guid, "imdb"),
                tmdb_id: guid_value(&m.guid, "tmdb"),
                plex_guid: guid_value(&m.guid, "plex"),
                title: m.title,
                year: m.year,
                kind,
                // Plex stores the IMDb/TMDB audience score in `audienceRating`;
                // the legacy `rating` field is empty for these libraries (and is
                // a critic score when present). Prefer audienceRating, fall back
                // to rating only when audienceRating is absent.
                score: m.audience_rating.or(m.rating),
                length,
                description: m.summary,
                genres: m.genre.into_iter().map(|g| g.tag).collect(),
                cast: m.role.into_iter().map(|r| r.tag).collect(),
                services: vec![Service::Plex],
                poster: m.thumb.map(|value| ImageRef {
                    value,
                    remote: false,
                }),
                backdrop: m.art.map(|value| ImageRef {
                    value,
                    remote: false,
                }),
            })
        })
        .collect())
}

/// Parse one `/library/sections/{key}/all` body into watched-title records.
///
/// A movie is "watched" when `viewCount > 0`; a show is "watched" (touched /
/// in-progress) when `viewedLeafCount > 0`. Items without an `imdb://` GUID are
/// skipped — user-data is keyed on `imdb_id` (D6), so a non-imdb watch row
/// would never surface in the catalogue read.
///
/// # Errors
/// Returns an error if the JSON does not match the expected shape.
pub fn parse_watch_history(json: &str) -> anyhow::Result<Vec<WatchRecord>> {
    let parsed: Container = serde_json::from_str(json)?;
    Ok(parsed
        .media_container
        .metadata
        .into_iter()
        .filter_map(|m| {
            let watched = match m.kind.as_str() {
                "movie" => m.view_count.unwrap_or(0) > 0,
                "show" => m.viewed_leaf_count.unwrap_or(0) > 0,
                _ => false,
            };
            if !watched {
                return None;
            }
            let key = guid_value(&m.guid, "imdb")?; // skip no-imdb items (W3)
            Some(WatchRecord {
                key,
                watched_at: m.last_viewed_at,
            })
        })
        .collect())
}

/// Live Plex client (raw HTTP — fetches sections then items).
pub struct PlexClient {
    client: reqwest::Client,
    base_url: String,
    token: String,
}

impl PlexClient {
    #[must_use]
    #[allow(clippy::needless_pass_by_value)] // trim_end_matches requires owned input
    pub fn new(base_url: String, token: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
        }
    }

    async fn get_json(&self, path: &str) -> anyhow::Result<String> {
        Ok(self
            .client
            .get(format!("{}{path}", self.base_url))
            .header("X-Plex-Token", &self.token)
            .header("Accept", "application/json")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?)
    }

    /// Walk every movie/show library section, applying `parse` to each section's
    /// `/all` body and flattening the results. Shared by `fetch` (titles) and
    /// `fetch_watch_history` (watch records) — only the per-body parser differs.
    async fn for_each_section<T>(
        &self,
        parse: impl Fn(&str) -> anyhow::Result<Vec<T>>,
    ) -> anyhow::Result<Vec<T>> {
        let sections: Sections = serde_json::from_str(&self.get_json("/library/sections").await?)?;
        let mut out = Vec::new();
        for dir in sections.media_container.directory {
            if dir.kind == "movie" || dir.kind == "show" {
                let body = self.get_json(&section_all_path(&dir.key)).await?;
                out.extend(parse(&body)?);
            }
        }
        Ok(out)
    }
}

#[derive(Deserialize)]
struct Sections {
    #[serde(rename = "MediaContainer")]
    media_container: SectionList,
}
#[derive(Deserialize)]
struct SectionList {
    #[serde(default, rename = "Directory")]
    directory: Vec<SectionDir>,
}
#[derive(Deserialize)]
struct SectionDir {
    key: String,
    #[serde(rename = "type")]
    kind: String,
}

#[async_trait]
impl CatalogueSource for PlexClient {
    fn name(&self) -> &'static str {
        "plex"
    }
    fn services(&self) -> &'static [Service] {
        &[Service::Plex]
    }

    async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>> {
        self.for_each_section(parse_section).await
    }

    fn watch_history_source(&self) -> Option<&'static str> {
        Some("plex")
    }

    async fn fetch_watch_history(&self) -> anyhow::Result<Vec<WatchRecord>> {
        self.for_each_section(parse_watch_history).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Service;

    #[test]
    fn parses_metadata_into_fetched_titles() {
        let json = include_str!("../../tests/fixtures/plex_section_all.json");
        let out = parse_section(json).unwrap();
        assert_eq!(out.len(), 2);

        let film = &out[0];
        assert_eq!(film.imdb_id.as_deref(), Some("tt1856101"));
        assert_eq!(film.tmdb_id.as_deref(), Some("335984"));
        assert_eq!(film.kind, TitleKind::Movie);
        assert_eq!(film.year, Some(2017));
        assert_eq!(film.genres, vec!["Sci-Fi".to_string(), "Drama".to_string()]);
        assert_eq!(
            film.cast,
            vec!["Ryan Gosling".to_string(), "Harrison Ford".to_string()]
        );
        assert_eq!(film.services, vec![Service::Plex]);
        assert_eq!(film.length.as_deref(), Some("164 min"));

        let show = &out[1];
        assert_eq!(show.kind, TitleKind::Series);
        assert_eq!(show.imdb_id.as_deref(), Some("tt11280740"));
    }

    #[test]
    fn section_all_path_requests_external_guids() {
        // Plex omits the <Guid> external-ID array (imdb://, tmdb://, plex://)
        // from a bulk /library/sections/{key}/all listing unless includeGuids=1
        // is set. Without it every Plex title deserializes with an empty guid
        // vec, so imdb_id/tmdb_id/plex_guid are all NULL and downstream
        // write/watch-history paths silently drop the title.
        let path = section_all_path("3");
        assert!(
            path.starts_with("/library/sections/3/all"),
            "unexpected base path: {path:?}"
        );
        assert!(
            path.contains("includeGuids=1"),
            "section listing must request external GUIDs, got {path:?}"
        );
    }

    #[test]
    fn parse_section_extracts_thumb_and_art_as_plex_refs() {
        let json = r#"{"MediaContainer":{"Metadata":[
          {"type":"movie","title":"M","year":2020,
           "thumb":"/library/metadata/1/thumb/9","art":"/library/metadata/1/art/9",
           "Guid":[{"id":"imdb://tt1"}]}
        ]}}"#;
        let out = parse_section(json).unwrap();
        let p = out[0].poster.as_ref().unwrap();
        assert!(!p.remote);
        assert_eq!(p.value, "/library/metadata/1/thumb/9");
        let b = out[0].backdrop.as_ref().unwrap();
        assert!(!b.remote);
        assert_eq!(b.value, "/library/metadata/1/art/9");
    }

    #[test]
    fn parse_section_reads_audience_rating_as_score() {
        // Plex stores the IMDb/TMDB audience score in `audienceRating`; the
        // legacy `rating` field is empty for these libraries (confirmed live:
        // 0/193 had `rating`, all had `audienceRating`). Prefer audienceRating,
        // fall back to `rating` only when audienceRating is absent.
        let json = r#"{"MediaContainer":{"Metadata":[
          {"type":"movie","title":"Alien","year":1979,"audienceRating":8.4,
           "audienceRatingImage":"imdb://image.rating","Guid":[{"id":"imdb://tt1"}]},
          {"type":"movie","title":"Both","year":2000,"rating":5.0,"audienceRating":7.2,
           "Guid":[{"id":"imdb://tt2"}]},
          {"type":"movie","title":"CriticOnly","year":2001,"rating":6.1,
           "Guid":[{"id":"imdb://tt3"}]}
        ]}}"#;
        let out = parse_section(json).unwrap();
        assert_eq!(out[0].score, Some(8.4), "audienceRating used as score");
        assert_eq!(
            out[1].score,
            Some(7.2),
            "audienceRating preferred over critic rating"
        );
        assert_eq!(
            out[2].score,
            Some(6.1),
            "falls back to rating when audienceRating absent"
        );
    }

    #[test]
    fn parse_section_handles_missing_thumb_art() {
        let json = r#"{"MediaContainer":{"Metadata":[
          {"type":"movie","title":"M","year":2020,"Guid":[{"id":"imdb://tt1"}]}
        ]}}"#;
        let out = parse_section(json).unwrap();
        assert!(out[0].poster.is_none());
        assert!(out[0].backdrop.is_none());
    }

    #[test]
    fn skips_items_with_unexpected_type() {
        // A "movie", a "show", and a stray top-level type (e.g. a collection).
        // Only the movie and show should survive; the unknown type is dropped
        // rather than coerced into a Movie.
        let json = r#"{
            "MediaContainer": {
                "Metadata": [
                    {"type": "movie", "title": "A Film", "year": 2020},
                    {"type": "collection", "title": "A Collection"},
                    {"type": "show", "title": "A Series", "year": 2021}
                ]
            }
        }"#;
        let out = parse_section(json).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].title, "A Film");
        assert_eq!(out[0].kind, TitleKind::Movie);
        assert_eq!(out[1].title, "A Series");
        assert_eq!(out[1].kind, TitleKind::Series);
    }

    #[test]
    fn parse_watch_history_keeps_watched_movies_and_inprogress_shows() {
        let json = r#"{"MediaContainer":{"Metadata":[
          {"type":"movie","title":"Watched Film","viewCount":1,"lastViewedAt":1600000000,
           "Guid":[{"id":"imdb://tt1"}]},
          {"type":"movie","title":"Unwatched Film","viewCount":0,
           "Guid":[{"id":"imdb://tt2"}]},
          {"type":"movie","title":"Never-Played Film",
           "Guid":[{"id":"imdb://tt3"}]},
          {"type":"show","title":"In-Progress Show","viewedLeafCount":4,"lastViewedAt":1700000000,
           "Guid":[{"id":"imdb://tt4"}]},
          {"type":"show","title":"Untouched Show","viewedLeafCount":0,
           "Guid":[{"id":"imdb://tt5"}]},
          {"type":"movie","title":"Watched No IMDb","viewCount":2,
           "Guid":[{"id":"tmdb://999"}]}
        ]}}"#;
        let out = parse_watch_history(json).unwrap();
        // Watched movie (tt1) + in-progress show (tt4) only. tt2/tt3/tt5 not watched;
        // the no-imdb watched item is skipped (W3).
        let keys: Vec<&str> = out.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, vec!["tt1", "tt4"]);
        assert_eq!(out[0].watched_at, Some(1_600_000_000));
        assert_eq!(out[1].watched_at, Some(1_700_000_000));
    }
}
