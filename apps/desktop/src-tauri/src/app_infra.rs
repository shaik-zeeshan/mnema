//! The Tauri side of `app_core::app_infra`: `initialize` (which also hands the
//! shared states to Tauri's state for the code still reading them there), the
//! command wrappers, and the startup and retention pieces that read the live
//! capture session.

use std::sync::Arc;

use app_core::app_infra::{self as core_infra, *};
use app_core::{AppCore, AppInfraState};
use tauri::{Manager, State};

pub mod quick_recall;
pub mod retention;
mod startup;
#[cfg(test)]
mod tests;
mod workers;

pub(crate) use retention::delete_recent_capture_from_app_handle;
pub(crate) use startup::{
    run_deferred_startup_blocking, startup_panicked, startup_report, StartupReport,
};

/// Opens app-infra on the core, then registers the same states with Tauri for
/// the commands and capture code that still read them there (S3).
pub fn initialize(app: &mut tauri::App) -> Result<(), AppInfraInitializeError> {
    let core = app.state::<Arc<AppCore>>().inner().clone();
    core_infra::initialize(&core)?;

    if !app.manage(core.state::<AppInfraState>().as_ref().clone()) {
        app_core::debug_log::log_error(
            "app infrastructure state was already initialized; refusing duplicate setup",
        );
        return Err(AppInfraInitializeError::Other(
            "app infrastructure state was already initialized".to_string(),
        ));
    }

    if !app.manage(core.state::<BackgroundWorkersState>().as_ref().clone()) {
        app_core::debug_log::log_error(
            "background workers state was already initialized; refusing duplicate setup",
        );
        return Err(AppInfraInitializeError::Other(
            "background workers state was already initialized".to_string(),
        ));
    }

    let deepgram = core.state::<app_core::transcription_deepgram::DeepgramAuthStatusState>();
    if !app.manage(app_core::transcription_deepgram::DeepgramAuthStatusState(
        Arc::clone(&deepgram.0),
    )) {
        app_core::debug_log::log_error(
            "deepgram auth status state was already initialized; refusing duplicate setup",
        );
        return Err(AppInfraInitializeError::Other(
            "deepgram auth status state was already initialized".to_string(),
        ));
    }
    Ok(())
}

#[tauri::command]
pub fn get_storage_location(core: State<'_, Arc<AppCore>>) -> Result<String, String> {
    core_infra::get_storage_location(&core)
}

#[tauri::command]
pub async fn get_app_infra_status(
    state: State<'_, AppInfraState>,
) -> Result<::app_infra::AppInfraStatus, String> {
    core_infra::get_app_infra_status(&state).await
}

#[tauri::command]
pub async fn get_retention_cleanup_status(
    infra: State<'_, AppInfraState>,
    settings: State<'_, app_core::recording_settings::RecordingSettingsState>,
) -> Result<::app_infra::RetentionCleanupSummary, String> {
    core_infra::get_retention_cleanup_status(&infra, &settings).await
}

#[tauri::command]
pub async fn list_app_jobs(state: State<'_, AppInfraState>) -> Result<Vec<AppJobDto>, String> {
    core_infra::list_app_jobs(&state).await
}

#[tauri::command]
pub async fn get_app_job(
    request: GetAppJobRequest,
    state: State<'_, AppInfraState>,
) -> Result<Option<AppJobDto>, String> {
    core_infra::get_app_job(request, &state).await
}

#[tauri::command]
pub async fn debug_insert_frame_and_enqueue_processing_job(
    request: DebugInsertFrameAndEnqueueProcessingJobRequest,
    state: State<'_, AppInfraState>,
    settings: State<'_, app_core::recording_settings::RecordingSettingsState>,
) -> Result<FrameProcessingJobDto, String> {
    core_infra::debug_insert_frame_and_enqueue_processing_job(request, &state, &settings).await
}

#[tauri::command]
pub async fn debug_insert_frame_and_enqueue_ocr(
    request: DebugInsertFrameAndEnqueueOcrRequest,
    state: State<'_, AppInfraState>,
    settings: State<'_, app_core::recording_settings::RecordingSettingsState>,
) -> Result<FrameProcessingJobDto, String> {
    core_infra::debug_insert_frame_and_enqueue_ocr(request, &state, &settings).await
}

#[tauri::command]
pub async fn reprocess_captured_frame_ocr(
    request: ReprocessCapturedFrameOcrRequest,
    state: State<'_, AppInfraState>,
    settings: State<'_, app_core::recording_settings::RecordingSettingsState>,
) -> Result<CapturedFrameReprocessingResultDto, String> {
    core_infra::reprocess_captured_frame_ocr(request, &state, &settings).await
}

#[tauri::command]
pub async fn reprocess_audio_segment_transcription(
    request: ReprocessAudioSegmentTranscriptionRequest,
    state: State<'_, AppInfraState>,
    core: State<'_, Arc<AppCore>>,
) -> Result<AudioSegmentTranscriptionReprocessingResultDto, String> {
    core_infra::reprocess_audio_segment_transcription(request, &state, &core).await
}

#[tauri::command]
pub async fn reprocess_audio_segment_speaker_analysis(
    request: ReprocessAudioSegmentTranscriptionRequest,
    state: State<'_, AppInfraState>,
    core: State<'_, Arc<AppCore>>,
) -> Result<AudioSegmentSpeakerAnalysisReprocessingResultDto, String> {
    core_infra::reprocess_audio_segment_speaker_analysis(request, &state, &core).await
}

#[tauri::command]
pub async fn reprocess_system_audio_speech_activity(
    request: ReprocessAudioSegmentTranscriptionRequest,
    state: State<'_, AppInfraState>,
    core: State<'_, Arc<AppCore>>,
) -> Result<SystemAudioSpeechActivityReprocessingResultDto, String> {
    core_infra::reprocess_system_audio_speech_activity(request, &state, &core).await
}

#[tauri::command]
pub async fn list_frames(
    request: Option<ListFramesRequest>,
    state: State<'_, AppInfraState>,
) -> Result<Vec<FrameDto>, String> {
    core_infra::list_frames(request, &state).await
}

#[tauri::command]
pub async fn list_frame_summaries_in_range(
    request: FrameCapturedAtRangeRequest,
    state: State<'_, AppInfraState>,
) -> Result<Vec<FrameSummaryDto>, String> {
    core_infra::list_frame_summaries_in_range(request, &state).await
}

#[tauri::command]
pub async fn get_latest_frame_in_range(
    request: FrameCapturedAtRangeRequest,
    state: State<'_, AppInfraState>,
) -> Result<Option<FrameDto>, String> {
    core_infra::get_latest_frame_in_range(request, &state).await
}

#[tauri::command]
pub async fn list_audio_segments(
    request: ListAudioSegmentsRequest,
    state: State<'_, AppInfraState>,
) -> Result<Vec<AudioSegmentDto>, String> {
    core_infra::list_audio_segments(request, &state).await
}

#[tauri::command]
pub async fn get_audio_segment_media(
    request: GetAudioSegmentMediaRequest,
    state: State<'_, AppInfraState>,
) -> Result<AudioSegmentMediaDto, String> {
    core_infra::get_audio_segment_media(request, &state).await
}

#[tauri::command]
pub async fn get_audio_segment_waveform_peaks(
    request: GetAudioSegmentWaveformPeaksRequest,
    state: State<'_, AppInfraState>,
) -> Result<Vec<f32>, String> {
    core_infra::get_audio_segment_waveform_peaks(request, &state).await
}

#[tauri::command]
pub async fn get_audio_segment(
    request: GetAudioSegmentRequest,
    state: State<'_, AppInfraState>,
) -> Result<Option<AudioSegmentDto>, String> {
    core_infra::get_audio_segment(request, &state).await
}

#[tauri::command]
pub async fn get_frame(
    request: GetFrameRequest,
    state: State<'_, AppInfraState>,
) -> Result<Option<FrameDto>, String> {
    core_infra::get_frame(request, &state).await
}

#[tauri::command]
pub async fn open_captured_url(
    frame_id: i64,
    core: State<'_, Arc<AppCore>>,
    state: State<'_, AppInfraState>,
) -> Result<bool, String> {
    core_infra::open_captured_url(frame_id, &core, &state).await
}

#[tauri::command]
pub async fn get_nearest_earlier_equivalent_frame(
    request: GetNearestEarlierEquivalentFrameRequest,
    state: State<'_, AppInfraState>,
) -> Result<Option<FrameDto>, String> {
    core_infra::get_nearest_earlier_equivalent_frame(request, &state).await
}

#[tauri::command]
pub async fn get_earliest_earlier_equivalent_frame(
    request: GetEarliestEarlierEquivalentFrameRequest,
    state: State<'_, AppInfraState>,
) -> Result<Option<FrameDto>, String> {
    core_infra::get_earliest_earlier_equivalent_frame(request, &state).await
}

#[tauri::command]
pub async fn get_timeline_window_around_frame(
    request: GetTimelineWindowAroundFrameRequest,
    state: State<'_, AppInfraState>,
) -> Result<FocusedFrameWindowDto, String> {
    core_infra::get_timeline_window_around_frame(request, &state).await
}

#[tauri::command]
pub async fn search_capture(
    request: SearchCaptureRequest,
    core: State<'_, Arc<AppCore>>,
    state: State<'_, AppInfraState>,
    query_embedder: State<'_, app_core::semantic_search_query::SemanticQueryEmbedderState>,
) -> Result<SearchCaptureResponseDto, String> {
    core_infra::search_capture(request, &core, &state, &query_embedder).await
}

#[tauri::command]
pub async fn list_processing_jobs(
    request: ListProcessingJobsRequest,
    state: State<'_, AppInfraState>,
) -> Result<Vec<ProcessingJobDto>, String> {
    core_infra::list_processing_jobs(request, &state).await
}

#[tauri::command]
pub async fn get_processing_job(
    request: GetProcessingJobRequest,
    state: State<'_, AppInfraState>,
) -> Result<Option<ProcessingJobDto>, String> {
    core_infra::get_processing_job(request, &state).await
}

#[tauri::command]
pub async fn retry_processing_job_now(
    job_id: i64,
    state: State<'_, AppInfraState>,
) -> Result<ProcessingJobDto, String> {
    core_infra::retry_processing_job_now(job_id, &state).await
}

#[tauri::command]
pub async fn get_processing_result(
    request: GetProcessingResultRequest,
    state: State<'_, AppInfraState>,
) -> Result<Option<ProcessingResultDto>, String> {
    core_infra::get_processing_result(request, &state).await
}

#[tauri::command]
pub async fn list_processing_results(
    request: ListProcessingResultsRequest,
    state: State<'_, AppInfraState>,
) -> Result<Vec<ProcessingResultDto>, String> {
    core_infra::list_processing_results(request, &state).await
}

#[tauri::command]
pub async fn list_speaker_turns(
    request: ListSpeakerTurnsRequest,
    state: State<'_, AppInfraState>,
) -> Result<Vec<SpeakerTurnDto>, String> {
    core_infra::list_speaker_turns(request, &state).await
}

#[tauri::command]
pub async fn list_person_profiles(
    state: State<'_, AppInfraState>,
) -> Result<Vec<PersonProfileDto>, String> {
    core_infra::list_person_profiles(&state).await
}

#[tauri::command]
pub async fn list_searchable_apps(
    state: State<'_, AppInfraState>,
) -> Result<Vec<::app_infra::SearchableApp>, String> {
    core_infra::list_searchable_apps(&state).await
}

#[tauri::command]
pub async fn create_person_profile(
    request: CreatePersonProfileRequest,
    state: State<'_, AppInfraState>,
) -> Result<PersonProfileDto, String> {
    core_infra::create_person_profile(request, &state).await
}

#[tauri::command]
pub async fn delete_person_profile(
    request: DeletePersonProfileRequest,
    state: State<'_, AppInfraState>,
) -> Result<(), String> {
    core_infra::delete_person_profile(request, &state).await
}

#[tauri::command]
pub async fn list_speaker_clusters(
    request: ListSpeakerClustersRequest,
    state: State<'_, AppInfraState>,
) -> Result<Vec<SpeakerClusterDto>, String> {
    core_infra::list_speaker_clusters(request, &state).await
}

#[tauri::command]
pub async fn name_speaker_cluster(
    request: NameSpeakerClusterRequest,
    state: State<'_, AppInfraState>,
) -> Result<SpeakerClusterDto, String> {
    core_infra::name_speaker_cluster(request, &state).await
}

#[tauri::command]
pub async fn link_speaker_cluster_to_person(
    request: LinkSpeakerClusterRequest,
    state: State<'_, AppInfraState>,
) -> Result<SpeakerClusterDto, String> {
    core_infra::link_speaker_cluster_to_person(request, &state).await
}

#[tauri::command]
pub async fn unlink_speaker_cluster_from_person(
    request: SpeakerClusterRequest,
    state: State<'_, AppInfraState>,
) -> Result<SpeakerClusterDto, String> {
    core_infra::unlink_speaker_cluster_from_person(request, &state).await
}

#[tauri::command]
pub async fn confirm_speaker_recognition_suggestion(
    request: ConfirmSpeakerSuggestionRequest,
    state: State<'_, AppInfraState>,
) -> Result<SpeakerClusterDto, String> {
    core_infra::confirm_speaker_recognition_suggestion(request, &state).await
}

#[tauri::command]
pub async fn reject_speaker_recognition_suggestion(
    request: SpeakerClusterRequest,
    state: State<'_, AppInfraState>,
) -> Result<SpeakerClusterDto, String> {
    core_infra::reject_speaker_recognition_suggestion(request, &state).await
}

#[tauri::command]
pub async fn merge_speaker_clusters(
    request: MergeSpeakerClustersRequest,
    state: State<'_, AppInfraState>,
) -> Result<SpeakerClusterDto, String> {
    core_infra::merge_speaker_clusters(request, &state).await
}

#[tauri::command]
pub async fn move_speaker_turn_to_cluster(
    request: MoveSpeakerTurnRequest,
    state: State<'_, AppInfraState>,
) -> Result<SpeakerTurnDto, String> {
    core_infra::move_speaker_turn_to_cluster(request, &state).await
}

#[tauri::command]
pub async fn get_frame_preview(
    request: frame_preview::GetFramePreviewRequest,
    core: State<'_, Arc<AppCore>>,
) -> Result<Option<frame_preview::FramePreviewDto>, String> {
    frame_preview::get_frame_preview(&core, request).await
}

#[tauri::command]
pub async fn get_frame_scrub_previews(
    request: frame_preview::GetFrameScrubPreviewsRequest,
    core: State<'_, Arc<AppCore>>,
) -> Result<frame_preview::FrameScrubPreviewsDto, String> {
    frame_preview::get_frame_scrub_previews(&core, request).await
}

#[tauri::command]
pub async fn get_scrub_preview_availability(
    request: frame_preview::GetScrubPreviewAvailabilityRequest,
    core: State<'_, Arc<AppCore>>,
) -> Result<frame_preview::ScrubPreviewAvailabilityDto, String> {
    frame_preview::get_scrub_preview_availability(&core, request).await
}

#[tauri::command]
pub fn get_scrub_preview_cache_status(
    core: State<'_, Arc<AppCore>>,
) -> Result<frame_preview::ScrubPreviewCacheStatusDto, String> {
    frame_preview::get_scrub_preview_cache_status(&core)
}

#[tauri::command]
pub fn clear_scrub_preview_cache(
    core: State<'_, Arc<AppCore>>,
) -> Result<frame_preview::ScrubPreviewCacheStatusDto, String> {
    frame_preview::clear_scrub_preview_cache(&core)
}

#[tauri::command]
pub fn cancel_active_frame_preview_video_requests(
    core: State<'_, Arc<AppCore>>,
) -> Result<usize, String> {
    frame_preview::cancel_active_frame_preview_video_requests(&core)
}
