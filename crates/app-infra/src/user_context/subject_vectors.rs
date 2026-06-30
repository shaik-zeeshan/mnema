//! Subject Vector store — embedding-free vector persistence for User Context.
//!
//! This module owns the `user_context_subject_vectors` table (migration
//! `0043`): one f32 vector per Subject, serialized as little-endian BLOB bytes,
//! plus a brute-force cosine k-NN over the (~2k) stored vectors. app-infra stays
//! **embedding-free**: the embedding model lives in the Tauri layer and feeds
//! finished vectors in here; this file only stores BLOBs and does pure-f32
//! cosine math — no model imports.
//!
//! `SubjectVectorStore` mirrors [`super::store::UserContextStore`]: it holds a
//! cheap-to-clone [`CaptureDb`] (write + read pools) built from
//! `database.handle()`. Writes route through the Writer Pool (`db.write()`);
//! the owner Reader Pool is `query_only`, so vector reads route through
//! `db.read()`.

use sqlx::Row;

use crate::db::CaptureDb;
use crate::Result;

/// SQLite-backed storage for Subject Vectors (migration `0043`).
///
/// Constructed the same way as [`super::store::UserContextStore`] —
/// `SubjectVectorStore::new(database.handle())` — so the two stores share the
/// same Encrypted Capture Index pools.
#[derive(Clone)]
pub struct SubjectVectorStore {
    db: CaptureDb,
}

/// Serialize an f32 slice to little-endian bytes for BLOB storage.
fn vector_to_blob(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Deserialize little-endian BLOB bytes back into an f32 vector. Trailing bytes
/// that do not complete a 4-byte lane are ignored (well-formed BLOBs never have
/// them — they are always written by [`vector_to_blob`]).
fn blob_to_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Pure-f32 cosine similarity. `None` when the dimensions disagree, either side
/// is empty, or either vector has zero magnitude (undefined cosine).
fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return None;
    }
    Some(dot / (norm_a.sqrt() * norm_b.sqrt()))
}

impl SubjectVectorStore {
    pub fn new(db: CaptureDb) -> Self {
        Self { db }
    }

    /// Insert or replace the vector for `subject`. Keyed on the NOCASE primary
    /// key, so re-embedding the same Subject (any casing) overwrites in place.
    /// `embedded_model` is the active model's `provider/model_id` identity, so a
    /// later model switch can tell stale-model vectors from current ones.
    pub async fn upsert_subject_vector(
        &self,
        subject: &str,
        embedding: &[f32],
        embedded_at_ms: i64,
        embedded_model: &str,
    ) -> Result<()> {
        let blob = vector_to_blob(embedding);
        sqlx::query(
            "INSERT INTO user_context_subject_vectors \
                 (subject, embedding, embedded_at_ms, embedded_model) \
                 VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (subject) DO UPDATE SET \
                 embedding = excluded.embedding, \
                 embedded_at_ms = excluded.embedded_at_ms, \
                 embedded_model = excluded.embedded_model",
        )
        .bind(subject)
        .bind(blob)
        .bind(embedded_at_ms)
        .bind(embedded_model)
        .execute(self.db.write())
        .await?;
        Ok(())
    }

    /// Fetch + deserialize a Subject's vector. `None` if the Subject has no row,
    /// or has a row whose vector is NULL (stale / awaiting (re)embed).
    pub async fn get_subject_vector(&self, subject: &str) -> Result<Option<Vec<f32>>> {
        let row = sqlx::query(
            "SELECT embedding FROM user_context_subject_vectors \
             WHERE subject = ?1 COLLATE NOCASE",
        )
        .bind(subject)
        .fetch_optional(self.db.read())
        .await?;
        Ok(row.and_then(|row| {
            row.get::<Option<Vec<u8>>, _>("embedding")
                .map(|blob| blob_to_vector(&blob))
        }))
    }

    /// Mark a Subject's vector stale: NULL out `embedding`, `embedded_at_ms`, and
    /// `embedded_model` so the backfill worker re-claims it under whatever model
    /// is active next. No-op-safe when the Subject has no row (the UPDATE simply
    /// matches zero rows; a missing Subject is already surfaced by
    /// [`Self::list_subjects_needing_embedding`]).
    pub async fn mark_subject_vector_stale(&self, subject: &str) -> Result<()> {
        sqlx::query(
            "UPDATE user_context_subject_vectors \
                SET embedding = NULL, embedded_at_ms = NULL, embedded_model = NULL \
             WHERE subject = ?1 COLLATE NOCASE",
        )
        .bind(subject)
        .execute(self.db.write())
        .await?;
        Ok(())
    }

    /// Distinct visible Subjects (non-dismissed Conclusions) that still need a
    /// vector under the *active* model: no row at all, a row whose vector is
    /// NULL, a row never tagged with a model, or a row embedded under a
    /// different model than `active_model` (the `provider/model_id` identity).
    /// This is the backfill worker's work queue — so a model switch re-embeds
    /// every stale-model vector. Ordered by `subject` for a deterministic drain;
    /// capped at `limit`.
    pub async fn list_subjects_needing_embedding(
        &self,
        active_model: &str,
        limit: i64,
    ) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT DISTINCT c.subject AS subject \
             FROM user_context_conclusions c \
             LEFT JOIN user_context_subject_vectors v \
                 ON v.subject = c.subject COLLATE NOCASE \
             WHERE c.status != 'dismissed' \
               AND (v.subject IS NULL \
                    OR v.embedding IS NULL \
                    OR v.embedded_model IS NULL \
                    OR v.embedded_model != ?1) \
             ORDER BY c.subject COLLATE NOCASE \
             LIMIT ?2",
        )
        .bind(active_model)
        .bind(limit)
        .fetch_all(self.db.read())
        .await?;
        Ok(rows.iter().map(|row| row.get::<String, _>("subject")).collect())
    }

    /// Brute-force cosine top-`k` over every stored Subject Vector embedded
    /// under the *active* model, in pure f32 Rust. Filtering on
    /// `embedded_model = active_model` excludes stale-model vectors at query
    /// time — closing the same-dimension-different-model garbage-cosine hole even
    /// before the worker re-embeds them. Returns `(subject, cosine_similarity)`
    /// descending by similarity. Fine at the ~2k-subject scale; no ANN index.
    pub async fn subject_vector_knn(
        &self,
        query: &[f32],
        active_model: &str,
        k: usize,
    ) -> Result<Vec<(String, f32)>> {
        let rows = sqlx::query(
            "SELECT subject, embedding FROM user_context_subject_vectors \
             WHERE embedding IS NOT NULL AND embedded_model = ?1",
        )
        .bind(active_model)
        .fetch_all(self.db.read())
        .await?;

        let mut scored: Vec<(String, f32)> = Vec::with_capacity(rows.len());
        for row in rows {
            let subject: String = row.get("subject");
            let blob: Vec<u8> = row.get("embedding");
            let vector = blob_to_vector(&blob);
            if let Some(similarity) = cosine_similarity(query, &vector) {
                scored.push((subject, similarity));
            }
        }
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
        Ok(scored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::CaptureDb;
    use sqlx::sqlite::SqlitePoolOptions;

    /// The active model identity used across most tests (the `provider/model_id`
    /// string the Tauri layer composes).
    const MODEL: &str = "mnema/nomic-embed-text-v1.5";

    /// Run an async test body on a current-thread runtime (the crate's `tokio`
    /// dep does not enable the `macros` feature; mirrors `store.rs`'s pattern).
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    /// An in-memory store with just the tables this module touches: the
    /// Conclusions table (drained by `list_subjects_needing_embedding`) and the
    /// `0043`/`0044` Subject Vectors table.
    async fn test_store() -> SubjectVectorStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory db should open");
        for statement in [
            "CREATE TABLE user_context_conclusions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                subject TEXT NOT NULL,
                statement TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 0.5,
                status TEXT NOT NULL DEFAULT 'visible',
                formed_at_ms INTEGER NOT NULL DEFAULT 0,
                last_supported_at_ms INTEGER NOT NULL DEFAULT 0,
                updated_at_ms INTEGER NOT NULL DEFAULT 0,
                created_at_ms INTEGER NOT NULL DEFAULT 0
            )",
            // Mirrors migrations 0043 + 0044.
            "CREATE TABLE user_context_subject_vectors (
                subject TEXT PRIMARY KEY COLLATE NOCASE,
                embedding BLOB,
                embedded_at_ms INTEGER,
                embedded_model TEXT
            )",
        ] {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("create table");
        }
        SubjectVectorStore::new(CaptureDb::single(pool))
    }

    async fn seed_conclusion(store: &SubjectVectorStore, subject: &str) {
        sqlx::query(
            "INSERT INTO user_context_conclusions (subject, statement, status) \
             VALUES (?1, 'because', 'visible')",
        )
        .bind(subject)
        .execute(store.db.write())
        .await
        .expect("seed conclusion");
    }

    #[test]
    fn upsert_then_get_round_trips_vector() {
        block_on(async {
            let store = test_store().await;
            let vector = vec![0.5f32, -1.25, 3.0, 0.0, 42.42];
            store
                .upsert_subject_vector("Rust", &vector, 1_000, MODEL)
                .await
                .expect("upsert");

            let fetched = store
                .get_subject_vector("Rust")
                .await
                .expect("get")
                .expect("vector present");
            assert_eq!(fetched, vector);

            // NOCASE primary key: different casing resolves to the same row.
            let fetched_nocase = store
                .get_subject_vector("rust")
                .await
                .expect("get nocase")
                .expect("vector present nocase");
            assert_eq!(fetched_nocase, vector);

            // Upsert overwrites in place.
            let replacement = vec![9.0f32, 8.0, 7.0];
            store
                .upsert_subject_vector("rust", &replacement, 2_000, MODEL)
                .await
                .expect("re-upsert");
            let after = store
                .get_subject_vector("Rust")
                .await
                .expect("get after")
                .expect("vector present after");
            assert_eq!(after, replacement);
        });
    }

    #[test]
    fn mark_stale_nulls_vector_and_get_returns_none() {
        block_on(async {
            let store = test_store().await;
            store
                .upsert_subject_vector("Tokio", &[1.0, 2.0, 3.0], 1_000, MODEL)
                .await
                .expect("upsert");
            assert!(store
                .get_subject_vector("Tokio")
                .await
                .expect("get")
                .is_some());

            store
                .mark_subject_vector_stale("Tokio")
                .await
                .expect("mark stale");
            assert!(store
                .get_subject_vector("Tokio")
                .await
                .expect("get after stale")
                .is_none());

            // No-op-safe when the Subject has no row.
            store
                .mark_subject_vector_stale("DoesNotExist")
                .await
                .expect("mark stale missing is a no-op");
        });
    }

    #[test]
    fn list_subjects_needing_embedding_returns_unembedded_visible_subjects() {
        block_on(async {
            let store = test_store().await;
            seed_conclusion(&store, "Alpha").await;
            seed_conclusion(&store, "Beta").await;
            seed_conclusion(&store, "Gamma").await;

            // Dismissed Subjects are excluded.
            sqlx::query(
                "INSERT INTO user_context_conclusions (subject, statement, status) \
                 VALUES ('Dismissed', 'x', 'dismissed')",
            )
            .execute(store.db.write())
            .await
            .expect("seed dismissed");

            // Beta has a fresh vector under the active model → excluded. Gamma's
            // vector is NULL (stale) → still listed.
            store
                .upsert_subject_vector("Beta", &[1.0, 0.0], 1_000, MODEL)
                .await
                .expect("embed Beta");
            store
                .upsert_subject_vector("Gamma", &[0.0, 1.0], 1_000, MODEL)
                .await
                .expect("embed Gamma");
            store
                .mark_subject_vector_stale("Gamma")
                .await
                .expect("stale Gamma");

            let pending = store
                .list_subjects_needing_embedding(MODEL, 10)
                .await
                .expect("list");
            assert_eq!(pending, vec!["Alpha".to_string(), "Gamma".to_string()]);

            // Limit is honored.
            let limited = store
                .list_subjects_needing_embedding(MODEL, 1)
                .await
                .expect("list limited");
            assert_eq!(limited, vec!["Alpha".to_string()]);
        });
    }

    #[test]
    fn list_subjects_needing_embedding_includes_vectors_under_a_different_model() {
        block_on(async {
            let store = test_store().await;
            seed_conclusion(&store, "Delta").await;
            // Delta was embedded under an OLD model. With a different active model
            // string, it must be re-listed for re-embedding even though it has a
            // non-NULL vector.
            store
                .upsert_subject_vector("Delta", &[1.0, 0.0], 1_000, "mnema/old-model")
                .await
                .expect("embed Delta under old model");

            let pending = store
                .list_subjects_needing_embedding(MODEL, 10)
                .await
                .expect("list");
            assert_eq!(pending, vec!["Delta".to_string()]);

            // Under its OWN model it is current → not listed.
            let pending_same = store
                .list_subjects_needing_embedding("mnema/old-model", 10)
                .await
                .expect("list same model");
            assert!(pending_same.is_empty());
        });
    }

    #[test]
    fn upsert_round_trips_embedded_model_and_knn_queries_by_it() {
        block_on(async {
            let store = test_store().await;
            store
                .upsert_subject_vector("Iron", &[1.0, 0.0], 5_000, MODEL)
                .await
                .expect("upsert");

            // The stored row is queryable by its model identity.
            let under_active = store
                .subject_vector_knn(&[1.0, 0.0], MODEL, 5)
                .await
                .expect("knn active");
            assert_eq!(under_active.len(), 1);
            assert_eq!(under_active[0].0, "Iron");

            // ...and absent under any other model identity.
            let under_other = store
                .subject_vector_knn(&[1.0, 0.0], "mnema/other", 5)
                .await
                .expect("knn other");
            assert!(under_other.is_empty());
        });
    }

    #[test]
    fn knn_returns_correct_descending_order() {
        block_on(async {
            let store = test_store().await;
            // Query points along +x. Cosine ignores magnitude.
            store
                .upsert_subject_vector("exact", &[1.0, 0.0], 1, MODEL)
                .await
                .expect("upsert exact");
            store
                .upsert_subject_vector("scaled", &[10.0, 0.0], 1, MODEL)
                .await
                .expect("upsert scaled");
            store
                .upsert_subject_vector("diagonal", &[1.0, 1.0], 1, MODEL)
                .await
                .expect("upsert diagonal");
            store
                .upsert_subject_vector("orthogonal", &[0.0, 1.0], 1, MODEL)
                .await
                .expect("upsert orthogonal");
            store
                .upsert_subject_vector("opposite", &[-1.0, 0.0], 1, MODEL)
                .await
                .expect("upsert opposite");
            // NULL-vector rows are skipped by the k-NN.
            store
                .upsert_subject_vector("stale", &[1.0, 0.0], 1, MODEL)
                .await
                .expect("upsert stale");
            store
                .mark_subject_vector_stale("stale")
                .await
                .expect("stale");

            let results = store
                .subject_vector_knn(&[1.0, 0.0], MODEL, 3)
                .await
                .expect("knn");

            assert_eq!(results.len(), 3);
            let subjects: Vec<&str> = results.iter().map(|(s, _)| s.as_str()).collect();
            // exact and scaled both cosine == 1.0; diagonal ~0.707 is third.
            assert!(subjects.contains(&"exact"));
            assert!(subjects.contains(&"scaled"));
            assert_eq!(subjects[2], "diagonal");

            // Descending by similarity.
            for pair in results.windows(2) {
                assert!(pair[0].1 >= pair[1].1, "results must be descending");
            }
            assert!((results[0].1 - 1.0).abs() < 1e-6);
            assert!((results[2].1 - (0.5f32).sqrt()).abs() < 1e-5);
        });
    }

    #[test]
    fn knn_skips_dimension_mismatches() {
        block_on(async {
            let store = test_store().await;
            store
                .upsert_subject_vector("two_d", &[1.0, 0.0], 1, MODEL)
                .await
                .expect("upsert 2d");
            store
                .upsert_subject_vector("three_d", &[1.0, 0.0, 0.0], 1, MODEL)
                .await
                .expect("upsert 3d");

            let results = store
                .subject_vector_knn(&[1.0, 0.0], MODEL, 5)
                .await
                .expect("knn");
            // Only the matching-dimension vector survives.
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].0, "two_d");
        });
    }

    #[test]
    fn knn_excludes_vectors_embedded_under_a_different_model() {
        block_on(async {
            let store = test_store().await;
            // Same dimension, same direction — but embedded under different model
            // strings. Only the active-model vector may rank (the same-dimension-
            // different-model garbage-cosine hole is closed at query time).
            store
                .upsert_subject_vector("current", &[1.0, 0.0], 1, MODEL)
                .await
                .expect("upsert current");
            store
                .upsert_subject_vector("legacy", &[1.0, 0.0], 1, "mnema/old-model")
                .await
                .expect("upsert legacy");

            let results = store
                .subject_vector_knn(&[1.0, 0.0], MODEL, 5)
                .await
                .expect("knn");
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].0, "current");
        });
    }
}
