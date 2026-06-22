//! Pure vector math for retrieval and the refine chips. No I/O, no async.

/// Dot product of two equal-length vectors. Shorter length wins if they differ.
#[must_use]
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Cosine similarity in [-1, 1]; 0.0 if either vector has zero magnitude.
#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let na = dot(a, a).sqrt();
    let nb = dot(b, b).sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot(a, b) / (na * nb)
}

/// A direction vector = `light - dark`, elementwise. Used to score "lightness".
#[must_use]
pub fn axis(light: &[f32], dark: &[f32]) -> Vec<f32> {
    light.iter().zip(dark).map(|(l, d)| l - d).collect()
}

/// Mean vector of a set. Empty input yields an empty vector.
#[must_use]
pub fn centroid(vectors: &[Vec<f32>]) -> Vec<f32> {
    let Some(first) = vectors.first() else {
        return Vec::new();
    };
    let mut acc = vec![0.0_f32; first.len()];
    for v in vectors {
        for (a, x) in acc.iter_mut().zip(v) {
            *a += x;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    // len() fits in f32 mantissa for any realistic vector count
    let n = vectors.len() as f32;
    for a in &mut acc {
        *a /= n;
    }
    acc
}

/// Top-`n` item ids by descending cosine similarity to `query`.
#[must_use]
pub fn rank_by_cosine(query: &[f32], items: &[(i64, Vec<f32>)], top_n: usize) -> Vec<i64> {
    let mut scored: Vec<(i64, f32)> = items
        .iter()
        .map(|(id, v)| (*id, cosine(query, v)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.into_iter().take(top_n).map(|(id, _)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_of_identical_vectors_is_one() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_handles_zero_vector() {
        assert!((cosine(&[0.0, 0.0], &[1.0, 1.0])).abs() < 1e-6);
    }

    #[test]
    fn axis_is_elementwise_difference() {
        assert_eq!(axis(&[1.0, 1.0], &[0.0, 2.0]), vec![1.0, -1.0]);
    }

    #[test]
    fn centroid_averages_componentwise() {
        assert_eq!(centroid(&[vec![0.0, 0.0], vec![2.0, 4.0]]), vec![1.0, 2.0]);
    }

    #[test]
    fn rank_orders_by_similarity_and_caps() {
        let q = vec![1.0, 0.0];
        let items = vec![
            (1, vec![0.0, 1.0]), // orthogonal
            (2, vec![1.0, 0.0]), // identical
            (3, vec![1.0, 0.1]), // close
        ];
        assert_eq!(rank_by_cosine(&q, &items, 2), vec![2, 3]);
    }
}
