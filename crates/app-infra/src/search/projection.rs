use super::equivalent_reuse::project_missing_equivalent_reuse_documents_for_processing_result;
use super::grouping::frame_search_group_key;
use super::retrieval::get_audio_segment_for_search;
use super::*;

pub(crate) async fn project_processing_result_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    result: &ProcessingResult,
) -> Result<()> {
    delete_projection_for_subject_processor(
        transaction,
        &result.subject_type,
        result.subject_id,
        &result.processor,
    )
    .await?;

    if result.processor == OCR_PROCESSOR && result.subject_type == FRAME_SUBJECT_TYPE {
        project_frame_ocr_result(transaction, result).await?;
    } else if result.processor == AUDIO_TRANSCRIPTION_PROCESSOR
        && result.subject_type == AUDIO_SEGMENT_SUBJECT_TYPE
    {
        project_audio_transcription_result(transaction, result).await?;
    }

    Ok(())
}

async fn delete_projection_for_subject_processor(
    transaction: &mut Transaction<'_, Sqlite>,
    subject_type: &str,
    subject_id: i64,
    processor: &str,
) -> Result<()> {
    // Clear only the documents *this processor* previously projected for the
    // anchor, before re-projecting its fresh result. Scoping by `processor` is a
    // correctness requirement, not tidiness: an audio segment accumulates more
    // than one processor's result chain, so a completion for a *different*
    // processor (e.g. `speaker_analysis`, which projects no search documents of
    // its own) must NOT wipe the transcription's `direct` docs — doing so silently
    // drops the segment's searchable transcript until the next restart backfill.
    //
    // The anchor-id predicate still seeks the per-anchor index
    // (`search_documents_frame_idx` / `search_documents_audio_idx`); the
    // `processing_result_id IN (… WHERE processor = ?)` subquery then narrows that
    // small per-anchor row set to this processor's documents (for a frame, both
    // its own `direct` doc and any borrowed `equivalent_reuse` doc are
    // OCR-sourced, so OCR scoping still matches the whole frame set).
    match subject_type {
        FRAME_SUBJECT_TYPE => {
            sqlx::query(
                "DELETE FROM search_documents \
                 WHERE anchor_type = 'frame' AND frame_id = ?1 \
                   AND processing_result_id IN \
                       (SELECT id FROM processing_results WHERE processor = ?2)",
            )
            .bind(subject_id)
            .bind(processor)
            .execute(&mut **transaction)
            .await?;
        }
        AUDIO_SEGMENT_SUBJECT_TYPE => {
            sqlx::query(
                "DELETE FROM search_documents \
                 WHERE anchor_type = 'audio' AND audio_segment_id = ?1 \
                   AND processing_result_id IN \
                       (SELECT id FROM processing_results WHERE processor = ?2)",
            )
            .bind(subject_id)
            .bind(processor)
            .execute(&mut **transaction)
            .await?;
        }
        _ => {}
    }

    Ok(())
}

/// Apply `project` to every item in `items`, committing in
/// [`PROJECTION_COMMIT_BATCH`]-sized chunks so the **Writer Pool** lock is released
/// between batches instead of being held across the whole (potentially large)
/// backlog. A single `BEGIN IMMEDIATE` over every item would block interactive
/// start/stop writes for the run's full duration — the start/stop-stall regression
/// every backfill and the off-lock reuse fan-out share. Concentrating the chunked
/// commit here means the lock-hold bound lives in one tested place rather than
/// being re-derived (and risked) at each call site. Callers must have produced
/// `items` from a **Reader Pool** scan, so no writer lock is held while reading.
///
/// `project` returns a boxed `Send` future rather than an `AsyncFnMut`: the
/// off-lock reuse fan-out borrows the OCR `&str` into its per-item future, and an
/// `AsyncFnMut` closure that captures a borrowed reference produces a future rustc
/// cannot prove `Send`, which the spawned completion worker requires.
pub(super) async fn commit_in_batches<T, F>(
    db: &CaptureDb,
    items: &[T],
    mut project: F,
) -> Result<()>
where
    F: for<'a> FnMut(
        &'a mut Transaction<'static, Sqlite>,
        &'a T,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>,
{
    for chunk in items.chunks(PROJECTION_COMMIT_BATCH) {
        let mut transaction = db.begin_write().await?;
        for item in chunk {
            project(&mut transaction, item).await?;
        }
        transaction.commit().await?;
    }
    Ok(())
}

/// The trimmed, non-empty OCR text for a result, or `None` when there is nothing
/// to project (so callers uniformly skip empty OCR without re-deriving the rule).
pub(super) fn ocr_result_text(result: &ProcessingResult) -> Option<&str> {
    result
        .result_text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// Insert the single `direct` search document for a freshly-OCR'd frame. This is
/// O(1) — one row + one FTS row — and is the only projection cheap enough to stay
/// inside the job-completion writer transaction.
async fn insert_direct_frame_ocr_document(
    transaction: &mut Transaction<'_, Sqlite>,
    frame: &Frame,
    result: &ProcessingResult,
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

    insert_search_document(
        transaction,
        NewSearchDocument {
            anchor_type: "frame",
            frame_id: Some(frame.id),
            audio_segment_id: None,
            processing_result_id: Some(result.id),
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
            text_source_kind: "direct",
            body_text: text,
            context_text: &context_text,
        },
    )
    .await
}

async fn project_frame_ocr_result(
    transaction: &mut Transaction<'_, Sqlite>,
    result: &ProcessingResult,
) -> Result<()> {
    let Some(frame) = get_frame_for_search_in_transaction(transaction, result.subject_id).await?
    else {
        return Ok(());
    };

    delete_equivalent_reuse_projections_for_source_result(transaction, &frame).await?;

    let Some(text) = ocr_result_text(result) else {
        return Ok(());
    };

    insert_direct_frame_ocr_document(transaction, &frame, result, text).await?;

    project_equivalent_reuse_documents_for_source_frame(transaction, &frame, result.id, text).await
}

/// Project a processing result writing only the cheap, O(1) `direct` document — no
/// equivalence-reuse fan-out. `complete_job` uses this so its writer-lock hold
/// stays in the low-millisecond range; the returned [`DeferredEquivalentReuse`]
/// obliges the caller to run the expensive fan-out off-lock right after the commit.
pub(crate) async fn project_processing_result_direct_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    result: &ProcessingResult,
) -> Result<DeferredEquivalentReuse> {
    delete_projection_for_subject_processor(
        transaction,
        &result.subject_type,
        result.subject_id,
        &result.processor,
    )
    .await?;

    if result.processor == OCR_PROCESSOR && result.subject_type == FRAME_SUBJECT_TYPE {
        if let Some(frame) =
            get_frame_for_search_in_transaction(transaction, result.subject_id).await?
        {
            // Atomically clear the completing frame's OWN stale `equivalent_reuse`
            // doc here, in the same transaction as the fresh `direct` insert. The
            // expensive candidate fan-out stays deferred off-lock, but this single
            // indexed per-frame delete must stay atomic: the
            // `delete_projection_for_subject_processor` above only matches reuse
            // docs whose `processing_result_id` points at an OCR result, so a reuse
            // doc orphaned to a NULL `processing_result_id` (by a source-result
            // delete, ON DELETE SET NULL) survives it. Without this, a crash
            // between this commit and the off-lock fan-out leaves the frame
            // carrying both a fresh `direct` doc and a stale orphaned
            // `equivalent_reuse` doc — a duplicate no backfill reconciles. The
            // off-lock fan-out re-runs this delete idempotently, so it is a no-op
            // there.
            super::equivalent_reuse::delete_equivalent_reuse_projection_for_frame(
                transaction,
                frame.id,
            )
            .await?;
            if let Some(text) = ocr_result_text(result) {
                insert_direct_frame_ocr_document(transaction, &frame, result, text).await?;
            }
        }
    } else if result.processor == AUDIO_TRANSCRIPTION_PROCESSOR
        && result.subject_type == AUDIO_SEGMENT_SUBJECT_TYPE
    {
        // Audio transcription has no equivalence-reuse fan-out, so its full
        // projection is already cheap enough to stay in the completion transaction.
        project_audio_transcription_result(transaction, result).await?;
    }

    Ok(DeferredEquivalentReuse { _private: () })
}

async fn project_audio_transcription_result(
    transaction: &mut Transaction<'_, Sqlite>,
    result: &ProcessingResult,
) -> Result<()> {
    let Some(text) = result
        .result_text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return Ok(());
    };

    let segment = get_audio_segment_for_search(&mut **transaction, result.subject_id).await?;
    let Some(segment) = segment else {
        return Ok(());
    };
    let fallback_span_end_ms = audio_segment_duration_ms(&segment)?;
    let spans = transcription_spans(result, text, fallback_span_end_ms);
    for (index, span) in spans.into_iter().enumerate() {
        let span_text = span.text.trim();
        if span_text.is_empty() {
            continue;
        }
        let absolute_start_at = timestamp_plus_ms(&segment.started_at, span.start_ms)?;
        let absolute_end_at = timestamp_plus_ms(&segment.started_at, span.end_ms)?;
        let group_key = format!("audio:{}:{index}", segment.id);
        let context_text = search_context_text(None, None, Some(segment.source_kind.as_str()));
        insert_search_document(
            transaction,
            NewSearchDocument {
                anchor_type: "audio",
                frame_id: None,
                audio_segment_id: Some(segment.id),
                processing_result_id: Some(result.id),
                span_start_ms: Some(span.start_ms as i64),
                span_end_ms: Some(span.end_ms as i64),
                absolute_start_at: &absolute_start_at,
                absolute_end_at: &absolute_end_at,
                source_kind: Some(segment.source_kind.as_str()),
                session_id: &segment.source_session_id,
                app_bundle_id: None,
                app_name: None,
                app_name_search_key: None,
                window_title: None,
                group_key: &group_key,
                text_source_kind: "direct",
                body_text: span_text,
                context_text: &context_text,
            },
        )
        .await?;
    }

    Ok(())
}

pub(super) struct NewSearchDocument<'a> {
    pub(super) anchor_type: &'a str,
    pub(super) frame_id: Option<i64>,
    pub(super) audio_segment_id: Option<i64>,
    pub(super) processing_result_id: Option<i64>,
    pub(super) span_start_ms: Option<i64>,
    pub(super) span_end_ms: Option<i64>,
    pub(super) absolute_start_at: &'a str,
    pub(super) absolute_end_at: &'a str,
    pub(super) source_kind: Option<&'a str>,
    pub(super) session_id: &'a str,
    pub(super) app_bundle_id: Option<&'a str>,
    pub(super) app_name: Option<&'a str>,
    pub(super) app_name_search_key: Option<&'a str>,
    pub(super) window_title: Option<&'a str>,
    pub(super) group_key: &'a str,
    pub(super) text_source_kind: &'a str,
    pub(super) body_text: &'a str,
    pub(super) context_text: &'a str,
}

pub(super) async fn insert_search_document(
    transaction: &mut Transaction<'_, Sqlite>,
    doc: NewSearchDocument<'_>,
) -> Result<()> {
    let insert = sqlx::query(
        "INSERT INTO search_documents (\
            anchor_type, frame_id, audio_segment_id, processing_result_id, span_start_ms, span_end_ms, \
            absolute_start_at, absolute_end_at, source_kind, session_id, app_bundle_id, app_name, app_name_search_key, window_title, \
            group_key, text_source_kind, body_text, context_text\
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
    )
    .bind(doc.anchor_type)
    .bind(doc.frame_id)
    .bind(doc.audio_segment_id)
    .bind(doc.processing_result_id)
    .bind(doc.span_start_ms)
    .bind(doc.span_end_ms)
    .bind(doc.absolute_start_at)
    .bind(doc.absolute_end_at)
    .bind(doc.source_kind)
    .bind(doc.session_id)
    .bind(
        doc.app_bundle_id
            .and_then(normalize_app_bundle_id_for_search)
            .unwrap_or_default(),
    )
    .bind(doc.app_name)
    .bind(doc.app_name_search_key.unwrap_or_default())
    .bind(doc.window_title)
    .bind(doc.group_key)
    .bind(doc.text_source_kind)
    .bind(doc.body_text)
    .bind(doc.context_text)
    .execute(&mut **transaction)
    .await?;
    let rowid = insert.last_insert_rowid();

    sqlx::query(
        "INSERT INTO search_documents_fts(rowid, body_text, context_text) VALUES (?1, ?2, ?3)",
    )
    .bind(rowid)
    .bind(doc.body_text)
    .bind(doc.context_text)
    .execute(&mut **transaction)
    .await?;

    Ok(())
}

#[derive(Debug, Clone)]
struct TranscriptSpan {
    start_ms: u64,
    end_ms: u64,
    text: String,
}

fn transcription_spans(
    result: &ProcessingResult,
    fallback_text: &str,
    fallback_end_ms: u64,
) -> Vec<TranscriptSpan> {
    if let Some(payload) = result.structured_payload_json.as_deref() {
        if let Ok(metadata) = serde_json::from_str::<TranscriptionMetadata>(payload) {
            let segments = metadata
                .segments
                .into_iter()
                .filter(|segment| !segment.text.trim().is_empty())
                .map(|segment| TranscriptSpan {
                    start_ms: segment.start_ms,
                    end_ms: segment.end_ms.max(segment.start_ms),
                    text: segment.text,
                })
                .collect::<Vec<_>>();
            if !segments.is_empty() {
                return segments;
            }

            if !metadata.words.is_empty() {
                return metadata
                    .words
                    .chunks(24)
                    .filter_map(|words| {
                        let first = words.first()?;
                        let last = words.last()?;
                        Some(TranscriptSpan {
                            start_ms: first.start_ms,
                            end_ms: last.end_ms.max(first.start_ms),
                            text: words
                                .iter()
                                .map(|word| word.text.as_str())
                                .collect::<Vec<_>>()
                                .join(" "),
                        })
                    })
                    .collect();
            }
        }
    }

    vec![TranscriptSpan {
        start_ms: 0,
        end_ms: fallback_end_ms,
        text: fallback_text.to_string(),
    }]
}

fn audio_segment_duration_ms(segment: &AudioSegment) -> Result<u64> {
    let started_at = OffsetDateTime::parse(&segment.started_at, &Rfc3339).map_err(|error| {
        AppInfraError::FrameBatchFinalize(format!(
            "invalid audio segment start timestamp '{}': {error}",
            segment.started_at
        ))
    })?;
    let ended_at = OffsetDateTime::parse(&segment.ended_at, &Rfc3339).map_err(|error| {
        AppInfraError::FrameBatchFinalize(format!(
            "invalid audio segment end timestamp '{}': {error}",
            segment.ended_at
        ))
    })?;
    let duration_ms = (ended_at - started_at).whole_milliseconds().max(0);
    Ok(duration_ms.try_into().unwrap_or(u64::MAX))
}

pub(super) fn timestamp_plus_ms(started_at: &str, offset_ms: u64) -> Result<String> {
    let start = OffsetDateTime::parse(started_at, &Rfc3339).map_err(|error| {
        AppInfraError::FrameBatchFinalize(format!(
            "invalid search timestamp '{started_at}': {error}"
        ))
    })?;
    let timestamp = start
        .checked_add(Duration::milliseconds(
            offset_ms.try_into().unwrap_or(i64::MAX),
        ))
        .ok_or_else(|| {
            AppInfraError::FrameBatchFinalize("search timestamp overflow".to_string())
        })?;
    Ok(timestamp.format(&Rfc3339).map_err(|error| {
        AppInfraError::FrameBatchFinalize(format!("failed to format search timestamp: {error}"))
    })?)
}

pub(super) fn map_processing_result_for_search(row: SqliteRow) -> Result<ProcessingResult> {
    Ok(ProcessingResult {
        id: row.get("id"),
        job_id: row.get("job_id"),
        subject_type: row.get("subject_type"),
        subject_id: row.get("subject_id"),
        processor: row.get("processor"),
        result_text: row.get("result_text"),
        // OCR rows store a zstd-compressed BLOB here; inflate to the JSON string the
        // rest of the pipeline expects. Same decode boundary as `map_processing_result`.
        structured_payload_json: crate::processing::structured_payload_json_from_row(&row)?,
        processor_version: row.get("processor_version"),
        redaction_detector_version: row.get("redaction_detector_version"),
        redaction_checked_at: row.get("redaction_checked_at"),
        created_at: row.get("created_at"),
    })
}

pub(super) fn normalized_source_session_id(session_id: Option<String>) -> Option<String> {
    session_id.and_then(|session_id| {
        let trimmed = session_id.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(super) async fn backfill_missing_equivalent_reuse_projections(db: &CaptureDb) -> Result<()> {
    // Scan candidates on the Reader Pool so the costly nested-EXISTS query does
    // not run under the writer lock, then project in batched write transactions.
    let rows = sqlx::query(
        "SELECT processing_results.id, processing_results.job_id, \
                processing_results.subject_type, processing_results.subject_id, \
                processing_results.processor, processing_results.result_text, \
                processing_results.structured_payload_json, \
                processing_results.processor_version, processing_results.redaction_detector_version, \
                processing_results.redaction_checked_at, processing_results.created_at \
         FROM processing_results \
         JOIN (\
            SELECT subject_type, subject_id, processor, MAX(id) AS id \
            FROM processing_results \
            WHERE subject_type = ?1 AND processor = ?2 \
            GROUP BY subject_type, subject_id, processor\
         ) latest_results ON latest_results.id = processing_results.id \
         JOIN frames AS source_frames ON source_frames.id = processing_results.subject_id \
         WHERE LENGTH(TRIM(COALESCE(processing_results.result_text, ''))) > 0 \
           AND source_frames.equivalence_status = 'ready' \
           AND source_frames.equivalence_hint IS NOT NULL \
           AND source_frames.equivalence_proof IS NOT NULL \
           AND source_frames.equivalence_version IS NOT NULL \
           AND EXISTS (\
                SELECT 1 FROM frames AS target_frames \
                WHERE target_frames.session_id = source_frames.session_id \
                  AND target_frames.id != source_frames.id \
                  AND target_frames.equivalence_status = 'ready' \
                  AND target_frames.equivalence_hint = source_frames.equivalence_hint \
                  AND NOT EXISTS (\
                        SELECT 1 FROM search_documents AS direct_docs \
                        WHERE direct_docs.anchor_type = 'frame' \
                          AND direct_docs.frame_id = target_frames.id \
                          AND direct_docs.text_source_kind = 'direct'\
                  ) \
                  AND NOT EXISTS (\
                        SELECT 1 FROM search_documents AS reuse_docs \
                        WHERE reuse_docs.anchor_type = 'frame' \
                          AND reuse_docs.frame_id = target_frames.id \
                          AND reuse_docs.text_source_kind = 'equivalent_reuse'\
                  )\
           ) \
         ORDER BY processing_results.id ASC",
    )
    .bind(FRAME_SUBJECT_TYPE)
    .bind(OCR_PROCESSOR)
    .fetch_all(db.read())
    .await?;

    let results = rows
        .into_iter()
        .map(map_processing_result_for_search)
        .collect::<Result<Vec<_>>>()?;

    commit_in_batches(db, &results, |transaction, result| {
        Box::pin(
            project_missing_equivalent_reuse_documents_for_processing_result(transaction, result),
        )
    })
    .await?;

    Ok(())
}

pub(super) async fn backfill_missing_app_bundle_id_projection(db: &CaptureDb) -> Result<()> {
    // Scan on the Reader Pool, resolve each bundle id in Rust, then apply the
    // UPDATEs in batched write transactions (an empty string marks "resolved, no
    // bundle id" so the row is not rescanned next startup).
    let rows = sqlx::query(
        "SELECT search_documents.id, frame_metadata_snapshots.snapshot_json \
         FROM search_documents \
         JOIN frames ON frames.id = search_documents.frame_id \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE search_documents.anchor_type = 'frame' \
           AND search_documents.app_bundle_id IS NULL",
    )
    .fetch_all(db.read())
    .await?;

    let mut updates: Vec<(i64, String)> = Vec::with_capacity(rows.len());
    for row in rows {
        let bundle_id = row
            .get::<Option<String>, _>("snapshot_json")
            .map(|snapshot_json| {
                serde_json::from_str::<capture_metadata::FrameMetadataSnapshot>(&snapshot_json)
            })
            .transpose()?
            .and_then(|snapshot| snapshot.app_bundle_id)
            .and_then(|bundle_id| {
                normalize_app_bundle_id_for_search(&bundle_id).map(str::to_string)
            });
        updates.push((row.get::<i64, _>("id"), bundle_id.unwrap_or_default()));
    }

    commit_in_batches(db, &updates, |transaction, item| {
        let (id, bundle_id) = item;
        Box::pin(async move {
            sqlx::query("UPDATE search_documents SET app_bundle_id = ?1 WHERE id = ?2")
                .bind(bundle_id)
                .bind(id)
                .execute(&mut **transaction)
                .await?;
            Ok(())
        })
    })
    .await?;

    Ok(())
}

pub(super) async fn backfill_missing_app_name_search_key_projection(db: &CaptureDb) -> Result<()> {
    // Scan on the Reader Pool, derive each search key in Rust, then apply the
    // UPDATEs in batched write transactions.
    let rows = sqlx::query(
        "SELECT id, app_name \
         FROM search_documents \
         WHERE app_name_search_key IS NULL",
    )
    .fetch_all(db.read())
    .await?;

    let mut updates: Vec<(i64, String)> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row.get("id");
        let search_key = row
            .get::<Option<String>, _>("app_name")
            .as_deref()
            .and_then(normalize_app_name_for_search)
            .unwrap_or_default();
        updates.push((id, search_key));
    }

    commit_in_batches(db, &updates, |transaction, item| {
        let (id, search_key) = item;
        Box::pin(async move {
            sqlx::query("UPDATE search_documents SET app_name_search_key = ?1 WHERE id = ?2")
                .bind(search_key)
                .bind(id)
                .execute(&mut **transaction)
                .await?;
            Ok(())
        })
    })
    .await?;

    Ok(())
}

pub(super) fn search_context_text(
    app_name: Option<&str>,
    window_title: Option<&str>,
    source_kind: Option<&str>,
) -> String {
    [app_name, window_title, source_kind]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use crate::search::test_support::*;
    use crate::search::SearchCaptureRequest;
    use crate::{
        AppInfra, AudioSegmentSourceKind, NewAudioSegment, NewFrame, ProcessingJobDraft,
        ProcessingResultDraft,
    };
    use audio_transcription::{TranscriptionMetadata, TranscriptionSegment};

    #[test]
    fn search_projects_completed_ocr_and_groups_equivalent_frames() {
        run_async_test(async {
            let dir = test_dir("ocr-groups");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let first = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-frame-a.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(crate::FrameEquivalence {
                        hint: Some("same-screen".to_string()),
                        proof: Some(vec![0; 1024]),
                        version: Some(1),
                        status: Some(crate::FrameEquivalenceStatus::Ready),
                        error: None,
                    }),
                )
                .await
                .expect("first frame should insert");
            let second = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-frame-b.jpg",
                        "2026-05-17T10:00:02Z",
                    )
                    .with_equivalence(crate::FrameEquivalence {
                        hint: Some("same-screen".to_string()),
                        proof: Some(vec![0; 1024]),
                        version: Some(1),
                        status: Some(crate::FrameEquivalenceStatus::Ready),
                        error: None,
                    }),
                )
                .await
                .expect("second frame should insert");

            for frame in [&first, &second] {
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new()
                        .with_result_text("quarterly roadmap search target"),
                )
                .await;
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "roadmap".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].match_count, 2);
            assert_eq!(response.frames[0].representative_frame.id, second.id);
            assert!(response.audio.is_empty());
        });
    }

    #[test]
    fn startup_backfills_search_projection_for_existing_latest_results() {
        run_async_test(async {
            let dir = test_dir("startup-backfill");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-startup-backfill.jpg",
                    "2026-05-17T10:00:00Z",
                ))
                .await
                .expect("frame should insert");

            for text in ["old upgraded text", "fresh upgraded text"] {
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text(text),
                )
                .await;
            }

            sqlx::query("DELETE FROM search_documents")
                .execute(infra.pool())
                .await
                .expect("search documents should delete");
            drop(infra);

            let reopened = AppInfra::initialize(&dir)
                .await
                .expect("infra should reopen");
            let stale = reopened
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
                .expect("stale search should succeed");
            assert!(stale.frames.is_empty());

            let fresh = reopened
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
            assert_eq!(fresh.frames[0].representative_frame.id, frame.id);
        });
    }

    #[test]
    fn fast_initialize_defers_search_projection_backfill_until_maintenance_runs() {
        run_async_test(async {
            let dir = test_dir("fast-init-defers-backfill");

            // Seed a frame + OCR result (projected on write), then delete the
            // projection so the index needs the startup repair to be searchable.
            let frame_id;
            {
                let infra = AppInfra::initialize(&dir)
                    .await
                    .expect("infra should initialize");
                let frame = infra
                    .insert_frame(&NewFrame::new(
                        "screen-session",
                        "/tmp/fast-init-defers-backfill.jpg",
                        "2026-05-17T10:00:00Z",
                    ))
                    .await
                    .expect("frame should insert");
                frame_id = frame.id;
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text("deferred backfill text"),
                )
                .await;
                sqlx::query("DELETE FROM search_documents")
                    .execute(infra.pool())
                    .await
                    .expect("search documents should delete");
            }

            let search_request = || SearchCaptureRequest {
                query: "deferred".to_string(),
                frame_limit: Some(5),
                frame_offset: None,
                audio_limit: Some(0),
                audio_offset: None,
                snapshot_document_id: None,
                refinements: None,
                query_embedding: None,
            };

            // The fast init path opens the index but must NOT run the projection
            // backfill — that is what keeps the expensive scans off the
            // window-open critical path — so the missing projection stays missing.
            let infra = AppInfra::initialize_fast_with_processing_registry(
                &dir,
                crate::default_processing_registry(),
            )
            .await
            .expect("fast infra should initialize");
            let before = infra
                .search_capture(search_request())
                .await
                .expect("search before maintenance should succeed");
            assert!(
                before.frames.is_empty(),
                "fast init should defer the search projection backfill"
            );

            // Running startup maintenance repairs the missing projection.
            infra
                .run_startup_maintenance()
                .await
                .expect("startup maintenance should run");
            let after = infra
                .search_capture(search_request())
                .await
                .expect("search after maintenance should succeed");
            assert_eq!(after.frames.len(), 1);
            assert_eq!(after.frames[0].representative_frame.id, frame_id);
        });
    }

    #[test]
    fn startup_backfill_does_not_double_project_multi_span_audio_result() {
        run_async_test(async {
            let dir = test_dir("backfill-audio-multi-span");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            // A single audio transcription result with two segments projects two
            // `direct` search_documents (one per span) for the same
            // processing_result. This is the exact case where the
            // backfill LEFT JOIN would row-multiply if its `IS NULL` anti-join
            // guard regressed to an inner join, so it must be re-projected once.
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/backfill-audio-multi-span.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:00:20Z",
                ))
                .await
                .expect("segment should insert");
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: vec![
                    TranscriptionSegment {
                        start_ms: 1_000,
                        end_ms: 2_500,
                        text: "deferred backfill alpha".to_string(),
                        confidence: None,
                    },
                    TranscriptionSegment {
                        start_ms: 3_000,
                        end_ms: 4_500,
                        text: "deferred backfill beta".to_string(),
                        confidence: None,
                    },
                ],
                words: Vec::new(),
                provenance: Default::default(),
            };
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
                    segment.id,
                ))
                .await
                .expect("transcription job should enqueue");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new()
                    .with_result_text("deferred backfill alpha deferred backfill beta")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            // Two `direct` docs were projected on write.
            let direct_count = || async {
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM search_documents \
                     WHERE audio_segment_id = ?1 AND text_source_kind = 'direct'",
                )
                .bind(segment.id)
                .fetch_one(infra.pool())
                .await
                .expect("direct doc count should load")
            };
            assert_eq!(
                direct_count().await,
                2,
                "write path projects two direct docs"
            );

            // Drop the projection so the startup backfill must repair it.
            sqlx::query("DELETE FROM search_documents")
                .execute(infra.pool())
                .await
                .expect("search documents should delete");
            assert_eq!(direct_count().await, 0);

            // Backfill must re-project the multi-span result exactly once: two
            // direct docs total, not four.
            infra
                .run_startup_maintenance()
                .await
                .expect("startup maintenance should run");
            assert_eq!(
                direct_count().await,
                2,
                "anti-join must re-project the multi-span audio result exactly once"
            );

            // Re-running the backfill while both direct docs already exist must be
            // a no-op for this result: the anti-join must NOT re-select an
            // already-projected multi-span result and append a second copy of its
            // spans (which an inner join / dropped `IS NULL` guard would do).
            infra
                .run_startup_maintenance()
                .await
                .expect("repeat startup maintenance should run");
            assert_eq!(
                direct_count().await,
                2,
                "anti-join must not double-project an already-projected multi-span result"
            );

            // Search returns the single grouped audio result, not N duplicates.
            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "deferred".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");
            assert!(response.frames.is_empty());
            assert_eq!(
                response.audio.len(),
                1,
                "the grouped audio result must not be duplicated by the backfill"
            );
        });
    }

    #[test]
    fn startup_backfill_marks_frames_without_app_bundle_id_as_checked() {
        run_async_test(async {
            let dir = test_dir("startup-backfill-empty-app-bundle");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-empty-app-bundle.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: None,
                            app_name: Some("Notes".to_string()),
                            window_title: Some("Planning".to_string()),
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
                )
                .await
                .expect("frame should insert");
            let frame_without_metadata = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-no-metadata.jpg",
                    "2026-05-17T10:00:01Z",
                ))
                .await
                .expect("frame without metadata should insert");
            for frame_id in [frame.id, frame_without_metadata.id] {
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame_id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text("empty bundle target"),
                )
                .await;
            }
            let inserted_null_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id IN (?1, ?2) \
                   AND (app_bundle_id IS NULL OR app_name_search_key IS NULL)",
            )
            .bind(frame.id)
            .bind(frame_without_metadata.id)
            .fetch_one(infra.pool())
            .await
            .expect("inserted null count should load");
            let inserted_checked_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id IN (?1, ?2) \
                   AND app_bundle_id = '' \
                   AND app_name_search_key IS NOT NULL",
            )
            .bind(frame.id)
            .bind(frame_without_metadata.id)
            .fetch_one(infra.pool())
            .await
            .expect("inserted checked count should load");

            assert_eq!(inserted_null_count, 0);
            assert_eq!(inserted_checked_count, 2);
            drop(infra);

            let reopened = AppInfra::initialize(&dir)
                .await
                .expect("infra should reopen");
            let null_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id IN (?1, ?2) AND app_bundle_id IS NULL",
            )
            .bind(frame.id)
            .bind(frame_without_metadata.id)
            .fetch_one(reopened.pool())
            .await
            .expect("null count should load");
            let checked_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id IN (?1, ?2) AND app_bundle_id = ''",
            )
            .bind(frame.id)
            .bind(frame_without_metadata.id)
            .fetch_one(reopened.pool())
            .await
            .expect("checked count should load");

            assert_eq!(null_count, 0);
            assert_eq!(checked_count, 2);
        });
    }

    #[test]
    fn search_projects_transcript_segments_and_sanitizes_plain_query() {
        run_async_test(async {
            let dir = test_dir("audio-segments");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/search-audio.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:00:20Z",
                ))
                .await
                .expect("segment should insert");
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: vec![TranscriptionSegment {
                    start_ms: 1_000,
                    end_ms: 2_500,
                    text: "search target phrase".to_string(),
                    confidence: None,
                }],
                words: Vec::new(),
                provenance: Default::default(),
            };
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
                    segment.id,
                ))
                .await
                .expect("transcription job should enqueue");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new()
                    .with_result_text("search target phrase")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "\"target\"".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert!(response.frames.is_empty());
            assert_eq!(response.audio.len(), 1);
            assert_eq!(response.audio[0].span_start_ms, 1_000);
            assert_eq!(response.audio[0].span_end_ms, 2_500);
        });
    }

    #[test]
    fn speaker_analysis_completion_keeps_transcript_search_documents() {
        run_async_test(async {
            let dir = test_dir("audio-speaker-analysis-keeps-transcript");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/search-audio-speaker-analysis.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:00:20Z",
                ))
                .await
                .expect("segment should insert");
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: vec![TranscriptionSegment {
                    start_ms: 1_000,
                    end_ms: 2_500,
                    text: "diarized search target phrase".to_string(),
                    confidence: None,
                }],
                words: Vec::new(),
                provenance: Default::default(),
            };
            let transcription_job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
                    segment.id,
                ))
                .await
                .expect("transcription job should enqueue");
            complete_job(
                &infra,
                transcription_job,
                ProcessingResultDraft::new()
                    .with_result_text("diarized search target phrase")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            // The transcription projected its `direct` audio search document(s).
            let direct_count = || async {
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM search_documents \
                     WHERE audio_segment_id = ?1 AND anchor_type = 'audio' \
                       AND text_source_kind = 'direct'",
                )
                .bind(segment.id)
                .fetch_one(infra.pool())
                .await
                .expect("direct doc count should load")
            };
            assert_eq!(
                direct_count().await,
                1,
                "transcription projects one direct audio document"
            );

            // Completing a `speaker_analysis` job for the SAME audio segment must
            // NOT wipe the transcription's `direct` documents: it projects no
            // search documents of its own, so its completion is an index no-op.
            // The pre-fix unscoped delete dropped every audio doc for the segment
            // and re-projected nothing, silently losing the searchable transcript.
            let speaker_job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_speaker_analysis(
                    segment.id,
                ))
                .await
                .expect("speaker analysis job should enqueue");
            complete_job(&infra, speaker_job, ProcessingResultDraft::new()).await;

            assert_eq!(
                direct_count().await,
                1,
                "speaker_analysis completion must preserve the transcript's search documents"
            );

            // The transcript is still searchable after diarization completes.
            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "target".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");
            assert!(response.frames.is_empty());
            assert_eq!(response.audio.len(), 1);
            assert_eq!(response.audio[0].span_start_ms, 1_000);
            assert_eq!(response.audio[0].span_end_ms, 2_500);
        });
    }

    #[test]
    fn search_projects_untimed_transcript_fallback_over_full_audio_segment() {
        run_async_test(async {
            let dir = test_dir("audio-untimed-fallback");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/search-audio-untimed.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:00:20Z",
                ))
                .await
                .expect("segment should insert");
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: Vec::new(),
                words: Vec::new(),
                provenance: Default::default(),
            };
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
                    segment.id,
                ))
                .await
                .expect("transcription job should enqueue");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new()
                    .with_result_text("untimed search target phrase")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "untimed".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 1);
            assert_eq!(response.audio[0].span_start_ms, 0);
            assert_eq!(response.audio[0].span_end_ms, 20_000);
            assert_eq!(response.audio[0].absolute_start_at, "2026-05-17T10:00:00Z");
            assert_eq!(response.audio[0].absolute_end_at, "2026-05-17T10:00:20Z");
        });
    }

    #[test]
    fn search_indexes_frame_context_terms() {
        run_async_test(async {
            let dir = test_dir("frame-context");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-frame-context.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: Some("com.example.Linear".to_string()),
                            app_name: Some("Linear".to_string()),
                            window_title: Some("Roadmap Grooming".to_string()),
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
                )
                .await
                .expect("frame should insert");
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                .await
                .expect("ocr job should enqueue");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("ordinary body text"),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "roadmap".to_string(),
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
            assert_eq!(response.frames[0].representative_frame.id, frame.id);
            assert_eq!(
                response.frames[0].app_bundle_id.as_deref(),
                Some("com.example.Linear")
            );
            assert_eq!(response.frames[0].app_name.as_deref(), Some("Linear"));
            assert_eq!(
                response.frames[0].window_title.as_deref(),
                Some("Roadmap Grooming")
            );
        });
    }

    #[test]
    fn cascaded_search_document_deletes_remove_fts_rows() {
        run_async_test(async {
            let dir = test_dir("fts-cascade");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-fts-cascade.jpg",
                    "2026-05-17T10:00:00Z",
                ))
                .await
                .expect("frame should insert");
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                .await
                .expect("ocr job should enqueue");
            let job_id = job.id;
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("cascade target phrase"),
            )
            .await;

            let count_before: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents_fts WHERE search_documents_fts MATCH 'cascade'",
            )
            .fetch_one(infra.pool())
            .await
            .expect("fts count should query");
            assert_eq!(count_before, 1);

            sqlx::query("DELETE FROM processing_results WHERE job_id = ?1")
                .bind(job_id)
                .execute(infra.pool())
                .await
                .expect("processing result delete should cascade");

            let count_after: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents_fts WHERE search_documents_fts MATCH 'cascade'",
            )
            .fetch_one(infra.pool())
            .await
            .expect("fts count should query");
            assert_eq!(count_after, 0);
        });
    }

    #[test]
    fn replacing_search_projection_keeps_fts_delete_trigger_idempotent() {
        run_async_test(async {
            let dir = test_dir("fts-replace");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-fts-replace.jpg",
                    "2026-05-17T10:00:00Z",
                ))
                .await
                .expect("frame should insert");

            for text in ["first target phrase", "second target phrase"] {
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text(text),
                )
                .await;
            }

            let first = infra
                .search_capture(SearchCaptureRequest {
                    query: "first".to_string(),
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
            let second = infra
                .search_capture(SearchCaptureRequest {
                    query: "second".to_string(),
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

            assert!(first.frames.is_empty());
            assert_eq!(second.frames.len(), 1);
        });
    }
}
