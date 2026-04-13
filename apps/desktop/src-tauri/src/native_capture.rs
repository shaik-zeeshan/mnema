mod activity;
mod microphone;
mod runtime;
mod segments;
#[cfg(test)]
mod tests;

use crate::native_capture_settings::{
    load_recording_settings_or_default, persist_recording_settings, validate_recording_settings,
};
use capture_microphone as microphone_capture;
use capture_types::{
    CaptureErrorResponse, CapturePermissionState, CapturePermissions, CapturePermissionsResponse,
    CaptureSources, CaptureSupportResponse, MicrophoneControllerState,
    NativeCaptureSessionResponse, RecordingSettings, StartNativeCaptureRequest,
    UpdateMicrophoneControllerRequest, UpdateRecordingSettingsRequest,
};
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

#[tauri::command]
pub fn get_capture_support() -> CaptureSupportResponse {
    let screen_support = capture_screen::support_for_current_platform();
    let microphone_supported = !matches!(
        microphone_capture::microphone_permission_state(),
        CapturePermissionState::Unsupported
    );

    CaptureSupportResponse {
        platform: screen_support.platform,
        native_capture_supported: screen_support.native_capture_supported,
        supported_sources: CaptureSources {
            screen: screen_support.screen,
            microphone: microphone_supported,
            system_audio: screen_support.system_audio,
        },
    }
}

#[tauri::command]
pub fn get_capture_permissions(
    state: tauri::State<'_, NativeCaptureState>,
) -> CapturePermissionsResponse {
    let runtime = state.lock().expect("native capture state poisoned");
    CapturePermissionsResponse {
        permissions: CapturePermissions {
            screen: capture_screen::screen_permission_state(),
            microphone: microphone_capture::microphone_permission_state(),
            system_audio: capture_screen::system_audio_permission_state(),
        },
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
    let settings_state = app_handle.state::<RecordingSettingsState>();
    let mut runtime = settings_state
        .lock()
        .expect("recording settings state poisoned");
    runtime.settings = load_recording_settings_or_default(app_handle);
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

    let result = start_native_capture(
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

    if let Err(error) = result {
        eprintln!(
            "failed to auto-start native capture: [{}] {}",
            error.code, error.message
        );
    }
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
pub fn update_recording_settings(
    request: UpdateRecordingSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    let settings = validate_recording_settings(request)?;
    persist_recording_settings(&app_handle, &settings)?;

    let mut runtime = state.lock().expect("recording settings state poisoned");
    let previous_save_directory = runtime.settings.save_directory.clone();
    runtime.settings = settings.clone();

    if previous_save_directory != settings.save_directory {
        eprintln!(
            "recording save directory changed from '{}' to '{}'; app infrastructure database location will update on next app start",
            previous_save_directory, settings.save_directory
        );
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
    let _ = request;

    let settings = recording_settings_state
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();

    let request = StartNativeCaptureRequest {
        capture_screen: settings.capture_screen,
        capture_microphone: settings.capture_microphone,
        capture_system_audio: settings.capture_system_audio,
    };

    let support = get_capture_support();
    let sources = validate_start_request(&request, &support)?;

    let microphone_device_id_for_capture = if request.capture_microphone {
        let preferences_runtime = microphone_controller_preferences_state
            .lock()
            .expect("microphone controller preferences state poisoned");
        let controller_state = microphone_capture::microphone_controller_state(
            preferences_runtime.preference.clone(),
            preferences_runtime.disconnect_policy.clone(),
        )?;

        if should_wait_for_same_microphone_device(&controller_state) {
            return Err(CaptureErrorResponse {
                code: "microphone_device_unavailable_waiting_for_selected_device".to_string(),
                message: "The selected microphone is unavailable. Reconnect the same device or change microphone preference."
                    .to_string(),
            });
        }

        resolve_capture_microphone_device_id(&controller_state)
    } else {
        None
    };

    let mut runtime = state.lock().expect("native capture state poisoned");
    if runtime.is_running {
        if runtime.requested_sources.as_ref() != Some(&sources) {
            return Err(CaptureErrorResponse {
                code: "capture_session_already_running".to_string(),
                message: "A native capture session is already running with different sources"
                    .to_string(),
            });
        }

        return Ok(NativeCaptureSessionResponse {
            session: session_from_runtime(&runtime),
        });
    }

    start_capture_runtime(
        &mut runtime,
        app_handle,
        &settings,
        sources,
        microphone_device_id_for_capture,
    )?;

    Ok(NativeCaptureSessionResponse {
        session: session_from_runtime(&runtime),
    })
}

#[tauri::command]
pub fn stop_native_capture(
    state: tauri::State<'_, NativeCaptureState>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let mut runtime = state.lock().expect("native capture state poisoned");

    if let Err(error) = stop_capture_runtime(&mut runtime) {
        if capture_screen::should_preserve_runtime_on_stop_error(&error) {
            return Err(error);
        }

        request_segment_loop_stop(&runtime);
        mark_runtime_session_stopped(&mut runtime);
        return Err(error);
    }

    request_segment_loop_stop(&runtime);
    mark_runtime_session_stopped(&mut runtime);
    let session = stopped_session_from_runtime(&runtime);

    Ok(NativeCaptureSessionResponse { session })
}
