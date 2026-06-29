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

async fn delete_equivalent_reuse_projection_for_frame(
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
