//! Catalogue sync subsystem: external clients behind `CatalogueSource`,
//! pure merge logic, DB reconciliation, and the run orchestrator.

use async_trait::async_trait;

use crate::models::{Service, TitleKind};

pub mod merge;

/// A source-agnostic catalogue row emitted by every `CatalogueSource`.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedTitle {
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub plex_guid: Option<String>,
    pub title: String,
    pub year: Option<i64>,
    pub kind: TitleKind,
    pub imdb_rating: Option<f64>,
    pub length: Option<String>,
    pub description: Option<String>,
    pub genres: Vec<String>,
    pub cast: Vec<String>,
    pub services: Vec<Service>,
}

/// An external catalogue client. A *client* is the unit of fetching and failure;
/// the *services* it owns are the unit of membership reconciliation.
#[async_trait]
pub trait CatalogueSource: Send + Sync {
    /// Client name, used for logging/orchestration (e.g. "plex", "motn").
    fn name(&self) -> &'static str;
    /// The `title_services` membership(s) this client owns
    /// (Plex → `[Plex]`; MOTN → `[Disney, Crunchyroll]`).
    fn services(&self) -> &'static [Service];
    /// Fetch the client's current catalogue.
    ///
    /// # Errors
    /// Returns an error if the upstream request fails or a body cannot be parsed.
    async fn fetch(&self) -> anyhow::Result<Vec<FetchedTitle>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetched_title_constructs() {
        let t = FetchedTitle {
            imdb_id: Some("tt1".into()),
            tmdb_id: None,
            plex_guid: None,
            title: "X".into(),
            year: Some(2020),
            kind: TitleKind::Movie,
            imdb_rating: None,
            length: None,
            description: None,
            genres: vec![],
            cast: vec![],
            services: vec![Service::Plex],
        };
        assert_eq!(t.services, vec![Service::Plex]);
    }
}
