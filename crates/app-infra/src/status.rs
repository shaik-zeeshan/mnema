use serde::Serialize;

use crate::frame_batch_store::FrameBatchCounts;
use crate::jobs::JobCounts;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfraStatus {
    pub database_path: String,
    /// Size of the SQLite file at `database_path`, from file metadata. `None`
    /// when the file cannot be stat'd (not yet created, or a permission error) —
    /// a debug readout must never fail the whole status call over its size.
    ///
    /// ponytail: the main DB file only; `-wal`/`-shm` sidecars are not summed.
    pub database_size_bytes: Option<u64>,
    pub migrations_ran: bool,
    /// Rows in `_sqlx_migrations` — how many schema migrations this DB has
    /// applied. `None` when the count query fails; a debug readout must never
    /// fail the whole status call.
    pub applied_migration_count: Option<i64>,
    pub worker_thread_count: usize,
    pub job_counts: JobCounts,
    pub frame_batch_counts: FrameBatchCounts,
}
