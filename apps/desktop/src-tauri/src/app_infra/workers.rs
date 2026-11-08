//! The processing, workspace-repair and retention workers. They read the live
//! capture session (recording active, workspaces in use, segments to keep), so
//! they spawn from the host until capture moves (S3). They spawn on the core's
//! runtime through Tauri's handle (the runtime `tauri::async_runtime::set`
//! installed), so app-core's worker control can track and abort them.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use capture_types::RetentionPolicy as SettingsRetentionPolicy;
use futures_util::{
    future::{select, Either},
    pin_mut,
};
use tauri::Manager;
use time::OffsetDateTime;

use app_core::app_infra::*;
use app_core::{AppCore, AppInfraState, CoreEvent};
#[cfg(test)]
use time::{format_description, PrimitiveDateTime};

use super::retention::retention_context_for_app;

const PROCESSING_WORKER_IDLE_POLL_INTERVAL: Duration = Duration::from_millis(500);
const PROCESSING_WORKER_ERROR_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const HIDDEN_SEGMENT_WORKSPACE_REPAIR_INTERVAL: Duration = Duration::from_secs(5 * 60);
const RETENTION_CLEANUP_RETRY_INTERVAL: Duration = Duration::from_secs(5 * 60);

pub(super) fn spawn_processing_worker(
    infra: AppInfraState,
    base_dir: PathBuf,
    app_handle: tauri::AppHandle,
    background_workers: BackgroundWorkersState,
) {
    let base_dir_display = base_dir.display().to_string();

    spawn_processing_worker_loop(
        Arc::clone(&infra),
        base_dir_display.clone(),
        ProcessingWorkerKind::NonTranscriptionAndFrameBatch,
        Some(app_handle.clone()),
        background_workers.clone(),
    );
    spawn_processing_worker_loop(
        Arc::clone(&infra),
        base_dir_display.clone(),
        ProcessingWorkerKind::AudioTranscription,
        None,
        background_workers.clone(),
    );
    spawn_processing_worker_loop(
        Arc::clone(&infra),
        base_dir_display,
        ProcessingWorkerKind::SpeakerAnalysis,
        None,
        background_workers.clone(),
    );

    spawn_hidden_segment_workspace_repair_worker(infra, base_dir, app_handle, background_workers);
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ProcessingWorkerKind {
    NonTranscriptionAndFrameBatch,
    AudioTranscription,
    SpeakerAnalysis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProcessingWorkerPass {
    DidWork,
    Idle,
    IdleFor(Duration),
}

impl ProcessingWorkerKind {
    fn name(self) -> &'static str {
        match self {
            Self::NonTranscriptionAndFrameBatch => "non-transcription processing/frame-batch",
            Self::AudioTranscription => "audio transcription",
            Self::SpeakerAnalysis => "speaker analysis",
        }
    }

    async fn process_once(
        self,
        infra: &::app_infra::AppInfra,
        app_handle: Option<&tauri::AppHandle>,
    ) -> ::app_infra::Result<ProcessingWorkerPass> {
        match self {
            Self::NonTranscriptionAndFrameBatch => {
                process_pending_jobs_once(infra, app_handle).await
            }
            Self::AudioTranscription => process_pending_audio_transcription_jobs_once(infra).await,
            Self::SpeakerAnalysis => process_pending_speaker_analysis_jobs_once(infra).await,
        }
    }
}

pub(super) fn spawn_processing_worker_loop(
    infra: AppInfraState,
    base_dir_display: String,
    worker_kind: ProcessingWorkerKind,
    app_handle: Option<tauri::AppHandle>,
    background_workers: BackgroundWorkersState,
) {
    let worker_name = worker_kind.name();
    app_core::debug_log::log_info(format!(
        "starting app infrastructure {worker_name} worker (base_dir='{}', idle_poll_ms={}, error_retry_ms={})",
        base_dir_display,
        PROCESSING_WORKER_IDLE_POLL_INTERVAL.as_millis(),
        PROCESSING_WORKER_ERROR_RETRY_INTERVAL.as_millis()
    ));

    let mut shutdown_rx = background_workers.subscribe();
    let handle = tauri::async_runtime::handle().inner().spawn(async move {
        let mut consecutive_failures = 0u64;

        loop {
            if *shutdown_rx.borrow() {
                break;
            }

            match worker_kind.process_once(&infra, app_handle.as_ref()).await {
                Ok(ProcessingWorkerPass::DidWork) => {
                    if consecutive_failures > 0 {
                        app_core::debug_log::log_info(format!(
                            "app infrastructure {worker_name} worker recovered after {} failed iteration(s) (base_dir='{}')",
                            consecutive_failures, base_dir_display
                        ));
                        consecutive_failures = 0;
                    }

                    continue;
                }
                Ok(pass @ (ProcessingWorkerPass::Idle | ProcessingWorkerPass::IdleFor(_))) => {
                    if consecutive_failures > 0 {
                        app_core::debug_log::log_info(format!(
                            "app infrastructure {worker_name} worker recovered after {} failed iteration(s) (base_dir='{}')",
                            consecutive_failures, base_dir_display
                        ));
                        consecutive_failures = 0;
                    }

                    let sleep_duration = match pass {
                        ProcessingWorkerPass::IdleFor(duration) => {
                            duration.min(PROCESSING_WORKER_IDLE_POLL_INTERVAL)
                        }
                        ProcessingWorkerPass::Idle => PROCESSING_WORKER_IDLE_POLL_INTERVAL,
                        ProcessingWorkerPass::DidWork => continue,
                    };

                    if shutdown_aware_sleep(&mut shutdown_rx, sleep_duration).await {
                        break;
                    }
                }
                Err(error) => {
                    consecutive_failures += 1;
                    app_core::debug_log::log_warn(format!(
                        "app infrastructure {worker_name} worker iteration failed (base_dir='{}', consecutive_failures={}, retry_in_ms={}): {error}",
                        base_dir_display,
                        consecutive_failures,
                        PROCESSING_WORKER_ERROR_RETRY_INTERVAL.as_millis()
                    ));
                    if shutdown_aware_sleep(
                        &mut shutdown_rx,
                        PROCESSING_WORKER_ERROR_RETRY_INTERVAL,
                    )
                    .await
                    {
                        break;
                    }
                }
            }
        }

        app_core::debug_log::log_info(format!(
            "stopped app infrastructure {worker_name} worker (base_dir='{}')",
            base_dir_display
        ));
    });
    // Processing workers pull from the queue-backed Processing Job table, so an
    // aborted in-flight job is reclaimed (requeued) on next launch. Track them
    // as abortable so quit doesn't block on the current job finishing.
    background_workers.track_abortable(handle);
}

pub(super) fn spawn_hidden_segment_workspace_repair_worker(
    infra: AppInfraState,
    base_dir: PathBuf,
    app_handle: tauri::AppHandle,
    background_workers: BackgroundWorkersState,
) {
    let recordings_root =
        app_core::managed_storage_layout::ManagedStorageLayout::from_base_dir(base_dir)
            .recordings_root();
    let recordings_root_display = recordings_root.display().to_string();

    app_core::debug_log::log_info(format!(
        "starting hidden segment workspace repair worker (recordings_root='{}', interval_ms={})",
        recordings_root_display,
        HIDDEN_SEGMENT_WORKSPACE_REPAIR_INTERVAL.as_millis()
    ));

    let mut shutdown_rx = background_workers.subscribe();
    let handle = tauri::async_runtime::handle().inner().spawn(async move {
        loop {
            if shutdown_aware_sleep(&mut shutdown_rx, HIDDEN_SEGMENT_WORKSPACE_REPAIR_INTERVAL)
                .await
            {
                break;
            }

            let active_workspace_dirs =
                active_workspace_dirs_for_hidden_workspace_repair(&app_handle);

            // Abandon an in-flight repair pass the instant shutdown is requested
            // so quit never blocks on a full scan. The pass only reads from the DB
            // and removes whole workspace dirs between awaits, so dropping the
            // future mid-pass leaves no partial state. `select` polls the shutdown
            // future first, so it also resolves immediately if shutdown was already
            // requested while computing `active_workspace_dirs`.
            let shutdown_changed = shutdown_rx.changed();
            let repair = repair_hidden_segment_workspaces_once(
                &infra,
                &recordings_root,
                &active_workspace_dirs,
                hidden_workspace_repair_retention_cutoff(&app_handle),
            );
            pin_mut!(shutdown_changed, repair);
            let repair_result = match select(shutdown_changed, repair).await {
                Either::Left((_, _)) => break,
                Either::Right((result, _)) => result,
            };

            match repair_result {
                Ok(result) => {
                    app_core::debug_log::log(format!(
                        "hidden segment workspace repair completed (recordings_root='{}', scanned={}, removed={}, skipped={})",
                        recordings_root_display,
                        result.scanned_workspace_count,
                        result.removed_workspace_count,
                        result.skipped_workspace_count
                    ));
                }
                Err(error) => {
                    app_core::debug_log::log_warn(format!(
                        "hidden segment workspace repair failed (recordings_root='{}'): {error}",
                        recordings_root_display
                    ));
                }
            }
        }

        app_core::debug_log::log_info(format!(
            "stopped hidden segment workspace repair worker (recordings_root='{}')",
            recordings_root_display
        ));
    });
    background_workers.track(handle);
}

pub(super) fn spawn_retention_cleanup_worker(
    infra: AppInfraState,
    app_handle: tauri::AppHandle,
    background_workers: BackgroundWorkersState,
) {
    let mut shutdown_rx = background_workers.subscribe();
    let mut retention_schedule_rx = background_workers.subscribe_retention_schedule();
    let handle = tauri::async_runtime::handle().inner().spawn(async move {
        let mut next_sleep = Duration::from_secs(0);
        loop {
            let mut schedule_changed = false;
            if !next_sleep.is_zero() {
                match retention_schedule_aware_sleep(
                    &mut shutdown_rx,
                    &mut retention_schedule_rx,
                    next_sleep,
                )
                .await
                {
                    RetentionSleepOutcome::Shutdown => break,
                    RetentionSleepOutcome::Elapsed => {}
                    RetentionSleepOutcome::ScheduleChanged => {
                        schedule_changed = true;
                        app_core::debug_log::log(
                            "retention cleanup worker woke for retention schedule change",
                        );
                    }
                }
            }
            let Some(settings_state) =
                app_handle.try_state::<app_core::recording_settings::RecordingSettingsState>()
            else {
                break;
            };
            let policy = settings_state
                .lock()
                .map(|guard| guard.settings.retention_policy)
                .unwrap_or(SettingsRetentionPolicy::Never);
            let context = retention_context_for_app(&app_handle, &infra).await;
            let _ = infra
                .capture_retention()
                .retry_pending_file_tombstones(&context)
                .await;
            let mut retry_soon = false;
            if policy != SettingsRetentionPolicy::Never {
                app_core::debug_log::log(format!(
                    "retention cleanup worker running cleanup (policy={}, triggered_by={})",
                    app_retention_policy(policy).as_str(),
                    if schedule_changed {
                        "settings_change"
                    } else {
                        "timer"
                    }
                ));
                match infra
                    .capture_retention()
                    .run_cleanup_with_mode(
                        app_retention_policy(policy),
                        local_now_for_retention(),
                        &context,
                        ::app_infra::RetentionCleanupMode::Automatic,
                    )
                    .await
                {
                    Ok(summary)
                        if summary.deleted_frames > 0
                            || summary.deleted_audio_segments > 0
                            || summary.deleted_capture_segments > 0 =>
                    {
                        retry_soon =
                            summary.skipped_running_jobs > 0 || summary.pending_file_tombstones > 0;
                        if summary.deleted_capture_segments > 0 {
                            let _ = super::frame_preview::clear_scrub_preview_cache_for_video_paths(
                                app_handle.clone(),
                                &summary.deleted_capture_segment_media_paths,
                            );
                        }
                        app_handle
                            .state::<Arc<AppCore>>()
                            .host()
                            .emit(CoreEvent::TimelineDataChanged(TimelineDataChangedPayload {
                                reason: "retention".to_string(),
                                deleted_before: summary.cutoff_ended_before.clone(),
                                started_at: None,
                                ended_at: None,
                                deleted_frame_ids: summary.deleted_frame_ids.clone(),
                                deleted_audio_segment_ids: summary
                                    .deleted_audio_segment_ids
                                    .clone(),
                            }));
                        app_core::debug_log::log_info(format!(
                            "retention cleanup worker completed cleanup (policy={}, status={}, eligible_segments={}, deleted_segments={}, deleted_frames={}, deleted_audio_segments={}, retry_soon={})",
                            summary.policy,
                            summary.status,
                            summary.eligible_capture_segments,
                            summary.deleted_capture_segments,
                            summary.deleted_frames,
                            summary.deleted_audio_segments,
                            retry_soon
                        ));
                    }
                    Ok(summary) => {
                        retry_soon =
                            summary.skipped_running_jobs > 0 || summary.pending_file_tombstones > 0;
                        app_core::debug_log::log(format!(
                            "retention cleanup worker completed cleanup (policy={}, status={}, eligible_segments={}, deleted_segments={}, deleted_frames={}, deleted_audio_segments={}, retry_soon={})",
                            summary.policy,
                            summary.status,
                            summary.eligible_capture_segments,
                            summary.deleted_capture_segments,
                            summary.deleted_frames,
                            summary.deleted_audio_segments,
                            retry_soon
                        ));
                    }
                    Err(error) => app_core::debug_log::log_warn(format!(
                        "retention cleanup worker failed: {error}"
                    )),
                }
            }
            next_sleep = if retry_soon {
                RETENTION_CLEANUP_RETRY_INTERVAL
            } else {
                duration_until_next_retention_daily_run()
            };
            app_core::debug_log::log(format!(
                "retention cleanup worker scheduled next wake (policy={}, next_sleep_ms={})",
                app_retention_policy(policy).as_str(),
                next_sleep.as_millis()
            ));
        }
    });
    background_workers.track(handle);
}

pub(super) fn duration_until_next_retention_daily_run() -> Duration {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let date = now.date();
    let target_time = time::Time::from_hms(0, 5, 0).unwrap_or(time::Time::MIDNIGHT);
    let mut target = date.with_time(target_time).assume_offset(now.offset());
    if target <= now {
        target = (date + time::Duration::days(1))
            .with_time(target_time)
            .assume_offset(now.offset());
    }
    let seconds = (target - now).whole_seconds().max(60) as u64;
    Duration::from_secs(seconds)
}

pub(super) async fn repair_hidden_segment_workspaces_once(
    infra: &::app_infra::AppInfra,
    recordings_root: &Path,
    active_workspace_dirs: &BTreeSet<String>,
    retention_cutoff: Option<String>,
) -> ::app_infra::Result<::app_infra::HiddenSegmentWorkspaceRepairResult> {
    infra
        .repair_hidden_segment_workspaces_with_context(
            recordings_root,
            &::app_infra::HiddenSegmentWorkspaceRepairContext {
                active_workspace_dirs: active_workspace_dirs.clone(),
                retention_cutoff,
            },
        )
        .await
}

/// The Retention Cleanup cutoff for the user's current policy, so the repair
/// ages out orphaned visible segments exactly as retention would. `None` when
/// retention is Never (or settings aren't managed yet), which keeps them.
pub(super) fn hidden_workspace_repair_retention_cutoff(
    app_handle: &tauri::AppHandle,
) -> Option<String> {
    let settings =
        app_handle.try_state::<app_core::recording_settings::RecordingSettingsState>()?;
    let policy = retention_policy_from_settings(&settings, None);
    ::app_infra::cutoff_ended_before(app_retention_policy(policy), local_now_for_retention())
}

pub(super) fn active_workspace_dirs_for_hidden_workspace_repair(
    app_handle: &tauri::AppHandle,
) -> BTreeSet<String> {
    let state = app_handle.state::<crate::native_capture::NativeCaptureState>();
    let Ok(runtime) = state.lock() else {
        return BTreeSet::new();
    };
    let runtime = runtime.runtime();
    if !runtime.is_running {
        return BTreeSet::new();
    }

    let mut active_workspace_dirs = BTreeSet::new();
    for screen_file in [
        runtime.recording_file.as_deref(),
        runtime
            .current_segment_output_files
            .as_ref()
            .and_then(|files| files.screen_file.as_deref()),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(workspace_dir) = hidden_workspace_dir_for_screen_recording_file(screen_file) {
            active_workspace_dirs.insert(workspace_dir);
        }
    }

    if active_workspace_dirs.is_empty()
        && runtime
            .source_sessions
            .as_ref()
            .and_then(|source_sessions| source_sessions.screen.as_ref())
            .is_some()
    {
        if let Some(planner) = runtime.segment_planner.as_ref() {
            active_workspace_dirs.insert(
                planner
                    .segment_workspace_dir(runtime.current_segment_index)
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }

    active_workspace_dirs
}

pub(super) fn hidden_workspace_dir_for_screen_recording_file(screen_file: &str) -> Option<String> {
    let path = Path::new(screen_file);
    if let Some(parent) = path.parent() {
        if ::app_infra::HiddenSegmentWorkspacePaths::from_workspace_dir(parent).is_some() {
            return Some(parent.to_string_lossy().to_string());
        }
    }

    let parent = path.parent()?;
    let stem = path.file_stem()?.to_str()?;
    let workspace_dir = parent.join(format!(".{stem}"));
    ::app_infra::HiddenSegmentWorkspacePaths::from_workspace_dir(&workspace_dir)?;
    Some(workspace_dir.to_string_lossy().to_string())
}

pub(super) async fn process_pending_jobs_once(
    infra: &::app_infra::AppInfra,
    app_handle: Option<&tauri::AppHandle>,
) -> ::app_infra::Result<ProcessingWorkerPass> {
    let did_processing = infra
        .process_next_processing_job_excluding_processors(&[
            ::app_infra::OCR_PROCESSOR,
            ::app_infra::AUDIO_TRANSCRIPTION_PROCESSOR,
            ::app_infra::SPEAKER_ANALYSIS_PROCESSOR,
        ])
        .await?
        .is_some();

    let did_finalize = match infra.process_next_frame_batch_job().await {
        Ok(result) => result.is_some(),
        Err(error) => {
            app_core::debug_log::log_warn(format!(
                "app infrastructure frame-batch finalization failed after state update; worker will continue: {error}"
            ));
            true
        }
    };

    let ocr_outcome = app_core::ocr_budget::process_pending_ocr_job_once(
        infra,
        live_recording_active(app_handle),
    )
    .await?;
    let did_ocr = ocr_outcome == app_core::ocr_budget::OcrProcessingPass::DidWork;
    let did_finalize_after_ocr = if did_ocr {
        match infra.process_next_frame_batch_job().await {
            Ok(result) => result.is_some(),
            Err(error) => {
                app_core::debug_log::log_warn(format!(
                    "app infrastructure frame-batch finalization failed after OCR state update; worker will continue: {error}"
                ));
                true
            }
        }
    } else {
        false
    };

    if did_processing || did_finalize || did_ocr || did_finalize_after_ocr {
        Ok(ProcessingWorkerPass::DidWork)
    } else if let app_core::ocr_budget::OcrProcessingPass::CoolingDown(duration) = ocr_outcome {
        Ok(ProcessingWorkerPass::IdleFor(duration))
    } else {
        Ok(ProcessingWorkerPass::Idle)
    }
}

#[cfg(test)]
pub(super) fn parse_job_timestamp(value: &str) -> Option<OffsetDateTime> {
    if let Ok(parsed) = OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
    {
        return Some(parsed);
    }

    let sqlite_format =
        format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]").ok()?;
    PrimitiveDateTime::parse(value, &sqlite_format)
        .ok()
        .map(|parsed| parsed.assume_utc())
}

#[cfg(test)]
pub(super) fn timestamp_delta_ms(start: Option<&str>, end: Option<&str>) -> Option<i64> {
    let start = parse_job_timestamp(start?)?;
    let end = parse_job_timestamp(end?)?;
    Some((end - start).whole_milliseconds().max(0) as i64)
}

pub(super) fn live_recording_active(app_handle: Option<&tauri::AppHandle>) -> bool {
    app_handle
        .and_then(|app_handle| app_handle.try_state::<crate::native_capture::NativeCaptureState>())
        .and_then(|state| {
            state
                .lock()
                .ok()
                .map(|lifecycle| lifecycle.runtime().is_running)
        })
        .unwrap_or(false)
}

pub(super) async fn process_pending_audio_transcription_jobs_once(
    infra: &::app_infra::AppInfra,
) -> ::app_infra::Result<ProcessingWorkerPass> {
    if infra
        .process_next_processing_job_for_processor(::app_infra::AUDIO_TRANSCRIPTION_PROCESSOR)
        .await?
        .is_some()
    {
        Ok(ProcessingWorkerPass::DidWork)
    } else {
        Ok(ProcessingWorkerPass::Idle)
    }
}

pub(super) async fn process_pending_speaker_analysis_jobs_once(
    infra: &::app_infra::AppInfra,
) -> ::app_infra::Result<ProcessingWorkerPass> {
    if infra
        .process_next_processing_job_for_processor(::app_infra::SPEAKER_ANALYSIS_PROCESSOR)
        .await?
        .is_some()
    {
        Ok(ProcessingWorkerPass::DidWork)
    } else {
        Ok(ProcessingWorkerPass::Idle)
    }
}
