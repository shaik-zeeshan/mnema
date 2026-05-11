use std::collections::BTreeSet;

use audio_transcription::{TranscriptionMetadata, TranscriptionSegment, TranscriptionWord};
use serde::{Deserialize, Serialize};
use speaker_analysis::{
    PersonEnrollment, PersonRecognitionRejection, RecognitionConfidence, SpeakerAnalysisOutput,
};
use sqlx::{sqlite::SqliteRow, Executor, QueryBuilder, Row, Sqlite, SqlitePool, Transaction};

use crate::{AppInfraError, AudioSegment, AudioSegmentSourceKind, NewAudioSegment, Result};

use super::{
    AudioTranscriptionJobPayload, Frame, FrameEquivalence, FrameEquivalenceStatus, FrameSummary,
    NewFrame, ProcessingJob, ProcessingJobDraft, ProcessingJobStatus, ProcessingResult,
    ProcessingResultDraft, ProcessingSubject, AUDIO_SEGMENT_SUBJECT_TYPE,
    AUDIO_TRANSCRIPTION_PROCESSOR, OCR_PROCESSOR, SPEAKER_ANALYSIS_PROCESSOR,
    SYSTEM_AUDIO_SPEECH_ACTIVITY_PROCESSOR,
};
use super::SystemAudioSpeechActivityJobPayload;

pub(crate) const ORPHANED_RUNNING_PROCESSING_JOB_ERROR: &str =
    "processing job was marked failed during startup recovery after the app shut down while it was running";
pub const SPEAKER_ANALYSIS_PAYLOAD_OPTION_KEY: &str = "speakerAnalysisPayload";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrameProcessingJob {
    pub frame: Frame,
    pub job: ProcessingJob,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingJobCompletion {
    pub job: ProcessingJob,
    pub result: ProcessingResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SegmentWorkspaceOcrReference {
    pub frame_id: i64,
    pub job_id: i64,
    pub status: ProcessingJobStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerTurnView {
    pub id: i64,
    pub audio_segment_id: i64,
    pub session_id: String,
    pub cluster_id: i64,
    pub segment_cluster_id: Option<i64>,
    pub provider_cluster_id: String,
    pub speaker_label: String,
    pub person_id: Option<i64>,
    pub suggested_person_id: Option<i64>,
    pub recognition_confidence: Option<String>,
    pub recognition_score: Option<f32>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub transcript_text: Option<String>,
    pub overlaps: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonProfile {
    pub id: i64,
    pub display_name: String,
    pub notes: Option<String>,
    pub embedding_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerClusterView {
    pub id: i64,
    pub session_id: String,
    pub provider: String,
    pub model_id: Option<String>,
    pub provider_cluster_id: String,
    pub speaker_label: String,
    pub person_id: Option<i64>,
    pub suggested_person_id: Option<i64>,
    pub recognition_confidence: Option<String>,
    pub recognition_score: Option<f32>,
    pub suggested_merge_target_cluster_id: Option<i64>,
    pub suggested_merge_score: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessingModelCleanupLock {
    pub processor: String,
    pub lock_token: String,
    pub acquired_model_keys: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FocusedFrameWindow {
    pub frames: Vec<Frame>,
    pub target_index: usize,
    pub has_newer: bool,
    pub has_older: bool,
}

#[derive(Clone)]
pub struct ProcessingStore {
    pool: SqlitePool,
}

impl ProcessingStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub(crate) async fn begin_transaction(&self) -> Result<Transaction<'_, Sqlite>> {
        Ok(self.pool.begin().await?)
    }

    pub async fn insert_frame(&self, frame: &NewFrame) -> Result<Frame> {
        let frame_id = insert_frame_record(&self.pool, frame).await?;
        self.get_required_frame(frame_id).await
    }

    pub async fn upsert_audio_segment(&self, segment: &NewAudioSegment) -> Result<AudioSegment> {
        upsert_audio_segment_record(&self.pool, segment).await?;
        get_audio_segment_by_unique_key(&self.pool, segment).await
    }

    pub async fn get_audio_segment(&self, audio_segment_id: i64) -> Result<Option<AudioSegment>> {
        get_audio_segment_optional(&self.pool, audio_segment_id).await
    }

    pub(crate) async fn insert_frame_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        frame: &NewFrame,
    ) -> Result<Frame> {
        let frame_id = insert_frame_record(&mut **transaction, frame).await?;
        get_frame_optional(&mut **transaction, frame_id)
            .await?
            .ok_or(AppInfraError::FrameNotFound(frame_id))
    }

    pub(crate) async fn get_frame_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        frame_id: i64,
    ) -> Result<Option<Frame>> {
        get_frame_optional(&mut **transaction, frame_id).await
    }

    pub async fn enqueue_job(&self, draft: &ProcessingJobDraft) -> Result<ProcessingJob> {
        let job_id = insert_processing_job_record(
            &self.pool,
            &draft.subject,
            &draft.processor,
            draft.payload_json.as_deref(),
        )
        .await?;

        self.get_required_job(job_id).await
    }

    pub(crate) async fn enqueue_processor_job_for_frame_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        frame_id: i64,
        processor: &str,
        payload_json: Option<&str>,
    ) -> Result<ProcessingJob> {
        let subject = ProcessingSubject::frame(frame_id);
        self.enqueue_job_in_transaction(transaction, &subject, processor, payload_json)
            .await
    }

    pub(crate) async fn enqueue_job_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        subject: &ProcessingSubject,
        processor: &str,
        payload_json: Option<&str>,
    ) -> Result<ProcessingJob> {
        let job_id =
            insert_processing_job_record(&mut **transaction, subject, processor, payload_json)
                .await?;

        get_processing_job_optional(&mut **transaction, job_id)
            .await?
            .ok_or(AppInfraError::ProcessingJobNotFound(job_id))
    }

    pub(crate) async fn get_latest_processing_job_for_subject_and_processor_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        subject: &ProcessingSubject,
        processor: &str,
    ) -> Result<Option<ProcessingJob>> {
        let row = sqlx::query(
            "SELECT \
                id, subject_type, subject_id, processor, status, attempt_count, payload_json, last_error, \
                created_at, updated_at, started_at, finished_at \
             FROM processing_jobs \
             WHERE subject_type = ?1 AND subject_id = ?2 AND processor = ?3 \
             ORDER BY id DESC \
             LIMIT 1",
        )
        .bind(subject.subject_type())
        .bind(subject.subject_id)
        .bind(processor)
        .fetch_optional(&mut **transaction)
        .await?;

        row.map(map_processing_job).transpose()
    }

    pub(crate) async fn requeue_processing_job_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        job_id: i64,
        payload_json: Option<&str>,
    ) -> Result<ProcessingJob> {
        let update = sqlx::query(
            "UPDATE processing_jobs \
             SET status = 'queued', \
                 payload_json = COALESCE(?2, payload_json), \
                 last_error = NULL, \
                 started_at = NULL, \
                 finished_at = NULL, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1 AND status IN ('completed', 'failed')",
        )
        .bind(job_id)
        .bind(payload_json)
        .execute(&mut **transaction)
        .await?;

        if update.rows_affected() == 0 {
            let current = get_processing_job_optional(&mut **transaction, job_id)
                .await?
                .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?;
            return Err(processing_job_invalid_transition(
                job_id,
                &current.status,
                ProcessingJobStatus::Queued.as_str(),
            ));
        }

        delete_processing_result_for_job(&mut **transaction, job_id).await?;

        get_processing_job_optional(&mut **transaction, job_id)
            .await?
            .ok_or(AppInfraError::ProcessingJobNotFound(job_id))
    }

    pub async fn get_frame(&self, frame_id: i64) -> Result<Option<Frame>> {
        get_frame_optional(&self.pool, frame_id).await
    }

    pub async fn list_earlier_frames_with_equivalence_hint_in_scope(
        &self,
        session_id: &str,
        before_frame_id: i64,
        equivalence_hint: &str,
        workspace_prefix: Option<&str>,
    ) -> Result<Vec<Frame>> {
        let rows = if let Some(workspace_prefix) = workspace_prefix {
            let like_pattern = format!("{}%", Self::escape_sql_like_pattern(workspace_prefix));
            sqlx::query(
                "SELECT id, session_id, file_path, captured_at, width, height, \
                        equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                        created_at, updated_at \
                 FROM frames \
                 WHERE session_id = ?1 AND id < ?2 AND equivalence_hint = ?3 AND file_path LIKE ?4 ESCAPE '\\' \
                 ORDER BY id DESC",
            )
            .bind(session_id)
            .bind(before_frame_id)
            .bind(equivalence_hint)
            .bind(like_pattern)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, session_id, file_path, captured_at, width, height, \
                        equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                        created_at, updated_at \
                 FROM frames \
                 WHERE session_id = ?1 AND id < ?2 AND equivalence_hint = ?3 \
                 ORDER BY id DESC",
            )
            .bind(session_id)
            .bind(before_frame_id)
            .bind(equivalence_hint)
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter().map(map_frame).collect()
    }

    pub(crate) async fn list_earlier_frames_with_equivalence_hint_in_scope_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        session_id: &str,
        before_frame_id: i64,
        equivalence_hint: &str,
        workspace_prefix: Option<&str>,
    ) -> Result<Vec<Frame>> {
        let rows = if let Some(workspace_prefix) = workspace_prefix {
            let like_pattern = format!("{}%", Self::escape_sql_like_pattern(workspace_prefix));
            sqlx::query(
                "SELECT id, session_id, file_path, captured_at, width, height, \
                        equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                        created_at, updated_at \
                 FROM frames \
                 WHERE session_id = ?1 AND id < ?2 AND equivalence_hint = ?3 AND file_path LIKE ?4 ESCAPE '\\' \
                 ORDER BY id DESC",
            )
            .bind(session_id)
            .bind(before_frame_id)
            .bind(equivalence_hint)
            .bind(like_pattern)
            .fetch_all(&mut **transaction)
            .await?
        } else {
            sqlx::query(
                "SELECT id, session_id, file_path, captured_at, width, height, \
                        equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                        created_at, updated_at \
                 FROM frames \
                 WHERE session_id = ?1 AND id < ?2 AND equivalence_hint = ?3 \
                 ORDER BY id DESC",
            )
            .bind(session_id)
            .bind(before_frame_id)
            .bind(equivalence_hint)
            .fetch_all(&mut **transaction)
            .await?
        };

        rows.into_iter().map(map_frame).collect()
    }

    pub async fn list_frames_for_segment_workspace(
        &self,
        session_id: &str,
        workspace_prefix: &str,
    ) -> Result<Vec<Frame>> {
        let like_pattern = format!("{}%", Self::escape_sql_like_pattern(workspace_prefix));
        let rows = sqlx::query(
            "SELECT id, session_id, file_path, captured_at, width, height, \
                    equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                    created_at, updated_at \
             FROM frames \
             WHERE session_id = ?1 AND file_path LIKE ?2 ESCAPE '\\' \
             ORDER BY captured_at ASC, id ASC",
        )
        .bind(session_id)
        .bind(like_pattern)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_frame).collect()
    }

    pub(crate) async fn list_nonterminal_ocr_references_for_workspace(
        &self,
        workspace_prefix: &str,
    ) -> Result<Vec<SegmentWorkspaceOcrReference>> {
        let rows = sqlx::query(
            "SELECT \
                frames.id AS frame_id, \
                processing_jobs.id AS job_id, \
                processing_jobs.status AS job_status \
             FROM frames \
             INNER JOIN processing_jobs ON processing_jobs.subject_id = frames.id \
                 AND processing_jobs.subject_type = ?2 \
                 AND processing_jobs.processor = ?3 \
             WHERE frames.file_path LIKE ?1 ESCAPE '\\' \
             ORDER BY frames.id ASC, processing_jobs.id ASC",
        )
        .bind(format!(
            "{}%",
            Self::escape_sql_like_pattern(workspace_prefix)
        ))
        .bind(super::FRAME_SUBJECT_TYPE)
        .bind(super::OCR_PROCESSOR)
        .fetch_all(&self.pool)
        .await?;

        let mut references = Vec::new();
        let mut seen_job_ids = std::collections::HashSet::new();

        for row in rows {
            let job_id = row.get::<i64, _>("job_id");
            let status = ProcessingJobStatus::from_str(&row.get::<String, _>("job_status"))?;
            if matches!(
                status,
                ProcessingJobStatus::Completed | ProcessingJobStatus::Failed
            ) {
                continue;
            }
            if seen_job_ids.insert(job_id) {
                references.push(SegmentWorkspaceOcrReference {
                    frame_id: row.get("frame_id"),
                    job_id,
                    status,
                });
            }
        }

        Ok(references)
    }

    fn escape_sql_like_pattern(value: &str) -> String {
        let mut escaped = String::with_capacity(value.len());

        for ch in value.chars() {
            match ch {
                '%' | '_' | '\\' => {
                    escaped.push('\\');
                    escaped.push(ch);
                }
                _ => escaped.push(ch),
            }
        }

        escaped
    }

    pub async fn list_frames(
        &self,
        session_id: Option<&str>,
        before_id: Option<i64>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<Frame>> {
        if matches!(limit, Some(0)) {
            return Ok(Vec::new());
        }

        let mut query_builder = QueryBuilder::<Sqlite>::new(
            "SELECT id, session_id, file_path, captured_at, width, height, \
                    equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                    created_at, updated_at FROM frames",
        );

        let mut has_where_clause = false;

        if let Some(session_id) = session_id {
            query_builder.push(" WHERE session_id = ");
            query_builder.push_bind(session_id);
            has_where_clause = true;
        }

        if let Some(before_id) = before_id {
            query_builder.push(if has_where_clause {
                " AND id < "
            } else {
                " WHERE id < "
            });
            query_builder.push_bind(before_id);
        }

        query_builder.push(" ORDER BY id DESC");

        match (limit, offset) {
            (Some(limit), Some(offset)) => {
                query_builder.push(" LIMIT ");
                query_builder.push_bind(limit as i64);
                query_builder.push(" OFFSET ");
                query_builder.push_bind(offset as i64);
            }
            (Some(limit), None) => {
                query_builder.push(" LIMIT ");
                query_builder.push_bind(limit as i64);
            }
            (None, Some(offset)) => {
                query_builder.push(" LIMIT -1 OFFSET ");
                query_builder.push_bind(offset as i64);
            }
            (None, None) => {}
        };

        let rows = query_builder.build().fetch_all(&self.pool).await?;

        rows.into_iter().map(map_frame).collect()
    }

    pub async fn list_frame_summaries_in_range(
        &self,
        captured_at_start: &str,
        captured_at_end: &str,
    ) -> Result<Vec<FrameSummary>> {
        let rows = sqlx::query(
            "SELECT id, captured_at \
             FROM frames \
             WHERE captured_at >= ?1 AND captured_at <= ?2 \
             ORDER BY captured_at DESC, id DESC",
        )
        .bind(captured_at_start)
        .bind(captured_at_end)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_frame_summary).collect()
    }

    pub async fn get_timeline_window_around_frame(
        &self,
        frame_id: i64,
        newer_limit: u32,
        older_limit: u32,
    ) -> Result<FocusedFrameWindow> {
        let target = self.get_required_frame(frame_id).await?;

        let newer_rows = sqlx::query(
            "SELECT id, session_id, file_path, captured_at, width, height, \
                    equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                    created_at, updated_at \
             FROM frames \
             WHERE id > ?1 \
             ORDER BY id ASC \
             LIMIT ?2",
        )
        .bind(frame_id)
        .bind(newer_limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let older_rows = sqlx::query(
            "SELECT id, session_id, file_path, captured_at, width, height, \
                    equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                    created_at, updated_at \
             FROM frames \
             WHERE id < ?1 \
             ORDER BY id DESC \
             LIMIT ?2",
        )
        .bind(frame_id)
        .bind(older_limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut newer_frames = newer_rows
            .into_iter()
            .map(map_frame)
            .collect::<Result<Vec<_>>>()?;
        newer_frames.reverse();

        let target_index = newer_frames.len();
        let older_frames = older_rows
            .into_iter()
            .map(map_frame)
            .collect::<Result<Vec<_>>>()?;

        let mut frames = newer_frames;
        frames.push(target);
        frames.extend(older_frames);

        let tail_id = frames
            .last()
            .map(|frame| frame.id)
            .ok_or(AppInfraError::FrameNotFound(frame_id))?;
        let head_id = frames
            .first()
            .map(|frame| frame.id)
            .ok_or(AppInfraError::FrameNotFound(frame_id))?;
        let has_newer = sqlx::query("SELECT 1 FROM frames WHERE id > ?1 LIMIT 1")
            .bind(head_id)
            .fetch_optional(&self.pool)
            .await?
            .is_some();
        let has_older = sqlx::query("SELECT 1 FROM frames WHERE id < ?1 LIMIT 1")
            .bind(tail_id)
            .fetch_optional(&self.pool)
            .await?
            .is_some();

        Ok(FocusedFrameWindow {
            frames,
            target_index,
            has_newer,
            has_older,
        })
    }

    pub async fn get_latest_frame_in_range(
        &self,
        captured_at_start: &str,
        captured_at_end: &str,
    ) -> Result<Option<Frame>> {
        let row = sqlx::query(
            "SELECT id, session_id, file_path, captured_at, width, height, \
                    equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                    created_at, updated_at \
             FROM frames \
             WHERE captured_at >= ?1 AND captured_at <= ?2 \
             ORDER BY captured_at DESC, id DESC \
             LIMIT 1",
        )
        .bind(captured_at_start)
        .bind(captured_at_end)
        .fetch_optional(&self.pool)
        .await?;

        row.map(map_frame).transpose()
    }

    pub async fn get_job(&self, job_id: i64) -> Result<Option<ProcessingJob>> {
        get_processing_job_optional(&self.pool, job_id).await
    }

    pub async fn list_jobs_for_subject(
        &self,
        subject: &ProcessingSubject,
    ) -> Result<Vec<ProcessingJob>> {
        let rows = sqlx::query(
            "SELECT \
                id, subject_type, subject_id, processor, status, attempt_count, payload_json, last_error, \
                created_at, updated_at, started_at, finished_at \
             FROM processing_jobs \
             WHERE subject_type = ?1 AND subject_id = ?2 \
             ORDER BY id DESC",
        )
        .bind(subject.subject_type())
        .bind(subject.subject_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_processing_job).collect()
    }

    pub async fn list_running_jobs_for_processor(
        &self,
        processor: &str,
    ) -> Result<Vec<ProcessingJob>> {
        let rows = sqlx::query(
            "SELECT \
                id, subject_type, subject_id, processor, status, attempt_count, payload_json, last_error, \
                created_at, updated_at, started_at, finished_at \
             FROM processing_jobs \
             WHERE processor = ?1 AND status = 'running' \
             ORDER BY id ASC",
        )
        .bind(processor)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_processing_job).collect()
    }

    pub async fn list_retargetable_jobs_for_processor(
        &self,
        processor: &str,
    ) -> Result<Vec<ProcessingJob>> {
        let rows = sqlx::query(
            "SELECT \
                id, subject_type, subject_id, processor, status, attempt_count, payload_json, last_error, \
                created_at, updated_at, started_at, finished_at \
             FROM processing_jobs \
             WHERE processor = ?1 AND status IN ('queued', 'failed') \
             ORDER BY id ASC",
        )
        .bind(processor)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_processing_job).collect()
    }

    pub async fn mark_queued_jobs_failed_for_processor(
        &self,
        processor: &str,
        last_error: &str,
    ) -> Result<u64> {
        let update = sqlx::query(
            "UPDATE processing_jobs \
             SET status = 'failed', \
                 last_error = ?2, \
                 finished_at = CURRENT_TIMESTAMP, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE processor = ?1 AND status = 'queued'",
        )
        .bind(processor)
        .bind(last_error)
        .execute(&self.pool)
        .await?;

        Ok(update.rows_affected())
    }

    pub async fn update_retargetable_job_payload(
        &self,
        job_id: i64,
        payload_json: &str,
    ) -> Result<Option<ProcessingJob>> {
        let update = sqlx::query(
            "UPDATE processing_jobs \
             SET payload_json = ?2, updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1 AND status IN ('queued', 'failed')",
        )
        .bind(job_id)
        .bind(payload_json)
        .execute(&self.pool)
        .await?;

        if update.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_job(job_id).await
    }

    pub async fn acquire_model_cleanup_locks(
        &self,
        processor: &str,
        model_keys: &BTreeSet<String>,
        lock_token: &str,
    ) -> Result<ProcessingModelCleanupLock> {
        let mut transaction = self.pool.begin().await?;
        let mut acquired_model_keys = BTreeSet::new();

        for model_key in model_keys {
            let insert = sqlx::query(
                "INSERT OR IGNORE INTO processing_model_cleanup_locks \
                    (processor, model_key, lock_token) \
                 VALUES (?1, ?2, ?3)",
            )
            .bind(processor)
            .bind(model_key)
            .bind(lock_token)
            .execute(&mut *transaction)
            .await?;

            if insert.rows_affected() > 0 {
                acquired_model_keys.insert(model_key.clone());
            }
        }

        transaction.commit().await?;

        Ok(ProcessingModelCleanupLock {
            processor: processor.to_string(),
            lock_token: lock_token.to_string(),
            acquired_model_keys,
        })
    }

    pub async fn release_model_cleanup_locks(
        &self,
        lock: &ProcessingModelCleanupLock,
    ) -> Result<u64> {
        let delete = sqlx::query(
            "DELETE FROM processing_model_cleanup_locks \
             WHERE processor = ?1 AND lock_token = ?2",
        )
        .bind(&lock.processor)
        .bind(&lock.lock_token)
        .execute(&self.pool)
        .await?;

        Ok(delete.rows_affected())
    }

    pub async fn clear_model_cleanup_locks(&self) -> Result<u64> {
        let delete = sqlx::query("DELETE FROM processing_model_cleanup_locks")
            .execute(&self.pool)
            .await?;

        Ok(delete.rows_affected())
    }

    pub async fn claim_next_queued_job(&self) -> Result<Option<ProcessingJob>> {
        self.claim_next_queued_job_matching_processor(None, &[])
            .await
    }

    pub async fn claim_next_queued_job_for_processor(
        &self,
        processor: &str,
    ) -> Result<Option<ProcessingJob>> {
        self.claim_next_queued_job_matching_processor(Some(processor), &[])
            .await
    }

    pub async fn claim_next_queued_job_excluding_processor(
        &self,
        excluded_processor: &str,
    ) -> Result<Option<ProcessingJob>> {
        self.claim_next_queued_job_excluding_processors(&[excluded_processor])
            .await
    }

    pub async fn claim_next_queued_job_excluding_processors(
        &self,
        excluded_processors: &[&str],
    ) -> Result<Option<ProcessingJob>> {
        self.claim_next_queued_job_matching_processor(None, excluded_processors)
            .await
    }

    async fn claim_next_queued_job_matching_processor(
        &self,
        processor: Option<&str>,
        excluded_processors: &[&str],
    ) -> Result<Option<ProcessingJob>> {
        loop {
            let mut transaction = self.pool.begin().await?;
            let row = match (processor, excluded_processors.is_empty()) {
                (Some(processor), _) => sqlx::query(
                    "SELECT \
                        pj.id, pj.subject_type, pj.subject_id, pj.processor, pj.status, pj.attempt_count, \
                        pj.payload_json, pj.last_error, pj.created_at, pj.updated_at, pj.started_at, \
                        pj.finished_at \
                     FROM processing_jobs AS pj \
                     WHERE pj.status = 'queued' \
                       AND pj.processor = ?1 \
                       AND NOT EXISTS ( \
                         SELECT 1 FROM processing_model_cleanup_locks AS lock \
                         WHERE lock.processor = pj.processor \
                           AND lock.model_key = CASE \
                             WHEN pj.processor IN ('ocr', 'audio_transcription', 'speaker_analysis') \
                              AND pj.payload_json IS NOT NULL \
                              AND json_valid(pj.payload_json) \
                             THEN CASE \
                               WHEN json_type(pj.payload_json, '$.provider') = 'text' \
                                AND json_type(pj.payload_json, '$.modelId') = 'text' \
                                AND NULLIF(TRIM(json_extract(pj.payload_json, '$.provider')), '') IS NOT NULL \
                                AND NULLIF(TRIM(json_extract(pj.payload_json, '$.modelId')), '') IS NOT NULL \
                               THEN TRIM(json_extract(pj.payload_json, '$.provider')) || '/' || TRIM(json_extract(pj.payload_json, '$.modelId')) \
                               ELSE NULL \
                             END \
                             ELSE NULL \
                           END \
                       ) \
                     ORDER BY pj.id ASC \
                     LIMIT 1",
                )
                .bind(processor)
                .fetch_optional(&mut *transaction)
                .await?,
                (None, false) => {
                    let mut query = sqlx::QueryBuilder::new(
                        "SELECT \
                            pj.id, pj.subject_type, pj.subject_id, pj.processor, pj.status, pj.attempt_count, \
                            pj.payload_json, pj.last_error, pj.created_at, pj.updated_at, pj.started_at, \
                            pj.finished_at \
                         FROM processing_jobs AS pj \
                         WHERE pj.status = 'queued' \
                           AND pj.processor NOT IN (",
                    );
                    let mut separated = query.separated(", ");
                    for excluded_processor in excluded_processors {
                        separated.push_bind(excluded_processor);
                    }
                    separated.push_unseparated(
                        ") \
                           AND NOT EXISTS ( \
                             SELECT 1 FROM processing_model_cleanup_locks AS lock \
                             WHERE lock.processor = pj.processor \
                               AND lock.model_key = CASE \
                                 WHEN pj.processor IN ('ocr', 'audio_transcription', 'speaker_analysis') \
                                  AND pj.payload_json IS NOT NULL \
                                  AND json_valid(pj.payload_json) \
                                 THEN CASE \
                                   WHEN json_type(pj.payload_json, '$.provider') = 'text' \
                                    AND json_type(pj.payload_json, '$.modelId') = 'text' \
                                    AND NULLIF(TRIM(json_extract(pj.payload_json, '$.provider')), '') IS NOT NULL \
                                    AND NULLIF(TRIM(json_extract(pj.payload_json, '$.modelId')), '') IS NOT NULL \
                                   THEN TRIM(json_extract(pj.payload_json, '$.provider')) || '/' || TRIM(json_extract(pj.payload_json, '$.modelId')) \
                                   ELSE NULL \
                                 END \
                                 ELSE NULL \
                               END \
                           ) \
                         ORDER BY pj.id ASC \
                         LIMIT 1",
                    );
                    query.build().fetch_optional(&mut *transaction).await?
                }
                (None, true) => sqlx::query(
                    "SELECT \
                        pj.id, pj.subject_type, pj.subject_id, pj.processor, pj.status, pj.attempt_count, \
                        pj.payload_json, pj.last_error, pj.created_at, pj.updated_at, pj.started_at, \
                        pj.finished_at \
                     FROM processing_jobs AS pj \
                     WHERE pj.status = 'queued' \
                       AND NOT EXISTS ( \
                         SELECT 1 FROM processing_model_cleanup_locks AS lock \
                         WHERE lock.processor = pj.processor \
                           AND lock.model_key = CASE \
                             WHEN pj.processor IN ('ocr', 'audio_transcription', 'speaker_analysis') \
                              AND pj.payload_json IS NOT NULL \
                              AND json_valid(pj.payload_json) \
                             THEN CASE \
                               WHEN json_type(pj.payload_json, '$.provider') = 'text' \
                                AND json_type(pj.payload_json, '$.modelId') = 'text' \
                                AND NULLIF(TRIM(json_extract(pj.payload_json, '$.provider')), '') IS NOT NULL \
                                AND NULLIF(TRIM(json_extract(pj.payload_json, '$.modelId')), '') IS NOT NULL \
                               THEN TRIM(json_extract(pj.payload_json, '$.provider')) || '/' || TRIM(json_extract(pj.payload_json, '$.modelId')) \
                               ELSE NULL \
                             END \
                             ELSE NULL \
                           END \
                       ) \
                     ORDER BY pj.id ASC \
                     LIMIT 1",
                )
                .fetch_optional(&mut *transaction)
                .await?,
            };

            let job_id = row.map(map_processing_job).transpose()?.map(|job| job.id);
            let Some(job_id) = job_id else {
                transaction.commit().await?;
                return Ok(None);
            };

            let update = sqlx::query(
                "UPDATE processing_jobs \
                 SET status = 'running', \
                     attempt_count = attempt_count + 1, \
                     last_error = NULL, \
                     started_at = CURRENT_TIMESTAMP, \
                     finished_at = NULL, \
                     updated_at = CURRENT_TIMESTAMP \
                 WHERE id = ?1 AND status = 'queued'",
            )
            .bind(job_id)
            .execute(&mut *transaction)
            .await?;

            if update.rows_affected() == 0 {
                transaction.rollback().await?;
                continue;
            }

            delete_processing_result_for_job(&mut *transaction, job_id).await?;

            let job = get_processing_job_optional(&mut *transaction, job_id)
                .await?
                .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?;
            transaction.commit().await?;
            return Ok(Some(job));
        }
    }

    pub async fn claim_queued_job(&self, job_id: i64) -> Result<Option<ProcessingJob>> {
        let mut transaction = self.pool.begin().await?;
        let Some(job) = get_processing_job_optional(&mut *transaction, job_id).await? else {
            transaction.commit().await?;
            return Ok(None);
        };
        if job.status != ProcessingJobStatus::Queued {
            transaction.commit().await?;
            return Ok(None);
        }
        if processing_job_model_cleanup_locked(&mut transaction, &job).await? {
            transaction.commit().await?;
            return Ok(None);
        }

        let update = sqlx::query(
            "UPDATE processing_jobs \
             SET status = 'running', \
                 attempt_count = attempt_count + 1, \
                 last_error = NULL, \
                 started_at = CURRENT_TIMESTAMP, \
                 finished_at = NULL, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1 AND status = 'queued'",
        )
        .bind(job_id)
        .execute(&mut *transaction)
        .await?;

        if update.rows_affected() == 0 {
            transaction.commit().await?;
            return Ok(None);
        }

        delete_processing_result_for_job(&mut *transaction, job_id).await?;

        let job = get_processing_job_optional(&mut *transaction, job_id)
            .await?
            .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?;
        transaction.commit().await?;
        Ok(Some(job))
    }

    pub async fn mark_job_running(&self, job_id: i64) -> Result<ProcessingJob> {
        let mut transaction = self.pool.begin().await?;

        let job = get_processing_job_optional(&mut *transaction, job_id)
            .await?
            .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?;

        if !matches!(
            job.status,
            ProcessingJobStatus::Completed | ProcessingJobStatus::Failed
        ) {
            return Err(processing_job_invalid_transition(
                job_id,
                &job.status,
                ProcessingJobStatus::Running.as_str(),
            ));
        }
        if processing_job_model_cleanup_locked(&mut transaction, &job).await? {
            transaction.commit().await?;
            return Err(processing_job_invalid_transition(
                job_id,
                &job.status,
                ProcessingJobStatus::Running.as_str(),
            ));
        }

        let update = sqlx::query(
            "UPDATE processing_jobs \
             SET status = 'running', \
                 attempt_count = attempt_count + 1, \
                 last_error = NULL, \
                 started_at = CURRENT_TIMESTAMP, \
                 finished_at = NULL, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1 AND status IN ('completed', 'failed')",
        )
        .bind(job_id)
        .execute(&mut *transaction)
        .await?;

        if update.rows_affected() == 0 {
            let current = get_processing_job_optional(&mut *transaction, job_id)
                .await?
                .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?;
            return Err(processing_job_invalid_transition(
                job_id,
                &current.status,
                ProcessingJobStatus::Running.as_str(),
            ));
        }

        delete_processing_result_for_job(&mut *transaction, job_id).await?;

        let job = get_processing_job_optional(&mut *transaction, job_id)
            .await?
            .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?;
        transaction.commit().await?;
        Ok(job)
    }

    pub async fn reconcile_orphaned_running_jobs(&self) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE processing_jobs \
             SET status = 'failed', \
                 last_error = ?1, \
                 finished_at = CURRENT_TIMESTAMP, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE status = 'running'",
        )
        .bind(ORPHANED_RUNNING_PROCESSING_JOB_ERROR)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    pub async fn backfill_frame_equivalence(&self) -> Result<u64> {
        let rows = sqlx::query(
            "SELECT id, file_path FROM frames WHERE equivalence_status IS NULL ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut updated = 0_u64;

        for row in rows {
            let frame_id: i64 = row.get("id");
            let file_path: String = row.get("file_path");
            let equivalence = match capture_screen::captured_frame_equivalence_from_image_path(
                std::path::Path::new(&file_path),
            ) {
                capture_screen::CapturedFrameEquivalenceOutcome::Ready(equivalence) => {
                    FrameEquivalence::ready(
                        equivalence.hint,
                        equivalence.proof,
                        equivalence.version,
                    )
                }
                capture_screen::CapturedFrameEquivalenceOutcome::Quarantined(error) => {
                    capture_runtime::debug_log!(
                        "[app-infra] quarantined frame {} during equivalence backfill: {}",
                        frame_id,
                        error
                    );
                    FrameEquivalence::quarantined(error)
                }
            };

            sqlx::query(
                "UPDATE frames \
                 SET equivalence_hint = ?2, \
                     equivalence_proof = ?3, \
                     equivalence_version = ?4, \
                     equivalence_status = ?5, \
                     equivalence_error = ?6, \
                     updated_at = CURRENT_TIMESTAMP \
                 WHERE id = ?1",
            )
            .bind(frame_id)
            .bind(equivalence.hint.as_deref())
            .bind(equivalence.proof.as_deref())
            .bind(equivalence.version)
            .bind(
                equivalence
                    .status
                    .as_ref()
                    .map(FrameEquivalenceStatus::as_str),
            )
            .bind(equivalence.error.as_deref())
            .execute(&self.pool)
            .await?;

            updated = updated.saturating_add(1);
        }

        Ok(updated)
    }

    pub async fn mark_job_failed(
        &self,
        job_id: i64,
        error_text: Option<&str>,
    ) -> Result<ProcessingJob> {
        let mut transaction = self.pool.begin().await?;

        let job = get_processing_job_optional(&mut *transaction, job_id)
            .await?
            .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?;

        if job.status != ProcessingJobStatus::Running {
            return Err(processing_job_invalid_transition(
                job_id,
                &job.status,
                ProcessingJobStatus::Failed.as_str(),
            ));
        }

        let update = sqlx::query(
            "UPDATE processing_jobs \
             SET status = 'failed', \
                  last_error = ?2, \
                  finished_at = CURRENT_TIMESTAMP, \
                  updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1 AND status = 'running'",
        )
        .bind(job_id)
        .bind(error_text)
        .execute(&mut *transaction)
        .await?;

        if update.rows_affected() == 0 {
            let current = get_processing_job_optional(&mut *transaction, job_id)
                .await?
                .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?;
            return Err(processing_job_invalid_transition(
                job_id,
                &current.status,
                ProcessingJobStatus::Failed.as_str(),
            ));
        }

        delete_processing_result_for_job(&mut *transaction, job_id).await?;

        let job = get_processing_job_optional(&mut *transaction, job_id)
            .await?
            .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?;
        transaction.commit().await?;
        Ok(job)
    }

    pub async fn complete_job(
        &self,
        job_id: i64,
        result: &ProcessingResultDraft,
    ) -> Result<ProcessingJobCompletion> {
        let mut transaction = self.pool.begin().await?;

        let job = get_processing_job_optional(&mut *transaction, job_id)
            .await?
            .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?;

        if job.status != ProcessingJobStatus::Running {
            return Err(processing_job_invalid_transition(
                job_id,
                &job.status,
                ProcessingJobStatus::Completed.as_str(),
            ));
        }

        let result_insert = sqlx::query(
            "INSERT INTO processing_results (\
                job_id, subject_type, subject_id, processor, result_text, structured_payload_json, processor_version\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(job_id)
        .bind(&job.subject_type)
        .bind(job.subject_id)
        .bind(&job.processor)
        .bind(result.result_text.as_deref())
        .bind(result.structured_payload_json.as_deref())
        .bind(result.processor_version.as_deref())
        .execute(&mut *transaction)
        .await?;

        let update = sqlx::query(
            "UPDATE processing_jobs \
             SET status = 'completed', \
                  last_error = NULL, \
                  finished_at = CURRENT_TIMESTAMP, \
                  updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1 AND status = 'running'",
        )
        .bind(job_id)
        .execute(&mut *transaction)
        .await?;

        if update.rows_affected() == 0 {
            let current = get_processing_job_optional(&mut *transaction, job_id)
                .await?
                .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?;
            return Err(processing_job_invalid_transition(
                job_id,
                &current.status,
                ProcessingJobStatus::Completed.as_str(),
            ));
        }

        let completed_job = get_processing_job_optional(&mut *transaction, job_id)
            .await?
            .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?;
        let result_id = result_insert.last_insert_rowid();
        let stored_result = get_processing_result_optional(&mut *transaction, result_id)
            .await?
            .ok_or(AppInfraError::ProcessingResultNotFound(result_id))?;

        if job.processor == AUDIO_TRANSCRIPTION_PROCESSOR
            && job.subject_type == AUDIO_SEGMENT_SUBJECT_TYPE
        {
            if let Some(payload_json) = speaker_analysis_payload_from_transcription_job(&job)? {
                let subject = ProcessingSubject::audio_segment(job.subject_id);
                let existing = self
                    .get_latest_processing_job_for_subject_and_processor_in_transaction(
                        &mut transaction,
                        &subject,
                        SPEAKER_ANALYSIS_PROCESSOR,
                    )
                    .await?;
                if existing.is_none() {
                    self.enqueue_job_in_transaction(
                        &mut transaction,
                        &subject,
                        SPEAKER_ANALYSIS_PROCESSOR,
                        Some(&payload_json),
                    )
                    .await?;
                } else if let Some(existing) = existing {
                    if existing.status == ProcessingJobStatus::Failed {
                        self.requeue_processing_job_in_transaction(
                            &mut transaction,
                            existing.id,
                            Some(&payload_json),
                        )
                        .await?;
                    }
                }
            }
        }
        if job.processor == SYSTEM_AUDIO_SPEECH_ACTIVITY_PROCESSOR
            && job.subject_type == AUDIO_SEGMENT_SUBJECT_TYPE
        {
            let speech_detected = result
                .structured_payload_json
                .as_deref()
                .and_then(|payload| serde_json::from_str::<serde_json::Value>(payload).ok())
                .and_then(|payload| payload.get("speechDetected").and_then(|value| value.as_bool()))
                .unwrap_or(false);
            if speech_detected {
                let payload = SystemAudioSpeechActivityJobPayload::from_job(&job)?;
                let subject = ProcessingSubject::audio_segment(job.subject_id);
                let existing = self
                    .get_latest_processing_job_for_subject_and_processor_in_transaction(
                        &mut transaction,
                        &subject,
                        AUDIO_TRANSCRIPTION_PROCESSOR,
                    )
                    .await?;
                if existing.is_none() {
                    self.enqueue_job_in_transaction(
                        &mut transaction,
                        &subject,
                        AUDIO_TRANSCRIPTION_PROCESSOR,
                        Some(&payload.transcription_payload),
                    )
                    .await?;
                } else if let Some(existing) = existing {
                    if existing.status == ProcessingJobStatus::Failed {
                        self.requeue_processing_job_in_transaction(
                            &mut transaction,
                            existing.id,
                            Some(&payload.transcription_payload),
                        )
                        .await?;
                    }
                }
            }
        }
        if job.processor == SPEAKER_ANALYSIS_PROCESSOR
            && job.subject_type == AUDIO_SEGMENT_SUBJECT_TYPE
        {
            if let Some(payload_json) = result.structured_payload_json.as_deref() {
                let output: SpeakerAnalysisOutput = serde_json::from_str(payload_json)?;
                persist_speaker_analysis_output(&mut transaction, job.subject_id, &output).await?;
            }
        }
        if job.subject_type == AUDIO_SEGMENT_SUBJECT_TYPE
            && matches!(
                job.processor.as_str(),
                AUDIO_TRANSCRIPTION_PROCESSOR | SPEAKER_ANALYSIS_PROCESSOR
            )
        {
            refresh_speaker_turn_transcript_texts(&mut transaction, job.subject_id).await?;
        }

        transaction.commit().await?;

        Ok(ProcessingJobCompletion {
            job: completed_job,
            result: stored_result,
        })
    }

    pub async fn get_result_for_job(&self, job_id: i64) -> Result<Option<ProcessingResult>> {
        let row = sqlx::query(
            "SELECT \
                id, job_id, subject_type, subject_id, processor, result_text, structured_payload_json, \
                processor_version, created_at \
             FROM processing_results \
             WHERE job_id = ?1",
        )
        .bind(job_id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(map_processing_result).transpose()
    }

    pub async fn list_results_for_subject(
        &self,
        subject: &ProcessingSubject,
    ) -> Result<Vec<ProcessingResult>> {
        let rows = sqlx::query(
            "SELECT \
                id, job_id, subject_type, subject_id, processor, result_text, structured_payload_json, \
                processor_version, created_at \
             FROM processing_results \
             WHERE subject_type = ?1 AND subject_id = ?2 \
             ORDER BY id DESC",
        )
        .bind(subject.subject_type())
        .bind(subject.subject_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_processing_result).collect()
    }

    pub async fn list_speaker_turns_for_audio_segment(
        &self,
        audio_segment_id: i64,
    ) -> Result<Vec<SpeakerTurnView>> {
        let rows = sqlx::query(
            "SELECT \
                speaker_turns.id, speaker_turns.audio_segment_id, speaker_turns.session_id, \
                speaker_turns.cluster_id, speaker_turns.segment_cluster_id, speaker_turns.start_ms, speaker_turns.end_ms, \
                speaker_turns.transcript_text, speaker_turns.overlaps, \
                recording_speaker_clusters.provider_cluster_id, \
                COALESCE(recording_speaker_clusters.transcript_local_label, recording_speaker_clusters.stable_label) AS speaker_label, \
                recording_speaker_clusters.person_id, \
                recording_speaker_clusters.recognition_person_id, \
                recording_speaker_clusters.recognition_confidence, \
                recording_speaker_clusters.recognition_score \
             FROM speaker_turns \
             INNER JOIN recording_speaker_clusters ON recording_speaker_clusters.id = speaker_turns.cluster_id \
             WHERE speaker_turns.audio_segment_id = ?1 \
             ORDER BY speaker_turns.start_ms ASC, speaker_turns.end_ms ASC, speaker_turns.id ASC",
        )
        .bind(audio_segment_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_speaker_turn_view).collect()
    }

    pub async fn list_person_profiles(&self) -> Result<Vec<PersonProfile>> {
        let rows = sqlx::query(
            "SELECT \
                person_profiles.id, person_profiles.display_name, person_profiles.notes, \
                COUNT(person_voice_embeddings.id) AS embedding_count, \
                person_profiles.created_at, person_profiles.updated_at \
             FROM person_profiles \
             LEFT JOIN person_voice_embeddings ON person_voice_embeddings.person_id = person_profiles.id \
             GROUP BY person_profiles.id \
             ORDER BY person_profiles.display_name COLLATE NOCASE ASC, person_profiles.id ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_person_profile).collect()
    }

    pub async fn create_person_profile(
        &self,
        display_name: &str,
        notes: Option<&str>,
    ) -> Result<PersonProfile> {
        let display_name = display_name.trim();
        if display_name.is_empty() {
            return Err(AppInfraError::SpeakerAnalysisEngine(
                "person display name cannot be empty".to_string(),
            ));
        }
        let result =
            sqlx::query("INSERT INTO person_profiles (display_name, notes) VALUES (?1, ?2)")
                .bind(display_name)
                .bind(notes.map(str::trim).filter(|value| !value.is_empty()))
                .execute(&self.pool)
                .await?;
        self.get_required_person_profile(result.last_insert_rowid())
            .await
    }

    pub async fn delete_person_profile(&self, person_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM person_profiles WHERE id = ?1")
            .bind(person_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_speaker_clusters_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<SpeakerClusterView>> {
        let rows = sqlx::query(
            "SELECT \
                id, session_id, provider, model_id, provider_cluster_id, \
                COALESCE(transcript_local_label, stable_label) AS speaker_label, \
                person_id, recognition_person_id, recognition_confidence, recognition_score, \
                suggested_merge_target_cluster_id, suggested_merge_score \
             FROM recording_speaker_clusters \
             WHERE session_id = ?1 \
             ORDER BY id ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(map_speaker_cluster_view).collect()
    }

    pub async fn name_speaker_cluster(
        &self,
        cluster_id: i64,
        label: &str,
    ) -> Result<SpeakerClusterView> {
        let label = label.trim();
        if label.is_empty() {
            return Err(AppInfraError::SpeakerAnalysisEngine(
                "speaker label cannot be empty".to_string(),
            ));
        }
        sqlx::query(
            "UPDATE recording_speaker_clusters \
             SET transcript_local_label = ?2, updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1",
        )
        .bind(cluster_id)
        .bind(label)
        .execute(&self.pool)
        .await?;
        self.get_required_speaker_cluster(cluster_id).await
    }

    pub async fn link_speaker_cluster_to_person(
        &self,
        cluster_id: i64,
        person_id: i64,
        add_embedding: bool,
    ) -> Result<SpeakerClusterView> {
        let mut transaction = self.pool.begin().await?;
        let cluster = get_speaker_cluster_row(&mut *transaction, cluster_id).await?;
        if cluster
            .person_id
            .is_some_and(|existing| existing != person_id)
        {
            persist_speaker_recognition_rejection_for_cluster(
                &mut transaction,
                &cluster,
                cluster_id,
                cluster.person_id,
            )
            .await?;
        }
        sqlx::query(
            "UPDATE recording_speaker_clusters \
             SET person_id = ?2, transcript_local_label = NULL, updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1",
        )
        .bind(cluster_id)
        .bind(person_id)
        .execute(&mut *transaction)
        .await?;
        if add_embedding {
            if let Some(embedding) = cluster.embedding {
                sqlx::query(
                    "INSERT INTO person_voice_embeddings (\
                        person_id, provider, model_id, embedding, source_session_id, source_cluster_id\
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .bind(person_id)
                .bind(&cluster.provider)
                .bind(cluster.model_id.as_deref().unwrap_or(""))
                .bind(embedding)
                .bind(&cluster.session_id)
                .bind(cluster_id)
                .execute(&mut *transaction)
                .await?;
            }
        }
        transaction.commit().await?;
        self.get_required_speaker_cluster(cluster_id).await
    }

    pub async fn unlink_speaker_cluster_from_person(
        &self,
        cluster_id: i64,
    ) -> Result<SpeakerClusterView> {
        let mut transaction = self.pool.begin().await?;
        let cluster = get_speaker_cluster_row(&mut *transaction, cluster_id).await?;
        persist_speaker_recognition_rejection_for_cluster(
            &mut transaction,
            &cluster,
            cluster_id,
            cluster.person_id,
        )
        .await?;
        sqlx::query(
            "UPDATE recording_speaker_clusters \
             SET person_id = NULL, updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1",
        )
        .bind(cluster_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.get_required_speaker_cluster(cluster_id).await
    }

    pub async fn confirm_speaker_recognition_suggestion(
        &self,
        cluster_id: i64,
        add_embedding: bool,
    ) -> Result<SpeakerClusterView> {
        let cluster = get_speaker_cluster_row(&self.pool, cluster_id).await?;
        let Some(person_id) = cluster.recognition_person_id else {
            return Err(AppInfraError::SpeakerAnalysisEngine(
                "speaker cluster has no recognition suggestion to confirm".to_string(),
            ));
        };
        self.link_speaker_cluster_to_person(cluster_id, person_id, add_embedding)
            .await
    }

    pub async fn reject_speaker_recognition_suggestion(
        &self,
        cluster_id: i64,
    ) -> Result<SpeakerClusterView> {
        let mut transaction = self.pool.begin().await?;
        let cluster = get_speaker_cluster_row(&mut *transaction, cluster_id).await?;
        persist_speaker_recognition_rejection_for_cluster(
            &mut transaction,
            &cluster,
            cluster_id,
            cluster.recognition_person_id,
        )
        .await?;
        sqlx::query(
            "UPDATE recording_speaker_clusters \
             SET recognition_person_id = NULL, recognition_confidence = NULL, recognition_score = NULL, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1",
        )
        .bind(cluster_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.get_required_speaker_cluster(cluster_id).await
    }

    pub async fn merge_speaker_clusters(
        &self,
        source_cluster_id: i64,
        target_cluster_id: i64,
    ) -> Result<SpeakerClusterView> {
        if source_cluster_id == target_cluster_id {
            return Err(AppInfraError::SpeakerAnalysisEngine(
                "cannot merge a speaker cluster into itself".to_string(),
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let source = get_speaker_cluster_row(&mut *transaction, source_cluster_id).await?;
        let target = get_speaker_cluster_row(&mut *transaction, target_cluster_id).await?;
        if source.session_id != target.session_id {
            return Err(AppInfraError::SpeakerAnalysisEngine(
                "speaker clusters must belong to the same session to merge".to_string(),
            ));
        }
        sqlx::query(
            "INSERT OR IGNORE INTO speaker_cluster_merges \
                (session_id, source_cluster_id, target_cluster_id) \
             VALUES (?1, ?2, ?3)",
        )
        .bind(&source.session_id)
        .bind(source_cluster_id)
        .bind(target_cluster_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE speaker_turns SET cluster_id = ?2, updated_at = CURRENT_TIMESTAMP \
             WHERE cluster_id = ?1",
        )
        .bind(source_cluster_id)
        .bind(target_cluster_id)
        .execute(&mut *transaction)
        .await?;
        purge_orphaned_speaker_cluster(&mut transaction, source_cluster_id).await?;
        transaction.commit().await?;
        self.get_required_speaker_cluster(target_cluster_id).await
    }

    pub async fn move_speaker_turn_to_cluster(
        &self,
        turn_id: i64,
        target_cluster_id: i64,
    ) -> Result<SpeakerTurnView> {
        let mut transaction = self.pool.begin().await?;
        let turn = fetch_required_speaker_turn(&mut *transaction, turn_id).await?;
        let target = get_speaker_cluster_row(&mut *transaction, target_cluster_id).await?;
        if turn.session_id != target.session_id {
            return Err(AppInfraError::SpeakerAnalysisEngine(
                "speaker turn and target cluster must belong to the same session".to_string(),
            ));
        }
        sqlx::query(
            "UPDATE speaker_turns \
             SET cluster_id = ?2, moved_to_cluster_id = ?2, updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1",
        )
        .bind(turn_id)
        .bind(target_cluster_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.get_required_speaker_turn(turn_id).await
    }

    pub async fn list_person_enrollments_for_speaker_model(
        &self,
        provider: &str,
        model_id: Option<&str>,
    ) -> Result<Vec<PersonEnrollment>> {
        let model_id = model_id.unwrap_or("");
        let rows = sqlx::query(
            "SELECT \
                person_profiles.id AS person_id, person_profiles.display_name, \
                person_voice_embeddings.embedding, person_voice_embeddings.model_id AS embedding_model_id \
             FROM person_voice_embeddings \
             INNER JOIN person_profiles ON person_profiles.id = person_voice_embeddings.person_id \
             WHERE person_voice_embeddings.provider = ?1 AND person_voice_embeddings.model_id = ?2 \
             ORDER BY person_profiles.id ASC, person_voice_embeddings.id ASC",
        )
        .bind(provider)
        .bind(model_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(PersonEnrollment {
                    person_id: row.get("person_id"),
                    display_name: row.get("display_name"),
                    embedding: row.get("embedding"),
                    embedding_model_id: row.get("embedding_model_id"),
                })
            })
            .collect()
    }

    pub async fn list_person_recognition_rejections_for_speaker_model(
        &self,
        provider: &str,
        model_id: Option<&str>,
    ) -> Result<Vec<PersonRecognitionRejection>> {
        let model_id = model_id.unwrap_or("");
        let rows = sqlx::query(
            "SELECT person_id, embedding, model_id AS embedding_model_id \
             FROM speaker_recognition_rejections \
             WHERE provider = ?1 AND model_id = ?2 \
             ORDER BY person_id ASC, id ASC",
        )
        .bind(provider)
        .bind(model_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(PersonRecognitionRejection {
                    person_id: row.get("person_id"),
                    embedding: row.get("embedding"),
                    embedding_model_id: row.get("embedding_model_id"),
                })
            })
            .collect()
    }

    async fn get_required_frame(&self, frame_id: i64) -> Result<Frame> {
        self.get_frame(frame_id)
            .await?
            .ok_or(AppInfraError::FrameNotFound(frame_id))
    }

    async fn get_required_job(&self, job_id: i64) -> Result<ProcessingJob> {
        self.get_job(job_id)
            .await?
            .ok_or(AppInfraError::ProcessingJobNotFound(job_id))
    }

    async fn get_required_person_profile(&self, person_id: i64) -> Result<PersonProfile> {
        let row = sqlx::query(
            "SELECT \
                person_profiles.id, person_profiles.display_name, person_profiles.notes, \
                COUNT(person_voice_embeddings.id) AS embedding_count, \
                person_profiles.created_at, person_profiles.updated_at \
             FROM person_profiles \
             LEFT JOIN person_voice_embeddings ON person_voice_embeddings.person_id = person_profiles.id \
             WHERE person_profiles.id = ?1 \
             GROUP BY person_profiles.id",
        )
        .bind(person_id)
        .fetch_one(&self.pool)
        .await?;
        map_person_profile(row)
    }

    async fn get_required_speaker_cluster(&self, cluster_id: i64) -> Result<SpeakerClusterView> {
        let row = sqlx::query(
            "SELECT \
                id, session_id, provider, model_id, provider_cluster_id, \
                COALESCE(transcript_local_label, stable_label) AS speaker_label, \
                person_id, recognition_person_id, recognition_confidence, recognition_score, \
                suggested_merge_target_cluster_id, suggested_merge_score \
             FROM recording_speaker_clusters \
             WHERE id = ?1",
        )
        .bind(cluster_id)
        .fetch_one(&self.pool)
        .await?;
        map_speaker_cluster_view(row)
    }

    async fn get_required_speaker_turn(&self, turn_id: i64) -> Result<SpeakerTurnView> {
        fetch_required_speaker_turn(&self.pool, turn_id).await
    }
}

async fn fetch_required_speaker_turn<'e, E>(executor: E, turn_id: i64) -> Result<SpeakerTurnView>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query(
        "SELECT \
            speaker_turns.id, speaker_turns.audio_segment_id, speaker_turns.session_id, \
            speaker_turns.cluster_id, speaker_turns.segment_cluster_id, speaker_turns.start_ms, speaker_turns.end_ms, \
            speaker_turns.transcript_text, speaker_turns.overlaps, \
            recording_speaker_clusters.provider_cluster_id, \
            COALESCE(recording_speaker_clusters.transcript_local_label, recording_speaker_clusters.stable_label) AS speaker_label, \
            recording_speaker_clusters.person_id, \
            recording_speaker_clusters.recognition_person_id, \
            recording_speaker_clusters.recognition_confidence, \
            recording_speaker_clusters.recognition_score \
         FROM speaker_turns \
         INNER JOIN recording_speaker_clusters ON recording_speaker_clusters.id = speaker_turns.cluster_id \
         WHERE speaker_turns.id = ?1",
    )
    .bind(turn_id)
    .fetch_one(executor)
    .await?;
    map_speaker_turn_view(row)
}

fn speaker_analysis_payload_from_transcription_job(job: &ProcessingJob) -> Result<Option<String>> {
    let Some(payload_json) = job.payload_json.as_deref() else {
        return Ok(None);
    };
    let payload: AudioTranscriptionJobPayload = serde_json::from_str(payload_json)?;
    let Some(value) = payload.options.get(SPEAKER_ANALYSIS_PAYLOAD_OPTION_KEY) else {
        return Ok(None);
    };
    Ok(Some(serde_json::to_string(value)?))
}

async fn persist_speaker_analysis_output(
    transaction: &mut Transaction<'_, Sqlite>,
    audio_segment_id: i64,
    output: &SpeakerAnalysisOutput,
) -> Result<()> {
    sqlx::query("DELETE FROM speaker_turns WHERE audio_segment_id = ?1")
        .bind(audio_segment_id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("DELETE FROM speaker_segment_clusters WHERE audio_segment_id = ?1")
        .bind(audio_segment_id)
        .execute(&mut **transaction)
        .await?;
    purge_orphaned_speaker_clusters_for_session_provider(
        transaction,
        &output.metadata.session_id,
        &output.metadata.provider,
    )
    .await?;

    let mut cluster_ids = std::collections::HashMap::<String, (i64, i64)>::new();
    for cluster in &output.clusters {
        let (suggested_person_id, recognition_confidence, recognition_score) = cluster
            .suggestion
            .as_ref()
            .map(|suggestion| {
                (
                    Some(suggestion.person_id),
                    Some(recognition_confidence_as_str(&suggestion.confidence)),
                    Some(suggestion.score),
                )
            })
            .unwrap_or((None, None, None));

        let merge_candidate = resolve_stable_speaker_cluster(
            transaction,
            &output.metadata.session_id,
            &output.metadata.provider,
            output.metadata.model_id.as_deref(),
            &cluster.embedding,
            suggested_person_id,
        )
        .await?;
        let stable_provider_cluster_id =
            if let Some(target_cluster_id) = merge_candidate.auto_merge_target_cluster_id {
                existing_speaker_cluster_provider_id(transaction, target_cluster_id).await?
            } else {
                format!("{audio_segment_id}:{}", cluster.provider_cluster_id)
            };

        sqlx::query(
            "INSERT INTO recording_speaker_clusters (\
                session_id, provider, model_id, provider_cluster_id, stable_label, \
                recognition_person_id, recognition_confidence, recognition_score, embedding, \
                suggested_merge_target_cluster_id, suggested_merge_score\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
             ON CONFLICT(session_id, provider, provider_cluster_id) DO UPDATE SET \
                model_id = excluded.model_id, \
                stable_label = COALESCE(recording_speaker_clusters.transcript_local_label, excluded.stable_label), \
                recognition_person_id = excluded.recognition_person_id, \
                recognition_confidence = excluded.recognition_confidence, \
                recognition_score = excluded.recognition_score, \
                embedding = excluded.embedding, \
                suggested_merge_target_cluster_id = excluded.suggested_merge_target_cluster_id, \
                suggested_merge_score = excluded.suggested_merge_score, \
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(&output.metadata.session_id)
        .bind(&output.metadata.provider)
        .bind(output.metadata.model_id.as_deref())
        .bind(&stable_provider_cluster_id)
        .bind(&cluster.stable_label)
        .bind(suggested_person_id)
        .bind(recognition_confidence)
        .bind(recognition_score)
        .bind(&cluster.embedding)
        .bind(merge_candidate.suggested_merge_target_cluster_id)
        .bind(merge_candidate.suggested_merge_score)
        .execute(&mut **transaction)
        .await?;

        let row = sqlx::query(
            "SELECT id FROM recording_speaker_clusters \
             WHERE session_id = ?1 AND provider = ?2 AND provider_cluster_id = ?3",
        )
        .bind(&output.metadata.session_id)
        .bind(&output.metadata.provider)
        .bind(&stable_provider_cluster_id)
        .fetch_one(&mut **transaction)
        .await?;
        let stable_cluster_id = row.get("id");

        sqlx::query(
            "INSERT INTO speaker_segment_clusters (\
                audio_segment_id, session_id, provider, model_id, provider_cluster_id, \
                stable_cluster_id, stable_label, embedding, embedding_model_id, \
                suggested_merge_target_cluster_id, suggested_merge_score\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
             ON CONFLICT(audio_segment_id, provider, provider_cluster_id) DO UPDATE SET \
                model_id = excluded.model_id, \
                stable_cluster_id = excluded.stable_cluster_id, \
                stable_label = excluded.stable_label, \
                embedding = excluded.embedding, \
                embedding_model_id = excluded.embedding_model_id, \
                suggested_merge_target_cluster_id = excluded.suggested_merge_target_cluster_id, \
                suggested_merge_score = excluded.suggested_merge_score, \
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(audio_segment_id)
        .bind(&output.metadata.session_id)
        .bind(&output.metadata.provider)
        .bind(output.metadata.model_id.as_deref())
        .bind(&cluster.provider_cluster_id)
        .bind(stable_cluster_id)
        .bind(&cluster.stable_label)
        .bind(&cluster.embedding)
        .bind(&cluster.embedding_model_id)
        .bind(merge_candidate.suggested_merge_target_cluster_id)
        .bind(merge_candidate.suggested_merge_score)
        .execute(&mut **transaction)
        .await?;

        let row = sqlx::query(
            "SELECT id FROM speaker_segment_clusters \
             WHERE audio_segment_id = ?1 AND provider = ?2 AND provider_cluster_id = ?3",
        )
        .bind(audio_segment_id)
        .bind(&output.metadata.provider)
        .bind(&cluster.provider_cluster_id)
        .fetch_one(&mut **transaction)
        .await?;
        cluster_ids.insert(
            cluster.provider_cluster_id.clone(),
            (stable_cluster_id, row.get("id")),
        );
    }

    for turn in &output.turns {
        let Some((cluster_id, segment_cluster_id)) =
            cluster_ids.get(&turn.provider_cluster_id).copied()
        else {
            continue;
        };
        sqlx::query(
            "INSERT INTO speaker_turns (\
                audio_segment_id, session_id, cluster_id, segment_cluster_id, start_ms, end_ms, transcript_text, overlaps\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(audio_segment_id)
        .bind(&output.metadata.session_id)
        .bind(cluster_id)
        .bind(segment_cluster_id)
        .bind(i64::try_from(turn.start_ms).unwrap_or(i64::MAX))
        .bind(i64::try_from(turn.end_ms).unwrap_or(i64::MAX))
        .bind(turn.transcript_text.as_deref())
        .bind(if turn.overlaps { 1_i64 } else { 0_i64 })
        .execute(&mut **transaction)
        .await?;
    }

    Ok(())
}

async fn purge_orphaned_speaker_clusters_for_session_provider(
    transaction: &mut Transaction<'_, Sqlite>,
    session_id: &str,
    provider: &str,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM recording_speaker_clusters \
         WHERE session_id = ?1 AND provider = ?2 \
           AND NOT EXISTS (\
                SELECT 1 FROM speaker_turns \
                WHERE speaker_turns.cluster_id = recording_speaker_clusters.id\
           )",
    )
    .bind(session_id)
    .bind(provider)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn purge_orphaned_speaker_cluster(
    transaction: &mut Transaction<'_, Sqlite>,
    cluster_id: i64,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM recording_speaker_clusters \
         WHERE id = ?1 \
           AND NOT EXISTS (\
                SELECT 1 FROM speaker_turns \
                WHERE speaker_turns.cluster_id = recording_speaker_clusters.id\
           )",
    )
    .bind(cluster_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
struct StableSpeakerClusterResolution {
    auto_merge_target_cluster_id: Option<i64>,
    suggested_merge_target_cluster_id: Option<i64>,
    suggested_merge_score: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
struct StableSpeakerClusterCandidate {
    id: i64,
    score: f32,
    person_id: Option<i64>,
}

const SPEAKER_CLUSTER_AUTO_REUSE_THRESHOLD: f32 = 0.82;
const SPEAKER_CLUSTER_SUGGEST_MERGE_THRESHOLD: f32 = 0.68;
const SPEAKER_CLUSTER_AMBIGUITY_MARGIN: f32 = 0.06;
const TIMED_TEXT_NEARBY_TURN_FALLBACK_MS: u64 = 500;

async fn resolve_stable_speaker_cluster(
    transaction: &mut Transaction<'_, Sqlite>,
    session_id: &str,
    provider: &str,
    model_id: Option<&str>,
    embedding: &[u8],
    recognition_person_id: Option<i64>,
) -> Result<StableSpeakerClusterResolution> {
    let incoming = f32_embedding_from_le_bytes(embedding);
    if incoming.is_empty() {
        return Ok(StableSpeakerClusterResolution::default());
    }

    let rows = sqlx::query(
        "SELECT id, embedding, person_id FROM recording_speaker_clusters \
         WHERE session_id = ?1 AND provider = ?2 AND COALESCE(model_id, '') = COALESCE(?3, '') \
           AND embedding IS NOT NULL \
         ORDER BY id ASC",
    )
    .bind(session_id)
    .bind(provider)
    .bind(model_id)
    .fetch_all(&mut **transaction)
    .await?;

    let mut candidates = rows
        .into_iter()
        .filter_map(|row| {
            let embedding: Vec<u8> = row.get("embedding");
            let score = cosine_similarity(&incoming, &f32_embedding_from_le_bytes(&embedding));
            score.is_finite().then_some(StableSpeakerClusterCandidate {
                id: row.get("id"),
                score,
                person_id: row.get("person_id"),
            })
        })
        .collect::<Vec<_>>();
    Ok(resolve_stable_speaker_cluster_from_candidates(
        &mut candidates,
        recognition_person_id,
    ))
}

fn resolve_stable_speaker_cluster_from_candidates(
    candidates: &mut [StableSpeakerClusterCandidate],
    recognition_person_id: Option<i64>,
) -> StableSpeakerClusterResolution {
    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.id.cmp(&right.id))
    });

    let Some(best) = candidates.first().copied() else {
        return StableSpeakerClusterResolution::default();
    };
    let second_score = candidates.get(1).map(|candidate| candidate.score);
    let ambiguous =
        second_score.is_some_and(|score| best.score - score < SPEAKER_CLUSTER_AMBIGUITY_MARGIN);
    let confirmed_person_conflict = recognition_person_id.zip(best.person_id).is_some_and(
        |(incoming_person_id, existing_person_id)| incoming_person_id != existing_person_id,
    );

    if best.score >= SPEAKER_CLUSTER_AUTO_REUSE_THRESHOLD
        && !ambiguous
        && !confirmed_person_conflict
    {
        StableSpeakerClusterResolution {
            auto_merge_target_cluster_id: Some(best.id),
            ..Default::default()
        }
    } else if best.score >= SPEAKER_CLUSTER_SUGGEST_MERGE_THRESHOLD {
        StableSpeakerClusterResolution {
            suggested_merge_target_cluster_id: Some(best.id),
            suggested_merge_score: Some(best.score),
            ..Default::default()
        }
    } else {
        StableSpeakerClusterResolution::default()
    }
}

async fn existing_speaker_cluster_provider_id(
    transaction: &mut Transaction<'_, Sqlite>,
    cluster_id: i64,
) -> Result<String> {
    let row =
        sqlx::query("SELECT provider_cluster_id FROM recording_speaker_clusters WHERE id = ?1")
            .bind(cluster_id)
            .fetch_one(&mut **transaction)
            .await?;
    Ok(row.get("provider_cluster_id"))
}

async fn refresh_speaker_turn_transcript_texts(
    transaction: &mut Transaction<'_, Sqlite>,
    audio_segment_id: i64,
) -> Result<()> {
    let Some(metadata) =
        latest_transcription_metadata_for_audio_segment(transaction, audio_segment_id).await?
    else {
        return Ok(());
    };
    let turns = speaker_turn_ranges_for_audio_segment(transaction, audio_segment_id).await?;
    if turns.is_empty() {
        return Ok(());
    }

    let runs = if metadata.words.is_empty() {
        metadata
            .segments
            .iter()
            .map(TimedTextRun::from_segment)
            .collect::<Vec<_>>()
    } else {
        metadata
            .words
            .iter()
            .map(TimedTextRun::from_word)
            .collect::<Vec<_>>()
    };

    let mut text_by_turn = std::collections::HashMap::<i64, Vec<String>>::new();
    for run in runs {
        if run.text.trim().is_empty() {
            continue;
        }
        if let Some(turn_id) = best_turn_for_timed_text_run(&turns, &run) {
            text_by_turn
                .entry(turn_id)
                .or_default()
                .push(run.text.trim().to_string());
        }
    }

    for turn in turns {
        let text = text_by_turn
            .remove(&turn.id)
            .map(|parts| parts.join(" "))
            .filter(|text| !text.trim().is_empty());
        sqlx::query(
            "UPDATE speaker_turns SET transcript_text = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
        )
        .bind(turn.id)
        .bind(text)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

async fn latest_transcription_metadata_for_audio_segment(
    transaction: &mut Transaction<'_, Sqlite>,
    audio_segment_id: i64,
) -> Result<Option<TranscriptionMetadata>> {
    let row = sqlx::query(
        "SELECT structured_payload_json FROM processing_results \
         WHERE subject_type = ?1 AND subject_id = ?2 AND processor = ?3 \
           AND structured_payload_json IS NOT NULL \
         ORDER BY id DESC LIMIT 1",
    )
    .bind(AUDIO_SEGMENT_SUBJECT_TYPE)
    .bind(audio_segment_id)
    .bind(AUDIO_TRANSCRIPTION_PROCESSOR)
    .fetch_optional(&mut **transaction)
    .await?;
    row.map(|row| serde_json::from_str(row.get::<String, _>("structured_payload_json").as_str()))
        .transpose()
        .map_err(AppInfraError::from)
}

#[derive(Debug, Clone)]
struct SpeakerTurnRange {
    id: i64,
    start_ms: u64,
    end_ms: u64,
}

async fn speaker_turn_ranges_for_audio_segment(
    transaction: &mut Transaction<'_, Sqlite>,
    audio_segment_id: i64,
) -> Result<Vec<SpeakerTurnRange>> {
    let rows = sqlx::query(
        "SELECT id, start_ms, end_ms FROM speaker_turns \
         WHERE audio_segment_id = ?1 ORDER BY start_ms ASC, end_ms ASC, id ASC",
    )
    .bind(audio_segment_id)
    .fetch_all(&mut **transaction)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| SpeakerTurnRange {
            id: row.get("id"),
            start_ms: u64::try_from(row.get::<i64, _>("start_ms")).unwrap_or_default(),
            end_ms: u64::try_from(row.get::<i64, _>("end_ms")).unwrap_or_default(),
        })
        .collect())
}

#[derive(Debug, Clone)]
struct TimedTextRun {
    start_ms: u64,
    end_ms: u64,
    text: String,
}

impl TimedTextRun {
    fn from_word(word: &TranscriptionWord) -> Self {
        Self {
            start_ms: word.start_ms,
            end_ms: word.end_ms,
            text: word.text.clone(),
        }
    }

    fn from_segment(segment: &TranscriptionSegment) -> Self {
        Self {
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            text: segment.text.clone(),
        }
    }
}

fn best_turn_for_timed_text_run(turns: &[SpeakerTurnRange], run: &TimedTextRun) -> Option<i64> {
    let run_midpoint = midpoint_ms(run.start_ms, run.end_ms);
    let mut best = None::<(&SpeakerTurnRange, u64, bool, u64)>;
    for turn in turns {
        let overlap = overlap_ms(turn.start_ms, turn.end_ms, run.start_ms, run.end_ms);
        if overlap == 0 {
            continue;
        }
        let contains_run_midpoint = turn.start_ms <= run_midpoint && run_midpoint <= turn.end_ms;
        let distance = midpoint_distance_ms(turn.start_ms, turn.end_ms, run.start_ms, run.end_ms);
        if best.is_none_or(
            |(best_turn, best_overlap, best_contains_midpoint, best_distance)| {
                overlap > best_overlap
                    || (overlap == best_overlap && contains_run_midpoint && !best_contains_midpoint)
                    || (overlap == best_overlap
                        && contains_run_midpoint == best_contains_midpoint
                        && distance < best_distance)
                    || (overlap == best_overlap
                        && contains_run_midpoint == best_contains_midpoint
                        && distance == best_distance
                        && turn.id < best_turn.id)
            },
        ) {
            best = Some((turn, overlap, contains_run_midpoint, distance));
        }
    }
    if let Some((turn, _, _, _)) = best {
        return Some(turn.id);
    }

    turns
        .iter()
        .map(|turn| {
            (
                turn,
                gap_ms(turn.start_ms, turn.end_ms, run.start_ms, run.end_ms),
                midpoint_distance_ms(turn.start_ms, turn.end_ms, run.start_ms, run.end_ms),
            )
        })
        .filter(|(_, gap, _)| *gap <= TIMED_TEXT_NEARBY_TURN_FALLBACK_MS)
        .min_by_key(|(turn, gap, distance)| (*gap, *distance, turn.id))
        .map(|(turn, _, _)| turn.id)
}

fn midpoint_ms(start_ms: u64, end_ms: u64) -> u64 {
    start_ms.saturating_add(end_ms) / 2
}

fn overlap_ms(start_a: u64, end_a: u64, start_b: u64, end_b: u64) -> u64 {
    end_a.min(end_b).saturating_sub(start_a.max(start_b))
}

fn gap_ms(start_a: u64, end_a: u64, start_b: u64, end_b: u64) -> u64 {
    if end_a < start_b {
        start_b - end_a
    } else if end_b < start_a {
        start_a - end_b
    } else {
        0
    }
}

fn midpoint_distance_ms(start_a: u64, end_a: u64, start_b: u64, end_b: u64) -> u64 {
    let mid_a = start_a.saturating_add(end_a) / 2;
    let mid_b = start_b.saturating_add(end_b) / 2;
    mid_a.abs_diff(mid_b)
}

fn f32_embedding_from_le_bytes(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return f32::NAN;
    }
    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;
    for (left, right) in left.iter().zip(right) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return f32::NAN;
    }
    dot / (left_norm.sqrt() * right_norm.sqrt())
}

fn recognition_confidence_as_str(confidence: &RecognitionConfidence) -> &'static str {
    match confidence {
        RecognitionConfidence::High => "high",
        RecognitionConfidence::Medium => "medium",
        RecognitionConfidence::Low => "low",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(id: i64, start_ms: u64, end_ms: u64) -> SpeakerTurnRange {
        SpeakerTurnRange {
            id,
            start_ms,
            end_ms,
        }
    }

    fn run(start_ms: u64, end_ms: u64) -> TimedTextRun {
        TimedTextRun {
            start_ms,
            end_ms,
            text: "hello".to_string(),
        }
    }

    fn candidate(id: i64, score: f32, person_id: Option<i64>) -> StableSpeakerClusterCandidate {
        StableSpeakerClusterCandidate {
            id,
            score,
            person_id,
        }
    }

    #[test]
    fn timed_text_alignment_picks_greatest_overlap() {
        let turns = vec![turn(1, 0, 800), turn(2, 400, 1_400)];

        assert_eq!(
            best_turn_for_timed_text_run(&turns, &run(500, 1_300)),
            Some(2)
        );
    }

    #[test]
    fn timed_text_alignment_prefers_midpoint_containing_turn_on_overlap_tie() {
        let turns = vec![turn(1, 0, 700), turn(2, 700, 1_300)];

        assert_eq!(
            best_turn_for_timed_text_run(&turns, &run(500, 900)),
            Some(2)
        );
    }

    #[test]
    fn timed_text_alignment_uses_midpoint_distance_after_overlap_and_midpoint_tie() {
        let turns = vec![turn(1, 100, 600), turn(2, 300, 1_000)];

        assert_eq!(
            best_turn_for_timed_text_run(&turns, &run(400, 800)),
            Some(2)
        );
    }

    #[test]
    fn timed_text_alignment_uses_earliest_turn_id_as_final_tie_breaker() {
        let turns = vec![turn(9, 0, 1_000), turn(3, 0, 1_000)];

        assert_eq!(
            best_turn_for_timed_text_run(&turns, &run(200, 800)),
            Some(3)
        );
    }

    #[test]
    fn timed_text_alignment_uses_nearby_fallback_within_limit() {
        let turns = vec![turn(1, 0, 1_000), turn(2, 2_000, 3_000)];

        assert_eq!(
            best_turn_for_timed_text_run(&turns, &run(1_450, 1_500)),
            Some(1)
        );
    }

    #[test]
    fn timed_text_alignment_leaves_unassigned_without_reasonable_turn() {
        let turns = vec![turn(1, 0, 1_000), turn(2, 3_000, 4_000)];

        assert_eq!(
            best_turn_for_timed_text_run(&turns, &run(1_600, 1_700)),
            None
        );
    }

    #[test]
    fn stable_cluster_resolution_auto_reuses_unambiguous_high_similarity() {
        let mut candidates = vec![candidate(1, 0.83, None), candidate(2, 0.70, None)];

        let resolution = resolve_stable_speaker_cluster_from_candidates(&mut candidates, None);

        assert_eq!(resolution.auto_merge_target_cluster_id, Some(1));
        assert_eq!(resolution.suggested_merge_target_cluster_id, None);
    }

    #[test]
    fn stable_cluster_resolution_suggests_for_ambiguous_high_similarity() {
        let mut candidates = vec![candidate(1, 0.83, None), candidate(2, 0.78, None)];

        let resolution = resolve_stable_speaker_cluster_from_candidates(&mut candidates, None);

        assert_eq!(resolution.auto_merge_target_cluster_id, None);
        assert_eq!(resolution.suggested_merge_target_cluster_id, Some(1));
    }

    #[test]
    fn stable_cluster_resolution_suggests_for_medium_similarity() {
        let mut candidates = vec![candidate(1, 0.70, None)];

        let resolution = resolve_stable_speaker_cluster_from_candidates(&mut candidates, None);

        assert_eq!(resolution.auto_merge_target_cluster_id, None);
        assert_eq!(resolution.suggested_merge_target_cluster_id, Some(1));
        assert_eq!(resolution.suggested_merge_score, Some(0.70));
    }

    #[test]
    fn stable_cluster_resolution_creates_independent_for_low_similarity() {
        let mut candidates = vec![candidate(1, 0.67, None)];

        let resolution = resolve_stable_speaker_cluster_from_candidates(&mut candidates, None);

        assert_eq!(resolution.auto_merge_target_cluster_id, None);
        assert_eq!(resolution.suggested_merge_target_cluster_id, None);
    }

    #[test]
    fn stable_cluster_resolution_has_no_match_when_provider_or_model_filter_removed_candidates() {
        let mut candidates = vec![];

        let resolution = resolve_stable_speaker_cluster_from_candidates(&mut candidates, None);

        assert_eq!(resolution.auto_merge_target_cluster_id, None);
        assert_eq!(resolution.suggested_merge_target_cluster_id, None);
    }

    #[test]
    fn stable_cluster_resolution_confirmed_person_conflict_blocks_auto_reuse() {
        let mut candidates = vec![candidate(1, 0.90, Some(10))];

        let resolution = resolve_stable_speaker_cluster_from_candidates(&mut candidates, Some(20));

        assert_eq!(resolution.auto_merge_target_cluster_id, None);
        assert_eq!(resolution.suggested_merge_target_cluster_id, Some(1));
    }
}

async fn insert_frame_record<'e, E>(executor: E, frame: &NewFrame) -> Result<i64>
where
    E: Executor<'e, Database = Sqlite>,
{
    let result = sqlx::query(
        "INSERT INTO frames (
            session_id,
            file_path,
            captured_at,
            width,
            height,
            equivalence_hint,
            equivalence_proof,
            equivalence_version,
            equivalence_status,
            equivalence_error,
            capture_segment_id
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )
    .bind(&frame.session_id)
    .bind(&frame.file_path)
    .bind(&frame.captured_at)
    .bind(frame.width)
    .bind(frame.height)
    .bind(frame.equivalence.hint.as_deref())
    .bind(frame.equivalence.proof.as_deref())
    .bind(frame.equivalence.version)
    .bind(
        frame
            .equivalence
            .status
            .as_ref()
            .map(FrameEquivalenceStatus::as_str),
    )
    .bind(frame.equivalence.error.as_deref())
    .bind(frame.capture_segment_id)
    .execute(executor)
    .await?;

    Ok(result.last_insert_rowid())
}

async fn insert_processing_job_record<'e, E>(
    executor: E,
    subject: &ProcessingSubject,
    processor: &str,
    payload_json: Option<&str>,
) -> Result<i64>
where
    E: Executor<'e, Database = Sqlite>,
{
    let result = sqlx::query(
        "INSERT INTO processing_jobs (subject_type, subject_id, processor, status, payload_json) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(subject.subject_type())
    .bind(subject.subject_id)
    .bind(processor)
    .bind(ProcessingJobStatus::Queued.as_str())
    .bind(payload_json)
    .execute(executor)
    .await?;

    Ok(result.last_insert_rowid())
}

async fn upsert_audio_segment_record<'e, E>(executor: E, segment: &NewAudioSegment) -> Result<()>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO audio_segments \
            (source_kind, source_session_id, segment_index, file_path, started_at, ended_at, capture_segment_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(source_kind, source_session_id, file_path) DO UPDATE SET \
            segment_index = excluded.segment_index, \
            started_at = excluded.started_at, \
            ended_at = excluded.ended_at, \
            capture_segment_id = COALESCE(excluded.capture_segment_id, audio_segments.capture_segment_id), \
            updated_at = CURRENT_TIMESTAMP",
    )
    .bind(segment.source_kind.as_str())
    .bind(&segment.source_session_id)
    .bind(segment.segment_index)
    .bind(&segment.file_path)
    .bind(&segment.started_at)
    .bind(&segment.ended_at)
    .bind(segment.capture_segment_id)
    .execute(executor)
    .await?;

    Ok(())
}

async fn get_audio_segment_by_unique_key<'e, E>(
    executor: E,
    segment: &NewAudioSegment,
) -> Result<AudioSegment>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query(
        "SELECT id, source_kind, source_session_id, segment_index, file_path, started_at, ended_at, capture_segment_id, created_at, updated_at \
         FROM audio_segments \
         WHERE source_kind = ?1 AND source_session_id = ?2 AND file_path = ?3",
    )
    .bind(segment.source_kind.as_str())
    .bind(&segment.source_session_id)
    .bind(&segment.file_path)
    .fetch_one(executor)
    .await?;

    map_audio_segment(row)
}

async fn get_audio_segment_optional<'e, E>(
    executor: E,
    audio_segment_id: i64,
) -> Result<Option<AudioSegment>>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query(
        "SELECT id, source_kind, source_session_id, segment_index, file_path, started_at, ended_at, capture_segment_id, created_at, updated_at \
         FROM audio_segments \
         WHERE id = ?1",
    )
    .bind(audio_segment_id)
    .fetch_optional(executor)
    .await?;

    row.map(map_audio_segment).transpose()
}

async fn get_frame_optional<'e, E>(executor: E, frame_id: i64) -> Result<Option<Frame>>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query(
        "SELECT id, session_id, file_path, captured_at, width, height, \
                equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                created_at, updated_at \
         FROM frames \
         WHERE id = ?1",
    )
    .bind(frame_id)
    .fetch_optional(executor)
    .await?;

    row.map(map_frame).transpose()
}

async fn get_processing_job_optional<'e, E>(
    executor: E,
    job_id: i64,
) -> Result<Option<ProcessingJob>>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query(
        "SELECT \
            id, subject_type, subject_id, processor, status, attempt_count, payload_json, last_error, \
            created_at, updated_at, started_at, finished_at \
         FROM processing_jobs \
         WHERE id = ?1",
    )
    .bind(job_id)
    .fetch_optional(executor)
    .await?;

    row.map(map_processing_job).transpose()
}

async fn get_processing_result_optional<'e, E>(
    executor: E,
    result_id: i64,
) -> Result<Option<ProcessingResult>>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query(
        "SELECT \
            id, job_id, subject_type, subject_id, processor, result_text, structured_payload_json, \
            processor_version, created_at \
         FROM processing_results \
         WHERE id = ?1",
    )
    .bind(result_id)
    .fetch_optional(executor)
    .await?;

    row.map(map_processing_result).transpose()
}

async fn delete_processing_result_for_job<'e, E>(executor: E, job_id: i64) -> Result<()>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query("DELETE FROM processing_results WHERE job_id = ?1")
        .bind(job_id)
        .execute(executor)
        .await?;
    Ok(())
}

fn map_audio_segment(row: SqliteRow) -> Result<AudioSegment> {
    Ok(AudioSegment {
        id: row.get("id"),
        source_kind: AudioSegmentSourceKind::from_str(row.get("source_kind")),
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

fn map_frame(row: SqliteRow) -> Result<Frame> {
    let equivalence_status = row
        .get::<Option<String>, _>("equivalence_status")
        .map(|status| {
            FrameEquivalenceStatus::from_str(&status)
                .ok_or(AppInfraError::InvalidFrameEquivalenceStatus(status))
        })
        .transpose()?;

    Ok(Frame {
        id: row.get("id"),
        session_id: row.get("session_id"),
        file_path: row.get("file_path"),
        captured_at: row.get("captured_at"),
        width: row.get("width"),
        height: row.get("height"),
        equivalence: FrameEquivalence {
            hint: row.get("equivalence_hint"),
            proof: row.get("equivalence_proof"),
            version: row.get("equivalence_version"),
            status: equivalence_status,
            error: row.get("equivalence_error"),
        },
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn map_frame_summary(row: SqliteRow) -> Result<FrameSummary> {
    Ok(FrameSummary {
        id: row.get("id"),
        captured_at: row.get("captured_at"),
    })
}

fn map_processing_job(row: SqliteRow) -> Result<ProcessingJob> {
    Ok(ProcessingJob {
        id: row.get("id"),
        subject_type: row.get("subject_type"),
        subject_id: row.get("subject_id"),
        processor: row.get("processor"),
        status: ProcessingJobStatus::from_str(row.get("status"))?,
        attempt_count: row.get("attempt_count"),
        payload_json: row.get("payload_json"),
        last_error: row.get("last_error"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
    })
}

async fn processing_job_model_cleanup_locked(
    transaction: &mut Transaction<'_, Sqlite>,
    job: &ProcessingJob,
) -> Result<bool> {
    let model_key = match processing_model_key_for_job(job) {
        Ok(Some(model_key)) => model_key,
        Ok(None) | Err(_) => return Ok(false),
    };

    let row = sqlx::query(
        "SELECT 1 FROM processing_model_cleanup_locks \
         WHERE processor = ?1 AND model_key = ?2 \
         LIMIT 1",
    )
    .bind(&job.processor)
    .bind(model_key)
    .fetch_optional(&mut **transaction)
    .await?;

    Ok(row.is_some())
}

fn processing_model_key_for_job(job: &ProcessingJob) -> Result<Option<String>> {
    match job.processor.as_str() {
        OCR_PROCESSOR => ocr_model_key_for_job(job),
        AUDIO_TRANSCRIPTION_PROCESSOR => audio_transcription_model_key_for_job(job),
        SPEAKER_ANALYSIS_PROCESSOR => speaker_analysis_model_key_for_job(job),
        _ => Ok(None),
    }
}

fn model_key(provider: &str, model_id: &str) -> String {
    format!("{provider}/{model_id}")
}

fn ocr_model_key_for_job(job: &ProcessingJob) -> Result<Option<String>> {
    let payload = ocr::FrozenOcrPayload::from_payload_json(job.payload_json.as_deref())
        .map_err(|error| AppInfraError::OcrEngine(error.to_string()))?;
    Ok(payload
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model_id| !model_id.is_empty())
        .map(|model_id| model_key(&payload.provider, model_id)))
}

fn audio_transcription_model_key_for_job(job: &ProcessingJob) -> Result<Option<String>> {
    let Some(payload_json) = job.payload_json.as_deref() else {
        return Ok(None);
    };
    let payload: AudioTranscriptionJobPayload = serde_json::from_str(payload_json)?;
    Ok(payload
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model_id| !model_id.is_empty())
        .map(|model_id| model_key(&payload.provider, model_id)))
}

fn speaker_analysis_model_key_for_job(job: &ProcessingJob) -> Result<Option<String>> {
    let Some(payload_json) = job.payload_json.as_deref() else {
        return Ok(None);
    };
    let payload: super::SpeakerAnalysisJobPayload = serde_json::from_str(payload_json)?;
    Ok(payload
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model_id| !model_id.is_empty())
        .map(|model_id| model_key(&payload.provider, model_id)))
}

fn map_processing_result(row: SqliteRow) -> Result<ProcessingResult> {
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

fn map_speaker_turn_view(row: SqliteRow) -> Result<SpeakerTurnView> {
    Ok(SpeakerTurnView {
        id: row.get("id"),
        audio_segment_id: row.get("audio_segment_id"),
        session_id: row.get("session_id"),
        cluster_id: row.get("cluster_id"),
        segment_cluster_id: row.get("segment_cluster_id"),
        provider_cluster_id: row.get("provider_cluster_id"),
        speaker_label: row.get("speaker_label"),
        person_id: row.get("person_id"),
        suggested_person_id: row.get("recognition_person_id"),
        recognition_confidence: row.get("recognition_confidence"),
        recognition_score: row
            .get::<Option<f64>, _>("recognition_score")
            .map(|score| score as f32),
        start_ms: u64::try_from(row.get::<i64, _>("start_ms")).unwrap_or_default(),
        end_ms: u64::try_from(row.get::<i64, _>("end_ms")).unwrap_or_default(),
        transcript_text: row.get("transcript_text"),
        overlaps: row.get::<i64, _>("overlaps") != 0,
    })
}

fn map_person_profile(row: SqliteRow) -> Result<PersonProfile> {
    Ok(PersonProfile {
        id: row.get("id"),
        display_name: row.get("display_name"),
        notes: row.get("notes"),
        embedding_count: row.get("embedding_count"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn map_speaker_cluster_view(row: SqliteRow) -> Result<SpeakerClusterView> {
    Ok(SpeakerClusterView {
        id: row.get("id"),
        session_id: row.get("session_id"),
        provider: row.get("provider"),
        model_id: row.get("model_id"),
        provider_cluster_id: row.get("provider_cluster_id"),
        speaker_label: row.get("speaker_label"),
        person_id: row.get("person_id"),
        suggested_person_id: row.get("recognition_person_id"),
        recognition_confidence: row.get("recognition_confidence"),
        recognition_score: row
            .get::<Option<f64>, _>("recognition_score")
            .map(|score| score as f32),
        suggested_merge_target_cluster_id: row.get("suggested_merge_target_cluster_id"),
        suggested_merge_score: row
            .get::<Option<f64>, _>("suggested_merge_score")
            .map(|score| score as f32),
    })
}

#[derive(Debug)]
struct SpeakerClusterRow {
    session_id: String,
    provider: String,
    model_id: Option<String>,
    person_id: Option<i64>,
    recognition_person_id: Option<i64>,
    embedding: Option<Vec<u8>>,
}

async fn get_speaker_cluster_row<'e, E>(executor: E, cluster_id: i64) -> Result<SpeakerClusterRow>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query(
        "SELECT session_id, provider, model_id, person_id, recognition_person_id, embedding \
         FROM recording_speaker_clusters \
         WHERE id = ?1",
    )
    .bind(cluster_id)
    .fetch_one(executor)
    .await?;
    Ok(SpeakerClusterRow {
        session_id: row.get("session_id"),
        provider: row.get("provider"),
        model_id: row.get("model_id"),
        person_id: row.get("person_id"),
        recognition_person_id: row.get("recognition_person_id"),
        embedding: row.get("embedding"),
    })
}

async fn persist_speaker_recognition_rejection_for_cluster(
    transaction: &mut Transaction<'_, Sqlite>,
    cluster: &SpeakerClusterRow,
    cluster_id: i64,
    person_id: Option<i64>,
) -> Result<()> {
    let (Some(person_id), Some(embedding)) = (person_id, cluster.embedding.as_ref()) else {
        return Ok(());
    };
    sqlx::query(
        "INSERT OR IGNORE INTO speaker_recognition_rejections (\
            person_id, provider, model_id, embedding, source_session_id, source_cluster_id\
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(person_id)
    .bind(&cluster.provider)
    .bind(cluster.model_id.as_deref().unwrap_or(""))
    .bind(embedding)
    .bind(&cluster.session_id)
    .bind(cluster_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn processing_job_invalid_transition(
    job_id: i64,
    from: &ProcessingJobStatus,
    to: &str,
) -> AppInfraError {
    AppInfraError::ProcessingJobInvalidTransition {
        job_id,
        from: from.as_str().to_string(),
        to: to.to_string(),
    }
}
