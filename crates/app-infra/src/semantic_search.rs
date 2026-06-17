//! **Semantic Index Backfill** store seam: the query that finds **Search Result
//! Anchor**s lacking a **Semantic Search Vector** and the persistence that stores
//! a derived vector.
//!
//! The embedding model work itself (loading a **Semantic Search Model**, deriving
//! the vector with fastembed) lives in the desktop layer / `semantic-search`
//! crate — app-infra deliberately takes no `ort`/`fastembed` dependency, exactly
//! as it takes no `ai-runtime` for User Context. This module owns only the SQL:
//!
//! - [`SemanticSearchStore::anchors_missing_vector`] — one query selecting
//!   `direct` anchors that have searchable `body_text` but no `vec0` row, ordered
//!   newest-first so the worker drains fresh capture before historical backlog
//!   (ADR 0036). The `direct`-only filter is the whole dedup policy: an
//!   `equivalent_reuse` anchor reuses its group's vector, so it is never selected.
//! - [`SemanticSearchStore::store_vector`] — write one **Semantic Search Vector**
//!   into the `search_document_vectors` vec0 table keyed to `search_documents.id`.
//!
//! Resumability is structural: progress lives entirely in the DB (the presence or
//! absence of a `vec0` row), never in worker memory. A restart mid-backfill
//! continues from exactly where the rows say it is — an already-vectored anchor is
//! filtered out by the `NOT IN` sub-select, and a reprocessed anchor (delete +
//! reinsert with a new id, dropping the old vec0 row via the slice-1 `AFTER
//! DELETE` trigger) reappears in the query automatically.

use sqlx::{Row, SqlitePool};

use crate::Result;

/// One **Search Result Anchor** that needs a **Semantic Search Vector**: its
/// `search_documents.id` (which is also its `vec0` rowid) and the raw `body_text`
/// to embed. Raw, not redacted: the vector lives inside the **Encrypted Capture
/// Index** at the same exposure as the FTS5 projection, so it embeds the same raw
/// body text Text Search already indexes (ADR 0036); redaction is enforced at
/// any egress boundary, never before embedding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorMissingVector {
    /// `search_documents.id`, used as the `vec0` rowid when storing the vector.
    pub anchor_id: i64,
    /// The raw `body_text` of the anchor (the embedding input).
    pub body_text: String,
}

/// Store seam for the **Semantic Index Backfill** worker.
#[derive(Clone)]
pub struct SemanticSearchStore {
    pool: SqlitePool,
}

impl SemanticSearchStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Select up to `limit` `direct` **Search Result Anchor**s that have
    /// searchable text but no **Semantic Search Vector** yet, newest-first.
    ///
    /// Newest-first (`absolute_start_at DESC, id DESC`) is the ADR-0036 ordering:
    /// freshly captured anchors preempt the historical backlog, which is drained
    /// from the newest end backward. Only `text_source_kind = 'direct'` rows are
    /// considered — an `equivalent_reuse` anchor reuses its group's vector, so
    /// structural frame dedup already collapses the count with no separate
    /// admission pass.
    ///
    /// The `NOT IN (SELECT rowid FROM search_document_vectors)` anti-join is what
    /// makes the sweep self-healing and resumable: any anchor already vectored is
    /// filtered out, so the same query covers live capture, historical backfill,
    /// and resume-after-restart in one pass.
    pub async fn anchors_missing_vector(&self, limit: i64) -> Result<Vec<AnchorMissingVector>> {
        let rows = sqlx::query(
            "SELECT search_documents.id AS id, search_documents.body_text AS body_text \
             FROM search_documents \
             WHERE search_documents.text_source_kind = 'direct' \
               AND search_documents.id NOT IN (\
                   SELECT rowid FROM search_document_vectors\
               ) \
             ORDER BY search_documents.absolute_start_at DESC, search_documents.id DESC \
             LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| AnchorMissingVector {
                anchor_id: row.get("id"),
                body_text: row.get("body_text"),
            })
            .collect())
    }

    /// Whether the `direct` **Search Result Anchor** `anchor_id` still exists and
    /// still lacks a **Semantic Search Vector**. The worker re-checks this just
    /// before storing so a vector derived from text that was deleted (retention /
    /// Delete Recent) mid-embed is never inserted as an orphan, and a concurrent
    /// reprocess that replaced the anchor id is not clobbered.
    pub async fn anchor_still_missing_vector(&self, anchor_id: i64) -> Result<bool> {
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT 1 \
             FROM search_documents \
             WHERE search_documents.id = ?1 \
               AND search_documents.text_source_kind = 'direct' \
               AND search_documents.id NOT IN (\
                   SELECT rowid FROM search_document_vectors\
               ) \
             LIMIT 1",
        )
        .bind(anchor_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(exists.is_some())
    }

    /// Store one **Semantic Search Vector** for `anchor_id` into the
    /// `search_document_vectors` vec0 table. `vector` is the model's f32 output;
    /// it is serialized little-endian, the byte layout vec0 expects.
    ///
    /// Uses `INSERT OR REPLACE` so a reprocess that re-derives the same anchor id
    /// overwrites cleanly. In the normal sweep the anchor has no row yet (the
    /// query already filtered vectored anchors out), so this is an insert; the
    /// `OR REPLACE` is belt-and-braces for the reprocess race the re-check above
    /// guards.
    pub async fn store_vector(&self, anchor_id: i64, vector: &[f32]) -> Result<()> {
        let blob = vector_to_le_bytes(vector);
        sqlx::query(
            "INSERT OR REPLACE INTO search_document_vectors (rowid, embedding) VALUES (?1, ?2)",
        )
        .bind(anchor_id)
        .bind(blob)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Drop and recreate the `search_document_vectors` vec0 table at `dimension`,
    /// returning the number of **Semantic Search Vector**s discarded.
    ///
    /// Used by the Settings re-index: switching the **Semantic Search Model
    /// Tier** produces incomparable vectors, so the whole index is re-derived.
    /// Critically, a switch can also change the vector *dimension* (e.g. 768-dim
    /// `nomic` → 1024-dim `bge-m3`), and `vec0` is a fixed-dimension virtual
    /// table — so the table must be rebuilt at the new model's dimension, not
    /// merely have its rows cleared, or the worker's first store under the new
    /// model would fail on a length mismatch. Recreating it re-exposes every
    /// `direct` anchor to [`anchors_missing_vector`], so the sweep backfills them
    /// under the new model (newest-first) with no in-memory state (ADR 0036). The
    /// `AFTER DELETE` trigger keys off the table *name*, so it stays valid across
    /// the recreate.
    pub async fn recreate_vectors_table(&self, dimension: usize) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
        let previous: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM search_document_vectors")
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query("DROP TABLE IF EXISTS search_document_vectors")
            .execute(&mut *tx)
            .await?;
        // `dimension` is a usize from the in-tree model catalog, never user input.
        sqlx::query(&format!(
            "CREATE VIRTUAL TABLE search_document_vectors USING vec0(embedding float[{dimension}])"
        ))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(u64::try_from(previous).unwrap_or(0))
    }

    /// Count of `direct` anchors still lacking a vector — the backlog size, used
    /// only for logging the sweep's progress (never a control signal).
    pub async fn count_anchors_missing_vector(&self) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) \
             FROM search_documents \
             WHERE search_documents.text_source_kind = 'direct' \
               AND search_documents.id NOT IN (\
                   SELECT rowid FROM search_document_vectors\
               )",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }
}

/// Serialize an f32 vector to the little-endian byte BLOB vec0 stores. The one
/// canonical serializer — the query path (`search.rs`) and the `db.rs`
/// round-trip test call this rather than re-implementing the byte layout.
pub(crate) fn vector_to_le_bytes(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::{
        AppInfra, NewFrame, ProcessingJob, ProcessingJobDraft, ProcessingResultDraft,
    };

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir(name: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("mnema-semantic-{name}-{}-{id}", std::process::id()))
    }

    fn run_async_test(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

    async fn complete_job(infra: &AppInfra, job: ProcessingJob, result: ProcessingResultDraft) {
        let running = infra
            .claim_queued_processing_job(job.id)
            .await
            .expect("job should claim")
            .expect("job should exist");
        infra
            .complete_processing_job(running.id, &result)
            .await
            .expect("job should complete");
    }

    /// Insert a frame at `captured_at`, OCR it with `text`, and return the frame id.
    /// The completed OCR projects a `direct` search_documents anchor on write.
    async fn seed_frame_with_text(infra: &AppInfra, captured_at: &str, text: &str) -> i64 {
        let frame = infra
            .insert_frame(&NewFrame::new(
                "screen-session",
                &format!("/tmp/semantic-{captured_at}.jpg"),
                captured_at,
            ))
            .await
            .expect("frame should insert");
        let job = infra
            .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
            .await
            .expect("ocr job should enqueue");
        complete_job(
            infra,
            job,
            ProcessingResultDraft::new().with_result_text(text),
        )
        .await;
        frame.id
    }

    /// A unit-length f32 vector of the right dimension whose direction encodes a
    /// tag, so KNN can later distinguish stored vectors if needed.
    fn unit_vector(dim: usize, seed: f32) -> Vec<f32> {
        let mut v = vec![0.0_f32; dim];
        v[0] = 1.0;
        v[dim - 1] = seed;
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        for x in v.iter_mut() {
            *x /= norm;
        }
        v
    }

    #[test]
    fn selects_only_direct_anchors_without_a_vector_newest_first() {
        run_async_test(async {
            let dir = test_dir("select-newest-first");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            // Three direct anchors at increasing capture times.
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "oldest body text").await;
            seed_frame_with_text(&infra, "2026-05-17T10:05:00Z", "middle body text").await;
            seed_frame_with_text(&infra, "2026-05-17T10:10:00Z", "newest body text").await;

            let store = infra.semantic_search();
            let missing = store
                .anchors_missing_vector(10)
                .await
                .expect("query should succeed");

            // All three direct anchors are returned, newest capture time first.
            assert_eq!(missing.len(), 3);
            assert_eq!(missing[0].body_text, "newest body text");
            assert_eq!(missing[1].body_text, "middle body text");
            assert_eq!(missing[2].body_text, "oldest body text");
        });
    }

    #[test]
    fn ignores_equivalent_reuse_anchors() {
        run_async_test(async {
            let dir = test_dir("ignore-equivalent");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            // Two equivalent frames; the first gets OCR (direct), the second
            // reuses it (equivalent_reuse) — only ONE direct anchor exists.
            let first = infra
                .insert_frame(
                    &NewFrame::new("screen-session", "/tmp/equiv-a.jpg", "2026-05-17T10:00:00Z")
                        .with_equivalence(crate::FrameEquivalence {
                            hint: Some("same-screen".to_string()),
                            proof: Some(vec![0; 1024]),
                            version: Some(1),
                            status: Some(crate::FrameEquivalenceStatus::Ready),
                            error: None,
                        }),
                )
                .await
                .expect("first frame inserts");
            infra
                .insert_frame(
                    &NewFrame::new("screen-session", "/tmp/equiv-b.jpg", "2026-05-17T10:00:02Z")
                        .with_equivalence(crate::FrameEquivalence {
                            hint: Some("same-screen".to_string()),
                            proof: Some(vec![0; 1024]),
                            version: Some(1),
                            status: Some(crate::FrameEquivalenceStatus::Ready),
                            error: None,
                        }),
                )
                .await
                .expect("second frame inserts");

            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(first.id))
                .await
                .expect("ocr job enqueues");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("shared screen text"),
            )
            .await;

            let store = infra.semantic_search();
            let missing = store
                .anchors_missing_vector(10)
                .await
                .expect("query should succeed");

            // Exactly one anchor needs a vector: the direct one. The
            // equivalent_reuse anchor reuses the group's vector and is excluded.
            assert_eq!(missing.len(), 1);
            assert_eq!(missing[0].anchor_id, first.id);
        });
    }

    #[test]
    fn store_vector_removes_anchor_from_the_missing_set() {
        run_async_test(async {
            let dir = test_dir("store-clears-missing");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "vectorize me").await;

            let store = infra.semantic_search();
            let missing = store
                .anchors_missing_vector(10)
                .await
                .expect("query succeeds");
            assert_eq!(missing.len(), 1);
            let anchor = &missing[0];

            store
                .store_vector(anchor.anchor_id, &unit_vector(768, 0.25))
                .await
                .expect("vector stores");

            // The anchor is no longer in the missing set: progress lives in the DB.
            let after = store
                .anchors_missing_vector(10)
                .await
                .expect("query succeeds");
            assert!(after.is_empty());
            assert_eq!(
                store
                    .count_anchors_missing_vector()
                    .await
                    .expect("count succeeds"),
                0
            );
            assert!(!store
                .anchor_still_missing_vector(anchor.anchor_id)
                .await
                .expect("recheck succeeds"));
        });
    }

    #[test]
    fn recreating_the_vector_table_re_exposes_every_anchor_for_re_index() {
        run_async_test(async {
            let dir = test_dir("recreate-reindex");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            // Three direct anchors, all vectored under the current model.
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;
            seed_frame_with_text(&infra, "2026-05-17T10:05:00Z", "bravo").await;
            seed_frame_with_text(&infra, "2026-05-17T10:10:00Z", "charlie").await;

            let store = infra.semantic_search();
            for anchor in store.anchors_missing_vector(10).await.expect("query") {
                store
                    .store_vector(anchor.anchor_id, &unit_vector(768, 0.5))
                    .await
                    .expect("store vector");
            }
            assert!(store
                .anchors_missing_vector(10)
                .await
                .expect("query")
                .is_empty());

            // A model switch rebuilds the whole index; every anchor re-appears in
            // the missing set so the sweep re-derives it under the new model.
            let removed = store
                .recreate_vectors_table(768)
                .await
                .expect("recreate succeeds");
            assert_eq!(removed, 3);
            assert_eq!(
                store
                    .count_anchors_missing_vector()
                    .await
                    .expect("count succeeds"),
                3
            );
            assert_eq!(
                store.anchors_missing_vector(10).await.expect("query").len(),
                3
            );
        });
    }

    #[test]
    fn recreating_at_a_new_dimension_swaps_the_vector_column() {
        run_async_test(async {
            let dir = test_dir("recreate-new-dimension");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            // One anchor, vectored under the 768-dim default tier.
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;
            let store = infra.semantic_search();
            let anchor = store.anchors_missing_vector(10).await.expect("query")[0].clone();
            store
                .store_vector(anchor.anchor_id, &unit_vector(768, 0.5))
                .await
                .expect("store 768-dim vector");

            // Switch to a 1024-dim tier (e.g. bge-m3): recreate at the new
            // dimension. The old vector is discarded and the column now accepts a
            // 1024-dim vector that the fixed float[768] table would have rejected.
            let removed = store
                .recreate_vectors_table(1024)
                .await
                .expect("recreate at 1024 succeeds");
            assert_eq!(removed, 1, "the single 768-dim vector is discarded");

            store
                .store_vector(anchor.anchor_id, &unit_vector(1024, 0.5))
                .await
                .expect("a 1024-dim vector now stores into the rebuilt table");

            // The AFTER DELETE trigger survived the recreate: it still drops the
            // matching vec0 row, so the anchor leaves the missing set.
            assert!(!store
                .anchor_still_missing_vector(anchor.anchor_id)
                .await
                .expect("recheck succeeds"));
        });
    }

    #[test]
    fn resumes_after_restart_without_re_embedding_or_dropping() {
        run_async_test(async {
            let dir = test_dir("resume");
            let store_dim = 768;

            // Seed three direct anchors, then vectorize the two newest (simulating
            // a sweep interrupted with one anchor still pending).
            let remaining_id;
            {
                let infra = AppInfra::initialize(&dir)
                    .await
                    .expect("infra should initialize");
                seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;
                seed_frame_with_text(&infra, "2026-05-17T10:05:00Z", "bravo").await;
                seed_frame_with_text(&infra, "2026-05-17T10:10:00Z", "charlie").await;

                let store = infra.semantic_search();
                let missing = store.anchors_missing_vector(10).await.expect("query");
                assert_eq!(missing.len(), 3);
                // Drain the two newest, leave the oldest unvectored.
                store
                    .store_vector(missing[0].anchor_id, &unit_vector(store_dim, 0.1))
                    .await
                    .expect("store newest");
                store
                    .store_vector(missing[1].anchor_id, &unit_vector(store_dim, 0.2))
                    .await
                    .expect("store middle");
                remaining_id = missing[2].anchor_id;
                drop(infra);
            }

            // Reopen the DB: the sweep must continue from DB state — exactly the
            // one un-vectored anchor remains, and the two already-vectored are not
            // re-selected.
            let reopened = AppInfra::initialize(&dir)
                .await
                .expect("infra should reopen");
            let store = reopened.semantic_search();
            let missing = store.anchors_missing_vector(10).await.expect("query");
            assert_eq!(missing.len(), 1, "only the un-vectored anchor resumes");
            assert_eq!(missing[0].anchor_id, remaining_id);
            assert_eq!(missing[0].body_text, "alpha");
        });
    }

    #[test]
    fn reprocessing_an_anchor_re_enqueues_it_for_a_replacement_vector() {
        run_async_test(async {
            let dir = test_dir("reprocess-replaces");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame_id = seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "original").await;

            let store = infra.semantic_search();
            let missing = store.anchors_missing_vector(10).await.expect("query");
            assert_eq!(missing.len(), 1);
            let original_anchor_id = missing[0].anchor_id;
            store
                .store_vector(original_anchor_id, &unit_vector(768, 0.3))
                .await
                .expect("store original vector");
            assert!(store
                .anchors_missing_vector(10)
                .await
                .expect("query")
                .is_empty());

            // Reprocess the frame: a new OCR result replaces the search projection
            // (delete + reinsert with a NEW id). The slice-1 AFTER DELETE trigger
            // drops the old vec0 row, so the new anchor reappears in the sweep.
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame_id))
                .await
                .expect("reprocess ocr job enqueues");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("reprocessed text"),
            )
            .await;

            let missing = store.anchors_missing_vector(10).await.expect("query");
            assert_eq!(
                missing.len(),
                1,
                "the reprocessed anchor needs a replacement vector"
            );
            assert_eq!(missing[0].body_text, "reprocessed text");
            assert_ne!(
                missing[0].anchor_id, original_anchor_id,
                "reprocessing reinserts the projection with a new id"
            );

            // Completing the new embedding stores the replacement vector.
            store
                .store_vector(missing[0].anchor_id, &unit_vector(768, 0.4))
                .await
                .expect("store replacement vector");
            assert!(store
                .anchors_missing_vector(10)
                .await
                .expect("query")
                .is_empty());
        });
    }

    #[test]
    fn deleting_an_anchor_mid_embed_is_caught_by_the_recheck() {
        run_async_test(async {
            let dir = test_dir("delete-mid-embed");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "soon to be deleted").await;

            let store = infra.semantic_search();
            let missing = store.anchors_missing_vector(10).await.expect("query");
            let anchor_id = missing[0].anchor_id;

            // The anchor (and its body text) disappears while the worker is mid-embed.
            sqlx::query("DELETE FROM search_documents WHERE id = ?1")
                .bind(anchor_id)
                .execute(infra.pool())
                .await
                .expect("anchor deletes");

            // The re-check guards against storing an orphan vector.
            assert!(!store
                .anchor_still_missing_vector(anchor_id)
                .await
                .expect("recheck succeeds"));
        });
    }
}
