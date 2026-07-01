use capture_microphone as microphone_capture;
use capture_types::{
    CaptureErrorResponse, CaptureSources, MicrophoneControllerState, MicrophoneDisconnectPolicy,
    MicrophonePreference, MicrophonePreferenceMode, UpdateMicrophoneControllerRequest,
};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

use super::output::set_current_microphone_output_file;
use super::runtime::{
    ensure_microphone_planner_for_runtime, now_unix_ms, refresh_runtime_planner_dates,
    NativeCaptureRuntime,
};
use super::NativeCaptureState;

#[derive(Debug, Clone)]
pub struct MicrophoneControllerPreferencesRuntime {
    pub preference: MicrophonePreference,
    pub disconnect_policy: MicrophoneDisconnectPolicy,
}

impl Default for MicrophoneControllerPreferencesRuntime {
    fn default() -> Self {
        Self {
            preference: MicrophonePreference {
                mode: MicrophonePreferenceMode::Default,
                device_id: None,
            },
            disconnect_policy: MicrophoneDisconnectPolicy::FallbackToDefault,
        }
    }
}

pub type MicrophoneControllerPreferencesState = Mutex<MicrophoneControllerPreferencesRuntime>;
pub type MicrophoneDeviceChangeNotifierState =
    Mutex<Option<microphone_capture::MicrophoneDeviceChangeNotifier>>;

const MICROPHONE_CONTROLLER_CHANGED_EVENT: &str = "microphone_controller_changed";
const MICROPHONE_AUTO_DISCONNECT_TRANSITION_FAILED_EVENT: &str =
    "microphone_auto_disconnect_transition_failed";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MicrophoneAutoDisconnectTransitionFailedEvent {
    pub context: String,
    pub code: String,
    pub message: String,
}

pub(super) fn microphone_auto_disconnect_transition_failed_event(
    error: &CaptureErrorResponse,
) -> MicrophoneAutoDisconnectTransitionFailedEvent {
    MicrophoneAutoDisconnectTransitionFailedEvent {
        context: "stop_before_wait_for_same_device".to_string(),
        code: error.code.clone(),
        message: error.message.clone(),
    }
}

fn validate_microphone_preference(
    preference: MicrophonePreference,
) -> Result<MicrophonePreference, CaptureErrorResponse> {
    if preference.mode != MicrophonePreferenceMode::SpecificDevice {
        return Ok(preference);
    }

    let device_id = preference
        .device_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| CaptureErrorResponse {
            code: "microphone_specific_device_id_required".to_string(),
            message: "A non-empty device_id is required when preference.mode is specific_device"
                .to_string(),
        })?;

    Ok(MicrophonePreference {
        mode: preference.mode,
        device_id: Some(device_id),
    })
}

fn emit_microphone_controller_changed(
    app_handle: &tauri::AppHandle,
    state: MicrophoneControllerState,
) {
    let _ = app_handle.emit(MICROPHONE_CONTROLLER_CHANGED_EVENT, state);
}

fn emit_microphone_auto_disconnect_transition_failed(
    app_handle: &tauri::AppHandle,
    error: &CaptureErrorResponse,
) {
    let payload = microphone_auto_disconnect_transition_failed_event(error);
    let _ = app_handle.emit(MICROPHONE_AUTO_DISCONNECT_TRANSITION_FAILED_EVENT, payload);
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn start_microphone_device_change_notifier(app_handle: tauri::AppHandle) {
    let notifier = microphone_capture::start_microphone_device_change_notifier({
        let app_handle = app_handle.clone();
        move || {
            let preferences_state = app_handle.state::<MicrophoneControllerPreferencesState>();
            let runtime = match preferences_state.lock() {
                Ok(runtime) => runtime,
                Err(_) => return,
            };

            let controller_state = match microphone_capture::microphone_controller_state(
                runtime.preference.clone(),
                runtime.disconnect_policy.clone(),
            ) {
                Ok(state) => state,
                Err(_) => return,
            };

            maybe_reconnect_waiting_microphone_session(&app_handle, &controller_state);
            emit_microphone_controller_changed(&app_handle, controller_state);
            // Device hotplug can flip per-source capture support (e.g. the first
            // microphone appearing after launch), and the tray menu bakes that
            // support into its source-item enablement — rebuild it.
            crate::status_bar::refresh(&app_handle);
        }
    });

    let notifier_state = app_handle.state::<MicrophoneDeviceChangeNotifierState>();
    let mut notifier_slot = notifier_state
        .lock()
        .expect("microphone device change notifier state poisoned");
    *notifier_slot = Some(notifier);
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn start_microphone_device_change_notifier(_app_handle: tauri::AppHandle) {}

pub(super) fn update_microphone_controller(
    request: UpdateMicrophoneControllerRequest,
    app_handle: &tauri::AppHandle,
    state: tauri::State<'_, MicrophoneControllerPreferencesState>,
) -> Result<MicrophoneControllerState, CaptureErrorResponse> {
    let preference = validate_microphone_preference(request.preference)?;
    let disconnect_policy = request.disconnect_policy;
    let controller_state = microphone_capture::microphone_controller_state(
        preference.clone(),
        disconnect_policy.clone(),
    )?;

    let mut runtime = state
        .lock()
        .expect("microphone controller preferences state poisoned");
    runtime.preference = preference;
    runtime.disconnect_policy = disconnect_policy;

    emit_microphone_controller_changed(app_handle, controller_state.clone());

    Ok(controller_state)
}

pub(super) fn should_wait_for_same_microphone_device(state: &MicrophoneControllerState) -> bool {
    state.preference.mode == MicrophonePreferenceMode::SpecificDevice
        && state.disconnect_policy == MicrophoneDisconnectPolicy::WaitForSameDevice
        && state.preference.device_id.is_some()
        && state.effective_device.is_none()
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) fn should_move_microphone_capture_to_waiting_state(
    runtime_is_running: bool,
    requested_sources: Option<&CaptureSources>,
    has_active_microphone_session: bool,
    state: &MicrophoneControllerState,
) -> bool {
    runtime_is_running
        && requested_sources.is_some_and(|sources| sources.microphone)
        && has_active_microphone_session
        && should_wait_for_same_microphone_device(state)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) fn next_microphone_output_file_for_runtime(
    runtime: &NativeCaptureRuntime,
) -> Result<String, CaptureErrorResponse> {
    let planner = runtime
        .microphone_planner
        .as_ref()
        .ok_or_else(|| CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture microphone planner missing while reconnecting microphone".to_string(),
        })?;
    let audio_dir = planner.audio_dir();
    std::fs::create_dir_all(&audio_dir).map_err(|error| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!("Failed to create capture audio directory: {error}"),
    })?;

    Ok(planner
        .microphone_reconnect_file(runtime.current_segment_index, now_unix_ms())
        .to_string_lossy()
        .to_string())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) fn should_reconnect_waiting_microphone_session(
    runtime: &NativeCaptureRuntime,
    state: &MicrophoneControllerState,
) -> bool {
    runtime.is_running
        && !runtime.inactivity.is_microphone_paused()
        && runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.microphone)
        && runtime.active_microphone_session.is_none()
        && state.preference.mode == MicrophonePreferenceMode::SpecificDevice
        && state.disconnect_policy == MicrophoneDisconnectPolicy::WaitForSameDevice
        && state.preference.device_id.is_some()
        && state.effective_device.is_some()
}

#[cfg(target_os = "macos")]
fn stop_active_microphone_session_for_device_transition(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    if let Some(session) = runtime.active_microphone_session.as_mut() {
        session.stop()?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn stop_active_microphone_session_for_device_transition(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    if let Some(session) = runtime.active_microphone_session.as_mut() {
        let finalization = session.stop_returning_finalization()?;
        super::segments::apply_windows_microphone_output_finalization(
            runtime.current_segment_output_files.as_mut(),
            &finalization,
        );
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn should_restart_active_microphone_session_for_effective_device_change(
    runtime: &NativeCaptureRuntime,
    state: &MicrophoneControllerState,
) -> bool {
    should_restart_active_microphone_session_for_effective_device_change_policy(
        runtime.is_running,
        runtime.inactivity.is_microphone_paused(),
        runtime.requested_sources.as_ref(),
        runtime.active_microphone_session.is_some(),
        runtime.microphone_device_id_for_capture.as_deref(),
        state,
    )
}

#[cfg(target_os = "windows")]
pub(super) fn should_restart_active_microphone_session_for_effective_device_change_policy(
    runtime_is_running: bool,
    is_microphone_paused: bool,
    requested_sources: Option<&CaptureSources>,
    has_active_microphone_session: bool,
    captured_device_id: Option<&str>,
    state: &MicrophoneControllerState,
) -> bool {
    if !runtime_is_running
        || is_microphone_paused
        || !requested_sources.is_some_and(|sources| sources.microphone)
        || !has_active_microphone_session
    {
        return false;
    }

    let can_change_effective_device = state.preference.mode == MicrophonePreferenceMode::Default
        || state.disconnect_policy == MicrophoneDisconnectPolicy::FallbackToDefault;
    if !can_change_effective_device {
        return false;
    }

    let Some(effective_device) = state.effective_device.as_ref() else {
        return false;
    };

    captured_device_id != Some(effective_device.id.as_str())
}

#[cfg(target_os = "windows")]
fn should_start_available_microphone_session_for_effective_device(
    runtime: &NativeCaptureRuntime,
    state: &MicrophoneControllerState,
) -> bool {
    runtime.is_running
        && !runtime.user_capture_paused
        && !runtime.inactivity.is_microphone_paused()
        && runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.microphone)
        && runtime.active_microphone_session.is_none()
        && state.effective_device.is_some()
        && (state.preference.mode == MicrophonePreferenceMode::Default
            || state.disconnect_policy == MicrophoneDisconnectPolicy::FallbackToDefault)
}

#[cfg(target_os = "macos")]
fn start_microphone_session_for_device_transition(
    runtime: &mut NativeCaptureRuntime,
    state: &MicrophoneControllerState,
    microphone_recording_file: String,
) -> Result<(), CaptureErrorResponse> {
    let session =
        microphone_capture::start_avfoundation_microphone_capture_session_for_file_with_device_id(
            &microphone_recording_file,
            state.preference.device_id.as_deref(),
        )?;

    runtime.active_microphone_session = Some(session);
    runtime.microphone_recording_file = Some(microphone_recording_file.clone());
    if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
        set_current_microphone_output_file(output_files, microphone_recording_file);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn start_microphone_session_for_device_transition(
    runtime: &mut NativeCaptureRuntime,
    state: &MicrophoneControllerState,
    microphone_recording_file: String,
) -> Result<(), CaptureErrorResponse> {
    let microphone_device_id_for_capture = resolve_capture_microphone_device_id(state);
    let session = microphone_capture::start_wasapi_microphone_capture_session_for_file(
        &microphone_recording_file,
        microphone_device_id_for_capture.as_deref(),
    )?;

    runtime.active_microphone_session = Some(Box::new(session));
    runtime.microphone_recording_file = Some(microphone_recording_file.clone());
    runtime.microphone_device_id_for_capture = microphone_device_id_for_capture;
    if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
        set_current_microphone_output_file(output_files, microphone_recording_file);
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn maybe_reconnect_waiting_microphone_session(
    app_handle: &tauri::AppHandle,
    state: &MicrophoneControllerState,
) {
    let capture_state = app_handle.state::<NativeCaptureState>();
    let mut runtime = match capture_state.lock() {
        Ok(runtime) => runtime,
        Err(_) => return,
    };
    let runtime = runtime.runtime_mut();

    let mut stop_failed_for_device_transition = false;
    let mut should_start_microphone_session = false;

    if should_move_microphone_capture_to_waiting_state(
        runtime.is_running,
        runtime.requested_sources.as_ref(),
        runtime.active_microphone_session.is_some(),
        state,
    ) {
        if let Err(error) = stop_active_microphone_session_for_device_transition(runtime) {
            super::debug_log::log(format!(
                "failed to stop microphone session while waiting for same device: [{}] {}",
                error.code, error.message
            ));
            emit_microphone_auto_disconnect_transition_failed(app_handle, &error);
            stop_failed_for_device_transition = true;
        }
        if !stop_failed_for_device_transition {
            runtime.active_microphone_session = None;
            #[cfg(target_os = "windows")]
            {
                runtime.microphone_device_id_for_capture = None;
            }
        }
    }

    #[cfg(target_os = "windows")]
    if should_restart_active_microphone_session_for_effective_device_change(&runtime, state) {
        if let Err(error) = stop_active_microphone_session_for_device_transition(runtime) {
            super::debug_log::log(format!(
                "failed to stop microphone session before effective device restart: [{}] {}",
                error.code, error.message
            ));
            emit_microphone_auto_disconnect_transition_failed(app_handle, &error);
            stop_failed_for_device_transition = true;
        }
        if !stop_failed_for_device_transition {
            runtime.active_microphone_session = None;
            should_start_microphone_session = true;
        }
    }

    if !should_start_microphone_session {
        should_start_microphone_session =
            should_reconnect_waiting_microphone_session(&runtime, state);
    }

    #[cfg(target_os = "windows")]
    if !should_start_microphone_session {
        should_start_microphone_session =
            should_start_available_microphone_session_for_effective_device(&runtime, state);
    }

    if stop_failed_for_device_transition || !should_start_microphone_session {
        return;
    }

    if ensure_microphone_planner_for_runtime(runtime, "reconnecting microphone").is_err() {
        return;
    }

    refresh_runtime_planner_dates(runtime);

    let microphone_recording_file = match next_microphone_output_file_for_runtime(&runtime) {
        Ok(path) => path,
        Err(_) => return,
    };

    if let Err(error) =
        start_microphone_session_for_device_transition(runtime, state, microphone_recording_file)
    {
        super::debug_log::log(format!(
            "failed to start microphone session after device transition: [{}] {}",
            error.code, error.message
        ));
        emit_microphone_auto_disconnect_transition_failed(app_handle, &error);
    }
}

#[cfg(target_os = "macos")]
pub(super) fn resolve_capture_microphone_device_id(
    state: &MicrophoneControllerState,
) -> Option<String> {
    state.effective_device.as_ref().and_then(|device| {
        if device.is_default {
            None
        } else {
            Some(device.id.clone())
        }
    })
}

#[cfg(target_os = "windows")]
pub(super) fn resolve_capture_microphone_device_id(
    state: &MicrophoneControllerState,
) -> Option<String> {
    state
        .effective_device
        .as_ref()
        .map(|device| device.id.clone())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(super) fn resolve_capture_microphone_device_id(
    state: &MicrophoneControllerState,
) -> Option<String> {
    let _ = state;
    None
}
