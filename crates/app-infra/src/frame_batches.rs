use std::path::Path;

use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, Executor, Row, Sqlite, SqlitePool, Transaction};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::{
    jobs::{BackgroundJob, BackgroundJobStatus, JobDescriptor, JobStore},
    processing::{Frame, FRAME_SUBJECT_TYPE, OCR_PROCESSOR},
    AppInfraError, Result,
};

pub const FRAME_BATCH_DURATION_MINUTES: i64 = 10;
/// Job kind identifier used when enqueuing frame-batch finalization work.
pub const FRAME_BATCH_FINALIZE_JOB_KIND: &str = "frame_batch_combine";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FrameBatchStatus {
    Open,
    Closed,
    Processing,
    Completed,
    Failed,
}

impl FrameBatchStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Processing => "processing",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            "processing" => Ok(Self::Processing),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            other => Err(AppInfraError::InvalidFrameBatchStatus(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrameBatchWindow {
    pub batch_key: String,
    pub started_at: String,
    pub ended_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrameBatch {
    pub id: i64,
    pub session_id: String,
    pub batch_key: String,
    pub batch_started_at: String,
    pub batch_ended_at: String,
    pub status: FrameBatchStatus,
    pub frame_count: i64,
    pub first_frame_at: Option<String>,
    pub last_frame_at: Option<String>,
    pub finalize_job_id: Option<i64>,
    pub finalized_output_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub completed_at: Option<String>,
    pub failed_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrameBatchWithFrames {
    pub batch: FrameBatch,
    pub frames: Vec<Frame>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrameBatchFinalizePayload {
    pub batch_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrameBatchFinalizeResult {
    pub batch: FrameBatch,
}

#[derive(Clone)]
pub struct FrameBatchStore {
    pool: SqlitePool,
    jobs: JobStore,
}

#[derive(Clone)]
pub struct FrameBatchRuntime {
    store: FrameBatchStore,
}

impl FrameBatchStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self {
            jobs: JobStore::new(pool.clone()),
            pool,
        }
    }

    pub async fn upsert_open_batch_for_frame(
        &self,
        session_id: &str,
        captured_at: &str,
    ) -> Result<FrameBatch> {
        let mut transaction = self.pool.begin().await?;
        let batch = self
            .upsert_open_batch_for_frame_in_transaction(&mut transaction, session_id, captured_at)
            .await?;
        transaction.commit().await?;

        Ok(batch)
    }

    pub(crate) async fn upsert_open_batch_for_frame_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        session_id: &str,
        captured_at: &str,
    ) -> Result<FrameBatch> {
        let window = frame_batch_window(captured_at)?;

        let result = sqlx::query(
            "INSERT INTO frame_batches (session_id, batch_key, batch_started_at, batch_ended_at, status) \
             VALUES (?1, ?2, ?3, ?4, 'open') \
             ON CONFLICT(session_id, batch_key) DO NOTHING",
        )
        .bind(session_id)
        .bind(&window.batch_key)
        .bind(&window.started_at)
        .bind(&window.ended_at)
        .execute(&mut **transaction)
        .await?;

        let batch_id = if result.rows_affected() > 0 {
            result.last_insert_rowid()
        } else {
            sqlx::query(
                "SELECT id FROM frame_batches WHERE session_id = ?1 AND batch_key = ?2 LIMIT 1",
            )
            .bind(session_id)
            .bind(&window.batch_key)
            .fetch_one(&mut **transaction)
            .await?
            .get::<i64, _>("id")
        };

        get_frame_batch_optional(&mut **transaction, batch_id)
            .await?
            .ok_or(AppInfraError::FrameBatchNotFound(batch_id))
    }

    pub async fn attach_frame_to_batch(
        &self,
        frame_id: i64,
        batch_id: i64,
        captured_at: &str,
    ) -> Result<FrameBatch> {
        let mut transaction = self.pool.begin().await?;

        let batch = self
            .attach_frame_to_batch_in_transaction(&mut transaction, frame_id, batch_id, captured_at)
            .await?;

        transaction.commit().await?;

        Ok(batch)
    }

    pub(crate) async fn attach_frame_to_batch_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        frame_id: i64,
        batch_id: i64,
        captured_at: &str,
    ) -> Result<FrameBatch> {
        let attached = sqlx::query("UPDATE frames SET frame_batch_id = ?2 WHERE id = ?1")
            .bind(frame_id)
            .bind(batch_id)
            .execute(&mut **transaction)
            .await?;

        if attached.rows_affected() == 0 {
            return Err(AppInfraError::FrameNotFound(frame_id));
        }

        sqlx::query(
            "UPDATE frame_batches \
             SET frame_count = frame_count + 1, \
                 first_frame_at = COALESCE(first_frame_at, ?2), \
                 last_frame_at = CASE \
                     WHEN last_frame_at IS NULL OR last_frame_at < ?2 THEN ?2 \
                     ELSE last_frame_at \
                 END, \
                  updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1",
        )
        .bind(batch_id)
        .bind(captured_at)
        .execute(&mut **transaction)
        .await?;

        let batch = get_frame_batch_optional(&mut **transaction, batch_id)
            .await?
            .ok_or(AppInfraError::FrameBatchNotFound(batch_id))?;

        Ok(batch)
    }

    pub async fn reconcile_closed_batches_without_finalize_jobs(&self) -> Result<u64> {
        sqlx::query(
            "UPDATE frame_batches \
             SET finalize_job_id = NULL, updated_at = CURRENT_TIMESTAMP \
              WHERE status = 'closed' \
                AND finalize_job_id IS NOT NULL \
                AND NOT EXISTS ( \
                    SELECT 1 FROM background_jobs \
                    WHERE background_jobs.id = frame_batches.finalize_job_id \
                      AND background_jobs.kind = ?1 \
                     AND background_jobs.status IN ('queued', 'running', 'completed') \
               )",
        )
        .bind(FRAME_BATCH_FINALIZE_JOB_KIND)
        .execute(&self.pool)
        .await?;

        let rows = sqlx::query(
            "SELECT id FROM frame_batches WHERE status = 'closed' AND finalize_job_id IS NULL ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut scheduled = 0;
        for row in rows {
            if self
                .enqueue_finalize_job_if_needed(row.get::<i64, _>("id"))
                .await?
                .is_some()
            {
                scheduled += 1;
            }
        }

        Ok(scheduled)
    }

    pub async fn get(&self, batch_id: i64) -> Result<Option<FrameBatch>> {
        get_frame_batch_optional(&self.pool, batch_id).await
    }

    pub async fn get_required(&self, batch_id: i64) -> Result<FrameBatch> {
        self.get(batch_id)
            .await?
            .ok_or(AppInfraError::FrameBatchNotFound(batch_id))
    }

    pub async fn list_batches(&self, session_id: Option<&str>) -> Result<Vec<FrameBatch>> {
        let rows = match session_id {
            Some(session_id) => {
                sqlx::query(
                    "SELECT id, session_id, batch_key, batch_started_at, batch_ended_at, status, frame_count, \
                        first_frame_at, last_frame_at, finalize_job_id, finalized_output_path, created_at, \
                        updated_at, closed_at, completed_at, failed_at, last_error \
                     FROM frame_batches \
                     WHERE session_id = ?1 \
                     ORDER BY id DESC",
                )
                .bind(session_id)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT id, session_id, batch_key, batch_started_at, batch_ended_at, status, frame_count, \
                        first_frame_at, last_frame_at, finalize_job_id, finalized_output_path, created_at, \
                        updated_at, closed_at, completed_at, failed_at, last_error \
                     FROM frame_batches \
                     ORDER BY id DESC",
                )
                .fetch_all(&self.pool)
                .await?
            }
        };

        rows.into_iter().map(map_frame_batch).collect()
    }

    pub async fn list_frames_for_batch(&self, batch_id: i64) -> Result<Vec<Frame>> {
        let rows = sqlx::query(
            "SELECT id, session_id, file_path, captured_at, width, height, content_fingerprint, created_at, updated_at \
             FROM frames \
             WHERE frame_batch_id = ?1 \
             ORDER BY id ASC",
        )
        .bind(batch_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_frame).collect()
    }

    pub async fn close_completed_batches_for_session(
        &self,
        session_id: &str,
        active_batch_id: Option<i64>,
    ) -> Result<Vec<FrameBatch>> {
        let mut transaction = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT id FROM frame_batches \
             WHERE session_id = ?1 AND status = 'open' AND (?2 IS NULL OR id != ?2) \
             ORDER BY id ASC",
        )
        .bind(session_id)
        .bind(active_batch_id)
        .fetch_all(&mut *transaction)
        .await?;

        let mut closed = Vec::new();
        for row in rows {
            let batch_id = row.get::<i64, _>("id");
            sqlx::query(
                "UPDATE frame_batches \
                 SET status = 'closed', closed_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP \
                 WHERE id = ?1 AND status = 'open'",
            )
            .bind(batch_id)
            .execute(&mut *transaction)
            .await?;

            if let Some(batch) = get_frame_batch_optional(&mut *transaction, batch_id).await? {
                closed.push(batch);
            }
        }

        transaction.commit().await?;
        Ok(closed)
    }

    pub async fn enqueue_finalize_job_if_needed(
        &self,
        batch_id: i64,
    ) -> Result<Option<BackgroundJob>> {
        let mut transaction = self.pool.begin().await?;
        let batch = get_frame_batch_optional(&mut *transaction, batch_id)
            .await?
            .ok_or(AppInfraError::FrameBatchNotFound(batch_id))?;

        if batch.status != FrameBatchStatus::Closed || batch.finalize_job_id.is_some() {
            transaction.commit().await?;
            return Ok(None);
        }

        let payload = serde_json::to_string(&FrameBatchFinalizePayload { batch_id })?;
        let job_result = sqlx::query(
            "INSERT INTO background_jobs (kind, status, payload_json) VALUES (?1, ?2, ?3)",
        )
        .bind(JobDescriptor::new(FRAME_BATCH_FINALIZE_JOB_KIND).kind())
        .bind(BackgroundJobStatus::Queued.as_str())
        .bind(&payload)
        .execute(&mut *transaction)
        .await?;
        let job_id = job_result.last_insert_rowid();

        let updated = sqlx::query(
            "UPDATE frame_batches \
             SET finalize_job_id = ?2, updated_at = CURRENT_TIMESTAMP \
              WHERE id = ?1 AND finalize_job_id IS NULL",
        )
        .bind(batch_id)
        .bind(job_id)
        .execute(&mut *transaction)
        .await?;

        if updated.rows_affected() == 0 {
            transaction.rollback().await?;
            return Ok(None);
        }

        let job = get_background_job_optional(&mut *transaction, job_id)
            .await?
            .ok_or(AppInfraError::JobNotFound(job_id))?;

        transaction.commit().await?;
        Ok(Some(job))
    }

    pub async fn claim_next_finalize_job(&self) -> Result<Option<BackgroundJob>> {
        loop {
            let mut transaction = self.pool.begin().await?;
            let row = sqlx::query(
                "SELECT background_jobs.id FROM background_jobs \
                 INNER JOIN frame_batches ON frame_batches.finalize_job_id = background_jobs.id \
                 WHERE background_jobs.kind = ?1 AND background_jobs.status = 'queued' \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM processing_jobs jobs \
                       INNER JOIN frames ON frames.id = jobs.subject_id \
                       WHERE frames.frame_batch_id = frame_batches.id \
                         AND jobs.subject_type = ?2 \
                         AND jobs.processor = ?3 \
                         AND jobs.status NOT IN ('completed', 'failed') \
                   ) \
                 ORDER BY background_jobs.id ASC LIMIT 1",
            )
            .bind(FRAME_BATCH_FINALIZE_JOB_KIND)
            .bind(FRAME_SUBJECT_TYPE)
            .bind(OCR_PROCESSOR)
            .fetch_optional(&mut *transaction)
            .await?;

            let Some(row) = row else {
                transaction.commit().await?;
                return Ok(None);
            };

            let job_id = row.get::<i64, _>("id");
            let update = sqlx::query(
                "UPDATE background_jobs \
                 SET status = 'running', attempt_count = attempt_count + 1, last_error = NULL, \
                     result_json = NULL, started_at = CURRENT_TIMESTAMP, finished_at = NULL, \
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

            let job = get_background_job_optional(&mut *transaction, job_id)
                .await?
                .ok_or(AppInfraError::JobNotFound(job_id))?;
            transaction.commit().await?;
            return Ok(Some(job));
        }
    }

    pub async fn payload_for_job(&self, job_id: i64) -> Result<FrameBatchFinalizePayload> {
        let job = self
            .jobs
            .get(job_id)
            .await?
            .ok_or(AppInfraError::JobNotFound(job_id))?;
        serde_json::from_str(job.payload_json.as_deref().unwrap_or("{}"))
            .map_err(AppInfraError::from)
    }

    pub async fn batch_with_frames_for_job(
        &self,
        job_id: i64,
    ) -> Result<Option<FrameBatchWithFrames>> {
        let payload = self.payload_for_job(job_id).await?;
        let batch = match self.get(payload.batch_id).await? {
            Some(batch) => batch,
            None => return Ok(None),
        };
        let frames = self.list_frames_for_batch(batch.id).await?;

        Ok(Some(FrameBatchWithFrames { batch, frames }))
    }

    pub async fn mark_batch_processing(&self, batch_id: i64) -> Result<FrameBatch> {
        self.transition_batch_status(
            batch_id,
            &[FrameBatchStatus::Closed],
            FrameBatchStatus::Processing,
        )
        .await
    }

    pub async fn mark_batch_completed(
        &self,
        batch_id: i64,
        finalized_output_path: Option<&str>,
    ) -> Result<FrameBatch> {
        sqlx::query(
            "UPDATE frame_batches \
             SET status = 'completed', finalized_output_path = ?2, completed_at = CURRENT_TIMESTAMP, \
                 failed_at = NULL, last_error = NULL, updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1 AND status = 'processing'",
        )
        .bind(batch_id)
        .bind(finalized_output_path)
        .execute(&self.pool)
        .await?;

        self.get_required(batch_id).await
    }

    pub async fn mark_batch_failed(&self, batch_id: i64, error_text: &str) -> Result<FrameBatch> {
        sqlx::query(
            "UPDATE frame_batches \
             SET status = 'failed', failed_at = CURRENT_TIMESTAMP, last_error = ?2, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1",
        )
        .bind(batch_id)
        .bind(error_text)
        .execute(&self.pool)
        .await?;

        self.get_required(batch_id).await
    }

    pub async fn mark_job_back_to_queued(&self, job_id: i64) -> Result<BackgroundJob> {
        sqlx::query(
            "UPDATE background_jobs \
             SET status = 'queued', last_error = NULL, finished_at = NULL, updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1 AND status = 'running'",
        )
        .bind(job_id)
        .execute(&self.pool)
        .await?;

        self.jobs
            .get(job_id)
            .await?
            .ok_or(AppInfraError::JobNotFound(job_id))
    }

    async fn transition_batch_status(
        &self,
        batch_id: i64,
        from: &[FrameBatchStatus],
        to: FrameBatchStatus,
    ) -> Result<FrameBatch> {
        let current = self.get_required(batch_id).await?;
        if !from.iter().any(|status| status == &current.status) {
            return Err(AppInfraError::FrameBatchInvalidTransition {
                batch_id,
                from: current.status.as_str().to_string(),
                to: to.as_str().to_string(),
            });
        }

        let statuses = from
            .iter()
            .map(FrameBatchStatus::as_str)
            .collect::<Vec<_>>()
            .join("', '");
        let query = format!(
            "UPDATE frame_batches SET status = '{}', updated_at = CURRENT_TIMESTAMP WHERE id = ?1 AND status IN ('{}')",
            to.as_str(), statuses
        );

        sqlx::query(&query)
            .bind(batch_id)
            .execute(&self.pool)
            .await?;
        self.get_required(batch_id).await
    }

    pub async fn is_batch_ocr_terminal(&self, batch_id: i64) -> Result<bool> {
        let pending = sqlx::query(
            "SELECT COUNT(*) AS pending_count \
             FROM processing_jobs jobs \
             INNER JOIN frames ON frames.id = jobs.subject_id \
             WHERE frames.frame_batch_id = ?1 \
               AND jobs.subject_type = ?2 \
               AND jobs.processor = ?3 \
               AND jobs.status NOT IN ('completed', 'failed')",
        )
        .bind(batch_id)
        .bind(FRAME_SUBJECT_TYPE)
        .bind(OCR_PROCESSOR)
        .fetch_one(&self.pool)
        .await?
        .get::<i64, _>("pending_count");

        Ok(pending == 0)
    }

    pub async fn close_and_schedule_completed_batches_for_frame(
        &self,
        session_id: &str,
        active_batch_id: i64,
    ) -> Result<Vec<FrameBatch>> {
        let closed = self
            .close_completed_batches_for_session(session_id, Some(active_batch_id))
            .await?;

        let mut first_error: Option<AppInfraError> = None;
        for batch in &closed {
            if let Err(error) = self.enqueue_finalize_job_if_needed(batch.id).await {
                eprintln!(
                    "failed to schedule finalize job for batch {}: {error}",
                    batch.id
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }

        if let Some(error) = first_error {
            return Err(error);
        }

        Ok(closed)
    }
}

impl FrameBatchRuntime {
    pub fn new(store: FrameBatchStore) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &FrameBatchStore {
        &self.store
    }

    pub async fn process_next_queued_job(&self) -> Result<Option<FrameBatchFinalizeResult>> {
        let Some(job) = self.store.claim_next_finalize_job().await? else {
            return Ok(None);
        };

        self.process_job(job.id).await.map(Some)
    }

    pub async fn process_job(&self, job_id: i64) -> Result<FrameBatchFinalizeResult> {
        let Some(batch_with_frames) = self.store.batch_with_frames_for_job(job_id).await? else {
            return Err(AppInfraError::JobNotFound(job_id));
        };

        let batch_id = batch_with_frames.batch.id;
        if !self.store.is_batch_ocr_terminal(batch_id).await? {
            self.store.mark_job_back_to_queued(job_id).await?;
            return Err(AppInfraError::FrameBatchOcrPending { batch_id });
        }

        if batch_with_frames.frames.is_empty() {
            let error = AppInfraError::EmptyFrameBatch { batch_id };
            self.store
                .jobs
                .mark_failed(job_id, Some(&error.to_string()))
                .await?;
            self.store
                .mark_batch_failed(batch_id, &error.to_string())
                .await?;
            return Err(error);
        }

        self.store.mark_batch_processing(batch_id).await?;

        // Clean up artifacts while the batch is still in "processing" state so
        // a crash during cleanup leaves both the batch and job retryable.
        let cleanup_errors = cleanup_frame_artifacts(&batch_with_frames.frames);
        if !cleanup_errors.is_empty() {
            eprintln!(
                "frame artifact cleanup for batch {batch_id}: {} file(s) failed to delete",
                cleanup_errors.len()
            );
            for (path, error) in &cleanup_errors {
                eprintln!("  failed to delete {path}: {error}");
            }
        }

        let batch = self
            .store
            .mark_batch_completed(batch_id, None)
            .await?;

        let result = FrameBatchFinalizeResult {
            batch: batch.clone(),
        };
        let result_json = serde_json::to_string(&result)?;
        self.store
            .jobs
            .mark_completed(job_id, Some(&result_json))
            .await?;

        Ok(result)
    }
}

fn frame_batch_window(captured_at: &str) -> Result<FrameBatchWindow> {
    let parsed = OffsetDateTime::parse(captured_at, &Rfc3339)
        .map_err(|_| AppInfraError::InvalidFrameBatchTimestamp(captured_at.to_string()))?;
    let timestamp = parsed.unix_timestamp();
    let batch_seconds = FRAME_BATCH_DURATION_MINUTES * 60;
    let batch_start = timestamp.div_euclid(batch_seconds) * batch_seconds;
    let batch_end = batch_start + batch_seconds;
    let started_at = OffsetDateTime::from_unix_timestamp(batch_start)
        .map_err(|_| AppInfraError::InvalidFrameBatchTimestamp(captured_at.to_string()))?;
    let ended_at = OffsetDateTime::from_unix_timestamp(batch_end)
        .map_err(|_| AppInfraError::InvalidFrameBatchTimestamp(captured_at.to_string()))?;

    Ok(FrameBatchWindow {
        batch_key: format!(
            "{}-{}",
            started_at.unix_timestamp(),
            ended_at.unix_timestamp()
        ),
        started_at: started_at
            .format(&Rfc3339)
            .map_err(|_| AppInfraError::InvalidFrameBatchTimestamp(captured_at.to_string()))?,
        ended_at: ended_at
            .format(&Rfc3339)
            .map_err(|_| AppInfraError::InvalidFrameBatchTimestamp(captured_at.to_string()))?,
    })
}

/// Returns `true` when `path` looks like an exported frame PNG artifact
/// matching the expected session export path shape:
///   `<root>/<session_id>-segment-<NNNN>/frames/frame-*.png`
///
/// Checks: absolute, no `..` components, parent directory named `frames`,
/// grandparent directory matching `*-segment-*`, and filename matching
/// `frame-*.png`.
fn is_safe_frame_artifact_path(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    if path.components().any(|c| c == std::path::Component::ParentDir) {
        return false;
    }
    let frames_dir = match path.parent() {
        Some(p) => p,
        None => return false,
    };
    let parent_is_frames = frames_dir
        .file_name()
        .map_or(false, |name| name == "frames");
    if !parent_is_frames {
        return false;
    }
    // Grandparent must be a session segment directory (*-segment-*)
    let segment_dir_name = frames_dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if !segment_dir_name.contains("-segment-") {
        return false;
    }
    let file_name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return false,
    };
    file_name.starts_with("frame-") && file_name.ends_with(".png")
}

fn cleanup_frame_artifacts(frames: &[Frame]) -> Vec<(String, std::io::Error)> {
    let mut errors = Vec::new();
    for frame in frames {
        let path = Path::new(&frame.file_path);
        if !is_safe_frame_artifact_path(path) {
            eprintln!(
                "skipping cleanup of frame artifact with unsafe path: {}",
                frame.file_path
            );
            continue;
        }
        if path.exists() {
            if let Err(error) = std::fs::remove_file(path) {
                errors.push((frame.file_path.clone(), error));
            }
        }
    }
    errors
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

fn map_frame_batch(row: SqliteRow) -> Result<FrameBatch> {
    Ok(FrameBatch {
        id: row.get("id"),
        session_id: row.get("session_id"),
        batch_key: row.get("batch_key"),
        batch_started_at: row.get("batch_started_at"),
        batch_ended_at: row.get("batch_ended_at"),
        status: FrameBatchStatus::from_str(row.get("status"))?,
        frame_count: row.get("frame_count"),
        first_frame_at: row.get("first_frame_at"),
        last_frame_at: row.get("last_frame_at"),
        finalize_job_id: row.get("finalize_job_id"),
        finalized_output_path: row.get("finalized_output_path"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        closed_at: row.get("closed_at"),
        completed_at: row.get("completed_at"),
        failed_at: row.get("failed_at"),
        last_error: row.get("last_error"),
    })
}

async fn get_frame_batch_optional<'e, E>(executor: E, batch_id: i64) -> Result<Option<FrameBatch>>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query(
        "SELECT id, session_id, batch_key, batch_started_at, batch_ended_at, status, frame_count, \
            first_frame_at, last_frame_at, finalize_job_id, finalized_output_path, created_at, \
            updated_at, closed_at, completed_at, failed_at, last_error \
         FROM frame_batches WHERE id = ?1",
    )
    .bind(batch_id)
    .fetch_optional(executor)
    .await?;

    row.map(map_frame_batch).transpose()
}

async fn get_background_job_optional<'e, E>(
    executor: E,
    job_id: i64,
) -> Result<Option<BackgroundJob>>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query(
        "SELECT id, kind, status, payload_json, result_json AS result_text, attempt_count, last_error, \
            created_at, updated_at, started_at, finished_at \
         FROM background_jobs WHERE id = ?1",
    )
    .bind(job_id)
    .fetch_optional(executor)
    .await?;

    row.map(|row| {
        Ok(BackgroundJob {
            id: row.get("id"),
            kind: row.get("kind"),
            status: match row.get::<String, _>("status").as_str() {
                "queued" => BackgroundJobStatus::Queued,
                "running" => BackgroundJobStatus::Running,
                "completed" => BackgroundJobStatus::Completed,
                "failed" => BackgroundJobStatus::Failed,
                other => return Err(AppInfraError::InvalidJobStatus(other.to_string())),
            },
            payload_json: row.get("payload_json"),
            result_text: row.get("result_text"),
            attempt_count: row.get("attempt_count"),
            last_error: row.get("last_error"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            started_at: row.get("started_at"),
            finished_at: row.get("finished_at"),
        })
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::Database, processing::NewFrame, ProcessingJobStatus};
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("frame-batches-{label}-{unique}"));
            fs::create_dir_all(&path).expect("test directory should exist");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn run_async_test(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

    #[test]
    fn frame_batch_window_rounds_into_ten_minute_bucket() {
        let window = frame_batch_window("2026-04-12T10:07:30Z").expect("window should build");
        assert_eq!(window.started_at, "2026-04-12T10:00:00Z");
        assert_eq!(window.ended_at, "2026-04-12T10:10:00Z");
    }

    #[test]
    fn closed_batches_get_finalize_jobs_once() {
        run_async_test(async {
            let dir = TestDir::new("close-schedule");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let store = FrameBatchStore::new(database.pool().clone());

            let active = store
                .upsert_open_batch_for_frame("session-a", "2026-04-12T10:11:00Z")
                .await
                .expect("active batch should exist");
            let closed = store
                .upsert_open_batch_for_frame("session-a", "2026-04-12T10:01:00Z")
                .await
                .expect("closed batch should exist");

            let closed_batches = store
                .close_and_schedule_completed_batches_for_frame("session-a", active.id)
                .await
                .expect("closed batches should schedule");
            assert_eq!(closed_batches.len(), 1);
            assert_eq!(closed_batches[0].id, closed.id);

            let stored = store
                .get_required(closed.id)
                .await
                .expect("batch should load");
            assert_eq!(stored.status, FrameBatchStatus::Closed);
            assert!(stored.finalize_job_id.is_some());
        });
    }

    #[test]
    fn finalize_runtime_waits_until_ocr_is_terminal() {
        run_async_test(async {
            let dir = TestDir::new("ocr-pending");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = FrameBatchStore::new(pool.clone());

            let batch = store
                .upsert_open_batch_for_frame("session-b", "2026-04-12T10:01:00Z")
                .await
                .expect("batch should exist");
            let frame = processing
                .insert_frame(&NewFrame::new(
                    "session-b",
                    "/tmp/session-b-segment-0001/frames/frame-1.png",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("frame should persist");
            store
                .attach_frame_to_batch(frame.id, batch.id, &frame.captured_at)
                .await
                .expect("frame should attach");
            let job = processing
                .enqueue_job(&crate::ProcessingJobDraft::for_frame_ocr(frame.id))
                .await
                .expect("ocr job should queue");

            let closed = store
                .close_completed_batches_for_session("session-b", None)
                .await
                .expect("batch should close");
            assert_eq!(closed.len(), 1);
            let finalize_job = store
                .enqueue_finalize_job_if_needed(batch.id)
                .await
                .expect("finalize job should queue")
                .expect("finalize job should exist");

            let runtime = FrameBatchRuntime::new(store.clone());
            let error = runtime
                .process_job(finalize_job.id)
                .await
                .expect_err("finalization should wait for OCR completion");
            assert!(
                matches!(error, AppInfraError::FrameBatchOcrPending { batch_id } if batch_id == batch.id)
            );

            let claimed = processing
                .claim_queued_job(job.id)
                .await
                .expect("ocr job should claim")
                .expect("ocr job should exist");
            assert_eq!(claimed.status, ProcessingJobStatus::Running);
            processing
                .mark_job_failed(job.id, Some("expected failure"))
                .await
                .expect("ocr job should fail terminally");

            let result = runtime
                .process_job(finalize_job.id)
                .await
                .expect("finalization should proceed after terminal OCR");
            assert_eq!(result.batch.status, FrameBatchStatus::Completed);
        });
    }

    #[test]
    fn claim_next_finalize_job_only_returns_ready_batches() {
        run_async_test(async {
            let dir = TestDir::new("ready-finalize-job");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = FrameBatchStore::new(pool.clone());

            let pending_batch = store
                .upsert_open_batch_for_frame("session-ready", "2026-04-12T10:01:00Z")
                .await
                .expect("pending batch should exist");
            let pending_frame = processing
                .insert_frame(&NewFrame::new(
                    "session-ready",
                    "/tmp/session-ready-segment-0001/frames/frame-1.png",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("pending frame should exist");
            store
                .attach_frame_to_batch(
                    pending_frame.id,
                    pending_batch.id,
                    &pending_frame.captured_at,
                )
                .await
                .expect("pending frame should attach");
            processing
                .enqueue_job(&crate::ProcessingJobDraft::for_frame_ocr(pending_frame.id))
                .await
                .expect("pending ocr job should enqueue");
            store
                .close_completed_batches_for_session("session-ready", None)
                .await
                .expect("pending batch should close");
            let _ = store
                .enqueue_finalize_job_if_needed(pending_batch.id)
                .await
                .expect("pending finalize job should enqueue");

            let ready_batch = store
                .upsert_open_batch_for_frame("session-ready", "2026-04-12T10:11:00Z")
                .await
                .expect("ready batch should exist");
            let ready_frame = processing
                .insert_frame(&NewFrame::new(
                    "session-ready",
                    "/tmp/session-ready-segment-0002/frames/frame-2.png",
                    "2026-04-12T10:11:00Z",
                ))
                .await
                .expect("ready frame should exist");
            store
                .attach_frame_to_batch(ready_frame.id, ready_batch.id, &ready_frame.captured_at)
                .await
                .expect("ready frame should attach");
            let ready_ocr = processing
                .enqueue_job(&crate::ProcessingJobDraft::for_frame_ocr(ready_frame.id))
                .await
                .expect("ready ocr job should enqueue");
            let claimed = processing
                .claim_queued_job(ready_ocr.id)
                .await
                .expect("ready ocr should claim")
                .expect("ready ocr should exist");
            assert_eq!(claimed.status, ProcessingJobStatus::Running);
            processing
                .mark_job_failed(ready_ocr.id, Some("ready terminal"))
                .await
                .expect("ready ocr should fail terminally");
            store
                .close_completed_batches_for_session("session-ready", Some(ready_batch.id + 1))
                .await
                .expect("ready batch should close");
            let ready_finalize = store
                .enqueue_finalize_job_if_needed(ready_batch.id)
                .await
                .expect("ready finalize job should enqueue")
                .expect("ready finalize job should exist");

            let claimed_job = store
                .claim_next_finalize_job()
                .await
                .expect("ready finalize job should claim")
                .expect("a finalize job should be ready");
            assert_eq!(claimed_job.id, ready_finalize.id);
        });
    }

    #[test]
    fn finalization_deletes_frame_png_artifacts() {
        run_async_test(async {
            let dir = TestDir::new("artifact-cleanup");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = FrameBatchStore::new(pool.clone());

            let frames_dir = dir.path().join("session-cleanup-segment-0001").join("frames");
            fs::create_dir_all(&frames_dir).expect("frames directory should be created");
            let frame_path = frames_dir.join("frame-1.png");
            fs::write(&frame_path, b"fake png").expect("frame file should be written");
            assert!(frame_path.exists());

            let frame_path_str = frame_path.to_string_lossy().to_string();
            let batch = store
                .upsert_open_batch_for_frame("session-cleanup", "2026-04-12T10:01:00Z")
                .await
                .expect("batch should exist");
            let frame = processing
                .insert_frame(&NewFrame::new(
                    "session-cleanup",
                    &frame_path_str,
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("frame should persist");
            store
                .attach_frame_to_batch(frame.id, batch.id, &frame.captured_at)
                .await
                .expect("frame should attach");

            let closed = store
                .close_completed_batches_for_session("session-cleanup", None)
                .await
                .expect("batch should close");
            assert_eq!(closed.len(), 1);
            let finalize_job = store
                .enqueue_finalize_job_if_needed(batch.id)
                .await
                .expect("finalize job should queue")
                .expect("finalize job should exist");

            let runtime = FrameBatchRuntime::new(store.clone());
            let result = runtime
                .process_job(finalize_job.id)
                .await
                .expect("finalization should succeed");
            assert_eq!(result.batch.status, FrameBatchStatus::Completed);
            assert!(!frame_path.exists(), "PNG frame artifact should be deleted after finalization");
        });
    }

    #[test]
    fn is_safe_frame_artifact_path_accepts_valid_paths() {
        assert!(is_safe_frame_artifact_path(Path::new(
            "/data/session/session-a-segment-0001/frames/frame-1717000123456-000042.png"
        )));
        assert!(is_safe_frame_artifact_path(Path::new(
            "/tmp/my-session-segment-0001/frames/frame-1.png"
        )));
    }

    #[test]
    fn is_safe_frame_artifact_path_rejects_missing_segment_grandparent() {
        // parent is `frames` but grandparent is not a segment directory
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/tmp/frames/frame-1.png"
        )));
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/data/session/frames/frame-1.png"
        )));
    }

    #[test]
    fn is_safe_frame_artifact_path_rejects_relative_paths() {
        assert!(!is_safe_frame_artifact_path(Path::new(
            "frames/frame-1.png"
        )));
    }

    #[test]
    fn is_safe_frame_artifact_path_rejects_path_traversal() {
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/data/session/../../../etc/frames/frame-1.png"
        )));
    }

    #[test]
    fn is_safe_frame_artifact_path_rejects_non_png_extension() {
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/data/session-a-segment-0001/frames/frame-1.txt"
        )));
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/data/session-a-segment-0001/frames/frame-1"
        )));
    }

    #[test]
    fn is_safe_frame_artifact_path_rejects_wrong_parent_dir() {
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/data/session-a-segment-0001/frame-1.png"
        )));
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/etc/passwd"
        )));
    }

    #[test]
    fn is_safe_frame_artifact_path_rejects_wrong_filename_prefix() {
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/data/session-a-segment-0001/frames/screenshot.png"
        )));
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/data/session-a-segment-0001/frames/not-a-frame.png"
        )));
    }

    #[test]
    fn cleanup_skips_unsafe_paths() {
        let dir = TestDir::new("unsafe-cleanup");

        // Create a file that looks like a frame but lives outside /frames/
        let bad_file = dir.path().join("important.txt");
        fs::write(&bad_file, b"important data").expect("file should be written");

        let frames = vec![Frame {
            id: 1,
            session_id: "s".to_string(),
            file_path: bad_file.to_string_lossy().to_string(),
            captured_at: "2026-04-12T10:01:00Z".to_string(),
            width: None,
            height: None,
            content_fingerprint: None,
            created_at: String::new(),
            updated_at: String::new(),
        }];

        let errors = cleanup_frame_artifacts(&frames);
        assert!(errors.is_empty(), "no errors expected for skipped paths");
        assert!(bad_file.exists(), "file with unsafe path must not be deleted");
    }

    #[test]
    fn finalize_columns_used_for_batch_completion() {
        run_async_test(async {
            let dir = TestDir::new("finalize-completion");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let store = FrameBatchStore::new(pool.clone());

            let batch = store
                .upsert_open_batch_for_frame("session-complete", "2026-04-12T10:01:00Z")
                .await
                .expect("batch should persist");
            store
                .close_completed_batches_for_session("session-complete", None)
                .await
                .expect("batch should close");
            store
                .mark_batch_processing(batch.id)
                .await
                .expect("batch should transition to processing");
            let completed = store
                .mark_batch_completed(batch.id, Some("/output/batch.mp4"))
                .await
                .expect("batch should complete");

            assert_eq!(completed.status, FrameBatchStatus::Completed);
            assert_eq!(
                completed.finalized_output_path.as_deref(),
                Some("/output/batch.mp4")
            );
        });
    }

    #[test]
    fn finalized_output_path_is_null_not_empty_string_after_finalization() {
        run_async_test(async {
            let dir = TestDir::new("null-output-path");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = FrameBatchStore::new(pool.clone());

            let batch = store
                .upsert_open_batch_for_frame("session-null-path", "2026-04-12T10:01:00Z")
                .await
                .expect("batch should exist");
            let frame = processing
                .insert_frame(&NewFrame::new(
                    "session-null-path",
                    "/tmp/session-null-path-segment-0001/frames/frame-1.png",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("frame should persist");
            store
                .attach_frame_to_batch(frame.id, batch.id, &frame.captured_at)
                .await
                .expect("frame should attach");

            store
                .close_completed_batches_for_session("session-null-path", None)
                .await
                .expect("batch should close");
            let finalize_job = store
                .enqueue_finalize_job_if_needed(batch.id)
                .await
                .expect("finalize job should queue")
                .expect("finalize job should exist");

            let runtime = FrameBatchRuntime::new(store.clone());
            let result = runtime
                .process_job(finalize_job.id)
                .await
                .expect("finalization should succeed");
            assert_eq!(result.batch.status, FrameBatchStatus::Completed);
            assert!(
                result.batch.finalized_output_path.is_none(),
                "finalized_output_path should be None, not Some(\"\")"
            );

            // Verify at the SQL level that the column is actually NULL, not an empty string.
            let raw_value: Option<String> = sqlx::query_scalar(
                "SELECT finalized_output_path FROM frame_batches WHERE id = ?1",
            )
            .bind(batch.id)
            .fetch_one(&pool)
            .await
            .expect("query should succeed");
            assert!(
                raw_value.is_none(),
                "finalized_output_path should be SQL NULL, got: {:?}",
                raw_value
            );
        });
    }

    #[test]
    fn finalization_cleans_up_artifacts_before_marking_batch_completed() {
        run_async_test(async {
            let dir = TestDir::new("cleanup-ordering");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = FrameBatchStore::new(pool.clone());

            let frames_dir = dir
                .path()
                .join("session-ordering-segment-0001")
                .join("frames");
            fs::create_dir_all(&frames_dir).expect("frames dir should be created");
            let frame_path = frames_dir.join("frame-1.png");
            fs::write(&frame_path, b"fake png").expect("frame file should be written");
            assert!(frame_path.exists());

            let frame_path_str = frame_path.to_string_lossy().to_string();
            let batch = store
                .upsert_open_batch_for_frame("session-ordering", "2026-04-12T10:01:00Z")
                .await
                .expect("batch should exist");
            let frame = processing
                .insert_frame(&NewFrame::new(
                    "session-ordering",
                    &frame_path_str,
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("frame should persist");
            store
                .attach_frame_to_batch(frame.id, batch.id, &frame.captured_at)
                .await
                .expect("frame should attach");

            store
                .close_completed_batches_for_session("session-ordering", None)
                .await
                .expect("batch should close");
            let finalize_job = store
                .enqueue_finalize_job_if_needed(batch.id)
                .await
                .expect("finalize job should queue")
                .expect("finalize job should exist");

            let runtime = FrameBatchRuntime::new(store.clone());
            let result = runtime
                .process_job(finalize_job.id)
                .await
                .expect("finalization should succeed");
            assert_eq!(result.batch.status, FrameBatchStatus::Completed);

            // Artifact should have been cleaned up.
            assert!(
                !frame_path.exists(),
                "frame artifact should be deleted after finalization"
            );

            // The background job should be marked completed.
            let final_job = store
                .jobs
                .get(finalize_job.id)
                .await
                .expect("job should load")
                .expect("job should exist");
            assert_eq!(final_job.status, BackgroundJobStatus::Completed);
        });
    }

    #[test]
    fn mark_batch_completed_with_none_stores_null() {
        run_async_test(async {
            let dir = TestDir::new("completed-null");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let store = FrameBatchStore::new(pool.clone());

            let batch = store
                .upsert_open_batch_for_frame("session-null", "2026-04-12T10:01:00Z")
                .await
                .expect("batch should exist");
            store
                .close_completed_batches_for_session("session-null", None)
                .await
                .expect("batch should close");
            store
                .mark_batch_processing(batch.id)
                .await
                .expect("batch should transition to processing");
            let completed = store
                .mark_batch_completed(batch.id, None)
                .await
                .expect("batch should complete");

            assert_eq!(completed.status, FrameBatchStatus::Completed);
            assert!(
                completed.finalized_output_path.is_none(),
                "finalized_output_path should be None when passed None"
            );

            let raw: Option<String> = sqlx::query_scalar(
                "SELECT finalized_output_path FROM frame_batches WHERE id = ?1",
            )
            .bind(batch.id)
            .fetch_one(&pool)
            .await
            .expect("query should succeed");
            assert!(raw.is_none(), "SQL column should be NULL, got: {:?}", raw);
        });
    }
}
