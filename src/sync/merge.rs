//! Pure merge logic: genre normalization + dedup/union of fetched titles.

use std::collections::{BTreeSet, HashMap};

use crate::models::{Service, TitleKind};
use crate::sync::{FetchedTitle, ImageRef};

/// Prefer a remote (public CDN) ref over a Plex token-proxy ref; else keep `current`.
fn prefer_image(current: Option<ImageRef>, incoming: Option<ImageRef>) -> Option<ImageRef> {
    match (current, incoming) {
        (Some(c), Some(i)) if !c.remote && i.remote => Some(i),
        (Some(c), _) => Some(c),
        (None, x) => x,
    }
}

/// Map one raw genre to its normalized form (lowercased, trimmed, aliased).
fn normalize_one(raw: &str) -> String {
    let g = raw.trim().to_lowercase();
    match g.as_str() {
        "sci-fi" | "scifi" | "science-fiction" => "science fiction".to_string(),
        "rom-com" | "romcom" => "romantic comedy".to_string(),
        "docu" | "documentaries" => "documentary".to_string(),
        _ => g,
    }
}

/// Normalize, drop empties, de-duplicate, and sort a title's genres.
#[must_use]
pub fn normalize_genres(raws: &[String]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for r in raws {
        let g = normalize_one(r);
        if !g.is_empty() {
            set.insert(g);
        }
    }
    set.into_iter().collect()
}

/// A deduplicated title ready for DB upsert (genres already normalized).
#[derive(Debug, Clone, PartialEq)]
pub struct MergedTitle {
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub plex_guid: Option<String>,
    pub title: String,
    pub year: i64,
    pub kind: TitleKind,
    pub score: Option<f64>,
    pub length: String,
    pub description: String,
    pub genres: Vec<String>,
    pub cast: Vec<String>,
    pub services: Vec<Service>,
    pub plex_rating_key: Option<String>,
    pub links: Vec<(Service, String)>,
    pub poster: Option<ImageRef>,
    pub backdrop: Option<ImageRef>,
}

/// Stable identity key (D6): `IMDb` id, else `plex:<guid>`, else `tmdb:<id>`,
/// else a `title:<lower>:<year>` fallback.
#[must_use]
pub fn identity_key(
    imdb: Option<&str>,
    plex_guid: Option<&str>,
    tmdb: Option<&str>,
    title: &str,
    year: i64,
) -> String {
    if let Some(i) = imdb {
        return i.to_string();
    }
    if let Some(g) = plex_guid {
        return format!("plex:{g}");
    }
    if let Some(m) = tmdb {
        return format!("tmdb:{m}");
    }
    format!("title:{}:{year}", title.to_lowercase())
}

/// Dedup fetched rows by identity, unioning services/genres/cast and filling
/// missing scalar fields from whichever row first provides them.
///
/// # Panics
/// Panics if a key in `order` is not found in `by_key` (invariant bug).
#[must_use]
pub fn merge(fetched: Vec<FetchedTitle>) -> Vec<MergedTitle> {
    // Upper bound on distinct keys is one per fetched row; size the maps once.
    let n = fetched.len();
    let mut order: Vec<String> = Vec::with_capacity(n);
    let mut by_key: HashMap<String, MergedTitle> = HashMap::with_capacity(n);
    // Accumulate raw (un-normalized) genres per key, normalize once at the end.
    let mut raw_genres: HashMap<String, Vec<String>> = HashMap::with_capacity(n);

    for f in fetched {
        let key = identity_key(
            f.imdb_id.as_deref(),
            f.plex_guid.as_deref(),
            f.tmdb_id.as_deref(),
            &f.title,
            f.year.unwrap_or(0),
        );
        // `f.genres` is only used here, so move it in rather than clone.
        raw_genres.entry(key.clone()).or_default().extend(f.genres);
        if let Some(existing) = by_key.get_mut(&key) {
            for s in f.services {
                if !existing.services.contains(&s) {
                    existing.services.push(s);
                }
            }
            for c in f.cast {
                if !existing.cast.contains(&c) {
                    existing.cast.push(c);
                }
            }
            existing.imdb_id = existing.imdb_id.take().or(f.imdb_id);
            existing.tmdb_id = existing.tmdb_id.take().or(f.tmdb_id);
            existing.plex_guid = existing.plex_guid.take().or(f.plex_guid);
            existing.score = existing.score.or(f.score);
            if existing.description.is_empty() {
                existing.description = f.description.unwrap_or_default();
            }
            if existing.length.is_empty() {
                existing.length = f.length.unwrap_or_default();
            }
            existing.poster = prefer_image(existing.poster.take(), f.poster);
            existing.backdrop = prefer_image(existing.backdrop.take(), f.backdrop);
            existing.plex_rating_key = existing.plex_rating_key.take().or(f.plex_rating_key);
            for (svc, link) in f.links {
                if !existing.links.iter().any(|(s, _)| *s == svc) {
                    existing.links.push((svc, link));
                }
            }
        } else {
            order.push(key.clone());
            by_key.insert(
                key,
                MergedTitle {
                    imdb_id: f.imdb_id,
                    tmdb_id: f.tmdb_id,
                    plex_guid: f.plex_guid,
                    title: f.title,
                    year: f.year.unwrap_or(0),
                    kind: f.kind,
                    score: f.score,
                    length: f.length.unwrap_or_default(),
                    description: f.description.unwrap_or_default(),
                    genres: Vec::new(),
                    cast: f.cast,
                    services: f.services,
                    plex_rating_key: f.plex_rating_key.clone(),
                    links: f.links.clone(),
                    poster: f.poster,
                    backdrop: f.backdrop,
                },
            );
        }
    }

    order
        .into_iter()
        .map(|key| {
            let mut m = by_key.remove(&key).expect("key present");
            m.genres = normalize_genres(&raw_genres.remove(&key).unwrap_or_default());
            m
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Service, TitleKind};
    use crate::sync::FetchedTitle;

    #[test]
    fn normalize_lowercases_trims_dedups_sorts() {
        let out = normalize_genres(&["  Action ".into(), "Drama".into(), "action".into()]);
        assert_eq!(out, vec!["action".to_string(), "drama".to_string()]);
    }

    #[test]
    fn normalize_applies_alias_map() {
        let out = normalize_genres(&["Sci-Fi".into(), "SciFi".into(), "Science-Fiction".into()]);
        assert_eq!(out, vec!["science fiction".to_string()]);
    }

    #[test]
    fn normalize_drops_empty() {
        let out = normalize_genres(&[String::new(), "   ".into(), "Comedy".into()]);
        assert_eq!(out, vec!["comedy".to_string()]);
    }

    #[test]
    fn identity_prefers_imdb_then_plex_then_tmdb_then_title() {
        assert_eq!(
            identity_key(Some("tt9"), Some("g"), Some("5"), "X", 2000),
            "tt9"
        );
        assert_eq!(
            identity_key(None, Some("g"), Some("5"), "X", 2000),
            "plex:g"
        );
        assert_eq!(identity_key(None, None, Some("5"), "X", 2000), "tmdb:5");
        assert_eq!(
            identity_key(None, None, None, "The Film", 2000),
            "title:the film:2000"
        );
    }

    #[test]
    fn merge_unions_services_and_genres_for_same_imdb() {
        let a = ft(
            Some("tt1"),
            TitleKind::Movie,
            vec!["Action".into()],
            vec![Service::Plex],
        );
        let b = ft(
            Some("tt1"),
            TitleKind::Movie,
            vec!["action".into(), "Drama".into()],
            vec![Service::Disney],
        );
        let out = merge(vec![a, b]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].services, vec![Service::Plex, Service::Disney]);
        assert_eq!(
            out[0].genres,
            vec!["action".to_string(), "drama".to_string()]
        );
    }

    #[test]
    fn merge_keeps_distinct_titles_in_first_seen_order() {
        let a = ft(Some("tt1"), TitleKind::Movie, vec![], vec![Service::Plex]);
        let b = ft(
            Some("tt2"),
            TitleKind::Series,
            vec![],
            vec![Service::Crunchyroll],
        );
        let out = merge(vec![a, b]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].imdb_id.as_deref(), Some("tt1"));
        assert_eq!(out[1].imdb_id.as_deref(), Some("tt2"));
    }

    #[test]
    fn merge_prefers_remote_image_over_plex_path() {
        use crate::sync::ImageRef;
        let mut plex_first = ft(Some("tt1"), TitleKind::Movie, vec![], vec![Service::Plex]);
        plex_first.poster = Some(ImageRef {
            value: "/library/p.jpg".into(),
            remote: false,
        });
        let mut motn_second = ft(Some("tt1"), TitleKind::Movie, vec![], vec![Service::Disney]);
        motn_second.poster = Some(ImageRef {
            value: "https://cdn/p.jpg".into(),
            remote: true,
        });
        motn_second.backdrop = Some(ImageRef {
            value: "https://cdn/b.jpg".into(),
            remote: true,
        });

        let out = merge(vec![plex_first, motn_second]);
        assert_eq!(out.len(), 1);
        let p = out[0].poster.as_ref().unwrap();
        assert!(p.remote, "remote CDN ref must win over a Plex path");
        assert_eq!(p.value, "https://cdn/p.jpg");
        let b = out[0].backdrop.as_ref().unwrap();
        assert!(b.remote);
        assert_eq!(b.value, "https://cdn/b.jpg");
    }

    #[test]
    fn merge_keeps_remote_when_plex_seen_second() {
        use crate::sync::ImageRef;
        let mut motn_first = ft(Some("tt1"), TitleKind::Movie, vec![], vec![Service::Disney]);
        motn_first.poster = Some(ImageRef {
            value: "https://cdn/p.jpg".into(),
            remote: true,
        });
        let mut plex_second = ft(Some("tt1"), TitleKind::Movie, vec![], vec![Service::Plex]);
        plex_second.poster = Some(ImageRef {
            value: "/library/p.jpg".into(),
            remote: false,
        });

        let out = merge(vec![motn_first, plex_second]);
        let p = out[0].poster.as_ref().unwrap();
        assert!(p.remote);
        assert_eq!(p.value, "https://cdn/p.jpg");
    }

    #[test]
    fn merge_leaves_images_none_when_absent() {
        let a = ft(Some("tt1"), TitleKind::Movie, vec![], vec![Service::Plex]);
        let b = ft(Some("tt1"), TitleKind::Movie, vec![], vec![Service::Disney]);
        let out = merge(vec![a, b]);
        assert_eq!(out.len(), 1);
        assert!(out[0].poster.is_none());
        assert!(out[0].backdrop.is_none());
    }

    // test helper
    fn ft(
        imdb: Option<&str>,
        kind: TitleKind,
        genres: Vec<String>,
        services: Vec<Service>,
    ) -> FetchedTitle {
        FetchedTitle {
            imdb_id: imdb.map(str::to_string),
            tmdb_id: None,
            plex_guid: None,
            title: "T".into(),
            year: Some(2001),
            kind,
            score: None,
            length: None,
            description: None,
            genres,
            cast: vec![],
            services,
            plex_rating_key: None,
            links: vec![],
            poster: None,
            backdrop: None,
        }
    }

    #[test]
    fn merge_unions_links_and_fills_rating_key() {
        use crate::models::{Service, TitleKind};
        let plex = FetchedTitle {
            imdb_id: Some("tt9".into()),
            tmdb_id: None,
            plex_guid: None,
            title: "X".into(),
            year: Some(2020),
            kind: TitleKind::Movie,
            score: None,
            length: None,
            description: None,
            genres: vec![],
            cast: vec![],
            services: vec![Service::Plex],
            plex_rating_key: Some("777".into()),
            links: vec![],
            poster: None,
            backdrop: None,
        };
        let motn = FetchedTitle {
            imdb_id: Some("tt9".into()),
            tmdb_id: None,
            plex_guid: None,
            title: "X".into(),
            year: Some(2020),
            kind: TitleKind::Movie,
            score: None,
            length: None,
            description: None,
            genres: vec![],
            cast: vec![],
            services: vec![Service::Crunchyroll],
            plex_rating_key: None,
            links: vec![(Service::Crunchyroll, "https://crunchyroll.com/x".into())],
            poster: None,
            backdrop: None,
        };
        let out = merge(vec![plex, motn]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].plex_rating_key.as_deref(), Some("777"));
        assert_eq!(
            out[0].links,
            vec![(Service::Crunchyroll, "https://crunchyroll.com/x".into())]
        );
    }
}
