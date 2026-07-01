//! On-device smoke harness for Windows transient-liveness recovery
//! (ADR 0023, GitHub issues #62/#63/#64, label `needs-smoke-test`).
//!
//! This is a *true on-device* harness like `windows_inactivity_smoke`: it cannot
//! synthesize monitor disconnects, workstation locks, or system sleep, so it drives
//! a real native capture and then prompts the operator (on stdout) to perform the
//! transient-liveness action while the harness watches runtime bookkeeping.
//!
//! Run from the repo with:
//! ```text
//! cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-transient-liveness-smoke
//! cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-transient-liveness-smoke --session-lock
//! cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-transient-liveness-smoke --system-suspend
//! ```
//!
//! Sequence:
//!   1. Start a screen + microphone recording. Session-lock and system-suspend
//!      modes also require independent system audio so Issues #63/#64 audio
//!      continuity/restart coverage is verified.
//!   2. Prompt the operator to disconnect the monitor, press Win+L, or sleep/wake
//!      the system; wait (bounded) for the selected transient-liveness pause while
//!      the session stays running.
//!   3. Prompt the operator to reconnect/unlock/confirm wake; wait (bounded) for
//!      capture to resume into a fresh segment with live output again.
//!   4. Stop capture, verify finalized output files, print the output directory.
//!
//! Exits non-zero on any timeout or failed assertion, zero on success, and
//! finalizes/cleans up like the inactivity smoke. On-device execution (the actual
//! unplug/replug) is left to the operator.

#[cfg(target_os = "windows")]
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicI32, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(target_os = "windows")]
use capture_types::{
    AudioSpeechDetectionSettings, AudioSpeechDetector, CaptureOutputFiles, CaptureSources,
    InactivityActivityMode, StartNativeCaptureRequest,
};
#[cfg(target_os = "windows")]
use tauri::Manager;

#[cfg(target_os = "windows")]
const SMOKE_ARG: &str = "--windows-transient-liveness-smoke";
#[cfg(target_os = "windows")]
const DEFAULT_MAX_DISCONNECT_WAIT_SECONDS: u64 = 90;
#[cfg(target_os = "windows")]
const DEFAULT_MAX_RECONNECT_WAIT_SECONDS: u64 = 90;

#[cfg(target_os = "windows")]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum SmokeScenario {
    DisplayUnavailable,
    SessionLock,
    SystemSuspend,
}

#[cfg(target_os = "windows")]
#[derive(Debug)]
struct SmokeConfig {
    scenario: SmokeScenario,
    max_disconnect_wait_seconds: u64,
    max_reconnect_wait_seconds: u64,
    save_directory: PathBuf,
}

#[cfg(target_os = "windows")]
#[derive(Debug, Clone)]
struct RuntimeSmokeSnapshot {
    session_is_running: bool,
    requested_sources: Option<CaptureSources>,
    source_sessions: Option<capture_types::SourceSessions>,
    output_files: Option<CaptureOutputFiles>,
    current_segment_output_files: Option<CaptureOutputFiles>,
    current_segment_index: u64,
    screen_paused: bool,
    microphone_paused: bool,
    system_audio_paused: bool,
    screen_paused_for_transient_display_unavailable: bool,
    screen_paused_for_transient_session_lock: bool,
    screen_paused_for_transient_system_suspend: bool,
    active_screen_session: bool,
    active_microphone_session: bool,
    active_system_audio_session: bool,
}

#[cfg(target_os = "windows")]
pub(crate) fn maybe_run_from_args_and_exit() {
    let args = std::env::args().collect::<Vec<_>>();
    if !args.iter().any(|arg| arg == SMOKE_ARG) {
        return;
    }

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        std::process::exit(0);
    }

    let config = match SmokeConfig::from_args(&args) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Windows transient-liveness smoke configuration error: {error}");
            print_usage();
            std::process::exit(2);
        }
    };

    let exit_code = Arc::new(AtomicI32::new(1));
    let setup_exit_code = Arc::clone(&exit_code);
    let app = tauri::Builder::default()
        .manage(super::NativeCaptureState::default())
        .manage(super::MicrophoneControllerPreferencesState::default())
        .manage(super::MicrophoneDeviceChangeNotifierState::default())
        .manage(super::SystemWakeNotifierState::default())
        .manage(super::MetadataNotifierState::default())
        .manage(super::PrivacyFilterRefreshState::default())
        .manage(super::RecordingSettingsState::default())
        .manage(super::CaptureMetadataState::default())
        .manage(super::AppNotificationsState::default())
        .manage(crate::status_bar::StatusBarState::default())
        .manage(crate::keyboard_bindings::KeyboardBindingsState::default())
        .manage(crate::one_time_prompts::OneTimePromptStateStore::default())
        .manage(crate::app_updates::AppUpdateSettingsState::default())
        .manage(crate::app_updates::AppUpdateRuntimeState::default())
        .manage(crate::audio_transcription_models::AudioTranscriptionModelDownloadState::default())
        .manage(crate::speaker_analysis_models::SpeakerAnalysisModelDownloadState::default())
        .manage(crate::ocr_models::OcrModelDownloadState::default())
        .manage(crate::windows::OnboardingStateStore::default())
        .manage(crate::windows::AppExitCoordinatorState::default())
        .manage(crate::BrokerOpenCaptureResultState::default())
        .manage(crate::broker_authorization_channel::BrokerAuthorizationChannelState::default())
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .setup(move |app| {
            let result = run_smoke(app, &config);
            let code = match result {
                Ok(()) => {
                    println!("Windows transient-liveness smoke: PASS");
                    0
                }
                Err(error) => {
                    eprintln!("Windows transient-liveness smoke: FAIL: {error}");
                    1
                }
            };
            setup_exit_code.store(code, Ordering::SeqCst);
            app.handle().exit(code);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Windows transient-liveness smoke Tauri application");

    app.run(|_, _| {});
    std::process::exit(exit_code.load(Ordering::SeqCst));
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn maybe_run_from_args_and_exit() {
    if std::env::args().any(|arg| arg == "--windows-transient-liveness-smoke") {
        eprintln!("Windows transient-liveness smoke is Windows-only");
        std::process::exit(2);
    }
}

#[cfg(target_os = "windows")]
impl SmokeConfig {
    fn from_args(args: &[String]) -> Result<Self, String> {
        let mut scenario = SmokeScenario::DisplayUnavailable;
        let mut max_disconnect_wait_seconds = DEFAULT_MAX_DISCONNECT_WAIT_SECONDS;
        let mut max_reconnect_wait_seconds = DEFAULT_MAX_RECONNECT_WAIT_SECONDS;
        let mut save_directory = default_smoke_save_directory();
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                SMOKE_ARG => {}
                "--session-lock" => {
                    scenario = SmokeScenario::SessionLock;
                }
                "--system-suspend" => {
                    scenario = SmokeScenario::SystemSuspend;
                }
                "--scenario" => {
                    index += 1;
                    let value = args
                        .get(index)
                        .ok_or_else(|| "--scenario requires a value".to_string())?;
                    scenario = SmokeScenario::from_arg(value)?;
                }
                "--display-unavailable" => {
                    scenario = SmokeScenario::DisplayUnavailable;
                }
                "--max-disconnect-wait-seconds" => {
                    index += 1;
                    max_disconnect_wait_seconds =
                        parse_u64_arg(args, index, "--max-disconnect-wait-seconds")?;
                }
                "--max-reconnect-wait-seconds" => {
                    index += 1;
                    max_reconnect_wait_seconds =
                        parse_u64_arg(args, index, "--max-reconnect-wait-seconds")?;
                }
                "--save-directory" => {
                    index += 1;
                    let value = args
                        .get(index)
                        .ok_or_else(|| "--save-directory requires a value".to_string())?;
                    save_directory = PathBuf::from(value);
                }
                arg if index == 0 || arg == "--" => {}
                other => return Err(format!("unknown argument: {other}")),
            }
            index += 1;
        }

        if max_disconnect_wait_seconds == 0 {
            return Err("--max-disconnect-wait-seconds must be at least 1".to_string());
        }
        if max_reconnect_wait_seconds == 0 {
            return Err("--max-reconnect-wait-seconds must be at least 1".to_string());
        }

        Ok(Self {
            scenario,
            max_disconnect_wait_seconds,
            max_reconnect_wait_seconds,
            save_directory,
        })
    }
}

#[cfg(target_os = "windows")]
fn parse_u64_arg(args: &[String], index: usize, name: &str) -> Result<u64, String> {
    args.get(index)
        .ok_or_else(|| format!("{name} requires a value"))?
        .parse::<u64>()
        .map_err(|error| format!("{name} must be an integer: {error}"))
}

#[cfg(target_os = "windows")]
fn default_smoke_save_directory() -> PathBuf {
    std::env::temp_dir().join(format!(
        "mnema-windows-transient-liveness-smoke-{}",
        std::process::id()
    ))
}

#[cfg(target_os = "windows")]
impl SmokeScenario {
    fn from_arg(value: &str) -> Result<Self, String> {
        match value {
            "display-unavailable" => Ok(Self::DisplayUnavailable),
            "session-lock" => Ok(Self::SessionLock),
            "system-suspend" => Ok(Self::SystemSuspend),
            other => Err(format!(
                "unknown --scenario value: {other}; expected display-unavailable, session-lock, or system-suspend"
            )),
        }
    }

    fn as_arg(self) -> &'static str {
        match self {
            Self::DisplayUnavailable => "display-unavailable",
            Self::SessionLock => "session-lock",
            Self::SystemSuspend => "system-suspend",
        }
    }

    fn pause_label(self) -> &'static str {
        match self {
            Self::DisplayUnavailable => "TransientLiveness { DisplayUnavailable }",
            Self::SessionLock => "TransientLiveness { SessionLock }",
            Self::SystemSuspend => "TransientLiveness { SystemSuspend }",
        }
    }
}

#[cfg(target_os = "windows")]
fn print_usage() {
    println!(
        "Windows transient-liveness smoke (ADR 0023, issues #62/#63/#64)\n\nRun from the repo with:\n  cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-transient-liveness-smoke\n  cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-transient-liveness-smoke --session-lock\n  cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-transient-liveness-smoke --system-suspend\n\nDefault scenario: display-unavailable (#62). It starts real screen + microphone capture, asks you to physically disconnect/reconnect a monitor, verifies the screen enters TransientLiveness {{ DisplayUnavailable }}, verifies microphone continuity, then verifies automatic screen resume.\n\nSession-lock scenario (#63): add --session-lock or --scenario session-lock. It starts real screen + microphone + independent system-audio when available, asks you to press Win+L/unlock, verifies TransientLiveness {{ SessionLock }}, verifies audio continuity, then verifies automatic screen resume.\n\nSystem-suspend scenario (#64): add --system-suspend or --scenario system-suspend. It starts real screen + microphone + independent system-audio when available, asks you to sleep and wake the machine manually, verifies all requested families enter TransientLiveness {{ SystemSuspend }} without ending the session, then verifies all families restart after wake.\n\nOptions:\n  --scenario <display-unavailable|session-lock|system-suspend>\n  --display-unavailable\n  --session-lock\n  --system-suspend\n  --max-disconnect-wait-seconds <seconds>\n  --max-reconnect-wait-seconds <seconds>\n  --save-directory <path>\n"
    );
}

#[cfg(target_os = "windows")]
fn run_smoke(app: &mut tauri::App, config: &SmokeConfig) -> Result<(), String> {
    println!(
        "Windows transient-liveness smoke: configuring isolated capture root at {}",
        config.save_directory.display()
    );
    let support = super::get_capture_support();
    if support.platform != "windows" {
        return Err(format!(
            "expected Windows capture platform, observed {}",
            support.platform
        ));
    }
    if !support.supported_sources.screen {
        return Err("transient-liveness smoke requires screen capture support".to_string());
    }
    if !support.supported_sources.microphone {
        return Err(
            "transient-liveness smoke requires microphone support to prove audio continuity"
                .to_string(),
        );
    }
    if matches!(
        config.scenario,
        SmokeScenario::SessionLock | SmokeScenario::SystemSuspend
    ) && !support.supported_sources.system_audio
    {
        return Err(format!(
            "{} transient-liveness smoke requires Windows system-audio support; support reported supported_sources.system_audio=false, so audio coverage cannot be verified",
            config.scenario.as_arg()
        ));
    }

    let sources = CaptureSources {
        screen: true,
        microphone: true,
        system_audio: matches!(
            config.scenario,
            SmokeScenario::SessionLock | SmokeScenario::SystemSuspend
        ),
    };
    install_smoke_recording_settings(app.handle(), config, &sources)?;
    crate::app_infra::initialize(app).map_err(|error| {
        format!("failed to initialize app infra for smoke capture root: {error}")
    })?;
    println!(
        "Windows transient-liveness smoke: scenario={} capturing screen + microphone + system_audio={}",
        config.scenario.as_arg(),
        sources.system_audio
    );

    let app_handle = app.handle().clone();
    let start_response = super::start_native_capture_inner(
        "windows-transient-liveness-smoke",
        StartNativeCaptureRequest {
            capture_screen: sources.screen,
            capture_microphone: sources.microphone,
            capture_system_audio: sources.system_audio,
        },
        app_handle.state::<super::NativeCaptureState>(),
        app_handle.state::<super::MicrophoneControllerPreferencesState>(),
        app_handle.state::<super::RecordingSettingsState>(),
        app_handle.state::<super::AppNotificationsState>(),
        app_handle.clone(),
    )
    .map_err(|error| {
        format!(
            "failed to start native capture: [{}] {}",
            error.code, error.message
        )
    })?;
    super::emit_native_capture_session_changed(&app_handle, &start_response.session);
    crate::status_bar::refresh(&app_handle);

    let start_snapshot = snapshot(&app_handle);
    verify_started_snapshot(&start_snapshot, &sources)?;
    println!(
        "Windows transient-liveness smoke: started segment {} with source sessions {}",
        start_snapshot.current_segment_index,
        format_source_sessions(start_snapshot.source_sessions.as_ref())
    );
    println!(
        "Windows transient-liveness smoke: initial active output paths {}",
        format_output_files(start_snapshot.current_segment_output_files.as_ref())
    );

    match config.scenario {
        SmokeScenario::DisplayUnavailable => println!(
            "\n>>> ACTION REQUIRED (#62): DISCONNECT the monitor now (unplug it, turn it off, or close the lid if that surfaces DisplayUnavailable).\n>>> Waiting up to {}s for the screen to enter TransientLiveness {{ DisplayUnavailable }}...",
            config.max_disconnect_wait_seconds
        ),
        SmokeScenario::SessionLock => println!(
            "\n>>> ACTION REQUIRED (#63): make sure screen + microphone + system audio are recording; press Win+L now, then unlock and return to this desktop.\n>>> Waiting up to {}s for the screen to enter TransientLiveness {{ SessionLock }} while microphone and system audio keep recording...",
            config.max_disconnect_wait_seconds
        ),
        SmokeScenario::SystemSuspend => println!(
            "\n>>> ACTION REQUIRED (#64): put Windows to sleep now, then wake it manually before the timeout expires. Do not use this from automation.\n>>> Waiting up to {}s after wake to observe all families paused as TransientLiveness {{ SystemSuspend }} while the session remains alive...",
            config.max_disconnect_wait_seconds
        ),
    }
    let paused_snapshot = wait_for_snapshot(
        &app_handle,
        Duration::from_secs(config.max_disconnect_wait_seconds),
        |snapshot| transient_screen_suspended(snapshot, &sources, config.scenario),
    )
    .inspect_err(|_| {
        let _ = stop_after_failure(&app_handle);
    })?;
    verify_transient_paused_snapshot(&start_snapshot, &paused_snapshot, &sources, config.scenario)?;
    println!(
        "Windows transient-liveness smoke: observed {}; session still running; committed outputs {}",
        config.scenario.pause_label(),
        format_output_files(paused_snapshot.output_files.as_ref())
    );

    match config.scenario {
        SmokeScenario::DisplayUnavailable => println!(
            "\n>>> ACTION REQUIRED (#62): RECONNECT the monitor now.\n>>> Waiting up to {}s for the screen to auto-resume...",
            config.max_reconnect_wait_seconds
        ),
        SmokeScenario::SessionLock => println!(
            "\n>>> ACTION REQUIRED (#63): confirm the workstation is unlocked, then keep the capture running.\n>>> Waiting up to {}s for the screen to auto-resume...",
            config.max_reconnect_wait_seconds
        ),
        SmokeScenario::SystemSuspend => println!(
            "\n>>> ACTION REQUIRED (#64): the system is awake; keep the desktop unlocked and capture running.\n>>> Waiting up to {}s for all families to restart after SystemSuspend...",
            config.max_reconnect_wait_seconds
        ),
    }
    let resumed_snapshot = wait_for_snapshot(
        &app_handle,
        Duration::from_secs(config.max_reconnect_wait_seconds),
        |snapshot| screen_resumed(snapshot, &sources),
    )
    .inspect_err(|_| {
        let _ = stop_after_failure(&app_handle);
    })?;
    verify_resumed_snapshot(
        &start_snapshot,
        &resumed_snapshot,
        &sources,
        config.scenario,
    )?;
    println!(
        "Windows transient-liveness smoke: observed screen resume into segment {}; active output paths {}",
        resumed_snapshot.current_segment_index,
        format_output_files(resumed_snapshot.current_segment_output_files.as_ref())
    );

    let stop_response = super::stop_native_capture_with_state(
        app_handle.state::<super::NativeCaptureState>(),
        &app_handle,
    )
    .map_err(|error| {
        format!(
            "failed to stop native capture: [{}] {}",
            error.code, error.message
        )
    })?;
    super::emit_native_capture_session_changed(&app_handle, &stop_response.session);
    crate::status_bar::refresh(&app_handle);
    if stop_response.session.is_running {
        return Err("stop returned a still-running native capture session".to_string());
    }
    println!(
        "Windows transient-liveness smoke: stopped and finalized outputs {}",
        format_output_files(stop_response.session.output_files.as_ref())
    );
    verify_final_outputs(stop_response.session.output_files.as_ref(), &sources)?;
    verify_capture_invariants(stop_response.session.output_files.as_ref(), &sources)?;
    println!(
        "Windows transient-liveness smoke: output directory {}",
        config.save_directory.display()
    );

    Ok(())
}

/// Assert the milestone capture invariants (#84) over the finalized artifacts.
///
/// This scenario performs a *normal* stop, so the only invariant that applies is
/// the #73 frame-index sidecar: every finalized screen segment — including the
/// pre-pause segment and the fresh post-resume segment the transient recovery
/// rotated into — must carry a monotonic frame-index sidecar. The #74
/// inactivity-tail invariant is exercised by the inactivity smoke instead, which
/// is the harness that can produce an inactivity stop.
#[cfg(target_os = "windows")]
fn verify_capture_invariants(
    outputs: Option<&CaptureOutputFiles>,
    requested: &CaptureSources,
) -> Result<(), String> {
    let outputs = outputs
        .ok_or_else(|| "final output bookkeeping missing; cannot verify capture invariants".to_string())?;
    if requested.screen {
        super::windows_smoke_invariants::assert_all_screen_segments_have_monotonic_sidecars(
            &outputs.screen_files,
        )?;
        println!(
            "Windows transient-liveness smoke: verified monotonic frame-index sidecars (#73) for {} finalized screen segment(s)",
            outputs.screen_files.len()
        );
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn install_smoke_recording_settings(
    app_handle: &tauri::AppHandle,
    config: &SmokeConfig,
    sources: &CaptureSources,
) -> Result<(), String> {
    let mut settings = super::settings::default_recording_settings();
    settings.capture_screen = sources.screen;
    settings.capture_microphone = sources.microphone;
    settings.capture_system_audio = sources.system_audio;
    settings.segment_duration_seconds = 300;
    settings.screen_frame_rate = 1;
    settings.save_directory = config.save_directory.to_string_lossy().to_string();
    settings.auto_start = false;
    // Disable inactivity pause so the only screen pause we can observe is the
    // transient-liveness suspension under test.
    settings.pause_capture_on_inactivity = false;
    settings.idle_timeout_seconds = 600;
    settings.inactivity_activity_mode = InactivityActivityMode::SystemInputOnly;
    settings.ocr.enabled = false;
    settings.transcription.enabled = false;
    settings.transcription.microphone_enabled = false;
    settings.transcription.system_audio_enabled = false;
    settings.speaker_analysis.separate_speakers = false;
    settings.speaker_analysis.recognize_saved_people = false;
    settings.audio_speech_detection = AudioSpeechDetectionSettings {
        detector: AudioSpeechDetector::Off,
    };
    settings.microphone_vad_adapter = AudioSpeechDetector::Off;
    settings.metadata.enabled = false;
    settings.native_capture_debug_logging_enabled = true;

    std::fs::create_dir_all(&config.save_directory)
        .map_err(|error| format!("failed to create smoke save directory: {error}"))?;
    let state = app_handle.state::<super::RecordingSettingsState>();
    let mut runtime = state
        .lock()
        .map_err(|_| "recording settings state poisoned".to_string())?;
    runtime.settings = settings;
    Ok(())
}

#[cfg(target_os = "windows")]
fn wait_for_snapshot(
    app_handle: &tauri::AppHandle,
    timeout: Duration,
    predicate: impl Fn(&RuntimeSmokeSnapshot) -> bool,
) -> Result<RuntimeSmokeSnapshot, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let current = snapshot(app_handle);
        if !current.session_is_running {
            return Err(format!(
                "native capture session ended unexpectedly while waiting; latest snapshot: {current:?}"
            ));
        }
        if predicate(&current) {
            return Ok(current);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for expected runtime state; latest snapshot: {current:?}"
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(target_os = "windows")]
fn snapshot(app_handle: &tauri::AppHandle) -> RuntimeSmokeSnapshot {
    use super::inactivity::{ScreenPauseReason, TransientLivenessTrigger};

    let state = app_handle.state::<super::NativeCaptureState>();
    let runtime = state.lock().expect("native capture state poisoned");
    let runtime = runtime.runtime();
    let screen_paused_for_transient_display_unavailable = matches!(
        runtime.inactivity.screen_pause_reason(),
        Some(ScreenPauseReason::TransientLiveness {
            trigger: TransientLivenessTrigger::DisplayUnavailable,
        })
    );
    let screen_paused_for_transient_session_lock = matches!(
        runtime.inactivity.screen_pause_reason(),
        Some(ScreenPauseReason::TransientLiveness {
            trigger: TransientLivenessTrigger::SessionLock,
        })
    );
    let screen_paused_for_transient_system_suspend = matches!(
        runtime.inactivity.screen_pause_reason(),
        Some(ScreenPauseReason::TransientLiveness {
            trigger: TransientLivenessTrigger::SystemSuspend,
        })
    );
    RuntimeSmokeSnapshot {
        session_is_running: runtime.is_running,
        requested_sources: runtime.requested_sources.clone(),
        source_sessions: runtime.source_sessions.clone(),
        output_files: runtime.output_files.clone(),
        current_segment_output_files: runtime.current_segment_output_files.clone(),
        current_segment_index: runtime.current_segment_index,
        screen_paused: runtime.inactivity.screen_paused,
        microphone_paused: runtime.inactivity.microphone_paused,
        system_audio_paused: runtime.inactivity.system_audio_paused,
        screen_paused_for_transient_display_unavailable,
        screen_paused_for_transient_session_lock,
        screen_paused_for_transient_system_suspend,
        active_screen_session: capture_screen::screen_capture_session_is_live(
            runtime.active_screen_session.as_ref(),
        ),
        active_microphone_session: runtime
            .active_microphone_session
            .as_ref()
            .is_some_and(|session| session.is_live()),
        active_system_audio_session: runtime
            .active_system_audio_session
            .as_ref()
            .is_some_and(|session| session.is_live()),
    }
}

#[cfg(target_os = "windows")]
fn verify_started_snapshot(
    snapshot: &RuntimeSmokeSnapshot,
    requested: &CaptureSources,
) -> Result<(), String> {
    if !snapshot.session_is_running {
        return Err("native capture did not report running after start".to_string());
    }
    if snapshot.requested_sources.as_ref() != Some(requested) {
        return Err(format!(
            "started requested sources mismatch: expected {requested:?}, got {:?}",
            snapshot.requested_sources
        ));
    }
    if requested.screen && !snapshot.active_screen_session {
        return Err("screen source has no live session at start".to_string());
    }
    if requested.microphone && !snapshot.active_microphone_session {
        return Err("microphone source has no live session at start".to_string());
    }
    if requested.system_audio && !snapshot.active_system_audio_session {
        return Err("system-audio source has no live session at start".to_string());
    }
    Ok(())
}

// The selected transient-liveness suspension condition: the screen carries the
// selected trigger, the session is still running, and the expected family liveness
// matches the scenario. DisplayUnavailable/SessionLock are screen-only outages;
// SystemSuspend is an all-family outage.
#[cfg(target_os = "windows")]
fn transient_screen_suspended(
    snapshot: &RuntimeSmokeSnapshot,
    requested: &CaptureSources,
    scenario: SmokeScenario,
) -> bool {
    if !snapshot.session_is_running
        || !snapshot.screen_paused
        || !screen_paused_for_scenario(snapshot, scenario)
        || snapshot.active_screen_session
    {
        return false;
    }

    match scenario {
        SmokeScenario::DisplayUnavailable | SmokeScenario::SessionLock => {
            (!requested.microphone
                || (!snapshot.microphone_paused && snapshot.active_microphone_session))
                && (!requested.system_audio
                    || (!snapshot.system_audio_paused && snapshot.active_system_audio_session))
        }
        SmokeScenario::SystemSuspend => {
            (!requested.microphone
                || (snapshot.microphone_paused && !snapshot.active_microphone_session))
                && (!requested.system_audio
                    || (snapshot.system_audio_paused && !snapshot.active_system_audio_session))
        }
    }
}

#[cfg(target_os = "windows")]
fn screen_paused_for_scenario(snapshot: &RuntimeSmokeSnapshot, scenario: SmokeScenario) -> bool {
    match scenario {
        SmokeScenario::DisplayUnavailable => {
            snapshot.screen_paused_for_transient_display_unavailable
        }
        SmokeScenario::SessionLock => snapshot.screen_paused_for_transient_session_lock,
        SmokeScenario::SystemSuspend => snapshot.screen_paused_for_transient_system_suspend,
    }
}

#[cfg(target_os = "windows")]
fn verify_transient_paused_snapshot(
    before: &RuntimeSmokeSnapshot,
    snapshot: &RuntimeSmokeSnapshot,
    requested: &CaptureSources,
    scenario: SmokeScenario,
) -> Result<(), String> {
    if !snapshot.session_is_running {
        return Err("session was not running during the transient pause".to_string());
    }
    if !screen_paused_for_scenario(snapshot, scenario) {
        return Err(format!(
            "screen was not paused for {}: {snapshot:?}",
            scenario.pause_label()
        ));
    }
    if snapshot.active_screen_session {
        return Err("screen session remained live while transient-paused".to_string());
    }
    match scenario {
        SmokeScenario::DisplayUnavailable | SmokeScenario::SessionLock => {
            if requested.microphone && snapshot.microphone_paused {
                return Err(
                    "microphone was paused during a screen-only transient outage".to_string(),
                );
            }
            if requested.microphone && !snapshot.active_microphone_session {
                return Err(
                    "microphone session went down during a screen-only transient outage; audio continuity broken"
                        .to_string(),
                );
            }
            if requested.system_audio && snapshot.system_audio_paused {
                return Err(
                    "system audio was paused during a screen-only transient outage".to_string(),
                );
            }
            if requested.system_audio && !snapshot.active_system_audio_session {
                return Err(
                    "system-audio session went down during a screen-only transient outage; audio continuity broken"
                        .to_string(),
                );
            }
            verify_audio_source_session_ids_stable(before, snapshot, requested, "transient pause")?;
        }
        SmokeScenario::SystemSuspend => {
            if requested.microphone
                && (!snapshot.microphone_paused || snapshot.active_microphone_session)
            {
                return Err(
                    "microphone did not suspend during SystemSuspend transient outage".to_string(),
                );
            }
            if requested.system_audio
                && (!snapshot.system_audio_paused || snapshot.active_system_audio_session)
            {
                return Err(
                    "system audio did not suspend during SystemSuspend transient outage"
                        .to_string(),
                );
            }
        }
    }
    Ok(())
}

// The screen resumed: not paused, a live screen session again, the session still
// running, and a fresh emitted segment.
#[cfg(target_os = "windows")]
fn screen_resumed(snapshot: &RuntimeSmokeSnapshot, requested: &CaptureSources) -> bool {
    snapshot.session_is_running
        && !snapshot.screen_paused
        && snapshot.active_screen_session
        && (!requested.microphone
            || (!snapshot.microphone_paused && snapshot.active_microphone_session))
        && (!requested.system_audio
            || (!snapshot.system_audio_paused && snapshot.active_system_audio_session))
}

#[cfg(target_os = "windows")]
fn verify_resumed_snapshot(
    before: &RuntimeSmokeSnapshot,
    snapshot: &RuntimeSmokeSnapshot,
    requested: &CaptureSources,
    scenario: SmokeScenario,
) -> Result<(), String> {
    if !snapshot.session_is_running {
        return Err("session was not running after the screen resume".to_string());
    }
    if snapshot.screen_paused || screen_paused_for_scenario(snapshot, scenario) {
        return Err(format!(
            "screen did not clear its transient pause: {snapshot:?}"
        ));
    }
    if requested.screen && !snapshot.active_screen_session {
        return Err("screen source has no live session after resume".to_string());
    }
    if requested.microphone && !snapshot.active_microphone_session {
        return Err("microphone session was lost across the transient recovery".to_string());
    }
    if requested.system_audio && !snapshot.active_system_audio_session {
        return Err("system-audio session was lost across the transient recovery".to_string());
    }
    if requested.microphone && snapshot.microphone_paused {
        return Err("microphone was paused after the screen resume".to_string());
    }
    if requested.system_audio && snapshot.system_audio_paused {
        return Err("system audio was paused after the screen resume".to_string());
    }
    if snapshot.current_segment_index <= before.current_segment_index {
        return Err(format!(
            "screen resume did not start a new emitted segment: before={}, after={}",
            before.current_segment_index, snapshot.current_segment_index
        ));
    }
    if scenario != SmokeScenario::SystemSuspend {
        verify_audio_source_session_ids_stable(before, snapshot, requested, "screen resume")?;
    }
    if requested.screen
        && snapshot
            .current_segment_output_files
            .as_ref()
            .and_then(|outputs| outputs.screen_file.as_deref())
            .is_none()
    {
        return Err("resumed screen segment has no live screen output path".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn verify_final_outputs(
    outputs: Option<&CaptureOutputFiles>,
    requested: &CaptureSources,
) -> Result<(), String> {
    let outputs =
        outputs.ok_or_else(|| "final output bookkeeping missing after stop".to_string())?;
    if requested.screen && outputs.screen_files.is_empty() {
        return Err("no finalized screen output files were recorded".to_string());
    }
    if requested.microphone && outputs.microphone_files.is_empty() {
        return Err(
            "no finalized microphone output files were recorded; rerun while microphone input is producing audio"
                .to_string(),
        );
    }
    if requested.system_audio && outputs.system_audio_files.is_empty() {
        return Err(
            "no finalized system-audio output files were recorded; rerun while system audio is producing audio"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn verify_audio_source_session_ids_stable(
    before: &RuntimeSmokeSnapshot,
    after: &RuntimeSmokeSnapshot,
    requested: &CaptureSources,
    phase: &str,
) -> Result<(), String> {
    if requested.microphone {
        let before_id = before
            .source_sessions
            .as_ref()
            .and_then(|sessions| sessions.microphone.as_ref())
            .map(|session| session.session_id.as_str())
            .ok_or_else(|| {
                format!(
                    "microphone source session id missing before {phase}; cannot verify stability"
                )
            })?;
        let after_id = after
            .source_sessions
            .as_ref()
            .and_then(|sessions| sessions.microphone.as_ref())
            .map(|session| session.session_id.as_str())
            .ok_or_else(|| {
                format!(
                    "microphone source session id missing after {phase}; cannot verify stability"
                )
            })?;
        if before_id != after_id {
            return Err(format!(
                "microphone source session id changed across {phase}: before={before_id} after={after_id}"
            ));
        }
    }
    if requested.system_audio {
        let before_id = before
            .source_sessions
            .as_ref()
            .and_then(|sessions| sessions.system_audio.as_ref())
            .map(|session| session.session_id.as_str())
            .ok_or_else(|| {
                format!("system-audio source session id missing before {phase}; cannot verify stability")
            })?;
        let after_id = after
            .source_sessions
            .as_ref()
            .and_then(|sessions| sessions.system_audio.as_ref())
            .map(|session| session.session_id.as_str())
            .ok_or_else(|| {
                format!(
                    "system-audio source session id missing after {phase}; cannot verify stability"
                )
            })?;
        if before_id != after_id {
            return Err(format!(
                "system-audio source session id changed across {phase}: before={before_id} after={after_id}"
            ));
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn stop_after_failure(app_handle: &tauri::AppHandle) -> Result<(), String> {
    let snapshot = snapshot(app_handle);
    if !snapshot.session_is_running {
        return Ok(());
    }
    super::stop_native_capture_with_state(
        app_handle.state::<super::NativeCaptureState>(),
        app_handle,
    )
    .map(|_| ())
    .map_err(|error| {
        format!(
            "failed to stop native capture after smoke failure: [{}] {}",
            error.code, error.message
        )
    })
}

#[cfg(target_os = "windows")]
fn format_source_sessions(source_sessions: Option<&capture_types::SourceSessions>) -> String {
    let Some(source_sessions) = source_sessions else {
        return "none".to_string();
    };
    format!(
        "screen={} microphone={} system_audio={}",
        source_sessions
            .screen
            .as_ref()
            .map(|session| session.session_id.as_str())
            .unwrap_or("none"),
        source_sessions
            .microphone
            .as_ref()
            .map(|session| session.session_id.as_str())
            .unwrap_or("none"),
        source_sessions
            .system_audio
            .as_ref()
            .map(|session| session.session_id.as_str())
            .unwrap_or("none")
    )
}

#[cfg(target_os = "windows")]
fn format_output_files(outputs: Option<&CaptureOutputFiles>) -> String {
    let Some(outputs) = outputs else {
        return "none".to_string();
    };
    format!(
        "screen_current={} screen_count={} microphone_current={} microphone_count={} system_audio_current={} system_audio_count={}",
        outputs.screen_file.as_deref().unwrap_or("none"),
        outputs.screen_files.len(),
        outputs.microphone_file.as_deref().unwrap_or("none"),
        outputs.microphone_files.len(),
        outputs.system_audio_file.as_deref().unwrap_or("none"),
        outputs.system_audio_files.len()
    )
}
