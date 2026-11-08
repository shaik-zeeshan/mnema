//! The microphone controller's host half: the device-change notifier, the
//! preference update and the waited-for-device reconnect drive
//! `NativeCaptureState` and the segment machinery (S3 parts b and c). The rest
//! is `app_core::native_capture::microphone`.

use app_core::native_capture::microphone::{
    emit_microphone_auto_disconnect_transition_failed, emit_microphone_controller_changed,
    resolve_capture_microphone_device_id, validate_microphone_preference,
    MicrophoneControllerPreferencesState, MicrophoneDeviceChangeNotifierState,
};
#[cfg(target_os = "macos")]
use app_core::native_capture::microphone::{
    restart_microphone_session_for_runtime, should_move_microphone_capture_to_waiting_state,
    should_reconnect_waiting_microphone_session, should_wait_for_same_microphone_device,
};
use app_core::AppCore;
use capture_microphone as microphone_capture;
use capture_types::{
    CaptureErrorResponse, MicrophoneControllerState, UpdateMicrophoneControllerRequest,
};
use std::sync::Arc;
use tauri::Manager;

use super::NativeCaptureState;

#[cfg(target_os = "macos")]
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
            emit_microphone_controller_changed(
                &app_handle.state::<Arc<AppCore>>(),
                controller_state,
            );
        }
    });

    let notifier_state = app_handle.state::<MicrophoneDeviceChangeNotifierState>();
    let mut notifier_slot = notifier_state
        .lock()
        .expect("microphone device change notifier state poisoned");
    *notifier_slot = Some(notifier);
}

#[cfg(not(target_os = "macos"))]
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
    drop(runtime);

    // A new preference can end (or start) a wait for a missing device; settle
    // it now rather than at the next device change.
    #[cfg(target_os = "macos")]
    maybe_reconnect_waiting_microphone_session(app_handle, &controller_state);

    emit_microphone_controller_changed(
        &app_handle.state::<Arc<AppCore>>(),
        controller_state.clone(),
    );

    Ok(controller_state)
}

#[cfg(target_os = "macos")]
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

    let mut stop_failed_while_waiting = false;

    if should_move_microphone_capture_to_waiting_state(
        runtime.is_running,
        runtime.requested_sources.as_ref(),
        runtime.active_microphone_session.is_some(),
        state,
    ) {
        if let Err(error) = super::segments::stop_microphone_session_for_runtime(runtime) {
            app_core::debug_log::log_warn(format!(
                "failed to stop microphone session while waiting for same device: [{}] {}",
                error.code, error.message
            ));
            emit_microphone_auto_disconnect_transition_failed(
                &app_handle.state::<Arc<AppCore>>(),
                &error,
            );
            stop_failed_while_waiting = true;
        }
        if !stop_failed_while_waiting {
            runtime.active_microphone_session = None;
        }
    }
    // Keeps the tick from reopening a device that is gone, whether or not a
    // session was running when it went; once it is back, the reconnect below
    // or the tick brings the microphone back.
    runtime.microphone_waiting_for_device = should_wait_for_same_microphone_device(state);

    if stop_failed_while_waiting || !should_reconnect_waiting_microphone_session(&runtime, state) {
        return;
    }

    let _ = restart_microphone_session_for_runtime(
        runtime,
        state.preference.device_id.as_deref(),
        "reconnecting microphone",
    );
}

/// The device a microphone reopen should use, from the current preferences:
/// `Some(None)` is the default device, and `None` means they could not be read
/// without waiting. The tick calls this holding the capture lock, while the
/// device-change notifier takes the preferences lock and then the capture
/// lock, so this only ever `try_lock`s: blocking here could deadlock.
#[cfg(target_os = "macos")]
pub(super) fn current_capture_microphone_device_id(
    app_handle: &tauri::AppHandle,
) -> Option<Option<String>> {
    let (preference, disconnect_policy) = {
        let state = app_handle.try_state::<MicrophoneControllerPreferencesState>()?;
        let preferences = state.try_lock().ok()?;
        (
            preferences.preference.clone(),
            preferences.disconnect_policy.clone(),
        )
    };
    let state =
        microphone_capture::microphone_controller_state(preference, disconnect_policy).ok()?;
    Some(resolve_capture_microphone_device_id(&state))
}
