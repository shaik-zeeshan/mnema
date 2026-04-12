use std::{
    any::Any,
    panic::{self, AssertUnwindSafe},
    sync::Arc,
};

use rayon::ThreadPool;
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, Row, SqlitePool};
use tokio::sync::oneshot;

use crate::error::{AppInfraError, Result};

pub(crate) const ORPHANED_RUNNING_JOB_ERROR: &str =
    "job was marked failed during startup recovery after the app shut down while it was running";
pub(crate) const DEBUG_CPU_JOB_KIND: &str = "debug_cpu_analysis";

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CpuJobSuccess<T> {
    pub value: T,
    pub result_text: Option<String>,
}

impl<T> CpuJobSuccess<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            result_text: None,
        }
    }

    pub fn with_result_text(mut self, result_text: impl Into<String>) -> Self {
        self.result_text = Some(result_text.into());
        self
    }
}

pub type CpuJobResult<T> = std::result::Result<CpuJobSuccess<T>, String>;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobCounts {
    pub total: i64,
    pub queued: i64,
    pub running: i64,
    pub completed: i64,
    pub failed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DebugCpuJobRequest {
    pub document_name: String,
    pub source_text: String,
}

impl DebugCpuJobRequest {
    pub(crate) fn normalized(self) -> Self {
        let document_name = self.document_name.trim();
        let source_text = self.source_text.trim();

        Self {
            document_name: if document_name.is_empty() {
                "debug-document".to_string()
            } else {
                document_name.to_string()
            },
            source_text: if source_text.is_empty() {
                "Simulated OCR input for the debug CPU job.".to_string()
            } else {
                source_text.to_string()
            },
        }
    }

    pub(crate) fn simulated_result_text(&self) -> String {
        let token_list = self
            .source_text
            .split_whitespace()
            .take(6)
            .collect::<Vec<_>>();
        let token_count = self.source_text.split_whitespace().count();
        let character_count = self.source_text.chars().count();
        let excerpt = if token_list.is_empty() {
            "no tokens detected".to_string()
        } else {
            token_list.join(" ")
        };
        let sample = self.source_text.as_bytes();
        let mut checksum = 0_u64;

        for round in 0..2_048_u64 {
            for (index, byte) in sample.iter().take(256).enumerate() {
                let weight = (index as u64 + 1) * (round + 1);
                checksum = checksum.wrapping_add((*byte as u64) * weight);
                checksum = checksum.rotate_left(((index + (round as usize % 13)) % 31 + 1) as u32);
            }
        }

        format!(
            "Simulated OCR analysis complete for {}: {} tokens, {} chars, checksum {:016x}, excerpt \"{}\"",
            self.document_name, token_count, character_count, checksum, excerpt
        )
    }
}

#[derive(Clone)]
pub struct JobStore {
    pool: SqlitePool,
}

impl JobStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
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
        .execute(&self.pool)
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
        .fetch_optional(&self.pool)
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
                .fetch_all(&self.pool)
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
                .fetch_all(&self.pool)
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
        .execute(&self.pool)
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
        .execute(&self.pool)
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
        .execute(&self.pool)
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
        .execute(&self.pool)
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
        .fetch_one(&self.pool)
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

    fn mark_failed_without_runtime(&self, job_id: i64, error_text: Option<&str>) -> Result<()> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| AppInfraError::AsyncRuntimeUnavailable)?
            .block_on(async {
                self.mark_failed(job_id, error_text).await?;
                Ok(())
            })
    }
}

#[derive(Clone)]
pub struct JobRuntime {
    pool: Arc<ThreadPool>,
    worker_thread_count: usize,
}

impl JobRuntime {
    pub fn new(worker_thread_count: usize) -> Result<Self> {
        let worker_thread_count = worker_thread_count.max(1);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker_thread_count)
            .thread_name(|index| format!("app-infra-cpu-{index}"))
            .build()?;

        Ok(Self {
            pool: Arc::new(pool),
            worker_thread_count,
        })
    }

    pub fn worker_thread_count(&self) -> usize {
        self.worker_thread_count
    }

    pub fn spawn_cpu<F, T>(
        &self,
        jobs: JobStore,
        job: BackgroundJob,
        task: F,
    ) -> Result<CpuJobHandle<T>>
    where
        F: FnOnce() -> CpuJobResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let runtime = match tokio::runtime::Handle::try_current() {
            Ok(runtime) => runtime,
            Err(_) => {
                let error = AppInfraError::AsyncRuntimeUnavailable;
                let error_text = error.to_string();
                jobs.mark_failed_without_runtime(job.id, Some(&error_text))?;
                return Err(error);
            }
        };
        let (sender, receiver) = oneshot::channel();
        let descriptor = JobDescriptor::new(job.kind.clone());
        let job_id = job.id;
        let pool = Arc::clone(&self.pool);

        runtime.spawn(async move {
            let output = run_cpu_job(pool, jobs, job_id, task).await;
            let _ = sender.send(output);
        });

        Ok(CpuJobHandle {
            job_id,
            descriptor,
            receiver,
        })
    }
}

pub struct CpuJobHandle<T> {
    job_id: i64,
    descriptor: JobDescriptor,
    receiver: oneshot::Receiver<Result<CpuJobResult<T>>>,
}

impl<T> CpuJobHandle<T> {
    pub fn job_id(&self) -> i64 {
        self.job_id
    }

    pub fn descriptor(&self) -> &JobDescriptor {
        &self.descriptor
    }

    pub async fn join(self) -> Result<CpuJobResult<T>> {
        self.receiver
            .await
            .map_err(|_| AppInfraError::WorkerTaskCancelled)?
    }
}

async fn run_cpu_job<F, T>(
    pool: Arc<ThreadPool>,
    jobs: JobStore,
    job_id: i64,
    task: F,
) -> Result<CpuJobResult<T>>
where
    F: FnOnce() -> CpuJobResult<T> + Send + 'static,
    T: Send + 'static,
{
    jobs.mark_running(job_id).await?;

    let (sender, receiver) = oneshot::channel();

    pool.spawn(move || {
        let output = match panic::catch_unwind(AssertUnwindSafe(task)) {
            Ok(output) => output,
            Err(payload) => Err(format_cpu_job_panic(payload)),
        };
        let _ = sender.send(output);
    });

    match receiver.await {
        Ok(Ok(output)) => {
            jobs.mark_completed(job_id, output.result_text.as_deref())
                .await?;
            Ok(Ok(output))
        }
        Ok(Err(error)) => {
            jobs.mark_failed(job_id, Some(error.as_str())).await?;
            Ok(Err(error))
        }
        Err(_) => {
            let error = AppInfraError::WorkerTaskCancelled;
            let error_text = error.to_string();
            jobs.mark_failed(job_id, Some(&error_text)).await?;
            Err(error)
        }
    }
}

fn format_cpu_job_panic(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        format!("cpu job panicked: {message}")
    } else if let Some(message) = payload.downcast_ref::<String>() {
        format!("cpu job panicked: {message}")
    } else {
        "cpu job panicked with a non-string payload".to_string()
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
