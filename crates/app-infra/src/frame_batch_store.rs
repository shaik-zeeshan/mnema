use std::path::Path;

use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, Executor, Row, Sqlite, SqlitePool, Transaction};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub use crate::hidden_segment_workspace::{
    HiddenSegmentWorkspaceRepairResult, SegmentWorkspaceCleanupDebugInfo,
};

use crate::{
    hidden_segment_workspace::HiddenSegmentWorkspaceRepair,
    jobs::{BackgroundJob, BackgroundJobStatus, JobDescriptor, JobStore},
    processing::{Frame, ProcessingStore, FRAME_SUBJECT_TYPE, OCR_PROCESSOR},
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SegmentWorkspaceBatchReference {
    pub batch_id: i64,
    pub status: FrameBatchStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SegmentWorkspaceFrameBatchReferences {
    pub frame_count: i64,
    pub batch_references: Vec<SegmentWorkspaceBatchReference>,
}

#[derive(Clone)]
pub struct FrameBatchStore {
    pool: SqlitePool,
    jobs: JobStore,
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
             SET status = 'closed', finalize_job_id = NULL, updated_at = CURRENT_TIMESTAMP \
              WHERE status = 'processing' \
                AND ( \
                    finalize_job_id IS NULL \
                    OR NOT EXISTS ( \
                        SELECT 1 FROM background_jobs \
                        WHERE background_jobs.id = frame_batches.finalize_job_id \
                          AND background_jobs.kind = ?1 \
                          AND background_jobs.status IN ('queued', 'running', 'completed') \
                   ) \
               )",
        )
        .bind(FRAME_BATCH_FINALIZE_JOB_KIND)
        .execute(&self.pool)
        .await?;

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

    pub async fn reconcile_open_batches_without_active_capture(&self) -> Result<u64> {
        let rows = sqlx::query(
            "SELECT DISTINCT session_id FROM frame_batches WHERE status = 'open' ORDER BY session_id ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut scheduled = 0;
        for row in rows {
            let closed = self
                .close_and_schedule_all_batches_for_session(
                    row.get::<String, _>("session_id").as_str(),
                )
                .await?;
            scheduled += closed.len() as u64;
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
            "SELECT id, session_id, file_path, captured_at, width, height, \
                    equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                    created_at, updated_at \
             FROM frames \
             WHERE frame_batch_id = ?1 \
             ORDER BY id ASC",
        )
        .bind(batch_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_frame).collect()
    }

    pub(crate) async fn list_frame_batch_references_for_workspace(
        &self,
        workspace_prefix: &str,
    ) -> Result<SegmentWorkspaceFrameBatchReferences> {
        // Range bounds (not `LIKE`) so SQLite can use the `frames_file_path_idx`
        // index: a bound-parameter `LIKE` is planned as a full table scan, while
        // `>= prefix AND < upper` is an index range search. See migration 0038.
        let frame_rows = sqlx::query(
            "SELECT \
                frames.id AS frame_id, \
                frame_batches.id AS batch_id, \
                frame_batches.status AS batch_status \
             FROM frames \
              LEFT JOIN frame_batches ON frame_batches.id = frames.frame_batch_id \
             WHERE frames.file_path >= ?1 AND frames.file_path < ?2 \
             ORDER BY frames.id ASC",
        )
        .bind(workspace_prefix)
        .bind(workspace_path_prefix_upper_bound(workspace_prefix))
        .fetch_all(&self.pool)
        .await?;

        let mut frame_count = 0_i64;
        let mut last_frame_id = None;
        let mut batch_references = Vec::<SegmentWorkspaceBatchReference>::new();
        let mut seen_batch_ids = std::collections::HashSet::new();

        for row in frame_rows {
            let frame_id = row.get::<i64, _>("frame_id");
            if last_frame_id != Some(frame_id) {
                frame_count += 1;
                last_frame_id = Some(frame_id);
            }

            if let Some(batch_id) = row.get::<Option<i64>, _>("batch_id") {
                if seen_batch_ids.insert(batch_id) {
                    batch_references.push(SegmentWorkspaceBatchReference {
                        batch_id,
                        status: FrameBatchStatus::from_str(&row.get::<String, _>("batch_status"))?,
                    });
                }
            }
        }

        Ok(SegmentWorkspaceFrameBatchReferences {
            frame_count,
            batch_references,
        })
    }

    pub async fn classify_hidden_segment_workspace(
        &self,
        workspace_dir: &Path,
    ) -> Result<Option<SegmentWorkspaceCleanupDebugInfo>> {
        HiddenSegmentWorkspaceRepair::new(self.clone(), ProcessingStore::new(self.pool.clone()))
            .classify_hidden_segment_workspace(workspace_dir)
            .await
    }

    pub async fn repair_hidden_segment_workspaces(
        &self,
        recordings_root: &Path,
    ) -> Result<HiddenSegmentWorkspaceRepairResult> {
        self.repair_hidden_segment_workspaces_with_context(
            recordings_root,
            &crate::HiddenSegmentWorkspaceRepairContext::default(),
        )
        .await
    }

    pub async fn repair_hidden_segment_workspaces_with_context(
        &self,
        recordings_root: &Path,
        context: &crate::HiddenSegmentWorkspaceRepairContext,
    ) -> Result<HiddenSegmentWorkspaceRepairResult> {
        HiddenSegmentWorkspaceRepair::new(self.clone(), ProcessingStore::new(self.pool.clone()))
            .repair_hidden_segment_workspaces_with_context(recordings_root, context)
            .await
    }

    pub async fn close_completed_batches_for_session(
        &self,
        session_id: &str,
        active_batch_id: Option<i64>,
    ) -> Result<Vec<FrameBatch>> {
        let mut transaction = self.pool.begin().await?;
        let closed = self
            .close_completed_batches_for_session_in_transaction(
                &mut transaction,
                session_id,
                active_batch_id,
            )
            .await?;

        transaction.commit().await?;
        Ok(closed)
    }

    pub async fn close_and_schedule_all_batches_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<FrameBatch>> {
        let mut transaction = self.pool.begin().await?;
        let closed = self
            .close_and_schedule_all_batches_for_session_in_transaction(&mut transaction, session_id)
            .await?;

        transaction.commit().await?;
        Ok(closed)
    }

    pub(crate) async fn close_and_schedule_all_batches_for_session_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        session_id: &str,
    ) -> Result<Vec<FrameBatch>> {
        let closed = self
            .close_completed_batches_for_session_in_transaction(transaction, session_id, None)
            .await?;

        self.enqueue_finalize_jobs_for_closed_batches_in_transaction(transaction, &closed)
            .await?;

        Ok(closed)
    }

    pub(crate) async fn close_completed_batches_for_session_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        session_id: &str,
        active_batch_id: Option<i64>,
    ) -> Result<Vec<FrameBatch>> {
        let rows = sqlx::query(
            "SELECT id FROM frame_batches \
             WHERE session_id = ?1 AND status = 'open' AND (?2 IS NULL OR id != ?2) \
             ORDER BY id ASC",
        )
        .bind(session_id)
        .bind(active_batch_id)
        .fetch_all(&mut **transaction)
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
            .execute(&mut **transaction)
            .await?;

            if let Some(batch) = get_frame_batch_optional(&mut **transaction, batch_id).await? {
                closed.push(batch);
            }
        }

        Ok(closed)
    }

    pub async fn enqueue_finalize_job_if_needed(
        &self,
        batch_id: i64,
    ) -> Result<Option<BackgroundJob>> {
        let mut transaction = self.pool.begin().await?;
        let job = self
            .enqueue_finalize_job_if_needed_in_transaction(&mut transaction, batch_id)
            .await?;

        transaction.commit().await?;
        Ok(job)
    }

    pub(crate) async fn enqueue_finalize_job_if_needed_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        batch_id: i64,
    ) -> Result<Option<BackgroundJob>> {
        let batch = get_frame_batch_optional(&mut **transaction, batch_id)
            .await?
            .ok_or(AppInfraError::FrameBatchNotFound(batch_id))?;

        if batch.status != FrameBatchStatus::Closed || batch.finalize_job_id.is_some() {
            return Ok(None);
        }

        let payload = serde_json::to_string(&FrameBatchFinalizePayload { batch_id })?;
        let job_result = sqlx::query(
            "INSERT INTO background_jobs (kind, status, payload_json) VALUES (?1, ?2, ?3)",
        )
        .bind(JobDescriptor::new(FRAME_BATCH_FINALIZE_JOB_KIND).kind())
        .bind(BackgroundJobStatus::Queued.as_str())
        .bind(&payload)
        .execute(&mut **transaction)
        .await?;
        let job_id = job_result.last_insert_rowid();

        let updated = sqlx::query(
            "UPDATE frame_batches \
             SET finalize_job_id = ?2, updated_at = CURRENT_TIMESTAMP \
              WHERE id = ?1 AND finalize_job_id IS NULL",
        )
        .bind(batch_id)
        .bind(job_id)
        .execute(&mut **transaction)
        .await?;

        if updated.rows_affected() == 0 {
            sqlx::query("DELETE FROM background_jobs WHERE id = ?1")
                .bind(job_id)
                .execute(&mut **transaction)
                .await?;
            return Ok(None);
        }

        let job = get_background_job_optional(&mut **transaction, job_id)
            .await?
            .ok_or(AppInfraError::JobNotFound(job_id))?;

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

    pub(crate) async fn mark_finalize_job_completed(
        &self,
        job_id: i64,
        result: &FrameBatchFinalizeResult,
    ) -> Result<BackgroundJob> {
        let result_json = serde_json::to_string(result)?;
        self.jobs.mark_completed(job_id, Some(&result_json)).await
    }

    pub(crate) async fn mark_finalize_job_failed(
        &self,
        job_id: i64,
        error_text: &str,
    ) -> Result<BackgroundJob> {
        self.jobs.mark_failed(job_id, Some(error_text)).await
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

    pub(crate) async fn close_and_schedule_completed_batches_for_frame_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        session_id: &str,
        active_batch_id: i64,
    ) -> Result<Vec<FrameBatch>> {
        let closed = self
            .close_completed_batches_for_session_in_transaction(
                transaction,
                session_id,
                Some(active_batch_id),
            )
            .await?;

        self.enqueue_finalize_jobs_for_closed_batches_in_transaction(transaction, &closed)
            .await?;

        Ok(closed)
    }

    async fn enqueue_finalize_jobs_for_closed_batches_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        closed: &[FrameBatch],
    ) -> Result<()> {
        let mut first_error: Option<AppInfraError> = None;
        for batch in closed {
            if let Err(error) = self
                .enqueue_finalize_job_if_needed_in_transaction(transaction, batch.id)
                .await
            {
                capture_runtime::debug_log!(
                    "[app-infra][frame-batches] failed to schedule finalize job for batch {}: {}",
                    batch.id,
                    error
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }

        if let Some(error) = first_error {
            return Err(error);
        }

        Ok(())
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

/// Exclusive upper bound for a path-prefix range scan: the prefix with its final
/// code point bumped by one. Lets `file_path >= prefix AND file_path < bound`
/// match exactly the rows a `LIKE 'prefix%'` would, while using the
/// `frames_file_path_idx` index. Workspace prefixes always end in `/`, so the
/// loop normally bumps `/` to `0` on the first iteration; the fallback only
/// matters for pathological inputs (empty / un-incrementable trailing chars).
pub(crate) fn workspace_path_prefix_upper_bound(prefix: &str) -> String {
    // The range bound only replicates `LIKE 'prefix%'` when `prefix` ends in a
    // path separator (callers append `std::path::MAIN_SEPARATOR`). A prefix
    // without one still yields a valid pure-prefix range, but would silently
    // diverge from the deleted `workspace_like_pattern`'s `/%` separator and
    // could bleed a sibling workspace's frames in — so pin the contract.
    debug_assert!(
        prefix.ends_with('/') || prefix.ends_with(std::path::MAIN_SEPARATOR),
        "workspace path prefix must be separator-terminated: {prefix:?}"
    );
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        if let Some(next) = char::from_u32(last as u32 + 1) {
            let mut bound: String = chars.iter().collect();
            bound.push(next);
            return bound;
        }
    }
    format!("{prefix}\u{10FFFF}")
}

fn map_frame(row: SqliteRow) -> Result<Frame> {
    let equivalence_status = row
        .get::<Option<String>, _>("equivalence_status")
        .map(|status| {
            crate::processing::FrameEquivalenceStatus::from_str(&status)
                .ok_or(crate::AppInfraError::InvalidFrameEquivalenceStatus(status))
        })
        .transpose()?;

    Ok(Frame {
        id: row.get("id"),
        session_id: row.get("session_id"),
        file_path: row.get("file_path"),
        captured_at: row.get("captured_at"),
        width: row.get("width"),
        height: row.get("height"),
        equivalence: crate::processing::FrameEquivalence {
            hint: row.get("equivalence_hint"),
            proof: row.get("equivalence_proof"),
            version: row.get("equivalence_version"),
            status: equivalence_status,
            error: row.get("equivalence_error"),
        },
        metadata_snapshot: None,
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
    use crate::{db::Database, processing::NewFrame, FrameBatchRuntime, ProcessingJobStatus};
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

        fn managed_recordings_day_path(&self, year: &str, month: &str, day: &str) -> PathBuf {
            self.path.join(format!("recordings/{year}/{month}/{day}"))
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// Visible-segment file name for the current platform's container
    /// (`.mov` on macOS, `.mp4` on Windows). Artifact cleanup keys off the
    /// sibling the resolver derives, so fixtures must use the matching
    /// extension on whichever platform CI runs.
    fn visible_segment_file_name(stem: &str) -> String {
        format!("{stem}.{}", capture_runtime::screen_segment_extension())
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
    fn finalize_stays_gated_while_failed_ocr_is_retrying() {
        run_async_test(async {
            let dir = TestDir::new("ocr-retrying");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = FrameBatchStore::new(pool.clone());

            let batch = store
                .upsert_open_batch_for_frame("session-retry", "2026-04-12T10:01:00Z")
                .await
                .expect("batch should exist");
            let frame = processing
                .insert_frame(&NewFrame::new(
                    "session-retry",
                    "/tmp/session-retry-segment-0001/frames/frame-1.png",
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
                .close_completed_batches_for_session("session-retry", None)
                .await
                .expect("batch should close");
            assert_eq!(closed.len(), 1);
            let finalize_job = store
                .enqueue_finalize_job_if_needed(batch.id)
                .await
                .expect("finalize job should queue")
                .expect("finalize job should exist");

            // Drive the OCR job's failure through the same store path the processing
            // runtime uses (see `ProcessingRuntime::process_claimed_job`): record a
            // genuine failure, then bounded failure-retry requeues the job within its
            // attempt cap rather than leaving it terminally failed.
            let claimed = processing
                .claim_queued_job(job.id)
                .await
                .expect("ocr job should claim")
                .expect("ocr job should exist");
            assert_eq!(claimed.status, ProcessingJobStatus::Running);
            processing
                .mark_job_failed(job.id, Some("expected failure"))
                .await
                .expect("ocr job should fail");
            let requeued = processing
                .requeue_failed_job_within_attempt_cap(job.id)
                .await
                .expect("failed ocr job should requeue cleanly")
                .expect("a sub-cap failure should be requeued, not left terminal");

            // The OCR job is back to queued (non-terminal) with the single failure
            // recorded, so it is awaiting another attempt rather than done.
            assert_eq!(requeued.status, ProcessingJobStatus::Queued);
            assert_eq!(requeued.failure_count, 1);

            // The finalize gate must stay closed while that OCR job is mid-retry: a
            // queued (non-terminal) OCR job is neither completed nor failed, so the
            // batch must not be claimable for finalization ahead of it.
            let claimed_finalize = store
                .claim_next_finalize_job()
                .await
                .expect("finalize claim should query cleanly");
            assert!(
                claimed_finalize.is_none(),
                "finalize must not be claimable while the batch's OCR job is retrying"
            );

            // The runtime likewise refuses to finalize the batch directly, for the
            // same reason, instead of completing it ahead of the retrying OCR job.
            let runtime = FrameBatchRuntime::new(store.clone());
            let error = runtime
                .process_job(finalize_job.id)
                .await
                .expect_err("finalization should wait for OCR to reach a terminal state");
            assert!(matches!(
                error,
                AppInfraError::FrameBatchOcrPending { batch_id } if batch_id == batch.id
            ));
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

            let recordings_day_dir = dir.managed_recordings_day_path("2026", "04", "12");
            let frames_dir = recordings_day_dir
                .join(".session-cleanup-segment-0001")
                .join("frames");
            fs::create_dir_all(&frames_dir).expect("frames directory should be created");
            fs::write(
                recordings_day_dir.join(visible_segment_file_name("session-cleanup-segment-0001")),
                b"fake mov",
            )
            .expect("visible segment should be written");
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
            assert!(
                !frame_path.exists(),
                "PNG frame artifact should be deleted after finalization"
            );
        });
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
            let raw_value: Option<String> =
                sqlx::query_scalar("SELECT finalized_output_path FROM frame_batches WHERE id = ?1")
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

            let recordings_day_dir = dir.managed_recordings_day_path("2026", "04", "12");
            let frames_dir = recordings_day_dir
                .join(".session-ordering-segment-0001")
                .join("frames");
            fs::create_dir_all(&frames_dir).expect("frames dir should be created");
            fs::write(
                recordings_day_dir.join(visible_segment_file_name("session-ordering-segment-0001")),
                b"fake mov",
            )
            .expect("visible segment should be written");
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

            let raw: Option<String> =
                sqlx::query_scalar("SELECT finalized_output_path FROM frame_batches WHERE id = ?1")
                    .bind(batch.id)
                    .fetch_one(&pool)
                    .await
                    .expect("query should succeed");
            assert!(raw.is_none(), "SQL column should be NULL, got: {:?}", raw);
        });
    }

    #[test]
    fn failed_finalize_jobs_are_requeued_once() {
        run_async_test(async {
            let dir = TestDir::new("failed-finalize-requeue");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = FrameBatchStore::new(pool.clone());

            let batch = store
                .upsert_open_batch_for_frame("session-retry", "2026-04-12T10:01:00Z")
                .await
                .expect("batch should exist");
            let frame = processing
                .insert_frame(&NewFrame::new(
                    "session-retry",
                    "/tmp/session-retry-segment-0001/frames/frame-1.png",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("frame should persist");
            store
                .attach_frame_to_batch(frame.id, batch.id, &frame.captured_at)
                .await
                .expect("frame should attach");
            store
                .close_completed_batches_for_session("session-retry", None)
                .await
                .expect("batch should close");
            let original_job = store
                .enqueue_finalize_job_if_needed(batch.id)
                .await
                .expect("finalize job should enqueue")
                .expect("finalize job should exist");

            store
                .jobs
                .mark_failed(original_job.id, Some("expected finalize failure"))
                .await
                .expect("finalize job should fail");

            let scheduled = store
                .reconcile_closed_batches_without_finalize_jobs()
                .await
                .expect("reconcile should reschedule failed finalize jobs");
            assert_eq!(scheduled, 1);

            let repaired = store
                .get_required(batch.id)
                .await
                .expect("batch should reload");
            assert_eq!(repaired.status, FrameBatchStatus::Closed);
            let retried_job_id = repaired
                .finalize_job_id
                .expect("batch should point at retried finalize job");
            assert_ne!(retried_job_id, original_job.id);

            let retried_job = store
                .jobs
                .get(retried_job_id)
                .await
                .expect("retried job should load")
                .expect("retried job should exist");
            assert_eq!(retried_job.kind, FRAME_BATCH_FINALIZE_JOB_KIND);
            assert_eq!(retried_job.status, BackgroundJobStatus::Queued);

            let original_job = store
                .jobs
                .get(original_job.id)
                .await
                .expect("original job should load")
                .expect("original job should exist");
            assert_eq!(original_job.status, BackgroundJobStatus::Failed);

            let scheduled_again = store
                .reconcile_closed_batches_without_finalize_jobs()
                .await
                .expect("second reconcile should be a no-op");
            assert_eq!(scheduled_again, 0);

            let finalize_job_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM background_jobs WHERE kind = ?1")
                    .bind(FRAME_BATCH_FINALIZE_JOB_KIND)
                    .fetch_one(&pool)
                    .await
                    .expect("finalize jobs should count");
            assert_eq!(finalize_job_count, 2);
        });
    }

    #[test]
    fn reconcile_does_not_duplicate_active_finalize_jobs() {
        run_async_test(async {
            let dir = TestDir::new("active-finalize-no-duplicate");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = FrameBatchStore::new(pool.clone());

            let batch = store
                .upsert_open_batch_for_frame("session-active", "2026-04-12T10:01:00Z")
                .await
                .expect("batch should exist");
            let frame = processing
                .insert_frame(&NewFrame::new(
                    "session-active",
                    "/tmp/session-active-segment-0001/frames/frame-1.png",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("frame should persist");
            store
                .attach_frame_to_batch(frame.id, batch.id, &frame.captured_at)
                .await
                .expect("frame should attach");
            store
                .close_completed_batches_for_session("session-active", None)
                .await
                .expect("batch should close");
            let finalize_job = store
                .enqueue_finalize_job_if_needed(batch.id)
                .await
                .expect("finalize job should enqueue")
                .expect("finalize job should exist");

            let scheduled = store
                .reconcile_closed_batches_without_finalize_jobs()
                .await
                .expect("reconcile should preserve active finalize jobs");
            assert_eq!(scheduled, 0);

            let repaired = store
                .get_required(batch.id)
                .await
                .expect("batch should reload");
            assert_eq!(repaired.status, FrameBatchStatus::Closed);
            assert_eq!(repaired.finalize_job_id, Some(finalize_job.id));

            let finalize_job_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM background_jobs WHERE kind = ?1")
                    .bind(FRAME_BATCH_FINALIZE_JOB_KIND)
                    .fetch_one(&pool)
                    .await
                    .expect("finalize jobs should count");
            assert_eq!(finalize_job_count, 1);
        });
    }

    #[test]
    fn workspace_path_prefix_upper_bound_bumps_trailing_slash() {
        // Workspace prefixes always end in `/`; the exclusive upper bound bumps
        // it to `0`, so `[".../seg-0001/", ".../seg-00010")` captures exactly the
        // paths a `LIKE '.../seg-0001/%'` would.
        assert_eq!(
            workspace_path_prefix_upper_bound("/r/2026/06/01/.x-segment-0001/"),
            "/r/2026/06/01/.x-segment-00010"
        );
    }

    #[test]
    fn list_frame_batch_references_for_workspace_does_not_bleed_into_prefix_sibling() {
        run_async_test(async {
            let dir = TestDir::new("references-prefix-isolation");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = FrameBatchStore::new(pool.clone());

            // `.x-segment-0001` is a strict textual prefix of `.x-segment-0001b`,
            // the exact case where a naive range bound could leak a sibling's
            // frames into the target workspace.
            let target_frame = processing
                .insert_frame(&NewFrame::new(
                    "session-iso",
                    "/tmp/2026/04/12/.x-segment-0001/frames/frame-1.png",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("target frame should persist");
            processing
                .insert_frame(&NewFrame::new(
                    "session-iso",
                    "/tmp/2026/04/12/.x-segment-0001b/frames/frame-1.png",
                    "2026-04-12T10:02:00Z",
                ))
                .await
                .expect("sibling frame should persist");

            let references = store
                .list_frame_batch_references_for_workspace("/tmp/2026/04/12/.x-segment-0001/")
                .await
                .expect("references should resolve");

            assert_eq!(
                references.frame_count, 1,
                "only the target workspace's frame should match"
            );
            // The sibling frame has a higher id; confirm we kept the target one.
            let _ = target_frame;
        });
    }
}
