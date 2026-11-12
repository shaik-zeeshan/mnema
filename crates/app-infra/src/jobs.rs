use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, Row};

use crate::db::CaptureDb;
use crate::error::{AppInfraError, Result};

pub(crate) const ORPHANED_RUNNING_JOB_ERROR: &str =
    "job was marked failed during startup recovery after the app shut down while it was running";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundJobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

impl BackgroundJobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            other => Err(AppInfraError::InvalidJobStatus(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobDescriptor {
    pub kind: String,
}

impl JobDescriptor {
    pub fn new(kind: impl Into<String>) -> Self {
        Self { kind: kind.into() }
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundJob {
    pub id: i64,
    pub kind: String,
    pub status: BackgroundJobStatus,
    pub payload_json: Option<String>,
    pub result_text: Option<String>,
    pub attempt_count: i64,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobCounts {
    pub total: i64,
    pub queued: i64,
    pub running: i64,
    pub completed: i64,
    pub failed: i64,
}

#[derive(Clone)]
pub struct JobStore {
    db: CaptureDb,
}

impl JobStore {
    pub(crate) fn new(db: CaptureDb) -> Self {
        Self { db }
    }

    pub async fn enqueue(
        &self,
        descriptor: &JobDescriptor,
        payload_json: Option<&str>,
    ) -> Result<BackgroundJob> {
        let result = sqlx::query(
            "INSERT INTO background_jobs (kind, status, payload_json) VALUES (?1, ?2, ?3)",
        )
        .bind(descriptor.kind())
        .bind(BackgroundJobStatus::Queued.as_str())
        .bind(payload_json)
        .execute(self.db.write())
        .await?;

        self.get_required(result.last_insert_rowid()).await
    }

    pub async fn get(&self, job_id: i64) -> Result<Option<BackgroundJob>> {
        let row = sqlx::query(
            "SELECT \
                id, kind, status, payload_json, result_json AS result_text, attempt_count, last_error, \
                created_at, updated_at, started_at, finished_at \
             FROM background_jobs \
             WHERE id = ?1",
        )
        .bind(job_id)
        .fetch_optional(self.db.read())
        .await?;

        row.map(map_background_job).transpose()
    }

    pub async fn list(&self, limit: Option<u32>) -> Result<Vec<BackgroundJob>> {
        if matches!(limit, Some(0)) {
            return Ok(Vec::new());
        }

        let rows = match limit {
            Some(limit) => {
                sqlx::query(
                    "SELECT \
                        id, kind, status, payload_json, result_json AS result_text, attempt_count, last_error, \
                        created_at, updated_at, started_at, finished_at \
                     FROM background_jobs \
                     ORDER BY id DESC \
                     LIMIT ?1",
                )
                .bind(limit as i64)
                .fetch_all(self.db.read())
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT \
                        id, kind, status, payload_json, result_json AS result_text, attempt_count, last_error, \
                        created_at, updated_at, started_at, finished_at \
                     FROM background_jobs \
                     ORDER BY id DESC",
                )
                .fetch_all(self.db.read())
                .await?
            }
        };

        rows.into_iter().map(map_background_job).collect()
    }

    pub async fn mark_running(&self, job_id: i64) -> Result<BackgroundJob> {
        sqlx::query(
            "UPDATE background_jobs \
             SET status = 'running', \
                 attempt_count = attempt_count + 1, \
                 last_error = NULL, \
                 result_json = NULL, \
                 started_at = CURRENT_TIMESTAMP, \
                 finished_at = NULL, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1",
        )
        .bind(job_id)
        .execute(self.db.write())
        .await?;

        self.get_required(job_id).await
    }

    pub async fn mark_completed(
        &self,
        job_id: i64,
        result_text: Option<&str>,
    ) -> Result<BackgroundJob> {
        sqlx::query(
            "UPDATE background_jobs \
             SET status = 'completed', \
                 result_json = ?2, \
                 last_error = NULL, \
                 finished_at = CURRENT_TIMESTAMP, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1",
        )
        .bind(job_id)
        .bind(result_text)
        .execute(self.db.write())
        .await?;

        self.get_required(job_id).await
    }

    pub async fn mark_failed(
        &self,
        job_id: i64,
        error_text: Option<&str>,
    ) -> Result<BackgroundJob> {
        sqlx::query(
            "UPDATE background_jobs \
             SET status = 'failed', \
                 result_json = NULL, \
                 last_error = ?2, \
                 finished_at = CURRENT_TIMESTAMP, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?1",
        )
        .bind(job_id)
        .bind(error_text)
        .execute(self.db.write())
        .await?;

        self.get_required(job_id).await
    }

    pub async fn reconcile_orphaned_running_jobs(&self) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE background_jobs \
             SET status = 'failed', \
                 result_json = NULL, \
                 last_error = ?1, \
                 finished_at = CURRENT_TIMESTAMP, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE status = 'running'",
        )
        .bind(ORPHANED_RUNNING_JOB_ERROR)
        .execute(self.db.write())
        .await?;

        Ok(result.rows_affected())
    }

    pub async fn counts(&self) -> Result<JobCounts> {
        let row = sqlx::query(
            "SELECT \
                COUNT(*) AS total, \
                COALESCE(SUM(CASE WHEN status = 'queued' THEN 1 ELSE 0 END), 0) AS queued, \
                COALESCE(SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END), 0) AS running, \
                COALESCE(SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END), 0) AS completed, \
                COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0) AS failed \
            FROM background_jobs",
        )
        .fetch_one(self.db.read())
        .await?;

        Ok(JobCounts {
            total: row.get("total"),
            queued: row.get("queued"),
            running: row.get("running"),
            completed: row.get("completed"),
            failed: row.get("failed"),
        })
    }

    async fn get_required(&self, job_id: i64) -> Result<BackgroundJob> {
        self.get(job_id)
            .await?
            .ok_or(AppInfraError::JobNotFound(job_id))
    }
}

fn map_background_job(row: SqliteRow) -> Result<BackgroundJob> {
    Ok(BackgroundJob {
        id: row.get("id"),
        kind: row.get("kind"),
        status: BackgroundJobStatus::from_str(row.get("status"))?,
        payload_json: row.get("payload_json"),
        result_text: row.get("result_text"),
        attempt_count: row.get("attempt_count"),
        last_error: row.get("last_error"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
    })
}

pub fn default_worker_thread_count() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1)
        .max(1)
}
