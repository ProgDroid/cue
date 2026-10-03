//! Read/write `title_embeddings`. Vectors are stored as little-endian f32 bytes.

use sqlx::SqlitePool;

/// Serialize a vector to little-endian f32 bytes for a BLOB column.
#[must_use]
pub fn encode(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Deserialize little-endian f32 bytes back to a vector. Trailing partial
/// chunks (should never happen) are ignored.
#[must_use]
pub fn decode(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}

/// Insert or replace the embedding for a title under a given model.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn upsert(
    pool: &SqlitePool,
    title_id: i64,
    vector: &[f32],
    model: &str,
) -> anyhow::Result<()> {
    let dims = i64::try_from(vector.len()).unwrap_or(0);
    sqlx::query(
        "INSERT INTO title_embeddings (title_id, vector, model, dims)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(title_id) DO UPDATE SET vector = excluded.vector,
             model = excluded.model, dims = excluded.dims",
    )
    .bind(title_id)
    .bind(encode(vector))
    .bind(model)
    .bind(dims)
    .execute(pool)
    .await?;
    Ok(())
}

/// Title ids that have no embedding row for `model`.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn missing_for_model(pool: &SqlitePool, model: &str) -> anyhow::Result<Vec<i64>> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT t.id FROM titles t
         LEFT JOIN title_embeddings e ON e.title_id = t.id AND e.model = ?
         WHERE e.title_id IS NULL
         ORDER BY t.id",
    )
    .bind(model)
    .fetch_all(pool)
    .await?;
    Ok(ids)
}

/// Every `(title_id, vector)` stored for `model`.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn load_all(pool: &SqlitePool, model: &str) -> anyhow::Result<Vec<(i64, Vec<f32>)>> {
    let rows: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT title_id, vector FROM title_embeddings WHERE model = ? ORDER BY title_id",
    )
    .bind(model)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id, b)| (id, decode(&b))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{init_pool, seed::seed_if_empty};

    fn round_trip(v: &[f32]) -> Vec<f32> {
        decode(&encode(v))
    }

    #[test]
    fn encode_decode_round_trips() {
        let v = vec![0.0, -1.5, 3.25, 1024.0];
        assert_eq!(round_trip(&v), v);
    }

    async fn seeded() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        seed_if_empty(&pool).await.unwrap();
        (pool, dir)
    }

    #[tokio::test]
    async fn missing_then_upsert_then_load() {
        let (pool, _dir) = seeded().await;
        let model = "test-model";

        let missing = missing_for_model(&pool, model).await.unwrap();
        assert_eq!(missing.len(), 28, "all seed titles start unembedded");

        upsert(&pool, missing[0], &[0.1, 0.2, 0.3], model)
            .await
            .unwrap();

        let still_missing = missing_for_model(&pool, model).await.unwrap();
        assert_eq!(still_missing.len(), 27);

        let loaded = load_all(&pool, model).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0, missing[0]);
        assert_eq!(loaded[0].1, vec![0.1, 0.2, 0.3]);
    }

    #[tokio::test]
    async fn upsert_replaces_existing() {
        let (pool, _dir) = seeded().await;
        let id = missing_for_model(&pool, "m").await.unwrap()[0];
        upsert(&pool, id, &[1.0], "m").await.unwrap();
        upsert(&pool, id, &[2.0, 2.0], "m").await.unwrap();
        let loaded = load_all(&pool, "m").await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].1, vec![2.0, 2.0]);
    }
}
