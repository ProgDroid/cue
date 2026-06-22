//! Plex client: list library sections and parse items into `FetchedTitle`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::models::{Service, TitleKind};
use crate::sync::{CatalogueSource, FetchedTitle};

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
    rating: Option<f64>,
    duration: Option<i64>,
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
                imdb_rating: m.rating,
                length,
                description: m.summary,
                genres: m.genre.into_iter().map(|g| g.tag).collect(),
                cast: m.role.into_iter().map(|r| r.tag).collect(),
                services: vec![Service::Plex],
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
        let sections: Sections = serde_json::from_str(&self.get_json("/library/sections").await?)?;
        let mut out = Vec::new();
        for dir in sections.media_container.directory {
            if dir.kind == "movie" || dir.kind == "show" {
                let body = self
                    .get_json(&format!("/library/sections/{}/all", dir.key))
                    .await?;
                out.extend(parse_section(&body)?);
            }
        }
        Ok(out)
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
}
