use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, Executor, QueryBuilder, Row, Sqlite, SqlitePool, Transaction};

use crate::{AppInfraError, AudioSegment, AudioSegmentSourceKind, NewAudioSegment, Result};

use super::{
    AudioTranscriptionJobPayload, Frame, FrameEquivalence, FrameEquivalenceStatus, FrameSummary,
    NewFrame, ProcessingJob, ProcessingJobDraft, ProcessingJobStatus, ProcessingResult,
    ProcessingResultDraft, ProcessingSubject, AUDIO_TRANSCRIPTION_PROCESSOR, OCR_PROCESSOR,
};

pub(crate) const ORPHANED_RUNNING_PROCESSING_JOB_ERROR: &str =
    "processing job was marked failed during startup recovery after the app shut down while it was running";

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
        self.claim_next_queued_job_matching_processor(None, None)
            .await
    }

    pub async fn claim_next_queued_job_for_processor(
        &self,
        processor: &str,
    ) -> Result<Option<ProcessingJob>> {
        self.claim_next_queued_job_matching_processor(Some(processor), None)
            .await
    }

    pub async fn claim_next_queued_job_excluding_processor(
        &self,
        excluded_processor: &str,
    ) -> Result<Option<ProcessingJob>> {
        self.claim_next_queued_job_matching_processor(None, Some(excluded_processor))
            .await
    }

    async fn claim_next_queued_job_matching_processor(
        &self,
        processor: Option<&str>,
        excluded_processor: Option<&str>,
    ) -> Result<Option<ProcessingJob>> {
        loop {
            let mut transaction = self.pool.begin().await?;
            let rows = match (processor, excluded_processor) {
                (Some(processor), _) => sqlx::query(
                    "SELECT \
                        id, subject_type, subject_id, processor, status, attempt_count, payload_json, last_error, \
                        created_at, updated_at, started_at, finished_at \
                     FROM processing_jobs \
                     WHERE status = 'queued' AND processor = ?1 \
                     ORDER BY id ASC",
                )
                .bind(processor)
                .fetch_all(&mut *transaction)
                .await?,
                (None, Some(excluded_processor)) => sqlx::query(
                    "SELECT \
                        id, subject_type, subject_id, processor, status, attempt_count, payload_json, last_error, \
                        created_at, updated_at, started_at, finished_at \
                     FROM processing_jobs \
                     WHERE status = 'queued' AND processor != ?1 \
                     ORDER BY id ASC",
                )
                .bind(excluded_processor)
                .fetch_all(&mut *transaction)
                .await?,
                (None, None) => sqlx::query(
                    "SELECT \
                        id, subject_type, subject_id, processor, status, attempt_count, payload_json, last_error, \
                        created_at, updated_at, started_at, finished_at \
                     FROM processing_jobs \
                     WHERE status = 'queued' \
                     ORDER BY id ASC",
                )
                .fetch_all(&mut *transaction)
                .await?,
            };

            let mut claimable_job_id = None;
            for row in rows {
                let job = map_processing_job(row)?;
                if processing_job_model_cleanup_locked(&mut transaction, &job).await? {
                    continue;
                }
                claimable_job_id = Some(job.id);
                break;
            }

            let job_id = claimable_job_id;
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
            equivalence_error
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
            (source_kind, source_session_id, segment_index, file_path, started_at, ended_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(source_kind, source_session_id, file_path) DO UPDATE SET \
            segment_index = excluded.segment_index, \
            started_at = excluded.started_at, \
            ended_at = excluded.ended_at, \
            updated_at = CURRENT_TIMESTAMP",
    )
    .bind(segment.source_kind.as_str())
    .bind(&segment.source_session_id)
    .bind(segment.segment_index)
    .bind(&segment.file_path)
    .bind(&segment.started_at)
    .bind(&segment.ended_at)
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
        "SELECT id, source_kind, source_session_id, segment_index, file_path, started_at, ended_at, created_at, updated_at \
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
        "SELECT id, source_kind, source_session_id, segment_index, file_path, started_at, ended_at, created_at, updated_at \
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
    let Some(model_key) = processing_model_key_for_job(job)? else {
        return Ok(false);
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
