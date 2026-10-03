//! Movie-of-the-Night (Streaming Availability) client for UK Disney+/Crunchyroll.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::db::motn_cache::CachedTitle;
use crate::db::{app_meta, motn_cache, motn_meta, sync_runs};
use crate::models::{Service, TitleKind};
use crate::sync::{CatalogueSource, FetchedTitle, ImageRef};

const MOTN_BASE: &str = "https://api.movieofthenight.com/v4";

/// The services we sync and their API catalog-id candidates.
const WANTED: [(Service, &str); 2] = [
    (Service::Disney, "disney"),
    (Service::Crunchyroll, "crunchyroll"),
];

/// From a `/v4/countries` body, return `(Service, catalog_id)` for the wanted
/// services that the given country actually lists.
///
/// Absent services are skipped; an unparseable body resolves to none (with a
/// warning). See [`parse_services`] for the variant that reports the failure.
#[must_use]
pub fn resolve_services(countries_json: &str, country: &str) -> Vec<(Service, String)> {
    parse_services(countries_json, country).unwrap_or_else(|e| {
        tracing::warn!("{e:#}");
        Vec::new()
    })
}

/// Like [`resolve_services`], but an unparseable body is an error rather than
/// an empty list. A country missing from the body resolves to `Ok(vec![])`.
///
/// # Errors
/// Returns an error if the body is not the expected `/v4/countries` shape.
pub fn parse_services(
    countries_json: &str,
    country: &str,
) -> anyhow::Result<Vec<(Service, String)>> {
    let parsed: HashMap<String, CountryEntry> = serde_json::from_str(countries_json)
        .map_err(|e| anyhow::anyhow!("MOTN /countries response did not parse: {e}"))?;
    let Some(entry) = parsed.get(country) else {
        return Ok(Vec::new());
    };
    let available: HashSet<&str> = entry.services.iter().map(|s| s.id.as_str()).collect();
    Ok(WANTED
        .iter()
        .filter(|(_, id)| available.contains(id))
        .map(|(svc, id)| (*svc, (*id).to_string()))
        .collect())
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
    /// The service catalog this change applies to (changes are per service).
    #[serde(default)]
    service: Option<ServiceRef>,
}

#[derive(Deserialize, Clone)]
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
    #[serde(default)]
    image_set: Option<ImageSet>,
}

#[derive(Deserialize, Clone)]
struct StreamOption {
    service: ServiceRef,
    #[serde(default)]
    link: Option<String>,
}

#[derive(Deserialize, Clone)]
struct ServiceRef {
    id: String,
}

#[derive(Deserialize, Clone)]
struct Named {
    name: String,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct ImageSet {
    #[serde(default)]
    vertical_poster: HashMap<String, String>,
    #[serde(default)]
    horizontal_poster: HashMap<String, String>,
}

/// Documented MOTN vertical poster sizes, most-preferred first.
const POSTER_SIZES: &[&str] = &["w480", "w360", "w600", "w240", "w720"];
/// Horizontal sizes for the backdrop, most-preferred first.
const BACKDROP_SIZES: &[&str] = &["w1080", "w720", "w600", "w480", "w360", "w240"];

/// Pick the first available size from `prefer`, as a remote `ImageRef`.
fn pick_size(m: &HashMap<String, String>, prefer: &[&str]) -> Option<ImageRef> {
    prefer.iter().find_map(|k| m.get(*k)).map(|value| ImageRef {
        value: value.clone(),
        remote: true,
    })
}

/// The wanted services `s` currently lists in `streamingOptions[country]`, with
/// no fallback (empty when it is on none of them).
fn attributed_services(s: &Show, country: &str, services: &[Service]) -> Vec<Service> {
    let available: HashSet<&str> = s
        .streaming_options
        .get(country)
        .map(|opts| opts.iter().map(|o| o.service.id.as_str()).collect())
        .unwrap_or_default();
    services
        .iter()
        .copied()
        .filter(|svc| wanted_id(*svc).is_some_and(|id| available.contains(id)))
        .collect()
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
        let attributed = attributed_services(&s, country, services);
        if attributed.is_empty() {
            services.to_vec()
        } else {
            attributed
        }
    };
    let links: Vec<(Service, String)> = s
        .streaming_options
        .get(country)
        .map(|opts| {
            services
                .iter()
                .filter_map(|svc| {
                    let id = wanted_id(*svc)?;
                    let link = opts
                        .iter()
                        .find(|o| o.service.id == id)
                        .and_then(|o| o.link.clone())?;
                    Some((*svc, link))
                })
                .collect()
        })
        .unwrap_or_default();
    let poster = s
        .image_set
        .as_ref()
        .and_then(|set| pick_size(&set.vertical_poster, POSTER_SIZES));
    let backdrop = s
        .image_set
        .as_ref()
        .and_then(|set| pick_size(&set.horizontal_poster, BACKDROP_SIZES));
    FetchedTitle {
        imdb_id: s.imdb_id,
        tmdb_id: s.tmdb_id,
        plex_guid: None,
        title: s.title,
        year,
        kind,
        score: s.rating.map(|r| r / 10.0),
        length,
        description: s.overview,
        genres: s.genres.into_iter().map(|g| g.name).collect(),
        cast: s.cast,
        services: svcs,
        plex_rating_key: None,
        links,
        poster,
        backdrop,
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
    /// `(show_id, service)`: a `removed` change with no embedded detail — drop
    /// just that service from the cached show (delete it if none remain).
    pub service_removals: Vec<(String, Service)>,
}

impl ParsedChanges {
    fn merge(&mut self, mut other: Self) {
        self.additions.append(&mut other.additions);
        self.removals.append(&mut other.removals);
        self.service_removals.append(&mut other.service_removals);
    }
}

/// Parse one `/changes` page into `(ParsedChanges, next_cursor)`.
///
/// Changes are per service, and the embedded `shows` detail is the show's
/// *current* state. So every `new`/`removed` change refreshes the show from that
/// detail: still on a wanted service → cache upsert (attributed to the services
/// it is on now); on none → removal. This keeps a show that left one service but
/// remains on the other, and drops one added then removed within the window.
/// Without detail, `new` is skipped with a warning and `removed` drops only the
/// change's service (or the whole show if the service is unknown). A show with
/// no `imdbId` is not cached by a `new`; a `removed` falls back to dropping the
/// service. Non-show item types are ignored.
///
/// # Errors
/// Returns an error only if the page JSON itself does not parse.
pub fn parse_changes(
    json: &str,
    country: &str,
    services: &[Service],
) -> anyhow::Result<(ParsedChanges, Option<String>)> {
    let page: ChangesPage = serde_json::from_str(json)?;
    let mut out = ParsedChanges::default();
    for ch in &page.changes {
        if ch.item_type.as_deref().is_some_and(|t| t != "show") {
            continue;
        }
        let removed = match ch.change_type.as_str() {
            "new" => false,
            "removed" => true,
            _ => continue,
        };
        let id = ch.show_id.clone();
        let drop_service =
            |out: &mut ParsedChanges| match ch.service.as_ref().and_then(|s| service_for_id(&s.id))
            {
                Some(svc) => out.service_removals.push((id.clone(), svc)),
                None => out.removals.push(id.clone()),
            };
        let Some(show) = page.shows.get(&ch.show_id) else {
            if removed {
                drop_service(&mut out);
            } else {
                tracing::warn!("MOTN /changes 'new' {} missing show detail", ch.show_id);
            }
            continue;
        };
        if attributed_services(show, country, services).is_empty() {
            out.removals.push(id);
            continue;
        }
        let ft = show_to_fetched(show.clone(), country, services);
        if ft.imdb_id.is_none() {
            if removed {
                drop_service(&mut out);
            } else {
                tracing::warn!("MOTN /changes 'new' {} has no imdbId; skipping", ch.show_id);
            }
            continue;
        }
        out.additions.push((id, CachedTitle::from(&ft)));
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
const CHANGES_OVERLAP_SECS: i64 = 6 * 3600;

/// Which fetch strategy this run uses.
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
async fn decide_mode(pool: &SqlitePool) -> anyhow::Result<SyncMode> {
    if motn_cache::count(pool).await? == 0 || !sync_runs::motn_recent_ok(pool).await? {
        return Ok(SyncMode::Seed);
    }
    let last_ok = sync_runs::last_ok_unix(pool).await?.unwrap_or(0);
    Ok(SyncMode::Delta {
        from: last_ok.saturating_sub(CHANGES_OVERLAP_SECS).max(0),
    })
}

/// Apply parsed `/changes` to the cache: upsert additions, delete removals, and
/// drop single services from cached shows (deleting any left with none). A
/// service removal for a show also in this run's additions is skipped: the
/// addition's embedded detail is the show's current state.
///
/// # Errors
/// Returns an error if any cache write fails.
async fn apply_changes(pool: &SqlitePool, parsed: &ParsedChanges) -> anyhow::Result<()> {
    for (show_id, ct) in &parsed.additions {
        motn_cache::upsert(pool, show_id, ct).await?;
    }
    for show_id in &parsed.removals {
        motn_cache::delete(pool, show_id).await?;
    }
    for (show_id, svc) in &parsed.service_removals {
        // An addition in this run carries the show's current state: it wins.
        if parsed.additions.iter().any(|(id, _)| id == show_id) {
            continue;
        }
        let Some(mut ct) = motn_cache::get(pool, show_id).await? else {
            continue;
        };
        ct.services.retain(|s| s != svc.as_str());
        ct.links.retain(|(s, _)| s != svc.as_str());
        if ct.services.is_empty() {
            motn_cache::delete(pool, show_id).await?;
        } else {
            motn_cache::upsert(pool, show_id, &ct).await?;
        }
    }
    Ok(())
}

/// The wanted service for a MOTN catalog id (`"disney"` → `Disney`, etc.).
fn service_for_id(id: &str) -> Option<Service> {
    WANTED.iter().find(|(_, w)| *w == id).map(|(s, _)| *s)
}

/// The MOTN catalog id for a wanted service (`Disney` → "disney", etc.).
fn wanted_id(svc: Service) -> Option<&'static str> {
    WANTED.iter().find(|(s, _)| *s == svc).map(|(_, id)| *id)
}

/// A failed seed blocks further seed attempts (no network call) for this long.
pub const SEED_BACKOFF_SECS: i64 = 3 * 86_400;
/// A cached `/countries` resolution is reused while younger than this.
pub const CATALOGS_MAX_AGE_SECS: i64 = 7 * 86_400;

/// MOTN answered HTTP 429: the run is aborted and remaining calls are skipped.
/// Returned inside an `anyhow::Error` (use `downcast_ref::<RateLimited>()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimited;

impl std::fmt::Display for RateLimited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MOTN rate limit (429)")
    }
}

impl std::error::Error for RateLimited {}

/// Live MOTN client.
pub struct MotnClient {
    client: reqwest::Client,
    api_key: String,
    country: String,
    pool: SqlitePool,
    /// API root (`MOTN_BASE` in production; a local fake server in tests).
    base: String,
}

impl MotnClient {
    #[must_use]
    pub fn new(api_key: String, country: String, pool: SqlitePool) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            country,
            pool,
            base: MOTN_BASE.to_string(),
        }
    }

    /// Test client against a local fake server at `base` (bypassing any proxy).
    ///
    /// # Panics
    /// Panics if the reqwest client cannot be built.
    #[cfg(test)]
    #[must_use]
    pub fn with_base(api_key: String, country: String, pool: SqlitePool, base: String) -> Self {
        Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("build test reqwest client"),
            api_key,
            country,
            pool,
            base,
        }
    }

    /// Send one GET to `{base}{path}`, counting it against the month's MOTN
    /// requests **before** sending. HTTP 429 → [`RateLimited`]; any other error
    /// status is an error.
    ///
    /// # Errors
    /// Returns an error if counting, the request, the status, or reading fails.
    async fn get_text(&self, path: &str, query: &[(&str, &str)]) -> anyhow::Result<String> {
        motn_meta::increment_requests(&self.pool).await?;
        let resp = self
            .client
            .get(format!("{}{path}", self.base))
            .header("X-API-Key", &self.api_key)
            .query(query)
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let n = motn_meta::requests_this_month(&self.pool)
                .await
                .unwrap_or(-1);
            tracing::warn!(
                "MOTN rate limit (429) on {path}; aborting run ({n} requests this month)"
            );
            return Err(RateLimited.into());
        }
        Ok(resp.error_for_status()?.text().await?)
    }

    /// The usable cached catalog resolution for this country, if any:
    /// `(catalogs_csv, services, checked_at)`. Unknown ids are dropped; a value
    /// with no known id counts as no cache.
    ///
    /// # Errors
    /// Returns an error if a database read fails.
    async fn cached_catalogs(&self) -> anyhow::Result<Option<(String, Vec<Service>, i64)>> {
        let Some(csv) = app_meta::get(&self.pool, &motn_meta::catalogs_key(&self.country)).await?
        else {
            return Ok(None);
        };
        let resolved: Vec<(Service, &str)> = csv
            .split(',')
            .map(str::trim)
            .filter_map(|id| service_for_id(id).map(|s| (s, id)))
            .collect();
        if resolved.is_empty() {
            return Ok(None);
        }
        let checked =
            motn_meta::get_i64(&self.pool, &motn_meta::catalogs_checked_key(&self.country))
                .await?
                .unwrap_or(0);
        let catalogs = resolved
            .iter()
            .map(|(_, id)| *id)
            .collect::<Vec<_>>()
            .join(",");
        let services = resolved.iter().map(|(s, _)| *s).collect();
        Ok(Some((catalogs, services, checked)))
    }

    /// Call `/countries` and resolve the wanted services for this country.
    ///
    /// # Errors
    /// Returns an error if the request fails, the body does not parse, or the
    /// country lists none of the wanted services.
    async fn countries_catalogs(&self) -> anyhow::Result<(String, Vec<Service>)> {
        let body = self.get_text("/countries", &[]).await?;
        let resolved = parse_services(&body, &self.country)?;
        if resolved.is_empty() {
            anyhow::bail!(
                "MOTN lists none of [disney, crunchyroll] for {}",
                self.country
            );
        }
        let catalogs = resolved
            .iter()
            .map(|(_, id)| id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let services = resolved.iter().map(|(s, _)| *s).collect();
        Ok((catalogs, services))
    }

    /// Resolve `(catalogs_csv, services)` — never empty. Reuses the per-country
    /// cache while it is under `CATALOGS_MAX_AGE_SECS` old and lists every wanted
    /// service (unless `force`, as on a full seed); otherwise calls `/countries`
    /// and caches the (non-empty) result. If `/countries` fails — HTTP error,
    /// unparseable body, or none of the wanted services listed — a cached value
    /// is used with a warning. A 429 is never masked by the cache: it aborts the
    /// run.
    ///
    /// # Errors
    /// Returns an error if `/countries` fails and no cached value exists, on a
    /// 429, or if a database read/write fails.
    async fn resolve_catalogs(&self, force: bool) -> anyhow::Result<(String, Vec<Service>)> {
        let cached = self.cached_catalogs().await?;
        if !force {
            if let Some((catalogs, services, checked)) = &cached {
                // A cached set lacking a wanted service is never fresh: re-check
                // /countries every run so a shrunken result can't stick for 7 days.
                let complete = WANTED.iter().all(|(w, _)| services.contains(w));
                if complete
                    && motn_meta::now_unix().saturating_sub(*checked) < CATALOGS_MAX_AGE_SECS
                {
                    return Ok((catalogs.clone(), services.clone()));
                }
            }
        }
        match self.countries_catalogs().await {
            Ok((catalogs, services)) => {
                app_meta::set(
                    &self.pool,
                    &motn_meta::catalogs_key(&self.country),
                    &catalogs,
                )
                .await?;
                motn_meta::set_i64(
                    &self.pool,
                    &motn_meta::catalogs_checked_key(&self.country),
                    motn_meta::now_unix(),
                )
                .await?;
                Ok((catalogs, services))
            }
            Err(e) if e.is::<RateLimited>() => Err(e),
            Err(e) => match cached {
                Some((catalogs, services, _)) => {
                    tracing::warn!(
                        "MOTN /countries failed ({e:#}); using cached catalogs {catalogs}"
                    );
                    Ok((catalogs, services))
                }
                None => Err(e),
            },
        }
    }

    /// One full seed attempt: force-resolve catalogs, paginate the search, and
    /// replace the cache. Zero shows is an error and leaves the cache untouched.
    ///
    /// # Errors
    /// Returns `(error, spent)` if any step fails or the seed returns no shows.
    /// `spent` is true once the first seed page request has been made (a failure
    /// in catalog resolution before that spends no seed requests).
    async fn seed(&self) -> Result<(), (anyhow::Error, bool)> {
        let (catalogs, services) = self.resolve_catalogs(true).await.map_err(|e| (e, false))?;
        let seed_pages = async {
            let entries = self.seed_pages(&catalogs, &services).await?;
            if entries.is_empty() {
                anyhow::bail!("MOTN seed returned no shows");
            }
            tracing::info!("MOTN full seed: {} shows", entries.len());
            motn_cache::replace_all(&self.pool, &entries).await
        };
        seed_pages.await.map_err(|e| (e, true))
    }

    /// Full pagination of `/shows/search/filters` → `(show_id, CachedTitle)` entries.
    ///
    /// # Errors
    /// Returns an error if any HTTP request fails or any page cannot be parsed.
    async fn seed_pages(
        &self,
        catalogs: &str,
        services: &[Service],
    ) -> anyhow::Result<Vec<(String, CachedTitle)>> {
        let mut out: Vec<(String, CachedTitle)> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut query = vec![("country", self.country.as_str()), ("catalogs", catalogs)];
            if let Some(c) = &cursor {
                query.push(("cursor", c.as_str()));
            }
            let body = self.get_text("/shows/search/filters", &query).await?;
            let (entries, next) = parse_page_entries(&body, &self.country, services)?;
            for (id, ft) in entries {
                out.push((id, CachedTitle::from(&ft)));
            }
            match next {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        Ok(out)
    }

    /// Paginate `/changes` for one `change_type` since `from` (Unix seconds).
    ///
    /// # Errors
    /// Returns an error if any HTTP request fails or any page cannot be parsed.
    async fn changes_pages(
        &self,
        catalogs: &str,
        services: &[Service],
        change_type: &str,
        from: i64,
    ) -> anyhow::Result<ParsedChanges> {
        let mut out = ParsedChanges::default();
        let from_str = from.to_string();
        let mut cursor: Option<String> = None;
        loop {
            let mut query = vec![
                ("country", self.country.as_str()),
                ("catalogs", catalogs),
                ("item_type", "show"),
                ("change_type", change_type),
                ("from", from_str.as_str()),
            ];
            if let Some(c) = &cursor {
                query.push(("cursor", c.as_str()));
            }
            let body = self.get_text("/changes", &query).await?;
            let (parsed, next) = parse_changes(&body, &self.country, services)?;
            out.merge(parsed);
            match next {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        Ok(out)
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

    /// Seed or delta, then return the whole cache. Never `Ok(vec![])` from
    /// catalog resolution or a zero-show seed: those are errors, so `run_sync`
    /// scopes the failure instead of reconciling the services to nothing.
    async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>> {
        match decide_mode(&self.pool).await? {
            SyncMode::Seed => {
                let now = motn_meta::now_unix();
                if let Some(failed) =
                    motn_meta::get_i64(&self.pool, motn_meta::SEED_FAILED_AT).await?
                {
                    let until = failed.saturating_add(SEED_BACKOFF_SECS);
                    if failed > 0 && now < until {
                        anyhow::bail!("MOTN seed back-off until {until}");
                    }
                }
                if let Err((e, spent)) = self.seed().await {
                    // Back off only when the attempt spent seed requests or hit a
                    // 429; a catalog-resolution failure costs ≤1 request per run.
                    if spent || e.is::<RateLimited>() {
                        if let Err(we) = motn_meta::set_i64(
                            &self.pool,
                            motn_meta::SEED_FAILED_AT,
                            motn_meta::now_unix(),
                        )
                        .await
                        {
                            tracing::error!("recording MOTN seed failure failed: {we:#}");
                        }
                    }
                    return Err(e);
                }
                motn_meta::set_i64(&self.pool, motn_meta::LAST_SEED_AT, motn_meta::now_unix())
                    .await?;
                app_meta::set(&self.pool, motn_meta::LAST_MODE, "seed").await?;
                motn_meta::set_i64(&self.pool, motn_meta::SEED_FAILED_AT, 0).await?;
            }
            SyncMode::Delta { from } => {
                let (catalogs, services) = self.resolve_catalogs(false).await?;
                let mut parsed = ParsedChanges::default();
                for change_type in ["new", "removed"] {
                    let page = self
                        .changes_pages(&catalogs, &services, change_type, from)
                        .await?;
                    parsed.merge(page);
                }
                tracing::info!(
                    "MOTN delta: +{} -{} -{} service",
                    parsed.additions.len(),
                    parsed.removals.len(),
                    parsed.service_removals.len()
                );
                apply_changes(&self.pool, &parsed).await?;
                let all = motn_cache::load_all(&self.pool).await?;
                if all.is_empty() {
                    // Never Ok(empty): run_sync would reconcile both services to
                    // nothing. The next run seeds (decide_mode sees an empty cache).
                    anyhow::bail!("MOTN delta emptied the cache");
                }
                app_meta::set(&self.pool, motn_meta::LAST_MODE, "delta").await?;
                return Ok(all);
            }
        }

        motn_cache::load_all(&self.pool).await
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
        assert!((t.score.unwrap() - 8.2).abs() < 1e-9, "rating 82 -> 8.2");
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
    fn parses_streaming_link_per_service() {
        let json = include_str!("../../tests/fixtures/motn_search_page1.json");
        let (titles, _) = parse_page(json, "gb", &[Service::Crunchyroll, Service::Disney]).unwrap();
        let with_link = titles
            .iter()
            .find(|t| t.links.iter().any(|(s, _)| *s == Service::Crunchyroll));
        assert!(
            with_link.is_some(),
            "a crunchyroll link should be attributed"
        );
        let (_, url) = with_link
            .unwrap()
            .links
            .iter()
            .find(|(s, _)| *s == Service::Crunchyroll)
            .unwrap();
        assert!(url.contains("crunchyroll.com"));
    }

    #[test]
    fn parse_page_falls_back_when_streaming_options_absent() {
        // No streamingOptions at all -> never drop the title; keep the searched set.
        let json = r#"{ "shows": [ { "title": "X", "showType": "movie" } ], "hasMore": false }"#;
        let (titles, _) = parse_page(json, "gb", &[Service::Disney]).unwrap();
        assert_eq!(titles[0].services, vec![Service::Disney]);
    }

    #[test]
    fn parse_page_extracts_image_set() {
        let json = r#"{
          "shows": [{
            "id": "1", "imdbId": "tt1", "title": "X", "showType": "movie",
            "releaseYear": 2020,
            "imageSet": {
              "verticalPoster": { "w240": "https://cdn/v240.jpg", "w480": "https://cdn/v480.jpg" },
              "horizontalPoster": { "w1080": "https://cdn/h1080.jpg" }
            },
            "streamingOptions": {}
          }],
          "hasMore": false
        }"#;
        let (titles, _) = parse_page(json, "gb", &[Service::Disney]).unwrap();
        let t = &titles[0];
        let p = t.poster.as_ref().unwrap();
        assert!(p.remote);
        assert_eq!(p.value, "https://cdn/v480.jpg"); // w480 preferred
        let b = t.backdrop.as_ref().unwrap();
        assert!(b.remote);
        assert_eq!(b.value, "https://cdn/h1080.jpg");
    }

    #[test]
    fn parse_page_handles_missing_image_set() {
        let json = r#"{"shows":[{"id":"1","imdbId":"tt1","title":"X","showType":"movie","releaseYear":2020,"streamingOptions":{}}],"hasMore":false}"#;
        let (titles, _) = parse_page(json, "gb", &[Service::Disney]).unwrap();
        assert!(titles[0].poster.is_none());
        assert!(titles[0].backdrop.is_none());
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
        assert_eq!(parsed.removals, Vec::<String>::new());
    }

    #[test]
    fn removed_from_one_service_keeps_show_on_the_other() {
        // Show 500 left Disney but its current detail still lists Crunchyroll:
        // refresh it (Crunchyroll only), don't delete it.
        let json = r#"{
            "changes": [
                {"changeType":"removed","itemType":"show","showId":"500","service":{"id":"disney"}}
            ],
            "shows": {
                "500": {"id":"500","imdbId":"tt5","title":"Both","showType":"series",
                        "streamingOptions":{"gb":[{"service":{"id":"crunchyroll"},
                                                   "link":"https://www.crunchyroll.com/x"}]}}
            },
            "hasMore": false
        }"#;
        let both = [Service::Disney, Service::Crunchyroll];
        let (parsed, _) = parse_changes(json, "gb", &both).unwrap();
        assert!(parsed.removals.is_empty(), "must not delete the whole show");
        assert_eq!(parsed.additions.len(), 1);
        assert_eq!(parsed.additions[0].0, "500");
        assert_eq!(
            parsed.additions[0].1.services,
            vec!["crunchyroll".to_string()]
        );
    }

    #[test]
    fn new_whose_current_detail_lists_no_wanted_service_is_a_removal() {
        // Added then removed inside the window: the embedded detail is current
        // state, so it must not be cached under the "both services" fallback.
        let json = r#"{
            "changes": [
                {"changeType":"new","itemType":"show","showId":"600","service":{"id":"disney"}}
            ],
            "shows": {
                "600": {"id":"600","imdbId":"tt6","title":"Gone","showType":"movie",
                        "streamingOptions":{"gb":[{"service":{"id":"netflix"}}]}}
            },
            "hasMore": false
        }"#;
        let (parsed, _) = parse_changes(json, "gb", &[Service::Disney]).unwrap();
        assert!(parsed.additions.is_empty());
        assert_eq!(parsed.removals, vec!["600".to_string()]);
    }

    #[test]
    fn removed_without_detail_drops_only_that_service() {
        let json = r#"{
            "changes": [
                {"changeType":"removed","itemType":"show","showId":"700","service":{"id":"crunchyroll"}}
            ],
            "shows": {},
            "hasMore": false
        }"#;
        let (parsed, _) =
            parse_changes(json, "gb", &[Service::Disney, Service::Crunchyroll]).unwrap();
        assert_eq!(parsed.removals, Vec::<String>::new());
        assert_eq!(
            parsed.service_removals,
            vec![("700".to_string(), Service::Crunchyroll)]
        );
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
            score: None,
            length: None,
            description: None,
            genres: vec![],
            cast: vec![],
            services: vec!["disney".into()],
            links: vec![],
            poster_url: None,
            backdrop_url: None,
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
            service_removals: vec![],
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

    #[tokio::test]
    async fn apply_service_removal_keeps_other_service_and_deletes_when_empty() {
        let (_dir, pool) = test_pool().await;
        let mut both = cached("Both");
        both.services = vec!["disney".into(), "crunchyroll".into()];
        both.links = vec![
            ("disney".into(), "https://www.disneyplus.com/x".into()),
            ("crunchyroll".into(), "https://www.crunchyroll.com/x".into()),
        ];
        motn_cache::upsert(&pool, "both", &both).await.unwrap();
        motn_cache::upsert(&pool, "solo", &cached("Solo"))
            .await
            .unwrap();
        let parsed = ParsedChanges {
            additions: vec![],
            removals: vec![],
            service_removals: vec![
                ("both".to_string(), Service::Disney),
                ("solo".to_string(), Service::Disney),
                ("absent".to_string(), Service::Disney),
            ],
        };
        apply_changes(&pool, &parsed).await.unwrap();
        let titles = motn_cache::load_all(&pool).await.unwrap();
        assert_eq!(titles.len(), 1, "solo lost its only service and is deleted");
        assert_eq!(titles[0].title, "Both");
        assert_eq!(titles[0].services, vec![Service::Crunchyroll]);
        assert_eq!(titles[0].links.len(), 1);
        assert_eq!(titles[0].links[0].0, Service::Crunchyroll);
    }

    #[tokio::test]
    async fn service_removal_skipped_when_same_run_adds_the_show() {
        // A detail-less `removed` (Disney) and a `new` with detail saying the show
        // is on Disney in the same run: the addition is current state, keep Disney.
        let (_dir, pool) = test_pool().await;
        motn_cache::upsert(&pool, "x", &cached("X")).await.unwrap();
        let parsed = ParsedChanges {
            additions: vec![("x".to_string(), cached("X"))],
            removals: vec![],
            service_removals: vec![("x".to_string(), Service::Disney)],
        };
        apply_changes(&pool, &parsed).await.unwrap();
        let titles = motn_cache::load_all(&pool).await.unwrap();
        assert_eq!(titles.len(), 1, "show must survive");
        assert_eq!(titles[0].services, vec![Service::Disney]);
    }
}

/// `MotnClient::fetch` against a local fake MOTN server. Kept apart from `tests`
/// and without `use actix_web::test` (that import shadows `#[test]`).
#[cfg(test)]
mod fetch_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::db::motn_meta::{
        catalogs_checked_key, catalogs_key, get_i64, now_unix, requests_this_month, set_i64,
        LAST_MODE, LAST_SEED_AT, SEED_FAILED_AT,
    };
    use crate::db::{app_meta, init_pool, motn_cache, sync_runs};

    const COUNTRIES: &str = r#"{"gb":{"countryCode":"gb","services":[
        {"id":"netflix"},{"id":"disney"},{"id":"crunchyroll"}]}}"#;
    const COUNTRIES_NETFLIX_ONLY: &str = r#"{"gb":{"services":[{"id":"netflix"}]}}"#;
    const PAGE1: &str = r#"{"shows":[{"id":"1","imdbId":"tt1","title":"A","showType":"movie",
        "streamingOptions":{"gb":[{"service":{"id":"disney"}}]}}],
        "hasMore":true,"nextCursor":"c1"}"#;
    const PAGE2: &str = r#"{"shows":[{"id":"2","imdbId":"tt2","title":"B","showType":"series",
        "streamingOptions":{"gb":[{"service":{"id":"crunchyroll"}}]}}],"hasMore":false}"#;
    const EMPTY_PAGE: &str = r#"{"shows":[],"hasMore":false}"#;
    const NO_CHANGES: &str = r#"{"changes":[],"shows":{},"hasMore":false}"#;

    /// Serve every request through `handler(path_and_query)`, counting hits.
    #[allow(clippy::unused_async)] // async per the test-helper contract; callers `.await` it
    async fn fake_motn(handler: fn(&str) -> (u16, String)) -> (String, Arc<AtomicUsize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let server = actix_web::HttpServer::new(move || {
            let counter = counter.clone();
            actix_web::App::new().default_service(actix_web::web::to(
                move |req: actix_web::HttpRequest| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    let pq = req
                        .uri()
                        .path_and_query()
                        .map_or_else(|| req.path().to_string(), ToString::to_string);
                    let (status, body) = handler(&pq);
                    async move {
                        actix_web::HttpResponse::build(
                            actix_web::http::StatusCode::from_u16(status).unwrap(),
                        )
                        .content_type("application/json")
                        .body(body)
                    }
                },
            ))
        })
        .workers(1)
        .disable_signals()
        .shutdown_timeout(0)
        .listen(listener)
        .unwrap()
        .run();
        actix_web::rt::spawn(server);
        (format!("http://127.0.0.1:{port}"), hits)
    }

    /// Countries ok, a 2-page seed, empty change feeds.
    fn standard(p: &str) -> (u16, String) {
        if p.starts_with("/countries") {
            (200, COUNTRIES.into())
        } else if p.starts_with("/shows/search/filters") {
            if p.contains("cursor=") {
                (200, PAGE2.into())
            } else {
                (200, PAGE1.into())
            }
        } else if p.starts_with("/changes") {
            (200, NO_CHANGES.into())
        } else {
            (404, String::new())
        }
    }

    async fn test_pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let url = format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        (dir, pool)
    }

    fn client(pool: &SqlitePool, country: &str, base: String) -> MotnClient {
        MotnClient::with_base("k".into(), country.into(), pool.clone(), base)
    }

    fn cached(title: &str) -> CachedTitle {
        CachedTitle {
            imdb_id: Some(format!("tt-{title}")),
            tmdb_id: None,
            title: title.to_string(),
            year: None,
            kind: "movie".into(),
            score: None,
            length: None,
            description: None,
            genres: vec![],
            cast: vec![],
            services: vec!["disney".into()],
            links: vec![],
            poster_url: None,
            backdrop_url: None,
        }
    }

    /// Non-empty cache plus a recent ok run → `decide_mode` picks a delta.
    async fn make_delta_ready(pool: &SqlitePool) {
        motn_cache::upsert(pool, "old", &cached("Old"))
            .await
            .unwrap();
        sync_runs::record(pool, "disney", "ok", 1, None)
            .await
            .unwrap();
    }

    async fn cache_titles(pool: &SqlitePool) -> Vec<String> {
        motn_cache::load_all(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect()
    }

    #[actix_web::test]
    async fn every_request_is_counted() {
        let (_dir, pool) = test_pool().await;
        let (base, hits) = fake_motn(standard).await;
        let titles = client(&pool, "gb", base).fetch().await.unwrap();
        assert_eq!(titles.len(), 2);
        let n = hits.load(Ordering::SeqCst);
        assert_eq!(n, 3, "countries + 2 seed pages");
        assert_eq!(
            requests_this_month(&pool).await.unwrap(),
            i64::try_from(n).unwrap()
        );
    }

    #[actix_web::test]
    async fn rate_limited_seed_records_backoff() {
        fn handler(p: &str) -> (u16, String) {
            if p.starts_with("/shows/search/filters") && p.contains("cursor=") {
                (429, String::new())
            } else {
                standard(p)
            }
        }
        let (_dir, pool) = test_pool().await;
        let (base, _hits) = fake_motn(handler).await;
        let err = client(&pool, "gb", base).fetch().await.unwrap_err();
        assert!(err.downcast_ref::<RateLimited>().is_some(), "got {err:#}");
        assert!(get_i64(&pool, SEED_FAILED_AT).await.unwrap().unwrap() > 0);
    }

    #[actix_web::test]
    async fn seed_backoff_errors_without_network() {
        let (_dir, pool) = test_pool().await;
        set_i64(&pool, SEED_FAILED_AT, now_unix() - 3600)
            .await
            .unwrap();
        let (base, hits) = fake_motn(standard).await;
        let err = client(&pool, "gb", base).fetch().await.unwrap_err();
        assert!(format!("{err:#}").contains("seed back-off"), "got {err:#}");
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        assert_eq!(requests_this_month(&pool).await.unwrap(), 0);
    }

    #[actix_web::test]
    async fn seed_backoff_expires_after_three_days() {
        let (_dir, pool) = test_pool().await;
        set_i64(&pool, SEED_FAILED_AT, now_unix() - 3 * 86_400 - 1)
            .await
            .unwrap();
        let (base, hits) = fake_motn(standard).await;
        client(&pool, "gb", base).fetch().await.unwrap();
        assert!(hits.load(Ordering::SeqCst) > 0);
    }

    #[actix_web::test]
    async fn catalogs_reused_within_seven_days() {
        let (_dir, pool) = test_pool().await;
        make_delta_ready(&pool).await;
        app_meta::set(&pool, &catalogs_key("gb"), "disney,crunchyroll")
            .await
            .unwrap();
        set_i64(&pool, &catalogs_checked_key("gb"), now_unix() - 60)
            .await
            .unwrap();
        let (base, hits) = fake_motn(standard).await;
        client(&pool, "gb", base).fetch().await.unwrap();
        assert_eq!(
            hits.load(Ordering::SeqCst),
            2,
            "only /changes new + removed, never /countries"
        );
    }

    #[actix_web::test]
    async fn catalogs_re_resolved_after_seven_days() {
        let (_dir, pool) = test_pool().await;
        make_delta_ready(&pool).await;
        app_meta::set(&pool, &catalogs_key("gb"), "disney,crunchyroll")
            .await
            .unwrap();
        let stale = now_unix() - 7 * 86_400 - 1;
        set_i64(&pool, &catalogs_checked_key("gb"), stale)
            .await
            .unwrap();
        let (base, hits) = fake_motn(standard).await;
        client(&pool, "gb", base).fetch().await.unwrap();
        assert_eq!(
            hits.load(Ordering::SeqCst),
            3,
            "/countries once + 2 changes"
        );
        let checked = get_i64(&pool, &catalogs_checked_key("gb"))
            .await
            .unwrap()
            .unwrap();
        assert!(checked > stale, "checked_at refreshed");
    }

    #[actix_web::test]
    async fn cached_catalogs_used_when_countries_fails() {
        fn handler(p: &str) -> (u16, String) {
            if p.starts_with("/countries") {
                (500, String::new())
            } else {
                standard(p)
            }
        }
        let (_dir, pool) = test_pool().await;
        make_delta_ready(&pool).await;
        app_meta::set(&pool, &catalogs_key("gb"), "disney,crunchyroll")
            .await
            .unwrap();
        set_i64(&pool, &catalogs_checked_key("gb"), now_unix() - 30 * 86_400)
            .await
            .unwrap();
        let (base, hits) = fake_motn(handler).await;
        let titles = client(&pool, "gb", base).fetch().await.unwrap();
        assert_eq!(titles.len(), 1, "delta ran over the cached catalogue");
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[actix_web::test]
    async fn modes_recorded() {
        let (_dir, pool) = test_pool().await;
        let (base, _hits) = fake_motn(standard).await;
        let c = client(&pool, "gb", base);
        c.fetch().await.unwrap();
        assert_eq!(
            app_meta::get(&pool, LAST_MODE).await.unwrap().as_deref(),
            Some("seed")
        );
        assert!(get_i64(&pool, LAST_SEED_AT).await.unwrap().unwrap() > 0);
        assert_eq!(get_i64(&pool, SEED_FAILED_AT).await.unwrap(), Some(0));

        sync_runs::record(&pool, "disney", "ok", 2, None)
            .await
            .unwrap();
        c.fetch().await.unwrap();
        assert_eq!(
            app_meta::get(&pool, LAST_MODE).await.unwrap().as_deref(),
            Some("delta")
        );
    }

    #[actix_web::test]
    async fn unparseable_countries_fails_and_keeps_cache() {
        fn handler(p: &str) -> (u16, String) {
            if p.starts_with("/countries") {
                (200, r#"{"gb": 1}"#.into())
            } else {
                standard(p)
            }
        }
        let (_dir, pool) = test_pool().await;
        make_delta_ready(&pool).await;
        let (base, _hits) = fake_motn(handler).await;
        assert!(client(&pool, "gb", base).fetch().await.is_err());
        assert_eq!(cache_titles(&pool).await, vec!["Old".to_string()]);
    }

    #[actix_web::test]
    async fn countries_listing_none_is_an_error() {
        fn handler(p: &str) -> (u16, String) {
            if p.starts_with("/countries") {
                (200, COUNTRIES_NETFLIX_ONLY.into())
            } else {
                standard(p)
            }
        }
        let (_dir, pool) = test_pool().await;
        make_delta_ready(&pool).await;
        let (base, _hits) = fake_motn(handler).await;
        assert!(client(&pool, "gb", base).fetch().await.is_err());
        assert_eq!(
            app_meta::get(&pool, &catalogs_key("gb")).await.unwrap(),
            None
        );
    }

    #[actix_web::test]
    async fn empty_seed_is_a_failed_seed() {
        fn handler(p: &str) -> (u16, String) {
            if p.starts_with("/shows/search/filters") {
                (200, EMPTY_PAGE.into())
            } else {
                standard(p)
            }
        }
        let (_dir, pool) = test_pool().await;
        // Cache rows but no recent ok run → a recovery seed is due.
        motn_cache::upsert(&pool, "old", &cached("Old"))
            .await
            .unwrap();
        let (base, _hits) = fake_motn(handler).await;
        let err = client(&pool, "gb", base).fetch().await.unwrap_err();
        assert!(format!("{err:#}").contains("no shows"), "got {err:#}");
        assert!(get_i64(&pool, SEED_FAILED_AT).await.unwrap().unwrap() > 0);
        assert_eq!(cache_titles(&pool).await, vec!["Old".to_string()]);
    }

    #[actix_web::test]
    async fn catalogs_cache_is_per_country() {
        let (_dir, pool) = test_pool().await;
        make_delta_ready(&pool).await;
        app_meta::set(&pool, &catalogs_key("us"), "disney")
            .await
            .unwrap();
        set_i64(&pool, &catalogs_checked_key("us"), now_unix() - 60)
            .await
            .unwrap();
        let (base, hits) = fake_motn(standard).await;
        client(&pool, "gb", base).fetch().await.unwrap();
        assert_eq!(
            hits.load(Ordering::SeqCst),
            3,
            "gb still resolves /countries"
        );
        assert_eq!(
            app_meta::get(&pool, &catalogs_key("gb"))
                .await
                .unwrap()
                .as_deref(),
            Some("disney,crunchyroll")
        );
    }

    #[actix_web::test]
    async fn countries_failure_before_any_seed_page_does_not_back_off() {
        fn handler(p: &str) -> (u16, String) {
            if p.starts_with("/countries") {
                (500, String::new())
            } else {
                standard(p)
            }
        }
        let (_dir, pool) = test_pool().await;
        let (base, hits) = fake_motn(handler).await;
        assert!(client(&pool, "gb", base).fetch().await.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 1, "only /countries was tried");
        assert!(
            get_i64(&pool, SEED_FAILED_AT).await.unwrap().unwrap_or(0) == 0,
            "no seed page was requested, so no back-off"
        );
    }

    #[actix_web::test]
    async fn countries_429_on_a_seed_backs_off() {
        fn handler(p: &str) -> (u16, String) {
            if p.starts_with("/countries") {
                (429, String::new())
            } else {
                standard(p)
            }
        }
        let (_dir, pool) = test_pool().await;
        let (base, _hits) = fake_motn(handler).await;
        let err = client(&pool, "gb", base).fetch().await.unwrap_err();
        assert!(err.downcast_ref::<RateLimited>().is_some(), "got {err:#}");
        assert!(get_i64(&pool, SEED_FAILED_AT).await.unwrap().unwrap() > 0);
    }

    #[actix_web::test]
    async fn delta_that_empties_the_cache_is_an_error() {
        fn handler(p: &str) -> (u16, String) {
            if p.starts_with("/changes") && p.contains("change_type=removed") {
                (
                    200,
                    r#"{"changes":[{"changeType":"removed","itemType":"show","showId":"old"}],
                        "shows":{},"hasMore":false}"#
                        .into(),
                )
            } else {
                standard(p)
            }
        }
        let (_dir, pool) = test_pool().await;
        make_delta_ready(&pool).await;
        app_meta::set(&pool, &catalogs_key("gb"), "disney,crunchyroll")
            .await
            .unwrap();
        set_i64(&pool, &catalogs_checked_key("gb"), now_unix() - 60)
            .await
            .unwrap();
        let (base, _hits) = fake_motn(handler).await;
        let err = client(&pool, "gb", base).fetch().await.unwrap_err();
        assert!(
            format!("{err:#}").contains("emptied the cache"),
            "got {err:#}"
        );
    }

    #[actix_web::test]
    async fn cached_catalogs_missing_a_service_are_rechecked() {
        let (_dir, pool) = test_pool().await;
        make_delta_ready(&pool).await;
        app_meta::set(&pool, &catalogs_key("gb"), "disney")
            .await
            .unwrap();
        set_i64(&pool, &catalogs_checked_key("gb"), now_unix() - 3600)
            .await
            .unwrap();
        let (base, hits) = fake_motn(standard).await;
        client(&pool, "gb", base).fetch().await.unwrap();
        assert_eq!(
            hits.load(Ordering::SeqCst),
            3,
            "/countries re-checked + 2 changes"
        );
        assert_eq!(
            app_meta::get(&pool, &catalogs_key("gb"))
                .await
                .unwrap()
                .as_deref(),
            Some("disney,crunchyroll")
        );
    }
}
