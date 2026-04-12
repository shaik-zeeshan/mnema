use serde::Serialize;

use crate::jobs::JobCounts;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfraStatus {
    pub database_path: String,
    pub migrations_ran: bool,
    pub worker_thread_count: usize,
    pub job_counts: JobCounts,
}
