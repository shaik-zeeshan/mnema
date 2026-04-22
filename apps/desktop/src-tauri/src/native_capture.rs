mod activity;
mod microphone;
mod runtime;
mod segments;
#[cfg(test)]
mod tests;

use crate::native_capture_settings::{
    default_recording_settings, load_recording_settings_from_disk, persist_recording_settings,
    validate_recording_settings,
};
use capture_microphone as microphone_capture;
use capture_types::{
    CaptureErrorResponse, CaptureOutputFiles, CapturePermissionState, CapturePermissions,
    CapturePermissionsResponse, CaptureSources, CaptureSupportResponse, InactivityActivityMode,
    MicrophoneControllerState, NativeCaptureDebugLogStatus, NativeCaptureSessionResponse,
    RecordingSettings, ScreenResolution, ScreenResolutionPreset, StartNativeCaptureRequest,
    UpdateMicrophoneControllerRequest, UpdateRecordingSettingsRequest, VideoBitrateMode,
    VideoBitratePreset, VideoBitrateSettings,
};
use std::sync::OnceLock;
use tauri::Manager;

pub use activity::IdleDebugInfo;
use microphone::{
    resolve_capture_microphone_device_id, should_wait_for_same_microphone_device,
    update_microphone_controller as update_microphone_controller_impl,
};
pub use microphone::{
    start_microphone_device_change_notifier, MicrophoneControllerPreferencesState,
    MicrophoneDeviceChangeNotifierState,
};
use runtime::{
    mark_runtime_session_stopped, request_segment_loop_stop, session_from_runtime,
    stopped_session_from_runtime, validate_start_request,
};
pub use runtime::{NativeCaptureState, RecordingSettingsState};
use segments::{start_capture_runtime, stop_capture_runtime};

#[derive(Debug, Clone, PartialEq, Eq)]
struct CaptureSupportSnapshot {
    platform: String,
    native_capture_supported: bool,
    supported_sources: CaptureSources,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturePermissionsSnapshot {
    screen: &'static str,
    microphone: &'static str,
    system_audio: &'static str,
}

fn capture_support_log_snapshot_state() -> &'static std::sync::Mutex<Option<CaptureSupportSnapshot>>
{
    static LAST_CAPTURE_SUPPORT_SNAPSHOT: OnceLock<
        std::sync::Mutex<Option<CaptureSupportSnapshot>>,
    > = OnceLock::new();

    LAST_CAPTURE_SUPPORT_SNAPSHOT.get_or_init(|| std::sync::Mutex::new(None))
}

fn capture_permissions_log_snapshot_state(
) -> &'static std::sync::Mutex<Option<CapturePermissionsSnapshot>> {
    static LAST_CAPTURE_PERMISSIONS_SNAPSHOT: OnceLock<
        std::sync::Mutex<Option<CapturePermissionsSnapshot>>,
    > = OnceLock::new();

    LAST_CAPTURE_PERMISSIONS_SNAPSHOT.get_or_init(|| std::sync::Mutex::new(None))
}

fn reset_capture_log_snapshots() {
    *capture_support_log_snapshot_state()
        .lock()
        .expect("capture support log snapshot poisoned") = None;
    *capture_permissions_log_snapshot_state()
        .lock()
        .expect("capture permissions log snapshot poisoned") = None;
}

fn capture_sources_from_settings(settings: &RecordingSettings) -> CaptureSources {
    CaptureSources {
        screen: settings.capture_screen,
        microphone: settings.capture_microphone,
        system_audio: settings.capture_system_audio,
    }
}

fn capture_sources_from_start_request(request: &StartNativeCaptureRequest) -> CaptureSources {
    CaptureSources {
        screen: request.capture_screen,
        microphone: request.capture_microphone,
        system_audio: request.capture_system_audio,
    }
}

fn format_capture_source_flags(sources: &CaptureSources) -> String {
    format!(
        "screen={}, microphone={}, system_audio={}",
        sources.screen, sources.microphone, sources.system_audio
    )
}

fn format_optional_capture_source_flags(sources: Option<&CaptureSources>) -> String {
    sources
        .map(format_capture_source_flags)
        .unwrap_or_else(|| "screen=unknown, microphone=unknown, system_audio=unknown".to_string())
}

fn runtime_log_session_id(runtime: &runtime::NativeCaptureRuntime) -> &str {
    runtime
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.screen.as_ref())
        .map(|session| session.session_id.as_str())
        .unwrap_or("unknown")
}

fn session_log_session_id(session: &capture_types::NativeCaptureSession) -> &str {
    session
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.screen.as_ref())
        .map(|source| source.session_id.as_str())
        .unwrap_or("unknown")
}

fn permission_state_label(state: &CapturePermissionState) -> &'static str {
    match state {
        CapturePermissionState::Granted => "granted",
        CapturePermissionState::Denied => "denied",
        CapturePermissionState::NotDetermined => "not_determined",
        CapturePermissionState::Unsupported => "unsupported",
        CapturePermissionState::Unknown => "unknown",
    }
}

fn format_screen_resolution(resolution: &ScreenResolution) -> String {
    match resolution {
        ScreenResolution::Preset { preset } => match preset {
            ScreenResolutionPreset::Original => "original".to_string(),
            ScreenResolutionPreset::P1080 => "1080p".to_string(),
            ScreenResolutionPreset::P720 => "720p".to_string(),
            ScreenResolutionPreset::P540 => "540p".to_string(),
        },
        ScreenResolution::Custom { width, height } => format!("{width}x{height}"),
    }
}

fn format_video_bitrate(settings: &VideoBitrateSettings) -> String {
    match settings.mode {
        VideoBitrateMode::Preset => {
            let preset = settings
                .preset
                .clone()
                .unwrap_or(VideoBitratePreset::Medium);
            let label = match preset {
                VideoBitratePreset::Low => "low",
                VideoBitratePreset::Medium => "medium",
                VideoBitratePreset::High => "high",
            };

            format!("preset:{label}")
        }
        VideoBitrateMode::Custom => format!("custom:{}mbps", settings.custom_mbps.unwrap_or(0)),
    }
}

fn inactivity_activity_mode_label(mode: &InactivityActivityMode) -> &'static str {
    match mode {
        InactivityActivityMode::SystemInputOnly => "system_input_only",
        InactivityActivityMode::SystemInputOrScreen => "system_input_or_screen",
        InactivityActivityMode::SystemInputOrScreenOrAudio => "system_input_or_screen_or_audio",
    }
}

fn recording_settings_overview(settings: &RecordingSettings) -> String {
    format!(
        "sources={}, auto_start={}, save_directory='{}', debug_logging={}, segment_duration_seconds={}, screen_frame_rate={}, screen_resolution={}, video_bitrate={}, pause_on_inactivity={}, idle_timeout_seconds={}, microphone_activity_sensitivity={}, system_audio_activity_sensitivity={}, activity_mode={}",
        format_capture_source_flags(&capture_sources_from_settings(settings)),
        settings.auto_start,
        settings.save_directory,
        settings.native_capture_debug_logging_enabled,
        settings.segment_duration_seconds,
        settings.screen_frame_rate,
        format_screen_resolution(&settings.screen_resolution),
        format_video_bitrate(&settings.video_bitrate),
        settings.pause_capture_on_inactivity,
        settings.idle_timeout_seconds,
        settings.microphone_activity_sensitivity,
        settings.system_audio_activity_sensitivity,
        inactivity_activity_mode_label(&settings.inactivity_activity_mode)
    )
}

fn describe_recording_settings_changes(
    previous: &RecordingSettings,
    next: &RecordingSettings,
) -> Vec<String> {
    let mut changes = Vec::new();
    let previous_sources = capture_sources_from_settings(previous);
    let next_sources = capture_sources_from_settings(next);

    if previous_sources != next_sources {
        changes.push(format!(
            "sources {} -> {}",
            format_capture_source_flags(&previous_sources),
            format_capture_source_flags(&next_sources)
        ));
    }

    if previous.auto_start != next.auto_start {
        changes.push(format!(
            "auto_start {} -> {}",
            previous.auto_start, next.auto_start
        ));
    }

    if previous.native_capture_debug_logging_enabled != next.native_capture_debug_logging_enabled {
        changes.push(format!(
            "debug_logging {} -> {}",
            previous.native_capture_debug_logging_enabled,
            next.native_capture_debug_logging_enabled
        ));
    }

    if previous.segment_duration_seconds != next.segment_duration_seconds {
        changes.push(format!(
            "segment_duration_seconds {} -> {}",
            previous.segment_duration_seconds, next.segment_duration_seconds
        ));
    }

    if previous.screen_frame_rate != next.screen_frame_rate {
        changes.push(format!(
            "screen_frame_rate {} -> {}",
            previous.screen_frame_rate, next.screen_frame_rate
        ));
    }

    if previous.screen_resolution != next.screen_resolution {
        changes.push(format!(
            "screen_resolution {} -> {}",
            format_screen_resolution(&previous.screen_resolution),
            format_screen_resolution(&next.screen_resolution)
        ));
    }

    if previous.video_bitrate != next.video_bitrate {
        changes.push(format!(
            "video_bitrate {} -> {}",
            format_video_bitrate(&previous.video_bitrate),
            format_video_bitrate(&next.video_bitrate)
        ));
    }

    if previous.pause_capture_on_inactivity != next.pause_capture_on_inactivity {
        changes.push(format!(
            "pause_on_inactivity {} -> {}",
            previous.pause_capture_on_inactivity, next.pause_capture_on_inactivity
        ));
    }

    if previous.idle_timeout_seconds != next.idle_timeout_seconds {
        changes.push(format!(
            "idle_timeout_seconds {} -> {}",
            previous.idle_timeout_seconds, next.idle_timeout_seconds
        ));
    }

    if previous.inactivity_activity_mode != next.inactivity_activity_mode {
        changes.push(format!(
            "activity_mode {} -> {}",
            inactivity_activity_mode_label(&previous.inactivity_activity_mode),
            inactivity_activity_mode_label(&next.inactivity_activity_mode)
        ));
    }

    if previous.microphone_activity_sensitivity != next.microphone_activity_sensitivity {
        changes.push(format!(
            "microphone_activity_sensitivity {} -> {}",
            previous.microphone_activity_sensitivity, next.microphone_activity_sensitivity
        ));
    }

    if previous.system_audio_activity_sensitivity != next.system_audio_activity_sensitivity {
        changes.push(format!(
            "system_audio_activity_sensitivity {} -> {}",
            previous.system_audio_activity_sensitivity, next.system_audio_activity_sensitivity
        ));
    }

    changes
}

fn format_output_file_counts(output_files: Option<&CaptureOutputFiles>) -> String {
    output_files
        .map(|output_files| {
            format!(
                "screen_files={}, microphone_files={}, system_audio_files={}",
                output_files.screen_files.len(),
                output_files.microphone_files.len(),
                output_files.system_audio_files.len()
            )
        })
        .unwrap_or_else(|| "screen_files=0, microphone_files=0, system_audio_files=0".to_string())
}

fn log_capture_support_if_changed(response: &CaptureSupportResponse) {
    let snapshot = CaptureSupportSnapshot {
        platform: response.platform.clone(),
        native_capture_supported: response.native_capture_supported,
        supported_sources: response.supported_sources.clone(),
    };
    let mut last_snapshot = capture_support_log_snapshot_state()
        .lock()
        .expect("capture support log snapshot poisoned");

    if last_snapshot.as_ref() == Some(&snapshot) {
        return;
    }

    *last_snapshot = Some(snapshot.clone());

    crate::native_capture_debug_log::log(format!(
        "observed native capture support (platform='{}', native_supported={}, supported_sources={})",
        snapshot.platform,
        snapshot.native_capture_supported,
        format_capture_source_flags(&snapshot.supported_sources)
    ));
}

fn log_capture_permissions_if_changed(permissions: &CapturePermissions) {
    let snapshot = CapturePermissionsSnapshot {
        screen: permission_state_label(&permissions.screen),
        microphone: permission_state_label(&permissions.microphone),
        system_audio: permission_state_label(&permissions.system_audio),
    };
    let mut last_snapshot = capture_permissions_log_snapshot_state()
        .lock()
        .expect("capture permissions log snapshot poisoned");

    if last_snapshot.as_ref() == Some(&snapshot) {
        return;
    }

    *last_snapshot = Some(snapshot.clone());

    crate::native_capture_debug_log::log(format!(
        "observed native capture permissions (screen={}, microphone={}, system_audio={})",
        snapshot.screen, snapshot.microphone, snapshot.system_audio
    ));
}

fn log_loaded_recording_settings(source: &str, settings: &RecordingSettings) {
    crate::native_capture_debug_log::log(format!(
        "loaded recording settings from {source} ({})",
        recording_settings_overview(settings)
    ));
}

fn log_recording_settings_changes(previous: &RecordingSettings, next: &RecordingSettings) {
    let changes = describe_recording_settings_changes(previous, next);

    if changes.is_empty() {
        return;
    }

    crate::native_capture_debug_log::log(format!(
        "updated recording settings ({})",
        changes.join(", ")
    ));
}

fn start_native_capture_inner(
    origin: &str,
    request: StartNativeCaptureRequest,
    state: tauri::State<'_, NativeCaptureState>,
    microphone_controller_preferences_state: tauri::State<'_, MicrophoneControllerPreferencesState>,
    recording_settings_state: tauri::State<'_, RecordingSettingsState>,
    app_handle: tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let incoming_sources = capture_sources_from_start_request(&request);
    let settings = recording_settings_state
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();

    let resolved_request = StartNativeCaptureRequest {
        capture_screen: settings.capture_screen,
        capture_microphone: settings.capture_microphone,
        capture_system_audio: settings.capture_system_audio,
    };
    let resolved_sources = capture_sources_from_start_request(&resolved_request);

    crate::native_capture_debug_log::log(format!(
        "attempting native capture {origin} start (incoming_sources={}, resolved_sources={}, save_directory='{}')",
        format_capture_source_flags(&incoming_sources),
        format_capture_source_flags(&resolved_sources),
        settings.save_directory
    ));

    let support = get_capture_support();
    let sources = match validate_start_request(&resolved_request, &support) {
        Ok(sources) => sources,
        Err(error) => {
            crate::native_capture_debug_log::log(format!(
                "rejected native capture {origin} start during source validation (resolved_sources={}, supported_sources={}): [{}] {}",
                format_capture_source_flags(&resolved_sources),
                format_capture_source_flags(&support.supported_sources),
                error.code,
                error.message
            ));
            return Err(error);
        }
    };

    let microphone_device_id_for_capture = if resolved_request.capture_microphone {
        let preferences_runtime = microphone_controller_preferences_state
            .lock()
            .expect("microphone controller preferences state poisoned");
        let controller_state = match microphone_capture::microphone_controller_state(
            preferences_runtime.preference.clone(),
            preferences_runtime.disconnect_policy.clone(),
        ) {
            Ok(state) => state,
            Err(error) => {
                crate::native_capture_debug_log::log(format!(
                    "failed to resolve microphone controller state for native capture {origin} start: [{}] {}",
                    error.code, error.message
                ));
                return Err(error);
            }
        };

        if should_wait_for_same_microphone_device(&controller_state) {
            let error = CaptureErrorResponse {
                code: "microphone_device_unavailable_waiting_for_selected_device".to_string(),
                message: "The selected microphone is unavailable. Reconnect the same device or change microphone preference."
                    .to_string(),
            };
            crate::native_capture_debug_log::log(format!(
                "rejected native capture {origin} start because the selected microphone is unavailable and wait-for-same-device is active: [{}] {}",
                error.code, error.message
            ));
            return Err(error);
        }

        resolve_capture_microphone_device_id(&controller_state)
    } else {
        None
    };

    let mut runtime = state.lock().expect("native capture state poisoned");
    if runtime.is_running {
        let existing_sources =
            format_optional_capture_source_flags(runtime.requested_sources.as_ref());
        let session_id = runtime_log_session_id(&runtime);

        if runtime.requested_sources.as_ref() != Some(&sources) {
            let error = CaptureErrorResponse {
                code: "capture_session_already_running".to_string(),
                message: "A native capture session is already running with different sources"
                    .to_string(),
            };
            crate::native_capture_debug_log::log(format!(
                "rejected native capture {origin} start because another session is already running (session_id='{}', existing_sources={}, requested_sources={}): [{}] {}",
                session_id,
                existing_sources,
                format_capture_source_flags(&sources),
                error.code,
                error.message
            ));
            return Err(error);
        }

        crate::native_capture_debug_log::log(format!(
            "native capture {origin} start requested while session is already running; returning existing session (session_id='{}', requested_sources={})",
            session_id, existing_sources
        ));

        return Ok(NativeCaptureSessionResponse {
            session: session_from_runtime(&runtime),
        });
    }

    let requested_sources_for_log = sources.clone();
    if let Err(error) = start_capture_runtime(
        &mut runtime,
        app_handle,
        &settings,
        sources,
        microphone_device_id_for_capture,
    ) {
        crate::native_capture_debug_log::log(format!(
            "failed to start native capture ({origin}, requested_sources={}): [{}] {}",
            format_capture_source_flags(&requested_sources_for_log),
            error.code,
            error.message
        ));
        return Err(error);
    }

    crate::native_capture_debug_log::log(format!(
        "started native capture successfully ({origin}, session_id='{}', requested_sources={}, segment_index={}, save_directory='{}')",
        runtime_log_session_id(&runtime),
        format_optional_capture_source_flags(runtime.requested_sources.as_ref()),
        runtime.current_segment_index,
        settings.save_directory
    ));

    Ok(NativeCaptureSessionResponse {
        session: session_from_runtime(&runtime),
    })
}

#[tauri::command]
pub fn get_capture_support() -> CaptureSupportResponse {
    let screen_support = capture_screen::support_for_current_platform();
    let microphone_supported = !matches!(
        microphone_capture::microphone_permission_state(),
        CapturePermissionState::Unsupported
    );

    let response = CaptureSupportResponse {
        platform: screen_support.platform,
        native_capture_supported: screen_support.native_capture_supported,
        supported_sources: CaptureSources {
            screen: screen_support.screen,
            microphone: microphone_supported,
            system_audio: screen_support.system_audio,
        },
    };

    log_capture_support_if_changed(&response);
    response
}

#[tauri::command]
pub fn get_capture_permissions(
    state: tauri::State<'_, NativeCaptureState>,
) -> CapturePermissionsResponse {
    let runtime = state.lock().expect("native capture state poisoned");
    let permissions = CapturePermissions {
        screen: capture_screen::screen_permission_state(),
        microphone: microphone_capture::microphone_permission_state(),
        system_audio: capture_screen::system_audio_permission_state(),
    };

    log_capture_permissions_if_changed(&permissions);

    CapturePermissionsResponse {
        permissions,
        session: session_from_runtime(&runtime),
    }
}

#[tauri::command]
pub fn get_idle_debug(state: tauri::State<'_, NativeCaptureState>) -> IdleDebugInfo {
    activity::get_idle_debug(state)
}

#[tauri::command]
pub fn get_microphone_controller_state(
    state: tauri::State<'_, MicrophoneControllerPreferencesState>,
) -> Result<MicrophoneControllerState, CaptureErrorResponse> {
    let runtime = state
        .lock()
        .expect("microphone controller preferences state poisoned");
    microphone_capture::microphone_controller_state(
        runtime.preference.clone(),
        runtime.disconnect_policy.clone(),
    )
}

#[tauri::command]
pub fn update_microphone_controller(
    request: UpdateMicrophoneControllerRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, MicrophoneControllerPreferencesState>,
) -> Result<MicrophoneControllerState, CaptureErrorResponse> {
    update_microphone_controller_impl(request, &app_handle, state)
}

pub fn initialize_recording_settings_from_disk(app_handle: &tauri::AppHandle) {
    reset_capture_log_snapshots();

    let loaded_settings = match load_recording_settings_from_disk(app_handle) {
        Some(settings) => {
            crate::native_capture_debug_log::configure(
                app_handle,
                settings.native_capture_debug_logging_enabled,
            );
            log_loaded_recording_settings("disk", &settings);
            settings
        }
        None => {
            let settings = default_recording_settings();
            crate::native_capture_debug_log::configure(
                app_handle,
                settings.native_capture_debug_logging_enabled,
            );
            log_loaded_recording_settings("defaults", &settings);
            settings
        }
    };

    let settings_state = app_handle.state::<RecordingSettingsState>();
    let mut runtime = settings_state
        .lock()
        .expect("recording settings state poisoned");
    runtime.settings = loaded_settings;
}

pub fn maybe_auto_start_native_capture(app_handle: &tauri::AppHandle) {
    let auto_start_enabled = {
        let settings_state = app_handle.state::<RecordingSettingsState>();
        let auto_start = settings_state
            .lock()
            .expect("recording settings state poisoned")
            .settings
            .auto_start;
        auto_start
    };

    if !auto_start_enabled {
        return;
    }

    let _ = start_native_capture_inner(
        "auto-start",
        StartNativeCaptureRequest {
            capture_screen: false,
            capture_microphone: false,
            capture_system_audio: false,
        },
        app_handle.state::<NativeCaptureState>(),
        app_handle.state::<MicrophoneControllerPreferencesState>(),
        app_handle.state::<RecordingSettingsState>(),
        app_handle.clone(),
    );
}

#[tauri::command]
pub fn get_recording_settings(
    state: tauri::State<'_, RecordingSettingsState>,
) -> RecordingSettings {
    state
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone()
}

#[tauri::command]
pub fn get_native_capture_debug_log_status(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> NativeCaptureDebugLogStatus {
    let enabled = state
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .native_capture_debug_logging_enabled;

    crate::native_capture_debug_log::status(&app_handle, enabled)
}

#[tauri::command]
pub fn delete_native_capture_debug_log(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<NativeCaptureDebugLogStatus, CaptureErrorResponse> {
    let enabled = state
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .native_capture_debug_logging_enabled;

    crate::native_capture_debug_log::delete(&app_handle, enabled)
}

#[tauri::command]
pub fn update_recording_settings(
    request: UpdateRecordingSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    let settings = validate_recording_settings(request)?;
    persist_recording_settings(&app_handle, &settings)?;

    let mut runtime = state.lock().expect("recording settings state poisoned");
    let previous_settings = runtime.settings.clone();
    let previous_save_directory = runtime.settings.save_directory.clone();
    runtime.settings = settings.clone();

    let save_directory_changed = previous_save_directory != settings.save_directory;
    let debug_logging_enabled_changed = previous_settings.native_capture_debug_logging_enabled
        != settings.native_capture_debug_logging_enabled;

    if previous_settings.native_capture_debug_logging_enabled
        && !settings.native_capture_debug_logging_enabled
    {
        log_recording_settings_changes(&previous_settings, &settings);

        if save_directory_changed {
            crate::native_capture_debug_log::log(format!(
                "recording save directory changed from '{}' to '{}'; app infrastructure database location will update on next app start",
                previous_save_directory, settings.save_directory
            ));
        }
    }

    crate::native_capture_debug_log::configure(
        &app_handle,
        settings.native_capture_debug_logging_enabled,
    );

    if !previous_settings.native_capture_debug_logging_enabled
        && settings.native_capture_debug_logging_enabled
    {
        reset_capture_log_snapshots();
    }

    drop(runtime);

    if settings.native_capture_debug_logging_enabled {
        if debug_logging_enabled_changed {
            crate::native_capture_debug_log::log(format!(
                "native capture debug logging {}",
                if previous_settings.native_capture_debug_logging_enabled {
                    "re-enabled"
                } else {
                    "enabled"
                }
            ));
        }

        log_recording_settings_changes(&previous_settings, &settings);
    }

    if save_directory_changed && settings.native_capture_debug_logging_enabled {
        crate::native_capture_debug_log::log(format!(
            "recording save directory changed from '{}' to '{}'; app infrastructure database location will update on next app start",
            previous_save_directory, settings.save_directory
        ));
    }

    Ok(settings)
}

#[tauri::command]
pub fn start_native_capture(
    request: StartNativeCaptureRequest,
    state: tauri::State<'_, NativeCaptureState>,
    microphone_controller_preferences_state: tauri::State<'_, MicrophoneControllerPreferencesState>,
    recording_settings_state: tauri::State<'_, RecordingSettingsState>,
    app_handle: tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    start_native_capture_inner(
        "command",
        request,
        state,
        microphone_controller_preferences_state,
        recording_settings_state,
        app_handle,
    )
}

#[tauri::command]
pub fn stop_native_capture(
    state: tauri::State<'_, NativeCaptureState>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let mut runtime = state.lock().expect("native capture state poisoned");
    let session_id = runtime_log_session_id(&runtime).to_string();
    let requested_sources = runtime.requested_sources.clone();
    let output_files_before_stop = runtime.output_files.clone();

    crate::native_capture_debug_log::log(format!(
        "received native capture stop request (is_running={}, session_id='{}', requested_sources={}, output_files_before_stop={})",
        runtime.is_running,
        session_id,
        format_optional_capture_source_flags(requested_sources.as_ref()),
        format_output_file_counts(output_files_before_stop.as_ref())
    ));

    if let Err(error) = stop_capture_runtime(&mut runtime) {
        if capture_screen::should_preserve_runtime_on_stop_error(&error) {
            crate::native_capture_debug_log::log(format!(
                "failed to stop native capture but preserved runtime for recovery (session_id='{}'): [{}] {}",
                session_id,
                error.code,
                error.message
            ));
            return Err(error);
        }

        request_segment_loop_stop(&runtime);
        mark_runtime_session_stopped(&mut runtime);
        crate::native_capture_debug_log::log(format!(
            "failed to stop native capture; runtime marked stopped (session_id='{}'): [{}] {}",
            session_id, error.code, error.message
        ));
        return Err(error);
    }

    request_segment_loop_stop(&runtime);
    mark_runtime_session_stopped(&mut runtime);
    let session = stopped_session_from_runtime(&runtime);

    crate::native_capture_debug_log::log(format!(
        "stopped native capture successfully (session_id='{}', requested_sources={}, finalized_outputs={})",
        session_log_session_id(&session),
        format_optional_capture_source_flags(session.requested_sources.as_ref()),
        format_output_file_counts(session.output_files.as_ref())
    ));

    Ok(NativeCaptureSessionResponse { session })
}
