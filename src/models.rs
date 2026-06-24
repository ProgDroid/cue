use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    Plex,
    Disney,
    Crunchyroll,
}

impl Service {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "plex" => Some(Self::Plex),
            "disney" => Some(Self::Disney),
            "crunchyroll" => Some(Self::Crunchyroll),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plex => "plex",
            Self::Disney => "disney",
            Self::Crunchyroll => "crunchyroll",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TitleKind {
    Movie,
    Series,
}

impl TitleKind {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "movie" => Some(Self::Movie),
            "series" => Some(Self::Series),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct TitleRow {
    pub id: i64,
    pub imdb_id: Option<String>,
    pub title: String,
    pub year: i64,
    #[sqlx(rename = "type")]
    pub kind: String,
    pub score: Option<f64>,
    pub anilist_score: Option<f64>,
    pub length: String,
    pub description: String,
}

/// Lightweight row for the catalogue list query (no `description`).
#[derive(Debug, Clone, FromRow)]
pub struct TitleListRow {
    pub id: i64,
    pub imdb_id: Option<String>,
    pub title: String,
    pub year: i64,
    #[sqlx(rename = "type")]
    pub kind: String,
    pub score: Option<f64>,
    pub anilist_score: Option<f64>,
    pub length: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TitleDto {
    pub id: i64,
    #[serde(rename = "imdbId")]
    pub imdb_id: Option<String>,
    pub title: String,
    pub year: i64,
    pub services: Vec<Service>,
    #[serde(rename = "type")]
    pub kind: TitleKind,
    pub genres: Vec<String>,
    pub score: Option<f64>,
    #[serde(rename = "anilistScore")]
    pub anilist_score: Option<f64>,
    pub len: String,
    pub desc: String,
    pub cast: Vec<String>,
    pub watched: bool,
    pub rating: Option<i64>,
    /// Services that resolve to a working watch link (availability only — no URLs).
    pub watchable: Vec<String>,
}

/// Slim list shape: `TitleDto` minus the DetailView-only `desc`/`cast`.
#[derive(Debug, Clone, Serialize)]
pub struct TitleListItem {
    pub id: i64,
    #[serde(rename = "imdbId")]
    pub imdb_id: Option<String>,
    pub title: String,
    pub year: i64,
    pub services: Vec<Service>,
    #[serde(rename = "type")]
    pub kind: TitleKind,
    pub genres: Vec<String>,
    pub score: Option<f64>,
    #[serde(rename = "anilistScore")]
    pub anilist_score: Option<f64>,
    pub len: String,
    pub watched: bool,
    pub rating: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dto_serializes_with_frontend_field_names() {
        let dto = TitleDto {
            id: 7,
            imdb_id: Some("tt0096895".to_string()),
            title: "Blade Runner 2049".to_string(),
            year: 2017,
            services: vec![Service::Plex],
            kind: TitleKind::Movie,
            genres: vec!["Sci-Fi".to_string(), "Drama".to_string()],
            score: Some(8.0),
            anilist_score: None,
            len: "164 min".to_string(),
            desc: "A replicant blade runner...".to_string(),
            cast: vec!["Ryan Gosling".to_string()],
            watched: false,
            rating: None,
            watchable: vec![],
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["type"], "movie");
        assert_eq!(v["imdbId"], "tt0096895");
        assert_eq!(v["services"], serde_json::json!(["plex"]));
        assert_eq!(v["score"], 8.0);
        assert_eq!(v["anilistScore"], serde_json::Value::Null);
        assert_eq!(v["rating"], serde_json::Value::Null);
    }

    #[test]
    fn enum_parsing_round_trips() {
        assert_eq!(Service::parse("crunchyroll"), Some(Service::Crunchyroll));
        assert_eq!(Service::parse("nope"), None);
        assert_eq!(TitleKind::parse("series"), Some(TitleKind::Series));
    }

    #[test]
    fn service_as_str_roundtrips_parse() {
        for s in [Service::Plex, Service::Disney, Service::Crunchyroll] {
            assert_eq!(Service::parse(s.as_str()), Some(s));
        }
    }

    #[test]
    fn title_kind_as_str_matches_db_values() {
        assert_eq!(TitleKind::Movie.as_str(), "movie");
        assert_eq!(TitleKind::Series.as_str(), "series");
    }
}
