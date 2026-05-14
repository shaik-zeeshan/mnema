use capture_microphone as microphone_capture;
use capture_runtime::{
    current_date_prefix, CaptureClock, RuntimeController, RuntimeSignal, RuntimeState,
    SegmentPlanner, SegmentSchedule,
};
use capture_types::{
    CaptureErrorResponse, CaptureOutputFiles, CaptureSources, CaptureSupportResponse,
    NativeCaptureSession, ScreenResolution, SourceSessionMeta, SourceSessions,
    StartNativeCaptureRequest,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

use super::inactivity::InactivityState;
use super::segments::FrameArtifactMessage;
use capture_vad::MicrophoneVadRuntime;

#[cfg(target_os = "macos")]
pub(crate) const MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS: u8 = 3;

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PrivacyCaptureSuspensionStatus {
    Retryable,
    RestartRequired,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrivacyCaptureSuspension {
    pub reason: String,
    pub last_error_code: String,
    pub last_error_message: String,
    pub recovery_attempts: u8,
    pub status: PrivacyCaptureSuspensionStatus,
}

#[cfg(target_os = "macos")]
impl PrivacyCaptureSuspension {
    pub fn new(error: &CaptureErrorResponse) -> Self {
        Self {
            reason: "privacy_filter_apply_failed".to_string(),
            last_error_code: error.code.clone(),
            last_error_message: error.message.clone(),
            recovery_attempts: 0,
            status: PrivacyCaptureSuspensionStatus::Retryable,
        }
    }

    pub fn can_retry(&self) -> bool {
        self.status == PrivacyCaptureSuspensionStatus::Retryable
            && self.recovery_attempts < MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS
    }

    pub fn record_recovery_failure(&mut self, error: &CaptureErrorResponse) {
        self.recovery_attempts = self.recovery_attempts.saturating_add(1);
        self.last_error_code = error.code.clone();
        self.last_error_message = error.message.clone();
        if self.recovery_attempts >= MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS {
            self.status = PrivacyCaptureSuspensionStatus::RestartRequired;
            self.reason = "privacy_recovery_restart_required".to_string();
        }
    }
}

#[derive(Debug, Default)]
pub struct NativeCaptureRuntime {
    pub is_running: bool,
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
    /// Independent output planner for the microphone source. When microphone is a requested
    /// source this holds a planner whose `session_id` differs from the screen session's so
    /// that microphone files use a distinct source-session id in the dated `audio/` output.
    pub microphone_planner: Option<SegmentPlanner>,
    /// Independent output planner for the system-audio source. When system audio is a
    /// requested source this holds a planner whose `session_id` differs from both the screen
    /// and microphone sessions.
    pub system_audio_planner: Option<SegmentPlanner>,
    pub frame_artifact_tx: Option<mpsc::Sender<FrameArtifactMessage>>,
    pub runtime_controller: RuntimeController,
    pub runtime_state: RuntimeState,
    pub inactivity: InactivityState,
    pub microphone_vad: MicrophoneVadRuntime,
    /// Per-source session metadata. Populated when a recording starts, cleared on reset.
    pub source_sessions: Option<SourceSessions>,
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
    #[cfg(target_os = "macos")]
    pub privacy_capture_suspension: Option<PrivacyCaptureSuspension>,
}

#[derive(Debug, Clone)]
pub(crate) struct SegmentLoopControl {
    pub(crate) stop: Arc<AtomicBool>,
}

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

fn source_session_suffix(raw_session_id: &str) -> String {
    raw_session_id
        .strip_prefix("native-session-")
        .or_else(|| raw_session_id.strip_prefix("session-"))
        .unwrap_or(raw_session_id)
        .replace('-', "_")
}

pub(super) fn prefixed_capture_id(prefix: &str) -> Result<String, CaptureErrorResponse> {
    let raw = capture_screen::new_session_id()?;
    Ok(format!("{prefix}_{}", source_session_suffix(&raw)))
}

pub(super) fn session_from_runtime(runtime: &NativeCaptureRuntime) -> NativeCaptureSession {
    NativeCaptureSession {
        is_running: session_reports_running(runtime),
        is_inactivity_paused: runtime.inactivity.is_paused,
        requested_sources: runtime.requested_sources.clone(),
        output_files: runtime.output_files.clone(),
        source_sessions: runtime.source_sessions.clone(),
    }
}

fn session_reports_running(runtime: &NativeCaptureRuntime) -> bool {
    if !runtime.is_running {
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        if runtime.privacy_capture_suspension.is_some() {
            return true;
        }

        if runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.screen)
            && !runtime.inactivity.is_screen_paused()
            && runtime.recording_file.is_none()
            && !capture_screen::screen_capture_session_is_live(
                runtime.active_screen_session.as_ref(),
            )
        {
            return false;
        }
    }

    true
}

pub(super) fn stopped_session_from_runtime(runtime: &NativeCaptureRuntime) -> NativeCaptureSession {
    NativeCaptureSession {
        is_running: false,
        is_inactivity_paused: false,
        requested_sources: runtime.requested_sources.clone(),
        output_files: runtime.output_files.clone(),
        source_sessions: runtime.source_sessions.clone(),
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
    runtime.microphone_vad = MicrophoneVadRuntime::default();
    runtime.segment_loop_control = None;
    runtime.capture_clock = None;
    runtime.segment_schedule = None;
    runtime.segment_planner = None;
    runtime.microphone_planner = None;
    runtime.system_audio_planner = None;
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
        runtime.privacy_capture_suspension = None;
    }

    runtime.runtime_controller = RuntimeController::default();
    runtime.runtime_state = RuntimeState::Idle;
}

pub(super) fn mark_runtime_session_failed(runtime: &mut NativeCaptureRuntime) {
    runtime.is_running = false;
    runtime.inactivity = InactivityState::default();
    runtime.microphone_vad = MicrophoneVadRuntime::default();
    runtime.segment_loop_control = None;
    runtime.capture_clock = None;
    runtime.segment_schedule = None;
    runtime.segment_planner = None;
    runtime.microphone_planner = None;
    runtime.system_audio_planner = None;
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
        runtime.privacy_capture_suspension = None;
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
    runtime.is_running = false;
    runtime.requested_sources = None;
    runtime.current_segment_sources = None;
    runtime.output_files = None;
    #[cfg(target_os = "macos")]
    {
        runtime.current_segment_output_files = None;
    }
    runtime.current_segment_index = 0;
    runtime.effective_screen_bitrate_bps = None;
    runtime.microphone_device_id_for_capture = None;
    runtime.segment_loop_control = None;
    runtime.capture_clock = None;
    runtime.segment_schedule = None;
    runtime.segment_planner = None;
    runtime.microphone_planner = None;
    runtime.system_audio_planner = None;
    runtime.frame_artifact_tx = None;
    runtime.inactivity = InactivityState::default();
    runtime.source_sessions = None;
    #[cfg(target_os = "macos")]
    {
        runtime.recording_file = None;
        runtime.microphone_recording_file = None;
        runtime.system_audio_recording_file = None;
        runtime.active_screen_session = None;
        runtime.active_microphone_session = None;
        runtime.privacy_capture_suspension = None;
    }
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
        .is_some_and(super::output::is_missing_requested_screen_output_failure_detail);

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

#[cfg(target_os = "macos")]
pub(super) fn privacy_suspended_sources_for_runtime_state(
    runtime: &NativeCaptureRuntime,
    microphone_paused: bool,
) -> Option<CaptureSources> {
    let microphone_active = runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.microphone)
        && !microphone_paused
        && (runtime.active_microphone_session.is_some()
            || runtime.microphone_recording_file.is_some());

    let active_sources = CaptureSources {
        screen: false,
        microphone: microphone_active,
        system_audio: false,
    };

    has_any_capture_sources(&active_sources).then_some(active_sources)
}

pub(super) fn screen_planner_for_runtime(
    runtime: &NativeCaptureRuntime,
) -> Option<&SegmentPlanner> {
    runtime.segment_planner.as_ref()
}

pub(super) fn microphone_planner_for_runtime(
    runtime: &NativeCaptureRuntime,
) -> Option<&SegmentPlanner> {
    runtime.microphone_planner.as_ref()
}

pub(super) fn system_audio_planner_for_runtime(
    runtime: &NativeCaptureRuntime,
) -> Option<&SegmentPlanner> {
    runtime.system_audio_planner.as_ref()
}

pub(super) fn refresh_runtime_planner_dates(runtime: &mut NativeCaptureRuntime) -> String {
    let date_prefix = current_date_prefix();

    if let Some(planner) = runtime.segment_planner.as_mut() {
        planner.set_date_prefix(date_prefix.clone());
    }
    if let Some(planner) = runtime.microphone_planner.as_mut() {
        planner.set_date_prefix(date_prefix.clone());
    }
    if let Some(planner) = runtime.system_audio_planner.as_mut() {
        planner.set_date_prefix(date_prefix.clone());
    }

    date_prefix
}

fn seed_source_planner_from_runtime(
    screen_planner: &SegmentPlanner,
    source_session_id: &str,
) -> SegmentPlanner {
    SegmentPlanner::with_date_prefix(
        screen_planner.save_root_dir(),
        source_session_id,
        screen_planner.date_prefix(),
    )
}

fn empty_source_sessions() -> SourceSessions {
    SourceSessions {
        screen: None,
        microphone: None,
        system_audio: None,
    }
}

fn source_session_started_at_seed(runtime: &NativeCaptureRuntime) -> u64 {
    runtime
        .source_sessions
        .as_ref()
        .and_then(|sessions| {
            sessions
                .screen
                .as_ref()
                .or(sessions.microphone.as_ref())
                .or(sessions.system_audio.as_ref())
                .map(|session| session.started_at_unix_ms)
        })
        .unwrap_or_else(now_unix_ms)
}

fn persist_microphone_source_session(
    runtime: &mut NativeCaptureRuntime,
    session_id: String,
) -> SourceSessionMeta {
    let started_at_unix_ms = runtime
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.microphone.as_ref())
        .map(|session| session.started_at_unix_ms)
        .unwrap_or_else(|| source_session_started_at_seed(runtime));
    let source_session = SourceSessionMeta {
        session_id,
        started_at_unix_ms,
    };

    runtime
        .source_sessions
        .get_or_insert_with(empty_source_sessions)
        .microphone = Some(source_session.clone());

    source_session
}

fn persist_system_audio_source_session(
    runtime: &mut NativeCaptureRuntime,
    session_id: String,
) -> SourceSessionMeta {
    let started_at_unix_ms = runtime
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.system_audio.as_ref())
        .map(|session| session.started_at_unix_ms)
        .unwrap_or_else(|| source_session_started_at_seed(runtime));
    let source_session = SourceSessionMeta {
        session_id,
        started_at_unix_ms,
    };

    runtime
        .source_sessions
        .get_or_insert_with(empty_source_sessions)
        .system_audio = Some(source_session.clone());

    source_session
}

pub(super) fn ensure_microphone_planner_for_runtime(
    runtime: &mut NativeCaptureRuntime,
    _context: &str,
) -> Result<Option<SegmentPlanner>, CaptureErrorResponse> {
    if let Some(planner) = runtime.microphone_planner.clone() {
        persist_microphone_source_session(runtime, planner.session_id().to_string());
        return Ok(Some(planner));
    }

    if !runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.microphone)
    {
        return Ok(None);
    }

    let Some(screen_planner) = runtime.segment_planner.clone() else {
        // Screen planner not yet available — cannot seed microphone planner yet.
        return Ok(None);
    };
    let source_session = runtime
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.microphone.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            persist_microphone_source_session(
                runtime,
                prefixed_capture_id("mic").unwrap_or_else(|_| format!("mic_{}", now_unix_ms())),
            )
        });

    let planner = seed_source_planner_from_runtime(&screen_planner, &source_session.session_id);
    runtime.microphone_planner = Some(planner.clone());

    Ok(Some(planner))
}

pub(super) fn ensure_system_audio_planner_for_runtime(
    runtime: &mut NativeCaptureRuntime,
    _context: &str,
) -> Result<Option<SegmentPlanner>, CaptureErrorResponse> {
    if let Some(planner) = runtime.system_audio_planner.clone() {
        persist_system_audio_source_session(runtime, planner.session_id().to_string());
        return Ok(Some(planner));
    }

    if !runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.system_audio)
    {
        return Ok(None);
    }

    let Some(screen_planner) = runtime.segment_planner.clone() else {
        // Screen planner not yet available — cannot seed system-audio planner yet.
        return Ok(None);
    };
    let source_session = runtime
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.system_audio.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            persist_system_audio_source_session(
                runtime,
                prefixed_capture_id("sysaudio")
                    .unwrap_or_else(|_| format!("sysaudio_{}", now_unix_ms())),
            )
        });

    let planner = seed_source_planner_from_runtime(&screen_planner, &source_session.session_id);
    runtime.system_audio_planner = Some(planner.clone());

    Ok(Some(planner))
}

pub(super) fn current_segment_sources_for_runtime(
    runtime: &NativeCaptureRuntime,
) -> Option<CaptureSources> {
    #[cfg(target_os = "macos")]
    if runtime.privacy_capture_suspension.is_some() {
        return privacy_suspended_sources_for_runtime_state(
            runtime,
            runtime.inactivity.is_microphone_paused(),
        );
    }

    if let Some(sources) = runtime.current_segment_sources.clone() {
        return has_any_capture_sources(&sources).then_some(sources);
    }

    #[cfg(target_os = "macos")]
    if runtime.current_segment_output_files.is_some()
        || capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref())
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

#[cfg(target_os = "macos")]
pub(super) fn microphone_backend_active_for_runtime(runtime: &NativeCaptureRuntime) -> bool {
    !runtime.inactivity.is_microphone_paused()
        && runtime.active_microphone_session.is_some()
        && runtime.microphone_recording_file.is_some()
        && current_segment_sources_for_runtime(runtime).is_some_and(|sources| sources.microphone)
}

#[cfg(target_os = "macos")]
pub(super) fn microphone_probe_active_for_runtime(runtime: &NativeCaptureRuntime) -> bool {
    runtime.active_microphone_session.is_some()
}

#[cfg(target_os = "macos")]
pub(super) fn system_audio_writer_active_for_runtime(runtime: &NativeCaptureRuntime) -> bool {
    !runtime.inactivity.is_system_audio_paused()
        && !runtime.inactivity.is_screen_paused()
        && capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref())
        && runtime.system_audio_recording_file.is_some()
        && current_segment_sources_for_runtime(runtime).is_some_and(|sources| sources.system_audio)
}

#[cfg(test)]
pub(super) fn should_rotate_segment(
    current_segment_index: u64,
    scheduled_segment_index: u64,
) -> bool {
    scheduled_segment_index > current_segment_index
}

#[cfg(test)]
mod tests {
    use super::source_session_suffix;
    #[cfg(target_os = "macos")]
    use super::{
        PrivacyCaptureSuspension, PrivacyCaptureSuspensionStatus,
        MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS,
    };
    #[cfg(target_os = "macos")]
    use capture_types::CaptureErrorResponse;

    #[test]
    fn source_session_suffix_removes_native_session_prefix() {
        assert_eq!(
            source_session_suffix("native-session-ceb00964-9039-4e1c-a770-c2c1a1251e83"),
            "ceb00964_9039_4e1c_a770_c2c1a1251e83"
        );
    }

    #[test]
    fn source_session_suffix_keeps_legacy_session_prefix_compatibility() {
        assert_eq!(
            source_session_suffix("session-ceb00964-9039-4e1c-a770-c2c1a1251e83"),
            "ceb00964_9039_4e1c_a770_c2c1a1251e83"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn privacy_capture_suspension_requires_restart_after_bounded_failures() {
        let error = CaptureErrorResponse {
            code: "privacy_filter_apply_failed".to_string(),
            message: "filter failed".to_string(),
        };
        let mut suspension = PrivacyCaptureSuspension::new(&error);

        for _ in 0..MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS {
            assert!(suspension.can_retry());
            suspension.record_recovery_failure(&error);
        }

        assert!(!suspension.can_retry());
        assert_eq!(
            suspension.status,
            PrivacyCaptureSuspensionStatus::RestartRequired
        );
        assert_eq!(suspension.reason, "privacy_recovery_restart_required");
    }
}
