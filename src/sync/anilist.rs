//! `AniList` enrichment: offline Fribb id-map (imdb/tmdb -> anilist) + batched
//! score fetch. Additive and wipe-guarded — any failure retains cached scores.

use std::collections::HashMap;

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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("testdata/fribb_sample.json");

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
