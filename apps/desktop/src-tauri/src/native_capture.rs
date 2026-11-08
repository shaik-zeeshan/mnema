//! The Tauri adapters over `app_core::native_capture`: one-line commands, plus
//! the native notifier registrations (NSWorkspace sleep/wake, screens
//! sleep/wake, focus/app changes, the Core Graphics display-reconfiguration
//! callback) that call app-core's handlers. The macOS banner for the
//! not-recording reminder is host-side too (`macos_notification`).

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
use tauri::{Manager, State};

type Core<'a> = State<'a, Arc<AppCore>>;

#[cfg(target_os = "macos")]
pub fn start_metadata_notifier(app_handle: tauri::AppHandle) {
    use app_core::native_capture::privacy::PrivacyRefreshReason;
    use cidre::ns;

    let core = app_handle.state::<Arc<AppCore>>().inner().clone();
    let mut center = ns::Workspace::shared().notification_center();
    let mut guards = Vec::new();
    for (notification, reason) in [
        (
            ns::workspace::notification::did_activate_app(),
            PrivacyRefreshReason::WorkspaceFocusChanged,
        ),
        (
            ns::workspace::notification::did_deactivate_app(),
            PrivacyRefreshReason::WorkspaceFocusChanged,
        ),
        (
            ns::workspace::notification::active_space_did_change(),
            PrivacyRefreshReason::WorkspaceFocusChanged,
        ),
        (
            ns::workspace::notification::did_launch_app(),
            PrivacyRefreshReason::WorkspaceAppChanged,
        ),
        (
            ns::workspace::notification::did_terminate_app(),
            PrivacyRefreshReason::WorkspaceAppChanged,
        ),
        (
            ns::workspace::notification::did_hide_app(),
            PrivacyRefreshReason::WorkspaceAppChanged,
        ),
        (
            ns::workspace::notification::did_unhide_app(),
            PrivacyRefreshReason::WorkspaceAppChanged,
        ),
    ] {
        guards.push(center.add_observer_guard(notification, None, None, {
            let core = core.clone();
            move |_notification| {
                app_core::native_capture::privacy::refresh::request_privacy_filter_refresh(
                    &core, reason,
                );
            }
        }));
    }

    core.state::<app_core::native_capture::metadata::MetadataNotifierState>()
        .replace(guards);
}

#[cfg(not(target_os = "macos"))]
pub fn start_metadata_notifier(_app_handle: tauri::AppHandle) {}

#[cfg(target_os = "macos")]
pub fn start_system_wake_notifier(app_handle: tauri::AppHandle) {
    use app_core::native_capture::system_wake::{self, SystemWakeNotifierState};
    use cidre::ns;

    let core = app_handle.state::<Arc<AppCore>>().inner().clone();
    let mut center = ns::Workspace::shared().notification_center();
    let will_sleep_guard =
        center.add_observer_guard(ns::workspace::notification::will_sleep(), None, None, {
            let core = core.clone();
            move |_notification| {
                system_wake::handle_system_will_sleep(&core);
            }
        });
    let did_wake_guard =
        center.add_observer_guard(ns::workspace::notification::did_wake(), None, None, {
            let core = core.clone();
            move |_notification| {
                system_wake::recover_screen_capture_after_system_wake(core.clone());
            }
        });
    let screens_did_sleep_guard = center.add_observer_guard(
        ns::workspace::notification::screens_did_sleep(),
        None,
        None,
        {
            let core = core.clone();
            move |_notification| {
                system_wake::handle_screens_did_sleep(&core);
            }
        },
    );
    let screens_did_wake_guard = center.add_observer_guard(
        ns::workspace::notification::screens_did_wake(),
        None,
        None,
        {
            let core = core.clone();
            move |_notification| {
                system_wake::handle_screens_did_wake(&core);
            }
        },
    );

    let notifier_state = core.state::<SystemWakeNotifierState>();
    let mut notifier_slot = notifier_state
        .0
        .lock()
        .expect("system wake notifier state poisoned");
    notifier_slot.clear();
    notifier_slot.push(will_sleep_guard);
    notifier_slot.push(did_wake_guard);
    notifier_slot.push(screens_did_sleep_guard);
    notifier_slot.push(screens_did_wake_guard);
}

#[cfg(not(target_os = "macos"))]
pub fn start_system_wake_notifier(_app_handle: tauri::AppHandle) {}

// `NSWorkspaceDidWake` is only posted on a *full* wake; dark wakes, Power Nap,
// and "Wake from Deep Idle" never post it. The display reconfiguration callback
// is the polling-free re-arm signal (`app_core::native_capture::system_wake`).
#[cfg(target_os = "macos")]
pub fn start_display_reconfiguration_notifier(app_handle: tauri::AppHandle) {
    use app_core::native_capture::system_wake::{
        display_reconfiguration_callback, set_display_reconfiguration_recovery_core,
        DisplayReconfigurationCallbackGuard, DisplayReconfigurationNotifierState,
    };
    use core_graphics::base::{kCGErrorSuccess, CGError};

    let core = app_handle.state::<Arc<AppCore>>().inner().clone();
    // The C callback can't capture state, so stash the core where it can reach
    // it. Set before registering so no early callback races a missing core.
    set_display_reconfiguration_recovery_core(&core);

    let error: CGError = unsafe {
        core_graphics::display::CGDisplayRegisterReconfigurationCallback(
            display_reconfiguration_callback,
            std::ptr::null(),
        )
    };
    if error != kCGErrorSuccess {
        app_core::debug_log::log_warn(format!(
            "failed to register display reconfiguration callback for wake recovery (CGError {error}); \
             relying on NSWorkspaceDidWake fallback"
        ));
        return;
    }

    let notifier_state = core.state::<DisplayReconfigurationNotifierState>();
    let mut slot = notifier_state
        .0
        .lock()
        .expect("display reconfiguration notifier state poisoned");
    *slot = Some(DisplayReconfigurationCallbackGuard);
}

#[cfg(not(target_os = "macos"))]
pub fn start_display_reconfiguration_notifier(_app_handle: tauri::AppHandle) {}

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
