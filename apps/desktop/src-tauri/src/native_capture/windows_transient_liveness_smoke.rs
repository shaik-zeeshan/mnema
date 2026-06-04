//! On-device smoke harness for Windows transient-liveness screen recovery
//! (ADR 0023, GitHub issue #62, label `needs-smoke-test`).
//!
//! This is a *true on-device* harness like `windows_inactivity_smoke`: it cannot
//! synthesize a monitor disconnect, so it drives a real native capture and then
//! prompts the operator (on stdout) to physically disconnect and reconnect a
//! display while the harness watches the runtime bookkeeping.
//!
//! Run from the repo with:
//! ```text
//! cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-transient-liveness-smoke
//! ```
//!
//! Sequence:
//!   1. Start a screen + microphone recording (screen drives the transient
//!      condition; the microphone proves audio continuity through the outage).
//!   2. Prompt the operator to disconnect the monitor; wait (bounded) for the
//!      screen to enter a `TransientLiveness { DisplayUnavailable }` pause while
//!      the session stays running and the microphone keeps recording.
//!   3. Prompt the operator to reconnect the monitor; wait (bounded) for the
//!      screen to resume into a fresh segment with live output again.
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
#[derive(Debug)]
struct SmokeConfig {
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
    screen_paused_for_transient_display_unavailable: bool,
    active_screen_session: bool,
    active_microphone_session: bool,
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
        let mut max_disconnect_wait_seconds = DEFAULT_MAX_DISCONNECT_WAIT_SECONDS;
        let mut max_reconnect_wait_seconds = DEFAULT_MAX_RECONNECT_WAIT_SECONDS;
        let mut save_directory = default_smoke_save_directory();

        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                SMOKE_ARG => {}
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
fn print_usage() {
    println!(
        "Windows transient-liveness smoke (ADR 0023)\n\nRun from the repo with:\n  cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-transient-liveness-smoke\n\nThis is a true on-device harness: it starts a real screen + microphone capture and then asks you to physically disconnect and reconnect a monitor mid-recording. When prompted, unplug the monitor (or lock the session / sleep the display) and wait; the harness verifies the screen enters a TransientLiveness {{ DisplayUnavailable }} pause while the session keeps running and the microphone keeps recording. When prompted again, reconnect the monitor; the harness verifies the screen auto-resumes into a fresh segment, then stops capture and prints the output directory.\n\nUse a multi-monitor machine or an external monitor you can unplug. On a single-display laptop, locking the session (Win+L) or closing the lid can also drive the display-unavailable condition.\n\nOptions:\n  --max-disconnect-wait-seconds N   max wait for the transient pause after you disconnect (default: 90)\n  --max-reconnect-wait-seconds N    max wait for the screen resume after you reconnect (default: 90)\n  --save-directory PATH             smoke capture root (default: temp mnema-windows-transient-liveness-smoke-PID)"
    );
}

#[cfg(target_os = "windows")]
fn run_smoke(app: &mut tauri::App, config: &SmokeConfig) -> Result<(), String> {
    println!(
        "Windows transient-liveness smoke: configuring isolated capture root at {}",
        config.save_directory.display()
    );
    install_smoke_recording_settings(app.handle(), config)?;
    crate::app_infra::initialize(app).map_err(|error| {
        format!("failed to initialize app infra for smoke capture root: {error}")
    })?;

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

    let sources = CaptureSources {
        screen: true,
        microphone: true,
        system_audio: false,
    };
    println!(
        "Windows transient-liveness smoke: capturing screen + microphone (system audio off)"
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

    println!(
        "\n>>> ACTION REQUIRED: DISCONNECT the monitor now (unplug it, or Win+L / close the lid).\n>>> Waiting up to {}s for the screen to enter a transient-liveness pause...",
        config.max_disconnect_wait_seconds
    );
    let paused_snapshot = wait_for_snapshot(
        &app_handle,
        Duration::from_secs(config.max_disconnect_wait_seconds),
        |snapshot| transient_screen_suspended(snapshot, &sources),
    )
    .inspect_err(|_| {
        let _ = stop_after_failure(&app_handle);
    })?;
    verify_transient_paused_snapshot(&paused_snapshot, &sources)?;
    println!(
        "Windows transient-liveness smoke: observed TransientLiveness {{ DisplayUnavailable }} screen pause; session still running; committed outputs {}",
        format_output_files(paused_snapshot.output_files.as_ref())
    );

    println!(
        "\n>>> ACTION REQUIRED: RECONNECT the monitor now (replug it, or unlock / open the lid).\n>>> Waiting up to {}s for the screen to auto-resume...",
        config.max_reconnect_wait_seconds
    );
    let resumed_snapshot = wait_for_snapshot(
        &app_handle,
        Duration::from_secs(config.max_reconnect_wait_seconds),
        |snapshot| screen_resumed(snapshot, &sources),
    )
    .inspect_err(|_| {
        let _ = stop_after_failure(&app_handle);
    })?;
    verify_resumed_snapshot(&start_snapshot, &resumed_snapshot, &sources)?;
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
    println!(
        "Windows transient-liveness smoke: output directory {}",
        config.save_directory.display()
    );

    Ok(())
}

#[cfg(target_os = "windows")]
fn install_smoke_recording_settings(
    app_handle: &tauri::AppHandle,
    config: &SmokeConfig,
) -> Result<(), String> {
    let mut settings = super::settings::default_recording_settings();
    settings.capture_screen = true;
    settings.capture_microphone = true;
    settings.capture_system_audio = false;
    settings.segment_duration_seconds = 300;
    settings.screen_frame_rate = 1;
    settings.save_directory = config.save_directory.to_string_lossy().to_string();
    settings.auto_start = false;
    // Disable inactivity pause so the only screen pause we can observe is the
    // transient-liveness display-unavailable suspension under test.
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
    RuntimeSmokeSnapshot {
        session_is_running: runtime.is_running,
        requested_sources: runtime.requested_sources.clone(),
        source_sessions: runtime.source_sessions.clone(),
        output_files: runtime.output_files.clone(),
        current_segment_output_files: runtime.current_segment_output_files.clone(),
        current_segment_index: runtime.current_segment_index,
        screen_paused: runtime.inactivity.screen_paused,
        microphone_paused: runtime.inactivity.microphone_paused,
        screen_paused_for_transient_display_unavailable,
        active_screen_session: capture_screen::screen_capture_session_is_live(
            runtime.active_screen_session.as_ref(),
        ),
        active_microphone_session: runtime
            .active_microphone_session
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
    if !snapshot.active_screen_session {
        return Err("screen source has no live session at start".to_string());
    }
    if !snapshot.active_microphone_session {
        return Err("microphone source has no live session at start".to_string());
    }
    Ok(())
}

// The transient-liveness screen suspension condition: the screen is paused for
// `TransientLiveness { DisplayUnavailable }`, its live session is gone, the
// session is still running, and the microphone keeps recording.
#[cfg(target_os = "windows")]
fn transient_screen_suspended(snapshot: &RuntimeSmokeSnapshot, requested: &CaptureSources) -> bool {
    snapshot.session_is_running
        && snapshot.screen_paused
        && snapshot.screen_paused_for_transient_display_unavailable
        && !snapshot.active_screen_session
        && (!requested.microphone || (!snapshot.microphone_paused && snapshot.active_microphone_session))
}

#[cfg(target_os = "windows")]
fn verify_transient_paused_snapshot(
    snapshot: &RuntimeSmokeSnapshot,
    requested: &CaptureSources,
) -> Result<(), String> {
    if !snapshot.session_is_running {
        return Err("session was not running during the transient pause".to_string());
    }
    if !snapshot.screen_paused_for_transient_display_unavailable {
        return Err(format!(
            "screen was not paused for TransientLiveness {{ DisplayUnavailable }}: {snapshot:?}"
        ));
    }
    if snapshot.active_screen_session {
        return Err("screen session remained live while transient-paused".to_string());
    }
    if requested.microphone && snapshot.microphone_paused {
        return Err("microphone was paused during a screen-only transient outage".to_string());
    }
    if requested.microphone && !snapshot.active_microphone_session {
        return Err(
            "microphone session went down during a screen-only transient outage; audio continuity broken"
                .to_string(),
        );
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
        && (!requested.microphone || snapshot.active_microphone_session)
}

#[cfg(target_os = "windows")]
fn verify_resumed_snapshot(
    before: &RuntimeSmokeSnapshot,
    snapshot: &RuntimeSmokeSnapshot,
    requested: &CaptureSources,
) -> Result<(), String> {
    if !snapshot.session_is_running {
        return Err("session was not running after the screen resume".to_string());
    }
    if snapshot.screen_paused || snapshot.screen_paused_for_transient_display_unavailable {
        return Err(format!("screen did not clear its transient pause: {snapshot:?}"));
    }
    if requested.screen && !snapshot.active_screen_session {
        return Err("screen source has no live session after resume".to_string());
    }
    if requested.microphone && !snapshot.active_microphone_session {
        return Err("microphone session was lost across the transient recovery".to_string());
    }
    if snapshot.current_segment_index <= before.current_segment_index {
        return Err(format!(
            "screen resume did not start a new emitted segment: before={}, after={}",
            before.current_segment_index, snapshot.current_segment_index
        ));
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
    // The microphone source session id must be preserved through the screen-only
    // outage (the audio source never stopped).
    if requested.microphone {
        let before_id = before
            .source_sessions
            .as_ref()
            .and_then(|sessions| sessions.microphone.as_ref())
            .map(|session| session.session_id.as_str());
        let after_id = snapshot
            .source_sessions
            .as_ref()
            .and_then(|sessions| sessions.microphone.as_ref())
            .map(|session| session.session_id.as_str());
        if before_id != after_id {
            return Err(
                "microphone source session id changed across the screen transient recovery"
                    .to_string(),
            );
        }
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
        "screen_current={} screen_count={} microphone_current={} microphone_count={}",
        outputs.screen_file.as_deref().unwrap_or("none"),
        outputs.screen_files.len(),
        outputs.microphone_file.as_deref().unwrap_or("none"),
        outputs.microphone_files.len()
    )
}
