use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, Executor, Row, Sqlite, SqlitePool, Transaction};

use crate::{AppInfraError, Result};

use super::{
    Frame, NewFrame, ProcessingJob, ProcessingJobDraft, ProcessingJobStatus, ProcessingResult,
    ProcessingResultDraft, ProcessingSubject, OCR_PROCESSOR,
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
pub struct FrameOcrEnqueueResult {
    pub frame: Frame,
    pub job: Option<ProcessingJob>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingJobCompletion {
    pub job: ProcessingJob,
    pub result: ProcessingResult,
}

#[derive(Clone)]
pub struct ProcessingStore {
    pool: SqlitePool,
}

impl ProcessingStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert_frame(&self, frame: &NewFrame) -> Result<Frame> {
        let frame_id = insert_frame_record(&self.pool, frame).await?;
        self.get_required_frame(frame_id).await
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

    pub async fn insert_frame_and_enqueue_processor_job(
        &self,
        frame: &NewFrame,
        processor: &str,
        payload_json: Option<&str>,
    ) -> Result<FrameProcessingJob> {
        let mut transaction = self.pool.begin().await?;

        let frame_id = insert_frame_record(&mut *transaction, frame).await?;
        let subject = ProcessingSubject::frame(frame_id);
        let job_id =
            insert_processing_job_record(&mut *transaction, &subject, processor, payload_json)
                .await?;

        let stored_frame = get_frame_optional(&mut *transaction, frame_id)
            .await?
            .ok_or(AppInfraError::FrameNotFound(frame_id))?;
        let stored_job = get_processing_job_optional(&mut *transaction, job_id)
            .await?
            .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?;

        transaction.commit().await?;

        Ok(FrameProcessingJob {
            frame: stored_frame,
            job: stored_job,
        })
    }

    pub async fn insert_frame_and_enqueue_job(
        &self,
        frame: &NewFrame,
        job: &ProcessingJobDraft,
    ) -> Result<FrameProcessingJob> {
        self.insert_frame_and_enqueue_processor_job(
            frame,
            &job.processor,
            job.payload_json.as_deref(),
        )
        .await
    }

    pub async fn insert_frame_and_enqueue_ocr_job(
        &self,
        frame: &NewFrame,
        payload_json: Option<&str>,
    ) -> Result<FrameProcessingJob> {
        self.insert_frame_and_enqueue_processor_job(frame, OCR_PROCESSOR, payload_json)
            .await
    }

    pub async fn insert_frame_and_maybe_enqueue_ocr_job(
        &self,
        frame: &NewFrame,
        payload_json: Option<&str>,
    ) -> Result<FrameOcrEnqueueResult> {
        let mut transaction = self.pool.begin().await?;

        let result = self
            .insert_frame_and_maybe_enqueue_ocr_job_in_transaction(
                &mut transaction,
                frame,
                payload_json,
            )
            .await?;

        transaction.commit().await?;

        Ok(result)
    }

    pub(crate) async fn insert_frame_and_maybe_enqueue_ocr_job_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        frame: &NewFrame,
        payload_json: Option<&str>,
    ) -> Result<FrameOcrEnqueueResult> {
        let frame_id = insert_frame_record(&mut **transaction, frame).await?;
        let stored_frame = get_frame_optional(&mut **transaction, frame_id)
            .await?
            .ok_or(AppInfraError::FrameNotFound(frame_id))?;

        let should_enqueue =
            should_enqueue_ocr_for_frame(&mut **transaction, &stored_frame).await?;
        let stored_job = if should_enqueue {
            let subject = ProcessingSubject::frame(frame_id);
            let job_id = insert_processing_job_record(
                &mut **transaction,
                &subject,
                OCR_PROCESSOR,
                payload_json,
            )
            .await?;

            Some(
                get_processing_job_optional(&mut **transaction, job_id)
                    .await?
                    .ok_or(AppInfraError::ProcessingJobNotFound(job_id))?,
            )
        } else {
            None
        };

        Ok(FrameOcrEnqueueResult {
            frame: stored_frame,
            job: stored_job,
        })
    }

    pub async fn get_frame(&self, frame_id: i64) -> Result<Option<Frame>> {
        get_frame_optional(&self.pool, frame_id).await
    }

    pub async fn list_frames(&self, session_id: Option<&str>) -> Result<Vec<Frame>> {
        let rows = match session_id {
            Some(session_id) => {
                sqlx::query(
                    "SELECT id, session_id, file_path, captured_at, width, height, content_fingerprint, created_at, updated_at \
                     FROM frames \
                     WHERE session_id = ?1 \
                     ORDER BY id DESC",
                )
                .bind(session_id)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT id, session_id, file_path, captured_at, width, height, content_fingerprint, created_at, updated_at \
                     FROM frames \
                     ORDER BY id DESC",
                )
                .fetch_all(&self.pool)
                .await?
            }
        };

        rows.into_iter().map(map_frame).collect()
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

    pub async fn claim_next_queued_job(&self) -> Result<Option<ProcessingJob>> {
        loop {
            let mut transaction = self.pool.begin().await?;
            let job_id = sqlx::query(
                "SELECT id FROM processing_jobs WHERE status = 'queued' ORDER BY id ASC LIMIT 1",
            )
            .fetch_optional(&mut *transaction)
            .await?
            .map(|row| row.get::<i64, _>("id"));

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
        "INSERT INTO frames (session_id, file_path, captured_at, width, height, content_fingerprint) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&frame.session_id)
    .bind(&frame.file_path)
    .bind(&frame.captured_at)
    .bind(frame.width)
    .bind(frame.height)
    .bind(frame.content_fingerprint.as_deref())
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

async fn get_frame_optional<'e, E>(executor: E, frame_id: i64) -> Result<Option<Frame>>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query(
        "SELECT id, session_id, file_path, captured_at, width, height, content_fingerprint, created_at, updated_at \
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

fn map_frame(row: SqliteRow) -> Result<Frame> {
    Ok(Frame {
        id: row.get("id"),
        session_id: row.get("session_id"),
        file_path: row.get("file_path"),
        captured_at: row.get("captured_at"),
        width: row.get("width"),
        height: row.get("height"),
        content_fingerprint: row.get("content_fingerprint"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

async fn should_enqueue_ocr_for_frame<'e, E>(executor: E, frame: &Frame) -> Result<bool>
where
    E: Executor<'e, Database = Sqlite>,
{
    let Some(content_fingerprint) = frame.content_fingerprint.as_deref() else {
        return Ok(true);
    };

    let previous_fingerprint = sqlx::query(
        "SELECT 1 \
         FROM frames \
         WHERE session_id = ?1 AND id < ?2 AND content_fingerprint = ?3 \
         LIMIT 1",
    )
    .bind(&frame.session_id)
    .bind(frame.id)
    .bind(content_fingerprint)
    .fetch_optional(executor)
    .await?;

    Ok(previous_fingerprint.is_none())
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
