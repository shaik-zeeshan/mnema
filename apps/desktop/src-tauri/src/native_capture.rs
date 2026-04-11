use capture_microphone as microphone_capture;
use capture_runtime::{
    CaptureClock, RuntimeController, RuntimeSignal, RuntimeState, SegmentPlanner, SegmentSchedule,
};
#[cfg(target_os = "macos")]
use capture_screen::RotateScreenCaptureSessionArgs;
use capture_screen::StopScreenCaptureSessionArgs;
use capture_types::{
    default_video_bitrate, CaptureErrorResponse, CaptureOutputFiles, CapturePermissionState,
    CapturePermissions, CapturePermissionsResponse, CaptureSources, CaptureSupportResponse,
    MicrophoneControllerState, MicrophoneDisconnectPolicy, MicrophonePreference,
    MicrophonePreferenceMode, NativeCaptureSession, NativeCaptureSessionResponse,
    RecordingSettings, ScreenResolution, ScreenResolutionPreset, StartNativeCaptureRequest,
    UpdateMicrophoneControllerRequest, UpdateRecordingSettingsRequest, VideoBitrateMode,
    VideoBitratePreset, VideoBitrateSettings,
};
use serde::Serialize;
#[cfg(target_os = "macos")]
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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

#[cfg(target_os = "macos")]
fn append_committed_segment_output_files(
    committed: &mut CaptureOutputFiles,
    segment: &CaptureOutputFiles,
) {
    if let Some(file) = segment.screen_file.as_ref() {
        set_current_screen_output_file(committed, file.clone());
    }
    if let Some(file) = segment.microphone_file.as_ref() {
        set_current_microphone_output_file(committed, file.clone());
    }
    if let Some(file) = segment.system_audio_file.as_ref() {
        set_current_system_audio_output_file(committed, file.clone());
    }
}

#[cfg(target_os = "macos")]
fn cleanup_unusable_segment_artifacts(
    output_files: Option<&CaptureOutputFiles>,
    recording_file: Option<&str>,
    microphone_recording_file: Option<&str>,
    system_audio_recording_file: Option<&str>,
) {
    let mut files_to_remove: BTreeSet<String> = BTreeSet::new();

    if let Some(output_files) = output_files {
        for file in &output_files.screen_files {
            let _ = files_to_remove.insert(file.clone());
        }
        for file in &output_files.microphone_files {
            let _ = files_to_remove.insert(file.clone());
        }
        for file in &output_files.system_audio_files {
            let _ = files_to_remove.insert(file.clone());
        }

        if let Some(file) = output_files.screen_file.as_ref() {
            let _ = files_to_remove.insert(file.clone());
        }
        if let Some(file) = output_files.microphone_file.as_ref() {
            let _ = files_to_remove.insert(file.clone());
        }
        if let Some(file) = output_files.system_audio_file.as_ref() {
            let _ = files_to_remove.insert(file.clone());
        }
    }

    if let Some(file) = recording_file {
        let _ = files_to_remove.insert(file.to_string());
    }
    if let Some(file) = microphone_recording_file {
        let _ = files_to_remove.insert(file.to_string());
    }
    if let Some(file) = system_audio_recording_file {
        let _ = files_to_remove.insert(file.to_string());
    }

    for file in files_to_remove {
        if let Err(error) = std::fs::remove_file(&file) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("failed removing unusable segment artifact {file}: {error}");
            }
        }
    }
}

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
const RECORDING_SETTINGS_FILE_NAME: &str = "recording-settings.json";
const MIN_CUSTOM_VIDEO_BITRATE_MBPS: u32 = 1;
const MAX_CUSTOM_VIDEO_BITRATE_MBPS: u32 = 40;
const MIN_EFFECTIVE_VIDEO_BITRATE_BPS: u32 = 500_000;
const MAX_EFFECTIVE_VIDEO_BITRATE_BPS: u32 = 120_000_000;
const VIDEO_BITRATE_ROUND_STEP_BPS: u32 = 250_000;

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

fn default_save_directory() -> String {
    std::env::var("HOME")
        .map(|home| Path::new(&home).join(".z_records"))
        .unwrap_or_else(|_| PathBuf::from(".z_records"))
        .to_string_lossy()
        .to_string()
}

fn default_recording_settings() -> RecordingSettings {
    RecordingSettings {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 30,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: default_save_directory(),
        auto_start: false,
    }
}

fn validate_screen_resolution(
    value: ScreenResolution,
) -> Result<ScreenResolution, CaptureErrorResponse> {
    match value {
        ScreenResolution::Preset { .. } => Ok(value),
        ScreenResolution::Custom { width, height } => {
            const MIN_DIMENSION: u32 = 16;
            const MAX_DIMENSION: u32 = 8192;

            if !(MIN_DIMENSION..=MAX_DIMENSION).contains(&width)
                || !(MIN_DIMENSION..=MAX_DIMENSION).contains(&height)
            {
                return Err(CaptureErrorResponse {
                    code: "invalid_recording_settings".to_string(),
                    message: format!(
                        "Custom screen resolution width/height must be between {MIN_DIMENSION} and {MAX_DIMENSION}"
                    ),
                });
            }

            Ok(ScreenResolution::Custom { width, height })
        }
    }
}

fn validate_video_bitrate(
    value: VideoBitrateSettings,
) -> Result<VideoBitrateSettings, CaptureErrorResponse> {
    match value.mode {
        VideoBitrateMode::Preset => Ok(VideoBitrateSettings {
            mode: VideoBitrateMode::Preset,
            preset: Some(value.preset.unwrap_or(VideoBitratePreset::Medium)),
            custom_mbps: None,
        }),
        VideoBitrateMode::Custom => {
            let custom_mbps = value.custom_mbps.ok_or_else(|| CaptureErrorResponse {
                code: "invalid_recording_settings".to_string(),
                message: "videoBitrate.customMbps is required when videoBitrate.mode is custom"
                    .to_string(),
            })?;

            if !(MIN_CUSTOM_VIDEO_BITRATE_MBPS..=MAX_CUSTOM_VIDEO_BITRATE_MBPS)
                .contains(&custom_mbps)
            {
                return Err(CaptureErrorResponse {
                    code: "invalid_recording_settings".to_string(),
                    message: format!(
                        "videoBitrate.customMbps must be between {MIN_CUSTOM_VIDEO_BITRATE_MBPS} and {MAX_CUSTOM_VIDEO_BITRATE_MBPS}"
                    ),
                });
            }

            Ok(VideoBitrateSettings {
                mode: VideoBitrateMode::Custom,
                preset: None,
                custom_mbps: Some(custom_mbps),
            })
        }
    }
}

fn video_bitrate_preset_factor(preset: VideoBitratePreset) -> f64 {
    match preset {
        VideoBitratePreset::Low => 0.07,
        VideoBitratePreset::Medium => 0.10,
        VideoBitratePreset::High => 0.14,
    }
}

fn resolve_bitrate_dimensions(screen_resolution: &ScreenResolution) -> Option<(u32, u32)> {
    match screen_resolution {
        ScreenResolution::Preset { preset } => match preset {
            ScreenResolutionPreset::Original => None,
            ScreenResolutionPreset::P1080 => Some((1920, 1080)),
            ScreenResolutionPreset::P720 => Some((1280, 720)),
            ScreenResolutionPreset::P540 => Some((960, 540)),
        },
        ScreenResolution::Custom { width, height } => Some((*width, *height)),
    }
}

fn clamp_and_round_bitrate_bits_per_second(raw_bps: f64) -> u32 {
    let clamped = raw_bps
        .clamp(
            MIN_EFFECTIVE_VIDEO_BITRATE_BPS as f64,
            MAX_EFFECTIVE_VIDEO_BITRATE_BPS as f64,
        )
        .round() as u64;
    let step = VIDEO_BITRATE_ROUND_STEP_BPS as u64;
    let rounded = ((clamped + (step / 2)) / step) * step;
    rounded as u32
}

fn compute_effective_screen_bitrate_bps(settings: &RecordingSettings) -> Option<u32> {
    if !settings.capture_screen {
        return None;
    }

    let bitrate = match settings.video_bitrate.mode {
        VideoBitrateMode::Custom => {
            let custom_mbps = settings.video_bitrate.custom_mbps? as f64;
            custom_mbps * 1_000_000.0
        }
        VideoBitrateMode::Preset => {
            let preset = settings
                .video_bitrate
                .preset
                .clone()
                .unwrap_or(VideoBitratePreset::Medium);
            let factor = video_bitrate_preset_factor(preset);
            let (width, height) =
                resolve_bitrate_dimensions(&settings.screen_resolution).unwrap_or((1920, 1080));
            (width as f64) * (height as f64) * (settings.screen_frame_rate as f64) * factor
        }
    };

    Some(clamp_and_round_bitrate_bits_per_second(bitrate))
}

fn is_original_screen_resolution(value: &ScreenResolution) -> bool {
    matches!(
        value,
        ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original
        }
    )
}

fn supports_non_original_screen_resolution() -> bool {
    capture_screen::support_for_current_platform().system_audio
}

fn validate_recording_settings(
    request: UpdateRecordingSettingsRequest,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    validate_recording_settings_with_resolution_support(
        request,
        supports_non_original_screen_resolution(),
    )
}

fn validate_recording_settings_with_resolution_support(
    request: UpdateRecordingSettingsRequest,
    non_original_resolution_supported: bool,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    if !request.capture_screen && !request.capture_microphone && !request.capture_system_audio {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "At least one capture source must be enabled".to_string(),
        });
    }

    if request.capture_system_audio && !request.capture_screen {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "System audio capture requires screen capture".to_string(),
        });
    }

    let save_directory = request.save_directory.trim().to_string();
    if save_directory.is_empty() {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "saveDirectory must be non-empty".to_string(),
        });
    }

    if request.segment_duration_seconds == 0 {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "segmentDurationSeconds must be greater than 0".to_string(),
        });
    }

    if !(1..=120).contains(&request.screen_frame_rate) {
        return Err(CaptureErrorResponse {
            code: "invalid_recording_settings".to_string(),
            message: "screenFrameRate must be between 1 and 120".to_string(),
        });
    }

    let screen_resolution = validate_screen_resolution(request.screen_resolution)?;
    let video_bitrate = validate_video_bitrate(request.video_bitrate)?;

    if request.capture_screen
        && !non_original_resolution_supported
        && !is_original_screen_resolution(&screen_resolution)
    {
        return Err(CaptureErrorResponse {
            code: "screen_resolution_unsupported".to_string(),
            message: "Selected screen resolution requires the ScreenCaptureKit backend (macOS 15+). On this backend, only the original display resolution is supported.".to_string(),
        });
    }

    Ok(RecordingSettings {
        capture_screen: request.capture_screen,
        capture_microphone: request.capture_microphone,
        capture_system_audio: request.capture_system_audio,
        segment_duration_seconds: request.segment_duration_seconds,
        screen_frame_rate: request.screen_frame_rate,
        screen_resolution,
        video_bitrate,
        save_directory,
        auto_start: request.auto_start,
    })
}

fn recording_settings_file_path(app_handle: &tauri::AppHandle) -> PathBuf {
    if let Ok(config_dir) = app_handle.path().app_config_dir() {
        return config_dir.join(RECORDING_SETTINGS_FILE_NAME);
    }

    PathBuf::from(default_save_directory()).join(RECORDING_SETTINGS_FILE_NAME)
}

fn load_recording_settings_from_disk(app_handle: &tauri::AppHandle) -> Option<RecordingSettings> {
    let path = recording_settings_file_path(app_handle);
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed = serde_json::from_str::<RecordingSettings>(&raw).ok()?;
    validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: parsed.capture_screen,
        capture_microphone: parsed.capture_microphone,
        capture_system_audio: parsed.capture_system_audio,
        segment_duration_seconds: parsed.segment_duration_seconds,
        screen_frame_rate: parsed.screen_frame_rate,
        screen_resolution: parsed.screen_resolution,
        video_bitrate: parsed.video_bitrate,
        save_directory: parsed.save_directory,
        auto_start: parsed.auto_start,
    })
    .ok()
}

fn persist_recording_settings(
    app_handle: &tauri::AppHandle,
    settings: &RecordingSettings,
) -> Result<(), CaptureErrorResponse> {
    let file_path = recording_settings_file_path(app_handle);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| CaptureErrorResponse {
            code: "io_error".to_string(),
            message: format!("Failed to create settings directory: {error}"),
        })?;
    }

    let serialized =
        serde_json::to_string_pretty(settings).map_err(|error| CaptureErrorResponse {
            code: "serialization_error".to_string(),
            message: format!("Failed to serialize recording settings: {error}"),
        })?;

    std::fs::write(file_path, serialized).map_err(|error| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!("Failed to persist recording settings: {error}"),
    })
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

fn set_current_microphone_output_file(output_files: &mut CaptureOutputFiles, file: String) {
    output_files.microphone_file = Some(file.clone());
    output_files.microphone_files.push(file);
}

fn set_current_screen_output_file(output_files: &mut CaptureOutputFiles, file: String) {
    output_files.screen_file = Some(file.clone());
    output_files.screen_files.push(file);
}

fn set_current_system_audio_output_file(output_files: &mut CaptureOutputFiles, file: String) {
    output_files.system_audio_file = Some(file.clone());
    output_files.system_audio_files.push(file);
}

fn mark_runtime_session_stopped(runtime: &mut NativeCaptureRuntime) {
    runtime.is_running = false;
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

            schedule.sleep_until_next_boundary(clock)
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

        if apply_runtime_signal(&mut runtime, RuntimeSignal::RotateRequested).is_err() {
            mark_runtime_session_failed(&mut runtime);
            break;
        }

        let scheduled_index = schedule.current_segment_index(clock.elapsed());
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
    _request: StartNativeCaptureRequest,
    state: tauri::State<'_, NativeCaptureState>,
    microphone_controller_preferences_state: tauri::State<'_, MicrophoneControllerPreferencesState>,
    recording_settings_state: tauri::State<'_, RecordingSettingsState>,
    app_handle: tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
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
