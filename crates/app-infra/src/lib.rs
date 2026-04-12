mod db;
pub mod error;
pub mod jobs;
pub mod status;

use std::path::Path;

use sqlx::SqlitePool;

pub use error::{AppInfraError, Result};
pub use jobs::{
    default_worker_thread_count, BackgroundJob, BackgroundJobStatus, CpuJobHandle, CpuJobResult,
    CpuJobSuccess, DebugCpuJobRequest, JobCounts, JobDescriptor, JobRuntime, JobStore,
};
pub use status::AppInfraStatus;

#[derive(Clone)]
pub struct AppInfra {
    database: db::Database,
    jobs: JobStore,
    runtime: JobRuntime,
}

impl AppInfra {
    pub async fn initialize<P: AsRef<Path>>(base_dir: P) -> Result<Self> {
        let database = db::Database::initialize(base_dir.as_ref()).await?;
        let jobs = JobStore::new(database.pool().clone());
        jobs.reconcile_orphaned_running_jobs().await?;
        let runtime = JobRuntime::new(default_worker_thread_count())?;

        Ok(Self {
            database,
            jobs,
            runtime,
        })
    }

    pub fn pool(&self) -> &SqlitePool {
        self.database.pool()
    }

    pub fn database_path(&self) -> &Path {
        self.database.database_path()
    }

    pub fn jobs(&self) -> &JobStore {
        &self.jobs
    }

    pub fn runtime(&self) -> &JobRuntime {
        &self.runtime
    }

    pub async fn enqueue_job(
        &self,
        descriptor: &JobDescriptor,
        payload_json: Option<&str>,
    ) -> Result<BackgroundJob> {
        self.jobs.enqueue(descriptor, payload_json).await
    }

    pub async fn list_jobs(&self) -> Result<Vec<BackgroundJob>> {
        self.jobs.list(None).await
    }

    pub async fn get_job(&self, job_id: i64) -> Result<Option<BackgroundJob>> {
        self.jobs.get(job_id).await
    }

    pub async fn submit_debug_cpu_job(
        &self,
        request: DebugCpuJobRequest,
    ) -> Result<BackgroundJob> {
        let request = request.normalized();
        let payload_json = serde_json::to_string(&request)?;
        let task_request = request.clone();
        let handle = self
            .spawn_cpu_job(
                JobDescriptor::new(jobs::DEBUG_CPU_JOB_KIND),
                Some(&payload_json),
                move || {
                    let result_text = task_request.simulated_result_text();
                    Ok(CpuJobSuccess::new(result_text.clone()).with_result_text(result_text))
                },
            )
            .await?;

        self.get_job(handle.job_id())
            .await?
            .ok_or(AppInfraError::JobNotFound(handle.job_id()))
    }

    pub async fn spawn_cpu_job<F, T>(
        &self,
        descriptor: JobDescriptor,
        payload_json: Option<&str>,
        task: F,
    ) -> Result<CpuJobHandle<T>>
    where
        F: FnOnce() -> CpuJobResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let job = self.jobs.enqueue(&descriptor, payload_json).await?;
        self.runtime.spawn_cpu(self.jobs.clone(), job, task)
    }

    pub async fn status(&self) -> Result<AppInfraStatus> {
        Ok(AppInfraStatus {
            database_path: self.database.database_path().display().to_string(),
            migrations_ran: self.database.migrations_ran(),
            worker_thread_count: self.runtime.worker_thread_count(),
            job_counts: self.jobs.counts().await?,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::mpsc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::{db::Database, jobs::ORPHANED_RUNNING_JOB_ERROR};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("app-infra-{label}-{unique}"));

            fs::create_dir_all(&path).expect("test directory should be created");

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
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

    fn build_test_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
    }

    #[test]
    fn database_reports_when_embedded_migrations_ran() {
        run_async_test(async {
            let dir = TestDir::new("migrations-ran");

            let first = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            assert!(first.migrations_ran());

            drop(first);

            let second = Database::initialize(dir.path())
                .await
                .expect("database should re-initialize");
            assert!(!second.migrations_ran());
        });
    }

    #[test]
    fn cpu_jobs_persist_running_and_completed_transitions() {
        run_async_test(async {
            let dir = TestDir::new("cpu-job-success");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let (started_tx, started_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();

            let handle = infra
                .spawn_cpu_job(
                    JobDescriptor::new("ocr"),
                    Some("{\"documentId\":1}"),
                    move || {
                        started_tx
                            .send(())
                            .expect("job should notify when it starts");
                        release_rx
                            .recv()
                            .expect("job should wait until the test releases it");

                        Ok(CpuJobSuccess::new("finished".to_string())
                            .with_result_text("recognized text"))
                    },
                )
                .await
                .expect("cpu job should spawn");

            started_rx.recv().expect("job should reach the worker pool");

            let running = infra
                .jobs()
                .get(handle.job_id())
                .await
                .expect("running job should be readable")
                .expect("running job should exist");
            assert_eq!(running.status, BackgroundJobStatus::Running);
            assert_eq!(running.attempt_count, 1);
            assert!(running.started_at.is_some());
            assert!(running.finished_at.is_none());

            release_tx.send(()).expect("test should release the job");

            let outcome = handle.join().await.expect("job join should succeed");
            assert_eq!(
                outcome,
                Ok(CpuJobSuccess::new("finished".to_string()).with_result_text("recognized text"))
            );

            let completed = infra
                .jobs()
                .get(running.id)
                .await
                .expect("completed job should be readable")
                .expect("completed job should exist");
            assert_eq!(completed.status, BackgroundJobStatus::Completed);
            assert_eq!(completed.result_text.as_deref(), Some("recognized text"));
            assert!(completed.finished_at.is_some());
            assert_eq!(completed.last_error, None);
        });
    }

    #[test]
    fn enqueued_jobs_are_persisted_as_queued() {
        run_async_test(async {
            let dir = TestDir::new("queued-job");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let queued = infra
                .enqueue_job(&JobDescriptor::new("ocr"), Some("{\"documentId\":1}"))
                .await
                .expect("job should enqueue");

            assert_eq!(queued.status, BackgroundJobStatus::Queued);
            assert_eq!(queued.payload_json.as_deref(), Some("{\"documentId\":1}"));
            assert_eq!(queued.attempt_count, 0);
            assert!(queued.started_at.is_none());
            assert!(queued.finished_at.is_none());
        });
    }

    #[test]
    fn cpu_jobs_persist_failed_transitions() {
        run_async_test(async {
            let dir = TestDir::new("cpu-job-failure");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let handle: CpuJobHandle<String> = infra
                .spawn_cpu_job(JobDescriptor::new("transcription"), None, || {
                    Err("transcription failed".to_string())
                })
                .await
                .expect("cpu job should spawn");

            let job_id = handle.job_id();
            let outcome = handle.join().await.expect("job join should complete");
            assert_eq!(outcome, Err("transcription failed".to_string()));

            let failed = infra
                .jobs()
                .get(job_id)
                .await
                .expect("failed job should be readable")
                .expect("failed job should exist");
            assert_eq!(failed.status, BackgroundJobStatus::Failed);
            assert_eq!(failed.last_error.as_deref(), Some("transcription failed"));
            assert!(failed.started_at.is_some());
            assert!(failed.finished_at.is_some());
            assert_eq!(failed.result_text, None);
        });
    }

    #[test]
    fn cpu_job_panics_are_persisted_as_failed() {
        run_async_test(async {
            let dir = TestDir::new("cpu-job-panic");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let handle: CpuJobHandle<String> = infra
                .spawn_cpu_job(JobDescriptor::new("transcription"), None, || {
                    panic!("worker panic");
                })
                .await
                .expect("cpu job should spawn");

            let job_id = handle.job_id();
            let outcome = handle.join().await.expect("job join should complete");
            assert_eq!(outcome, Err("cpu job panicked: worker panic".to_string()));

            let failed = infra
                .jobs()
                .get(job_id)
                .await
                .expect("failed job should be readable")
                .expect("failed job should exist");
            assert_eq!(failed.status, BackgroundJobStatus::Failed);
            assert_eq!(
                failed.last_error.as_deref(),
                Some("cpu job panicked: worker panic")
            );
            assert!(failed.started_at.is_some());
            assert!(failed.finished_at.is_some());
            assert_eq!(failed.result_text, None);
        });
    }

    #[test]
    fn startup_reconciles_orphaned_running_jobs() {
        run_async_test(async {
            let dir = TestDir::new("orphaned-running-job");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let queued = infra
                .enqueue_job(&JobDescriptor::new("ocr"), Some("{\"documentId\":1}"))
                .await
                .expect("job should enqueue");

            let running = infra
                .jobs()
                .mark_running(queued.id)
                .await
                .expect("job should be marked running");
            assert_eq!(running.status, BackgroundJobStatus::Running);

            drop(infra);

            let recovered = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should re-initialize");

            let failed = recovered
                .jobs()
                .get(queued.id)
                .await
                .expect("recovered job should be readable")
                .expect("recovered job should exist");
            assert_eq!(failed.status, BackgroundJobStatus::Failed);
            assert_eq!(
                failed.last_error.as_deref(),
                Some(ORPHANED_RUNNING_JOB_ERROR)
            );
            assert!(failed.finished_at.is_some());
        });
    }

    #[test]
    fn spawn_setup_failures_mark_jobs_failed() {
        let dir = TestDir::new("spawn-setup-failure");

        let (jobs, job, runtime) = {
            let setup_runtime = build_test_runtime();
            setup_runtime.block_on(async {
                let database = Database::initialize(dir.path())
                    .await
                    .expect("database should initialize");
                let jobs = JobStore::new(database.pool().clone());
                let job = jobs
                    .enqueue(&JobDescriptor::new("ocr"), Some("{\"documentId\":1}"))
                    .await
                    .expect("job should enqueue");
                let runtime = JobRuntime::new(1).expect("job runtime should initialize");

                (jobs, job, runtime)
            })
        };

        let error = runtime
            .spawn_cpu(jobs.clone(), job.clone(), || {
                Ok(CpuJobSuccess::new("done".to_string()))
            })
            .err()
            .expect("spawning without a tokio runtime should fail");
        assert!(matches!(error, AppInfraError::AsyncRuntimeUnavailable));

        let verify_runtime = build_test_runtime();
        verify_runtime.block_on(async {
            let failed = jobs
                .get(job.id)
                .await
                .expect("failed job should be readable")
                .expect("failed job should exist");
            assert_eq!(failed.status, BackgroundJobStatus::Failed);
            assert_eq!(
                failed.last_error.as_deref(),
                Some("background jobs require an active Tokio runtime")
            );
            assert!(failed.started_at.is_none());
            assert!(failed.finished_at.is_some());
        });
    }
}
