//! Movie-of-the-Night (Streaming Availability) client for UK Disney+/Crunchyroll.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::db::motn_cache::CachedTitle;
use crate::db::{motn_cache, sync_runs};
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

/// One `/changes` response page. `shows` is a map keyed by showId.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangesPage {
    #[serde(default)]
    changes: Vec<ChangeEntry>,
    #[serde(default)]
    shows: HashMap<String, Show>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangeEntry {
    change_type: String,
    #[serde(default)]
    item_type: Option<String>,
    show_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Show {
    #[serde(default)]
    id: String,
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

/// Map one MOTN show into a `FetchedTitle`, attributing only the wanted services
/// its per-country `streamingOptions` actually lists (falling back to the searched
/// set when availability is missing). Shared by the seed and `/changes` paths.
fn show_to_fetched(s: Show, country: &str, services: &[Service]) -> FetchedTitle {
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
}

/// `(show_id, title)` pairs from one search page, used to seed the cache.
type PageEntry = (String, FetchedTitle);

/// Parse one search page into `(titles, next_cursor)`. (Kept for callers/tests;
/// delegates to `parse_page_entries` and drops the show ids.)
///
/// # Errors
/// Returns an error if the JSON does not match the expected shape.
pub fn parse_page(
    json: &str,
    country: &str,
    services: &[Service],
) -> anyhow::Result<(Vec<FetchedTitle>, Option<String>)> {
    let (entries, cursor) = parse_page_entries(json, country, services)?;
    let titles = entries.into_iter().map(|(_, ft)| ft).collect();
    Ok((titles, cursor))
}

/// Parse one search page into `((show_id, FetchedTitle), next_cursor)` for seeding
/// the cache.
///
/// # Errors
/// Returns an error if the JSON does not match the expected shape.
pub fn parse_page_entries(
    json: &str,
    country: &str,
    services: &[Service],
) -> anyhow::Result<(Vec<PageEntry>, Option<String>)> {
    let page: Page = serde_json::from_str(json)?;
    let entries = page
        .shows
        .into_iter()
        .map(|s| (s.id.clone(), show_to_fetched(s, country, services)))
        .collect();
    let cursor = if page.has_more {
        page.next_cursor
    } else {
        None
    };
    Ok((entries, cursor))
}

/// Additions/removals distilled from one or more `/changes` pages.
#[derive(Debug, Default)]
pub struct ParsedChanges {
    pub additions: Vec<(String, CachedTitle)>, // (show_id, title)
    pub removals: Vec<String>,                 // show_ids
}

impl ParsedChanges {
    #[allow(dead_code)] // used by fetch() in the delta loop (Task 5)
    fn merge(&mut self, mut other: Self) {
        self.additions.append(&mut other.additions);
        self.removals.append(&mut other.removals);
    }
}

/// Parse one `/changes` page into `(ParsedChanges, next_cursor)`.
///
/// `new` changes are turned into cache upserts using the embedded `shows` detail;
/// a `new` change whose show is missing from `shows` or lacks an `imdbId` is
/// skipped with a warning rather than aborting the sync. `removed` changes need
/// only the `showId`. Non-show item types are ignored.
///
/// # Errors
/// Returns an error only if the page JSON itself does not parse.
pub fn parse_changes(
    json: &str,
    country: &str,
    services: &[Service],
) -> anyhow::Result<(ParsedChanges, Option<String>)> {
    let mut page: ChangesPage = serde_json::from_str(json)?;
    let mut out = ParsedChanges::default();
    for ch in &page.changes {
        if ch.item_type.as_deref().is_some_and(|t| t != "show") {
            continue;
        }
        match ch.change_type.as_str() {
            "new" => {
                let Some(show) = page.shows.remove(&ch.show_id) else {
                    tracing::warn!("MOTN /changes 'new' {} missing show detail", ch.show_id);
                    continue;
                };
                let ft = show_to_fetched(show, country, services);
                if ft.imdb_id.is_none() {
                    tracing::warn!("MOTN /changes 'new' {} has no imdbId; skipping", ch.show_id);
                    continue;
                }
                out.additions
                    .push((ch.show_id.clone(), CachedTitle::from(&ft)));
            }
            "removed" => out.removals.push(ch.show_id.clone()),
            _ => {}
        }
    }
    let cursor = if page.has_more {
        page.next_cursor
    } else {
        None
    };
    Ok((out, cursor))
}

/// Safety overlap subtracted from the `from` timestamp so a change straddling the
/// previous run's boundary is never missed. Idempotent: re-applying a `new`/`removed`
/// for the same `showId` is a no-op.
#[allow(dead_code)] // consumed by fetch() in the incremental-sync loop (Task 5)
const CHANGES_OVERLAP_SECS: i64 = 6 * 3600;

/// Which fetch strategy this run uses.
#[allow(dead_code)] // consumed by fetch() in the incremental-sync loop (Task 5)
enum SyncMode {
    /// Full pagination of `/shows/search/filters`, replacing the whole cache.
    Seed,
    /// `/changes` since `from` (Unix seconds), applied to the existing cache.
    Delta { from: i64 },
}

/// Decide between a full seed and an incremental delta:
/// - empty cache (fresh install) → `Seed`
/// - no successful MOTN run within 25 days (gap exceeds the 31-day window) → `Seed`
/// - otherwise → `Delta` from the last-ok timestamp minus the overlap buffer.
///
/// # Errors
/// Returns an error if any database query fails.
#[allow(dead_code)] // consumed by fetch() in the incremental-sync loop (Task 5)
async fn decide_mode(pool: &SqlitePool) -> anyhow::Result<SyncMode> {
    if motn_cache::count(pool).await? == 0 || !sync_runs::motn_recent_ok(pool).await? {
        return Ok(SyncMode::Seed);
    }
    let last_ok = sync_runs::last_ok_unix(pool).await?.unwrap_or(0);
    Ok(SyncMode::Delta {
        from: last_ok.saturating_sub(CHANGES_OVERLAP_SECS).max(0),
    })
}

/// Apply parsed `/changes` to the cache: upsert additions, delete removals.
///
/// # Errors
/// Returns an error if any cache write fails.
#[allow(dead_code)] // consumed by fetch() in the incremental-sync loop (Task 5)
async fn apply_changes(pool: &SqlitePool, parsed: &ParsedChanges) -> anyhow::Result<()> {
    for (show_id, ct) in &parsed.additions {
        motn_cache::upsert(pool, show_id, ct).await?;
    }
    for show_id in &parsed.removals {
        motn_cache::delete(pool, show_id).await?;
    }
    Ok(())
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
        let (titles, _) = parse_page(json, "gb", &[Service::Disney, Service::Crunchyroll]).unwrap();
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

    #[test]
    fn parse_page_entries_pairs_show_id_with_title() {
        let json = r#"{
            "shows": [
                {"id":"100","imdbId":"tt1","title":"A","showType":"movie","releaseYear":2020,
                 "streamingOptions":{"gb":[{"service":{"id":"disney"}}]}}
            ],
            "hasMore": false
        }"#;
        let (entries, cursor) = parse_page_entries(json, "gb", &[Service::Disney]).unwrap();
        assert_eq!(cursor, None);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "100");
        assert_eq!(entries[0].1.title, "A");
        assert_eq!(entries[0].1.services, vec![Service::Disney]);
    }

    #[test]
    fn parse_changes_collects_new_and_removed() {
        let json = r#"{
            "changes": [
                {"changeType":"new","itemType":"show","showId":"100"},
                {"changeType":"removed","itemType":"show","showId":"200"}
            ],
            "shows": {
                "100": {"id":"100","imdbId":"tt1","title":"A","showType":"movie",
                        "streamingOptions":{"gb":[{"service":{"id":"disney"}}]}}
            },
            "hasMore": false
        }"#;
        let (parsed, cursor) = parse_changes(json, "gb", &[Service::Disney]).unwrap();
        assert_eq!(cursor, None);
        assert_eq!(parsed.additions.len(), 1);
        assert_eq!(parsed.additions[0].0, "100");
        assert_eq!(parsed.removals, vec!["200".to_string()]);
    }

    #[test]
    fn parse_changes_skips_new_without_show_detail_or_imdb() {
        // "new" 300 has no entry in `shows`; "new" 400 has detail but no imdbId.
        let json = r#"{
            "changes": [
                {"changeType":"new","itemType":"show","showId":"300"},
                {"changeType":"new","itemType":"show","showId":"400"}
            ],
            "shows": {
                "400": {"id":"400","title":"NoImdb","showType":"movie",
                        "streamingOptions":{"gb":[{"service":{"id":"disney"}}]}}
            },
            "hasMore": false
        }"#;
        let (parsed, _) = parse_changes(json, "gb", &[Service::Disney]).unwrap();
        assert!(parsed.additions.is_empty());
        assert!(parsed.removals.is_empty());
    }

    #[test]
    fn parse_changes_propagates_cursor() {
        let json = r#"{"changes":[],"shows":{},"hasMore":true,"nextCursor":"abc"}"#;
        let (_, cursor) = parse_changes(json, "gb", &[Service::Disney]).unwrap();
        assert_eq!(cursor, Some("abc".to_string()));
    }

    use crate::db::{init_pool, motn_cache, sync_runs};

    async fn test_pool() -> (tempfile::TempDir, sqlx::SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let url = format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        (dir, pool)
    }

    fn cached(title: &str) -> CachedTitle {
        CachedTitle {
            imdb_id: Some(format!("tt-{title}")),
            tmdb_id: None,
            title: title.to_string(),
            year: None,
            kind: "movie".into(),
            imdb_rating: None,
            length: None,
            description: None,
            genres: vec![],
            cast: vec![],
            services: vec!["disney".into()],
        }
    }

    #[tokio::test]
    async fn decide_mode_seeds_on_empty_cache() {
        let (_dir, pool) = test_pool().await;
        assert!(matches!(decide_mode(&pool).await.unwrap(), SyncMode::Seed));
    }

    #[tokio::test]
    async fn decide_mode_seeds_when_no_recent_ok_run() {
        let (_dir, pool) = test_pool().await;
        motn_cache::upsert(&pool, "1", &cached("A")).await.unwrap();
        // Cache non-empty but no ok run within 25 days -> Seed (recovery re-seed).
        assert!(matches!(decide_mode(&pool).await.unwrap(), SyncMode::Seed));
    }

    #[tokio::test]
    async fn decide_mode_deltas_when_cache_and_recent_ok() {
        let (_dir, pool) = test_pool().await;
        motn_cache::upsert(&pool, "1", &cached("A")).await.unwrap();
        sync_runs::record(&pool, "disney", "ok", 1, None)
            .await
            .unwrap();
        match decide_mode(&pool).await.unwrap() {
            SyncMode::Delta { from } => assert!(from > 0),
            SyncMode::Seed => panic!("expected delta"),
        }
    }

    #[tokio::test]
    async fn apply_changes_adds_and_removes() {
        let (_dir, pool) = test_pool().await;
        motn_cache::upsert(&pool, "keep", &cached("Keep"))
            .await
            .unwrap();
        motn_cache::upsert(&pool, "drop", &cached("Drop"))
            .await
            .unwrap();
        let parsed = ParsedChanges {
            additions: vec![("new1".to_string(), cached("New"))],
            removals: vec!["drop".to_string()],
        };
        apply_changes(&pool, &parsed).await.unwrap();
        let titles: Vec<String> = motn_cache::load_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect();
        assert!(titles.contains(&"Keep".to_string()));
        assert!(titles.contains(&"New".to_string()));
        assert!(!titles.contains(&"Drop".to_string()));
    }
}
