use capture_microphone as microphone_capture;
use capture_screen::StopScreenCaptureSessionArgs;
use capture_types::{
    CaptureErrorResponse, CaptureOutputFiles, CapturePermissionState, CapturePermissions,
    CapturePermissionsResponse, CaptureSources, CaptureSupportResponse, MicrophoneControllerState,
    MicrophoneDisconnectPolicy, MicrophonePreference, MicrophonePreferenceMode,
    NativeCaptureSession, NativeCaptureSessionResponse, StartNativeCaptureRequest,
    UpdateMicrophoneControllerRequest,
};
use serde::Serialize;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Emitter;
use tauri::Manager;

#[cfg(target_os = "macos")]
fn maybe_remove_intermediate_file(file: &str, label: &str, failures: &mut Vec<String>) {
    match std::fs::remove_file(file) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            failures.push(format!(
                "failed to remove intermediate {label} recording file: {error}"
            ));
        }
    }
}

#[cfg(target_os = "macos")]
fn microphone_output_files(output_files: &CaptureOutputFiles) -> Vec<&str> {
    if !output_files.microphone_files.is_empty() {
        output_files
            .microphone_files
            .iter()
            .map(String::as_str)
            .collect()
    } else {
        output_files
            .microphone_file
            .as_deref()
            .into_iter()
            .collect()
    }
}

#[cfg(target_os = "macos")]
fn finalize_capture_outputs(
    output_files: Option<&CaptureOutputFiles>,
    recording_file: Option<&str>,
    microphone_recording_file: Option<&str>,
    system_audio_recording_file: Option<&str>,
    requested_sources: Option<&CaptureSources>,
) -> Result<(), CaptureErrorResponse> {
    let Some(output_files) = output_files else {
        return Ok(());
    };

    let mut failures: Vec<String> = Vec::new();
    let microphone_files = microphone_output_files(output_files);

    if output_files.microphone_file.is_some() && output_files.microphone_files.is_empty() {
        let microphone_file = output_files
            .microphone_file
            .as_deref()
            .expect("checked microphone_file is present");
        let source_recording = microphone_recording_file.or(recording_file);

        if let Some(source_recording) = source_recording {
            if source_recording != microphone_file {
                if let Err(error) = capture_writers::convert_recording_audio_to_m4a(
                    source_recording,
                    microphone_file,
                ) {
                    failures.push(format!(
                        "microphone output conversion failed: {}",
                        error.message
                    ));
                }
            }
        } else {
            failures
                .push("microphone output conversion failed: missing source recording".to_string());
        }
    }

    if let Some(system_audio_file) = output_files.system_audio_file.as_deref() {
        if let Some(source_recording) = system_audio_recording_file {
            if source_recording != system_audio_file {
                if let Err(error) = capture_writers::convert_recording_audio_to_m4a(
                    source_recording,
                    system_audio_file,
                ) {
                    failures.push(format!(
                        "system audio output conversion failed: {}",
                        error.message
                    ));
                }
            }
        } else {
            failures.push(
                "system audio output conversion failed: missing source recording".to_string(),
            );
        }
    }

    if requested_sources.is_some_and(|sources| sources.system_audio) {
        if let Some(recording_file) = recording_file {
            if let Err(error) = capture_screen::strip_audio_from_recording_file(recording_file) {
                failures.push(format!(
                    "screen output video-only conversion failed: {}",
                    error.message
                ));
            }
        }
    }

    if let Some(microphone_recording_file) = microphone_recording_file {
        if !microphone_files.contains(&microphone_recording_file) {
            maybe_remove_intermediate_file(microphone_recording_file, "microphone", &mut failures);
        }
    }

    if let Some(system_audio_recording_file) = system_audio_recording_file {
        if output_files.system_audio_file.as_deref() != Some(system_audio_recording_file) {
            maybe_remove_intermediate_file(
                system_audio_recording_file,
                "system audio",
                &mut failures,
            );
        }
    }

    capture_writers::aggregate_output_processing_failures(failures)
}

#[derive(Debug, Default)]
pub struct NativeCaptureRuntime {
    pub is_running: bool,
    pub session_id: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub requested_sources: Option<CaptureSources>,
    pub output_files: Option<CaptureOutputFiles>,
    #[cfg(target_os = "macos")]
    pub recording_file: Option<String>,
    #[cfg(target_os = "macos")]
    pub microphone_recording_file: Option<String>,
    #[cfg(target_os = "macos")]
    pub system_audio_recording_file: Option<String>,
    #[cfg(target_os = "macos")]
    pub active_screen_session: Option<capture_screen::ActiveCaptureSession>,
    #[cfg(target_os = "macos")]
    pub active_microphone_session: Option<microphone_capture::AvFoundationMicrophoneCaptureSession>,
}

pub type NativeCaptureState = Mutex<NativeCaptureRuntime>;

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
struct MicrophoneAutoDisconnectTransitionFailedEvent {
    context: String,
    code: String,
    message: String,
}

fn microphone_auto_disconnect_transition_failed_event(
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

pub fn emit_microphone_controller_changed(
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

            emit_microphone_controller_changed(&app_handle, controller_state);
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

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

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

fn session_from_runtime(runtime: &NativeCaptureRuntime) -> NativeCaptureSession {
    NativeCaptureSession {
        is_running: runtime.is_running,
        session_id: runtime.session_id.clone(),
        started_at_unix_ms: runtime.started_at_unix_ms,
        requested_sources: runtime.requested_sources.clone(),
        output_files: runtime.output_files.clone(),
    }
}

fn stopped_session_from_runtime(runtime: &NativeCaptureRuntime) -> NativeCaptureSession {
    NativeCaptureSession {
        is_running: false,
        session_id: runtime.session_id.clone(),
        started_at_unix_ms: runtime.started_at_unix_ms,
        requested_sources: runtime.requested_sources.clone(),
        output_files: runtime.output_files.clone(),
    }
}

fn validate_start_request(
    request: &StartNativeCaptureRequest,
    support: &CaptureSupportResponse,
) -> Result<CaptureSources, CaptureErrorResponse> {
    if !request.capture_screen && !request.capture_microphone && !request.capture_system_audio {
        return Err(CaptureErrorResponse {
            code: "invalid_request".to_string(),
            message: "At least one capture source must be enabled".to_string(),
        });
    }

    if !support.native_capture_supported {
        return Err(CaptureErrorResponse {
            code: "unsupported_platform".to_string(),
            message: "Native capture is currently supported only on macOS".to_string(),
        });
    }

    if request.capture_system_audio && !support.supported_sources.system_audio {
        return Err(CaptureErrorResponse {
            code: "system_audio_unsupported".to_string(),
            message: "System audio capture requires macOS 15.0 or newer".to_string(),
        });
    }

    if request.capture_system_audio && !request.capture_screen {
        return Err(CaptureErrorResponse {
            code: "system_audio_requires_screen".to_string(),
            message: "System audio-only capture is not supported; enable screen capture as well"
                .to_string(),
        });
    }

    Ok(CaptureSources {
        screen: request.capture_screen,
        microphone: request.capture_microphone,
        system_audio: request.capture_system_audio,
    })
}

fn should_wait_for_same_microphone_device(state: &MicrophoneControllerState) -> bool {
    state.preference.mode == MicrophonePreferenceMode::SpecificDevice
        && state.disconnect_policy == MicrophoneDisconnectPolicy::WaitForSameDevice
        && state.preference.device_id.is_some()
        && state.effective_device.is_none()
}

#[cfg(target_os = "macos")]
fn should_move_microphone_capture_to_waiting_state(
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

#[cfg(target_os = "macos")]
fn next_microphone_output_file_for_runtime(
    runtime: &NativeCaptureRuntime,
) -> Result<String, CaptureErrorResponse> {
    let file_name = format!("microphone-{}.m4a", now_unix_ms());

    if let Some(existing_screen_file) = runtime
        .output_files
        .as_ref()
        .and_then(|output_files| output_files.screen_file.as_deref())
    {
        return Ok(std::path::Path::new(existing_screen_file)
            .parent()
            .expect("screen output path should have parent")
            .join(file_name)
            .to_string_lossy()
            .to_string());
    }

    let session_id = runtime
        .session_id
        .as_deref()
        .ok_or_else(|| CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture session id missing while reconnecting microphone".to_string(),
        })?;

    let session_dir = std::env::temp_dir()
        .join("z-native-capture")
        .join(session_id);
    std::fs::create_dir_all(&session_dir).map_err(|e| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!("Failed to create capture session directory: {e}"),
    })?;

    Ok(session_dir.join(file_name).to_string_lossy().to_string())
}

#[cfg(target_os = "macos")]
fn should_reconnect_waiting_microphone_session(
    runtime: &NativeCaptureRuntime,
    state: &MicrophoneControllerState,
) -> bool {
    runtime.is_running
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
fn maybe_reconnect_waiting_microphone_session(
    app_handle: &tauri::AppHandle,
    state: &MicrophoneControllerState,
) {
    let capture_state = app_handle.state::<NativeCaptureState>();
    let mut runtime = match capture_state.lock() {
        Ok(runtime) => runtime,
        Err(_) => return,
    };

    let mut stop_failed_while_waiting = false;

    if should_move_microphone_capture_to_waiting_state(
        runtime.is_running,
        runtime.requested_sources.as_ref(),
        runtime.active_microphone_session.is_some(),
        state,
    ) {
        if let Some(session) = runtime.active_microphone_session.as_mut() {
            if let Err(error) = session.stop() {
                eprintln!(
                    "failed to stop microphone session while waiting for same device: [{}] {}",
                    error.code, error.message
                );
                emit_microphone_auto_disconnect_transition_failed(app_handle, &error);
                stop_failed_while_waiting = true;
            }
        }
        if !stop_failed_while_waiting {
            runtime.active_microphone_session = None;
        }
    }

    if stop_failed_while_waiting || !should_reconnect_waiting_microphone_session(&runtime, state) {
        return;
    }

    let microphone_recording_file = match next_microphone_output_file_for_runtime(&runtime) {
        Ok(path) => path,
        Err(_) => return,
    };

    let mic_start =
        microphone_capture::start_avfoundation_microphone_capture_session_for_file_with_device_id(
            &microphone_recording_file,
            state.preference.device_id.as_deref(),
        );

    if let Ok(session) = mic_start {
        runtime.active_microphone_session = Some(session);
        runtime.microphone_recording_file = Some(microphone_recording_file.clone());
        if let Some(output_files) = runtime.output_files.as_mut() {
            set_current_microphone_output_file(output_files, microphone_recording_file);
        }
    }
}

fn resolve_capture_microphone_device_id(state: &MicrophoneControllerState) -> Option<String> {
    state.effective_device.as_ref().and_then(|device| {
        if device.is_default {
            None
        } else {
            Some(device.id.clone())
        }
    })
}

fn set_current_microphone_output_file(output_files: &mut CaptureOutputFiles, file: String) {
    output_files.microphone_file = Some(file.clone());
    output_files.microphone_files.push(file);
}

fn mark_runtime_session_stopped(runtime: &mut NativeCaptureRuntime) {
    runtime.is_running = false;
    #[cfg(target_os = "macos")]
    {
        runtime.active_screen_session = None;
        runtime.active_microphone_session = None;
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

    emit_microphone_controller_changed(&app_handle, controller_state.clone());

    Ok(controller_state)
}

#[tauri::command]
pub fn start_native_capture(
    request: StartNativeCaptureRequest,
    state: tauri::State<'_, NativeCaptureState>,
    microphone_controller_preferences_state: tauri::State<'_, MicrophoneControllerPreferencesState>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
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

    if request.capture_screen || request.capture_system_audio {
        let screen_ok = capture_screen::ensure_screen_permission();
        if !screen_ok {
            return Err(CaptureErrorResponse {
                code: "screen_permission_denied".to_string(),
                message: if request.capture_system_audio {
                    "Screen capture permission is required for system audio capture"
                } else {
                    "Screen capture permission is required"
                }
                .to_string(),
            });
        }
    }

    if request.capture_microphone {
        let microphone_ok = microphone_capture::ensure_microphone_permission();
        if !microphone_ok {
            return Err(CaptureErrorResponse {
                code: "microphone_permission_denied".to_string(),
                message: "Microphone permission is required".to_string(),
            });
        }
    }

    #[cfg(target_os = "macos")]
    {
        let started = now_unix_ms();
        let session_id = capture_screen::new_session_id()?;
        let mut output_files = CaptureOutputFiles {
            screen_file: None,
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
        };

        let mut recording_file: Option<String> = None;
        let mut microphone_recording_file: Option<String> = None;
        let mut system_audio_recording_file: Option<String> = None;
        let mut active_screen_session: Option<capture_screen::ActiveCaptureSession> = None;
        let mut active_microphone_session: Option<
            microphone_capture::AvFoundationMicrophoneCaptureSession,
        > = None;

        if sources.screen || sources.system_audio {
            let screen_sources = capture_screen::ScreenCaptureSources {
                screen: sources.screen,
                system_audio: sources.system_audio,
            };
            let screen_capture =
                capture_screen::start_capture_session(&session_id, &screen_sources)?;
            output_files.screen_file = screen_capture.output_files.screen_file;
            output_files.system_audio_file = screen_capture.output_files.system_audio_file;
            recording_file = Some(screen_capture.recording_file);
            system_audio_recording_file = screen_capture.system_audio_recording_file;
            active_screen_session = Some(screen_capture.session);
        }

        if sources.microphone {
            let microphone_output_file =
                if let Some(existing_screen_file) = output_files.screen_file.as_deref() {
                    std::path::Path::new(existing_screen_file)
                        .parent()
                        .expect("screen output path should have parent")
                        .join("microphone.m4a")
                        .to_string_lossy()
                        .to_string()
                } else {
                    let session_dir = std::env::temp_dir()
                        .join("z-native-capture")
                        .join(&session_id);
                    std::fs::create_dir_all(&session_dir).map_err(|e| CaptureErrorResponse {
                        code: "io_error".to_string(),
                        message: format!("Failed to create capture session directory: {e}"),
                    })?;
                    session_dir
                        .join("microphone.m4a")
                        .to_string_lossy()
                        .to_string()
                };

            let mic_start =
                microphone_capture::start_avfoundation_microphone_capture_session_for_file_with_device_id(
                    &microphone_output_file,
                    microphone_device_id_for_capture.as_deref(),
                );

            match mic_start {
                Ok(session) => {
                    set_current_microphone_output_file(
                        &mut output_files,
                        microphone_output_file.clone(),
                    );
                    microphone_recording_file = Some(microphone_output_file);
                    active_microphone_session = Some(session);
                }
                Err(error) => {
                    if let Err(rollback_error) =
                        capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                            active_session: &mut active_screen_session,
                        })
                    {
                        return Err(CaptureErrorResponse {
                            code: error.code,
                            message: format!(
                                "{}; additionally failed to rollback screen capture session: [{}] {}",
                                error.message, rollback_error.code, rollback_error.message
                            ),
                        });
                    }

                    return Err(error);
                }
            }
        }

        runtime.is_running = true;
        runtime.started_at_unix_ms = Some(started);
        runtime.session_id = Some(session_id);
        runtime.requested_sources = Some(sources);
        runtime.output_files = Some(output_files);
        runtime.recording_file = recording_file;
        runtime.microphone_recording_file = microphone_recording_file;
        runtime.system_audio_recording_file = system_audio_recording_file;
        runtime.active_screen_session = active_screen_session;
        runtime.active_microphone_session = active_microphone_session;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = sources;
        let _ = microphone_device_id_for_capture;
        return Err(CaptureErrorResponse {
            code: "unsupported_platform".to_string(),
            message: "Native capture is currently supported only on macOS".to_string(),
        });
    }

    Ok(NativeCaptureSessionResponse {
        session: session_from_runtime(&runtime),
    })
}

#[tauri::command]
pub fn stop_native_capture(
    state: tauri::State<'_, NativeCaptureState>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let mut runtime = state.lock().expect("native capture state poisoned");

    let stop_result: Result<(), CaptureErrorResponse> = {
        #[cfg(target_os = "macos")]
        {
            let output_files = runtime.output_files.clone();
            let recording_file = runtime.recording_file.clone();
            let system_audio_recording_file = runtime.system_audio_recording_file.clone();
            let requested_sources = runtime.requested_sources.clone();

            let mut first_error: Option<CaptureErrorResponse> = None;

            if let Some(session) = runtime.active_microphone_session.as_mut() {
                if let Err(error) = session.stop() {
                    first_error = Some(error);
                }
                runtime.active_microphone_session = None;
            }

            if let Err(error) =
                capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                    active_session: &mut runtime.active_screen_session,
                })
            {
                if capture_screen::should_preserve_runtime_on_stop_error(&error) {
                    return Err(error);
                }

                if first_error.is_none() {
                    first_error = Some(error);
                }
            }

            if let Err(error) = finalize_capture_outputs(
                output_files.as_ref(),
                recording_file.as_deref(),
                runtime.microphone_recording_file.as_deref(),
                system_audio_recording_file.as_deref(),
                requested_sources.as_ref(),
            ) {
                if let Some(previous_error) = first_error.take() {
                    first_error = Some(CaptureErrorResponse {
                        code: previous_error.code,
                        message: format!(
                            "{}; additionally failed to finalize capture outputs: [{}] {}",
                            previous_error.message, error.code, error.message
                        ),
                    });
                } else {
                    first_error = Some(error);
                }
            }

            if let Some(error) = first_error {
                Err(error)
            } else {
                Ok(())
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok(())
        }
    };

    if let Err(error) = stop_result {
        if capture_screen::should_preserve_runtime_on_stop_error(&error) {
            return Err(error);
        }

        mark_runtime_session_stopped(&mut runtime);
        return Err(error);
    }

    mark_runtime_session_stopped(&mut runtime);
    let session = stopped_session_from_runtime(&runtime);

    Ok(NativeCaptureSessionResponse { session })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_start_request_rejects_system_audio_when_not_supported() {
        let request = StartNativeCaptureRequest {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: true,
        };
        let support = CaptureSupportResponse {
            platform: "macos".to_string(),
            native_capture_supported: true,
            supported_sources: CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            },
        };

        let error =
            validate_start_request(&request, &support).expect_err("must reject system audio");
        assert_eq!(error.code, "system_audio_unsupported");
    }

    #[test]
    fn mark_runtime_session_stopped_preserves_session_metadata() {
        let mut runtime = NativeCaptureRuntime {
            is_running: true,
            session_id: Some("session-1".to_string()),
            started_at_unix_ms: Some(123),
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            }),
            output_files: Some(CaptureOutputFiles {
                screen_file: Some("/tmp/screen.mov".to_string()),
                microphone_file: Some("/tmp/microphone.m4a".to_string()),
                microphone_files: vec!["/tmp/microphone.m4a".to_string()],
                system_audio_file: None,
            }),
            #[cfg(target_os = "macos")]
            recording_file: Some("/tmp/screen.mov".to_string()),
            #[cfg(target_os = "macos")]
            microphone_recording_file: Some("/tmp/microphone.mov".to_string()),
            #[cfg(target_os = "macos")]
            system_audio_recording_file: None,
            #[cfg(target_os = "macos")]
            active_screen_session: None,
            #[cfg(target_os = "macos")]
            active_microphone_session: None,
        };

        mark_runtime_session_stopped(&mut runtime);

        assert!(!runtime.is_running);
        assert_eq!(runtime.session_id, Some("session-1".to_string()));
        assert_eq!(runtime.started_at_unix_ms, Some(123));
        assert!(runtime.requested_sources.is_some());
        assert!(runtime.output_files.is_some());
    }

    #[test]
    fn stopped_session_from_runtime_preserves_finalized_metadata() {
        let runtime = NativeCaptureRuntime {
            is_running: true,
            session_id: Some("session-1".to_string()),
            started_at_unix_ms: Some(123),
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: true,
                system_audio: true,
            }),
            output_files: Some(CaptureOutputFiles {
                screen_file: Some("/tmp/screen.mov".to_string()),
                microphone_file: Some("/tmp/microphone.m4a".to_string()),
                microphone_files: vec!["/tmp/microphone.m4a".to_string()],
                system_audio_file: Some("/tmp/system-audio.m4a".to_string()),
            }),
            #[cfg(target_os = "macos")]
            recording_file: None,
            #[cfg(target_os = "macos")]
            microphone_recording_file: None,
            #[cfg(target_os = "macos")]
            system_audio_recording_file: None,
            #[cfg(target_os = "macos")]
            active_screen_session: None,
            #[cfg(target_os = "macos")]
            active_microphone_session: None,
        };

        let session = stopped_session_from_runtime(&runtime);

        assert!(!session.is_running);
        assert_eq!(session.session_id, Some("session-1".to_string()));
        assert_eq!(session.started_at_unix_ms, Some(123));
        assert!(session.requested_sources.as_ref().is_some_and(|sources| {
            sources.screen && sources.microphone && sources.system_audio
        }));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_reconnect_waiting_microphone_session_when_device_returns() {
        let runtime = NativeCaptureRuntime {
            is_running: true,
            session_id: Some("session-1".to_string()),
            started_at_unix_ms: Some(123),
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            }),
            output_files: Some(CaptureOutputFiles {
                screen_file: Some("/tmp/screen.mov".to_string()),
                microphone_file: Some("/tmp/microphone.m4a".to_string()),
                microphone_files: vec!["/tmp/microphone.m4a".to_string()],
                system_audio_file: None,
            }),
            recording_file: Some("/tmp/screen.mov".to_string()),
            microphone_recording_file: Some("/tmp/microphone.m4a".to_string()),
            system_audio_recording_file: None,
            active_screen_session: None,
            active_microphone_session: None,
        };
        let state = MicrophoneControllerState {
            devices: vec![capture_types::MicrophoneDevice {
                id: "mic-1".to_string(),
                name: "Mic 1".to_string(),
                is_default: false,
            }],
            preference: MicrophonePreference {
                mode: MicrophonePreferenceMode::SpecificDevice,
                device_id: Some("mic-1".to_string()),
            },
            disconnect_policy: MicrophoneDisconnectPolicy::WaitForSameDevice,
            effective_device: Some(capture_types::MicrophoneDevice {
                id: "mic-1".to_string(),
                name: "Mic 1".to_string(),
                is_default: false,
            }),
        };

        assert!(should_reconnect_waiting_microphone_session(
            &runtime, &state
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_not_reconnect_waiting_microphone_session_while_device_missing() {
        let runtime = NativeCaptureRuntime {
            is_running: true,
            session_id: Some("session-1".to_string()),
            started_at_unix_ms: Some(123),
            requested_sources: Some(CaptureSources {
                screen: false,
                microphone: true,
                system_audio: false,
            }),
            output_files: Some(CaptureOutputFiles {
                screen_file: None,
                microphone_file: Some("/tmp/microphone.m4a".to_string()),
                microphone_files: vec!["/tmp/microphone.m4a".to_string()],
                system_audio_file: None,
            }),
            recording_file: None,
            microphone_recording_file: Some("/tmp/microphone.m4a".to_string()),
            system_audio_recording_file: None,
            active_screen_session: None,
            active_microphone_session: None,
        };
        let state = MicrophoneControllerState {
            devices: vec![],
            preference: MicrophonePreference {
                mode: MicrophonePreferenceMode::SpecificDevice,
                device_id: Some("mic-1".to_string()),
            },
            disconnect_policy: MicrophoneDisconnectPolicy::WaitForSameDevice,
            effective_device: None,
        };

        assert!(!should_reconnect_waiting_microphone_session(
            &runtime, &state
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_move_microphone_capture_to_waiting_state_when_selected_device_missing() {
        let state = MicrophoneControllerState {
            devices: vec![],
            preference: MicrophonePreference {
                mode: MicrophonePreferenceMode::SpecificDevice,
                device_id: Some("mic-1".to_string()),
            },
            disconnect_policy: MicrophoneDisconnectPolicy::WaitForSameDevice,
            effective_device: None,
        };

        assert!(should_move_microphone_capture_to_waiting_state(
            true,
            Some(&CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            }),
            true,
            &state,
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn next_microphone_output_file_for_runtime_uses_new_segment_name() {
        let runtime = NativeCaptureRuntime {
            is_running: true,
            session_id: Some("session-1".to_string()),
            started_at_unix_ms: Some(123),
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            }),
            output_files: Some(CaptureOutputFiles {
                screen_file: Some("/tmp/screen.mov".to_string()),
                microphone_file: Some("/tmp/microphone.m4a".to_string()),
                microphone_files: vec!["/tmp/microphone.m4a".to_string()],
                system_audio_file: None,
            }),
            recording_file: Some("/tmp/screen.mov".to_string()),
            microphone_recording_file: Some("/tmp/microphone.m4a".to_string()),
            system_audio_recording_file: None,
            active_screen_session: None,
            active_microphone_session: None,
        };

        let path = next_microphone_output_file_for_runtime(&runtime)
            .expect("should build next microphone segment path");

        assert!(path.starts_with("/tmp/microphone-"));
        assert!(path.ends_with(".m4a"));
        assert_ne!(path, "/tmp/microphone.m4a");
    }

    #[test]
    fn set_current_microphone_output_file_tracks_all_segments() {
        let mut output_files = CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
        };

        set_current_microphone_output_file(&mut output_files, "/tmp/microphone-1.m4a".to_string());
        set_current_microphone_output_file(&mut output_files, "/tmp/microphone-2.m4a".to_string());

        assert_eq!(
            output_files.microphone_file,
            Some("/tmp/microphone-2.m4a".to_string())
        );
        assert_eq!(
            output_files.microphone_files,
            vec![
                "/tmp/microphone-1.m4a".to_string(),
                "/tmp/microphone-2.m4a".to_string()
            ]
        );
    }

    #[test]
    fn microphone_auto_disconnect_transition_failed_event_has_expected_payload() {
        let error = CaptureErrorResponse {
            code: "microphone_stop_failed".to_string(),
            message: "stop failed".to_string(),
        };

        let payload = microphone_auto_disconnect_transition_failed_event(&error);

        assert_eq!(payload.context, "stop_before_wait_for_same_device");
        assert_eq!(payload.code, "microphone_stop_failed");
        assert_eq!(payload.message, "stop failed");
    }
}
