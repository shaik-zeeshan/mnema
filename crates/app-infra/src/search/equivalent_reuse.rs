use super::grouping::frame_search_group_key;
use super::*;

/// The second half of an OCR result's **Search Index Projection**, still owed once
/// the completion transaction commits: the equivalence-reuse fan-out, run off the
/// writer lock by [`DeferredEquivalentReuse::run`].
///
/// [`project_processing_result_direct_in_transaction`] hands this back so the split
/// projection is one protocol a completion path cannot half-apply — taking the
/// cheap `direct` document obliges you to the fan-out, and `#[must_use]` flags
/// dropping the obligation on the floor (which would silently lose reused text until
/// the next startup backfill reconciled it). A non-OCR result still carries an
/// obligation, whose `run` is a no-op, so every completion path stays uniform.
#[must_use = "the equivalence-reuse fan-out must run (DeferredEquivalentReuse::run) \
              after the completion transaction commits"]
pub(crate) struct DeferredEquivalentReuse {
    pub(super) _private: (),
}

impl DeferredEquivalentReuse {
    /// Run the deferred equivalence-reuse fan-out off the completion writer lock.
    /// Call this AFTER committing the transaction that carried the `direct`
    /// projection, passing the same result that was projected. A no-op for non-OCR
    /// results (audio transcription has no fan-out).
    pub(crate) async fn run(self, db: &CaptureDb, result: &ProcessingResult) -> Result<()> {
        project_equivalent_reuse_for_ocr_result_off_lock(db, result).await
    }
}

/// Fan the OCR text of `result` out onto every visually-equivalent frame, off the
/// completion writer lock. The candidate scan runs on the Reader Pool and the
/// inserts commit in [`PROJECTION_COMMIT_BATCH`] chunks so the writer lock is
/// released between batches instead of held for the whole (per-frame FTS) fan-out
/// — the multi-second hold that used to stall interactive capture start/stop. The
/// produced documents are derived data already reconciled by
/// [`backfill_missing_equivalent_reuse_projections`], so doing this after the
/// commit (rather than inside it) is crash-safe.
pub(crate) async fn project_equivalent_reuse_for_ocr_result_off_lock(
    db: &CaptureDb,
    result: &ProcessingResult,
) -> Result<()> {
    if result.processor != OCR_PROCESSOR || result.subject_type != FRAME_SUBJECT_TYPE {
        return Ok(());
    }

    // Read the source frame + equivalence candidates on the Reader Pool so no
    // writer lock is held while scanning.
    let mut read_tx = db.read().begin().await?;
    let Some(source_frame) =
        get_frame_for_search_in_transaction(&mut read_tx, result.subject_id).await?
    else {
        return Ok(());
    };
    let candidates = equivalent_reuse_candidate_frames(&mut read_tx, &source_frame).await?;
    // The canonical row reuse documents borrow their text from: the source
    // frame's `direct` projection, committed by the completion transaction before
    // this off-lock fan-out runs. If it is missing there is nothing to borrow.
    let canonical_id = frame_direct_search_document_id(&mut read_tx, source_frame.id).await?;
    drop(read_tx);
    let Some(canonical_id) = canonical_id else {
        return Ok(());
    };

    // Clear any stale reuse documents for this source first (covers re-projection
    // when the OCR text changed or became empty). Short, indexed deletes.
    {
        let mut transaction = db.begin_write().await?;
        delete_equivalent_reuse_projections_for_source_result(&mut transaction, &source_frame)
            .await?;
        transaction.commit().await?;
    }

    if ocr_result_text(result).is_none() {
        return Ok(());
    }
    let result_id = result.id;

    commit_in_batches(db, &candidates, |transaction, frame| {
        Box::pin(async move {
            // Re-validate against the live row inside the write tx: a candidate
            // that gained its own direct projection since the read snapshot must
            // not be overwritten with a reused document.
            if frame_has_projection(&mut *transaction, frame.id, "direct").await? {
                return Ok(());
            }
            project_equivalent_reuse_document_for_frame(
                &mut *transaction,
                frame,
                Some(result_id),
                canonical_id,
            )
            .await
        })
    })
    .await?;

    Ok(())
}

pub(super) async fn project_missing_equivalent_reuse_documents_for_processing_result(
    transaction: &mut Transaction<'_, Sqlite>,
    result: &ProcessingResult,
) -> Result<()> {
    let Some(frame) = get_frame_for_search_in_transaction(transaction, result.subject_id).await?
    else {
        return Ok(());
    };

    if ocr_result_text(result).is_none() {
        return Ok(());
    }

    project_missing_equivalent_reuse_documents_for_source_frame(transaction, &frame, result.id).await
}

pub(super) async fn delete_equivalent_reuse_projections_for_source_result(
    transaction: &mut Transaction<'_, Sqlite>,
    source_frame: &Frame,
) -> Result<()> {
    // Clear stale reuse documents per equivalent frame, seeking
    // `search_documents_frame_idx` (frame_id) — proven sub-millisecond. This
    // covers every current target, including ones whose `processing_result_id`
    // was set NULL when their source result was deleted (the orphaned-reuse
    // reprojection case). We deliberately do NOT also run a bulk delete keyed on
    // `processing_result_id`: that form full-scanned the multi-million-row
    // search_documents table on every OCR completion (~2.5s of writer-lock hold
    // even when it matched zero rows), and it is redundant here — current targets
    // are all covered below, and a frame that has dropped out of the equivalence
    // set is independently cleared by `delete_projection_for_subject_processor`
    // once it gains its own direct projection.
    for frame in equivalent_reuse_candidate_frames(transaction, source_frame).await? {
        sqlx::query(
            "DELETE FROM search_documents \
             WHERE text_source_kind = 'equivalent_reuse' \
               AND anchor_type = 'frame' \
               AND frame_id = ?1",
        )
        .bind(frame.id)
        .execute(&mut **transaction)
        .await?;
    }

    delete_equivalent_reuse_projection_for_frame(transaction, source_frame.id).await?;

    Ok(())
}

pub(crate) async fn project_equivalent_frame_reuse_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    frame: &Frame,
    related_frame_id: i64,
) -> Result<()> {
    // Resolve the canonical `direct` document for the related frame: if the
    // related frame is itself a reuse document, follow its own
    // `canonical_search_document_id`; otherwise it is the canonical. The new reuse
    // document borrows its text from there and inherits the canonical's
    // `processing_result_id` (so redaction lookups still resolve the source).
    let Some(canonical_doc) = sqlx::query(
        "SELECT canonical.id AS canonical_id, \
                canonical.processing_result_id AS processing_result_id \
         FROM search_documents AS related \
         JOIN search_documents AS canonical \
           ON canonical.id = COALESCE(related.canonical_search_document_id, related.id) \
         WHERE related.anchor_type = 'frame' \
           AND related.frame_id = ?1 \
           AND (\
                related.processing_result_id IS NULL \
                OR related.processing_result_id IN (\
                    SELECT id FROM processing_results \
                    WHERE subject_type = 'frame' AND processor = ?2\
                )\
           ) \
         ORDER BY related.id DESC LIMIT 1",
    )
    .bind(related_frame_id)
    .bind(OCR_PROCESSOR)
    .fetch_optional(&mut **transaction)
    .await?
    else {
        return Ok(());
    };

    project_equivalent_reuse_document_for_frame(
        transaction,
        frame,
        canonical_doc.get("processing_result_id"),
        canonical_doc.get("canonical_id"),
    )
    .await
}

pub(super) async fn project_equivalent_reuse_documents_for_source_frame(
    transaction: &mut Transaction<'_, Sqlite>,
    source_frame: &Frame,
    processing_result_id: i64,
) -> Result<()> {
    let Some(canonical_id) =
        frame_direct_search_document_id(transaction, source_frame.id).await?
    else {
        return Ok(());
    };
    let frames = equivalent_reuse_candidate_frames(transaction, source_frame).await?;

    for frame in frames {
        if frame_has_projection(transaction, frame.id, "direct").await? {
            continue;
        }
        project_equivalent_reuse_document_for_frame(
            transaction,
            &frame,
            Some(processing_result_id),
            canonical_id,
        )
        .await?;
    }

    Ok(())
}

async fn project_missing_equivalent_reuse_documents_for_source_frame(
    transaction: &mut Transaction<'_, Sqlite>,
    source_frame: &Frame,
    processing_result_id: i64,
) -> Result<()> {
    let Some(canonical_id) =
        frame_direct_search_document_id(transaction, source_frame.id).await?
    else {
        return Ok(());
    };
    let frames = equivalent_reuse_candidate_frames(transaction, source_frame).await?;

    for frame in frames {
        if frame_has_projection(transaction, frame.id, "direct").await?
            || frame_has_projection(transaction, frame.id, "equivalent_reuse").await?
        {
            continue;
        }
        project_equivalent_reuse_document_for_frame(
            transaction,
            &frame,
            Some(processing_result_id),
            canonical_id,
        )
        .await?;
    }

    Ok(())
}

async fn equivalent_reuse_candidate_frames(
    transaction: &mut Transaction<'_, Sqlite>,
    source_frame: &Frame,
) -> Result<Vec<Frame>> {
    let Some((hint, proof, version)) = source_frame.equivalence.ready_parts() else {
        return Ok(Vec::new());
    };

    let rows = sqlx::query(
        "SELECT frames.id, session_id, file_path, captured_at, width, height, \
                equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                frame_metadata_snapshots.snapshot_json AS metadata_snapshot_json, \
                frames.created_at, frames.updated_at, \
                COALESCE((\
                    SELECT COUNT(*) FROM secret_redactions \
                    WHERE secret_redactions.anchor_type = 'frame' \
                      AND secret_redactions.frame_id = frames.id\
                ), 0) AS secret_redaction_count \
         FROM frames \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE frames.session_id = ?1 \
           AND frames.id != ?2 \
           AND frames.equivalence_hint = ?3",
    )
    .bind(&source_frame.session_id)
    .bind(source_frame.id)
    .bind(hint)
    .fetch_all(&mut **transaction)
    .await?;

    let mut frames = Vec::new();
    for row in rows {
        let frame = map_frame_for_search(row)?;
        if !equivalent_reuse_scope_allows_source(&frame, source_frame) {
            continue;
        }
        let Some((_target_hint, target_proof, target_version)) = frame.equivalence.ready_parts()
        else {
            continue;
        };
        if target_version != version
            || !capture_screen::captured_frame_equivalence_proofs_match(
                version,
                proof,
                target_proof,
            )
        {
            continue;
        }
        frames.push(frame);
    }

    Ok(frames)
}

pub(super) async fn get_frame_for_search_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    frame_id: i64,
) -> Result<Option<Frame>> {
    let row = sqlx::query(
        "SELECT frames.id, session_id, file_path, captured_at, width, height, \
                equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                frame_metadata_snapshots.snapshot_json AS metadata_snapshot_json, \
                frames.created_at, frames.updated_at \
         FROM frames \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE frames.id = ?1",
    )
    .bind(frame_id)
    .fetch_optional(&mut **transaction)
    .await?;

    row.map(map_frame_for_search).transpose()
}

async fn frame_has_projection(
    transaction: &mut Transaction<'_, Sqlite>,
    frame_id: i64,
    text_source_kind: &str,
) -> Result<bool> {
    Ok(sqlx::query(
        "SELECT 1 FROM search_documents \
         WHERE search_documents.anchor_type = 'frame' \
           AND search_documents.frame_id = ?1 \
           AND search_documents.text_source_kind = ?2 \
         LIMIT 1",
    )
    .bind(frame_id)
    .bind(text_source_kind)
    .fetch_optional(&mut **transaction)
    .await?
    .is_some())
}

fn equivalent_reuse_scope_allows_source(target_frame: &Frame, source_frame: &Frame) -> bool {
    match CapturedFrameEquivalenceScope::from_frame(target_frame) {
        CapturedFrameEquivalenceScope::Session => true,
        CapturedFrameEquivalenceScope::HiddenSegmentWorkspace { frames_dir_prefix } => {
            source_frame.file_path.starts_with(&frames_dir_prefix)
        }
    }
}

/// The `search_documents.id` of a frame's `direct` projection, if it has one.
/// This is the canonical row an `equivalent_reuse` document borrows its text
/// from (`canonical_search_document_id`).
async fn frame_direct_search_document_id(
    transaction: &mut Transaction<'_, Sqlite>,
    frame_id: i64,
) -> Result<Option<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM search_documents \
         WHERE anchor_type = 'frame' AND frame_id = ?1 AND text_source_kind = 'direct' \
         ORDER BY id DESC LIMIT 1",
    )
    .bind(frame_id)
    .fetch_optional(&mut **transaction)
    .await?)
}

/// Project an `equivalent_reuse` document for `frame`. The row stores NULL
/// `body_text` and borrows the canonical `direct` row's text through
/// `canonical_search_document_id` (visually-identical frames share one copy
/// instead of duplicating it), so it is also kept out of the FTS index.
async fn project_equivalent_reuse_document_for_frame(
    transaction: &mut Transaction<'_, Sqlite>,
    frame: &Frame,
    processing_result_id: Option<i64>,
    canonical_search_document_id: i64,
) -> Result<()> {
    let (app_bundle_id, app_name, window_title) = frame
        .metadata_snapshot
        .as_ref()
        .map(|metadata| {
            (
                metadata.app_bundle_id.clone(),
                metadata.app_name.clone(),
                metadata.window_title.clone(),
            )
        })
        .unwrap_or((None, None, None));
    let group_key = frame_search_group_key(frame);
    let context_text = search_context_text(app_name.as_deref(), window_title.as_deref(), None);
    let app_name_search_key = app_name.as_deref().and_then(normalize_app_name_for_search);

    delete_equivalent_reuse_projection_for_frame(transaction, frame.id).await?;

    insert_search_document(
        transaction,
        NewSearchDocument {
            anchor_type: "frame",
            frame_id: Some(frame.id),
            audio_segment_id: None,
            processing_result_id,
            span_start_ms: None,
            span_end_ms: None,
            absolute_start_at: &frame.captured_at,
            absolute_end_at: &frame.captured_at,
            source_kind: None,
            session_id: &frame.session_id,
            app_bundle_id: app_bundle_id.as_deref(),
            app_name: app_name.as_deref(),
            app_name_search_key: app_name_search_key.as_deref(),
            window_title: window_title.as_deref(),
            group_key: &group_key,
            text_source_kind: "equivalent_reuse",
            body_text: None,
            canonical_search_document_id: Some(canonical_search_document_id),
            context_text: &context_text,
        },
    )
    .await
}

pub(super) async fn delete_equivalent_reuse_projection_for_frame(
    transaction: &mut Transaction<'_, Sqlite>,
    frame_id: i64,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM search_documents \
         WHERE text_source_kind = 'equivalent_reuse' \
           AND anchor_type = 'frame' \
           AND frame_id = ?1",
    )
    .bind(frame_id)
    .execute(&mut **transaction)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::test_support::*;
    use crate::search::SearchCaptureRequest;
    use crate::{AppInfra, NewFrame, ProcessingJobDraft, ProcessingResultDraft};

    #[test]
    fn direct_projection_in_transaction_clears_completing_frames_own_orphaned_reuse_doc() {
        // The completion path splits projection into a cheap in-transaction
        // `direct` insert plus an OFF-LOCK fan-out that owns the per-frame reuse
        // cleanup. Under text dedup a reuse doc borrows the canonical `direct`
        // row's text via `canonical_search_document_id` (ON DELETE CASCADE), so
        // deleting the source's processing_result — which drops the orphaned source
        // `direct` doc — cascades the borrowed reuse doc away (no stale orphan can
        // survive). The in-transaction direct projection must then still leave the
        // completing frame with exactly one `direct` doc and no reuse doc.
        run_async_test(async {
            let dir = test_dir("direct-intx-orphan-leak");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-intx-orphan".to_string()),
                proof: Some(vec![41; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };

            // Source frame S, OCR'd; equivalent frame F borrows S's text into a
            // real `equivalent_reuse` doc.
            let source = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/intx-orphan-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("source frame should capture");
            let source_job = source.job.expect("source frame should enqueue OCR");
            let source_job_id = source_job.id;
            complete_job(
                &infra,
                source_job,
                ProcessingResultDraft::new().with_result_text("stale orphan text"),
            )
            .await;

            let target = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/intx-orphan-target.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("target frame should capture");
            assert!(target.job.is_none());

            // Delete S's processing_result: ON DELETE SET NULL orphans S's own
            // `direct` doc, the orphan trigger drops it, and the canonical-FK
            // cascade removes F's borrowed reuse doc in lockstep — so F is left
            // with NO reuse doc at all.
            sqlx::query("DELETE FROM processing_results WHERE job_id = ?1")
                .bind(source_job_id)
                .execute(infra.pool())
                .await
                .expect("source result delete should cascade reuse search away");
            let orphan_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id = ?1 AND text_source_kind = 'equivalent_reuse'",
            )
            .bind(target.frame.id)
            .fetch_one(infra.pool())
            .await
            .expect("orphan count should load");
            assert_eq!(
                orphan_count, 0,
                "deleting the source result cascades F's borrowed reuse doc away"
            );

            // F gets its OWN fresh OCR result; run ONLY the in-transaction direct
            // projection and drop the deferred fan-out unrun — simulating a crash
            // after the completion commit but before the off-lock cleanup.
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(target.frame.id))
                .await
                .expect("ocr job should enqueue");
            sqlx::query(
                "INSERT INTO processing_results (job_id, subject_type, subject_id, processor, result_text) \
                 VALUES (?1, 'frame', ?2, 'ocr', 'fresh direct text')",
            )
            .bind(job.id)
            .bind(target.frame.id)
            .execute(infra.pool())
            .await
            .expect("processing result should insert");
            let result_id: i64 =
                sqlx::query_scalar("SELECT id FROM processing_results WHERE job_id = ?1")
                    .bind(job.id)
                    .fetch_one(infra.pool())
                    .await
                    .expect("result id should load");

            let result = crate::ProcessingResult {
                id: result_id,
                job_id: job.id,
                subject_type: "frame".to_string(),
                subject_id: target.frame.id,
                processor: "ocr".to_string(),
                result_text: Some("fresh direct text".to_string()),
                structured_payload_json: None,
                processor_version: None,
                redaction_detector_version: None,
                redaction_checked_at: None,
                created_at: "2026-05-17T10:00:02Z".to_string(),
            };

            let mut tx = infra.pool().begin().await.expect("write tx should begin");
            let deferred =
                crate::search::project_processing_result_direct_in_transaction(&mut tx, &result)
                    .await
                    .expect("direct projection should succeed");
            tx.commit().await.expect("commit should succeed");
            drop(deferred);

            let direct_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id = ?1 AND text_source_kind = 'direct'",
            )
            .bind(target.frame.id)
            .fetch_one(infra.pool())
            .await
            .expect("direct count should load");
            assert_eq!(direct_count, 1, "the fresh direct doc is projected in-transaction");

            // The completing frame's own stale orphaned reuse doc MUST be gone:
            // the off-lock fan-out cannot be relied on for it (a crash before it
            // runs leaves an unreconciled duplicate that surfaces stale OCR text).
            let stale_reuse_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id = ?1 AND text_source_kind = 'equivalent_reuse'",
            )
            .bind(target.frame.id)
            .fetch_one(infra.pool())
            .await
            .expect("reuse count should load");
            assert_eq!(
                stale_reuse_count, 0,
                "the in-transaction direct projection must atomically clear the completing \
                 frame's own stale orphaned equivalent_reuse doc"
            );
        });
    }

    #[test]
    fn startup_backfills_missing_equivalent_reuse_projection_when_direct_projection_exists() {
        run_async_test(async {
            let dir = test_dir("startup-backfill-equivalent-reuse");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-startup-reuse".to_string()),
                proof: Some(vec![31; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-startup-reuse-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("first frame should capture");
            let job = first.job.expect("first frame should enqueue OCR");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("historical reuse target"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-startup-reuse-duplicate.jpg",
                        "2026-05-17T10:00:02Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            sqlx::query(
                "DELETE FROM search_documents \
                 WHERE frame_id = ?1 AND text_source_kind = 'equivalent_reuse'",
            )
            .bind(second.frame.id)
            .execute(infra.pool())
            .await
            .expect("equivalent reuse projection should delete");

            let direct_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id = ?1 AND text_source_kind = 'direct'",
            )
            .bind(first.frame.id)
            .fetch_one(infra.pool())
            .await
            .expect("direct projection count should load");
            assert_eq!(direct_count, 1);
            drop(infra);

            let reopened = AppInfra::initialize(&dir)
                .await
                .expect("infra should reopen");

            // The startup backfill re-creates `second`'s reuse doc, now a deduped
            // row: NULL body_text borrowing `first`'s `direct` doc via the
            // canonical FK.
            let reuse_doc: (Option<String>, Option<i64>) = sqlx::query_as(
                "SELECT body_text, canonical_search_document_id FROM search_documents \
                 WHERE frame_id = ?1 AND text_source_kind = 'equivalent_reuse'",
            )
            .bind(second.frame.id)
            .fetch_one(reopened.pool())
            .await
            .expect("backfilled reuse doc should exist");
            assert!(reuse_doc.0.is_none(), "reuse doc stores NULL body_text");
            assert!(
                reuse_doc.1.is_some(),
                "reuse doc points at the canonical direct doc"
            );

            // Keyword search matches the canonical frame only — the reuse row is
            // not in FTS, so `first` (the `direct` anchor) is the single hit.
            let response = reopened
                .search_capture(SearchCaptureRequest {
                    query: "historical".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].match_count, 1);
            assert_eq!(response.frames[0].representative_frame.id, first.frame.id);
            assert_eq!(response.frames[0].text_source_kind, "direct");
        });
    }

    #[test]
    fn search_projects_ocr_skipped_equivalent_frames() {
        run_async_test(async {
            let dir = test_dir("skipped-equivalent");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-skip-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(crate::FrameEquivalence {
                        hint: Some("same-screen".to_string()),
                        proof: Some(vec![7; 1024]),
                        version: Some(1),
                        status: Some(crate::FrameEquivalenceStatus::Ready),
                        error: None,
                    }),
                    None,
                )
                .await
                .expect("first frame should capture");
            let job = first.job.expect("first frame should enqueue OCR");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("duplicate coverage target"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-skip-duplicate.jpg",
                        "2026-05-17T10:00:02Z",
                    )
                    .with_equivalence(crate::FrameEquivalence {
                        hint: Some("same-screen".to_string()),
                        proof: Some(vec![7; 1024]),
                        version: Some(1),
                        status: Some(crate::FrameEquivalenceStatus::Ready),
                        error: None,
                    }),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());
            assert_eq!(
                second
                    .ocr_admission_decision
                    .as_ref()
                    .map(|decision| decision.reason),
                Some(crate::OcrAdmissionReason::SkippedEquivalentFrame)
            );

            // The OCR-skipped duplicate still gets a deduped reuse doc (NULL
            // body_text + canonical FK), but it is not in FTS — so keyword search
            // surfaces only the canonical `direct` frame.
            let reuse_doc_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id = ?1 AND text_source_kind = 'equivalent_reuse' \
                   AND body_text IS NULL AND canonical_search_document_id IS NOT NULL",
            )
            .bind(second.frame.id)
            .fetch_one(infra.pool())
            .await
            .expect("reuse doc count should load");
            assert_eq!(reuse_doc_count, 1);

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "coverage".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].match_count, 1);
            assert_eq!(response.frames[0].representative_frame.id, first.frame.id);
            assert_eq!(response.frames[0].text_source_kind, "direct");
        });
    }

    #[test]
    fn search_projects_equivalent_reuse_through_duplicate_chain() {
        run_async_test(async {
            let dir = test_dir("skipped-equivalent-chain");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-chain".to_string()),
                proof: Some(vec![9; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-chain-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("first frame should capture");
            let job = first.job.expect("first frame should enqueue OCR");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("duplicate chain target"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-chain-second.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            let third = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-chain-third.jpg",
                        "2026-05-17T10:00:02Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("third frame should capture");
            assert!(third.job.is_none());

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "chain".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            // The two duplicates (second, third) get deduped reuse docs that
            // borrow `first`'s text but are not in FTS, so keyword search returns
            // only the canonical `first` frame.
            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].match_count, 1);
            assert_eq!(response.frames[0].representative_frame.id, first.frame.id);
            let reuse_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id IN (?1, ?2) AND text_source_kind = 'equivalent_reuse'",
            )
            .bind(second.frame.id)
            .bind(third.frame.id)
            .fetch_one(infra.pool())
            .await
            .expect("reuse count should load");
            assert_eq!(reuse_count, 2, "both duplicates carry a deduped reuse doc");
        });
    }

    #[test]
    fn source_ocr_reprojection_replaces_orphaned_equivalent_reuse_text() {
        run_async_test(async {
            let dir = test_dir("reuse-source-reproject");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-reproject".to_string()),
                proof: Some(vec![19; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-reproject-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("first frame should capture");
            let source_job = first.job.expect("first frame should enqueue OCR");
            let source_job_id = source_job.id;
            complete_job(
                &infra,
                source_job,
                ProcessingResultDraft::new().with_result_text("stale reuse target"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-reproject-target.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            sqlx::query("DELETE FROM processing_results WHERE job_id = ?1")
                .bind(source_job_id)
                .execute(infra.pool())
                .await
                .expect("source processing result delete should orphan reuse search");

            let replacement_job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(first.frame.id))
                .await
                .expect("replacement job should enqueue");
            complete_job(
                &infra,
                replacement_job,
                ProcessingResultDraft::new().with_result_text("fresh reuse target"),
            )
            .await;

            let stale = infra
                .search_capture(SearchCaptureRequest {
                    query: "stale".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("stale search should succeed");
            assert!(stale.frames.is_empty());

            let fresh = infra
                .search_capture(SearchCaptureRequest {
                    query: "fresh".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("fresh search should succeed");
            // Re-OCR reprojects `first`'s `direct` text and re-borrows it onto
            // `second`'s deduped reuse doc; only the canonical `first` is in FTS.
            assert_eq!(fresh.frames.len(), 1);
            assert_eq!(fresh.frames[0].match_count, 1);
            assert_eq!(fresh.frames[0].representative_frame.id, first.frame.id);
        });
    }

    #[test]
    fn direct_ocr_reprojection_clears_current_frames_orphaned_equivalent_reuse_text() {
        run_async_test(async {
            let dir = test_dir("reuse-current-frame-reproject");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-current-reproject".to_string()),
                proof: Some(vec![20; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-current-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("first frame should capture");
            let source_job = first.job.expect("first frame should enqueue OCR");
            let source_job_id = source_job.id;
            complete_job(
                &infra,
                source_job,
                ProcessingResultDraft::new().with_result_text("old duplicate text"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-current-target.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            sqlx::query("DELETE FROM processing_results WHERE job_id = ?1")
                .bind(source_job_id)
                .execute(infra.pool())
                .await
                .expect("source processing result delete should orphan reuse search");

            let replacement_job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(second.frame.id))
                .await
                .expect("replacement job should enqueue");
            complete_job(
                &infra,
                replacement_job,
                ProcessingResultDraft::new().with_result_text("new direct text"),
            )
            .await;

            let old = infra
                .search_capture(SearchCaptureRequest {
                    query: "old".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("old search should succeed");
            assert!(old.frames.is_empty());

            let equivalent_reuse_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id = ?1 AND text_source_kind = 'equivalent_reuse'",
            )
            .bind(second.frame.id)
            .fetch_one(infra.pool())
            .await
            .expect("reuse count should load");
            assert_eq!(equivalent_reuse_count, 0);

            let fresh = infra
                .search_capture(SearchCaptureRequest {
                    query: "new".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("fresh search should succeed");
            // `second` now owns the `direct` text; `first` borrows it through a
            // deduped reuse doc that is not in FTS, so the canonical `second` is
            // the single keyword hit.
            assert_eq!(fresh.frames.len(), 1);
            assert_eq!(fresh.frames[0].match_count, 1);
            assert_eq!(fresh.frames[0].representative_frame.id, second.frame.id);
        });
    }

    #[test]
    fn source_ocr_reprojection_to_empty_clears_equivalent_reuse_text() {
        run_async_test(async {
            let dir = test_dir("reuse-source-empty");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-empty-reproject".to_string()),
                proof: Some(vec![21; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-empty-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("first frame should capture");
            let source_job = first.job.expect("first frame should enqueue OCR");
            complete_job(
                &infra,
                source_job,
                ProcessingResultDraft::new().with_result_text("vanishing reuse target"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-empty-target.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            let replacement_job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(first.frame.id))
                .await
                .expect("replacement job should enqueue");
            complete_job(
                &infra,
                replacement_job,
                ProcessingResultDraft::new().with_result_text("   "),
            )
            .await;

            let stale = infra
                .search_capture(SearchCaptureRequest {
                    query: "vanishing".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("stale search should succeed");
            assert!(stale.frames.is_empty());
        });
    }

    #[test]
    fn equivalent_reuse_projection_uses_raw_source_ocr_text() {
        run_async_test(async {
            let dir = test_dir("reuse-raw-source-text");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-raw-source".to_string()),
                proof: Some(vec![22; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-raw-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone())
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: Some("com.example.SourceOnly".to_string()),
                            app_name: Some("SourceOnlyApp".to_string()),
                            window_title: Some("Source Window".to_string()),
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
                    None,
                )
                .await
                .expect("first frame should capture");
            let source_job = first.job.expect("first frame should enqueue OCR");
            complete_job(
                &infra,
                source_job,
                ProcessingResultDraft::new().with_result_text("shared body target"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-raw-target.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_equivalence(equivalence)
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: Some("com.example.TargetOnly".to_string()),
                            app_name: Some("TargetOnlyApp".to_string()),
                            window_title: Some("Target Window".to_string()),
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            let source_context = infra
                .search_capture(SearchCaptureRequest {
                    query: "SourceOnlyApp".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("source context search should succeed");
            assert_eq!(source_context.frames.len(), 1);
            assert_eq!(source_context.frames[0].match_count, 1);
            assert_eq!(
                source_context.frames[0].representative_frame.id,
                first.frame.id
            );

            // Dedup tradeoff: the duplicate's deduped reuse doc is not in FTS, so
            // its OWN distinct context (`TargetOnlyApp`) is no longer independently
            // keyword-searchable — only the canonical frame is indexed. (In
            // practice, visually-identical frames share the same app/window, so
            // this only loses recall on synthetic divergent metadata like here.)
            let target_context = infra
                .search_capture(SearchCaptureRequest {
                    query: "TargetOnlyApp".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("target context search should succeed");
            assert!(target_context.frames.is_empty());

            // The reuse doc still borrows the SOURCE frame's raw OCR text, now via
            // the canonical FK rather than a copy.
            let borrowed_text: String = sqlx::query_scalar(
                "SELECT canonical.body_text \
                 FROM search_documents AS reuse \
                 JOIN search_documents AS canonical \
                   ON canonical.id = reuse.canonical_search_document_id \
                 WHERE reuse.frame_id = ?1 AND reuse.text_source_kind = 'equivalent_reuse'",
            )
            .bind(second.frame.id)
            .fetch_one(infra.pool())
            .await
            .expect("borrowed canonical text should load");
            assert_eq!(borrowed_text, "shared body target");
        });
    }

    #[test]
    fn equivalent_reuse_search_reports_source_result_redactions() {
        run_async_test(async {
            let dir = test_dir("reuse-source-redactions");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-redaction-source".to_string()),
                proof: Some(vec![24; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let source = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-redaction-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("source frame should capture");
            let source_job = source.job.expect("source frame should enqueue OCR");
            complete_job(
                &infra,
                source_job,
                ProcessingResultDraft::new().with_result_text("redacted shared target"),
            )
            .await;
            let source_result_id: i64 = sqlx::query_scalar(
                "SELECT id FROM processing_results WHERE subject_type = ?1 AND subject_id = ?2 ORDER BY id DESC LIMIT 1",
            )
            .bind(FRAME_SUBJECT_TYPE)
            .bind(source.frame.id)
            .fetch_one(infra.pool())
            .await
            .expect("source result should exist");
            sqlx::query(
                "INSERT INTO secret_redactions \
                    (anchor_type, frame_id, audio_segment_id, processing_result_id, category, redacted_start, redacted_end, detector_version) \
                 VALUES ('frame', ?1, NULL, ?2, 'api_key', 0, 8, 'test')",
            )
            .bind(source.frame.id)
            .bind(source_result_id)
            .execute(infra.pool())
            .await
            .expect("source redaction should insert");

            let target = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-redaction-target.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("target frame should capture");
            assert!(target.job.is_none());

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "redacted".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");
            // Keyword search surfaces the canonical `direct` source frame (the
            // deduped reuse frame is not in FTS), which reports its own redaction.
            assert_eq!(response.frames.len(), 1);
            let result = &response.frames[0];
            assert_eq!(result.text_source_kind, "direct");
            assert_eq!(result.representative_frame.id, source.frame.id);
            assert_eq!(result.secret_redaction_count, 1);
            assert!(result.has_secret_redactions);
        });
    }

    #[test]
    fn source_ocr_projection_respects_hidden_workspace_equivalence_scope() {
        run_async_test(async {
            let dir = test_dir("reuse-hidden-scope");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-scope".to_string()),
                proof: Some(vec![23; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let source = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-hidden-scope-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                )
                .await
                .expect("source frame should insert");
            let hidden_frame_path = dir
                .join("recordings/2026/05/17/.screen-session-segment-0001/frames/frame-1.jpg")
                .to_string_lossy()
                .to_string();
            infra
                .insert_frame(
                    &NewFrame::new("screen-session", &hidden_frame_path, "2026-05-17T10:00:01Z")
                        .with_equivalence(equivalence),
                )
                .await
                .expect("hidden frame should insert");

            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(source.id))
                .await
                .expect("source job should enqueue");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("scope reuse target"),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "scope".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].match_count, 1);
            assert_eq!(response.frames[0].representative_frame.id, source.id);
            assert_eq!(response.frames[0].text_source_kind, "direct");
        });
    }

    #[test]
    fn equivalent_reuse_cascades_away_on_source_result_delete() {
        // Under text dedup the reuse doc owns no text of its own — it borrows the
        // canonical `direct` row via `canonical_search_document_id` (ON DELETE
        // CASCADE). Deleting the source's processing_result drops the orphaned
        // source `direct` doc, and the cascade removes the borrowing reuse doc with
        // it, so the cluster stops surfacing. (Pre-dedup the reuse doc carried its
        // own text copy and survived; that resilience is traded for the storage
        // win — there is no live text to keep once the source `direct` doc is gone.)
        run_async_test(async {
            let dir = test_dir("reuse-source-delete");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-screen".to_string()),
                proof: Some(vec![17; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-reuse-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("first frame should capture");
            let source_job = first.job.expect("first frame should enqueue OCR");
            let source_job_id = source_job.id;
            complete_job(
                &infra,
                source_job,
                ProcessingResultDraft::new().with_result_text("retained reuse target"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-reuse-target.jpg",
                        "2026-05-17T10:00:02Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            sqlx::query("DELETE FROM processing_results WHERE job_id = ?1")
                .bind(source_job_id)
                .execute(infra.pool())
                .await
                .expect("source processing result delete should cascade reuse search away");

            // The reuse doc cascaded away with its canonical `direct` doc: no
            // search_documents rows remain for either frame.
            let remaining: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents WHERE frame_id IN (?1, ?2)",
            )
            .bind(first.frame.id)
            .bind(second.frame.id)
            .fetch_one(infra.pool())
            .await
            .expect("remaining doc count should load");
            assert_eq!(remaining, 0);

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "retained".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert!(response.frames.is_empty());
        });
    }

    /// Capture an OCR'd source frame plus a visually-equivalent duplicate, return
    /// `(source, duplicate, source_direct_doc_id, duplicate_reuse_doc_id)`.
    async fn seed_source_and_duplicate(
        infra: &AppInfra,
        tag: u8,
        text: &str,
    ) -> (crate::Frame, crate::Frame, i64, i64) {
        let equivalence = crate::FrameEquivalence {
            hint: Some(format!("same-dedup-{tag}")),
            proof: Some(vec![tag; 1024]),
            version: Some(1),
            status: Some(crate::FrameEquivalenceStatus::Ready),
            error: None,
        };
        let source = infra
            .capture_frame(
                &NewFrame::new(
                    "screen-session",
                    &format!("/tmp/dedup-source-{tag}.jpg"),
                    "2026-05-17T10:00:00Z",
                )
                .with_equivalence(equivalence.clone()),
                None,
            )
            .await
            .expect("source frame should capture");
        complete_job(
            infra,
            source.job.expect("source frame should enqueue OCR"),
            ProcessingResultDraft::new().with_result_text(text),
        )
        .await;
        let duplicate = infra
            .capture_frame(
                &NewFrame::new(
                    "screen-session",
                    &format!("/tmp/dedup-dup-{tag}.jpg"),
                    "2026-05-17T10:00:01Z",
                )
                .with_equivalence(equivalence),
                None,
            )
            .await
            .expect("duplicate frame should capture");
        assert!(duplicate.job.is_none(), "duplicate frame skips OCR");

        let direct_id: i64 = sqlx::query_scalar(
            "SELECT id FROM search_documents WHERE frame_id = ?1 AND text_source_kind = 'direct'",
        )
        .bind(source.frame.id)
        .fetch_one(infra.pool())
        .await
        .expect("source direct doc should exist");
        let reuse_id: i64 = sqlx::query_scalar(
            "SELECT id FROM search_documents \
             WHERE frame_id = ?1 AND text_source_kind = 'equivalent_reuse'",
        )
        .bind(duplicate.frame.id)
        .fetch_one(infra.pool())
        .await
        .expect("duplicate reuse doc should exist");

        (source.frame, duplicate.frame, direct_id, reuse_id)
    }

    #[test]
    fn equivalent_reuse_stores_null_text_and_borrows_canonical() {
        run_async_test(async {
            let dir = test_dir("dedup-storage");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let (source, _dup, direct_id, reuse_id) =
                seed_source_and_duplicate(&infra, 57, "dedup target alpha").await;

            // The reuse row stores NULL body_text and points at the canonical
            // `direct` row.
            let (body_text, canonical): (Option<String>, Option<i64>) = sqlx::query_as(
                "SELECT body_text, canonical_search_document_id FROM search_documents WHERE id = ?1",
            )
            .bind(reuse_id)
            .fetch_one(infra.pool())
            .await
            .expect("reuse row should load");
            assert!(body_text.is_none(), "reuse row stores NULL body_text");
            assert_eq!(canonical, Some(direct_id), "reuse row borrows the canonical");

            // The reuse row resolves its text through the canonical FK (the same
            // COALESCE/JOIN the semantic read uses).
            let resolved: String = sqlx::query_scalar(
                "SELECT COALESCE(reuse.body_text, canonical.body_text) \
                 FROM search_documents AS reuse \
                 LEFT JOIN search_documents AS canonical \
                   ON canonical.id = reuse.canonical_search_document_id \
                 WHERE reuse.id = ?1",
            )
            .bind(reuse_id)
            .fetch_one(infra.pool())
            .await
            .expect("resolved text should load");
            assert_eq!(resolved, "dedup target alpha");

            // Keyword search matches the canonical frame only.
            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "alpha".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");
            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].match_count, 1);
            assert_eq!(response.frames[0].representative_frame.id, source.id);

            // No FTS row exists for the reuse anchor (only the canonical matched).
            let fts_rowids: Vec<i64> = sqlx::query_scalar(
                "SELECT rowid FROM search_documents_fts WHERE search_documents_fts MATCH 'alpha'",
            )
            .fetch_all(infra.pool())
            .await
            .expect("fts rowids should load");
            assert!(fts_rowids.contains(&direct_id), "canonical is indexed in FTS");
            assert!(
                !fts_rowids.contains(&reuse_id),
                "the reuse anchor has no FTS row"
            );

            // No vector exists for the reuse anchor (only `direct` anchors embed).
            let reuse_vectors: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_document_vectors WHERE rowid = ?1",
            )
            .bind(reuse_id)
            .fetch_one(infra.pool())
            .await
            .expect("reuse vector count should load");
            assert_eq!(reuse_vectors, 0);
        });
    }

    #[test]
    fn semantic_read_resolves_reuse_text_from_canonical() {
        run_async_test(async {
            let dir = test_dir("dedup-semantic-read");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let (_source, duplicate, _direct_id, reuse_id) =
                seed_source_and_duplicate(&infra, 58, "semantic dedup target").await;

            // Force a vector onto the reuse anchor (production never embeds reuse
            // rows — Slice 2) so the semantic read path actually hydrates a reuse
            // row and must resolve its text through the canonical FK.
            let blob: Vec<u8> = seeded_vector(2)
                .iter()
                .flat_map(|component| component.to_le_bytes())
                .collect();
            sqlx::query(
                "INSERT INTO search_document_vectors (rowid, embedding) \
                 VALUES (?1, vec_quantize_int8(?2, 'unit'))",
            )
            .bind(reuse_id)
            .bind(blob)
            .execute(infra.pool())
            .await
            .expect("forced reuse vector should insert");

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "unrelated".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: Some(seeded_vector(2)),
                })
                .await
                .expect("hybrid search should succeed");

            // The reuse frame surfaces by meaning and its snippet carries the
            // canonical row's text (COALESCEd through the FK), not an empty body.
            let reuse_hit = response
                .frames
                .iter()
                .find(|frame| frame.representative_frame.id == duplicate.id)
                .expect("reuse frame should surface semantically");
            assert!(reuse_hit.found_by_meaning);
            assert!(
                reuse_hit.snippet.contains("semantic dedup target"),
                "snippet resolves the canonical text, was: {:?}",
                reuse_hit.snippet
            );
        });
    }

    #[test]
    fn no_reuse_row_violates_the_dedup_invariant() {
        run_async_test(async {
            let dir = test_dir("dedup-invariant");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            // Exercise the live, off-lock and chained reuse paths.
            seed_source_and_duplicate(&infra, 59, "invariant target one").await;
            let _ = seed_source_and_duplicate(&infra, 60, "invariant target two").await;

            let violations: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE text_source_kind = 'equivalent_reuse' \
                   AND body_text IS NOT NULL \
                   AND canonical_search_document_id IS NULL",
            )
            .fetch_one(infra.pool())
            .await
            .expect("violation count should load");
            assert_eq!(violations, 0);
        });
    }
}
