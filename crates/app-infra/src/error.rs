use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppInfraError>;

#[derive(Debug, Error)]
pub enum AppInfraError {
    #[error("failed to access app infrastructure files: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("database migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to build the CPU worker pool: {0}")]
    WorkerPoolBuild(#[from] rayon::ThreadPoolBuildError),
    #[error("background jobs require an active Tokio runtime")]
    AsyncRuntimeUnavailable,
    #[error("cpu job worker stopped before returning a result")]
    WorkerTaskCancelled,
    #[error("background job {0} was not found")]
    JobNotFound(i64),
    #[error("invalid background job status: {0}")]
    InvalidJobStatus(String),
}
