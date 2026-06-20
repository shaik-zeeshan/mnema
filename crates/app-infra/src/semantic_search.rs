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

use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

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
    /// `search_document_vectors` vec0 table, **conditioned on the `direct`
    /// `search_documents` row still existing**. `vector` is the model's f32
    /// output; it is serialized little-endian, the byte layout vec0 expects.
    /// Returns whether a row was actually written.
    ///
    /// The insert is a single atomic `INSERT OR REPLACE … SELECT … WHERE` over
    /// `search_documents`: the rowid and the existence predicate are evaluated in
    /// the same statement, so there is **no re-check-then-store gap**. If a
    /// retention / Delete Recent cascade removed the anchor between the worker's
    /// embed and this store (the `AFTER DELETE` trigger having dropped nothing,
    /// since no vec0 row existed yet), the `SELECT` matches zero rows and **no
    /// orphan vector is inserted** — a meaning vector of deleted captured content
    /// can never persist at rest (M1 / privacy concern #6, ADR 0036). The worker's
    /// preceding `anchor_still_missing_vector` re-check is now an optimization, not
    /// the correctness boundary; this statement is.
    ///
    /// `OR REPLACE` still covers the reprocess race (a re-derived anchor id
    /// overwrites cleanly); in the normal sweep the anchor has no row yet, so this
    /// is an insert.
    ///
    /// Rejects a non-finite vector (any `NaN`/`±inf` component) before touching
    /// the table: vec0 stores such a blob silently, but a `NaN` distance sorts
    /// non-deterministically under the KNN order and `anchor_still_missing_vector`
    /// would treat the poisoned vector as done and never retry it (L1). The
    /// in-tree pipeline cannot produce one (every embedding is L2-normalized over
    /// guaranteed-non-empty text), so this is defensive against a corrupt/
    /// pathological ONNX graph only.
    pub async fn store_vector(&self, anchor_id: i64, vector: &[f32]) -> Result<bool> {
        if vector.iter().any(|component| !component.is_finite()) {
            return Err(crate::AppInfraError::InvalidSearchRequest(format!(
                "refusing to store a non-finite Semantic Search Vector for anchor {anchor_id} \
                 (a NaN/inf component would poison KNN ordering)"
            )));
        }
        let blob = vector_to_le_bytes(vector);
        let result = sqlx::query(
            "INSERT OR REPLACE INTO search_document_vectors (rowid, embedding) \
             SELECT search_documents.id, ?2 \
             FROM search_documents \
             WHERE search_documents.id = ?1 \
               AND search_documents.text_source_kind = 'direct'",
        )
        .bind(anchor_id)
        .bind(blob)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Store a vector **only if its length matches the live `vec0` column
    /// dimension**, returning whether it was stored.
    ///
    /// This is the worker-side half of the single dimension authority (the live
    /// `vec0` column width is the one source of truth — see
    /// [`live_vector_dimension`]). The two-step model switch (persist `model_id`,
    /// then recreate the table) is non-atomic across the worker: between the
    /// embedder reloading at the new dimension and the table being rebuilt — and
    /// **permanently** if the rebuild ever fails — an embedded vector would be the
    /// wrong length for the table. A raw [`store_vector`] would have vec0 reject
    /// the blob and the sweep would error-loop that doomed batch every retry
    /// forever. Here a mismatch is a **skip, not an error**: the anchor stays in
    /// the missing set and is re-embedded once the dimensions agree (after the
    /// rebuild lands, or after startup reconciliation self-heals a stuck table),
    /// so the worker idles instead of error-looping.
    ///
    /// `Ok(true)` — stored. `Ok(false)` — skipped: either on a dimension mismatch
    /// (or the table is absent), **or** because the `direct` anchor row no longer
    /// exists (a delete raced the store — [`store_vector`] inserts nothing, so no
    /// orphan is left). `Err` — a non-finite vector (L1) or a real DB failure.
    pub async fn store_vector_if_dimension_matches(
        &self,
        anchor_id: i64,
        vector: &[f32],
    ) -> Result<bool> {
        match self.live_vector_dimension().await? {
            // Length matches the live column: attempt the atomic row-conditioned
            // store. It still returns `false` if the anchor vanished mid-embed, so
            // a delete racing this store leaves nothing behind.
            Some(dimension) if dimension == vector.len() => {
                self.store_vector(anchor_id, vector).await
            }
            // Mismatch or no table: skip without error so the sweep idles rather
            // than error-looping a vector the live column can never accept.
            _ => Ok(false),
        }
    }

    /// The **live `vec0` column dimension** of `search_document_vectors` — the
    /// single source of truth for the active vector width, read straight from the
    /// table definition rather than inferred from the (separately persisted)
    /// selected model.
    ///
    /// Parses the `float[N]` declared in the `CREATE VIRTUAL TABLE … USING
    /// vec0(embedding float[N])` DDL stored in `sqlite_master`. Returns `None`
    /// when the table is absent or its DDL is unexpectedly shaped (treated as
    /// "no usable dimension" — the worker idles and the query path degrades to
    /// keyword-only rather than erroring).
    pub async fn live_vector_dimension(&self) -> Result<Option<usize>> {
        live_vector_dimension(&self.pool).await
    }

    /// Reconcile the live `vec0` table dimension against `expected_dimension`,
    /// recreating the table only when they disagree. Returns `Some(discarded)`
    /// with the number of vectors dropped if a recreate happened, or `None` if
    /// the table already matched (no-op).
    ///
    /// This is the **startup self-heal** for a permanently-stuck switch: if a
    /// model switch persisted a new `model_id` but the table recreate failed (DB
    /// busy under the worker's concurrent writes — `DROP TABLE` needs an exclusive
    /// lock), the table is left at the old dimension while the selection names a
    /// new-dimension model. Both the worker (`store_vector_if_dimension_matches`)
    /// and the query path then read the live column and skip/idle — search never
    /// hard-fails, but the index also never rebuilds. Running this on the
    /// deferred-startup seam with the selected model's expected dimension brings
    /// the table back into agreement so the sweep can backfill under the new
    /// model. Idempotent: a matching table is left untouched.
    pub async fn reconcile_vectors_table(
        &self,
        expected_dimension: usize,
    ) -> Result<Option<u64>> {
        match self.live_vector_dimension().await? {
            Some(dimension) if dimension == expected_dimension => Ok(None),
            // Mismatched OR absent: rebuild at the expected dimension so the
            // worker/query path's live-dimension authority agrees with the
            // selected model again.
            _ => Ok(Some(self.recreate_vectors_table(expected_dimension).await?)),
        }
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
        // Count existing vectors only when the table is actually present: this is
        // also reached from `reconcile_vectors_table`'s "absent → rebuild" self-heal
        // path, where the table is missing — an unguarded `COUNT(*)` would raise
        // "no such table" and abort the very rebuild that path exists to perform. A
        // missing table discarded zero vectors.
        let table_present: Option<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master \
             WHERE type = 'table' AND name = 'search_document_vectors'",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let previous: i64 = if table_present.is_some() {
            sqlx::query_scalar("SELECT COUNT(*) FROM search_document_vectors")
                .fetch_one(&mut *tx)
                .await?
        } else {
            0
        };
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

/// The **Semantic Search** read seam: run the `vec0` KNN nearest to
/// `query_embedding`, **filter-then-rank**, and return the **Semantic Candidate
/// Set** — the in-scope **Search Result Anchor** rowids, nearest-first.
///
/// This is the read-time counterpart to [`SemanticSearchStore::store_vector`]'s
/// write, and it owns everything `vec0`-substrate-specific so the meaning tier's
/// vector format and KNN live in one place beside the write serializer (ADR 0036;
/// a future int8/binary/ANN change is then a single-module edit, not a fusion-SQL
/// edit in `search.rs`). It owns:
///
/// - **Blob serialization** of the f32-LE query vector (via [`vector_to_le_bytes`],
///   the same byte layout the store writes) — `search.rs` never touches the format.
/// - **The KNN SQL**: `embedding MATCH ? AND k = ? AND rowid IN (<subquery>)`.
///   sqlite-vec requires an explicit `k` (or LIMIT); pairing it with the in-scope
///   `rowid IN (…)` set makes this filter-then-rank — the top-k is computed over
///   the refined slice, not post-filtered after ranking the whole corpus (ADR
///   0036; no ANN in v1, so this is a brute-force scan of the filtered set). The
///   `push_scope` callback appends the in-scope rowid sub-select, keeping
///   `push_search_refinement_predicates` (shared with **Text Search**) living in
///   `search.rs`. The seam never takes a materialized id list — only the closure
///   that appends the `rowid IN (<subquery>)` predicate — so filter-then-rank
///   stays a single SQL pass.
/// - **The live-dimension gate**: the query embedder emits a vector sized for the
///   *selected model*, but the `vec0` column only changes when the table is
///   rebuilt. If they disagree (a model switch in flight, or stuck after a failed
///   rebuild) the KNN is skipped and an **empty candidate set** is returned —
///   feeding vec0 a wrong-length blob would error, so gating at the single
///   dimension authority keeps the read off the vec0 error path and lets the
///   degrade-to-keyword wrapper in `search.rs` see a clean empty list.
///
/// Returns **order-only** `Vec<i64>` (rank-only, no distance): **Hybrid Search**
/// fuses by rank, so list *position* is the entire payload; surfacing a distance
/// would invite the weighted-score fusion ADR 0036 rejected. `Ok(vec![])` on a
/// dimension mismatch (a clean empty set, degrade-to-keyword); `Err` only on a
/// real DB failure (which the wrapper swallows).
pub(crate) async fn knn_in_scope_anchors<F>(
    pool: &SqlitePool,
    query_embedding: &[f32],
    k: i64,
    push_scope: F,
) -> Result<Vec<i64>>
where
    F: FnOnce(&mut QueryBuilder<'_, Sqlite>),
{
    // Live-dimension authority: skip the KNN (returning a clean empty candidate
    // set) on a query-vector/table dimension mismatch, so the read degrades to
    // keyword-only deterministically instead of via a vec0 length error.
    if !live_vector_dimension(pool)
        .await?
        .is_some_and(|dimension| dimension == query_embedding.len())
    {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT search_document_vectors.rowid \
         FROM search_document_vectors \
         WHERE search_document_vectors.embedding MATCH ",
    );
    query.push_bind(vector_to_le_bytes(query_embedding));
    query.push(" AND k = ");
    query.push_bind(k);
    query.push(" AND search_document_vectors.rowid IN (");
    push_scope(&mut query);
    query.push(")");

    // The KNN returns rows ascending by distance, so the rowids come back
    // nearest-first — the Semantic Candidate Set's order is the whole payload.
    let rows = query.build().fetch_all(pool).await?;
    Ok(rows.into_iter().map(|row| row.get::<i64, _>(0)).collect())
}

/// Read the **live `vec0` column dimension** of `search_document_vectors` from a
/// raw pool — the single source of truth for the active vector width. Shared by
/// the store seam ([`SemanticSearchStore::live_vector_dimension`]) and the query
/// path (`search.rs`), which holds only a `&SqlitePool`. Returns `None` when the
/// table is absent or its DDL is unexpectedly shaped (treated as "no usable
/// dimension" — caller idles / degrades to keyword-only rather than erroring).
pub(crate) async fn live_vector_dimension(pool: &SqlitePool) -> Result<Option<usize>> {
    let sql: Option<String> = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master \
         WHERE type = 'table' AND name = 'search_document_vectors'",
    )
    .fetch_optional(pool)
    .await?;
    Ok(sql.as_deref().and_then(parse_vec0_dimension))
}

/// Parse the declared dimension `N` out of a `vec0(embedding float[N])` table
/// DDL (`sqlite_master.sql`). The whole feature keys its dimension authority off
/// this — the recreate writes exactly this shape (see
/// [`SemanticSearchStore::recreate_vectors_table`]), so the parse is the inverse
/// of that format. Returns `None` on any shape it does not recognize, so an
/// unexpected DDL degrades to "no usable dimension" rather than a wrong guess.
fn parse_vec0_dimension(sql: &str) -> Option<usize> {
    // Tolerate arbitrary whitespace/casing around the `float[N]` declaration.
    let lowered = sql.to_ascii_lowercase();
    let open = lowered.find("float[")? + "float[".len();
    let close = lowered[open..].find(']')? + open;
    lowered[open..close].trim().parse::<usize>().ok()
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
    fn live_vector_dimension_reads_the_actual_vec0_column_width() {
        run_async_test(async {
            let dir = test_dir("live-dimension");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let store = infra.semantic_search();

            // The migration ships a float[768] table.
            assert_eq!(
                store.live_vector_dimension().await.expect("dim reads"),
                Some(768)
            );

            // Recreating at a new dimension is reflected immediately by the live
            // read — the single source of truth is the table, not any persisted
            // model selection.
            store
                .recreate_vectors_table(1024)
                .await
                .expect("recreate at 1024");
            assert_eq!(
                store.live_vector_dimension().await.expect("dim reads"),
                Some(1024)
            );
        });
    }

    #[test]
    fn store_vector_skips_a_wrong_dimension_vector_without_erroring() {
        run_async_test(async {
            let dir = test_dir("store-wrong-dim-skips");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "vectorize me").await;

            let store = infra.semantic_search();
            let anchor = store.anchors_missing_vector(10).await.expect("query")[0].clone();

            // The live table is float[768]. A 1024-dim vector (an embedder reloaded
            // at a new dimension before the table was rebuilt — the non-atomic
            // switch window, or a permanently-stuck table) does NOT fatally error:
            // it is skipped (`Ok(false)`), so the worker idles instead of
            // error-looping a doomed batch every 30s.
            let stored = store
                .store_vector_if_dimension_matches(anchor.anchor_id, &unit_vector(1024, 0.5))
                .await
                .expect("a dimension mismatch is a skip, not a fatal error");
            assert!(!stored, "the wrong-dimension vector is skipped");

            // The anchor stays in the missing set: it is re-embedded once the
            // dimensions agree (after the rebuild / startup reconciliation).
            assert!(store
                .anchor_still_missing_vector(anchor.anchor_id)
                .await
                .expect("recheck"));

            // A correctly-sized 768-dim vector stores normally and clears the anchor.
            let stored = store
                .store_vector_if_dimension_matches(anchor.anchor_id, &unit_vector(768, 0.5))
                .await
                .expect("matching dimension stores");
            assert!(stored, "the matching-dimension vector is stored");
            assert!(!store
                .anchor_still_missing_vector(anchor.anchor_id)
                .await
                .expect("recheck"));
        });
    }

    #[test]
    fn reconcile_rebuilds_a_table_whose_dimension_disagrees_with_the_model() {
        run_async_test(async {
            let dir = test_dir("reconcile-mismatch");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;

            let store = infra.semantic_search();
            // Simulate a permanently-stuck state: the live table is float[768] (the
            // migration default) but the selected model now expects 1024 dims (the
            // rebuild failed at switch time, leaving the table at the old width).
            let anchor = store.anchors_missing_vector(10).await.expect("query")[0].clone();
            store
                .store_vector(anchor.anchor_id, &unit_vector(768, 0.5))
                .await
                .expect("store a stale 768-dim vector");
            assert_eq!(store.live_vector_dimension().await.expect("dim"), Some(768));

            // Startup reconciliation against the selected model's expected 1024 dims
            // rebuilds the table (discarding the stale vector) so the live dimension
            // agrees with the model again and the sweep can backfill under it.
            let discarded = store
                .reconcile_vectors_table(1024)
                .await
                .expect("reconcile succeeds");
            assert_eq!(discarded, Some(1), "the stale 768-dim vector is discarded");
            assert_eq!(store.live_vector_dimension().await.expect("dim"), Some(1024));
            // The anchor is re-exposed for re-embedding under the new model.
            assert!(store
                .anchor_still_missing_vector(anchor.anchor_id)
                .await
                .expect("recheck"));
        });
    }

    #[test]
    fn reconcile_is_a_no_op_when_the_dimension_already_matches() {
        run_async_test(async {
            let dir = test_dir("reconcile-match");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;

            let store = infra.semantic_search();
            let anchor = store.anchors_missing_vector(10).await.expect("query")[0].clone();
            store
                .store_vector(anchor.anchor_id, &unit_vector(768, 0.5))
                .await
                .expect("store a 768-dim vector");

            // The migration default (768) already matches the default model's
            // dimension: reconciliation is a no-op and the existing vector survives.
            let discarded = store
                .reconcile_vectors_table(768)
                .await
                .expect("reconcile succeeds");
            assert_eq!(discarded, None, "a matching table is left untouched");
            assert!(!store
                .anchor_still_missing_vector(anchor.anchor_id)
                .await
                .expect("recheck"));
        });
    }

    #[test]
    fn parse_vec0_dimension_extracts_the_declared_width() {
        assert_eq!(
            super::parse_vec0_dimension(
                "CREATE VIRTUAL TABLE search_document_vectors USING vec0(embedding float[768])"
            ),
            Some(768)
        );
        // Tolerates casing/whitespace variation in the stored DDL.
        assert_eq!(
            super::parse_vec0_dimension("create virtual table x using vec0(embedding FLOAT[ 1024 ])"),
            Some(1024)
        );
        // Unrecognized shapes degrade to None (no usable dimension).
        assert_eq!(super::parse_vec0_dimension("CREATE TABLE other (id INTEGER)"), None);
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

    #[test]
    fn storing_for_a_deleted_anchor_inserts_no_orphan_vector() {
        run_async_test(async {
            let dir = test_dir("store-deleted-no-orphan");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "soon to be deleted").await;

            let store = infra.semantic_search();
            let anchor = store.anchors_missing_vector(10).await.expect("query")[0].clone();

            // Simulate the M1 delete-races-store window: the anchor's
            // search_documents row is removed (retention / Delete Recent cascade)
            // AFTER the worker passed its re-check but BEFORE the store lands. The
            // AFTER DELETE trigger drops nothing because no vec0 row exists yet.
            sqlx::query("DELETE FROM search_documents WHERE id = ?1")
                .bind(anchor.anchor_id)
                .execute(infra.pool())
                .await
                .expect("anchor deletes");

            // The atomic row-conditioned store inserts NOTHING for a vanished
            // anchor and returns cleanly (no orphan, no error): a meaning vector of
            // deleted content can never persist at rest.
            let stored = store
                .store_vector(anchor.anchor_id, &unit_vector(768, 0.5))
                .await
                .expect("store returns cleanly for a deleted anchor");
            assert!(!stored, "no row is written for a deleted anchor");

            // Prove no vec0 row exists for the gone anchor id.
            let orphan: Option<i64> = sqlx::query_scalar(
                "SELECT rowid FROM search_document_vectors WHERE rowid = ?1",
            )
            .bind(anchor.anchor_id)
            .fetch_optional(infra.pool())
            .await
            .expect("orphan probe");
            assert!(orphan.is_none(), "no orphan vector was inserted");

            // The dimension-guarded path is just as safe (it routes through the
            // same atomic store): a delete racing it also leaves nothing.
            let stored = store
                .store_vector_if_dimension_matches(anchor.anchor_id, &unit_vector(768, 0.5))
                .await
                .expect("dimension-guarded store returns cleanly for a deleted anchor");
            assert!(!stored, "the dimension-guarded path also writes no orphan");
        });
    }

    // ----------------------------------------------------------------------
    // Read seam: `knn_in_scope_anchors` — the Semantic Candidate Set
    // ----------------------------------------------------------------------

    use sqlx::{QueryBuilder, Sqlite};

    /// A one-hot unit vector keyed to `seed`: two distinct seeds are orthogonal,
    /// so L2-distance KNN order between stored anchors is unambiguous. Mirrors the
    /// `search.rs` integration test's `seeded_vector` helper.
    fn seeded_vector(dim: usize, seed: usize) -> Vec<f32> {
        let mut v = vec![0.0_f32; dim];
        v[seed % dim] = 1.0;
        v
    }

    /// Append the in-scope rowid sub-select that `knn_in_scope_anchors` constrains
    /// the KNN to. This is the store-test stand-in for `search.rs`'s
    /// `push_in_scope_anchor_rowids` closure: with no refinement scope it selects
    /// every `direct` anchor (the unrefined "all in scope" set); when
    /// `only_anchor_id` is set it narrows to that single anchor, exercising
    /// filter-then-rank without pulling in FTS/grouping.
    fn push_direct_scope(query: &mut QueryBuilder<'_, Sqlite>, only_anchor_id: Option<i64>) {
        query.push(
            "SELECT search_documents.id FROM search_documents \
             WHERE search_documents.text_source_kind = 'direct'",
        );
        if let Some(anchor_id) = only_anchor_id {
            query.push(" AND search_documents.id = ");
            query.push_bind(anchor_id);
        }
    }

    #[test]
    fn knn_returns_in_scope_anchors_nearest_first() {
        run_async_test(async {
            let dir = test_dir("knn-nearest-first");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;
            seed_frame_with_text(&infra, "2026-05-17T10:05:00Z", "bravo").await;
            seed_frame_with_text(&infra, "2026-05-17T10:10:00Z", "charlie").await;

            let store = infra.semantic_search();
            // Store three orthogonal vectors, one per anchor, at distinct seeds.
            let mut anchors = store.anchors_missing_vector(10).await.expect("query");
            anchors.sort_by_key(|a| a.anchor_id);
            for (offset, anchor) in anchors.iter().enumerate() {
                store
                    .store_vector(anchor.anchor_id, &seeded_vector(768, offset + 1))
                    .await
                    .expect("vector stores");
            }

            // Query exactly the second anchor's vector: it must come back first,
            // and every in-scope anchor is present (the KNN ranks the whole set).
            let query_vector = seeded_vector(768, 2);
            let candidates =
                super::knn_in_scope_anchors(infra.pool(), &query_vector, 200, |q| {
                    push_direct_scope(q, None)
                })
                .await
                .expect("knn succeeds");

            assert_eq!(candidates.len(), 3, "all in-scope anchors are ranked");
            assert_eq!(
                candidates[0], anchors[1].anchor_id,
                "the anchor whose vector equals the query is nearest-first"
            );
        });
    }

    #[test]
    fn knn_filter_then_rank_excludes_out_of_scope_anchors() {
        run_async_test(async {
            let dir = test_dir("knn-filter-then-rank");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "in scope").await;
            seed_frame_with_text(&infra, "2026-05-17T10:05:00Z", "out of scope").await;

            let store = infra.semantic_search();
            let mut anchors = store.anchors_missing_vector(10).await.expect("query");
            anchors.sort_by_key(|a| a.anchor_id);
            let in_scope_id = anchors[0].anchor_id;
            let out_scope_id = anchors[1].anchor_id;

            // Give the OUT-of-scope anchor the vector nearest the query, so a
            // post-filter (rank first, filter second) would rank it #1 and then
            // drop it — leaving the in-scope answer lost. The seam's `rowid IN
            // (<scope>)` runs *before* ranking, so it never enters the candidate set.
            store
                .store_vector(out_scope_id, &seeded_vector(768, 5))
                .await
                .expect("out-of-scope vector stores");
            store
                .store_vector(in_scope_id, &seeded_vector(768, 6))
                .await
                .expect("in-scope vector stores");

            let query_vector = seeded_vector(768, 5);
            let candidates =
                super::knn_in_scope_anchors(infra.pool(), &query_vector, 200, |q| {
                    push_direct_scope(q, Some(in_scope_id))
                })
                .await
                .expect("knn succeeds");

            assert_eq!(
                candidates,
                vec![in_scope_id],
                "only the in-scope anchor is returned; the nearer out-of-scope \
                 neighbor is pre-filtered, never ranked-then-dropped"
            );
        });
    }

    #[test]
    fn knn_returns_an_empty_set_on_a_dimension_mismatch() {
        run_async_test(async {
            let dir = test_dir("knn-dimension-mismatch");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;

            let store = infra.semantic_search();
            let anchor = store.anchors_missing_vector(10).await.expect("query")[0].clone();
            store
                .store_vector(anchor.anchor_id, &seeded_vector(768, 1))
                .await
                .expect("vector stores into the float[768] table");

            // The live column is float[768]. A 1024-dim query vector (a model
            // switch in flight, or stuck after a failed rebuild) disagrees with the
            // single dimension authority, so the seam returns a CLEAN EMPTY set
            // (Ok(vec![])) rather than erroring on a wrong-length blob — the read
            // degrades to keyword-only at the source.
            let mismatched_query = seeded_vector(1024, 1);
            let candidates =
                super::knn_in_scope_anchors(infra.pool(), &mismatched_query, 200, |q| {
                    push_direct_scope(q, None)
                })
                .await
                .expect("a dimension mismatch is a clean empty set, not an error");
            assert!(
                candidates.is_empty(),
                "a dimension mismatch yields an empty Semantic Candidate Set"
            );

            // A correctly-sized query reaches the KNN and returns the anchor.
            let matched_query = seeded_vector(768, 1);
            let candidates =
                super::knn_in_scope_anchors(infra.pool(), &matched_query, 200, |q| {
                    push_direct_scope(q, None)
                })
                .await
                .expect("matching dimension queries the KNN");
            assert_eq!(candidates, vec![anchor.anchor_id]);
        });
    }

    #[test]
    fn storing_a_non_finite_vector_is_rejected_and_writes_nothing() {
        run_async_test(async {
            let dir = test_dir("store-non-finite-rejected");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "vectorize me").await;

            let store = infra.semantic_search();
            let anchor = store.anchors_missing_vector(10).await.expect("query")[0].clone();

            // A vector with a NaN component (only producible by a corrupt/
            // pathological ONNX graph, never the in-tree L2-normalized pipeline) is
            // rejected before the INSERT, so it never poisons the KNN order.
            let mut poisoned = unit_vector(768, 0.5);
            poisoned[3] = f32::NAN;
            assert!(
                store.store_vector(anchor.anchor_id, &poisoned).await.is_err(),
                "a NaN component is rejected"
            );

            // An infinite component is rejected the same way.
            let mut poisoned = unit_vector(768, 0.5);
            poisoned[3] = f32::INFINITY;
            assert!(
                store.store_vector(anchor.anchor_id, &poisoned).await.is_err(),
                "an inf component is rejected"
            );

            // Nothing was written: the anchor is still in the missing set and is
            // retried (rather than being silently marked done with a poison vector).
            assert!(store
                .anchor_still_missing_vector(anchor.anchor_id)
                .await
                .expect("recheck"));
            assert_eq!(
                store.count_anchors_missing_vector().await.expect("count"),
                1
            );
        });
    }
}
