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
const SMOKE_ARG: &str = "--windows-inactivity-smoke";
#[cfg(target_os = "windows")]
const DEFAULT_IDLE_TIMEOUT_SECONDS: u64 = 2;
#[cfg(target_os = "windows")]
const DEFAULT_MAX_IDLE_WAIT_SECONDS: u64 = 45;
#[cfg(target_os = "windows")]
const DEFAULT_MAX_STATE_WAIT_SECONDS: u64 = 20;

#[cfg(target_os = "windows")]
#[derive(Debug)]
struct SmokeConfig {
    idle_timeout_seconds: u64,
    max_idle_wait_seconds: u64,
    max_state_wait_seconds: u64,
    save_directory: PathBuf,
}

#[cfg(target_os = "windows")]
#[derive(Debug, Clone)]
struct RuntimeSmokeSnapshot {
    session_is_running: bool,
    session_is_inactivity_paused: bool,
    requested_sources: Option<CaptureSources>,
    current_segment_sources: Option<CaptureSources>,
    source_sessions: Option<capture_types::SourceSessions>,
    output_files: Option<CaptureOutputFiles>,
    current_segment_output_files: Option<CaptureOutputFiles>,
    current_segment_index: u64,
    screen_paused: bool,
    microphone_paused: bool,
    system_audio_paused: bool,
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
            eprintln!("Windows inactivity smoke configuration error: {error}");
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
                    println!("Windows inactivity smoke: PASS");
                    0
                }
                Err(error) => {
                    eprintln!("Windows inactivity smoke: FAIL: {error}");
                    1
                }
            };
            setup_exit_code.store(code, Ordering::SeqCst);
            app.handle().exit(code);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Windows inactivity smoke Tauri application");

    app.run(|_, _| {});
    std::process::exit(exit_code.load(Ordering::SeqCst));
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn maybe_run_from_args_and_exit() {
    if std::env::args().any(|arg| arg == "--windows-inactivity-smoke") {
        eprintln!("Windows inactivity smoke is Windows-only");
        std::process::exit(2);
    }
}

#[cfg(target_os = "windows")]
impl SmokeConfig {
    fn from_args(args: &[String]) -> Result<Self, String> {
        let mut idle_timeout_seconds = DEFAULT_IDLE_TIMEOUT_SECONDS;
        let mut max_idle_wait_seconds = DEFAULT_MAX_IDLE_WAIT_SECONDS;
        let mut max_state_wait_seconds = DEFAULT_MAX_STATE_WAIT_SECONDS;
        let mut save_directory = default_smoke_save_directory();

        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                SMOKE_ARG => {}
                "--idle-timeout-seconds" => {
                    index += 1;
                    idle_timeout_seconds = parse_u64_arg(args, index, "--idle-timeout-seconds")?;
                }
                "--max-idle-wait-seconds" => {
                    index += 1;
                    max_idle_wait_seconds = parse_u64_arg(args, index, "--max-idle-wait-seconds")?;
                }
                "--max-state-wait-seconds" => {
                    index += 1;
                    max_state_wait_seconds =
                        parse_u64_arg(args, index, "--max-state-wait-seconds")?;
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

        if idle_timeout_seconds == 0 {
            return Err("--idle-timeout-seconds must be at least 1".to_string());
        }
        if max_idle_wait_seconds <= idle_timeout_seconds {
            return Err("--max-idle-wait-seconds must exceed --idle-timeout-seconds".to_string());
        }
        if max_state_wait_seconds == 0 {
            return Err("--max-state-wait-seconds must be at least 1".to_string());
        }

        Ok(Self {
            idle_timeout_seconds,
            max_idle_wait_seconds,
            max_state_wait_seconds,
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
        "mnema-windows-inactivity-smoke-{}",
        std::process::id()
    ))
}

#[cfg(target_os = "windows")]
fn print_usage() {
    println!(
        "Windows inactivity smoke\n\nRun from the repo with:\n  cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-inactivity-smoke\n\nDuring the smoke, stop touching the keyboard/mouse until prompted. The harness starts a real native capture using screen, microphone, and system-audio when supported, waits for Windows idle to cross the configured inactivity threshold, verifies backend pause bookkeeping, sends a real Win32 mouse input event to resume, verifies resumed segment bookkeeping, stops capture, and prints the smoke output directory.\n\nOptions:\n  --idle-timeout-seconds N      inactivity threshold (default: 2)\n  --max-idle-wait-seconds N     max wait for real Windows idle (default: 45)\n  --max-state-wait-seconds N    max wait for pause/resume bookkeeping (default: 20)\n  --save-directory PATH         smoke capture root (default: temp mnema-windows-inactivity-smoke-PID)"
    );
}

#[cfg(target_os = "windows")]
fn run_smoke(app: &mut tauri::App, config: &SmokeConfig) -> Result<(), String> {
    println!(
        "Windows inactivity smoke: configuring isolated capture root at {}",
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

    let sources = CaptureSources {
        screen: support.supported_sources.screen,
        microphone: support.supported_sources.microphone,
        system_audio: support.supported_sources.system_audio,
    };
    let source_count = count_sources(&sources);
    println!(
        "Windows inactivity smoke: supported sources screen={} microphone={} system_audio={}",
        sources.screen, sources.microphone, sources.system_audio
    );
    if source_count < 2 {
        return Err(format!(
            "real multi-source smoke requires at least two supported source families; observed {source_count}"
        ));
    }

    let app_handle = app.handle().clone();
    let start_response = super::start_native_capture_inner(
        "windows-inactivity-smoke",
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
        "Windows inactivity smoke: started segment {} with source sessions {}",
        start_snapshot.current_segment_index,
        format_source_sessions(start_snapshot.source_sessions.as_ref())
    );
    println!(
        "Windows inactivity smoke: initial active output paths {}",
        format_output_files(start_snapshot.current_segment_output_files.as_ref())
    );

    println!(
        "Windows inactivity smoke: do not touch keyboard/mouse; waiting for Windows idle >= {}s",
        config.idle_timeout_seconds
    );
    wait_for_windows_idle(config.idle_timeout_seconds, config.max_idle_wait_seconds)?;

    let paused_snapshot = wait_for_snapshot(
        &app_handle,
        Duration::from_secs(config.max_state_wait_seconds),
        |snapshot| all_requested_sources_paused(snapshot, &sources),
    )
    .inspect_err(|_| {
        let _ = stop_after_failure(&app_handle);
    })?;
    verify_paused_snapshot(&paused_snapshot, &sources)?;
    println!(
        "Windows inactivity smoke: observed inactivity pause; committed outputs {}",
        format_output_files(paused_snapshot.output_files.as_ref())
    );

    thread::sleep(Duration::from_secs(2));
    send_resume_input()?;
    println!("Windows inactivity smoke: sent Win32 mouse input event; waiting for resume");

    let resumed_snapshot = wait_for_snapshot(
        &app_handle,
        Duration::from_secs(config.max_state_wait_seconds),
        |snapshot| all_requested_sources_resumed(snapshot, &sources),
    )
    .inspect_err(|_| {
        let _ = stop_after_failure(&app_handle);
    })?;
    verify_resumed_snapshot(&start_snapshot, &resumed_snapshot, &sources)?;
    println!(
        "Windows inactivity smoke: observed resume into segment {}; active output paths {}",
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
        "Windows inactivity smoke: stopped and finalized outputs {}",
        format_output_files(stop_response.session.output_files.as_ref())
    );
    verify_final_outputs(stop_response.session.output_files.as_ref(), &sources)?;
    println!(
        "Windows inactivity smoke: output directory {}",
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
    settings.capture_system_audio = true;
    settings.segment_duration_seconds = 300;
    settings.screen_frame_rate = 1;
    settings.save_directory = config.save_directory.to_string_lossy().to_string();
    settings.auto_start = false;
    settings.pause_capture_on_inactivity = true;
    settings.idle_timeout_seconds = config.idle_timeout_seconds;
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
fn wait_for_windows_idle(timeout_seconds: u64, max_wait_seconds: u64) -> Result<(), String> {
    let idle_target_ms = timeout_seconds.saturating_mul(1000);
    let deadline = Instant::now() + Duration::from_secs(max_wait_seconds);
    loop {
        let idle_ms = super::system_idle::current_system_idle_ms()
            .ok_or_else(|| "Windows GetLastInputInfo did not return an idle reading".to_string())?;
        if idle_ms >= idle_target_ms {
            println!("Windows inactivity smoke: Windows idle reached {idle_ms}ms");
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Windows idle did not reach {idle_target_ms}ms within {max_wait_seconds}s; stop user input and rerun the smoke"
            ));
        }
        thread::sleep(Duration::from_millis(200));
    }
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
    let state = app_handle.state::<super::NativeCaptureState>();
    let runtime = state.lock().expect("native capture state poisoned");
    let runtime = runtime.runtime();
    RuntimeSmokeSnapshot {
        session_is_running: runtime.is_running,
        session_is_inactivity_paused: runtime.inactivity.is_paused,
        requested_sources: runtime.requested_sources.clone(),
        current_segment_sources: runtime.current_segment_sources.clone(),
        source_sessions: runtime.source_sessions.clone(),
        output_files: runtime.output_files.clone(),
        current_segment_output_files: runtime.current_segment_output_files.clone(),
        current_segment_index: runtime.current_segment_index,
        screen_paused: runtime.inactivity.screen_paused,
        microphone_paused: runtime.inactivity.microphone_paused,
        system_audio_paused: runtime.inactivity.system_audio_paused,
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
    if snapshot.current_segment_sources.as_ref() != Some(requested) {
        return Err(format!(
            "started current segment sources mismatch: expected {requested:?}, got {:?}",
            snapshot.current_segment_sources
        ));
    }
    verify_active_sessions(snapshot, requested)?;
    verify_current_outputs_named_for_sources(snapshot, requested)
}

#[cfg(target_os = "windows")]
fn verify_paused_snapshot(
    snapshot: &RuntimeSmokeSnapshot,
    requested: &CaptureSources,
) -> Result<(), String> {
    if !snapshot.session_is_running || !snapshot.session_is_inactivity_paused {
        return Err(format!("runtime was not inactivity paused: {snapshot:?}"));
    }
    if requested.screen && !snapshot.screen_paused {
        return Err("screen source was not marked paused".to_string());
    }
    if requested.microphone && !snapshot.microphone_paused {
        return Err("microphone source was not marked paused".to_string());
    }
    if requested.system_audio && !snapshot.system_audio_paused {
        return Err("system-audio source was not marked paused".to_string());
    }
    if requested.screen && snapshot.active_screen_session {
        return Err("screen session remained live while paused".to_string());
    }
    if requested.microphone && snapshot.active_microphone_session {
        return Err("microphone session remained live while paused".to_string());
    }
    if requested.system_audio && snapshot.active_system_audio_session {
        return Err("system-audio session remained live while paused".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn verify_resumed_snapshot(
    before: &RuntimeSmokeSnapshot,
    snapshot: &RuntimeSmokeSnapshot,
    requested: &CaptureSources,
) -> Result<(), String> {
    if !snapshot.session_is_running || snapshot.session_is_inactivity_paused {
        return Err(format!(
            "runtime did not resume from inactivity pause: {snapshot:?}"
        ));
    }
    if snapshot.current_segment_sources.as_ref() != Some(requested) {
        return Err(format!(
            "resumed current segment sources mismatch: expected {requested:?}, got {:?}",
            snapshot.current_segment_sources
        ));
    }
    if snapshot.current_segment_index <= before.current_segment_index {
        return Err(format!(
            "resume did not start a new emitted segment: before={}, after={}",
            before.current_segment_index, snapshot.current_segment_index
        ));
    }
    verify_source_session_ids_preserved(
        before.source_sessions.as_ref(),
        snapshot.source_sessions.as_ref(),
        requested,
    )?;
    verify_active_sessions(snapshot, requested)?;
    verify_current_outputs_named_for_sources(snapshot, requested)?;
    verify_output_paths_changed(before, snapshot, requested)
}

#[cfg(target_os = "windows")]
fn verify_source_session_ids_preserved(
    before: Option<&capture_types::SourceSessions>,
    after: Option<&capture_types::SourceSessions>,
    requested: &CaptureSources,
) -> Result<(), String> {
    let before = before.ok_or_else(|| "initial source session metadata missing".to_string())?;
    let after = after.ok_or_else(|| "resumed source session metadata missing".to_string())?;
    if requested.screen
        && before
            .screen
            .as_ref()
            .map(|session| session.session_id.as_str())
            != after
                .screen
                .as_ref()
                .map(|session| session.session_id.as_str())
    {
        return Err("screen source session id changed across inactivity resume".to_string());
    }
    if requested.microphone
        && before
            .microphone
            .as_ref()
            .map(|session| session.session_id.as_str())
            != after
                .microphone
                .as_ref()
                .map(|session| session.session_id.as_str())
    {
        return Err("microphone source session id changed across inactivity resume".to_string());
    }
    if requested.system_audio
        && before
            .system_audio
            .as_ref()
            .map(|session| session.session_id.as_str())
            != after
                .system_audio
                .as_ref()
                .map(|session| session.session_id.as_str())
    {
        return Err("system-audio source session id changed across inactivity resume".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn verify_active_sessions(
    snapshot: &RuntimeSmokeSnapshot,
    requested: &CaptureSources,
) -> Result<(), String> {
    if requested.screen && !snapshot.active_screen_session {
        return Err("screen source has no live session".to_string());
    }
    if requested.microphone && !snapshot.active_microphone_session {
        return Err("microphone source has no live session".to_string());
    }
    if requested.system_audio && !snapshot.active_system_audio_session {
        return Err("system-audio source has no live session".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn verify_current_outputs_named_for_sources(
    snapshot: &RuntimeSmokeSnapshot,
    requested: &CaptureSources,
) -> Result<(), String> {
    let outputs = snapshot
        .current_segment_output_files
        .as_ref()
        .ok_or_else(|| "current segment output bookkeeping was missing".to_string())?;
    let source_sessions = snapshot
        .source_sessions
        .as_ref()
        .ok_or_else(|| "source session metadata was missing".to_string())?;

    if requested.screen {
        let session_id = &source_sessions
            .screen
            .as_ref()
            .ok_or_else(|| "screen source session metadata missing".to_string())?
            .session_id;
        verify_path_contains(outputs.screen_file.as_deref(), session_id, "screen")?;
    }
    if requested.microphone {
        let session_id = &source_sessions
            .microphone
            .as_ref()
            .ok_or_else(|| "microphone source session metadata missing".to_string())?
            .session_id;
        verify_path_contains(outputs.microphone_file.as_deref(), session_id, "microphone")?;
    }
    if requested.system_audio {
        let session_id = &source_sessions
            .system_audio
            .as_ref()
            .ok_or_else(|| "system-audio source session metadata missing".to_string())?
            .session_id;
        verify_path_contains(
            outputs.system_audio_file.as_deref(),
            session_id,
            "system-audio",
        )?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn verify_path_contains(path: Option<&str>, session_id: &str, label: &str) -> Result<(), String> {
    let path = path.ok_or_else(|| format!("{label} current output path missing"))?;
    if !path.contains(session_id) {
        return Err(format!(
            "{label} current output path does not contain source session id {session_id}: {path}"
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn verify_output_paths_changed(
    before: &RuntimeSmokeSnapshot,
    after: &RuntimeSmokeSnapshot,
    requested: &CaptureSources,
) -> Result<(), String> {
    let before_outputs = before
        .current_segment_output_files
        .as_ref()
        .ok_or_else(|| "initial current output bookkeeping missing".to_string())?;
    let after_outputs = after
        .current_segment_output_files
        .as_ref()
        .ok_or_else(|| "resumed current output bookkeeping missing".to_string())?;
    if requested.screen && before_outputs.screen_file == after_outputs.screen_file {
        return Err("screen resume reused the pre-pause output path".to_string());
    }
    if requested.microphone && before_outputs.microphone_file == after_outputs.microphone_file {
        return Err("microphone resume reused the pre-pause output path".to_string());
    }
    if requested.system_audio && before_outputs.system_audio_file == after_outputs.system_audio_file
    {
        return Err("system-audio resume reused the pre-pause output path".to_string());
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
        return Err("no finalized microphone output files were recorded; rerun while microphone input is producing audio".to_string());
    }
    if requested.system_audio && outputs.system_audio_files.is_empty() {
        return Err("no finalized system-audio output files were recorded; rerun while system audio is playing".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn all_requested_sources_paused(
    snapshot: &RuntimeSmokeSnapshot,
    requested: &CaptureSources,
) -> bool {
    snapshot.session_is_running
        && snapshot.session_is_inactivity_paused
        && (!requested.screen || snapshot.screen_paused)
        && (!requested.microphone || snapshot.microphone_paused)
        && (!requested.system_audio || snapshot.system_audio_paused)
}

#[cfg(target_os = "windows")]
fn all_requested_sources_resumed(
    snapshot: &RuntimeSmokeSnapshot,
    requested: &CaptureSources,
) -> bool {
    snapshot.session_is_running
        && !snapshot.session_is_inactivity_paused
        && snapshot.current_segment_sources.as_ref() == Some(requested)
        && (!requested.screen || (!snapshot.screen_paused && snapshot.active_screen_session))
        && (!requested.microphone
            || (!snapshot.microphone_paused && snapshot.active_microphone_session))
        && (!requested.system_audio
            || (!snapshot.system_audio_paused && snapshot.active_system_audio_session))
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
fn send_resume_input() -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT,
    };

    let mut input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 1,
                dy: 0,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
    if sent != 1 {
        return Err("SendInput failed to synthesize resume input".to_string());
    }

    input.Anonymous.mi.dx = -1;
    let sent = unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
    if sent != 1 {
        return Err("SendInput failed to restore mouse position after resume input".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn count_sources(sources: &CaptureSources) -> usize {
    usize::from(sources.screen)
        + usize::from(sources.microphone)
        + usize::from(sources.system_audio)
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
