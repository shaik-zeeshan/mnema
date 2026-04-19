use crate::native_capture_inactivity::InactivityState;
use crate::native_capture_settings::default_recording_settings;
use capture_microphone as microphone_capture;
use capture_runtime::{
    CaptureClock, RuntimeController, RuntimeSignal, RuntimeState, SegmentPlanner, SegmentSchedule,
};
use capture_types::{
    CaptureErrorResponse, CaptureOutputFiles, CaptureSources, CaptureSupportResponse,
    NativeCaptureSession, RecordingSettings, ScreenResolution, StartNativeCaptureRequest,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

use super::segments::FrameArtifactMessage;

#[derive(Debug, Default)]
pub struct NativeCaptureRuntime {
    pub is_running: bool,
    pub session_id: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub requested_sources: Option<CaptureSources>,
    pub current_segment_sources: Option<CaptureSources>,
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
    pub frame_artifact_tx: Option<mpsc::Sender<FrameArtifactMessage>>,
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
pub(crate) struct SegmentLoopControl {
    pub(crate) stop: Arc<AtomicBool>,
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

pub(super) fn request_segment_loop_stop(runtime: &NativeCaptureRuntime) {
    if let Some(control) = runtime.segment_loop_control.as_ref() {
        control.stop.store(true, Ordering::Relaxed);
    }
}

pub(super) fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
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

pub(super) fn now_monotonic_marker_ms() -> u64 {
    now_monotonic_ms().saturating_add(1)
}

pub(super) fn session_from_runtime(runtime: &NativeCaptureRuntime) -> NativeCaptureSession {
    NativeCaptureSession {
        is_running: runtime.is_running,
        is_inactivity_paused: runtime.inactivity.is_paused,
        session_id: runtime.session_id.clone(),
        started_at_unix_ms: runtime.started_at_unix_ms,
        requested_sources: runtime.requested_sources.clone(),
        output_files: runtime.output_files.clone(),
    }
}

pub(super) fn stopped_session_from_runtime(runtime: &NativeCaptureRuntime) -> NativeCaptureSession {
    NativeCaptureSession {
        is_running: false,
        is_inactivity_paused: false,
        session_id: runtime.session_id.clone(),
        started_at_unix_ms: runtime.started_at_unix_ms,
        requested_sources: runtime.requested_sources.clone(),
        output_files: runtime.output_files.clone(),
    }
}

pub(super) fn validate_start_request(
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

pub(super) fn mark_runtime_session_stopped(runtime: &mut NativeCaptureRuntime) {
    runtime.is_running = false;
    runtime.inactivity = InactivityState::default();
    runtime.segment_loop_control = None;
    runtime.capture_clock = None;
    runtime.segment_schedule = None;
    runtime.segment_planner = None;
    runtime.frame_artifact_tx = None;
    runtime.effective_screen_bitrate_bps = None;
    runtime.current_segment_sources = None;
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

pub(super) fn mark_runtime_session_failed(runtime: &mut NativeCaptureRuntime) {
    runtime.is_running = false;
    runtime.inactivity = InactivityState::default();
    runtime.segment_loop_control = None;
    runtime.capture_clock = None;
    runtime.segment_schedule = None;
    runtime.segment_planner = None;
    runtime.frame_artifact_tx = None;
    runtime.effective_screen_bitrate_bps = None;
    runtime.current_segment_sources = None;
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

pub(super) fn apply_runtime_signal(
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

pub(super) fn reset_runtime_after_start_error(runtime: &mut NativeCaptureRuntime) {
    runtime.runtime_controller = RuntimeController::default();
    runtime.runtime_state = RuntimeState::Idle;
}

pub(super) fn should_recover_from_segment_finalize_error(error: &CaptureErrorResponse) -> bool {
    let is_missing_requested_screen_output =
        capture_writers::single_output_processing_failure_detail(
            &error.message,
            &[
                "microphone output conversion failed: ",
                "system audio output conversion failed: ",
                "screen output video-only conversion failed: ",
                "failed to remove intermediate ",
            ],
        )
        .is_some_and(
            crate::native_capture_output::is_missing_requested_screen_output_failure_detail,
        );

    capture_screen::should_recover_from_segment_finalize_error(error)
        || (error.code == "capture_output_processing_failed" && is_missing_requested_screen_output)
}

pub(super) fn has_any_capture_sources(sources: &CaptureSources) -> bool {
    sources.screen || sources.microphone || sources.system_audio
}

pub(super) fn active_sources_for_inactivity_paused_state(
    requested_sources: &CaptureSources,
    screen_paused: bool,
    microphone_paused: bool,
    system_audio_paused: bool,
) -> Option<CaptureSources> {
    // system_audio is captured through the screen session backend, so it
    // requires both the screen session to be live (!screen_paused) AND the
    // system audio family to be active (!system_audio_paused).
    let active_sources = CaptureSources {
        screen: requested_sources.screen && !screen_paused,
        microphone: requested_sources.microphone && !microphone_paused,
        system_audio: requested_sources.system_audio && !system_audio_paused && !screen_paused,
    };

    has_any_capture_sources(&active_sources).then_some(active_sources)
}

pub(super) fn current_segment_sources_for_runtime(
    runtime: &NativeCaptureRuntime,
) -> Option<CaptureSources> {
    if let Some(sources) = runtime.current_segment_sources.clone() {
        return has_any_capture_sources(&sources).then_some(sources);
    }

    #[cfg(target_os = "macos")]
    if runtime.current_segment_output_files.is_some()
        || runtime.active_screen_session.is_some()
        || runtime.active_microphone_session.is_some()
    {
        return runtime.requested_sources.as_ref().and_then(|sources| {
            active_sources_for_inactivity_paused_state(
                sources,
                runtime.inactivity.screen_paused,
                runtime.inactivity.microphone_paused,
                runtime.inactivity.system_audio_paused,
            )
        });
    }

    None
}

pub(super) fn should_rotate_segment(
    current_segment_index: u64,
    scheduled_segment_index: u64,
) -> bool {
    scheduled_segment_index > current_segment_index
}
