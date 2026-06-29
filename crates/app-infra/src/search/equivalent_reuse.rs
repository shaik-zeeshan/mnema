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
    drop(read_tx);

    // Clear any stale reuse documents for this source first (covers re-projection
    // when the OCR text changed or became empty). Short, indexed deletes.
    {
        let mut transaction = db.begin_write().await?;
        delete_equivalent_reuse_projections_for_source_result(&mut transaction, &source_frame)
            .await?;
        transaction.commit().await?;
    }

    let Some(text) = ocr_result_text(result) else {
        return Ok(());
    };
    // Own the per-fan-out data so each item future borrows only its `'a`
    // transaction + frame, never this enclosing scope. A `for<'a>` batch closure
    // that captured the borrowed `text`/`result` would force them to outlive every
    // possible `'a` (including `'static`) and fail to type-check.
    let result_id = result.id;
    let text = text.to_string();

    commit_in_batches(db, &candidates, |transaction, frame| {
        let text = text.clone();
        Box::pin(async move {
            // Re-validate against the live row inside the write tx: a candidate
            // that gained its own direct projection since the read snapshot must
            // not be overwritten with reused text.
            if frame_has_projection(&mut *transaction, frame.id, "direct").await? {
                return Ok(());
            }
            project_equivalent_reuse_document_for_frame(
                &mut *transaction,
                frame,
                Some(result_id),
                &text,
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

    let Some(text) = result
        .result_text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return Ok(());
    };

    project_missing_equivalent_reuse_documents_for_source_frame(
        transaction,
        &frame,
        result.id,
        text,
    )
    .await
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
    let Some(source_doc) = sqlx::query(
        "SELECT search_documents.processing_result_id, \
                COALESCE(processing_results.result_text, search_documents.body_text) AS source_text \
         FROM search_documents \
         LEFT JOIN processing_results ON processing_results.id = search_documents.processing_result_id \
         WHERE search_documents.anchor_type = 'frame' \
           AND search_documents.frame_id = ?1 \
           AND (\
                search_documents.processing_result_id IS NULL \
                OR search_documents.processing_result_id IN (\
                    SELECT id FROM processing_results \
                    WHERE subject_type = 'frame' AND processor = ?2\
                )\
           ) \
         ORDER BY search_documents.id DESC LIMIT 1",
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
        source_doc.get("processing_result_id"),
        source_doc.get::<String, _>("source_text").trim(),
    )
    .await
}

pub(super) async fn project_equivalent_reuse_documents_for_source_frame(
    transaction: &mut Transaction<'_, Sqlite>,
    source_frame: &Frame,
    processing_result_id: i64,
    text: &str,
) -> Result<()> {
    let frames = equivalent_reuse_candidate_frames(transaction, source_frame).await?;

    for frame in frames {
        if frame_has_projection(transaction, frame.id, "direct").await? {
            continue;
        }
        project_equivalent_reuse_document_for_frame(
            transaction,
            &frame,
            Some(processing_result_id),
            text,
        )
        .await?;
    }

    Ok(())
}

async fn project_missing_equivalent_reuse_documents_for_source_frame(
    transaction: &mut Transaction<'_, Sqlite>,
    source_frame: &Frame,
    processing_result_id: i64,
    text: &str,
) -> Result<()> {
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
            text,
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

async fn project_equivalent_reuse_document_for_frame(
    transaction: &mut Transaction<'_, Sqlite>,
    frame: &Frame,
    processing_result_id: Option<i64>,
    text: &str,
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
            body_text: text,
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
        // cleanup. If the off-lock half never runs (crash / permanent error
        // between the completion commit and the fan-out), the completing frame is
        // left with BOTH a fresh `direct` doc AND its stale `equivalent_reuse` doc
        // orphaned to a NULL `processing_result_id` (by a source-result delete,
        // ON DELETE SET NULL). No startup backfill reconciles that — the reuse
        // backfill only ADDS docs for frames missing both. So the in-transaction
        // direct projection must clear the frame's own reuse doc atomically.
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

            // Delete S's processing_result: the FK cascade NULLs F's reuse doc's
            // `processing_result_id`, leaving F with a stale, *orphaned* reuse doc.
            sqlx::query("DELETE FROM processing_results WHERE job_id = ?1")
                .bind(source_job_id)
                .execute(infra.pool())
                .await
                .expect("source result delete should orphan reuse search");
            let orphan_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id = ?1 AND text_source_kind = 'equivalent_reuse' \
                   AND processing_result_id IS NULL",
            )
            .bind(target.frame.id)
            .fetch_one(infra.pool())
            .await
            .expect("orphan count should load");
            assert_eq!(orphan_count, 1, "F starts with one orphaned reuse doc");

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
            assert_eq!(response.frames[0].match_count, 2);
            assert_eq!(response.frames[0].representative_frame.id, second.frame.id);
            assert_eq!(response.frames[0].text_source_kind, "equivalent_reuse");
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
            assert_eq!(response.frames[0].match_count, 2);
            assert_eq!(response.frames[0].representative_frame.id, second.frame.id);
            assert_eq!(response.frames[0].text_source_kind, "equivalent_reuse");
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

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].match_count, 3);
            assert_eq!(response.frames[0].representative_frame.id, third.frame.id);
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
            assert_eq!(fresh.frames.len(), 1);
            assert_eq!(fresh.frames[0].match_count, 2);
            assert_eq!(fresh.frames[0].representative_frame.id, second.frame.id);
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
            assert_eq!(fresh.frames.len(), 1);
            assert_eq!(fresh.frames[0].match_count, 2);
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
            assert_eq!(target_context.frames.len(), 1);
            assert_eq!(target_context.frames[0].match_count, 1);
            assert_eq!(
                target_context.frames[0].representative_frame.id,
                second.frame.id
            );
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
            let reuse = response
                .frames
                .iter()
                .find(|result| result.text_source_kind == "equivalent_reuse")
                .expect("equivalent reuse result should exist");

            assert_eq!(reuse.representative_frame.id, target.frame.id);
            assert_eq!(reuse.secret_redaction_count, 1);
            assert!(reuse.has_secret_redactions);
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
    fn equivalent_reuse_search_survives_source_result_delete() {
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
                .expect("source processing result delete should not remove reuse search");

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

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].representative_frame.id, second.frame.id);
            assert_eq!(response.frames[0].text_source_kind, "equivalent_reuse");
        });
    }
}
