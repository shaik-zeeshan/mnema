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
//!   (ADR 0036). The `direct`-only filter is the whole dedup policy: **only**
//!   `direct` anchors ever carry a **Semantic Search Vector**. An
//!   `equivalent_reuse` anchor is never embedded and has no `vec0` row, so it is
//!   not itself KNN-reachable — its group's `direct` representative is the row
//!   that surfaces for the whole dedup group.
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

use crate::db::CaptureDb;
use crate::Result;

/// The `app_settings` key stamping **which model's embedding space** the live
/// `search_document_vectors` table holds — the **model epoch guard**.
///
/// The vec0 table records only its column *width*, and width alone stopped being a
/// usable discriminator the moment the catalog gained models that share a dimension
/// with an already-shipping one (`gte-modernbert-base` and
/// `granite-embedding-english-r2` are 768 like `nomic-embed-text-v1.5`;
/// `granite-embedding-small-english-r2` is 384 like `multilingual-e5-small`). Without
/// this stamp, switching between two same-width models would leave every stale vector
/// in place — a different embedding space, no error, no self-heal, degraded search
/// that looks healthy.
///
/// The stamp is written in the SAME transaction as the DROP+CREATE
/// ([`SemanticSearchStore::recreate_vectors_table`]), so table and stamp can never
/// disagree, and it is checked on every write
/// ([`SemanticSearchStore::store_vectors_if_model_matches`]) and on startup
/// reconciliation ([`SemanticSearchStore::reconcile_vectors_table`]).
///
/// The **query** path deliberately does NOT check it. The only way the table can
/// hold vectors from a model other than the selected one is a write that slipped
/// through, and the write gate is what prevents that: a half-applied switch leaves
/// the table freshly recreated and therefore *empty*, which returns no results
/// rather than wrong ones.
const VECTORS_MODEL_KEY: &str = "semantic_search.vectors_model_id";

/// The **embedding recipe** the stored vectors were produced under — everything
/// outside the model weights that changes what vector a given text becomes.
///
/// The model id alone is not the whole embedding space. `semantic-search`'s
/// document path also decides how many token windows a document contributes
/// (`runtime::MAX_DOCUMENT_CHUNKS`) and how those windows are pooled
/// (`weighted_mean_pool_l2`, byte-length weighted). Change either and the same text
/// under the same weights produces a *different* vector — but `model_id` does not
/// move, so a stamp keyed on the id alone reads "healthy" while the index silently
/// holds two incomparable generations of vector: pre-change rows keep their old
/// values forever, because `anchors_missing_vector` only re-derives anchors with
/// **no** vector at all.
///
/// Bumping this string is therefore the one switch that forces a full re-index.
/// Bump it whenever the document embed path changes shape:
/// - `MAX_DOCUMENT_CHUNKS` moves (more or fewer windows per document), or
/// - the cross-chunk pooling rule changes (uniform → weighted, weights redefined), or
/// - the per-model prompt strings or the window budget change.
///
/// `v2` is this change: cap 2 windows + byte-length-weighted pooling. `v1` was the
/// uncapped uniform mean that shipped before it.
const EMBED_INDEX_RECIPE: &str = "v2-cap2-wmean";

/// The full identity of the embedding space a vec0 table holds: the model id AND
/// the [`EMBED_INDEX_RECIPE`] it was produced under. This composite — not the bare
/// model id — is what the stamp stores and every gate compares.
pub fn vectors_index_epoch(model_id: &str) -> String {
    format!("{model_id}@{EMBED_INDEX_RECIPE}")
}

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
    db: CaptureDb,
}

impl SemanticSearchStore {
    pub(crate) fn new(db: CaptureDb) -> Self {
        Self { db }
    }

    /// Select up to `limit` `direct` **Search Result Anchor**s that have
    /// searchable text but no **Semantic Search Vector** yet, newest-first.
    ///
    /// Newest-first (`absolute_start_at DESC, id DESC`) is the ADR-0036 ordering:
    /// freshly captured anchors preempt the historical backlog, which is drained
    /// from the newest end backward. Only `text_source_kind = 'direct'` rows are
    /// considered: **only** `direct` anchors are ever embedded, so an
    /// `equivalent_reuse` anchor gets no `vec0` row and is not itself KNN-reachable
    /// — its group's `direct` representative is what surfaces. Structural frame
    /// dedup thus collapses the embed count with no separate admission pass.
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
        .fetch_all(self.db.read())
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
        .fetch_optional(self.db.read())
        .await?;
        Ok(exists.is_some())
    }

    /// Store one **Semantic Search Vector** for `anchor_id` into the
    /// `search_document_vectors` vec0 table, **conditioned on the `direct`
    /// `search_documents` row still existing**. `vector` is the model's f32
    /// output; it is serialized little-endian, the byte layout vec0 expects.
    /// Returns whether a row was actually written.
    ///
    /// The write is a **DELETE-then-INSERT in one transaction**, keyed on
    /// `anchor_id`: vec0 (sqlite-vec 0.1.9) does **not** honor `OR REPLACE` —
    /// re-inserting an existing rowid raises a `UNIQUE constraint` error rather
    /// than replacing — so an upsert has to delete any prior vector for the anchor
    /// first, then insert the new one. Wrapping both in a single transaction makes
    /// the replace atomic: a re-embed of an already-vectored anchor swaps the
    /// vector with no constraint error and no torn state if the insert fails
    /// mid-way (the DELETE rolls back too, leaving the old vector intact).
    ///
    /// The INSERT is row-conditioned: the rowid and the existence predicate are
    /// evaluated in the same `SELECT … WHERE` over `search_documents`, so there is
    /// **no re-check-then-store gap**. If a retention / Delete Recent cascade
    /// removed the anchor between the worker's embed and this store (the `AFTER
    /// DELETE` trigger having dropped the vec0 row, which the DELETE here also
    /// covers), the `SELECT` matches zero rows and **no orphan vector is
    /// inserted** — a meaning vector of deleted captured content can never persist
    /// at rest (M1 / privacy concern #6, ADR 0036). The worker's preceding
    /// `anchor_still_missing_vector` re-check is now an optimization, not the
    /// correctness boundary; this transaction is.
    ///
    /// Rejects a non-finite vector (any `NaN`/`±inf` component) before touching
    /// the table: vec0 stores such a blob silently, but a `NaN` distance sorts
    /// non-deterministically under the KNN order and `anchor_still_missing_vector`
    /// would treat the poisoned vector as done and never retry it (L1). The
    /// in-tree pipeline cannot produce one (every embedding is L2-normalized over
    /// guaranteed-non-empty text), so this is defensive against a corrupt/
    /// pathological ONNX graph only.
    ///
    /// **Internal primitive — call [`SemanticSearchStore::store_vector_if_model_matches`]
    /// instead.** This is `pub(crate)` (not `pub`) on purpose (F13): it does NO
    /// live-dimension check, so a caller that reaches it directly bypasses the single
    /// dimension authority and can write a wrong-length blob the live `vec0` column
    /// would reject (or, worse under a future same-dimension model, a cross-model
    /// vector). The gate lives in
    /// [`SemanticSearchStore::store_vector_if_model_matches`]; the worker calls
    /// only that. Narrowing visibility to `pub(crate)` keeps the in-crate tests
    /// compiling while making it impossible for external code to skip the gate.
    ///
    /// No production caller remains: the gate now owns its write transaction (so the
    /// stamp it checks is the stamp the write lands under), and the tests keep this
    /// as their deliberately UNGATED primitive for seeding a pre-stamp table.
    #[allow(dead_code)]
    pub(crate) async fn store_vector(&self, anchor_id: i64, vector: &[f32]) -> Result<bool> {
        if vector.iter().any(|component| !component.is_finite()) {
            return Err(crate::AppInfraError::InvalidSearchRequest(format!(
                "refusing to store a non-finite Semantic Search Vector for anchor {anchor_id} \
                 (a NaN/inf component would poison KNN ordering)"
            )));
        }
        let blob = vector_to_le_bytes(vector);
        let mut tx = self.db.begin_write().await?;
        // Drop any existing vector for this anchor first — vec0 0.1.9 rejects a
        // re-insert of the same rowid with a UNIQUE constraint error rather than
        // replacing, so the upsert must DELETE then INSERT. Harmless when no prior
        // vector exists (the normal sweep), and atomic with the INSERT below.
        sqlx::query("DELETE FROM search_document_vectors WHERE rowid = ?1")
            .bind(anchor_id)
            .execute(&mut *tx)
            .await?;
        let result = sqlx::query(
            "INSERT INTO search_document_vectors (rowid, embedding) \
             SELECT search_documents.id, vec_quantize_int8(?2, 'unit') \
             FROM search_documents \
             WHERE search_documents.id = ?1 \
               AND search_documents.text_source_kind = 'direct'",
        )
        .bind(anchor_id)
        .bind(blob)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }

    /// Store a vector **only if it belongs to the live table's embedding space** —
    /// its producing `model_id` matches the table's stamp AND its length matches the
    /// live `vec0` column width — returning whether it was stored.
    ///
    /// This is the worker-side half of the index authority. The two-step model
    /// switch (rebuild the table, then persist `model_id`) is non-atomic across the
    /// worker: between the embedder reloading under the new model and the table
    /// being rebuilt — and **permanently** if the rebuild ever fails — an embedded
    /// vector would belong to a different embedding space than the table. A raw
    /// [`store_vector`] would either have vec0 reject the blob (different width, so
    /// the sweep error-loops that doomed batch every retry forever) or, far worse,
    /// **silently accept it** when the two models share a width. Here a mismatch is a
    /// **skip, not an error**: the anchor stays in the missing set and is re-embedded
    /// once table and model agree (after the rebuild lands, or after startup
    /// reconciliation self-heals a stuck table), so the worker idles instead of
    /// error-looping or contaminating the index.
    ///
    /// `Ok(true)` — stored. `Ok(false)` — skipped: the table names a different model
    /// (or is unstamped, or absent), the vector length disagrees with the live
    /// column, **or** the `direct` anchor row no longer exists (a delete raced the
    /// store — [`store_vector`] inserts nothing, so no orphan is left). `Err` — a
    /// non-finite vector (L1) or a real DB failure.
    ///
    /// **The model stamp is the discriminator, not the width** ([`VECTORS_MODEL_KEY`]).
    /// Dimension equality is kept as a second, cheaper gate — it is what actually
    /// stops vec0 from rejecting a blob — but it is no longer load-bearing for
    /// cross-model contamination, and must not be relied on as such: the catalog now
    /// ships models that share a dimension with another on purpose.
    pub async fn store_vector_if_model_matches(
        &self,
        model_id: &str,
        anchor_id: i64,
        vector: &[f32],
    ) -> Result<bool> {
        // One anchor is just a batch of one: delegating keeps the stamp/width gate
        // in ONE place, evaluated inside the write transaction it guards. Checking
        // here and storing through a second transaction would reopen the
        // check-then-write window a model switch lands in.
        Ok(self
            .store_vectors_if_model_matches(model_id, &[(anchor_id, vector.to_vec())])
            .await?
            .first()
            .copied()
            .unwrap_or(false))
    }

    /// Batched counterpart to [`store_vector_if_model_matches`]: stores a whole
    /// sweep batch in **one** write transaction (one writer-lock acquisition for the
    /// batch instead of one per anchor), returning a per-anchor `stored` flag
    /// aligned to `pairs`. Each `true`/`false` carries the exact same meaning as the
    /// single-anchor call — `true` stored, `false` skipped (the table names another
    /// model, dimension mismatch, no table, or the `direct` anchor vanished
    /// mid-embed so the row-conditioned INSERT affected nothing). Reducing the
    /// lock-acquisition rate is the point: the per-anchor version made the background
    /// sweep grab the writer lock once per vector, churning contention with
    /// foreground capture writes.
    ///
    /// All-or-nothing on a real DB failure: a genuine `Err` rolls the batch back, so
    /// the caller retries the whole batch (transient). The model stamp and the live
    /// dimension are both read once up front; the same index-authority invariant as
    /// [`store_vector_if_model_matches`] applies (a vector from another model's
    /// embedding space, or of the wrong length, is skipped, never inserted).
    pub async fn store_vectors_if_model_matches(
        &self,
        model_id: &str,
        pairs: &[(i64, Vec<f32>)],
    ) -> Result<Vec<bool>> {
        if pairs.is_empty() {
            return Ok(Vec::new());
        }
        // Non-finite guard (defensive; mirrors `store_vector`). A NaN/inf component
        // would poison KNN ordering, so refuse the batch rather than store it.
        for (anchor_id, vector) in pairs {
            if vector.iter().any(|component| !component.is_finite()) {
                return Err(crate::AppInfraError::InvalidSearchRequest(format!(
                    "refusing to store a non-finite Semantic Search Vector for anchor {anchor_id} \
                     (a NaN/inf component would poison KNN ordering)"
                )));
            }
        }
        let mut tx = self.db.begin_write().await?;
        // The gate is read INSIDE the write transaction. `begin_write` is BEGIN
        // IMMEDIATE, so once it returns no other writer can commit a
        // `recreate_vectors_table` until this batch commits — the gate therefore
        // describes the table this batch actually writes into. Reading the stamp
        // BEFORE taking the writer lock leaves the classic check-then-write window,
        // and a Settings model switch is exactly what lands in it (the sweep embeds
        // for seconds, then queues for the lock behind the switch's rebuild). With
        // two catalog models sharing a width the length check below cannot catch it
        // either, so the stale-model vector would be stored silently AND
        // permanently: the anchor leaves the missing set (never re-embedded) and
        // startup reconciliation no-ops because the stamp agrees with the selection.
        //
        // The table holds another model's embedding space, another embedding RECIPE
        // (see [`EMBED_INDEX_RECIPE`]), or is unstamped → every anchor is a skip,
        // awaiting reconciliation. Dropping `tx` here rolls back an empty transaction.
        if stamped_vectors_model(&mut tx).await? != Some(vectors_index_epoch(model_id)) {
            return Ok(vec![false; pairs.len()]);
        }
        // No live table → every anchor is a skip (awaiting re-index), no write needed.
        let Some(dimension) = live_vector_dimension_in_tx(&mut tx).await? else {
            return Ok(vec![false; pairs.len()]);
        };
        let mut outcomes = Vec::with_capacity(pairs.len());
        for (anchor_id, vector) in pairs {
            // Length mismatch with the live column: skip (not an error), exactly as
            // the single-anchor gate does, so a mid-switch vector idles instead of
            // being rejected by vec0.
            if vector.len() != dimension {
                outcomes.push(false);
                continue;
            }
            let blob = vector_to_le_bytes(vector);
            // DELETE-then-INSERT upsert (vec0 0.1.9 rejects same-rowid re-insert), the
            // INSERT row-conditioned on the `direct` anchor still existing so a delete
            // racing the store leaves no orphan.
            sqlx::query("DELETE FROM search_document_vectors WHERE rowid = ?1")
                .bind(anchor_id)
                .execute(&mut *tx)
                .await?;
            let result = sqlx::query(
                "INSERT INTO search_document_vectors (rowid, embedding) \
                 SELECT search_documents.id, vec_quantize_int8(?2, 'unit') \
                 FROM search_documents \
                 WHERE search_documents.id = ?1 \
                   AND search_documents.text_source_kind = 'direct'",
            )
            .bind(anchor_id)
            .bind(blob)
            .execute(&mut *tx)
            .await?;
            outcomes.push(result.rows_affected() > 0);
        }
        tx.commit().await?;
        Ok(outcomes)
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
        live_vector_dimension(self.db.read()).await
    }

    /// The **index epoch stamped on the live `search_document_vectors` table** —
    /// which embedding space its vectors belong to — or `None` when no stamp has
    /// been written yet (see [`VECTORS_MODEL_KEY`]).
    ///
    /// The value is the composite [`vectors_index_epoch`] (`model_id@recipe`), not a
    /// bare model id: compare it against `vectors_index_epoch(model_id)`, never
    /// against `model_id`. It is returned raw so callers can log what is actually
    /// stamped when it disagrees.
    ///
    /// `None` means a table that predates the stamp entirely (migration `0039`, or a
    /// `0039`-era install that never went through [`recreate_vectors_table`]).
    /// [`reconcile_vectors_table`] rebuilds it rather than adopting it — its recipe
    /// is unknowable.
    pub async fn live_vector_model(&self) -> Result<Option<String>> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?1")
                .bind(VECTORS_MODEL_KEY)
                .fetch_optional(self.db.read())
                .await?;
        Ok(value)
    }

    /// Reconcile the live `vec0` table against the selected model — both its
    /// **stamped model id** and its column width — recreating the table only when
    /// they disagree. Returns `Some(discarded)` with the number of vectors dropped
    /// if a recreate happened, or `None` if the table already matched (no-op).
    ///
    /// This is the **startup self-heal** for a permanently-stuck switch: if a
    /// model switch persisted a new `model_id` but the table recreate failed (DB
    /// busy under the worker's concurrent writes — `DROP TABLE` needs an exclusive
    /// lock), the table is left holding the old model's vectors while the selection
    /// names a new one. Both the worker ([`store_vectors_if_model_matches`]) and the
    /// query path then skip/idle — search never hard-fails, but the index also never
    /// rebuilds. Running this on the deferred-startup seam with the selected model
    /// brings the table back into agreement so the sweep can backfill under it.
    /// Idempotent: a matching table is left untouched.
    ///
    /// **An unstamped table is rebuilt, not adopted.** An earlier revision stamped a
    /// pre-stamp table in place when its width matched, reasoning that the old
    /// pairwise-distinct-dimension regime made a matching width imply a matching
    /// model. Adding [`EMBED_INDEX_RECIPE`] to the stamp retires that argument: the
    /// width tells us nothing about which *recipe* produced the rows, and an
    /// unstamped table is by definition one written before the recipe was recorded —
    /// i.e. under the uncapped uniform-mean `v1` path. Adopting it would stamp `v2`
    /// onto `v1` vectors, which is exactly the silent two-generation index the recipe
    /// exists to prevent. Rebuilding costs a re-embed on a dev machine and nothing on
    /// a fresh install (migration `0039`'s table is empty, so `Some(0)` is discarded).
    pub async fn reconcile_vectors_table(
        &self,
        model_id: &str,
        expected_dimension: usize,
    ) -> Result<Option<u64>> {
        // Decide INSIDE a write transaction (`begin_write` is BEGIN IMMEDIATE), so a
        // concurrent `recreate_vectors_table` — a Settings model switch, which the
        // user can run the moment the window opens, while this runs on the
        // deferred-startup seam — cannot commit between the observation and the
        // adopt-stamp. Reading first and stamping after would let adoption stamp the
        // model it *saw* onto a table that has since been rebuilt for another one:
        // a stamp naming an embedding space the table does not hold, which the write
        // gate then rejects on every batch for the rest of the session (search
        // silently stays keyword-only until the next restart).
        let mut tx = self.db.begin_write().await?;
        let live_model = stamped_vectors_model(&mut tx).await?;
        let live_dimension = live_vector_dimension_in_tx(&mut tx).await?;
        let expected_epoch = vectors_index_epoch(model_id);
        match (live_model.as_deref(), live_dimension) {
            // Stamp (model AND recipe) and width both agree with the selection:
            // nothing to do. This is the common case on every launch after the first.
            (Some(stamped), Some(dimension))
                if stamped == expected_epoch && dimension == expected_dimension =>
            {
                Ok(None)
            }
            // Anything else — a different model's stamp, a stamp from an older
            // embedding recipe, an unstamped (pre-recipe) table, a width
            // disagreement, or no table at all — rebuilds under the selected model so
            // the worker's index authority agrees with the selection again. Release
            // the writer first: `recreate_vectors_table` opens its own
            // `BEGIN IMMEDIATE`, which would otherwise queue behind this
            // transaction's lock. The rebuild is self-consistent on its own (table and
            // stamp commit together), so it needs no state carried over from the
            // observation above.
            _ => {
                drop(tx);
                Ok(Some(
                    self.recreate_vectors_table(model_id, expected_dimension).await?,
                ))
            }
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
    ///
    /// **Load-bearing invariant — the model stamp.** The rebuilt table carries
    /// `model_id` in the [`VECTORS_MODEL_KEY`] stamp, written **inside this same
    /// transaction** as the DROP+CREATE, so the table and the name of the embedding
    /// space it holds can never disagree — not on a crash, not on a rollback.
    /// Together with [`store_vectors_if_model_matches`], that stamp is the
    /// discriminator between the old and new embedding spaces during a switch. It
    /// replaced the old "every catalog model has a distinct dimension" invariant,
    /// which the catalog deliberately broke to add the ModernBERT English options
    /// (768 = nomic, 384 = multilingual-e5-small). Do not reintroduce the width as
    /// the identity check.
    pub async fn recreate_vectors_table(
        &self,
        model_id: &str,
        dimension: usize,
    ) -> Result<u64> {
        let mut tx = self.db.begin_write().await?;
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
        // int8 (not float) matches the migration: the write/query paths quantize
        // via `vec_quantize_int8(?, 'unit')`, so a recreate must keep the same dtype
        // or the quantized blobs would not match a float column.
        sqlx::query(&format!(
            "CREATE VIRTUAL TABLE search_document_vectors USING vec0(embedding int8[{dimension}])"
        ))
        .execute(&mut *tx)
        .await?;
        // Same transaction as the DROP+CREATE: the table and its model stamp commit
        // together or not at all.
        stamp_vectors_model(&mut tx, model_id).await?;
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
        .fetch_one(self.db.read())
        .await?;
        Ok(count)
    }

    /// Count of stored **Semantic Search Vector**s — the size of the live index.
    ///
    /// Reports `0` (never an error) when the `vec0` table is absent, the same
    /// "absent → nothing stored" reading [`recreate_vectors_table`] takes: this
    /// is a debug readout, and a missing table means an empty index.
    pub async fn count_vectors(&self) -> Result<i64> {
        if self.live_vector_dimension().await?.is_none() {
            return Ok(0);
        }
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM search_document_vectors")
                .fetch_one(self.db.read())
                .await?,
        )
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
         WHERE search_document_vectors.embedding MATCH vec_quantize_int8(",
    );
    query.push_bind(vector_to_le_bytes(query_embedding));
    // Quantize the query vector to int8 the same way stored vectors are, so the
    // KNN compares like with like against the int8[] column. The embedder
    // guarantees unit vectors, so 'unit' (input range [-1,1]) is the right scale
    // and unit-vector L2 ordering ≡ cosine ordering — rank is preserved.
    query.push(", 'unit')");
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

/// Read the [`VECTORS_MODEL_KEY`] stamp inside a caller-owned write transaction —
/// the read half of the gate, taken under the writer lock so no `recreate_vectors_table`
/// can commit between the check and the write it guards.
async fn stamped_vectors_model(tx: &mut sqlx::Transaction<'_, Sqlite>) -> Result<Option<String>> {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?1")
        .bind(VECTORS_MODEL_KEY)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(value)
}

/// [`live_vector_dimension`] read inside a caller-owned write transaction, so the
/// width the gate checks is the width of the table the write lands in.
async fn live_vector_dimension_in_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
) -> Result<Option<usize>> {
    let sql: Option<String> = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master \
         WHERE type = 'table' AND name = 'search_document_vectors'",
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(sql.as_deref().and_then(parse_vec0_dimension))
}

/// Write the [`VECTORS_MODEL_KEY`] stamp inside a caller-owned write transaction.
///
/// Taking the transaction (rather than the pool) is the point: the stamp must land
/// in the SAME transaction as the DROP+CREATE it describes, so a rolled-back rebuild
/// can never leave a stamp that names a table that was not built.
async fn stamp_vectors_model(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    model_id: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO app_settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
    )
    .bind(VECTORS_MODEL_KEY)
    .bind(vectors_index_epoch(model_id))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Parse the declared dimension `N` out of a `vec0(embedding int8[N])` table
/// DDL (`sqlite_master.sql`). The whole feature keys its dimension authority off
/// this — the recreate writes exactly this shape (see
/// [`SemanticSearchStore::recreate_vectors_table`]), so the parse is the inverse
/// of that format. Returns `None` on any shape it does not recognize, so an
/// unexpected DDL degrades to "no usable dimension" rather than a wrong guess.
fn parse_vec0_dimension(sql: &str) -> Option<usize> {
    // Parse the `[N]` after the dtype, dtype-agnostic (int8 today, float in
    // legacy DDL) — the only bracket in the vec0 column declaration.
    let lowered = sql.to_ascii_lowercase();
    let open = lowered.find('[')? + 1;
    let close = lowered[open..].find(']')? + open;
    lowered[open..close].trim().parse::<usize>().ok()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{vectors_index_epoch, VECTORS_MODEL_KEY};
    use crate::{
        AppInfra, NewFrame, ProcessingJob, ProcessingJobDraft, ProcessingResultDraft,
    };

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Real catalog model ids used as the vec0 table's model stamp in these tests.
    /// `MODEL_A` and `MODEL_B` are two DIFFERENT models that share the same 768
    /// width — the exact collision the model-stamp epoch guard exists for, and the
    /// one a dimension check cannot see. `MODEL_WIDE` is 1024, so a switch to it
    /// changes the width too.
    const MODEL_A: &str = "nomic-embed-text-v1.5";
    const MODEL_B: &str = "gte-modernbert-base";
    const MODEL_WIDE: &str = "bge-m3";

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
                .recreate_vectors_table(MODEL_A, 768)
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
                .recreate_vectors_table(MODEL_WIDE, 1024)
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
    fn re_storing_a_vector_for_the_same_anchor_replaces_it_without_a_unique_error() {
        run_async_test(async {
            let dir = test_dir("re-store-same-anchor-replaces");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "vectorize me").await;

            let store = infra.semantic_search();
            let anchor = store.anchors_missing_vector(10).await.expect("query")[0].clone();

            // First store: the normal sweep path inserts a fresh vector.
            let first = seeded_vector(768, 1);
            let stored = store
                .store_vector(anchor.anchor_id, &first)
                .await
                .expect("first store succeeds");
            assert!(stored, "the first vector is written");

            // Second store of a DIFFERENT vector for the SAME anchor_id must
            // succeed (the DELETE+INSERT upsert replaces it). vec0 0.1.9 does not
            // honor OR REPLACE, so a naive re-insert would raise a UNIQUE
            // constraint error here — this asserts the upsert path is correct.
            let second = seeded_vector(768, 5);
            let stored = store
                .store_vector(anchor.anchor_id, &second)
                .await
                .expect("re-storing the same anchor replaces, never UNIQUE-errors");
            assert!(stored, "the replacement vector is written");

            // Exactly one row exists for the anchor (the upsert replaced, not
            // appended).
            let row_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_document_vectors WHERE rowid = ?1",
            )
            .bind(anchor.anchor_id)
            .fetch_one(infra.pool())
            .await
            .expect("count probe");
            assert_eq!(row_count, 1, "exactly one vector row remains for the anchor");

            // The stored vector is the SECOND one: a KNN query for `second` ranks
            // this anchor nearest (distance ~0), confirming the replace landed the
            // new vector, not the old.
            let candidates =
                super::knn_in_scope_anchors(infra.pool(), &second, 200, |q| {
                    push_direct_scope(q, None)
                })
                .await
                .expect("knn succeeds");
            assert_eq!(
                candidates,
                vec![anchor.anchor_id],
                "the stored vector is the second one (the upsert replaced it)"
            );
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
                .recreate_vectors_table(MODEL_WIDE, 1024)
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
            // Stamp the migration-fresh table for MODEL_A, as startup reconciliation
            // does before the worker's first sweep.
            store
                .reconcile_vectors_table(MODEL_A, 768)
                .await
                .expect("adopt the migration table");

            // The live table is int8[768]. A 1024-dim vector (an embedder reloaded
            // at a new dimension before the table was rebuilt — the non-atomic
            // switch window, or a permanently-stuck table) does NOT fatally error:
            // it is skipped (`Ok(false)`), so the worker idles instead of
            // error-looping a doomed batch every 30s.
            let stored = store
                .store_vector_if_model_matches(MODEL_A, anchor.anchor_id, &unit_vector(1024, 0.5))
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
                .store_vector_if_model_matches(MODEL_A, anchor.anchor_id, &unit_vector(768, 0.5))
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
    fn store_vectors_batch_stores_matching_and_skips_wrong_dimension_in_one_call() {
        run_async_test(async {
            let dir = test_dir("store-batch-mixed");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;
            seed_frame_with_text(&infra, "2026-05-17T10:00:01Z", "bravo").await;

            let store = infra.semantic_search();
            let missing = store.anchors_missing_vector(10).await.expect("query");
            assert_eq!(missing.len(), 2, "two anchors await a vector");
            store
                .reconcile_vectors_table(MODEL_A, 768)
                .await
                .expect("adopt the migration table");

            // One correctly-sized (768) vector and one wrong-dimension (1024) vector
            // in a single batched call: the matching one stores, the mismatch is
            // skipped (not an error, not stored), and the per-anchor outcomes line up
            // with the input order.
            let outcomes = store
                .store_vectors_if_model_matches(MODEL_A, &[
                    (missing[0].anchor_id, unit_vector(768, 0.5)),
                    (missing[1].anchor_id, unit_vector(1024, 0.5)),
                ])
                .await
                .expect("a dimension mismatch is a skip, not a fatal error");
            assert_eq!(outcomes, vec![true, false], "stored, then skipped");

            // The stored anchor leaves the missing set; the skipped one stays for a
            // later re-embed once dimensions agree.
            assert!(!store
                .anchor_still_missing_vector(missing[0].anchor_id)
                .await
                .expect("recheck"));
            assert!(store
                .anchor_still_missing_vector(missing[1].anchor_id)
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
                .reconcile_vectors_table(MODEL_WIDE, 1024)
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

    /// An unstamped table is REBUILT, not adopted in place.
    ///
    /// An earlier revision stamped it and kept the rows, arguing that the pre-stamp
    /// pairwise-distinct-dimension regime made a matching width imply a matching
    /// model. Recording the embedding recipe in the stamp retires that argument: the
    /// width says nothing about which pooling rule produced the rows, and an
    /// unstamped table is by construction pre-recipe (uncapped uniform mean).
    /// Adopting it would stamp the current recipe onto vectors that do not have it.
    #[test]
    fn reconcile_rebuilds_an_unstamped_table_because_its_recipe_is_unknowable() {
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
            // The migration-0039 table carries no stamp at all.
            assert_eq!(store.live_vector_model().await.expect("stamp"), None);

            let discarded = store
                .reconcile_vectors_table(MODEL_A, 768)
                .await
                .expect("reconcile succeeds");
            assert_eq!(
                discarded,
                Some(1),
                "the pre-recipe vector is discarded, not adopted"
            );
            assert_eq!(
                store.live_vector_model().await.expect("stamp"),
                Some(vectors_index_epoch(MODEL_A)),
                "the rebuild stamps model AND recipe"
            );
            assert!(
                store
                    .anchor_still_missing_vector(anchor.anchor_id)
                    .await
                    .expect("recheck"),
                "the anchor is re-queued so it is re-embedded under the current recipe"
            );

            // And a second run is a plain no-op — stamp and width both agree now.
            assert_eq!(
                store
                    .reconcile_vectors_table(MODEL_A, 768)
                    .await
                    .expect("reconcile succeeds"),
                None
            );
        });
    }

    /// The recipe half of the epoch: the SAME model at the SAME width, but vectors
    /// produced under a different document-embed recipe, must be rebuilt and must not
    /// be written into.
    ///
    /// This is the failure a model-id-only stamp cannot see. `MAX_DOCUMENT_CHUNKS`
    /// and the cross-chunk pooling rule both change what vector a text becomes while
    /// `model_id` stays put, and `anchors_missing_vector` only re-derives anchors
    /// with NO vector — so without this, pre-change rows would keep their old values
    /// forever and the index would silently hold two incomparable generations.
    #[test]
    fn a_recipe_change_at_the_same_model_and_width_rebuilds_the_index() {
        run_async_test(async {
            let dir = test_dir("recipe-change");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;

            let store = infra.semantic_search();
            let anchor = store.anchors_missing_vector(10).await.expect("query")[0].clone();
            store
                .recreate_vectors_table(MODEL_A, 768)
                .await
                .expect("build under MODEL_A at the current recipe");
            assert!(store
                .store_vector_if_model_matches(MODEL_A, anchor.anchor_id, &unit_vector(768, 0.5))
                .await
                .expect("stores under the matching epoch"));

            // Simulate the next recipe bump by stamping an older epoch for the SAME
            // model at the SAME width — byte-identical table shape, different rows.
            let mut tx = infra.database.begin_write().await.expect("writer");
            sqlx::query(
                "INSERT INTO app_settings (key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )
            .bind(VECTORS_MODEL_KEY)
            .bind(format!("{MODEL_A}@v1-uncapped-mean"))
            .execute(&mut *tx)
            .await
            .expect("stamp the old recipe");
            tx.commit().await.expect("commit");

            // The write gate rejects it: same model, same width, wrong recipe.
            assert!(
                !store
                    .store_vector_if_model_matches(
                        MODEL_A,
                        anchor.anchor_id,
                        &unit_vector(768, 0.5)
                    )
                    .await
                    .expect("a recipe mismatch is a skip, not an error"),
                "a vector from the current recipe must not join an older recipe's index"
            );

            // And reconciliation rebuilds, even though model and width both match.
            assert_eq!(
                store
                    .reconcile_vectors_table(MODEL_A, 768)
                    .await
                    .expect("reconcile succeeds"),
                Some(1),
                "a stale-recipe index is discarded, not left in place"
            );
            assert_eq!(
                store.live_vector_model().await.expect("stamp"),
                Some(vectors_index_epoch(MODEL_A))
            );
        });
    }

    /// The reconcile cells no other test reaches: a width disagreement while the
    /// stamp AGREES, and a missing table while a stamp survives. Both must rebuild.
    ///
    /// The first is reachable if a catalog descriptor's dimension is ever corrected
    /// for an existing id; the second is the "absent → rebuild" self-heal the
    /// function exists for. Neither is covered by the model-mismatch tests, because
    /// in both of these the stamped MODEL is the selected one.
    #[test]
    fn reconcile_rebuilds_on_a_width_disagreement_and_on_a_missing_table() {
        run_async_test(async {
            let dir = test_dir("reconcile-matrix");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let store = infra.semantic_search();

            // (stamp matches, width differs) → rebuild at the expected width, stamp
            // unchanged.
            store
                .recreate_vectors_table(MODEL_A, 768)
                .await
                .expect("build under MODEL_A at 768");
            assert_eq!(
                store
                    .reconcile_vectors_table(MODEL_A, 1024)
                    .await
                    .expect("reconcile succeeds"),
                Some(0),
                "a width disagreement rebuilds even when the stamp agrees"
            );
            assert_eq!(store.live_vector_dimension().await.expect("dim"), Some(1024));
            assert_eq!(
                store.live_vector_model().await.expect("stamp"),
                Some(vectors_index_epoch(MODEL_A))
            );

            // (stamp survives, table absent) → rebuild. The stamp outliving its table
            // is only reachable by dropping the table out from under it, which is what
            // a failed/partial rebuild in an older build could leave behind.
            sqlx::query("DROP TABLE search_document_vectors")
                .execute(infra.pool())
                .await
                .expect("drop the table, leaving the stamp behind");
            assert_eq!(store.live_vector_dimension().await.expect("dim"), None);
            assert_eq!(
                store
                    .reconcile_vectors_table(MODEL_A, 768)
                    .await
                    .expect("reconcile succeeds"),
                Some(0),
                "an absent table rebuilds rather than being reported healthy"
            );
            assert_eq!(store.live_vector_dimension().await.expect("dim"), Some(768));
        });
    }

    /// A stamp that outlives its table (a DROP that landed without a rebuild) makes
    /// every anchor a SKIP — not an error, not a panic — so the sweep idles until
    /// startup reconciliation rebuilds, instead of error-looping a doomed batch every
    /// 30 s forever.
    ///
    /// This is the one gate arm no other test reaches: every other skip is decided by
    /// the stamp comparison, this one by the in-transaction WIDTH read returning
    /// `None` while the stamp AGREES. It is also the only place the multi-element
    /// `vec![false; pairs.len()]` alignment is observable.
    #[test]
    fn a_stamped_but_missing_table_skips_the_batch_instead_of_erroring() {
        run_async_test(async {
            let dir = test_dir("stamped-no-table");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;
            seed_frame_with_text(&infra, "2026-05-17T10:01:00Z", "beta").await;

            let store = infra.semantic_search();
            let missing = store.anchors_missing_vector(10).await.expect("query");
            assert_eq!(missing.len(), 2);
            store
                .recreate_vectors_table(MODEL_A, 768)
                .await
                .expect("build under MODEL_A");
            sqlx::query("DROP TABLE search_document_vectors")
                .execute(infra.pool())
                .await
                .expect("drop the table out from under its stamp");

            let outcomes = store
                .store_vectors_if_model_matches(
                    MODEL_A,
                    &[
                        (missing[0].anchor_id, unit_vector(768, 0.5)),
                        (missing[1].anchor_id, unit_vector(768, 0.25)),
                    ],
                )
                .await
                .expect("an absent table is a skip, not an error");
            assert_eq!(
                outcomes,
                vec![false, false],
                "one flag per input, in input order, even on the early-return arm"
            );

            // ...and the self-heal recovers it: the rebuild lands and BOTH anchors are
            // still queued, so nothing was silently marked stored.
            assert_eq!(
                store
                    .reconcile_vectors_table(MODEL_A, 768)
                    .await
                    .expect("reconcile succeeds"),
                Some(0)
            );
            for anchor in &missing[..2] {
                assert!(store
                    .anchor_still_missing_vector(anchor.anchor_id)
                    .await
                    .expect("recheck"));
            }
        });
    }

    /// The whole reason the model stamp exists: two DIFFERENT models at the SAME
    /// width. A width check reads green on every assertion here, so this is the
    /// regression that a revert to the old dimension-only gate would reintroduce.
    #[test]
    fn a_same_dimension_model_switch_is_caught_by_the_model_stamp() {
        run_async_test(async {
            let dir = test_dir("same-dim-switch");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;

            let store = infra.semantic_search();
            let anchor = store.anchors_missing_vector(10).await.expect("query")[0].clone();
            store
                .reconcile_vectors_table(MODEL_A, 768)
                .await
                .expect("adopt under MODEL_A");

            // Switching to MODEL_B rebuilds the table at the SAME 768 width, so the
            // live column is byte-for-byte the same shape it was before.
            let discarded = store
                .recreate_vectors_table(MODEL_B, 768)
                .await
                .expect("switch to MODEL_B");
            assert_eq!(discarded, 0);
            assert_eq!(
                store.live_vector_dimension().await.expect("dim"),
                Some(768),
                "the width is unchanged — which is exactly why it cannot be the guard"
            );
            assert_eq!(
                store.live_vector_model().await.expect("stamp"),
                Some(vectors_index_epoch(MODEL_B))
            );

            // An in-flight vector from the OLD model is the right length for the live
            // column and would have been silently accepted by a dimension-only gate.
            // The stamp rejects it — as a skip, so the sweep idles rather than
            // error-looping, and the anchor stays queued for re-embedding under
            // MODEL_B.
            let stored = store
                .store_vector_if_model_matches(MODEL_A, anchor.anchor_id, &unit_vector(768, 0.5))
                .await
                .expect("a model mismatch is a skip, not a fatal error");
            assert!(!stored, "the stale-model vector must not enter MODEL_B's index");
            assert!(store
                .anchor_still_missing_vector(anchor.anchor_id)
                .await
                .expect("recheck"));

            // The batched path enforces the same rule.
            let outcomes = store
                .store_vectors_if_model_matches(
                    MODEL_A,
                    &[(anchor.anchor_id, unit_vector(768, 0.5))],
                )
                .await
                .expect("batched model mismatch is a skip");
            assert_eq!(outcomes, vec![false]);

            // Under the model the table actually names, the same vector stores.
            assert!(store
                .store_vector_if_model_matches(MODEL_B, anchor.anchor_id, &unit_vector(768, 0.5))
                .await
                .expect("matching model stores"));
        });
    }

    /// The write gate must be evaluated against the table the batch actually
    /// writes into, not against a snapshot read before the writer lock is taken.
    ///
    /// A Settings model switch (`recreate_vectors_table`) is exactly what lands in
    /// that window: the backfill worker embeds a batch under MODEL_A (~3.5s), the
    /// user picks MODEL_B in Settings, and the rebuild commits while the worker's
    /// store is still waiting for the write lock. When the two models share a width
    /// (768 = nomic / gte-modernbert), the length check cannot catch it either — so
    /// a stale-model vector lands in MODEL_B's index, the anchor is no longer in the
    /// missing set (never re-embedded), and startup reconciliation is a NO-OP
    /// because the stamp agrees with the selection. Permanent, silent contamination.
    #[test]
    fn a_switch_committing_mid_store_cannot_contaminate_the_new_index() {
        run_async_test(async {
            let dir = test_dir("switch-races-store");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;

            let store = infra.semantic_search();
            let anchor = store.anchors_missing_vector(10).await.expect("query")[0].clone();
            store
                .recreate_vectors_table(MODEL_A, 768)
                .await
                .expect("build under MODEL_A");

            // Hold the writer so the in-flight store parks between its gate check and
            // its INSERT — the window a Settings switch lands in.
            let mut switch = store.db.begin_write().await.expect("hold the writer");

            let store_for_task = store.clone();
            let anchor_id = anchor.anchor_id;
            let pending = tokio::spawn(async move {
                store_for_task
                    .store_vectors_if_model_matches(MODEL_A, &[(anchor_id, unit_vector(768, 0.5))])
                    .await
                    .expect("the store returns cleanly")
            });
            // Let the batch read the gate (the stamp still says MODEL_A) and block on
            // the write lock.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

            // The switch to MODEL_B — same 768 width — commits while the store waits.
            sqlx::query("DROP TABLE IF EXISTS search_document_vectors")
                .execute(&mut *switch)
                .await
                .expect("drop");
            sqlx::query(
                "CREATE VIRTUAL TABLE search_document_vectors USING vec0(embedding int8[768])",
            )
            .execute(&mut *switch)
            .await
            .expect("create");
            super::stamp_vectors_model(&mut switch, MODEL_B)
                .await
                .expect("stamp MODEL_B");
            switch.commit().await.expect("the switch commits");

            let outcomes = pending.await.expect("the store task joins");

            assert_eq!(
                store.live_vector_model().await.expect("stamp"),
                Some(vectors_index_epoch(MODEL_B)),
                "the table now names MODEL_B"
            );
            assert_eq!(
                outcomes,
                vec![false],
                "a MODEL_A vector must not be written into MODEL_B's index"
            );
            assert_eq!(
                store.count_vectors().await.expect("count"),
                0,
                "MODEL_B's fresh index must hold no MODEL_A vector"
            );
            assert!(
                store
                    .anchor_still_missing_vector(anchor.anchor_id)
                    .await
                    .expect("recheck"),
                "the anchor must stay queued for re-embedding under MODEL_B"
            );
        });
    }

    /// Adopting a pre-stamp table must not write a stamp decided BEFORE the writer
    /// lock was taken.
    ///
    /// Startup reconciliation runs on the deferred-startup seam — i.e. after the
    /// window is already open — so a Settings model switch can commit between
    /// reconciliation observing "unstamped, 768 wide" and its adopt-stamp landing.
    /// The stamp then names a model the table does not hold (here: a 768-model
    /// stamped on a freshly rebuilt 1024-wide table), which nothing self-heals until
    /// the next restart: the write gate rejects every batch and search silently
    /// stays keyword-only for the whole session.
    #[test]
    fn an_adopt_racing_a_switch_never_stamps_a_table_it_did_not_inspect() {
        run_async_test(async {
            let dir = test_dir("adopt-races-switch");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;

            let store = infra.semantic_search();
            let anchor = store.anchors_missing_vector(10).await.expect("query")[0].clone();
            store
                .store_vector(anchor.anchor_id, &unit_vector(768, 0.5))
                .await
                .expect("seed the pre-stamp migration table");
            assert_eq!(
                store.live_vector_model().await.expect("stamp"),
                None,
                "the migration-0039 table carries no stamp"
            );

            // The user's model switch holds the writer while startup reconciliation
            // makes its observation.
            let mut switch = store.db.begin_write().await.expect("hold the writer");

            let store_for_task = store.clone();
            let pending = tokio::spawn(async move {
                store_for_task
                    .reconcile_vectors_table(MODEL_A, 768)
                    .await
                    .expect("reconcile returns cleanly")
            });
            // Let reconciliation observe "unstamped at 768" and block on the writer.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

            // The switch to the 1024-wide model rebuilds and stamps, then commits.
            sqlx::query("DROP TABLE IF EXISTS search_document_vectors")
                .execute(&mut *switch)
                .await
                .expect("drop");
            sqlx::query(
                "CREATE VIRTUAL TABLE search_document_vectors USING vec0(embedding int8[1024])",
            )
            .execute(&mut *switch)
            .await
            .expect("create");
            super::stamp_vectors_model(&mut switch, MODEL_WIDE)
                .await
                .expect("stamp MODEL_WIDE");
            switch.commit().await.expect("the switch commits");

            pending.await.expect("the reconcile task joins");

            // Whichever way reconciliation went, the stamp must name an embedding
            // space the live table can actually hold. Both orderings are legitimate —
            // the switch's 1024 table may survive, or reconciliation's rebuild may
            // land last at 768 — but the (stamp, width) PAIR must be coherent, which
            // is the thing the pre-fix code could violate.
            let stamp = store.live_vector_model().await.expect("stamp");
            let width = store.live_vector_dimension().await.expect("dim");
            let coherent = [
                (vectors_index_epoch(MODEL_WIDE), 1024usize),
                (vectors_index_epoch(MODEL_A), 768usize),
            ];
            assert!(
                coherent
                    .iter()
                    .any(|(epoch, dim)| stamp.as_deref() == Some(epoch.as_str())
                        && width == Some(*dim)),
                "stamp {stamp:?} names an embedding space the live {width:?}-wide table cannot hold"
            );
        });
    }

    /// Reconciliation must rebuild on a stamp disagreement even when the width is
    /// identical — the startup self-heal counterpart of the write gate above.
    #[test]
    fn reconcile_rebuilds_when_only_the_model_stamp_disagrees() {
        run_async_test(async {
            let dir = test_dir("reconcile-same-dim-model");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;

            let store = infra.semantic_search();
            let anchor = store.anchors_missing_vector(10).await.expect("query")[0].clone();
            store
                .recreate_vectors_table(MODEL_A, 768)
                .await
                .expect("build under MODEL_A");
            store
                .store_vector(anchor.anchor_id, &unit_vector(768, 0.5))
                .await
                .expect("store a MODEL_A vector");

            // The selection now names MODEL_B at the same 768 width — a stuck switch
            // a dimension-only reconciler would have declared healthy.
            let discarded = store
                .reconcile_vectors_table(MODEL_B, 768)
                .await
                .expect("reconcile succeeds");
            assert_eq!(discarded, Some(1), "the stale MODEL_A vector is discarded");
            assert_eq!(
                store.live_vector_model().await.expect("stamp"),
                Some(vectors_index_epoch(MODEL_B))
            );
            assert!(store
                .anchor_still_missing_vector(anchor.anchor_id)
                .await
                .expect("recheck"));
        });
    }

    /// A rolled-back rebuild must not leave a stamp naming a table that was never
    /// built — the reason the stamp rides inside the DROP+CREATE transaction.
    #[test]
    fn the_model_stamp_and_the_table_commit_together() {
        run_async_test(async {
            let dir = test_dir("stamp-atomicity");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let store = infra.semantic_search();

            store
                .recreate_vectors_table(MODEL_A, 768)
                .await
                .expect("build under MODEL_A");

            // A rebuild that fails inside its transaction (an invalid width vec0
            // refuses) must roll BOTH the table and the stamp back to MODEL_A. This
            // half aborts at the CREATE — i.e. BEFORE `stamp_vectors_model` runs — so
            // on its own it proves the DROP rolls back, NOT that the stamp shares the
            // transaction: it stays green even if the stamp were written on a separate
            // autocommit connection. The trigger case below is what pins that.
            assert!(
                store.recreate_vectors_table(MODEL_B, 0).await.is_err(),
                "a zero-width vec0 column is rejected"
            );
            assert_eq!(
                store.live_vector_model().await.expect("stamp"),
                Some(vectors_index_epoch(MODEL_A)),
                "a rolled-back rebuild leaves the previous stamp, never the new one"
            );
            assert_eq!(store.live_vector_dimension().await.expect("dim"), Some(768));
        });
    }

    /// A rebuild that fails **after** the stamp write must roll the new table back
    /// too — the reason the stamp rides inside the DROP+CREATE transaction.
    ///
    /// The zero-width case above cannot prove this: vec0 rejects the CREATE one
    /// statement BEFORE `stamp_vectors_model` executes, so "the stamp is unchanged"
    /// holds even if the stamp were written on its own autocommit connection — the
    /// exact regression the doc comment claims to guard. Aborting the stamp itself is
    /// the only failure that lands after the DROP+CREATE, so it is the only one that
    /// observes the two committing together.
    #[test]
    fn a_rebuild_that_fails_at_the_stamp_rolls_the_table_back_too() {
        run_async_test(async {
            let dir = test_dir("stamp-rollback");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(&infra, "2026-05-17T10:00:00Z", "alpha").await;

            let store = infra.semantic_search();
            let anchor = store.anchors_missing_vector(10).await.expect("query")[0].clone();
            store
                .recreate_vectors_table(MODEL_A, 768)
                .await
                .expect("build under MODEL_A");
            assert!(store
                .store_vector_if_model_matches(MODEL_A, anchor.anchor_id, &unit_vector(768, 0.5))
                .await
                .expect("stores under MODEL_A"));

            // Fault injection at the LAST statement of the rebuild: any attempt to
            // stamp MODEL_B aborts. Both trigger kinds are installed because the stamp
            // is an upsert — an existing row takes the DO UPDATE path.
            for event in ["INSERT", "UPDATE"] {
                sqlx::query(&format!(
                    "CREATE TRIGGER fail_stamp_on_{event} BEFORE {event} ON app_settings \
                     WHEN NEW.key = '{VECTORS_MODEL_KEY}' AND NEW.value LIKE '{MODEL_B}@%' \
                     BEGIN SELECT RAISE(ABORT, 'stamp refused'); END"
                ))
                .execute(infra.pool())
                .await
                .expect("install the fault-injection trigger");
            }

            // The DROP and the CREATE both succeed inside the rebuild's transaction;
            // the stamp is what fails. Everything must go back.
            assert!(
                store.recreate_vectors_table(MODEL_B, 1024).await.is_err(),
                "a refused stamp fails the whole rebuild"
            );
            assert_eq!(
                store.live_vector_dimension().await.expect("dim"),
                Some(768),
                "the DROP+CREATE rolled back with the stamp — the old table is still live"
            );
            assert_eq!(
                store.live_vector_model().await.expect("stamp"),
                Some(vectors_index_epoch(MODEL_A)),
                "a failed rebuild leaves the previous epoch, never a half-applied one"
            );
            assert_eq!(
                store.count_vectors().await.expect("count"),
                1,
                "a failed switch must not silently discard the existing index"
            );
            assert!(!store
                .anchor_still_missing_vector(anchor.anchor_id)
                .await
                .expect("recheck"));
        });
    }

    #[test]
    fn parse_vec0_dimension_extracts_the_declared_width() {
        // The live (int8) DDL the migration + recreate now emit.
        assert_eq!(
            super::parse_vec0_dimension(
                "CREATE VIRTUAL TABLE search_document_vectors USING vec0(embedding int8[768])"
            ),
            Some(768)
        );
        // Legacy float DDL still parses (dtype-agnostic bracket parse).
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
            // Stamp the table for MODEL_A first. Without this the gated call below
            // returns `false` at the STAMP gate and never reaches the row-conditioned
            // INSERT it is supposed to be exercising — the assertion would hold
            // whether or not the anchor was deleted, i.e. it would pass for the wrong
            // reason and stop guarding the delete race entirely.
            store
                .reconcile_vectors_table(MODEL_A, 768)
                .await
                .expect("stamp the table so the gated path actually runs");

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
                .store_vector_if_model_matches(MODEL_A, anchor.anchor_id, &unit_vector(768, 0.5))
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

    /// A unit vector at angle `theta` in the (e0, e1) plane: `cos²+sin² = 1`, so
    /// it is exactly unit-length and within the [-1,1] range `vec_quantize_int8(_,
    /// 'unit')` assumes. Dot product between two such vectors is `cos(Δθ)`, so a
    /// spread of distinct angles gives an unambiguous, well-separated f32 ranking.
    fn angled_unit_vector(dim: usize, theta: f32) -> Vec<f32> {
        let mut v = vec![0.0_f32; dim];
        v[0] = theta.cos();
        v[1] = theta.sin();
        v
    }

    fn dot(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn int8_quantized_knn_preserves_the_f32_ranking_order_for_unit_vectors() {
        run_async_test(async {
            let dir = test_dir("int8-ranking-parity");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            // Five direct anchors, each given a distinct unit vector spread across
            // angles 0.0, 0.3, 0.6, 0.9, 1.2 rad in the (e0, e1) plane.
            for i in 0..5 {
                seed_frame_with_text(&infra, &format!("2026-05-17T10:0{i}:00Z"), &format!("doc {i}"))
                    .await;
            }
            let store = infra.semantic_search();
            let mut anchors = store.anchors_missing_vector(10).await.expect("query");
            anchors.sort_by_key(|a| a.anchor_id);

            let dim = 768;
            let vectors: Vec<Vec<f32>> = (0..anchors.len())
                .map(|i| angled_unit_vector(dim, i as f32 * 0.3))
                .collect();
            for (anchor, v) in anchors.iter().zip(&vectors) {
                store
                    .store_vector(anchor.anchor_id, v)
                    .await
                    .expect("vector stores (quantized to int8 on write)");
            }

            // Query at 0.65 rad: off every stored angle, so the f32 cosine ranking
            // is strict and well-separated (no ties for int8 quantization to flip).
            let query = angled_unit_vector(dim, 0.65);

            // Ground truth: anchors ordered by DESCENDING f32 cosine (== dot, unit).
            let mut expected: Vec<i64> = anchors.iter().map(|a| a.anchor_id).collect();
            expected.sort_by(|&a, &b| {
                let ia = anchors.iter().position(|x| x.anchor_id == a).unwrap();
                let ib = anchors.iter().position(|x| x.anchor_id == b).unwrap();
                dot(&query, &vectors[ib])
                    .partial_cmp(&dot(&query, &vectors[ia]))
                    .unwrap()
            });

            // The int8-quantized KNN (column, stored vectors, AND query vector all
            // int8) must return the SAME top-k ORDER as the f32 cosine ranking.
            // Rank stability is the contract — distances differ, order does not.
            let candidates =
                super::knn_in_scope_anchors(infra.pool(), &query, 200, |q| {
                    push_direct_scope(q, None)
                })
                .await
                .expect("knn succeeds");
            assert_eq!(
                candidates, expected,
                "int8 KNN order matches the f32 cosine order for unit vectors"
            );
        });
    }
}
