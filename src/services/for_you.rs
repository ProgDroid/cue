//! "For you" ranking: pure scoring plus a cached service.
//!
//! The scoring half is pure (no I/O, no async): a mean-centred, normalised
//! vector set and per-candidate top-k scoring against the user's ratings.
//! [`ForYouService`] wraps it with I/O: it loads embeddings and ratings from
//! `SQLite`, ranks in `spawn_blocking`, and caches the vector set and the
//! ranking keyed on a data fingerprint.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::Mutex;

use crate::db::embeddings;
use crate::services::embeddings::EMBED_MODEL;
use crate::services::similarity::{centroid, dot};

/// Number of nearest positives averaged into `pos(c)`.
pub const FOR_YOU_K: usize = 5;
/// Weight of the nearest-negative penalty.
pub const FOR_YOU_LAMBDA: f32 = 0.5;
/// Most positives kept (highest weight, then most recent).
pub const MAX_POSITIVES: usize = 300;
/// Most negatives kept (highest weight, then most recent).
pub const MAX_NEGATIVES: usize = 100;
/// Fewer positives than this yields no recommendations.
pub const MIN_BASIS: usize = 3;

/// Catalogue embeddings, mean-centred and re-normalised.
#[derive(Debug, Clone, Default)]
pub struct VectorSet {
    pub ids: Vec<i64>,
    pub index: HashMap<i64, usize>,
    pub vecs: Vec<Vec<f32>>,
}

impl VectorSet {
    /// Keep only vectors of the most common length, subtract the mean vector,
    /// re-normalise, and drop zero-norm (or non-finite) results and duplicate ids.
    #[must_use]
    pub fn build(raw: Vec<(i64, Vec<f32>)>) -> Self {
        let mut by_len: HashMap<usize, usize> = HashMap::new();
        for (_, v) in &raw {
            *by_len.entry(v.len()).or_default() += 1;
        }
        // Most common length; ties go to the longer one so the result is deterministic.
        let Some(len) = by_len
            .into_iter()
            .filter(|(len, _)| *len > 0)
            .max_by_key(|&(len, count)| (count, len))
            .map(|(len, _)| len)
        else {
            return Self::default();
        };
        let kept: Vec<(i64, Vec<f32>)> = raw.into_iter().filter(|(_, v)| v.len() == len).collect();
        let vectors: Vec<Vec<f32>> = kept.iter().map(|(_, v)| v.clone()).collect();
        let mean = centroid(&vectors);

        let mut set = Self::default();
        for ((id, mut v), _) in kept.into_iter().zip(vectors) {
            if set.index.contains_key(&id) {
                continue;
            }
            for (x, m) in v.iter_mut().zip(&mean) {
                *x -= m;
            }
            let norm = dot(&v, &v).sqrt();
            if !norm.is_finite() || norm <= f32::EPSILON {
                continue;
            }
            for x in &mut v {
                *x /= norm;
            }
            set.index.insert(id, set.ids.len());
            set.ids.push(id);
            set.vecs.push(v);
        }
        set
    }
}

/// One user rating of a catalogue title.
#[derive(Debug, Clone)]
pub struct Rated {
    pub title_id: i64,
    pub rating: i64,
    /// `YYYY-MM-DD HH:MM:SS`; compares lexicographically.
    pub rated_at: String,
}

/// Ranked recommendation ids plus the number of positives they are based on.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ForYouResult {
    pub ids: Vec<i64>,
    pub basis: usize,
}

/// `(title_id, weight)` pairs for the capped positive and negative sets.
#[derive(Debug, Default)]
pub(crate) struct Basis {
    pub positives: Vec<(i64, f32)>,
    pub negatives: Vec<(i64, f32)>,
    /// Positives before capping (`|P|`).
    pub positive_count: usize,
}

/// `steps / 4` for a rating distance of 1..=4 (0.25..=1.0).
fn weight(steps: i64) -> f32 {
    f32::from(u8::try_from(steps).unwrap_or(0)) / 4.0
}

/// Sort by weight descending, then `rated_at` descending (most recent first), and cap.
fn capped(mut items: Vec<(i64, f32, &str)>, cap: usize) -> Vec<(i64, f32)> {
    items.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| b.2.cmp(a.2)));
    items.truncate(cap);
    items.into_iter().map(|(id, w, _)| (id, w)).collect()
}

/// Split ratings into weighted, capped positives (7-10) and negatives (1-4).
/// Ratings of titles missing from `set` and ratings of 5-6 are ignored.
pub(crate) fn select_basis(set: &VectorSet, ratings: &[Rated]) -> Basis {
    let mut pos: Vec<(i64, f32, &str)> = Vec::new();
    let mut neg: Vec<(i64, f32, &str)> = Vec::new();
    for r in ratings {
        if !set.index.contains_key(&r.title_id) {
            continue;
        }
        let at = r.rated_at.as_str();
        match r.rating {
            7..=10 => pos.push((r.title_id, weight(r.rating - 6), at)),
            1..=4 => neg.push((r.title_id, weight(5 - r.rating), at)),
            _ => {}
        }
    }
    let positive_count = pos.len();
    Basis {
        positives: capped(pos, MAX_POSITIVES),
        negatives: capped(neg, MAX_NEGATIVES),
        positive_count,
    }
}

/// `score(c) = pos(c) - lambda * neg(c)` for a normalised candidate vector `c`.
///
/// `pos`: weighted mean cosine of the `k = min(FOR_YOU_K, |P|)` positives with the
/// highest plain cosine (selection ignores weights). `neg`: the highest plain cosine
/// to any negative, times that negative's weight (0 when there are none).
pub(crate) fn candidate_score(
    c: &[f32],
    positives: &[(&[f32], f32)],
    negatives: &[(&[f32], f32)],
) -> f32 {
    let mut cos: Vec<(f32, f32)> = positives.iter().map(|(v, w)| (dot(c, v), *w)).collect();
    let k = FOR_YOU_K.min(cos.len());
    let pos = if k == 0 {
        0.0
    } else {
        if k < cos.len() {
            cos.select_nth_unstable_by(k - 1, |a, b| b.0.total_cmp(&a.0));
        }
        let (sum_wc, sum_w) = cos[..k]
            .iter()
            .fold((0.0_f32, 0.0_f32), |(swc, sw), (cs, w)| {
                (swc + w * cs, sw + w)
            });
        sum_wc / sum_w
    };
    let neg = negatives
        .iter()
        .map(|(v, w)| (dot(c, v), *w))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(0.0, |(cs, w)| cs * w);
    FOR_YOU_LAMBDA.mul_add(-neg, pos)
}

/// Rank every candidate (in `set`, not in `excluded`) for the given ratings.
/// Fewer than `MIN_BASIS` positives yields no ids.
#[must_use]
#[allow(clippy::implicit_hasher)] // fixed interface: callers always pass the default hasher
pub fn rank(set: &VectorSet, ratings: &[Rated], excluded: &HashSet<i64>) -> ForYouResult {
    let sel = select_basis(set, ratings);
    if sel.positive_count < MIN_BASIS {
        return ForYouResult {
            ids: Vec::new(),
            basis: sel.positive_count,
        };
    }
    let vec_of = |&(id, w): &(i64, f32)| (set.vecs[set.index[&id]].as_slice(), w);
    let positives: Vec<(&[f32], f32)> = sel.positives.iter().map(vec_of).collect();
    let negatives: Vec<(&[f32], f32)> = sel.negatives.iter().map(vec_of).collect();

    let mut scored: Vec<(i64, f32)> = set
        .ids
        .iter()
        .zip(&set.vecs)
        .filter(|(id, _)| !excluded.contains(id))
        .map(|(id, v)| (*id, candidate_score(v, &positives, &negatives)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ForYouResult {
        ids: scored.into_iter().map(|(id, _)| id).collect(),
        basis: sel.positive_count,
    }
}

/// Cache key for the vector set: changes whenever embeddings, titles or syncs do.
#[derive(Debug, Clone, PartialEq, Eq)]
struct VectorKey {
    embeddings: i64,
    max_embedded_id: i64,
    titles: i64,
    last_sync_id: i64,
}

/// Cache key for a ranked result: the vector key plus the user's rating and watch state.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResultKey {
    vectors: VectorKey,
    ratings: i64,
    rating_sum: i64,
    last_rated_at: String,
    watched: i64,
}

#[derive(Default)]
struct Cache {
    vectors: Option<(VectorKey, Arc<VectorSet>)>,
    result: Option<(ResultKey, ForYouResult)>,
}

/// Computes and caches "For you" rankings.
///
/// Cheap cache-key queries run on every call; embeddings are reloaded only when the
/// vector key changes and the ranking is recomputed only when the result key changes.
#[derive(Default)]
pub struct ForYouService {
    cache: Mutex<Cache>,
    computations: AtomicUsize,
    vector_builds: AtomicUsize,
}

impl ForYouService {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of times a ranking was computed (test hook).
    #[cfg(test)]
    pub fn computations(&self) -> usize {
        self.computations.load(Ordering::Relaxed)
    }

    /// Number of times the vector set was rebuilt (test hook).
    #[cfg(test)]
    pub fn vector_builds(&self) -> usize {
        self.vector_builds.load(Ordering::Relaxed)
    }

    /// The ranked recommendations for the current catalogue and user data.
    ///
    /// # Errors
    /// Returns an error if a query fails or a background task panics.
    pub async fn get(&self, pool: &SqlitePool) -> anyhow::Result<ForYouResult> {
        // One lock for the whole call: concurrent requests share a single computation.
        let mut cache = self.cache.lock().await;
        let key = result_key(pool).await?;

        if let Some((k, r)) = &cache.result {
            if *k == key {
                return Ok(r.clone());
            }
        }

        let set = match &cache.vectors {
            Some((k, set)) if *k == key.vectors => Arc::clone(set),
            _ => {
                let raw = embeddings::load_all(pool, EMBED_MODEL).await?;
                let set =
                    Arc::new(tokio::task::spawn_blocking(move || VectorSet::build(raw)).await?);
                self.vector_builds.fetch_add(1, Ordering::Relaxed);
                cache.vectors = Some((key.vectors.clone(), Arc::clone(&set)));
                set
            }
        };

        let (ratings, excluded) = load_user_state(pool).await?;
        let result = tokio::task::spawn_blocking(move || rank(&set, &ratings, &excluded)).await?;
        self.computations.fetch_add(1, Ordering::Relaxed);
        cache.result = Some((key, result.clone()));
        drop(cache);
        Ok(result)
    }
}

async fn result_key(pool: &SqlitePool) -> anyhow::Result<ResultKey> {
    let row: (i64, i64, i64, i64, i64, i64, String, i64) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM title_embeddings WHERE model = ?1),
            (SELECT COALESCE(MAX(title_id), 0) FROM title_embeddings WHERE model = ?1),
            (SELECT COUNT(*) FROM titles),
            (SELECT COALESCE(MAX(id), 0) FROM sync_runs),
            (SELECT COUNT(*) FROM user_ratings),
            (SELECT COALESCE(SUM(rating), 0) FROM user_ratings),
            (SELECT COALESCE(MAX(rated_at), '') FROM user_ratings),
            (SELECT COUNT(*) FROM watch_history)",
    )
    .bind(EMBED_MODEL)
    .fetch_one(pool)
    .await?;
    Ok(ResultKey {
        vectors: VectorKey {
            embeddings: row.0,
            max_embedded_id: row.1,
            titles: row.2,
            last_sync_id: row.3,
        },
        ratings: row.4,
        rating_sum: row.5,
        last_rated_at: row.6,
        watched: row.7,
    })
}

/// Ratings of catalogue titles, and the ids to exclude (rated or watched titles).
async fn load_user_state(pool: &SqlitePool) -> anyhow::Result<(Vec<Rated>, HashSet<i64>)> {
    let rows: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT t.id, r.rating, r.rated_at
         FROM user_ratings r JOIN titles t ON t.imdb_id = r.imdb_id",
    )
    .fetch_all(pool)
    .await?;
    let watched: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM titles WHERE imdb_id IN (SELECT imdb_id FROM watch_history)",
    )
    .fetch_all(pool)
    .await?;
    let mut excluded: HashSet<i64> = watched.into_iter().collect();
    excluded.extend(rows.iter().map(|(id, _, _)| *id));
    let ratings = rows
        .into_iter()
        .map(|(title_id, rating, rated_at)| Rated {
            title_id,
            rating,
            rated_at,
        })
        .collect();
    Ok((ratings, excluded))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::similarity::cosine;

    /// A set of already-normalised vectors, bypassing centring.
    fn raw_set(items: Vec<(i64, Vec<f32>)>) -> VectorSet {
        let mut s = VectorSet::default();
        for (id, v) in items {
            s.index.insert(id, s.ids.len());
            s.ids.push(id);
            s.vecs.push(v);
        }
        s
    }

    fn rated(title_id: i64, rating: i64, rated_at: &str) -> Rated {
        Rated {
            title_id,
            rating,
            rated_at: rated_at.to_string(),
        }
    }

    /// Unit vector at cosine `x` to `[1, 0]`.
    fn at_cos(x: f32) -> Vec<f32> {
        vec![x, x.mul_add(-x, 1.0).sqrt()]
    }

    #[test]
    fn vector_set_skips_mismatched_lengths() {
        let s = VectorSet::build(vec![
            (1, vec![1.0, 0.0]),
            (2, vec![0.0, 1.0]),
            (3, vec![1.0, 0.0, 0.0]),
        ]);
        assert!(!s.index.contains_key(&3));
        assert!(s.index.contains_key(&1) && s.index.contains_key(&2));
    }

    #[test]
    fn vector_set_drops_zero_norm_after_centring() {
        // Identical vectors centre to zero and are all dropped.
        let s = VectorSet::build(vec![(1, vec![1.0, 2.0]), (2, vec![1.0, 2.0])]);
        assert!(s.ids.is_empty());
    }

    #[test]
    fn vector_set_is_unit_length() {
        let s = VectorSet::build(vec![
            (1, vec![3.0, 1.0]),
            (2, vec![1.0, 5.0]),
            (3, vec![0.0, 0.0]),
        ]);
        for v in &s.vecs {
            assert!((dot(v, v) - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn centring_separates_vectors_that_share_a_common_component() {
        // Raw cosines all ~0.99; after centring 1 and 2 point opposite ways.
        let s = VectorSet::build(vec![
            (1, vec![10.0, 1.0]),
            (2, vec![10.0, -1.0]),
            (3, vec![10.0, 0.0]),
        ]);
        let (a, b) = (&s.vecs[s.index[&1]], &s.vecs[s.index[&2]]);
        assert!(cosine(a, b) < -0.9);
    }

    #[test]
    fn basis_below_three_returns_no_ids() {
        let set = raw_set(vec![
            (1, vec![1.0, 0.0]),
            (2, vec![0.0, 1.0]),
            (3, at_cos(0.5)),
        ]);
        let ratings = [
            rated(1, 9, "2026-01-01 00:00:00"),
            rated(2, 7, "2026-01-02 00:00:00"),
        ];
        let r = rank(&set, &ratings, &HashSet::from([1, 2]));
        assert!(r.ids.is_empty());
        assert_eq!(r.basis, 2);
    }

    #[test]
    fn ratings_for_unknown_titles_are_ignored() {
        let set = raw_set(vec![
            (1, vec![1.0, 0.0]),
            (2, vec![0.0, 1.0]),
            (3, at_cos(0.5)),
        ]);
        let ratings = [
            rated(1, 9, "2026-01-01 00:00:00"),
            rated(2, 9, "2026-01-02 00:00:00"),
            rated(99, 9, "2026-01-03 00:00:00"),
        ];
        let r = rank(&set, &ratings, &HashSet::from([1, 2]));
        assert_eq!(r.basis, 2);
        assert!(r.ids.is_empty());
    }

    #[test]
    fn neighbours_are_selected_by_cosine_not_weight() {
        let c = [1.0_f32, 0.0];
        let near = at_cos(0.9);
        let far = at_cos(0.3);
        let mut positives: Vec<(&[f32], f32)> = (0..5).map(|_| (near.as_slice(), 0.25)).collect();
        positives.push((far.as_slice(), 1.0));
        // Weighted-cos selection would pick the far one (1.0*0.3 > 0.25*0.9).
        let score = candidate_score(&c, &positives, &[]);
        assert!((score - 0.9).abs() < 1e-5, "score = {score}");
    }

    #[test]
    fn pos_is_weighted_mean_of_selected_cosines() {
        let c = [1.0_f32, 0.0];
        let (a, b) = (at_cos(0.8), at_cos(0.4));
        let score = candidate_score(&c, &[(&a, 1.0), (&b, 0.25)], &[]);
        let expected = 0.25_f32.mul_add(0.4, 0.8) / 1.25;
        assert!((score - expected).abs() < 1e-5);
    }

    #[test]
    fn neg_uses_highest_cosine_negative_times_its_weight() {
        let c = [1.0_f32, 0.0];
        let (p, n1, n2) = (at_cos(0.5), at_cos(0.9), at_cos(0.8));
        // Closest negative (cos 0.9) has weight 0.25 -> neg = 0.225, not 1.0*0.8.
        let score = candidate_score(&c, &[(&p, 1.0)], &[(&n1, 0.25), (&n2, 1.0)]);
        let expected = FOR_YOU_LAMBDA.mul_add(-(0.9 * 0.25), 0.5);
        assert!((score - expected).abs() < 1e-5, "score = {score}");
    }

    #[test]
    fn disliked_neighbour_penalises() {
        // Three likes at x; candidates 10 and 11 are equally close to them.
        // 10 is also near the rating-1 title 4 (along y), so it ranks second.
        let set = raw_set(vec![
            (1, vec![1.0, 0.0, 0.0]),
            (2, vec![1.0, 0.0, 0.0]),
            (3, vec![1.0, 0.0, 0.0]),
            (4, vec![0.0, 1.0, 0.0]),
            (10, vec![0.8, 0.6, 0.0]),
            (11, vec![0.8, 0.0, 0.6]),
        ]);
        let ratings = [
            rated(1, 10, "2026-01-01 00:00:00"),
            rated(2, 10, "2026-01-02 00:00:00"),
            rated(3, 10, "2026-01-03 00:00:00"),
            rated(4, 1, "2026-01-04 00:00:00"),
        ];
        let r = rank(&set, &ratings, &HashSet::from([1, 2, 3, 4]));
        assert_eq!(r.ids, vec![11, 10]);
        assert_eq!(r.basis, 3);
    }

    #[test]
    fn excluded_titles_never_appear() {
        let set = raw_set(vec![
            (1, vec![1.0, 0.0]),
            (2, vec![1.0, 0.0]),
            (3, vec![1.0, 0.0]),
            (4, at_cos(0.9)),
            (5, at_cos(0.8)),
            (6, at_cos(0.7)),
        ]);
        let ratings = [
            rated(1, 9, "2026-01-01 00:00:00"),
            rated(2, 9, "2026-01-02 00:00:00"),
            rated(3, 9, "2026-01-03 00:00:00"),
        ];
        // 1-3 are rated; 5 is watched.
        let r = rank(&set, &ratings, &HashSet::from([1, 2, 3, 5]));
        assert_eq!(r.ids, vec![4, 6]);
    }

    #[test]
    fn ties_break_by_id() {
        let set = raw_set(vec![
            (1, vec![1.0, 0.0]),
            (2, vec![1.0, 0.0]),
            (3, vec![1.0, 0.0]),
            (30, at_cos(0.6)),
            (10, at_cos(0.6)),
            (20, at_cos(0.6)),
        ]);
        let ratings = [
            rated(1, 9, "2026-01-01 00:00:00"),
            rated(2, 9, "2026-01-02 00:00:00"),
            rated(3, 9, "2026-01-03 00:00:00"),
        ];
        let r = rank(&set, &ratings, &HashSet::from([1, 2, 3]));
        assert_eq!(r.ids, vec![10, 20, 30]);
    }

    #[test]
    fn caps_positives_at_300_by_weight_then_recency() {
        // Ids 1..=299 rated 9 (w 0.75). Ids 300 and 301 rated 7 (w 0.5): 300 is
        // older, so it is the one dropped.
        let set = raw_set((1..=301).map(|i| (i, vec![1.0, 0.0])).collect());
        let mut ratings: Vec<Rated> = (1..=299)
            .map(|i| rated(i, 9, "2026-03-01 00:00:00"))
            .collect();
        ratings.push(rated(300, 7, "2020-01-01 00:00:00"));
        ratings.push(rated(301, 7, "2026-01-01 00:00:00"));
        let sel = select_basis(&set, &ratings);
        assert_eq!(sel.positives.len(), MAX_POSITIVES);
        assert!(sel.positives.iter().any(|(id, _)| *id == 301));
        assert!(!sel.positives.iter().any(|(id, _)| *id == 300));
        assert_eq!(sel.positive_count, 301);
    }

    #[test]
    fn caps_negatives_at_100_and_ignores_neutral_ratings() {
        let set = raw_set((1..=103).map(|i| (i, vec![1.0, 0.0])).collect());
        let mut ratings: Vec<Rated> = (1..=101)
            .map(|i| rated(i, 4, "2026-01-01 00:00:00"))
            .collect();
        ratings.push(rated(102, 1, "2026-01-01 00:00:00"));
        ratings.push(rated(103, 5, "2026-01-01 00:00:00"));
        let sel = select_basis(&set, &ratings);
        assert_eq!(sel.negatives.len(), MAX_NEGATIVES);
        assert_eq!(sel.negatives[0], (102, 1.0));
        assert!(sel.positives.is_empty());
    }

    #[test]
    fn weights_follow_the_rating_formula() {
        let set = raw_set((1..=4).map(|i| (i, vec![1.0, 0.0])).collect());
        let ratings = [
            rated(1, 7, "2026-01-01 00:00:00"),
            rated(2, 10, "2026-01-01 00:00:00"),
            rated(3, 4, "2026-01-01 00:00:00"),
            rated(4, 1, "2026-01-01 00:00:00"),
        ];
        let sel = select_basis(&set, &ratings);
        assert_eq!(sel.positives, vec![(2, 1.0), (1, 0.25)]);
        assert_eq!(sel.negatives, vec![(4, 1.0), (3, 0.25)]);
    }
}

#[cfg(test)]
mod service_tests {
    use super::*;
    use crate::db::{embeddings, init_pool, user_data};
    use crate::services::embeddings::EMBED_MODEL;
    use sqlx::SqlitePool;

    async fn fresh_pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (init_pool(&url).await.unwrap(), dir)
    }

    /// Insert a title with an embedding; returns its id.
    async fn seed(pool: &SqlitePool, imdb: &str, vector: &[f32]) -> i64 {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO titles (imdb_id, title, year, type) VALUES (?, 'T', 2020, 'movie') RETURNING id",
        )
        .bind(imdb)
        .fetch_one(pool)
        .await
        .unwrap();
        embeddings::upsert(pool, id, vector, EMBED_MODEL)
            .await
            .unwrap();
        id
    }

    /// Three liked titles, two candidates (closer first).
    async fn seed_catalogue(pool: &SqlitePool) {
        for (imdb, v) in [
            ("tt1", [1.0, 0.0, 0.0]),
            ("tt2", [0.9, 0.1, 0.0]),
            ("tt3", [0.9, -0.1, 0.0]),
            ("tt4", [0.8, 0.05, 0.0]),
            ("tt5", [0.0, 1.0, 0.0]),
        ] {
            seed(pool, imdb, &v).await;
        }
        for imdb in ["tt1", "tt2", "tt3"] {
            user_data::set_rating(pool, imdb, 9).await.unwrap();
        }
    }

    #[tokio::test]
    async fn result_cache_hits_until_a_rating_changes() {
        let (pool, _dir) = fresh_pool().await;
        seed_catalogue(&pool).await;
        let svc = ForYouService::new();
        let first = svc.get(&pool).await.unwrap();
        let second = svc.get(&pool).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(first.basis, 3);
        assert_eq!(svc.computations(), 1);

        user_data::set_rating(&pool, "tt1", 8).await.unwrap();
        svc.get(&pool).await.unwrap();
        assert_eq!(svc.computations(), 2);
        // A rating change never rebuilds the vector set.
        assert_eq!(svc.vector_builds(), 1);

        // Watching a title also invalidates the result.
        user_data::set_watched(&pool, "tt4", true).await.unwrap();
        let after_watch = svc.get(&pool).await.unwrap();
        assert_eq!(svc.computations(), 3);
        assert_eq!(after_watch.ids.len(), 1);
    }

    #[tokio::test]
    async fn vector_cache_rebuilds_after_new_embeddings() {
        let (pool, _dir) = fresh_pool().await;
        seed_catalogue(&pool).await;
        let svc = ForYouService::new();
        svc.get(&pool).await.unwrap();
        svc.get(&pool).await.unwrap();
        assert_eq!(svc.vector_builds(), 1);

        seed(&pool, "tt6", &[0.5, 0.5, 0.0]).await;
        let r = svc.get(&pool).await.unwrap();
        assert_eq!(svc.vector_builds(), 2);
        assert_eq!(r.ids.len(), 3);
    }
}
