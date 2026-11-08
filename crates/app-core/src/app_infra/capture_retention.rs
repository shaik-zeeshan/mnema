//! Retention Cleanup's commands and Delete Recent Capture: the halves that need
//! the live capture session (segments still being written, the recording
//! boundary). The storage halves live beside them in `crate::app_infra`.

use std::sync::Arc;

use capture_types::CaptureSources;
use time::OffsetDateTime;

use crate::app_infra::*;
use crate::{AppCore, AppInfraState, CoreEvent};

#[derive(Debug, Clone)]
pub(super) struct ActiveCaptureSegmentRef {
    source_kind: ::app_infra::CaptureSourceKind,
    source_session_id: String,
    segment_index: i64,
}

pub(super) fn active_capture_segment_refs_for_sources(
    source_sessions: capture_types::SourceSessions,
    sources: CaptureSources,
    segment_index: i64,
) -> Vec<ActiveCaptureSegmentRef> {
    let mut refs = Vec::new();
    if sources.screen {
        if let Some(source_session) = source_sessions.screen {
            refs.push(ActiveCaptureSegmentRef {
                source_kind: ::app_infra::CaptureSourceKind::Screen,
                source_session_id: source_session.session_id,
                segment_index,
            });
        }
    }
    if sources.microphone {
        if let Some(source_session) = source_sessions.microphone {
            refs.push(ActiveCaptureSegmentRef {
                source_kind: ::app_infra::CaptureSourceKind::Microphone,
                source_session_id: source_session.session_id,
                segment_index,
            });
        }
    }
    if sources.system_audio {
        if let Some(source_session) = source_sessions.system_audio {
            refs.push(ActiveCaptureSegmentRef {
                source_kind: ::app_infra::CaptureSourceKind::SystemAudio,
                source_session_id: source_session.session_id,
                segment_index,
            });
        }
    }
    refs
}

pub(super) async fn retention_context_for_app(
    core: &AppCore,
    infra: &::app_infra::AppInfra,
) -> ::app_infra::RetentionCleanupContext {
    let save_directory = Some(infra.base_dir().display().to_string());
    let mut active_capture_segment_refs = Vec::new();
    if let Some(state) = core.try_state::<crate::native_capture::lifecycle::NativeCaptureState>() {
        if let Ok(lifecycle) = state.lock() {
            let runtime = lifecycle.runtime();
            if runtime.is_running {
                let segment_index = runtime.current_segment_index.try_into().ok();
                let source_sessions = runtime.source_sessions.clone();
                let sources = runtime
                    .current_segment_sources
                    .clone()
                    .or_else(|| runtime.requested_sources.clone());
                if let (Some(segment_index), Some(source_sessions), Some(sources)) =
                    (segment_index, source_sessions, sources)
                {
                    active_capture_segment_refs = active_capture_segment_refs_for_sources(
                        source_sessions,
                        sources,
                        segment_index,
                    );
                }
            }
        }
    }
    let active_source_session_ids = active_capture_segment_refs
        .iter()
        .map(|active_ref| active_ref.source_session_id.clone())
        .collect();
    let mut active_capture_segment_ids = Vec::new();
    for active_ref in active_capture_segment_refs {
        match infra
            .capture_retention()
            .capture_segment_by_source(
                active_ref.source_kind,
                &active_ref.source_session_id,
                active_ref.segment_index,
            )
            .await
        {
            Ok(Some(segment)) => active_capture_segment_ids.push(segment.id),
            Ok(None) => {}
            Err(error) => crate::debug_log::log_warn(format!(
                "failed to resolve active capture segment for retention context (source_kind={}, source_session_id='{}', segment_index={}): {error}",
                active_ref.source_kind.as_str(),
                active_ref.source_session_id,
                active_ref.segment_index
            )),
        }
    }
    ::app_infra::RetentionCleanupContext {
        active_capture_segment_ids,
        active_source_session_ids,
        save_directory,
    }
}

pub async fn preview_retention_cleanup(
    request: Option<PreviewRetentionCleanupRequest>,
    core: &AppCore,
    infra: &AppInfraState,
    settings: &crate::recording_settings::RecordingSettingsState,
) -> Result<::app_infra::RetentionCleanupSummary, String> {
    let policy =
        retention_policy_from_settings(settings, request.and_then(|request| request.policy));
    let context = retention_context_for_app(core, infra).await;
    Arc::clone(infra)
        .capture_retention()
        .preview_cleanup(
            app_retention_policy(policy),
            local_now_for_retention(),
            &context,
        )
        .await
        .map_err(|error| format!("failed to preview retention cleanup: {error}"))
}

pub async fn run_retention_cleanup_now(
    core: &AppCore,
    infra: &AppInfraState,
    settings: &crate::recording_settings::RecordingSettingsState,
) -> Result<::app_infra::RetentionCleanupSummary, String> {
    let policy = retention_policy_from_settings(settings, None);
    let context = retention_context_for_app(core, infra).await;
    let summary = Arc::clone(infra)
        .capture_retention()
        .run_cleanup(
            app_retention_policy(policy),
            local_now_for_retention(),
            &context,
        )
        .await
        .map_err(|error| format!("failed to run retention cleanup: {error}"))?;
    if summary.deleted_capture_segments > 0 {
        let _ = frame_preview::clear_scrub_preview_cache_for_video_paths(
            core,
            &summary.deleted_capture_segment_media_paths,
        );
    }
    if summary.deleted_frames > 0
        || summary.deleted_audio_segments > 0
        || summary.deleted_capture_segments > 0
    {
        core.host()
            .emit(CoreEvent::TimelineDataChanged(TimelineDataChangedPayload {
                reason: "retention".to_string(),
                deleted_before: summary.cutoff_ended_before.clone(),
                started_at: None,
                ended_at: None,
                deleted_frame_ids: summary.deleted_frame_ids.clone(),
                deleted_audio_segment_ids: summary.deleted_audio_segment_ids.clone(),
            }));
    }
    Ok(summary)
}

pub async fn delete_recent_capture_from_core(
    core: &Arc<AppCore>,
    window_seconds: i64,
) -> Result<DeleteRecentCaptureSummaryDto, String> {
    let infra = core
        .try_state::<AppInfraState>()
        .ok_or_else(|| "App infra is not initialized".to_string())?;
    delete_recent_capture_inner(core, &infra, window_seconds).await
}

pub(super) async fn delete_recent_capture_inner(
    core: &Arc<AppCore>,
    infra: &::app_infra::AppInfra,
    window_seconds: i64,
) -> Result<DeleteRecentCaptureSummaryDto, String> {
    let window_seconds = validate_delete_recent_window(window_seconds)?;

    let session_before_delete =
        crate::native_capture::control::current_native_capture_session(core);
    let should_resume_after_boundary =
        should_resume_delete_recent_capture_boundary(&session_before_delete);
    if should_resume_after_boundary {
        crate::native_capture::control::pause_native_capture_from_core(core).map_err(|error| {
            format!(
                "failed to create recording boundary before deletion: {}",
                error.message
            )
        })?;
    }
    let (started_at, ended_at) =
        format_delete_recent_capture_window(window_seconds, OffsetDateTime::now_utc())?;

    let deletion = match delete_recent_capture_rows(infra, &started_at, &ended_at).await {
        Ok(deletion) => deletion,
        Err(error) => {
            return complete_delete_recent_capture_boundary(
                should_resume_after_boundary,
                Err(error),
                || {
                    crate::native_capture::control::resume_native_capture_from_core(core)
                        .map(|_| ())
                        .map_err(|error| error.message)
                },
            );
        }
    };
    let retention_context = retention_context_for_app(core, infra).await;
    if !deletion.capture_segment_media_paths.is_empty() {
        let _ = frame_preview::clear_scrub_preview_cache_for_video_paths(
            core,
            &deletion.capture_segment_media_paths,
        );
    }

    core.host()
        .emit(CoreEvent::TimelineDataChanged(TimelineDataChangedPayload {
            reason: "delete_recent_capture".to_string(),
            deleted_before: None,
            started_at: Some(started_at.clone()),
            ended_at: Some(ended_at.clone()),
            deleted_frame_ids: deletion.frame_ids.clone(),
            deleted_audio_segment_ids: deletion.audio_segment_ids.clone(),
        }));

    // ADR 0029: the derived **User Context** cascade already ran atomically with
    // the raw delete inside `delete_recent_capture_rows` (so a failure rolls back
    // the whole privacy delete rather than leaving the dossier describing deleted
    // content). All that's left here is to notify any open Insights surface.
    if deletion.deleted_user_context_activities > 0 || deletion.deleted_user_context_conclusions > 0
    {
        core.host().emit(CoreEvent::UserContextChanged);
    }

    let post_delete_result = async {
        let file_delete_errors =
            delete_recent_capture_files_and_tombstone(infra, &deletion, &retention_context).await?;
        let pending_file_tombstones = infra
            .capture_retention()
            .pending_file_tombstone_count()
            .await
            .map_err(|error| format!("failed to count pending file tombstones: {error}"))?;
        Ok((file_delete_errors, pending_file_tombstones))
    }
    .await;
    let (file_delete_errors, pending_file_tombstones) = complete_delete_recent_capture_boundary(
        should_resume_after_boundary,
        post_delete_result,
        || {
            crate::native_capture::control::resume_native_capture_from_core(core)
                .map(|_| ())
                .map_err(|error| error.message)
        },
    )?;

    Ok(DeleteRecentCaptureSummaryDto {
        window_seconds,
        started_at,
        ended_at,
        deleted_capture_segments: deletion.deleted_capture_segments,
        deleted_frames: deletion.deleted_frames,
        deleted_audio_segments: deletion.deleted_audio_segments,
        deleted_processing_jobs: deletion.deleted_processing_jobs,
        deleted_processing_results: deletion.deleted_processing_results,
        deleted_background_jobs: deletion.deleted_background_jobs,
        deleted_frame_batches: deletion.deleted_frame_batches,
        deleted_search_documents: deletion.deleted_search_documents,
        pending_file_tombstones,
        file_delete_errors,
    })
}

pub async fn delete_recent_capture(
    request: DeleteRecentCaptureRequestDto,
    core: &Arc<AppCore>,
    infra: &AppInfraState,
) -> Result<DeleteRecentCaptureSummaryDto, String> {
    delete_recent_capture_inner(core, infra, request.window_seconds).await
}
