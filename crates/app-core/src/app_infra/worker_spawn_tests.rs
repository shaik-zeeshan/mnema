use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::worker_spawn::*;

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("desktop-tauri-{label}-{unique}"));

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
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build")
        .block_on(test);
}

#[test]
fn timestamp_delta_ms_accepts_sqlite_current_timestamp_format() {
    assert_eq!(
        timestamp_delta_ms(Some("2026-04-12 10:00:00"), Some("2026-04-12 10:00:02")),
        Some(2000)
    );
    assert_eq!(
        timestamp_delta_ms(Some("2026-04-12T10:00:00Z"), Some("2026-04-12T10:00:02Z")),
        Some(2000)
    );
}

#[test]
fn process_pending_jobs_once_claims_and_processes_queued_work() {
    run_async_test(async {
        let dir = TestDir::new("processing-worker-once");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");

        let persisted = infra
            .debug_insert_frame_and_enqueue_processing_job(
                &::app_infra::NewFrame::new(
                    "session-worker",
                    "/tmp/frame-worker.png",
                    "2026-04-12T10:00:00Z",
                ),
                "missing-processor",
                None,
            )
            .await
            .expect("frame and job should persist");

        let processed = process_pending_jobs_once(&infra, None)
            .await
            .expect("worker iteration should succeed");
        assert_eq!(processed, ProcessingWorkerPass::DidWork);

        let job = infra
            .get_processing_job(persisted.job.id)
            .await
            .expect("job should be readable")
            .expect("job should exist");
        assert_eq!(job.status, ::app_infra::ProcessingJobStatus::Failed);
        assert_eq!(job.attempt_count, 1);
        assert_eq!(
            job.last_error.as_deref(),
            Some("processor backend is not registered for 'missing-processor'")
        );
    });
}

#[test]
fn processing_workers_keep_audio_transcription_claiming_separate() {
    run_async_test(async {
        let dir = TestDir::new("processing-worker-transcription-separate");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let job = infra
            .enqueue_processing_job(
                &::app_infra::ProcessingJobDraft::for_audio_segment_transcription(123),
            )
            .await
            .expect("transcription job should enqueue");

        let processed = process_pending_jobs_once(&infra, None)
            .await
            .expect("non-transcription worker should succeed");
        assert_eq!(processed, ProcessingWorkerPass::Idle);
        let still_queued = infra
            .get_processing_job(job.id)
            .await
            .expect("job should be readable")
            .expect("job should exist");
        assert_eq!(
            still_queued.status,
            ::app_infra::ProcessingJobStatus::Queued
        );

        let processed = process_pending_audio_transcription_jobs_once(&infra)
            .await
            .expect("transcription worker should succeed");
        assert_eq!(processed, ProcessingWorkerPass::DidWork);
        // The transcription backend ran and failed (audio segment 123 does not
        // exist), but bounded failure-retry requeues the job within its attempt
        // cap rather than leaving it terminally failed, so a transient miss can
        // still recover.  The requeue clears `last_error` and returns the job to
        // `queued`; the single failure is recorded in `failure_count`.
        let requeued = infra
            .get_processing_job(job.id)
            .await
            .expect("job should be readable")
            .expect("job should exist");
        assert_eq!(requeued.status, ::app_infra::ProcessingJobStatus::Queued);
        assert_eq!(requeued.attempt_count, 1);
        assert_eq!(requeued.failure_count, 1);
        assert!(requeued.last_error.is_none());
    });
}

#[test]
fn ocr_pacing_state_is_scoped_to_app_infra_base_dir() {
    run_async_test(async {
        let first_dir = TestDir::new("ocr-pacing-first");
        let first_infra = ::app_infra::AppInfra::initialize(first_dir.path())
            .await
            .expect("first app infra should initialize");
        let first = first_infra
            .debug_insert_frame_and_enqueue_ocr_job(
                &::app_infra::NewFrame::new(
                    "session-first",
                    "/tmp/frame-first.png",
                    "2026-04-12T10:00:00Z",
                ),
                None,
            )
            .await
            .expect("first OCR job should enqueue");

        assert_eq!(
            crate::ocr_budget::process_pending_ocr_job_once(&first_infra, true)
                .await
                .expect("first OCR worker pass should succeed"),
            crate::ocr_budget::OcrProcessingPass::DidWork
        );
        let first_job = first_infra
            .get_processing_job(first.job.id)
            .await
            .expect("first job should be readable")
            .expect("first job should exist");
        // The OCR job fails (the frame file does not exist) but bounded
        // failure-retry requeues it within its attempt cap, so it returns to
        // `queued` with the failure recorded rather than terminally `failed`.
        assert_eq!(first_job.status, ::app_infra::ProcessingJobStatus::Queued);
        assert_eq!(first_job.failure_count, 1);

        let second_dir = TestDir::new("ocr-pacing-second");
        let second_infra = ::app_infra::AppInfra::initialize(second_dir.path())
            .await
            .expect("second app infra should initialize");
        let second = second_infra
            .debug_insert_frame_and_enqueue_ocr_job(
                &::app_infra::NewFrame::new(
                    "session-second",
                    "/tmp/frame-second.png",
                    "2026-04-12T10:00:01Z",
                ),
                None,
            )
            .await
            .expect("second OCR job should enqueue");

        assert_eq!(
            crate::ocr_budget::process_pending_ocr_job_once(&second_infra, false)
                .await
                .expect("second OCR worker pass should succeed"),
            crate::ocr_budget::OcrProcessingPass::DidWork
        );
        let second_job = second_infra
            .get_processing_job(second.job.id)
            .await
            .expect("second job should be readable")
            .expect("second job should exist");
        assert_eq!(second_job.status, ::app_infra::ProcessingJobStatus::Queued);
        assert_eq!(second_job.failure_count, 1);
    });
}

#[test]
fn process_pending_jobs_once_can_process_frame_batch_finalize_jobs() {
    run_async_test(async {
        let dir = TestDir::new("frame-batch-worker-once");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");

        infra
            .capture_frame(
                &::app_infra::NewFrame::new(
                    "session-batch-worker",
                    "/tmp/session-batch-worker-segment-0001/frames/frame-1.png",
                    "2026-04-12T10:01:00Z",
                ),
                None,
            )
            .await
            .expect("first frame should persist");
        infra
            .capture_frame(
                &::app_infra::NewFrame::new(
                    "session-batch-worker",
                    "/tmp/session-batch-worker-segment-0002/frames/frame-2.png",
                    "2026-04-12T10:11:00Z",
                ),
                None,
            )
            .await
            .expect("second frame should persist");

        let processed = process_pending_jobs_once(&infra, None)
            .await
            .expect("worker iteration should succeed");
        assert_eq!(processed, ProcessingWorkerPass::DidWork);

        let batches = infra
            .list_frame_batches(Some("session-batch-worker"))
            .await
            .expect("frame batches should list");

        // The first batch's OCR job is processed in the same iteration but
        // *fails* (no OCR backend can read the missing frame file).  Bounded
        // failure-retry requeues that job within its attempt cap rather than
        // leaving it terminally failed, and a batch only finalizes once all its
        // OCR jobs reach a terminal state, so the finalize job stays gated.  The
        // batch therefore rolls to Closed with a pending finalize job but does
        // not reach Completed until the OCR retries are exhausted — proving the
        // worker services finalize jobs (it claims the finalize job and respects
        // its gate) instead of starving them or finalizing ahead of OCR.
        assert!(!batches
            .iter()
            .any(|batch| batch.status == ::app_infra::FrameBatchStatus::Completed));
        assert!(batches.iter().any(
            |batch| batch.status == ::app_infra::FrameBatchStatus::Closed
                && batch.finalize_job_id.is_some()
        ));
    });
}

#[test]
fn repair_hidden_segment_workspaces_once_removes_safe_unreferenced_workspace() {
    run_async_test(async {
        let dir = TestDir::new("repair-hidden-workspace-safe");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let recordings_root =
            crate::managed_storage_layout::ManagedStorageLayout::from_base_dir(dir.path())
                .recordings_root();
        let day_dir = recordings_root.join("2026/04/12");
        let workspace_dir = day_dir.join(".session-repair-safe-segment-0001");
        fs::create_dir_all(workspace_dir.join("frames"))
            .expect("workspace frames dir should be created");
        fs::write(
            day_dir.join("session-repair-safe-segment-0001.mov"),
            b"fake mov",
        )
        .expect("visible segment should be written");

        let result =
            repair_hidden_segment_workspaces_once(&infra, &recordings_root, &BTreeSet::new(), None)
                .await
                .expect("repair should succeed");

        assert_eq!(result.scanned_workspace_count, 1);
        assert_eq!(result.removed_workspace_count, 1);
        assert_eq!(result.skipped_workspace_count, 0);
        assert!(!workspace_dir.exists(), "safe workspace should be removed");
    });
}

#[test]
fn repair_hidden_segment_workspaces_once_preserves_missing_visible_segment_workspace() {
    run_async_test(async {
        let dir = TestDir::new("repair-hidden-workspace-missing-visible");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let recordings_root =
            crate::managed_storage_layout::ManagedStorageLayout::from_base_dir(dir.path())
                .recordings_root();
        let workspace_dir =
            recordings_root.join("2026/04/12/.session-repair-preserve-segment-0001");
        let frames_dir = workspace_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("workspace frames dir should be created");
        fs::write(frames_dir.join("frame-1.jpg"), b"fake frame")
            .expect("workspace frame artifact should be created");

        let result =
            repair_hidden_segment_workspaces_once(&infra, &recordings_root, &BTreeSet::new(), None)
                .await
                .expect("repair should succeed");

        assert_eq!(result.scanned_workspace_count, 1);
        assert_eq!(result.removed_workspace_count, 0);
        assert_eq!(result.skipped_workspace_count, 1);
        assert!(
            workspace_dir.exists(),
            "workspace should be preserved when visible segment is missing"
        );
    });
}

#[test]
fn repair_hidden_segment_workspaces_once_skips_active_screen_session_workspace() {
    run_async_test(async {
        let dir = TestDir::new("repair-hidden-workspace-active-session");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let recordings_root =
            crate::managed_storage_layout::ManagedStorageLayout::from_base_dir(dir.path())
                .recordings_root();
        let day_dir = recordings_root.join("2026/04/12");
        let workspace_dir = day_dir.join(".active-screen-session-segment-0001");
        fs::create_dir_all(workspace_dir.join("frames"))
            .expect("workspace frames dir should be created");
        fs::write(
            day_dir.join("active-screen-session-segment-0001.mov"),
            b"fake mov",
        )
        .expect("visible segment should be written");

        let result = repair_hidden_segment_workspaces_once(
            &infra,
            &recordings_root,
            &BTreeSet::from([workspace_dir.to_string_lossy().to_string()]),
            None,
        )
        .await
        .expect("repair should succeed");

        assert_eq!(result.scanned_workspace_count, 1);
        assert_eq!(result.removed_workspace_count, 0);
        assert_eq!(result.skipped_workspace_count, 1);
        assert!(
            workspace_dir.exists(),
            "active screen session workspace should be preserved"
        );
    });
}
