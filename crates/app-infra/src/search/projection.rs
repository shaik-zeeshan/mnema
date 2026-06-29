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
    _processor: &str,
) -> Result<()> {
    // Seek the per-anchor index (`search_documents_frame_idx` /
    // `search_documents_audio_idx`) on a single column instead of the old
    // cross-column `OR` + `processing_result_id IN (SELECT … WHERE processor = ?)`
    // subquery, which forced a full scan of the (now multi-million-row)
    // search_documents table — a ~3s writer-lock hold per OCR completion.
    //
    // Equivalent row set: every search document for a frame is OCR-sourced (its
    // own `direct` doc plus any borrowed `equivalent_reuse` doc), and every doc
    // for an audio segment is transcription-sourced, so scoping by the anchor id
    // matches exactly what the processor subquery selected.
    match subject_type {
        FRAME_SUBJECT_TYPE => {
            sqlx::query(
                "DELETE FROM search_documents \
                 WHERE anchor_type = 'frame' AND frame_id = ?1",
            )
            .bind(subject_id)
            .execute(&mut **transaction)
            .await?;
        }
        AUDIO_SEGMENT_SUBJECT_TYPE => {
            sqlx::query(
                "DELETE FROM search_documents \
                 WHERE anchor_type = 'audio' AND audio_segment_id = ?1",
            )
            .bind(subject_id)
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
        structured_payload_json: row.get("structured_payload_json"),
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
