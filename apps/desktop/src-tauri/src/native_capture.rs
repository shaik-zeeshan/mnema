//! The Tauri adapters over `app_core::native_capture`: one-line commands. The
//! OS notifier registrations are app-core's (`native_capture::os_notifiers`);
//! the macOS banner for the not-recording reminder is host-side
//! (`macos_notification`).

#[cfg(target_os = "macos")]
pub(crate) mod macos_notification;

use std::sync::Arc;

use app_core::native_capture::app_candidates::{
    self, AppIconResolution, BrowserUrlSupportResponse, CheckBrowserUrlSupportRequest,
    ResolveAppIconsRequest,
};
use app_core::native_capture::app_notifications::{self, AppNotification};
use app_core::native_capture::lifecycle::NativeCaptureState;
use app_core::native_capture::microphone::{bounded_clip, controller};
use app_core::native_capture::permissions::{
    self, BrowserUrlAccessibilityStatus, SystemAudioAccessHint,
};
use app_core::native_capture::{control, not_recording_reminder, settings_update};
use app_core::privacy_redaction_sources::PrivacyAppCandidate;
use app_core::AppCore;
use capture_types::{
    CaptureErrorResponse, CapturePermissionsResponse, CaptureSupportResponse, IdleDebugInfo,
    MicrophoneControllerState, NativeCaptureSessionResponse, RecordingSettings,
    RecordingSettingsDomainUpdateResponse, StartNativeCaptureRequest, UpdateAccessSettingsRequest,
    UpdateAiRuntimeSettingsRequest, UpdateCaptureSourceSettingsRequest,
    UpdateCaptureTimingSettingsRequest, UpdateDeveloperSettingsRequest,
    UpdateDisplaySettingsRequest, UpdateInactivitySettingsRequest, UpdateMetadataSettingsRequest,
    UpdateMicrophoneControllerRequest, UpdateProcessingSettingsRequest,
    UpdateRecordingSettingsRequest, UpdateStorageSettingsRequest, UpdateUserContextSettingsRequest,
    UpdateVideoSettingsRequest,
};
use tauri::State;

type Core<'a> = State<'a, Arc<AppCore>>;

#[tauri::command]
pub async fn list_privacy_app_candidates() -> Result<Vec<PrivacyAppCandidate>, String> {
    app_candidates::list_privacy_app_candidates().await
}

#[tauri::command]
pub async fn resolve_app_icons(
    core: Core<'_>,
    request: ResolveAppIconsRequest,
) -> Result<Vec<AppIconResolution>, String> {
    app_candidates::resolve_app_icons(&core, request).await
}

#[tauri::command]
pub async fn check_browser_url_support(
    request: CheckBrowserUrlSupportRequest,
) -> Result<BrowserUrlSupportResponse, String> {
    app_candidates::check_browser_url_support(request).await
}

#[tauri::command]
pub fn get_capture_support() -> CaptureSupportResponse {
    permissions::get_capture_support()
}

#[tauri::command]
pub fn get_system_audio_access_hint(core: Core<'_>) -> SystemAudioAccessHint {
    permissions::get_system_audio_access_hint(&core)
}

#[tauri::command]
pub fn get_capture_permissions(core: Core<'_>) -> CapturePermissionsResponse {
    permissions::get_capture_permissions(&core)
}

#[tauri::command]
pub async fn request_capture_permission(
    kind: String,
    core: Core<'_>,
) -> Result<CapturePermissionsResponse, String> {
    permissions::request_capture_permission(kind, &core).await
}

#[tauri::command]
pub fn open_capture_privacy_settings(kind: String, core: Core<'_>) -> Result<(), String> {
    permissions::open_capture_privacy_settings(kind, &core)
}

#[tauri::command]
pub fn get_browser_url_accessibility_status() -> BrowserUrlAccessibilityStatus {
    permissions::get_browser_url_accessibility_status()
}

#[tauri::command]
pub fn request_browser_url_accessibility() -> BrowserUrlAccessibilityStatus {
    permissions::request_browser_url_accessibility()
}

#[tauri::command]
pub fn open_browser_url_accessibility_settings(core: Core<'_>) -> Result<(), String> {
    permissions::open_browser_url_accessibility_settings(&core)
}

/// `(async)`: the free-space read can block on a slow volume, so keep it off
/// the main thread.
#[tauri::command(async)]
pub fn get_capture_health(
    core: Core<'_>,
) -> app_core::native_capture::capture_health::CaptureHealthDto {
    app_core::native_capture::capture_health::capture_health_snapshot(
        &core.state::<NativeCaptureState>(),
        &core,
    )
}

#[tauri::command]
pub fn get_idle_debug(core: Core<'_>) -> IdleDebugInfo {
    app_core::native_capture::activity::get_idle_debug(&core.state::<NativeCaptureState>())
}

#[tauri::command]
pub fn get_app_notifications(core: Core<'_>) -> Vec<AppNotification> {
    app_notifications::get_app_notifications(&core)
}

#[tauri::command]
pub fn clear_app_notification(id: String, core: Core<'_>) -> Vec<AppNotification> {
    app_notifications::clear_app_notification(id, &core)
}

#[tauri::command]
pub fn clear_app_notifications(core: Core<'_>) -> Vec<AppNotification> {
    app_notifications::clear_app_notifications(&core)
}

#[tauri::command]
pub fn snooze_not_recording_reminder(until_unix_ms: u64, core: Core<'_>) {
    not_recording_reminder::snooze_not_recording_reminder(until_unix_ms, &core)
}

#[tauri::command]
pub fn get_microphone_controller_state(
    core: Core<'_>,
) -> Result<MicrophoneControllerState, CaptureErrorResponse> {
    controller::get_microphone_controller_state(&core)
}

#[tauri::command]
pub async fn update_microphone_controller(
    request: UpdateMicrophoneControllerRequest,
    core: Core<'_>,
) -> Result<MicrophoneControllerState, CaptureErrorResponse> {
    controller::update_microphone_controller_async(request, &core).await
}

#[tauri::command]
pub fn get_microphone_activity_level() -> Option<f32> {
    bounded_clip::get_microphone_activity_level()
}

#[tauri::command]
pub async fn start_system_audio_level_probe(core: Core<'_>) -> Result<(), String> {
    bounded_clip::start_system_audio_level_probe(&core).await
}

#[tauri::command]
pub fn get_system_audio_probe_level() -> Option<f32> {
    bounded_clip::get_system_audio_probe_level()
}

#[tauri::command]
pub async fn record_bounded_microphone_clip(
    duration_ms: u64,
    core: Core<'_>,
) -> Result<String, CaptureErrorResponse> {
    bounded_clip::record_bounded_microphone_clip(duration_ms, &core).await
}

#[tauri::command]
pub fn get_recording_settings(core: Core<'_>) -> RecordingSettings {
    settings_update::get_recording_settings(&core)
}

#[tauri::command]
pub fn update_recording_settings(
    request: UpdateRecordingSettingsRequest,
    core: Core<'_>,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    settings_update::update_recording_settings(request, &core)
}

#[tauri::command]
pub async fn update_capture_source_settings(
    request: UpdateCaptureSourceSettingsRequest,
    core: Core<'_>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    settings_update::update_capture_source_settings(request, &core).await
}

#[tauri::command]
pub fn update_capture_timing_settings(
    request: UpdateCaptureTimingSettingsRequest,
    core: Core<'_>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    settings_update::update_capture_timing_settings(request, &core)
}

#[tauri::command]
pub fn update_video_settings(
    request: UpdateVideoSettingsRequest,
    core: Core<'_>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    settings_update::update_video_settings(request, &core)
}

#[tauri::command]
pub fn update_storage_settings(
    request: UpdateStorageSettingsRequest,
    core: Core<'_>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    settings_update::update_storage_settings(request, &core)
}

#[tauri::command]
pub fn update_display_settings(
    request: UpdateDisplaySettingsRequest,
    core: Core<'_>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    settings_update::update_display_settings(request, &core)
}

#[tauri::command]
pub fn update_metadata_settings(
    request: UpdateMetadataSettingsRequest,
    core: Core<'_>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    settings_update::update_metadata_settings(request, &core)
}

#[tauri::command]
pub fn update_inactivity_settings(
    request: UpdateInactivitySettingsRequest,
    core: Core<'_>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    settings_update::update_inactivity_settings(request, &core)
}

#[tauri::command]
pub fn update_processing_settings(
    request: UpdateProcessingSettingsRequest,
    core: Core<'_>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    settings_update::update_processing_settings(request, &core)
}

#[tauri::command]
pub fn update_developer_settings(
    request: UpdateDeveloperSettingsRequest,
    core: Core<'_>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    settings_update::update_developer_settings(request, &core)
}

#[tauri::command]
pub fn update_access_settings(
    request: UpdateAccessSettingsRequest,
    core: Core<'_>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    settings_update::update_access_settings(request, &core)
}

#[tauri::command]
pub fn update_ai_runtime_settings(
    request: UpdateAiRuntimeSettingsRequest,
    core: Core<'_>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    settings_update::update_ai_runtime_settings(request, &core)
}

#[tauri::command]
pub fn update_user_context_settings(
    request: UpdateUserContextSettingsRequest,
    core: Core<'_>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    settings_update::update_user_context_settings(request, &core)
}

#[tauri::command]
pub fn update_semantic_search_settings(
    request: capture_types::UpdateSemanticSearchSettingsRequest,
    core: Core<'_>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    settings_update::update_semantic_search_settings(request, &core)
}

#[tauri::command]
pub fn start_native_capture(
    request: StartNativeCaptureRequest,
    core: Core<'_>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    control::start_native_capture(request, &core)
}

#[tauri::command]
pub fn pause_native_capture(
    core: Core<'_>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    control::pause_native_capture_from_core(&core)
}

#[tauri::command]
pub fn resume_native_capture(
    core: Core<'_>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    control::resume_native_capture_from_core(&core)
}

/// Stopping does a synchronous capture-index write; app-core runs it on the
/// blocking pool so this main-thread command stays responsive.
#[tauri::command]
pub async fn stop_native_capture(
    core: Core<'_>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    control::stop_native_capture(&core).await
}
