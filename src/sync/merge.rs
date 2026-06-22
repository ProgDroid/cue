//! Pure merge logic: genre normalization + dedup/union of fetched titles.

use std::collections::BTreeSet;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
