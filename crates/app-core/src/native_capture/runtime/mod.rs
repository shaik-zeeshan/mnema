use capture_microphone as microphone_capture;
use capture_runtime::{
    CaptureClock, RuntimeController, RuntimeSignal, RuntimeState, SegmentPlanner, SegmentSchedule,
};
use capture_types::{
    CaptureErrorResponse, CaptureOutputFiles, CaptureSources, CaptureSupportResponse,
    NativeCaptureSession, ScreenResolution, SourceSessions, StartNativeCaptureRequest,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

use super::frame_artifacts::FrameArtifactMessage;
use crate::native_capture_inactivity::InactivityState;
use capture_vad::MicrophoneVadRuntime;

mod planners;
pub use planners::{
    current_segment_sources_for_runtime, ensure_microphone_planner_for_runtime,
    ensure_system_audio_planner_for_runtime, microphone_planner_for_runtime,
    refresh_runtime_planner_dates, screen_planner_for_runtime, system_audio_planner_for_runtime,
};
#[cfg(target_os = "macos")]
pub use planners::{
    microphone_backend_active_for_runtime, microphone_probe_active_for_runtime,
    next_segment_estimate_for_runtime, recordings_root_for_runtime,
    system_audio_probe_active_for_runtime, system_audio_writer_active_for_runtime,
};

#[cfg(target_os = "macos")]
pub const MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS: u8 = 3;

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureSuspensionStatus {
    Retryable,
    RestartRequired,
}

/// Why screen capture was suspended. Both kinds drive the same recovery loop —
/// restart a fresh screen segment once it's possible again — but they retry on
/// different terms.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureSuspensionKind {
    /// A privacy content-filter apply failed. The failure is likely persistent,
    /// so recovery is capped and then escalates to a manual stop/start.
    PrivacyFilter,
    /// The capture display became unavailable (display sleep, screen lock, lid
    /// close, monitor disconnect). This is a transient liveness condition, not an
    /// error, so recovery never gives up: it waits for a display to return and
    /// then resumes capture automatically. Screen-only: system audio runs on a
    /// process tap and stops only while the Mac is away (locked, or every
    /// display asleep), under its own hold (ADR 0021, amended).
    DisplayUnavailable,
    /// The recordings volume is too low on free space to safely open the next
    /// segment file. Like [`CaptureSuspensionKind::DisplayUnavailable`] this is a
    /// transient liveness condition, not an error: recovery never gives up and
    /// auto-resumes once free space recovers above the resume threshold.
    LowDisk,
}

/// A suspension of screen capture that the segment loop keeps trying to recover
/// from. Despite the name it now covers privacy-filter failures, transient
/// display-unavailable conditions, and low disk (see [`CaptureSuspensionKind`]);
/// `kind` selects the retry policy.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSuspension {
    pub kind: CaptureSuspensionKind,
    pub reason: String,
    pub last_error_code: String,
    pub last_error_message: String,
    pub recovery_attempts: u8,
    pub status: CaptureSuspensionStatus,
    /// When the suspension began. Survives recovery-attempt bumps; `None` only
    /// if the wall clock is unreadable.
    pub since: Option<SystemTime>,
}

#[cfg(target_os = "macos")]
impl CaptureSuspension {
    pub fn with_kind(kind: CaptureSuspensionKind, error: &CaptureErrorResponse) -> Self {
        let reason = match kind {
            CaptureSuspensionKind::PrivacyFilter => "privacy_filter_apply_failed",
            CaptureSuspensionKind::DisplayUnavailable => "capture_display_unavailable",
            CaptureSuspensionKind::LowDisk => "capture_low_disk",
        };
        Self {
            kind,
            reason: reason.to_string(),
            last_error_code: error.code.clone(),
            last_error_message: error.message.clone(),
            recovery_attempts: 0,
            status: CaptureSuspensionStatus::Retryable,
            since: Some(SystemTime::now()),
        }
    }

    pub fn can_retry(&self) -> bool {
        if self.status != CaptureSuspensionStatus::Retryable {
            return false;
        }
        match self.kind {
            // A display coming back is expected, not a failure to give up on.
            CaptureSuspensionKind::DisplayUnavailable => true,
            // Free space recovering is expected, not a failure to give up on.
            CaptureSuspensionKind::LowDisk => true,
            CaptureSuspensionKind::PrivacyFilter => {
                self.recovery_attempts < MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS
            }
        }
    }

    pub fn record_recovery_failure(&mut self, error: &CaptureErrorResponse) {
        self.recovery_attempts = self.recovery_attempts.saturating_add(1);
        self.last_error_code = error.code.clone();
        self.last_error_message = error.message.clone();
        // Only privacy-filter failures escalate to "needs a manual restart". A
        // display-unavailable or low-disk suspension keeps retrying so it can
        // resume on its own once the display returns / free space recovers.
        if self.kind == CaptureSuspensionKind::PrivacyFilter
            && self.recovery_attempts >= MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS
        {
            self.status = CaptureSuspensionStatus::RestartRequired;
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
    pub screen_frame_rate: f64,
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
    pub user_capture_paused: bool,
    /// Set while the Mac is locked or every display is asleep, and after until
    /// the microphone device runs again; nothing reopens the microphone
    /// meanwhile. A running device is stopped with its output paused for
    /// inactivity, so activity resumes it afterwards.
    pub microphone_away_hold: bool,
    /// Set while the Mac is away and system audio is requested: the tap is torn
    /// down (its running IOProc keeps the Mac from idle-sleeping) and nothing
    /// starts one until the tick sees the Mac back and rebuilds it.
    pub system_audio_away_hold: bool,
    /// Wall-clock start of the current tap; `None` with no tap. Silence is
    /// judged over this, not the recording's age: time without a tap gave it
    /// nothing to hear.
    pub system_audio_tap_started_at_unix_ms: Option<u64>,
    /// Set while a `WaitForSameDevice` microphone's device is gone, by the
    /// device-change notifier, which clears it when the device returns.
    pub microphone_waiting_for_device: bool,
    /// True while a bounded out-of-band recording (voice enrollment) holds the
    /// microphone. Capture leaves the family alone until it is handed back.
    pub microphone_lent_out: bool,
    pub microphone_vad: MicrophoneVadRuntime,
    /// Per-source session metadata. Populated when a recording starts, cleared on reset.
    pub source_sessions: Option<SourceSessions>,
    /// Wall-clock start of this recording session, for "recording since".
    /// Set once at start, cleared on stop; re-anchoring never touches it.
    pub recording_started_at_unix_ms: Option<u64>,
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
    /// The system-audio capture family: its own Core Audio process tap, with no
    /// screen or display dependency (ADR 0052).
    #[cfg(target_os = "macos")]
    pub active_system_audio_session: Option<super::system_audio::SystemAudioFamily>,
    #[cfg(target_os = "macos")]
    pub capture_suspension: Option<CaptureSuspension>,
    /// Injectable free-space probe used by the low-disk preflight, the rotation
    /// boundary check, and the suspension recovery driver. `None` means "use the
    /// production default" ([`crate::disk_space::default_free_space_probe`], which
    /// reads `fs2::available_space`); tests set a scripted probe so the
    /// suspend/resume logic is exercisable without a real full disk. Kept as an
    /// `Option` so the struct keeps deriving `Default` (a bare `fn` pointer does
    /// not implement `Default`).
    #[cfg(target_os = "macos")]
    pub free_space_probe: Option<crate::disk_space::FreeSpaceProbe>,
    /// Injectable "is the Mac away" probe, read wherever the microphone could
    /// open: `None` means the live lock and display state
    /// ([`super::microphone_away::mac_is_away`]). Tests script it, since they can lock
    /// neither the Mac nor its displays.
    #[cfg(target_os = "macos")]
    pub away_probe: Option<fn() -> bool>,
    /// Stands in for the microphone session's device where the away hold and
    /// the reopen check look at it: a real session needs the hardware and a TCC
    /// grant, so tests cannot hold one.
    #[cfg(all(test, target_os = "macos"))]
    pub test_microphone_device: Option<TestMicrophoneDevice>,
}

#[cfg(all(test, target_os = "macos"))]
#[derive(Debug, Default)]
pub struct TestMicrophoneDevice {
    pub running: bool,
    /// The device is gone: a restart reports it did not come back.
    pub gone: bool,
}

#[cfg(target_os = "macos")]
impl NativeCaptureRuntime {
    /// Resolve the effective free-space probe: the injected one if a test set it,
    /// otherwise the production default. Kept here so the `None`-means-default
    /// convention lives in one place.
    pub fn free_space_probe(&self) -> crate::disk_space::FreeSpaceProbe {
        self.free_space_probe
            .unwrap_or(crate::disk_space::default_free_space_probe)
    }
}

#[derive(Debug, Clone)]
pub struct SegmentLoopControl {
    pub stop: Arc<AtomicBool>,
    wake: Arc<SegmentLoopWake>,
}

#[derive(Debug, Default)]
pub struct SegmentLoopWake {
    notified: Mutex<bool>,
    cv: Condvar,
}

impl SegmentLoopWake {
    fn notify(&self) {
        if let Ok(mut notified) = self.notified.lock() {
            *notified = true;
            self.cv.notify_all();
        }
    }
}

impl SegmentLoopControl {
    pub fn new() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            wake: Arc::new(SegmentLoopWake::default()),
        }
    }

    pub fn notify(&self) {
        self.wake.notify();
    }

    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.notify();
    }

    pub fn wait_timeout(&self, timeout: Duration) {
        if self.stop.load(Ordering::Relaxed) || timeout.is_zero() {
            return;
        }
        let Ok(mut notified) = self.wake.notified.lock() else {
            return;
        };
        if *notified {
            *notified = false;
            return;
        }
        let Ok((mut notified, _)) = self.wake.cv.wait_timeout(notified, timeout) else {
            return;
        };
        *notified = false;
    }
}

pub fn request_segment_loop_stop(runtime: &NativeCaptureRuntime) {
    if let Some(control) = runtime.segment_loop_control.as_ref() {
        control.request_stop();
    }
}

pub fn now_unix_ms() -> u64 {
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

pub fn now_monotonic_marker_ms() -> u64 {
    now_monotonic_ms().saturating_add(1)
}

fn source_session_suffix(raw_session_id: &str) -> String {
    raw_session_id
        .strip_prefix("native-session-")
        .or_else(|| raw_session_id.strip_prefix("session-"))
        .unwrap_or(raw_session_id)
        .replace('-', "_")
}

pub fn prefixed_capture_id(prefix: &str) -> Result<String, CaptureErrorResponse> {
    let raw = capture_screen::new_session_id()?;
    Ok(format!("{prefix}_{}", source_session_suffix(&raw)))
}

pub fn session_from_runtime(runtime: &NativeCaptureRuntime) -> NativeCaptureSession {
    #[cfg(target_os = "macos")]
    let is_low_disk_suspended = runtime
        .capture_suspension
        .as_ref()
        .is_some_and(|s| s.kind == CaptureSuspensionKind::LowDisk);
    #[cfg(not(target_os = "macos"))]
    let is_low_disk_suspended = false;

    NativeCaptureSession {
        is_running: session_reports_running(runtime),
        is_inactivity_paused: runtime.inactivity.is_paused,
        is_user_paused: runtime.user_capture_paused,
        is_low_disk_suspended,
        requested_sources: runtime.requested_sources.clone(),
        output_files: runtime.output_files.clone(),
        source_sessions: runtime.source_sessions.clone(),
        recording_started_at_unix_ms: runtime.recording_started_at_unix_ms,
    }
}

fn session_reports_running(runtime: &NativeCaptureRuntime) -> bool {
    if !runtime.is_running {
        return false;
    }

    if runtime.user_capture_paused {
        return true;
    }

    #[cfg(target_os = "macos")]
    {
        if runtime.capture_suspension.is_some() {
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

pub fn stopped_session_from_runtime(runtime: &NativeCaptureRuntime) -> NativeCaptureSession {
    NativeCaptureSession {
        is_running: false,
        is_inactivity_paused: false,
        is_user_paused: false,
        is_low_disk_suspended: false,
        requested_sources: runtime.requested_sources.clone(),
        output_files: runtime.output_files.clone(),
        source_sessions: runtime.source_sessions.clone(),
        recording_started_at_unix_ms: None,
    }
}

pub fn validate_start_request(
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

    Ok(CaptureSources {
        screen: request.capture_screen,
        microphone: request.capture_microphone,
        system_audio: request.capture_system_audio,
    })
}

pub fn mark_runtime_session_stopped(runtime: &mut NativeCaptureRuntime) {
    runtime.is_running = false;
    runtime.recording_started_at_unix_ms = None;
    runtime.inactivity = InactivityState::default();
    runtime.user_capture_paused = false;
    runtime.microphone_away_hold = false;
    runtime.system_audio_away_hold = false;
    runtime.microphone_waiting_for_device = false;
    // A lend that never came back (its recording panicked) must not hold the
    // next session's microphone.
    runtime.microphone_lent_out = false;
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
        stop_system_audio_session(runtime);
        runtime.capture_suspension = None;
    }

    runtime.runtime_controller = RuntimeController::default();
    runtime.runtime_state = RuntimeState::Idle;
}

/// Tears the tap down, closing its in-flight segment. Terminal for the session.
#[cfg(target_os = "macos")]
pub fn stop_system_audio_session(runtime: &mut NativeCaptureRuntime) {
    runtime.system_audio_tap_started_at_unix_ms = None;
    if let Some(session) = runtime.active_system_audio_session.take() {
        session.stop();
    }
}

#[cfg(test)]
mod segment_loop_control_tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn segment_loop_wait_returns_early_when_notified() {
        let control = SegmentLoopControl::new();
        let worker = control.clone();
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let started = Instant::now();
            worker.wait_timeout(Duration::from_secs(5));
            tx.send(started.elapsed())
                .expect("elapsed wait should be sent");
        });

        std::thread::sleep(Duration::from_millis(20));
        control.notify();
        let elapsed = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("wait should return after notification");
        assert!(elapsed < Duration::from_secs(1));
    }

    #[test]
    fn segment_loop_stop_notifies_waiter() {
        let control = SegmentLoopControl::new();
        let worker = control.clone();
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            worker.wait_timeout(Duration::from_secs(5));
            tx.send(()).expect("stop wake should be sent");
        });

        std::thread::sleep(Duration::from_millis(20));
        control.request_stop();
        rx.recv_timeout(Duration::from_secs(1))
            .expect("wait should return after stop");
    }
}

pub fn mark_runtime_session_failed(runtime: &mut NativeCaptureRuntime) {
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
        stop_system_audio_session(runtime);
        runtime.capture_suspension = None;
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

pub fn apply_runtime_signal(
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

pub fn reset_runtime_after_start_error(runtime: &mut NativeCaptureRuntime) {
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
    runtime.recording_started_at_unix_ms = None;
    #[cfg(target_os = "macos")]
    {
        runtime.recording_file = None;
        runtime.microphone_recording_file = None;
        runtime.system_audio_recording_file = None;
        runtime.active_screen_session = None;
        runtime.active_microphone_session = None;
        stop_system_audio_session(runtime);
        runtime.capture_suspension = None;
    }
    runtime.runtime_controller = RuntimeController::default();
    runtime.runtime_state = RuntimeState::Idle;
}

pub fn should_recover_from_segment_finalize_error(error: &CaptureErrorResponse) -> bool {
    let is_missing_requested_screen_output =
        capture_writers::single_output_processing_failure_detail(
            &error.message,
            &[
                "microphone output conversion failed: ",
                "system audio output conversion failed: ",
                "failed to remove intermediate ",
            ],
        )
        .is_some_and(
            crate::native_capture_output::is_missing_requested_screen_output_failure_detail,
        );

    capture_screen::should_recover_from_segment_finalize_error(error)
        || (error.code == "capture_output_processing_failed" && is_missing_requested_screen_output)
}

pub fn has_any_capture_sources(sources: &CaptureSources) -> bool {
    sources.screen || sources.microphone || sources.system_audio
}

pub fn active_sources_for_inactivity_paused_state(
    requested_sources: &CaptureSources,
    screen_paused: bool,
    microphone_paused: bool,
    system_audio_paused: bool,
) -> Option<CaptureSources> {
    let active_sources = CaptureSources {
        screen: requested_sources.screen && !screen_paused,
        microphone: requested_sources.microphone && !microphone_paused,
        system_audio: requested_sources.system_audio && !system_audio_paused,
    };

    has_any_capture_sources(&active_sources).then_some(active_sources)
}

/// What is still recording while screen capture is suspended.
///
/// Only the screen stops. The microphone keeps its own session, and system audio
/// keeps its own tap — a display going away is not a reason for either to stop
/// (ADR 0021, amended). Low disk is the exception: it stops the microphone too,
/// which is why `microphone_paused`/the live-session check decide rather than the
/// suspension kind.
#[cfg(target_os = "macos")]
pub fn privacy_suspended_sources_for_runtime_state(
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

    let system_audio_active = runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.system_audio)
        && !runtime.inactivity.is_system_audio_paused()
        && !system_audio_stops_with_suspension(runtime);

    let active_sources = CaptureSources {
        screen: false,
        microphone: microphone_active,
        system_audio: system_audio_active,
    };

    has_any_capture_sources(&active_sources).then_some(active_sources)
}

/// Whether the suspension the runtime is entering (or already in) takes the
/// system-audio tap down with the screen.
///
/// Only [`CaptureSuspensionKind::LowDisk`] does, and not because of the screen:
/// every source writes to the same volume (ADR 0040). A display sleeping,
/// locking or disconnecting leaves the tap recording (ADR 0021, amended).
#[cfg(target_os = "macos")]
pub fn system_audio_stops_with_suspension(runtime: &NativeCaptureRuntime) -> bool {
    runtime
        .capture_suspension
        .as_ref()
        .is_some_and(|suspension| suspension.kind == CaptureSuspensionKind::LowDisk)
}

#[cfg(test)]
pub fn should_rotate_segment(current_segment_index: u64, scheduled_segment_index: u64) -> bool {
    scheduled_segment_index > current_segment_index
}

#[cfg(test)]
mod tests;
