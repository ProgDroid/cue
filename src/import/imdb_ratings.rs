use serde::Deserialize;

/// One importable rating row distilled from the `IMDb` export.
pub struct RatingImport {
    pub imdb_id: String,
    pub rating: i64,
    /// `IMDb` `Date Rated` (`YYYY-MM-DD`) when valid; `None` falls back to the DB default.
    pub rated_at: Option<String>,
}

/// Result of parsing an `IMDb` ratings CSV.
pub struct ParsedImport {
    pub rows: Vec<RatingImport>,
    pub skipped: usize,
}

/// Columns consumed from the `IMDb` export; header-named so order/extra columns don't matter.
#[derive(Deserialize)]
struct ImdbRow {
    #[serde(rename = "Const")]
    const_id: String,
    #[serde(rename = "Your Rating")]
    your_rating: Option<String>,
    #[serde(rename = "Date Rated")]
    date_rated: Option<String>,
}

/// True when `s` is exactly `YYYY-MM-DD` and names a real calendar date
/// (month lengths and leap years checked).
fn looks_like_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |range: std::ops::Range<usize>| s[range].bytes().all(|c| c.is_ascii_digit());
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let year: u32 = s[0..4].parse().unwrap_or(0);
    let month: u8 = s[5..7].parse().unwrap_or(0);
    let day: u8 = s[8..10].parse().unwrap_or(0);
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days_in_month).contains(&day)
}

/// Parse an `IMDb` ratings-export CSV into validated rows.
///
/// Rows with a blank `Const` are ignored. Rows whose `Your Rating` is missing,
/// non-integer, or outside 1–10 are counted in `skipped`. A `Date Rated` that
/// is not `YYYY-MM-DD` is dropped to `None`.
#[must_use]
pub fn parse_ratings(csv: &str) -> ParsedImport {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(csv.as_bytes());

    let mut rows = Vec::new();
    let mut skipped = 0_usize;

    for result in reader.deserialize::<ImdbRow>() {
        let Ok(row) = result else {
            skipped += 1;
            continue;
        };
        let const_id = row.const_id.trim();
        if const_id.is_empty() {
            continue; // ignored, not skipped
        }
        let rating = match row
            .your_rating
            .as_deref()
            .map(str::trim)
            .and_then(|s| s.parse::<i64>().ok())
        {
            Some(r) if (1..=10).contains(&r) => r,
            _ => {
                skipped += 1;
                continue;
            }
        };
        let rated_at = row
            .date_rated
            .as_deref()
            .map(str::trim)
            .filter(|s| looks_like_iso_date(s))
            .map(ToString::to_string);

        rows.push(RatingImport {
            imdb_id: const_id.to_string(),
            rating,
            rated_at,
        });
    }

    ParsedImport { rows, skipped }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Const,Your Rating,Date Rated,Title,Genres\n\
tt0111161,10,2019-03-14,The Shawshank Redemption,Drama\n\
tt0137523,9,2020-01-02,\"Fight Club\",Drama\n\
tt0110912,8,not-a-date,\"Pulp Fiction, a film\",\"Crime, Drama\"\n\
tt0000001,0,2021-05-05,Bad Low,Drama\n\
tt0000002,11,2021-05-06,Bad High,Drama\n\
tt0000003,,2021-05-07,Unrated,Drama\n";

    #[test]
    fn parses_valid_rows_and_counts_skips() {
        let parsed = parse_ratings(SAMPLE);
        // 3 valid (10, 9, 8); 3 skipped (0, 11, blank).
        assert_eq!(parsed.rows.len(), 3);
        assert_eq!(parsed.skipped, 3);
    }

    #[test]
    fn preserves_quoted_title_and_maps_fields() {
        let parsed = parse_ratings(SAMPLE);
        let pulp = parsed
            .rows
            .iter()
            .find(|r| r.imdb_id == "tt0110912")
            .expect("pulp row present");
        assert_eq!(pulp.rating, 8);
    }

    #[test]
    fn keeps_valid_date_and_drops_invalid() {
        let parsed = parse_ratings(SAMPLE);
        let shawshank = parsed
            .rows
            .iter()
            .find(|r| r.imdb_id == "tt0111161")
            .unwrap();
        let pulp = parsed
            .rows
            .iter()
            .find(|r| r.imdb_id == "tt0110912")
            .unwrap();
        assert_eq!(shawshank.rated_at.as_deref(), Some("2019-03-14"));
        assert_eq!(pulp.rated_at, None); // "not-a-date" dropped
    }

    #[test]
    fn blank_const_is_ignored_not_skipped() {
        let csv = "Const,Your Rating,Date Rated\n\
,7,2021-01-01\n\
tt0111161,7,2021-01-01\n";
        let parsed = parse_ratings(csv);
        assert_eq!(parsed.rows.len(), 1);
        assert_eq!(parsed.skipped, 0); // blank Const ignored, not counted
    }

    #[test]
    fn empty_or_headerless_yields_no_rows() {
        assert_eq!(parse_ratings("").rows.len(), 0);
        // No recognizable Const column -> every row fails -> no rows.
        assert_eq!(parse_ratings("a,b,c\n1,2,3\n").rows.len(), 0);
    }

    #[test]
    fn iso_date_rejects_impossible_calendar_dates() {
        assert!(looks_like_iso_date("2020-02-29"), "leap day");
        assert!(looks_like_iso_date("2000-02-29"), "400-year leap day");
        assert!(looks_like_iso_date("2021-12-31"));
        assert!(!looks_like_iso_date("2021-02-29"), "not a leap year");
        assert!(
            !looks_like_iso_date("1900-02-29"),
            "century, not a leap year"
        );
        assert!(!looks_like_iso_date("2021-02-31"));
        assert!(!looks_like_iso_date("2021-04-31"));
        assert!(!looks_like_iso_date("2021-00-10"));
        assert!(!looks_like_iso_date("2021-13-10"));
        assert!(!looks_like_iso_date("2021-01-00"));
    }
}
