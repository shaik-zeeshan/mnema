use audio_transcription::TranscriptionMetadata;
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, Executor, Row, Sqlite, SqlitePool, Transaction};
use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime};

use crate::{
    captured_frame_equivalence::CapturedFrameEquivalenceScope,
    processing::{map_frame_for_search, Frame},
    AppInfraError, AudioSegment, AudioSegmentSourceKind, ProcessingResult, Result,
    AUDIO_SEGMENT_SUBJECT_TYPE, AUDIO_TRANSCRIPTION_PROCESSOR, FRAME_SUBJECT_TYPE, OCR_PROCESSOR,
};

const DEFAULT_GROUP_LIMIT: u32 = 5;
const MAX_GROUP_LIMIT: u32 = 50;
const MIN_HIT_FETCH_LIMIT: i64 = 250;
const MAX_HIT_FETCH_LIMIT: i64 = 5_000;
const HIT_FETCH_OVERFETCH_PER_GROUP: i64 = 50;
const AUDIO_GROUP_GAP_MS: u64 = 2_000;
const AUDIO_FRAME_ALIGNMENT_WINDOW_SECONDS: i64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchCaptureRequest {
    pub query: String,
    pub frame_limit: Option<u32>,
    pub frame_offset: Option<u32>,
    pub audio_limit: Option<u32>,
    pub audio_offset: Option<u32>,
    pub snapshot_document_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchCaptureResponse {
    pub normalized_query: String,
    pub snapshot_document_id: i64,
    pub frames: Vec<FrameSearchResult>,
    pub audio: Vec<AudioSearchResult>,
    pub has_more_frames: bool,
    pub has_more_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrameSearchResult {
    pub group_key: String,
    pub representative_frame: Frame,
    pub group_start_at: String,
    pub group_end_at: String,
    pub match_count: u32,
    pub snippet: String,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub thumbnail_frame_id: i64,
    pub text_source_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioSearchResult {
    pub group_key: String,
    pub audio_segment: AudioSegment,
    pub source_kind: AudioSegmentSourceKind,
    pub span_start_ms: u64,
    pub span_end_ms: u64,
    pub absolute_start_at: String,
    pub absolute_end_at: String,
    pub match_count: u32,
    pub snippet: String,
    pub aligned_frame: Option<Frame>,
}

#[derive(Clone)]
pub struct SearchStore {
    pool: SqlitePool,
}

impl SearchStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub(crate) async fn backfill_missing_projections(&self) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT processing_results.id, processing_results.job_id, \
                    processing_results.subject_type, processing_results.subject_id, \
                    processing_results.processor, processing_results.result_text, \
                    processing_results.structured_payload_json, \
                    processing_results.processor_version, processing_results.created_at \
             FROM processing_results \
             JOIN (\
                SELECT subject_type, subject_id, processor, MAX(id) AS id \
                FROM processing_results \
                WHERE (subject_type = ?1 AND processor = ?2) \
                   OR (subject_type = ?3 AND processor = ?4) \
                GROUP BY subject_type, subject_id, processor\
             ) latest_results ON latest_results.id = processing_results.id \
             WHERE LENGTH(TRIM(COALESCE(processing_results.result_text, ''))) > 0 \
               AND NOT EXISTS (\
                    SELECT 1 FROM search_documents \
                    WHERE search_documents.text_source_kind = 'direct' \
                      AND search_documents.processing_result_id = processing_results.id\
               ) \
             ORDER BY processing_results.id ASC",
        )
        .bind(FRAME_SUBJECT_TYPE)
        .bind(OCR_PROCESSOR)
        .bind(AUDIO_SEGMENT_SUBJECT_TYPE)
        .bind(AUDIO_TRANSCRIPTION_PROCESSOR)
        .fetch_all(&mut *transaction)
        .await?;

        for row in rows {
            project_processing_result_in_transaction(
                &mut transaction,
                &map_processing_result_for_search(row)?,
            )
            .await?;
        }

        transaction.commit().await?;
        Ok(())
    }

    pub async fn search_capture(
        &self,
        request: SearchCaptureRequest,
    ) -> Result<SearchCaptureResponse> {
        let normalized_query = normalize_query(&request.query);
        if normalized_query.chars().count() < 2 {
            return Ok(SearchCaptureResponse {
                normalized_query,
                snapshot_document_id: 0,
                frames: Vec::new(),
                audio: Vec::new(),
                has_more_frames: false,
                has_more_audio: false,
            });
        }

        let fts_query = fts_query_for_plain_text(&normalized_query);
        if fts_query.is_empty() {
            return Ok(SearchCaptureResponse {
                normalized_query,
                snapshot_document_id: 0,
                frames: Vec::new(),
                audio: Vec::new(),
                has_more_frames: false,
                has_more_audio: false,
            });
        }

        let frame_limit = clamp_limit(request.frame_limit);
        let frame_offset = request.frame_offset.unwrap_or(0) as usize;
        let audio_limit = clamp_limit(request.audio_limit);
        let audio_offset = request.audio_offset.unwrap_or(0) as usize;
        let snapshot_document_id = match request.snapshot_document_id {
            Some(id) => id.max(0),
            None => fetch_search_document_high_water_mark(&self.pool).await?,
        };

        let frame_end = frame_offset.saturating_add(frame_limit as usize);
        let audio_end = audio_offset.saturating_add(audio_limit as usize);
        let all_frame_groups = if frame_limit == 0 {
            Vec::new()
        } else {
            fetch_grouped_frame_hits(
                &self.pool,
                &fts_query,
                snapshot_document_id,
                frame_offset,
                frame_limit,
            )
            .await?
        };
        let all_audio_groups = if audio_limit == 0 {
            Vec::new()
        } else {
            fetch_grouped_audio_hits(
                &self.pool,
                &fts_query,
                snapshot_document_id,
                audio_offset,
                audio_limit,
            )
            .await?
        };
        let frames = all_frame_groups
            .iter()
            .skip(frame_offset)
            .take(frame_limit as usize)
            .cloned()
            .collect::<Vec<_>>();
        let mut audio = all_audio_groups
            .iter()
            .skip(audio_offset)
            .take(audio_limit as usize)
            .cloned()
            .collect::<Vec<_>>();
        align_audio_results(&self.pool, &mut audio).await?;

        Ok(SearchCaptureResponse {
            normalized_query,
            snapshot_document_id,
            frames,
            audio,
            has_more_frames: all_frame_groups.len() > frame_end,
            has_more_audio: all_audio_groups.len() > audio_end,
        })
    }
}

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
    sqlx::query(
        "DELETE FROM search_documents \
         WHERE anchor_type = CASE WHEN ?1 = 'frame' THEN 'frame' ELSE 'audio' END \
           AND ((?1 = 'frame' AND frame_id = ?2) OR (?1 = 'audio_segment' AND audio_segment_id = ?2))\
           AND processing_result_id IN (SELECT id FROM processing_results WHERE processor = ?3)",
    )
    .bind(subject_type)
    .bind(subject_id)
    .bind(processor)
    .execute(&mut **transaction)
    .await?;

    Ok(())
}

async fn project_frame_ocr_result(
    transaction: &mut Transaction<'_, Sqlite>,
    result: &ProcessingResult,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT frames.id, session_id, file_path, captured_at, width, height, \
                equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                frame_metadata_snapshots.snapshot_json AS metadata_snapshot_json, \
                frames.created_at, frames.updated_at \
         FROM frames \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE frames.id = ?1",
    )
    .bind(result.subject_id)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(frame) = row.map(map_frame_for_search).transpose()? else {
        return Ok(());
    };

    delete_equivalent_reuse_projections_for_source_result(transaction, result, &frame).await?;

    let Some(text) = result
        .result_text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return Ok(());
    };

    let (app_name, window_title) = frame
        .metadata_snapshot
        .as_ref()
        .map(|metadata| (metadata.app_name.clone(), metadata.window_title.clone()))
        .unwrap_or((None, None));

    let group_key = frame_search_group_key(&frame);
    let context_text = search_context_text(app_name.as_deref(), window_title.as_deref(), None);

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
            app_name: app_name.as_deref(),
            window_title: window_title.as_deref(),
            group_key: &group_key,
            text_source_kind: "direct",
            body_text: text,
            context_text: &context_text,
        },
    )
    .await?;

    project_equivalent_reuse_documents_for_source_frame(transaction, &frame, result.id, text).await
}

async fn delete_equivalent_reuse_projections_for_source_result(
    transaction: &mut Transaction<'_, Sqlite>,
    result: &ProcessingResult,
    source_frame: &Frame,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM search_documents \
         WHERE text_source_kind = 'equivalent_reuse' \
           AND processing_result_id IN (\
                SELECT id FROM processing_results \
                WHERE subject_type = ?1 AND subject_id = ?2 AND processor = ?3\
           )",
    )
    .bind(&result.subject_type)
    .bind(result.subject_id)
    .bind(&result.processor)
    .execute(&mut **transaction)
    .await?;

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

async fn project_equivalent_reuse_documents_for_source_frame(
    transaction: &mut Transaction<'_, Sqlite>,
    source_frame: &Frame,
    processing_result_id: i64,
    text: &str,
) -> Result<()> {
    let frames = equivalent_reuse_candidate_frames(transaction, source_frame).await?;

    for frame in frames {
        let has_direct_projection = sqlx::query(
            "SELECT 1 FROM search_documents \
             WHERE search_documents.anchor_type = 'frame' \
               AND search_documents.frame_id = ?1 \
               AND search_documents.text_source_kind = 'direct' \
             LIMIT 1",
        )
        .bind(frame.id)
        .fetch_optional(&mut **transaction)
        .await?
        .is_some();
        if has_direct_projection {
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
                frames.created_at, frames.updated_at \
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
    let (app_name, window_title) = frame
        .metadata_snapshot
        .as_ref()
        .map(|metadata| (metadata.app_name.clone(), metadata.window_title.clone()))
        .unwrap_or((None, None));
    let group_key = frame_search_group_key(frame);
    let context_text = search_context_text(app_name.as_deref(), window_title.as_deref(), None);

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
            app_name: app_name.as_deref(),
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
                app_name: None,
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

struct NewSearchDocument<'a> {
    anchor_type: &'a str,
    frame_id: Option<i64>,
    audio_segment_id: Option<i64>,
    processing_result_id: Option<i64>,
    span_start_ms: Option<i64>,
    span_end_ms: Option<i64>,
    absolute_start_at: &'a str,
    absolute_end_at: &'a str,
    source_kind: Option<&'a str>,
    session_id: &'a str,
    app_name: Option<&'a str>,
    window_title: Option<&'a str>,
    group_key: &'a str,
    text_source_kind: &'a str,
    body_text: &'a str,
    context_text: &'a str,
}

async fn insert_search_document(
    transaction: &mut Transaction<'_, Sqlite>,
    doc: NewSearchDocument<'_>,
) -> Result<()> {
    let insert = sqlx::query(
        "INSERT INTO search_documents (\
            anchor_type, frame_id, audio_segment_id, processing_result_id, span_start_ms, span_end_ms, \
            absolute_start_at, absolute_end_at, source_kind, session_id, app_name, window_title, \
            group_key, text_source_kind, body_text, context_text\
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
    .bind(doc.app_name)
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

fn timestamp_plus_ms(started_at: &str, offset_ms: u64) -> Result<String> {
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

fn normalize_query(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fts_query_for_plain_text(query: &str) -> String {
    let terms = query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>();
    let mut searchable_terms = terms
        .iter()
        .copied()
        .filter(|term| term.chars().count() >= 2)
        .collect::<Vec<_>>();
    if searchable_terms.is_empty() && query.chars().count() >= 2 {
        searchable_terms = terms;
    }
    searchable_terms
        .into_iter()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

fn clamp_limit(limit: Option<u32>) -> u32 {
    if limit == Some(0) {
        return 0;
    }
    limit
        .unwrap_or(DEFAULT_GROUP_LIMIT)
        .clamp(1, MAX_GROUP_LIMIT)
}

fn hit_fetch_limit(offset: usize, limit: u32) -> i64 {
    let requested_groups = offset
        .saturating_add(limit as usize)
        .saturating_add(1)
        .min((MAX_HIT_FETCH_LIMIT / HIT_FETCH_OVERFETCH_PER_GROUP) as usize);
    ((requested_groups as i64) * HIT_FETCH_OVERFETCH_PER_GROUP)
        .max(MIN_HIT_FETCH_LIMIT)
        .min(MAX_HIT_FETCH_LIMIT)
}

fn frame_search_group_key(frame: &Frame) -> String {
    frame
        .equivalence
        .ready_parts()
        .map(|(hint, proof, version)| {
            let scope = frame_search_group_scope_identity(frame);
            format!(
                "frame:eq:{}:{version}:{hint}:{}:{scope}",
                frame.session_id,
                proof_identity(proof)
            )
        })
        .unwrap_or_else(|| format!("frame:{}", frame.id))
}

fn frame_search_group_scope_identity(frame: &Frame) -> String {
    match CapturedFrameEquivalenceScope::from_frame(frame) {
        CapturedFrameEquivalenceScope::Session => "scope:session".to_string(),
        CapturedFrameEquivalenceScope::HiddenSegmentWorkspace { frames_dir_prefix } => {
            format!(
                "scope:hidden:{}",
                proof_identity(frames_dir_prefix.as_bytes())
            )
        }
    }
}

fn proof_identity(proof: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in proof {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn search_context_text(
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

#[derive(Debug, Clone)]
struct FrameHit {
    group_key: String,
    frame: Frame,
    snippet: String,
    rank: f64,
    app_name: Option<String>,
    window_title: Option<String>,
    text_source_kind: String,
}

#[derive(Debug, Clone)]
struct AudioHit {
    audio_segment: AudioSegment,
    source_kind: AudioSegmentSourceKind,
    span_start_ms: u64,
    span_end_ms: u64,
    snippet: String,
    rank: f64,
}

async fn fetch_search_document_high_water_mark(pool: &SqlitePool) -> Result<i64> {
    let row =
        sqlx::query("SELECT COALESCE(MAX(id), 0) AS snapshot_document_id FROM search_documents")
            .fetch_one(pool)
            .await?;
    Ok(row.get("snapshot_document_id"))
}

async fn fetch_frame_hits(
    pool: &SqlitePool,
    fts_query: &str,
    snapshot_document_id: i64,
    hit_offset: i64,
    hit_limit: i64,
) -> Result<Vec<FrameHit>> {
    let rows = sqlx::query(
        "SELECT search_documents.group_key, search_documents.app_name, search_documents.window_title, \
                search_documents.text_source_kind, \
                CASE \
                    WHEN instr(snippet(search_documents_fts, 0, '<mark>', '</mark>', '...', 12), '<mark>') > 0 \
                    THEN snippet(search_documents_fts, 0, '<mark>', '</mark>', '...', 12) \
                    ELSE snippet(search_documents_fts, 1, '<mark>', '</mark>', '...', 12) \
                END AS snippet, \
                bm25(search_documents_fts, 5.0, 1.0) AS rank, \
                frames.id, frames.session_id, frames.file_path, frames.captured_at, frames.width, frames.height, \
                frames.equivalence_hint, frames.equivalence_proof, frames.equivalence_version, \
                frames.equivalence_status, frames.equivalence_error, \
                frame_metadata_snapshots.snapshot_json AS metadata_snapshot_json, \
                frames.created_at, frames.updated_at \
         FROM search_documents_fts \
         JOIN search_documents ON search_documents.id = search_documents_fts.rowid \
         JOIN frames ON frames.id = search_documents.frame_id \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE search_documents_fts MATCH ?1 \
           AND search_documents.anchor_type = 'frame' \
           AND search_documents.id <= ?2 \
         ORDER BY rank ASC, search_documents.absolute_start_at DESC, search_documents.id DESC
         LIMIT ?3 OFFSET ?4",
    )
    .bind(fts_query)
    .bind(snapshot_document_id)
    .bind(hit_limit)
    .bind(hit_offset)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(FrameHit {
                group_key: row.get("group_key"),
                app_name: row.get("app_name"),
                window_title: row.get("window_title"),
                text_source_kind: row.get("text_source_kind"),
                snippet: row.get("snippet"),
                rank: row.get("rank"),
                frame: map_frame_for_search(row)?,
            })
        })
        .collect()
}

async fn fetch_grouped_frame_hits(
    pool: &SqlitePool,
    fts_query: &str,
    snapshot_document_id: i64,
    offset: usize,
    limit: u32,
) -> Result<Vec<FrameSearchResult>> {
    let needed_groups = offset.saturating_add(limit as usize).saturating_add(1);
    let mut hit_limit = hit_fetch_limit(offset, limit);
    let mut hit_offset = 0_i64;
    let mut all_hits = Vec::new();
    loop {
        let hits =
            fetch_frame_hits(pool, fts_query, snapshot_document_id, hit_offset, hit_limit).await?;
        let hit_count = hits.len() as i64;
        all_hits.extend(hits);
        let groups = group_frame_hits(&all_hits);
        if groups.len() >= needed_groups || hit_count < hit_limit {
            return Ok(groups);
        }
        hit_offset = hit_offset.saturating_add(hit_count);
        hit_limit = MAX_HIT_FETCH_LIMIT;
    }
}

async fn fetch_audio_hits(
    pool: &SqlitePool,
    fts_query: &str,
    snapshot_document_id: i64,
    hit_offset: i64,
    hit_limit: i64,
) -> Result<Vec<AudioHit>> {
    let rows = sqlx::query(
        "SELECT search_documents.id AS document_id, search_documents.group_key, \
                search_documents.span_start_ms, search_documents.span_end_ms, \
                search_documents.absolute_start_at, search_documents.absolute_end_at, \
                CASE \
                    WHEN instr(snippet(search_documents_fts, 0, '<mark>', '</mark>', '...', 12), '<mark>') > 0 \
                    THEN snippet(search_documents_fts, 0, '<mark>', '</mark>', '...', 12) \
                    ELSE snippet(search_documents_fts, 1, '<mark>', '</mark>', '...', 12) \
                END AS snippet, \
                bm25(search_documents_fts, 5.0, 1.0) AS rank, \
                audio_segments.id, audio_segments.source_kind, audio_segments.source_session_id, \
                audio_segments.segment_index, audio_segments.file_path, audio_segments.started_at, \
                audio_segments.ended_at, audio_segments.capture_segment_id, audio_segments.created_at, audio_segments.updated_at \
         FROM search_documents_fts \
         JOIN search_documents ON search_documents.id = search_documents_fts.rowid \
         JOIN audio_segments ON audio_segments.id = search_documents.audio_segment_id \
         WHERE search_documents_fts MATCH ?1 \
           AND search_documents.anchor_type = 'audio' \
           AND search_documents.id <= ?2 \
         ORDER BY rank ASC, search_documents.absolute_start_at DESC, search_documents.id DESC
         LIMIT ?3 OFFSET ?4",
    )
    .bind(fts_query)
    .bind(snapshot_document_id)
    .bind(hit_limit)
    .bind(hit_offset)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(map_audio_hit).collect()
}

async fn fetch_grouped_audio_hits(
    pool: &SqlitePool,
    fts_query: &str,
    snapshot_document_id: i64,
    offset: usize,
    limit: u32,
) -> Result<Vec<AudioSearchResult>> {
    let needed_groups = offset.saturating_add(limit as usize).saturating_add(1);
    let mut hit_limit = hit_fetch_limit(offset, limit);
    let mut hit_offset = 0_i64;
    let mut all_hits = Vec::new();
    loop {
        let hits =
            fetch_audio_hits(pool, fts_query, snapshot_document_id, hit_offset, hit_limit).await?;
        let hit_count = hits.len() as i64;
        all_hits.extend(hits);
        let groups = group_audio_hits(&all_hits)?;
        if groups.len() >= needed_groups || hit_count < hit_limit {
            return Ok(groups);
        }
        hit_offset = hit_offset.saturating_add(hit_count);
        hit_limit = MAX_HIT_FETCH_LIMIT;
    }
}

fn group_frame_hits(hits: &[FrameHit]) -> Vec<FrameSearchResult> {
    let mut groups: Vec<(String, Vec<FrameHit>)> = Vec::new();
    for hit in hits {
        let group_index = groups.iter().position(|(_group_key, group_hits)| {
            group_hits
                .first()
                .is_some_and(|representative| frame_hits_are_equivalent(representative, &hit))
        });
        if let Some(index) = group_index {
            groups[index].1.push(hit.clone());
        } else {
            groups.push((hit.group_key.clone(), vec![hit.clone()]));
        }
    }

    let mut results = groups
        .into_iter()
        .filter_map(|(group_key, mut hits)| {
            hits.sort_by(|a, b| {
                a.rank
                    .total_cmp(&b.rank)
                    .then_with(|| b.frame.captured_at.cmp(&a.frame.captured_at))
            });
            let representative = hits
                .iter()
                .max_by(|a, b| a.frame.captured_at.cmp(&b.frame.captured_at))?;
            let group_start_at = hits
                .iter()
                .map(|hit| hit.frame.captured_at.as_str())
                .min()
                .unwrap_or(representative.frame.captured_at.as_str())
                .to_string();
            let group_end_at = hits
                .iter()
                .map(|hit| hit.frame.captured_at.as_str())
                .max()
                .unwrap_or(representative.frame.captured_at.as_str())
                .to_string();
            let best_rank = hits
                .iter()
                .map(|hit| hit.rank)
                .min_by(|a, b| a.total_cmp(b))
                .unwrap_or(f64::INFINITY);
            Some((
                best_rank,
                FrameSearchResult {
                    group_key,
                    representative_frame: representative.frame.clone(),
                    group_start_at,
                    group_end_at,
                    match_count: hits.len() as u32,
                    snippet: hits[0].snippet.clone(),
                    app_name: representative.app_name.clone(),
                    window_title: representative.window_title.clone(),
                    thumbnail_frame_id: representative.frame.id,
                    text_source_kind: representative.text_source_kind.clone(),
                },
            ))
        })
        .collect::<Vec<_>>();

    results.sort_by(|(a_rank, a), (b_rank, b)| {
        a_rank
            .total_cmp(b_rank)
            .then_with(|| b.group_end_at.cmp(&a.group_end_at))
    });
    results.into_iter().map(|(_rank, result)| result).collect()
}

fn frame_hits_are_equivalent(left: &FrameHit, right: &FrameHit) -> bool {
    if left.frame.session_id != right.frame.session_id {
        return false;
    }

    let Some((_left_hint, left_proof, left_version)) = left.frame.equivalence.ready_parts() else {
        return left.frame.id == right.frame.id;
    };
    let Some((_right_hint, right_proof, right_version)) = right.frame.equivalence.ready_parts()
    else {
        return false;
    };
    CapturedFrameEquivalenceScope::from_frame(&left.frame)
        == CapturedFrameEquivalenceScope::from_frame(&right.frame)
        && left_version == right_version
        && capture_screen::captured_frame_equivalence_proofs_match(
            left_version,
            left_proof,
            right_proof,
        )
}

fn group_audio_hits(hits: &[AudioHit]) -> Result<Vec<AudioSearchResult>> {
    let mut hits = hits.to_vec();
    hits.sort_by(|a, b| {
        a.audio_segment
            .id
            .cmp(&b.audio_segment.id)
            .then_with(|| a.span_start_ms.cmp(&b.span_start_ms))
            .then_with(|| a.span_end_ms.cmp(&b.span_end_ms))
            .then_with(|| a.rank.total_cmp(&b.rank))
    });

    let mut groups: Vec<Vec<AudioHit>> = Vec::new();
    for hit in hits {
        if let Some(last_group) = groups.last_mut() {
            if let Some(last) = last_group.last() {
                if last.audio_segment.id == hit.audio_segment.id
                    && hit.span_start_ms <= last.span_end_ms.saturating_add(AUDIO_GROUP_GAP_MS)
                {
                    last_group.push(hit);
                    continue;
                }
            }
        }
        groups.push(vec![hit]);
    }

    let mut results = Vec::new();
    for mut group in groups {
        group.sort_by(|a, b| {
            a.rank
                .total_cmp(&b.rank)
                .then_with(|| a.span_start_ms.cmp(&b.span_start_ms))
        });
        let first = group.first().expect("group should not be empty");
        let span_start_ms = group.iter().map(|hit| hit.span_start_ms).min().unwrap_or(0);
        let span_end_ms = group
            .iter()
            .map(|hit| hit.span_end_ms)
            .max()
            .unwrap_or(span_start_ms);
        let absolute_start_at = timestamp_plus_ms(&first.audio_segment.started_at, span_start_ms)?;
        let absolute_end_at = timestamp_plus_ms(&first.audio_segment.started_at, span_end_ms)?;
        results.push((
            first.rank,
            AudioSearchResult {
                group_key: format!(
                    "audio:{}:{}-{}",
                    first.audio_segment.id, span_start_ms, span_end_ms
                ),
                audio_segment: first.audio_segment.clone(),
                source_kind: first.source_kind.clone(),
                span_start_ms,
                span_end_ms,
                absolute_start_at,
                absolute_end_at,
                match_count: group.len() as u32,
                snippet: first.snippet.clone(),
                aligned_frame: None,
            },
        ));
    }

    results.sort_by(|(a_rank, a), (b_rank, b)| {
        a_rank
            .total_cmp(b_rank)
            .then_with(|| b.absolute_start_at.cmp(&a.absolute_start_at))
            .then_with(|| a.group_key.cmp(&b.group_key))
    });
    Ok(results.into_iter().map(|(_rank, result)| result).collect())
}

fn map_processing_result_for_search(row: SqliteRow) -> Result<ProcessingResult> {
    Ok(ProcessingResult {
        id: row.get("id"),
        job_id: row.get("job_id"),
        subject_type: row.get("subject_type"),
        subject_id: row.get("subject_id"),
        processor: row.get("processor"),
        result_text: row.get("result_text"),
        structured_payload_json: row.get("structured_payload_json"),
        processor_version: row.get("processor_version"),
        created_at: row.get("created_at"),
    })
}

async fn align_audio_results(pool: &SqlitePool, results: &mut [AudioSearchResult]) -> Result<()> {
    for result in results {
        result.aligned_frame = find_aligned_frame(
            pool,
            &result.audio_segment.source_session_id,
            &result.absolute_start_at,
        )
        .await?;
    }
    Ok(())
}

async fn find_aligned_frame(
    pool: &SqlitePool,
    session_id: &str,
    absolute_start_at: &str,
) -> Result<Option<Frame>> {
    let target = OffsetDateTime::parse(absolute_start_at, &Rfc3339).map_err(|error| {
        AppInfraError::FrameBatchFinalize(format!(
            "invalid search timestamp '{absolute_start_at}': {error}"
        ))
    })?;
    let before_start = target
        .checked_sub(Duration::seconds(AUDIO_FRAME_ALIGNMENT_WINDOW_SECONDS))
        .ok_or_else(|| {
            AppInfraError::FrameBatchFinalize("search alignment timestamp overflow".to_string())
        })?
        .format(&Rfc3339)
        .map_err(|error| {
            AppInfraError::FrameBatchFinalize(format!("failed to format search timestamp: {error}"))
        })?;
    let after_end = target
        .checked_add(Duration::seconds(AUDIO_FRAME_ALIGNMENT_WINDOW_SECONDS))
        .ok_or_else(|| {
            AppInfraError::FrameBatchFinalize("search alignment timestamp overflow".to_string())
        })?
        .format(&Rfc3339)
        .map_err(|error| {
            AppInfraError::FrameBatchFinalize(format!("failed to format search timestamp: {error}"))
        })?;

    let before = sqlx::query(
        "SELECT frames.id, session_id, file_path, captured_at, width, height, \
                equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                frame_metadata_snapshots.snapshot_json AS metadata_snapshot_json, \
                frames.created_at, frames.updated_at \
         FROM frames \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE session_id = ?1 AND captured_at >= ?2 AND captured_at <= ?3 \
         ORDER BY captured_at DESC, frames.id DESC LIMIT 1",
    )
    .bind(session_id)
    .bind(before_start)
    .bind(absolute_start_at)
    .fetch_optional(pool)
    .await?;
    if let Some(row) = before {
        return map_frame_for_search(row).map(Some);
    }

    let after = sqlx::query(
        "SELECT frames.id, session_id, file_path, captured_at, width, height, \
                equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                frame_metadata_snapshots.snapshot_json AS metadata_snapshot_json, \
                frames.created_at, frames.updated_at \
         FROM frames \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE session_id = ?1 AND captured_at > ?2 AND captured_at <= ?3 \
         ORDER BY captured_at ASC, frames.id ASC LIMIT 1",
    )
    .bind(session_id)
    .bind(absolute_start_at)
    .bind(after_end)
    .fetch_optional(pool)
    .await?;

    after.map(map_frame_for_search).transpose()
}

async fn get_audio_segment_for_search<'e, E>(
    executor: E,
    audio_segment_id: i64,
) -> Result<Option<AudioSegment>>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query(
        "SELECT id, source_kind, source_session_id, segment_index, file_path, started_at, ended_at, \
                capture_segment_id, created_at, updated_at \
         FROM audio_segments WHERE id = ?1",
    )
    .bind(audio_segment_id)
    .fetch_optional(executor)
    .await?;

    row.map(map_audio_segment_for_search).transpose()
}

fn map_audio_hit(row: SqliteRow) -> Result<AudioHit> {
    let source_kind =
        AudioSegmentSourceKind::from_str(row.get::<String, _>("source_kind").as_str());
    let audio_segment = AudioSegment {
        id: row.get("id"),
        source_kind: source_kind.clone(),
        source_session_id: row.get("source_session_id"),
        segment_index: row.get("segment_index"),
        file_path: row.get("file_path"),
        started_at: row.get("started_at"),
        ended_at: row.get("ended_at"),
        capture_segment_id: row.get("capture_segment_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    };
    Ok(AudioHit {
        audio_segment,
        source_kind,
        span_start_ms: row
            .get::<Option<i64>, _>("span_start_ms")
            .unwrap_or(0)
            .max(0) as u64,
        span_end_ms: row.get::<Option<i64>, _>("span_end_ms").unwrap_or(0).max(0) as u64,
        snippet: row.get("snippet"),
        rank: row.get("rank"),
    })
}

fn map_audio_segment_for_search(row: SqliteRow) -> Result<AudioSegment> {
    Ok(AudioSegment {
        id: row.get("id"),
        source_kind: AudioSegmentSourceKind::from_str(row.get::<String, _>("source_kind").as_str()),
        source_session_id: row.get("source_session_id"),
        segment_index: row.get("segment_index"),
        file_path: row.get("file_path"),
        started_at: row.get("started_at"),
        ended_at: row.get("ended_at"),
        capture_segment_id: row.get("capture_segment_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::{AppInfra, NewAudioSegment, NewFrame, ProcessingJobDraft, ProcessingResultDraft};
    use audio_transcription::{TranscriptionMetadata, TranscriptionSegment};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir(name: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("mnema-search-{name}-{}-{id}", std::process::id()))
    }

    async fn complete_job(
        infra: &AppInfra,
        job: crate::ProcessingJob,
        result: ProcessingResultDraft,
    ) {
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

    fn run_async_test(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

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
                })
                .await
                .expect("fresh search should succeed");
            assert_eq!(fresh.frames.len(), 1);
            assert_eq!(fresh.frames[0].representative_frame.id, frame.id);
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
    fn audio_hits_group_chronologically_before_rank_ordering() {
        let segment = AudioSegment {
            id: 7,
            source_kind: AudioSegmentSourceKind::Microphone,
            source_session_id: "mic-session".to_string(),
            segment_index: 1,
            file_path: "/tmp/audio.m4a".to_string(),
            started_at: "2026-05-17T10:00:00Z".to_string(),
            ended_at: "2026-05-17T10:00:20Z".to_string(),
            capture_segment_id: None,
            created_at: "2026-05-17T10:00:00Z".to_string(),
            updated_at: "2026-05-17T10:00:00Z".to_string(),
        };
        let hit = |span_start_ms, span_end_ms, rank| AudioHit {
            audio_segment: segment.clone(),
            source_kind: AudioSegmentSourceKind::Microphone,
            span_start_ms,
            span_end_ms,
            snippet: format!("hit {span_start_ms}"),
            rank,
        };

        let hits = vec![
            hit(4_000, 4_500, -10.0),
            hit(1_000, 1_500, -1.0),
            hit(2_200, 2_500, -5.0),
        ];
        let groups = group_audio_hits(&hits).expect("grouping should succeed");

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].span_start_ms, 1_000);
        assert_eq!(groups[0].span_end_ms, 4_500);
        assert_eq!(groups[0].match_count, 3);
    }

    #[test]
    fn audio_groups_preserve_best_relevance_before_recency() {
        let segment = AudioSegment {
            id: 7,
            source_kind: AudioSegmentSourceKind::Microphone,
            source_session_id: "mic-session".to_string(),
            segment_index: 1,
            file_path: "/tmp/audio.m4a".to_string(),
            started_at: "2026-05-17T10:00:00Z".to_string(),
            ended_at: "2026-05-17T10:00:20Z".to_string(),
            capture_segment_id: None,
            created_at: "2026-05-17T10:00:00Z".to_string(),
            updated_at: "2026-05-17T10:00:00Z".to_string(),
        };
        let hit = |span_start_ms, rank| AudioHit {
            audio_segment: segment.clone(),
            source_kind: AudioSegmentSourceKind::Microphone,
            span_start_ms,
            span_end_ms: span_start_ms + 500,
            snippet: format!("hit {span_start_ms}"),
            rank,
        };

        let hits = vec![hit(10_000, -1.0), hit(1_000, -10.0)];
        let groups = group_audio_hits(&hits).expect("grouping should succeed");

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].span_start_ms, 1_000);
        assert_eq!(groups[1].span_start_ms, 10_000);
    }

    #[test]
    fn frame_groups_preserve_best_relevance_before_recency() {
        let frame = |id: i64, captured_at: &str| Frame {
            id,
            session_id: "screen-session".to_string(),
            file_path: format!("/tmp/relevance-{id}.jpg"),
            captured_at: captured_at.to_string(),
            width: None,
            height: None,
            equivalence: crate::FrameEquivalence {
                hint: None,
                proof: None,
                version: None,
                status: None,
                error: None,
            },
            metadata_snapshot: None,
            created_at: captured_at.to_string(),
            updated_at: captured_at.to_string(),
        };
        let hit = |id, captured_at, rank| FrameHit {
            group_key: format!("frame:{id}"),
            frame: frame(id, captured_at),
            snippet: format!("hit {id}"),
            rank,
            app_name: None,
            window_title: None,
            text_source_kind: "direct".to_string(),
        };

        let hits = vec![
            hit(1, "2026-05-17T10:00:00Z", -10.0),
            hit(2, "2026-05-17T10:10:00Z", -1.0),
        ];
        let groups = group_frame_hits(&hits);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].representative_frame.id, 1);
        assert_eq!(groups[1].representative_frame.id, 2);
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
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].representative_frame.id, frame.id);
        });
    }

    #[test]
    fn search_ranks_body_matches_ahead_of_context_matches() {
        run_async_test(async {
            let dir = test_dir("body-context-rank");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let context_match = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-context-rank-a.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: Some("com.example.Roadmap".to_string()),
                            app_name: Some("Roadmap".to_string()),
                            window_title: None,
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
                )
                .await
                .expect("context frame should insert");
            let body_match = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-context-rank-b.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: Some("com.example.Notes".to_string()),
                            app_name: Some("Notes".to_string()),
                            window_title: None,
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
                )
                .await
                .expect("body frame should insert");

            let context_job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(context_match.id))
                .await
                .expect("context job should enqueue");
            complete_job(
                &infra,
                context_job,
                ProcessingResultDraft::new().with_result_text("ordinary body text"),
            )
            .await;
            let body_job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(body_match.id))
                .await
                .expect("body job should enqueue");
            complete_job(
                &infra,
                body_job,
                ProcessingResultDraft::new().with_result_text("roadmap appears in captured text"),
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
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 2);
            assert_eq!(response.frames[0].representative_frame.id, body_match.id);
            assert_eq!(response.frames[1].representative_frame.id, context_match.id);
        });
    }

    #[test]
    fn search_preserves_short_symbol_qualified_terms() {
        run_async_test(async {
            let dir = test_dir("short-symbol-query");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-short-symbol.jpg",
                    "2026-05-17T10:00:00Z",
                ))
                .await
                .expect("frame should insert");
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                .await
                .expect("ocr job should enqueue");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("C# compiler notes"),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "C#".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].representative_frame.id, frame.id);
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
    fn search_has_more_uses_grouped_frame_results() {
        run_async_test(async {
            let dir = test_dir("grouped-has-more");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-screen".to_string()),
                proof: Some(vec![11; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            for index in 0..260 {
                let frame = infra
                    .insert_frame(
                        &NewFrame::new(
                            "screen-session",
                            &format!("/tmp/search-grouped-has-more-{index}.jpg"),
                            &format!("2026-05-17T10:{:02}:{:02}Z", index / 60, index % 60),
                        )
                        .with_equivalence(equivalence.clone()),
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
                    ProcessingResultDraft::new().with_result_text("collapsed target phrase"),
                )
                .await;
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "collapsed".to_string(),
                    frame_limit: Some(5),
                    frame_offset: Some(0),
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert!(!response.has_more_frames);
        });
    }

    #[test]
    fn search_has_more_uses_grouped_audio_results() {
        run_async_test(async {
            let dir = test_dir("grouped-audio-has-more");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/search-grouped-audio.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:05:00Z",
                ))
                .await
                .expect("segment should insert");
            let spans = (0..260)
                .map(|index| TranscriptionSegment {
                    start_ms: index * 1_000,
                    end_ms: index * 1_000 + 500,
                    text: "collapsed audio target".to_string(),
                    confidence: None,
                })
                .collect::<Vec<_>>();
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: spans,
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
                    .with_result_text("collapsed audio target")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "collapsed".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: Some(0),
                    snapshot_document_id: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 1);
            assert!(!response.has_more_audio);
        });
    }

    #[test]
    fn frame_search_paginates_beyond_hit_fetch_batch_cap() {
        run_async_test(async {
            let dir = test_dir("frame-beyond-hit-cap");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let mut transaction = infra.pool().begin().await.expect("tx should begin");
            for index in 0..5_006_u64 {
                let captured_at = timestamp_plus_ms("2026-05-17T10:00:00Z", index * 1_000)
                    .expect("timestamp should format");
                let insert = sqlx::query(
                    "INSERT INTO frames (session_id, file_path, captured_at) VALUES (?1, ?2, ?3)",
                )
                .bind("screen-session")
                .bind(format!("/tmp/search-deep-frame-{index}.jpg"))
                .bind(&captured_at)
                .execute(&mut *transaction)
                .await
                .expect("frame should insert");
                let frame_id = insert.last_insert_rowid();
                insert_search_document(
                    &mut transaction,
                    NewSearchDocument {
                        anchor_type: "frame",
                        frame_id: Some(frame_id),
                        audio_segment_id: None,
                        processing_result_id: None,
                        span_start_ms: None,
                        span_end_ms: None,
                        absolute_start_at: &captured_at,
                        absolute_end_at: &captured_at,
                        source_kind: None,
                        session_id: "screen-session",
                        app_name: None,
                        window_title: None,
                        group_key: &format!("frame:{frame_id}"),
                        text_source_kind: "direct",
                        body_text: "deepframe target",
                        context_text: "",
                    },
                )
                .await
                .expect("search document should insert");
            }
            transaction.commit().await.expect("tx should commit");

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "deepframe".to_string(),
                    frame_limit: Some(5),
                    frame_offset: Some(5_000),
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 5);
            assert!(response.has_more_frames);
        });
    }

    #[test]
    fn audio_search_paginates_beyond_hit_fetch_batch_cap() {
        run_async_test(async {
            let dir = test_dir("audio-beyond-hit-cap");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/search-deep-audio.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T14:15:00Z",
                ))
                .await
                .expect("segment should insert");
            let mut transaction = infra.pool().begin().await.expect("tx should begin");
            for index in 0..5_006_u64 {
                let start_ms = index * 3_000;
                let end_ms = start_ms + 500;
                let absolute_start_at = timestamp_plus_ms(&segment.started_at, start_ms)
                    .expect("start timestamp should format");
                let absolute_end_at = timestamp_plus_ms(&segment.started_at, end_ms)
                    .expect("end timestamp should format");
                insert_search_document(
                    &mut transaction,
                    NewSearchDocument {
                        anchor_type: "audio",
                        frame_id: None,
                        audio_segment_id: Some(segment.id),
                        processing_result_id: None,
                        span_start_ms: Some(start_ms as i64),
                        span_end_ms: Some(end_ms as i64),
                        absolute_start_at: &absolute_start_at,
                        absolute_end_at: &absolute_end_at,
                        source_kind: Some(segment.source_kind.as_str()),
                        session_id: &segment.source_session_id,
                        app_name: None,
                        window_title: None,
                        group_key: &format!("audio:{}:{index}", segment.id),
                        text_source_kind: "direct",
                        body_text: "deepaudio target",
                        context_text: "",
                    },
                )
                .await
                .expect("search document should insert");
            }
            transaction.commit().await.expect("tx should commit");

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "deepaudio".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: Some(5_000),
                    snapshot_document_id: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 5);
            assert!(response.has_more_audio);
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
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].representative_frame.id, second.frame.id);
            assert_eq!(response.frames[0].text_source_kind, "equivalent_reuse");
        });
    }

    #[test]
    fn audio_search_aligns_to_near_earlier_frame() {
        run_async_test(async {
            let dir = test_dir("audio-alignment-near-earlier");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(&NewFrame::new(
                    "shared-session",
                    "/tmp/alignment-near-frame.jpg",
                    "2026-05-17T10:00:56Z",
                ))
                .await
                .expect("frame should insert");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "shared-session",
                    1,
                    "/tmp/search-audio-alignment-near.m4a",
                    "2026-05-17T10:01:00Z",
                    "2026-05-17T10:01:20Z",
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
                    text: "alignment target phrase".to_string(),
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
                    .with_result_text("alignment target phrase")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "alignment".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 1);
            assert_eq!(
                response.audio[0]
                    .aligned_frame
                    .as_ref()
                    .map(|frame| frame.id),
                Some(frame.id)
            );
        });
    }

    #[test]
    fn audio_search_does_not_align_stale_earlier_frame() {
        run_async_test(async {
            let dir = test_dir("audio-alignment-stale-earlier");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            infra
                .insert_frame(&NewFrame::new(
                    "shared-session",
                    "/tmp/alignment-frame.jpg",
                    "2026-05-17T10:00:00Z",
                ))
                .await
                .expect("frame should insert");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "shared-session",
                    1,
                    "/tmp/search-audio-alignment.m4a",
                    "2026-05-17T10:01:00Z",
                    "2026-05-17T10:01:20Z",
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
                    text: "alignment target phrase".to_string(),
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
                    .with_result_text("alignment target phrase")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "alignment".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 1);
            assert_eq!(
                response.audio[0]
                    .aligned_frame
                    .as_ref()
                    .map(|frame| frame.id),
                None
            );
        });
    }

    #[test]
    fn search_pagination_uses_snapshot_document_high_water_mark() {
        run_async_test(async {
            let dir = test_dir("pagination-snapshot");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            for (path, captured_at) in [
                ("/tmp/search-page-a.jpg", "2026-05-17T10:00:00Z"),
                ("/tmp/search-page-b.jpg", "2026-05-17T10:00:01Z"),
            ] {
                let frame = infra
                    .insert_frame(&NewFrame::new("screen-session", path, captured_at))
                    .await
                    .expect("frame should insert");
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text("snapshot target phrase"),
                )
                .await;
            }

            let first_page = infra
                .search_capture(SearchCaptureRequest {
                    query: "snapshot".to_string(),
                    frame_limit: Some(1),
                    frame_offset: Some(0),
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                })
                .await
                .expect("first page search should succeed");
            let first_frame_id = first_page.frames[0].representative_frame.id;

            let newer_frame = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-page-c.jpg",
                    "2026-05-17T10:00:02Z",
                ))
                .await
                .expect("newer frame should insert");
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(newer_frame.id))
                .await
                .expect("ocr job should enqueue");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("snapshot target phrase"),
            )
            .await;

            let second_page = infra
                .search_capture(SearchCaptureRequest {
                    query: "snapshot".to_string(),
                    frame_limit: Some(1),
                    frame_offset: Some(1),
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: Some(first_page.snapshot_document_id),
                })
                .await
                .expect("second page search should succeed");

            assert_eq!(second_page.frames.len(), 1);
            assert_ne!(
                second_page.frames[0].representative_frame.id,
                first_frame_id
            );
            assert_ne!(
                second_page.frames[0].representative_frame.id,
                newer_frame.id
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
                })
                .await
                .expect("search should succeed");

            assert!(first.frames.is_empty());
            assert_eq!(second.frames.len(), 1);
        });
    }

    #[test]
    fn frame_search_does_not_group_same_hint_with_different_proofs() {
        run_async_test(async {
            let dir = test_dir("frame-proof-grouping");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            for (path, proof) in [
                ("/tmp/search-proof-a.jpg", vec![0; 1024]),
                ("/tmp/search-proof-b.jpg", vec![255; 1024]),
            ] {
                let frame = infra
                    .insert_frame(
                        &NewFrame::new("screen-session", path, "2026-05-17T10:00:00Z")
                            .with_equivalence(crate::FrameEquivalence {
                                hint: Some("same-hint".to_string()),
                                proof: Some(proof),
                                version: Some(1),
                                status: Some(crate::FrameEquivalenceStatus::Ready),
                                error: None,
                            }),
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
                    ProcessingResultDraft::new().with_result_text("proof target phrase"),
                )
                .await;
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "proof".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 2);
            let mut group_keys = response
                .frames
                .iter()
                .map(|frame| frame.group_key.as_str())
                .collect::<Vec<_>>();
            group_keys.sort_unstable();
            group_keys.dedup();
            assert_eq!(group_keys.len(), 2);
            assert_eq!(
                response
                    .frames
                    .iter()
                    .map(|frame| frame.match_count)
                    .collect::<Vec<_>>(),
                vec![1, 1]
            );
        });
    }

    #[test]
    fn frame_search_does_not_group_equivalent_proofs_across_hidden_workspaces() {
        run_async_test(async {
            let dir = test_dir("frame-hidden-workspace-grouping");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-hidden-proof".to_string()),
                proof: Some(vec![31; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };

            for (index, segment) in ["0001", "0002"].into_iter().enumerate() {
                let frame_path = dir
                    .join(format!(
                        "recordings/2026/05/17/.screen-session-segment-{segment}/frames/frame-1.jpg"
                    ))
                    .to_string_lossy()
                    .to_string();
                let frame = infra
                    .insert_frame(
                        &NewFrame::new(
                            "screen-session",
                            &frame_path,
                            &format!("2026-05-17T10:00:0{index}Z"),
                        )
                        .with_equivalence(equivalence.clone()),
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
                    ProcessingResultDraft::new().with_result_text("hidden scope phrase"),
                )
                .await;
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "hidden".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 2);
            let mut group_keys = response
                .frames
                .iter()
                .map(|frame| frame.group_key.as_str())
                .collect::<Vec<_>>();
            group_keys.sort_unstable();
            group_keys.dedup();
            assert_eq!(group_keys.len(), 2);
            assert_eq!(
                response
                    .frames
                    .iter()
                    .map(|frame| frame.match_count)
                    .collect::<Vec<_>>(),
                vec![1, 1]
            );
        });
    }
}
