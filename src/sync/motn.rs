//! Movie-of-the-Night (Streaming Availability) client for UK Disney+/Crunchyroll.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use serde::Deserialize;

use crate::models::{Service, TitleKind};
use crate::sync::{CatalogueSource, FetchedTitle};

const MOTN_BASE: &str = "https://api.movieofthenight.com/v4";

/// The services we sync and their API catalog-id candidates.
const WANTED: [(Service, &str); 2] = [
    (Service::Disney, "disney"),
    (Service::Crunchyroll, "crunchyroll"),
];

/// From a `/v4/countries` body, return `(Service, catalog_id)` for the wanted
/// services that the given country actually lists. Absent services are skipped.
#[must_use]
pub fn resolve_services(countries_json: &str, country: &str) -> Vec<(Service, String)> {
    let parsed: HashMap<String, CountryEntry> = match serde_json::from_str(countries_json) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("MOTN /countries response did not parse: {e}");
            return Vec::new();
        }
    };
    let Some(entry) = parsed.get(country) else {
        return Vec::new();
    };
    let available: HashSet<&str> = entry.services.iter().map(|s| s.id.as_str()).collect();
    WANTED
        .iter()
        .filter(|(_, id)| available.contains(id))
        .map(|(svc, id)| (*svc, (*id).to_string()))
        .collect()
}

#[derive(Deserialize, Default)]
struct CountryEntry {
    // `/v4/countries` returns `services` as an ARRAY of service objects (each
    // with an `id`), NOT a map keyed by id. Deserializing it as a map silently
    // fails the whole parse and resolves to no services.
    #[serde(default)]
    services: Vec<ServiceEntry>,
}

#[derive(Deserialize)]
struct ServiceEntry {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    #[serde(default)]
    shows: Vec<Show>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Show {
    imdb_id: Option<String>,
    tmdb_id: Option<String>,
    title: String,
    overview: Option<String>,
    #[allow(clippy::struct_field_names)] // mirrors camelCase API field `showType`
    show_type: String,
    release_year: Option<i64>,
    first_air_year: Option<i64>,
    #[serde(default)]
    genres: Vec<Named>,
    #[serde(default)]
    cast: Vec<String>,
    rating: Option<f64>,
    runtime: Option<i64>,
    episode_count: Option<i64>,
    // Per-country streaming availability; each option names the `service` it's on.
    #[serde(default)]
    streaming_options: HashMap<String, Vec<StreamOption>>,
}

#[derive(Deserialize)]
struct StreamOption {
    service: ServiceRef,
}

#[derive(Deserialize)]
struct ServiceRef {
    id: String,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

/// Parse one search page into `(titles, next_cursor)`.
///
/// Each title is stamped with only the wanted services its
/// `streamingOptions[country]` actually lists, so a combined
/// `disney,crunchyroll` search attributes each title correctly instead of
/// tagging every result with both services.
///
/// # Errors
/// Returns an error if the JSON does not match the expected shape.
pub fn parse_page(
    json: &str,
    country: &str,
    services: &[Service],
) -> anyhow::Result<(Vec<FetchedTitle>, Option<String>)> {
    let page: Page = serde_json::from_str(json)?;
    let titles = page
        .shows
        .into_iter()
        .map(|s| {
            let kind = if s.show_type == "series" {
                TitleKind::Series
            } else {
                TitleKind::Movie
            };
            let year = s.release_year.or(s.first_air_year);
            let length = match kind {
                TitleKind::Movie => s.runtime.map(|m| format!("{m} min")),
                TitleKind::Series => s.episode_count.map(|e| format!("{e} eps")),
            };
            // Attribute only the wanted services this title is actually on
            // (scoped so the borrow of `streaming_options` ends before the moves).
            let svcs = {
                let available: HashSet<&str> = s
                    .streaming_options
                    .get(country)
                    .map(|opts| opts.iter().map(|o| o.service.id.as_str()).collect())
                    .unwrap_or_default();
                let attributed: Vec<Service> = services
                    .iter()
                    .copied()
                    .filter(|svc| wanted_id(*svc).is_some_and(|id| available.contains(id)))
                    .collect();
                // A search result should be on at least one searched service; if
                // streamingOptions is missing/unexpected, never drop the title —
                // fall back to the searched set.
                if attributed.is_empty() {
                    services.to_vec()
                } else {
                    attributed
                }
            };
            FetchedTitle {
                imdb_id: s.imdb_id,
                tmdb_id: s.tmdb_id,
                plex_guid: None,
                title: s.title,
                year,
                kind,
                imdb_rating: s.rating.map(|r| r / 10.0),
                length,
                description: s.overview,
                genres: s.genres.into_iter().map(|g| g.name).collect(),
                cast: s.cast,
                services: svcs,
            }
        })
        .collect();
    let cursor = if page.has_more {
        page.next_cursor
    } else {
        None
    };
    Ok((titles, cursor))
}

/// The MOTN catalog id for a wanted service (`Disney` → "disney", etc.).
fn wanted_id(svc: Service) -> Option<&'static str> {
    WANTED.iter().find(|(s, _)| *s == svc).map(|(_, id)| *id)
}

/// Live MOTN client.
pub struct MotnClient {
    client: reqwest::Client,
    api_key: String,
    country: String,
}

impl MotnClient {
    #[must_use]
    pub fn new(api_key: String, country: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            country,
        }
    }
}

#[async_trait]
impl CatalogueSource for MotnClient {
    fn name(&self) -> &'static str {
        "motn"
    }

    fn services(&self) -> &'static [Service] {
        &[Service::Disney, Service::Crunchyroll]
    }

    async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>> {
        let countries = self
            .client
            .get(format!("{MOTN_BASE}/countries"))
            .header("X-API-Key", &self.api_key)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let resolved = resolve_services(&countries, &self.country);
        if resolved.is_empty() {
            tracing::warn!(
                "MOTN lists none of [disney, crunchyroll] for {}",
                self.country
            );
            return Ok(Vec::new());
        }
        let catalog_ids: Vec<String> = resolved.iter().map(|(_, id)| id.clone()).collect();
        let services: Vec<Service> = resolved.iter().map(|(s, _)| *s).collect();
        let catalogs = catalog_ids.join(",");

        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut req = self
                .client
                .get(format!("{MOTN_BASE}/shows/search/filters"))
                .header("X-API-Key", &self.api_key)
                .query(&[
                    ("country", self.country.as_str()),
                    ("catalogs", catalogs.as_str()),
                ]);
            if let Some(c) = &cursor {
                req = req.query(&[("cursor", c.as_str())]);
            }
            let body = req.send().await?.error_for_status()?.text().await?;
            let (mut titles, next) = parse_page(&body, &self.country, &services)?;
            out.append(&mut titles);
            match next {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Service, TitleKind};

    #[test]
    fn resolve_services_maps_present_and_skips_absent() {
        let json = include_str!("../../tests/fixtures/motn_countries.json");
        let resolved = resolve_services(json, "gb");
        let disney = resolved.iter().find(|(s, _)| *s == Service::Disney);
        let crunchy = resolved.iter().find(|(s, _)| *s == Service::Crunchyroll);
        assert_eq!(disney.map(|(_, id)| id.as_str()), Some("disney"));
        assert_eq!(crunchy.map(|(_, id)| id.as_str()), Some("crunchyroll"));
        assert_eq!(resolved.len(), 2, "only our two services, never netflix");
    }

    #[test]
    fn parse_page_reads_movie_fields_and_cursor() {
        let json = include_str!("../../tests/fixtures/motn_search_page1.json");
        let (titles, cursor) = parse_page(json, "gb", &[Service::Disney]).unwrap();
        assert_eq!(cursor.as_deref(), Some("354912:Coco"));
        let t = &titles[0];
        assert_eq!(t.imdb_id.as_deref(), Some("tt2380307"));
        assert_eq!(t.kind, TitleKind::Movie);
        assert_eq!(t.year, Some(2017));
        assert_eq!(t.length.as_deref(), Some("105 min"));
        assert!(
            (t.imdb_rating.unwrap() - 8.2).abs() < 1e-9,
            "rating 82 -> 8.2"
        );
        assert_eq!(t.services, vec![Service::Disney]);
    }

    #[test]
    fn parse_page_reads_series_and_terminal_cursor() {
        let json = include_str!("../../tests/fixtures/motn_search_page2.json");
        let (titles, cursor) = parse_page(json, "gb", &[Service::Crunchyroll]).unwrap();
        assert_eq!(cursor, None);
        let t = &titles[0];
        assert_eq!(t.kind, TitleKind::Series);
        assert_eq!(t.year, Some(2024));
        assert_eq!(t.length.as_deref(), Some("25 eps"));
        assert_eq!(t.services, vec![Service::Crunchyroll]);
    }

    #[test]
    fn parse_page_attributes_services_per_title() {
        // Combined disney,crunchyroll search: each title must be tagged only with
        // the service(s) its streamingOptions[gb] actually lists, not both.
        let json = include_str!("../../tests/fixtures/motn_search_mixed.json");
        let (titles, _) =
            parse_page(json, "gb", &[Service::Disney, Service::Crunchyroll]).unwrap();
        assert_eq!(
            titles[0].services,
            vec![Service::Disney],
            "disney-only title tagged disney only"
        );
        assert_eq!(
            titles[1].services,
            vec![Service::Crunchyroll],
            "crunchyroll title (also a Prime addon) tagged crunchyroll only, not both"
        );
    }

    #[test]
    fn parse_page_falls_back_when_streaming_options_absent() {
        // No streamingOptions at all -> never drop the title; keep the searched set.
        let json = r#"{ "shows": [ { "title": "X", "showType": "movie" } ], "hasMore": false }"#;
        let (titles, _) = parse_page(json, "gb", &[Service::Disney]).unwrap();
        assert_eq!(titles[0].services, vec![Service::Disney]);
    }
}
