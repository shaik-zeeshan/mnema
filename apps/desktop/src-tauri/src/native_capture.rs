use crate::native_capture_inactivity::{
    ActivityPolicyEvaluation, ActivitySnapshot, InactivityState,
};
use crate::native_capture_output::{
    append_committed_segment_output_files, cleanup_unusable_segment_artifacts,
    finalize_capture_outputs, set_current_microphone_output_file, set_current_screen_output_file,
    set_current_system_audio_output_file,
};
#[cfg(test)]
use crate::native_capture_settings::validate_recording_settings_with_resolution_support;
use crate::native_capture_settings::{
    compute_effective_screen_bitrate_bps, default_recording_settings,
    load_recording_settings_from_disk, persist_recording_settings, validate_recording_settings,
};
use capture_microphone as microphone_capture;
use capture_runtime::{
    CaptureClock, RuntimeController, RuntimeSignal, RuntimeState, SegmentPlanner, SegmentSchedule,
};
#[cfg(target_os = "macos")]
use capture_screen::RotateScreenCaptureSessionArgs;
use capture_screen::StopScreenCaptureSessionArgs;
use capture_types::{
    CaptureErrorResponse, CaptureOutputFiles, CapturePermissionState, CapturePermissions,
    CapturePermissionsResponse, CaptureSources, CaptureSupportResponse, MicrophoneControllerState,
    MicrophoneDisconnectPolicy, MicrophonePreference, MicrophonePreferenceMode,
    NativeCaptureSession, NativeCaptureSessionResponse, RecordingSettings, ScreenResolution,
    StartNativeCaptureRequest, UpdateMicrophoneControllerRequest, UpdateRecordingSettingsRequest,
};
use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::Emitter;
use tauri::Manager;

#[cfg(target_os = "macos")]
fn stop_active_sessions_after_failure(runtime: &mut NativeCaptureRuntime) {
    if let Some(session) = runtime.active_microphone_session.as_mut() {
        let _ = session.stop();
    }
    runtime.active_microphone_session = None;

    let _ = capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
        active_session: &mut runtime.active_screen_session,
    });
}

#[cfg(target_os = "macos")]
fn cleanup_failed_segment_dir(segment_dir: &Path) {
    if let Err(error) = std::fs::remove_dir_all(segment_dir) {
        if error.kind() != std::io::ErrorKind::NotFound {
            eprintln!(
                "failed removing unusable segment directory {}: {}",
                segment_dir.display(),
                error
            );
        }
    }
}

fn request_segment_loop_stop(runtime: &NativeCaptureRuntime) {
    if let Some(control) = runtime.segment_loop_control.as_ref() {
        control.stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug, Default)]
pub struct NativeCaptureRuntime {
    pub is_running: bool,
    pub session_id: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub requested_sources: Option<CaptureSources>,
    pub output_files: Option<CaptureOutputFiles>,
    #[cfg(target_os = "macos")]
    pub current_segment_output_files: Option<CaptureOutputFiles>,
    pub current_segment_index: u64,
    pub screen_frame_rate: u32,
    pub screen_resolution: ScreenResolution,
    pub effective_screen_bitrate_bps: Option<u32>,
    pub microphone_device_id_for_capture: Option<String>,
    pub segment_loop_control: Option<SegmentLoopControl>,
    pub capture_clock: Option<CaptureClock>,
    pub segment_schedule: Option<SegmentSchedule>,
    pub segment_planner: Option<SegmentPlanner>,
    pub runtime_controller: RuntimeController,
    pub runtime_state: RuntimeState,
    pub inactivity: InactivityState,
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

#[derive(Debug, Clone)]
pub(crate) struct SegmentLoopControl {
    stop: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct RecordingSettingsRuntime {
    pub settings: RecordingSettings,
}

impl Default for RecordingSettingsRuntime {
    fn default() -> Self {
        Self {
            settings: default_recording_settings(),
        }
    }
}

pub type RecordingSettingsState = Mutex<RecordingSettingsRuntime>;

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

fn monotonic_epoch() -> &'static Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now)
}

fn now_monotonic_ms() -> u64 {
    monotonic_epoch()
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn now_monotonic_marker_ms() -> u64 {
    now_monotonic_ms().saturating_add(1)
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
        is_inactivity_paused: runtime.inactivity.is_paused,
        session_id: runtime.session_id.clone(),
        started_at_unix_ms: runtime.started_at_unix_ms,
        requested_sources: runtime.requested_sources.clone(),
        output_files: runtime.output_files.clone(),
    }
}

fn stopped_session_from_runtime(runtime: &NativeCaptureRuntime) -> NativeCaptureSession {
    NativeCaptureSession {
        is_running: false,
        is_inactivity_paused: false,
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
        .current_segment_output_files
        .as_ref()
        .and_then(|output_files| output_files.screen_file.as_deref())
        .or_else(|| {
            runtime
                .output_files
                .as_ref()
                .and_then(|output_files| output_files.screen_file.as_deref())
        })
    {
        return Ok(std::path::Path::new(existing_screen_file)
            .parent()
            .expect("screen output path should have parent")
            .join(file_name)
            .to_string_lossy()
            .to_string());
    }

    let planner = runtime
        .segment_planner
        .as_ref()
        .ok_or_else(|| CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture segment planner missing while reconnecting microphone".to_string(),
        })?;
    let session_dir = planner.segment_dir(runtime.current_segment_index);
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
        && !runtime.inactivity.is_paused
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
        if let Some(output_files) = runtime.current_segment_output_files.as_mut() {
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

fn mark_runtime_session_stopped(runtime: &mut NativeCaptureRuntime) {
    runtime.is_running = false;
    runtime.inactivity = InactivityState::default();
    runtime.segment_loop_control = None;
    runtime.capture_clock = None;
    runtime.segment_schedule = None;
    runtime.segment_planner = None;
    runtime.effective_screen_bitrate_bps = None;
    #[cfg(target_os = "macos")]
    {
        runtime.current_segment_output_files = None;
    }
    #[cfg(target_os = "macos")]
    {
        runtime.active_screen_session = None;
        runtime.active_microphone_session = None;
    }

    runtime.runtime_controller = RuntimeController::default();
    runtime.runtime_state = RuntimeState::Idle;
}

fn mark_runtime_session_failed(runtime: &mut NativeCaptureRuntime) {
    runtime.is_running = false;
    runtime.inactivity = InactivityState::default();
    runtime.segment_loop_control = None;
    runtime.capture_clock = None;
    runtime.segment_schedule = None;
    runtime.segment_planner = None;
    runtime.effective_screen_bitrate_bps = None;
    #[cfg(target_os = "macos")]
    {
        runtime.current_segment_output_files = None;
    }
    #[cfg(target_os = "macos")]
    {
        runtime.active_screen_session = None;
        runtime.active_microphone_session = None;
    }

    if let Ok(state) = runtime
        .runtime_controller
        .apply(RuntimeSignal::SourceFailed)
    {
        runtime.runtime_state = state;
    } else {
        runtime.runtime_controller = RuntimeController::default();
        runtime.runtime_state = RuntimeState::Failed;
    }
}

fn apply_runtime_signal(
    runtime: &mut NativeCaptureRuntime,
    signal: RuntimeSignal,
) -> Result<(), CaptureErrorResponse> {
    runtime
        .runtime_controller
        .apply(signal)
        .map(|state| {
            runtime.runtime_state = state;
        })
        .map_err(|error| CaptureErrorResponse {
            code: "invalid_runtime_state_transition".to_string(),
            message: format!(
                "Invalid runtime transition from {:?} with {:?}",
                error.from, error.signal
            ),
        })
}

fn reset_runtime_after_start_error(runtime: &mut NativeCaptureRuntime) {
    runtime.runtime_controller = RuntimeController::default();
    runtime.runtime_state = RuntimeState::Idle;
}

fn should_rotate_segment(current_segment_index: u64, scheduled_segment_index: u64) -> bool {
    scheduled_segment_index > current_segment_index
}

#[cfg(target_os = "macos")]
fn current_system_idle_ms() -> Option<u64> {
    crate::native_capture_system_idle::current_system_idle_ms()
}

#[cfg(target_os = "macos")]
fn current_activity_snapshot() -> ActivitySnapshot {
    ActivitySnapshot {
        system_input_idle_ms: current_system_idle_ms(),
        screen_activity_idle_ms: capture_screen::screen_activity_idle_ms(),
    }
}

#[cfg(target_os = "macos")]
fn pause_runtime_for_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    if runtime.inactivity.is_paused {
        return Ok(());
    }

    let current_segment_output_files = runtime.current_segment_output_files.clone();
    let recording_file = runtime.recording_file.clone();
    let microphone_recording_file = runtime.microphone_recording_file.clone();
    let system_audio_recording_file = runtime.system_audio_recording_file.clone();
    let requested_sources = runtime.requested_sources.clone();

    if let Some(session) = runtime.active_microphone_session.as_mut() {
        session.stop()?;
    }
    runtime.active_microphone_session = None;

    capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
        active_session: &mut runtime.active_screen_session,
    })?;

    finalize_capture_outputs(
        current_segment_output_files.as_ref(),
        recording_file.as_deref(),
        microphone_recording_file.as_deref(),
        system_audio_recording_file.as_deref(),
        requested_sources.as_ref(),
    )?;

    if let (Some(committed), Some(segment)) = (
        runtime.output_files.as_mut(),
        current_segment_output_files.as_ref(),
    ) {
        append_committed_segment_output_files(committed, segment);
    }

    runtime.current_segment_output_files = None;
    runtime.recording_file = None;
    runtime.microphone_recording_file = None;
    runtime.system_audio_recording_file = None;
    runtime.inactivity.is_paused = true;

    Ok(())
}

#[cfg(target_os = "macos")]
fn resume_runtime_from_inactivity(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    if !runtime.inactivity.is_paused {
        return Ok(());
    }

    let Some(planner) = runtime.segment_planner.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture segment planner missing while resuming inactivity".to_string(),
        });
    };
    let Some(sources) = runtime.requested_sources.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture sources missing while resuming inactivity".to_string(),
        });
    };
    let Some(schedule) = runtime.segment_schedule.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture schedule missing while resuming inactivity".to_string(),
        });
    };
    let Some(clock) = runtime.capture_clock.clone() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Capture clock missing while resuming inactivity".to_string(),
        });
    };

    let scheduled_index = schedule.current_segment_index(clock.elapsed());
    let next_index = (runtime.current_segment_index + 1).max(scheduled_index);
    let segment_dir = planner.segment_dir(next_index);

    let (
        segment_outputs,
        recording_file,
        microphone_recording_file,
        system_audio_recording_file,
        active_screen_session,
        active_microphone_session,
    ) = start_segment(
        &segment_dir,
        &sources,
        runtime.screen_frame_rate,
        &runtime.screen_resolution,
        runtime.effective_screen_bitrate_bps,
        runtime.microphone_device_id_for_capture.as_deref(),
    )?;

    runtime.current_segment_index = next_index;
    runtime.current_segment_output_files = Some(segment_outputs);
    runtime.recording_file = recording_file;
    runtime.microphone_recording_file = microphone_recording_file;
    runtime.system_audio_recording_file = system_audio_recording_file;
    runtime.active_screen_session = active_screen_session;
    runtime.active_microphone_session = active_microphone_session;
    runtime.inactivity.is_paused = false;

    Ok(())
}

#[cfg(target_os = "macos")]
fn start_segment(
    session_dir: &Path,
    sources: &CaptureSources,
    screen_frame_rate: u32,
    screen_resolution: &ScreenResolution,
    effective_screen_bitrate_bps: Option<u32>,
    microphone_device_id: Option<&str>,
) -> Result<
    (
        CaptureOutputFiles,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<capture_screen::ActiveCaptureSession>,
        Option<microphone_capture::AvFoundationMicrophoneCaptureSession>,
    ),
    CaptureErrorResponse,
> {
    std::fs::create_dir_all(session_dir).map_err(|e| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!("Failed to create capture segment directory: {e}"),
    })?;

    let mut output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: None,
        microphone_files: Vec::new(),
        system_audio_file: None,
        system_audio_files: Vec::new(),
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
        let screen_capture = capture_screen::start_capture_session(
            session_dir,
            &screen_sources,
            screen_frame_rate,
            screen_resolution,
            effective_screen_bitrate_bps,
        )?;

        if let Some(screen_file) = screen_capture.output_files.screen_file {
            set_current_screen_output_file(&mut output_files, screen_file);
        }
        if let Some(system_audio_file) = screen_capture.output_files.system_audio_file {
            set_current_system_audio_output_file(&mut output_files, system_audio_file);
        }

        recording_file = Some(screen_capture.recording_file);
        system_audio_recording_file = screen_capture.system_audio_recording_file;
        active_screen_session = Some(screen_capture.session);
    }

    if sources.microphone {
        let microphone_output_file = session_dir
            .join("microphone.m4a")
            .to_string_lossy()
            .to_string();

        let mic_start =
            microphone_capture::start_avfoundation_microphone_capture_session_for_file_with_device_id(
                &microphone_output_file,
                microphone_device_id,
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

    Ok((
        output_files,
        recording_file,
        microphone_recording_file,
        system_audio_recording_file,
        active_screen_session,
        active_microphone_session,
    ))
}

#[cfg(target_os = "macos")]
fn spawn_segment_loop(app_handle: tauri::AppHandle) -> SegmentLoopControl {
    let control = SegmentLoopControl {
        stop: Arc::new(AtomicBool::new(false)),
    };
    let stop = control.stop.clone();

    thread::spawn(move || loop {
        let sleep_duration = {
            let capture_state = app_handle.state::<NativeCaptureState>();
            let runtime = match capture_state.lock() {
                Ok(runtime) => runtime,
                Err(_) => break,
            };

            if !runtime.is_running {
                break;
            }

            let Some(schedule) = runtime.segment_schedule.as_ref() else {
                break;
            };
            let Some(clock) = runtime.capture_clock.as_ref() else {
                break;
            };

            let until_boundary = schedule.sleep_until_next_boundary(clock);
            until_boundary.min(Duration::from_secs(1))
        };

        if !sleep_duration.is_zero() {
            thread::sleep(sleep_duration);
        }

        if stop.load(Ordering::Relaxed) {
            break;
        }

        let capture_state = app_handle.state::<NativeCaptureState>();
        let mut runtime = match capture_state.lock() {
            Ok(runtime) => runtime,
            Err(_) => break,
        };

        if !runtime.is_running || stop.load(Ordering::Relaxed) {
            break;
        }

        let now = now_monotonic_marker_ms();
        let activity_snapshot = current_activity_snapshot();
        let effective_idle = runtime
            .inactivity
            .effective_idle_for_snapshot(now, activity_snapshot);

        if runtime
            .inactivity
            .should_resume_from_inactivity(now, activity_snapshot)
        {
            if let Err(error) = resume_runtime_from_inactivity(&mut runtime) {
                if !capture_screen::should_preserve_runtime_on_stop_error(&error) {
                    mark_runtime_session_failed(&mut runtime);
                    break;
                }
            } else {
                eprintln!(
                    "resumed native capture after activity (effective_idle_ms={}, effective_source={}, idle_timeout_seconds={})",
                    effective_idle.idle_ms,
                    effective_idle.source.as_str(),
                    runtime.inactivity.idle_timeout_seconds
                );
            }
        }

        if runtime
            .inactivity
            .should_pause_for_inactivity(now, activity_snapshot)
        {
            if let Err(error) = pause_runtime_for_inactivity(&mut runtime) {
                if !capture_screen::should_preserve_runtime_on_stop_error(&error) {
                    mark_runtime_session_failed(&mut runtime);
                    break;
                }
            } else {
                eprintln!(
                    "paused native capture for inactivity threshold crossing (effective_idle_ms={}, effective_source={}, idle_timeout_seconds={})",
                    effective_idle.idle_ms,
                    effective_idle.source.as_str(),
                    runtime.inactivity.idle_timeout_seconds
                );
            }
            continue;
        }

        if runtime.inactivity.is_paused {
            continue;
        }

        let previous_segment_output_files = runtime.current_segment_output_files.clone();
        let recording_file = runtime.recording_file.clone();
        let microphone_recording_file = runtime.microphone_recording_file.clone();
        let system_audio_recording_file = runtime.system_audio_recording_file.clone();
        let requested_sources = runtime.requested_sources.clone();

        let Some(planner) = runtime.segment_planner.clone() else {
            mark_runtime_session_failed(&mut runtime);
            break;
        };
        let Some(sources) = runtime.requested_sources.clone() else {
            mark_runtime_session_failed(&mut runtime);
            break;
        };
        let Some(schedule) = runtime.segment_schedule.clone() else {
            mark_runtime_session_failed(&mut runtime);
            break;
        };
        let Some(clock) = runtime.capture_clock.clone() else {
            mark_runtime_session_failed(&mut runtime);
            break;
        };

        let scheduled_index = schedule.current_segment_index(clock.elapsed());
        if !should_rotate_segment(runtime.current_segment_index, scheduled_index) {
            continue;
        }

        if apply_runtime_signal(&mut runtime, RuntimeSignal::RotateRequested).is_err() {
            mark_runtime_session_failed(&mut runtime);
            break;
        }

        let next_index = (runtime.current_segment_index + 1).max(scheduled_index);
        let segment_dir = planner.segment_dir(next_index);
        if let Err(_error) = std::fs::create_dir_all(&segment_dir) {
            mark_runtime_session_failed(&mut runtime);
            break;
        }

        let mut next_segment_outputs = CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        };
        let mut next_recording_file = runtime.recording_file.clone();
        let mut next_microphone_recording_file = runtime.microphone_recording_file.clone();
        let mut next_system_audio_recording_file = runtime.system_audio_recording_file.clone();
        let mut legacy_rotated = false;

        if sources.screen || sources.system_audio {
            let rotate_result =
                capture_screen::rotate_screen_capture_session(RotateScreenCaptureSessionArgs {
                    active_session: &mut runtime.active_screen_session,
                    segment_dir: &segment_dir,
                });

            match rotate_result {
                Ok(rotated) => {
                    if let Some(file) = rotated.output_files.screen_file {
                        set_current_screen_output_file(&mut next_segment_outputs, file);
                    }
                    if let Some(file) = rotated.output_files.system_audio_file {
                        set_current_system_audio_output_file(&mut next_segment_outputs, file);
                    }
                    next_recording_file = Some(rotated.recording_file);
                    next_system_audio_recording_file = rotated.system_audio_recording_file;
                }
                Err(error) if error.code == "capture_rotation_requires_restart" => {
                    legacy_rotated = true;
                }
                Err(_) => {
                    cleanup_failed_segment_dir(&segment_dir);
                    stop_active_sessions_after_failure(&mut runtime);
                    mark_runtime_session_failed(&mut runtime);
                    break;
                }
            }
        }

        if legacy_rotated {
            if capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                active_session: &mut runtime.active_screen_session,
            })
            .is_err()
            {
                cleanup_failed_segment_dir(&segment_dir);
                stop_active_sessions_after_failure(&mut runtime);
                mark_runtime_session_failed(&mut runtime);
                break;
            }

            let screen_only_sources = CaptureSources {
                screen: sources.screen,
                microphone: false,
                system_audio: sources.system_audio,
            };

            let started_segment = start_segment(
                &segment_dir,
                &screen_only_sources,
                runtime.screen_frame_rate,
                &runtime.screen_resolution,
                runtime.effective_screen_bitrate_bps,
                runtime.microphone_device_id_for_capture.as_deref(),
            );

            let (
                started_outputs,
                started_recording_file,
                started_microphone_recording_file,
                started_system_audio_recording_file,
                active_screen_session,
                _,
            ) = match started_segment {
                Ok(value) => value,
                Err(_) => {
                    cleanup_failed_segment_dir(&segment_dir);
                    stop_active_sessions_after_failure(&mut runtime);
                    mark_runtime_session_failed(&mut runtime);
                    break;
                }
            };

            next_segment_outputs = started_outputs;
            next_recording_file = started_recording_file;
            next_microphone_recording_file = started_microphone_recording_file;
            next_system_audio_recording_file = started_system_audio_recording_file;
            runtime.active_screen_session = active_screen_session;

            if sources.microphone {
                if let Some(session) = runtime.active_microphone_session.as_mut() {
                    let microphone_output_file = planner
                        .microphone_file(next_index)
                        .to_string_lossy()
                        .to_string();
                    if session.rotate_output_file(&microphone_output_file).is_err() {
                        cleanup_failed_segment_dir(&segment_dir);
                        cleanup_unusable_segment_artifacts(
                            Some(&next_segment_outputs),
                            next_recording_file.as_deref(),
                            next_microphone_recording_file.as_deref(),
                            next_system_audio_recording_file.as_deref(),
                        );
                        stop_active_sessions_after_failure(&mut runtime);
                        mark_runtime_session_failed(&mut runtime);
                        break;
                    }
                    set_current_microphone_output_file(
                        &mut next_segment_outputs,
                        microphone_output_file.clone(),
                    );
                    next_microphone_recording_file = Some(microphone_output_file);
                }
            }
        } else if sources.microphone {
            if let Some(session) = runtime.active_microphone_session.as_mut() {
                let microphone_output_file = planner
                    .microphone_file(next_index)
                    .to_string_lossy()
                    .to_string();
                if session.rotate_output_file(&microphone_output_file).is_err() {
                    cleanup_failed_segment_dir(&segment_dir);
                    cleanup_unusable_segment_artifacts(
                        Some(&next_segment_outputs),
                        next_recording_file.as_deref(),
                        next_microphone_recording_file.as_deref(),
                        next_system_audio_recording_file.as_deref(),
                    );
                    stop_active_sessions_after_failure(&mut runtime);
                    mark_runtime_session_failed(&mut runtime);
                    break;
                }
                set_current_microphone_output_file(
                    &mut next_segment_outputs,
                    microphone_output_file.clone(),
                );
                next_microphone_recording_file = Some(microphone_output_file);
            }
        }

        if finalize_capture_outputs(
            previous_segment_output_files.as_ref(),
            recording_file.as_deref(),
            microphone_recording_file.as_deref(),
            system_audio_recording_file.as_deref(),
            requested_sources.as_ref(),
        )
        .is_err()
        {
            cleanup_failed_segment_dir(&segment_dir);
            cleanup_unusable_segment_artifacts(
                previous_segment_output_files.as_ref(),
                recording_file.as_deref(),
                microphone_recording_file.as_deref(),
                system_audio_recording_file.as_deref(),
            );
            cleanup_unusable_segment_artifacts(
                Some(&next_segment_outputs),
                next_recording_file.as_deref(),
                next_microphone_recording_file.as_deref(),
                next_system_audio_recording_file.as_deref(),
            );
            stop_active_sessions_after_failure(&mut runtime);
            mark_runtime_session_failed(&mut runtime);
            break;
        }

        if let (Some(committed), Some(segment)) = (
            runtime.output_files.as_mut(),
            previous_segment_output_files.as_ref(),
        ) {
            append_committed_segment_output_files(committed, segment);
        }

        runtime.current_segment_index = next_index;
        runtime.current_segment_output_files = Some(next_segment_outputs);
        runtime.recording_file = next_recording_file;
        runtime.microphone_recording_file = next_microphone_recording_file;
        runtime.system_audio_recording_file = next_system_audio_recording_file;

        if apply_runtime_signal(&mut runtime, RuntimeSignal::SourcesReady).is_err() {
            stop_active_sessions_after_failure(&mut runtime);
            mark_runtime_session_failed(&mut runtime);
            break;
        }
    });

    control
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

pub fn initialize_recording_settings_from_disk(app_handle: &tauri::AppHandle) {
    let settings_state = app_handle.state::<RecordingSettingsState>();
    let mut runtime = settings_state
        .lock()
        .expect("recording settings state poisoned");
    runtime.settings =
        load_recording_settings_from_disk(app_handle).unwrap_or_else(default_recording_settings);
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
    runtime.settings = settings.clone();
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

    let settings = {
        recording_settings_state
            .lock()
            .expect("recording settings state poisoned")
            .settings
            .clone()
    };

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

    if settings.capture_screen || settings.capture_system_audio {
        let screen_ok = capture_screen::ensure_screen_permission();
        if !screen_ok {
            return Err(CaptureErrorResponse {
                code: "screen_permission_denied".to_string(),
                message: if settings.capture_system_audio {
                    "Screen capture permission is required for system audio capture"
                } else {
                    "Screen capture permission is required"
                }
                .to_string(),
            });
        }
    }

    runtime.runtime_controller = RuntimeController::default();
    runtime.runtime_state = RuntimeState::Idle;
    apply_runtime_signal(&mut runtime, RuntimeSignal::StartRequested)?;

    if settings.capture_microphone {
        let microphone_ok = microphone_capture::ensure_microphone_permission();
        if !microphone_ok {
            return Err(CaptureErrorResponse {
                code: "microphone_permission_denied".to_string(),
                message: "Microphone permission is required".to_string(),
            });
        }
    }

    let start_result: Result<(), CaptureErrorResponse> = {
        #[cfg(target_os = "macos")]
        {
            let started = now_unix_ms();
            let started_monotonic = now_monotonic_marker_ms();
            let session_id = capture_screen::new_session_id()?;
            let segment_planner =
                SegmentPlanner::new(settings.save_directory.clone(), session_id.clone());
            let segment_schedule =
                SegmentSchedule::new(Duration::from_secs(settings.segment_duration_seconds));
            let capture_clock = CaptureClock::start_now();
            std::fs::create_dir_all(Path::new(&settings.save_directory)).map_err(|e| {
                CaptureErrorResponse {
                    code: "io_error".to_string(),
                    message: format!("Failed to create capture save directory: {e}"),
                }
            })?;

            let segment_index = 1;
            let first_segment_dir = segment_planner.segment_dir(segment_index);
            let effective_screen_bitrate_bps = compute_effective_screen_bitrate_bps(&settings);
            capture_screen::reset_last_screen_activity_unix_ms();

            let (
                segment_outputs,
                recording_file,
                microphone_recording_file,
                system_audio_recording_file,
                active_screen_session,
                active_microphone_session,
            ) = start_segment(
                &first_segment_dir,
                &sources,
                settings.screen_frame_rate,
                &settings.screen_resolution,
                effective_screen_bitrate_bps,
                microphone_device_id_for_capture.as_deref(),
            )?;

            let output_files = CaptureOutputFiles {
                screen_file: None,
                screen_files: Vec::new(),
                microphone_file: None,
                microphone_files: Vec::new(),
                system_audio_file: None,
                system_audio_files: Vec::new(),
            };

            let segment_loop_control = spawn_segment_loop(app_handle);

            runtime.is_running = true;
            runtime.inactivity =
                InactivityState::from_recording_settings(&settings, started_monotonic);
            runtime.started_at_unix_ms = Some(started);
            runtime.session_id = Some(session_id);
            runtime.requested_sources = Some(sources);
            runtime.output_files = Some(output_files);
            runtime.current_segment_output_files = Some(segment_outputs);
            runtime.current_segment_index = segment_index;
            runtime.screen_frame_rate = settings.screen_frame_rate;
            runtime.screen_resolution = settings.screen_resolution.clone();
            runtime.effective_screen_bitrate_bps = effective_screen_bitrate_bps;
            runtime.microphone_device_id_for_capture = microphone_device_id_for_capture;
            runtime.segment_loop_control = Some(segment_loop_control);
            runtime.capture_clock = Some(capture_clock);
            runtime.segment_schedule = Some(segment_schedule);
            runtime.segment_planner = Some(segment_planner);
            runtime.recording_file = recording_file;
            runtime.microphone_recording_file = microphone_recording_file;
            runtime.system_audio_recording_file = system_audio_recording_file;
            runtime.active_screen_session = active_screen_session;
            runtime.active_microphone_session = active_microphone_session;
            apply_runtime_signal(&mut runtime, RuntimeSignal::SourcesReady)?;
            Ok(())
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = sources;
            let _ = microphone_device_id_for_capture;
            Err(CaptureErrorResponse {
                code: "unsupported_platform".to_string(),
                message: "Native capture is currently supported only on macOS".to_string(),
            })
        }
    };

    if let Err(error) = start_result {
        reset_runtime_after_start_error(&mut runtime);
        return Err(error);
    }

    Ok(NativeCaptureSessionResponse {
        session: session_from_runtime(&runtime),
    })
}

// ─── Idle Debug ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleDebugInfo {
    /// Current system-level idle time in milliseconds, if available.
    pub system_idle_ms: Option<u64>,
    /// Whether native system idle readings are available on this platform.
    pub system_idle_available: bool,
    /// Whether the inactivity gating feature is enabled.
    pub inactivity_enabled: bool,
    /// Configured inactivity timeout in seconds (0 when feature is disabled).
    pub idle_timeout_seconds: u64,
    /// Whether capture is currently paused due to inactivity.
    pub is_inactivity_paused: bool,
    /// Detector source identifier.  "core_graphics" on macOS, "unavailable" elsewhere.
    pub detector_source: String,
    /// Configured activity policy mode used for inactivity decisions.
    pub activity_mode: String,
    /// Last observed screen sample timestamp in unix milliseconds, if any.
    pub screen_activity_last_unix_ms: Option<u64>,
    /// Current screen activity idle derived from latest screen sample, if any.
    pub screen_activity_idle_ms: Option<u64>,
    /// Effective idle time used by inactivity policy for this sample.
    pub effective_idle_ms: u64,
    /// Source selected for effective idle determination.
    #[serde(rename = "effectiveActivitySource")]
    pub effective_idle_source: String,
    /// Raw evaluated source samples for this snapshot.
    pub activity_sources: Vec<IdleDebugActivitySource>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleDebugActivitySource {
    pub kind: String,
    pub available: bool,
    pub idle_ms: Option<u64>,
    pub selected: bool,
}

#[tauri::command]
pub fn get_idle_debug(state: tauri::State<'_, NativeCaptureState>) -> IdleDebugInfo {
    let runtime = state.lock().expect("native capture state poisoned");
    let now = now_monotonic_marker_ms();
    let system_idle_ms = crate::native_capture_system_idle::current_system_idle_ms();
    let screen_activity_last_unix_ms = capture_screen::last_screen_activity_unix_ms();
    let screen_activity_idle_ms = capture_screen::screen_activity_idle_ms();
    let activity_snapshot = ActivitySnapshot {
        system_input_idle_ms: system_idle_ms,
        screen_activity_idle_ms,
    };
    let policy = runtime
        .inactivity
        .evaluate_policy_for_snapshot(now, activity_snapshot);
    let effective_idle = policy.effective_idle;
    // Reflect actual probe availability: the probe is only considered available
    // when it returned a valid reading.  A None result (invalid value, non-macOS,
    // or a future platform stub) maps to unavailable so the UI can distinguish
    // "no reading yet" from "probe not functional".
    let system_idle_available = system_idle_ms.is_some();
    let detector_source = if cfg!(target_os = "macos") {
        if system_idle_available {
            "core_graphics".to_string()
        } else {
            "core_graphics_unavailable".to_string()
        }
    } else {
        "unavailable".to_string()
    };
    IdleDebugInfo {
        system_idle_ms,
        system_idle_available,
        inactivity_enabled: runtime.inactivity.enabled,
        idle_timeout_seconds: runtime.inactivity.idle_timeout_seconds,
        is_inactivity_paused: runtime.inactivity.is_paused,
        detector_source,
        activity_mode: match runtime.inactivity.activity_mode {
            capture_types::InactivityActivityMode::SystemInputOnly => {
                "system_input_only".to_string()
            }
            capture_types::InactivityActivityMode::SystemInputOrScreen => {
                "system_input_or_screen".to_string()
            }
        },
        screen_activity_last_unix_ms,
        screen_activity_idle_ms,
        effective_idle_ms: effective_idle.idle_ms,
        effective_idle_source: effective_idle.source.as_str().to_string(),
        activity_sources: idle_debug_activity_sources(&policy),
    }
}

fn idle_debug_activity_sources(policy: &ActivityPolicyEvaluation) -> Vec<IdleDebugActivitySource> {
    policy
        .sources
        .iter()
        .map(|source| IdleDebugActivitySource {
            kind: source.kind.as_str().to_string(),
            available: source.available,
            idle_ms: source.idle_ms,
            selected: source.kind == policy.effective_idle.source,
        })
        .collect()
}

#[tauri::command]
pub fn stop_native_capture(
    state: tauri::State<'_, NativeCaptureState>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let mut runtime = state.lock().expect("native capture state poisoned");

    let stop_result: Result<(), CaptureErrorResponse> = {
        #[cfg(target_os = "macos")]
        {
            if runtime.is_running {
                apply_runtime_signal(&mut runtime, RuntimeSignal::StopRequested)?;
            }

            let current_segment_output_files = runtime.current_segment_output_files.clone();
            let recording_file = runtime.recording_file.clone();
            let microphone_recording_file = runtime.microphone_recording_file.clone();
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
                current_segment_output_files.as_ref(),
                recording_file.as_deref(),
                microphone_recording_file.as_deref(),
                system_audio_recording_file.as_deref(),
                requested_sources.as_ref(),
            ) {
                cleanup_unusable_segment_artifacts(
                    current_segment_output_files.as_ref(),
                    recording_file.as_deref(),
                    microphone_recording_file.as_deref(),
                    system_audio_recording_file.as_deref(),
                );
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

            if first_error.is_none() {
                if let (Some(committed), Some(segment)) = (
                    runtime.output_files.as_mut(),
                    current_segment_output_files.as_ref(),
                ) {
                    append_committed_segment_output_files(committed, segment);
                }
            }

            if let Some(error) = first_error {
                Err(error)
            } else {
                if runtime.runtime_state == RuntimeState::Stopping {
                    apply_runtime_signal(&mut runtime, RuntimeSignal::SourcesStopped)?;
                }
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

        request_segment_loop_stop(&runtime);
        mark_runtime_session_stopped(&mut runtime);
        return Err(error);
    }

    request_segment_loop_stop(&runtime);
    mark_runtime_session_stopped(&mut runtime);
    let session = stopped_session_from_runtime(&runtime);

    Ok(NativeCaptureSessionResponse { session })
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::{
        default_inactivity_activity_mode, default_video_bitrate, ScreenResolutionPreset,
        VideoBitrateMode, VideoBitratePreset, VideoBitrateSettings,
    };

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
    fn validate_recording_settings_rejects_all_sources_disabled() {
        let error = validate_recording_settings(UpdateRecordingSettingsRequest {
            capture_screen: false,
            capture_microphone: false,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        })
        .expect_err("all sources disabled must be rejected");

        assert_eq!(error.code, "invalid_recording_settings");
        assert_eq!(error.message, "At least one capture source must be enabled");
    }

    #[test]
    fn validate_recording_settings_rejects_system_audio_without_screen() {
        let error = validate_recording_settings(UpdateRecordingSettingsRequest {
            capture_screen: false,
            capture_microphone: true,
            capture_system_audio: true,
            segment_duration_seconds: 60,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        })
        .expect_err("system audio without screen must be rejected");

        assert_eq!(error.code, "invalid_recording_settings");
        assert_eq!(
            error.message,
            "System audio capture requires screen capture"
        );
    }

    #[test]
    fn validate_recording_settings_allows_storing_resolution_when_screen_disabled() {
        let settings = validate_recording_settings_with_resolution_support(
            UpdateRecordingSettingsRequest {
                capture_screen: false,
                capture_microphone: true,
                capture_system_audio: false,
                segment_duration_seconds: 60,
                screen_frame_rate: 30,
                screen_resolution: ScreenResolution::Custom {
                    width: 1280,
                    height: 720,
                },
                video_bitrate: default_video_bitrate(),
                save_directory: "/tmp".to_string(),
                auto_start: false,
                pause_capture_on_inactivity: true,
                idle_timeout_seconds: 10,
                inactivity_activity_mode: default_inactivity_activity_mode(),
            },
            true,
        )
        .expect("resolution settings should still be storable");

        assert_eq!(
            settings.screen_resolution,
            ScreenResolution::Custom {
                width: 1280,
                height: 720,
            }
        );
    }

    #[test]
    fn validate_recording_settings_allows_non_original_resolution_when_screen_disabled_on_fallback_backend(
    ) {
        let settings = validate_recording_settings_with_resolution_support(
            UpdateRecordingSettingsRequest {
                capture_screen: false,
                capture_microphone: true,
                capture_system_audio: false,
                segment_duration_seconds: 60,
                screen_frame_rate: 30,
                screen_resolution: ScreenResolution::Preset {
                    preset: ScreenResolutionPreset::P720,
                },
                video_bitrate: default_video_bitrate(),
                save_directory: "/tmp".to_string(),
                auto_start: false,
                pause_capture_on_inactivity: true,
                idle_timeout_seconds: 10,
                inactivity_activity_mode: default_inactivity_activity_mode(),
            },
            false,
        )
        .expect("resolution should be allowed when screen capture is disabled");

        assert_eq!(
            settings.screen_resolution,
            ScreenResolution::Preset {
                preset: ScreenResolutionPreset::P720,
            }
        );
    }

    #[test]
    fn validate_recording_settings_rejects_non_original_resolution_when_screen_enabled_on_fallback_backend(
    ) {
        let error = validate_recording_settings_with_resolution_support(
            UpdateRecordingSettingsRequest {
                capture_screen: true,
                capture_microphone: false,
                capture_system_audio: false,
                segment_duration_seconds: 60,
                screen_frame_rate: 30,
                screen_resolution: ScreenResolution::Preset {
                    preset: ScreenResolutionPreset::P720,
                },
                video_bitrate: default_video_bitrate(),
                save_directory: "/tmp".to_string(),
                auto_start: false,
                pause_capture_on_inactivity: true,
                idle_timeout_seconds: 10,
                inactivity_activity_mode: default_inactivity_activity_mode(),
            },
            false,
        )
        .expect_err("fallback backend must reject non-original resolution when screen is enabled");

        assert_eq!(error.code, "screen_resolution_unsupported");
    }

    #[test]
    fn validate_recording_settings_rejects_too_small_custom_resolution() {
        let error = validate_recording_settings(UpdateRecordingSettingsRequest {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Custom {
                width: 8,
                height: 8,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        })
        .expect_err("too small resolution should be rejected");

        assert_eq!(error.code, "invalid_recording_settings");
    }

    #[test]
    fn validate_recording_settings_defaults_preset_bitrate_when_preset_value_missing() {
        let settings = validate_recording_settings(UpdateRecordingSettingsRequest {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: VideoBitrateSettings {
                mode: VideoBitrateMode::Preset,
                preset: None,
                custom_mbps: Some(12),
            },
            save_directory: "/tmp".to_string(),
            auto_start: false,
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        })
        .expect("preset mode should normalize bitrate values");

        assert_eq!(settings.video_bitrate.mode, VideoBitrateMode::Preset);
        assert_eq!(
            settings.video_bitrate.preset,
            Some(VideoBitratePreset::Medium)
        );
        assert_eq!(settings.video_bitrate.custom_mbps, None);
    }

    #[test]
    fn validate_recording_settings_rejects_custom_bitrate_out_of_range() {
        let error = validate_recording_settings(UpdateRecordingSettingsRequest {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: VideoBitrateSettings {
                mode: VideoBitrateMode::Custom,
                preset: Some(VideoBitratePreset::High),
                custom_mbps: Some(41),
            },
            save_directory: "/tmp".to_string(),
            auto_start: false,
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        })
        .expect_err("custom bitrate above max should be rejected");

        assert_eq!(error.code, "invalid_recording_settings");
        assert_eq!(
            error.message,
            "videoBitrate.customMbps must be between 1 and 40"
        );
    }

    #[test]
    fn compute_effective_screen_bitrate_uses_preset_formula() {
        let settings = RecordingSettings {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::P720,
            },
            video_bitrate: VideoBitrateSettings {
                mode: VideoBitrateMode::Preset,
                preset: Some(VideoBitratePreset::Medium),
                custom_mbps: None,
            },
            save_directory: "/tmp".to_string(),
            auto_start: false,
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        };

        let bitrate = compute_effective_screen_bitrate_bps(&settings)
            .expect("screen capture should produce a bitrate");

        assert_eq!(bitrate, 2_750_000);
    }

    #[test]
    fn compute_effective_screen_bitrate_uses_custom_value() {
        let settings = RecordingSettings {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: VideoBitrateSettings {
                mode: VideoBitrateMode::Custom,
                preset: Some(VideoBitratePreset::Low),
                custom_mbps: Some(7),
            },
            save_directory: "/tmp".to_string(),
            auto_start: false,
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        };

        let bitrate = compute_effective_screen_bitrate_bps(&settings)
            .expect("screen capture should produce a bitrate");

        assert_eq!(bitrate, 7_000_000);
    }

    #[test]
    fn compute_effective_screen_bitrate_none_when_screen_disabled() {
        let settings = RecordingSettings {
            capture_screen: false,
            capture_microphone: true,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::P1080,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        };

        assert_eq!(compute_effective_screen_bitrate_bps(&settings), None);
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
                screen_files: vec!["/tmp/screen.mov".to_string()],
                microphone_file: Some("/tmp/microphone.m4a".to_string()),
                microphone_files: vec!["/tmp/microphone.m4a".to_string()],
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            current_segment_output_files: None,
            current_segment_index: 1,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            effective_screen_bitrate_bps: None,
            microphone_device_id_for_capture: None,
            segment_loop_control: None,
            capture_clock: None,
            segment_schedule: None,
            segment_planner: None,
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
            runtime_controller: RuntimeController::default(),
            runtime_state: RuntimeState::Idle,
            inactivity: InactivityState::default(),
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
                screen_files: vec!["/tmp/screen.mov".to_string()],
                microphone_file: Some("/tmp/microphone.m4a".to_string()),
                microphone_files: vec!["/tmp/microphone.m4a".to_string()],
                system_audio_file: Some("/tmp/system-audio.m4a".to_string()),
                system_audio_files: vec!["/tmp/system-audio.m4a".to_string()],
            }),
            current_segment_output_files: None,
            current_segment_index: 1,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            effective_screen_bitrate_bps: None,
            microphone_device_id_for_capture: None,
            segment_loop_control: None,
            capture_clock: None,
            segment_schedule: None,
            segment_planner: None,
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
            runtime_controller: RuntimeController::default(),
            runtime_state: RuntimeState::Idle,
            inactivity: InactivityState::default(),
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
                screen_files: vec!["/tmp/screen.mov".to_string()],
                microphone_file: Some("/tmp/microphone.m4a".to_string()),
                microphone_files: vec!["/tmp/microphone.m4a".to_string()],
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            current_segment_output_files: None,
            current_segment_index: 1,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            effective_screen_bitrate_bps: None,
            microphone_device_id_for_capture: None,
            segment_loop_control: None,
            capture_clock: None,
            segment_schedule: None,
            segment_planner: None,
            recording_file: Some("/tmp/screen.mov".to_string()),
            microphone_recording_file: Some("/tmp/microphone.m4a".to_string()),
            system_audio_recording_file: None,
            active_screen_session: None,
            active_microphone_session: None,
            runtime_controller: RuntimeController::default(),
            runtime_state: RuntimeState::Idle,
            inactivity: InactivityState::default(),
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
                screen_files: Vec::new(),
                microphone_file: Some("/tmp/microphone.m4a".to_string()),
                microphone_files: vec!["/tmp/microphone.m4a".to_string()],
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            current_segment_output_files: None,
            current_segment_index: 1,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            effective_screen_bitrate_bps: None,
            microphone_device_id_for_capture: None,
            segment_loop_control: None,
            capture_clock: None,
            segment_schedule: None,
            segment_planner: None,
            recording_file: None,
            microphone_recording_file: Some("/tmp/microphone.m4a".to_string()),
            system_audio_recording_file: None,
            active_screen_session: None,
            active_microphone_session: None,
            runtime_controller: RuntimeController::default(),
            runtime_state: RuntimeState::Idle,
            inactivity: InactivityState::default(),
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
                screen_files: vec!["/tmp/screen.mov".to_string()],
                microphone_file: Some("/tmp/microphone.m4a".to_string()),
                microphone_files: vec!["/tmp/microphone.m4a".to_string()],
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            current_segment_output_files: None,
            current_segment_index: 1,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            effective_screen_bitrate_bps: None,
            microphone_device_id_for_capture: None,
            segment_loop_control: None,
            capture_clock: None,
            segment_schedule: None,
            segment_planner: None,
            recording_file: Some("/tmp/screen.mov".to_string()),
            microphone_recording_file: Some("/tmp/microphone.m4a".to_string()),
            system_audio_recording_file: None,
            active_screen_session: None,
            active_microphone_session: None,
            runtime_controller: RuntimeController::default(),
            runtime_state: RuntimeState::Idle,
            inactivity: InactivityState::default(),
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
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
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
    fn should_rotate_segment_only_after_boundary_crossing() {
        assert!(!should_rotate_segment(1, 1));
        assert!(should_rotate_segment(1, 2));
        assert!(should_rotate_segment(3, 5));
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
