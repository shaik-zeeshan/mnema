//! The capture tests moved from the desktop crate with the segment machinery and
//! the Recording Lifecycle. They run on `None` cores: the code paths a test
//! drives never need the host.

use super::activity::lock_runtime_for_idle_debug;
#[cfg(target_os = "macos")]
use super::lifecycle::user_resume_sources;
use super::lifecycle::RecordingLifecycle;
#[cfg(target_os = "macos")]
use super::microphone_away::{
    apply_microphone_away, reopen_unwakeable_microphone, settle_microphone_reopen,
    tick_microphone_away,
};
use super::segments::next_emitted_segment_index;
#[cfg(target_os = "macos")]
use super::segments::{
    apply_microphone_output_finalization, apply_stopped_microphone_finalization,
    audio_duration_time_to_ms, audio_segment_started_at_unix_ms_for_file,
    audio_segment_window_from_duration_ms, cleanup_failed_segment_dirs,
    committed_audio_segments_for_output_files, low_disk_recovery_sources,
    pause_microphone_for_inactivity, pause_runtime_for_inactivity, pause_screen_for_inactivity,
    pause_system_audio_for_inactivity, plan_live_rotation_segment,
    process_inactivity_audio_transitions_for_snapshot, reanchor_active_segment_timing,
    recover_screen_capture_after_wake_with_start_segment, resume_microphone_from_inactivity,
    resume_runtime_from_inactivity, resume_screen_from_inactivity,
    resume_screen_from_inactivity_with_start_segment, resume_system_audio_from_inactivity,
    segment_loop_sleep_duration, should_defer_screen_resume_for_missing_display,
    stop_capture_runtime, StartedSegmentState,
};
#[cfg(target_os = "macos")]
use crate::native_capture::microphone::should_reconnect_waiting_microphone_session;
#[cfg(target_os = "macos")]
use crate::native_capture::microphone_away::should_resume_microphone_after_activity;
use crate::native_capture::runtime::{
    active_sources_for_inactivity_paused_state, current_segment_sources_for_runtime,
    ensure_system_audio_planner_for_runtime, microphone_backend_active_for_runtime,
    should_rotate_segment, system_audio_writer_active_for_runtime, NativeCaptureRuntime,
};
#[cfg(target_os = "macos")]
use crate::native_capture::runtime::{
    CaptureSuspension, CaptureSuspensionKind, TestMicrophoneDevice,
};
use crate::native_capture_inactivity::{
    ActivitySnapshot, AudioActivitySourceState, InactivityState,
};
use crate::native_capture_output::set_current_microphone_output_file;
use crate::recording_settings::{
    compute_effective_screen_bitrate_bps, validate_recording_settings,
    validate_recording_settings_with_resolution_support,
};
#[cfg(target_os = "macos")]
use capture_runtime::{
    current_date_prefix, CaptureClock, RuntimeSignal, SegmentPlanner, SegmentSchedule,
};
use capture_runtime::{RuntimeController, RuntimeState};
use capture_types::{
    default_appearance, default_audio_speech_detection_settings,
    default_audio_transcription_settings, default_inactivity_activity_mode,
    default_metadata_settings, default_microphone_vad_adapter, default_ocr_settings,
    default_preview_cache_ttl_seconds, default_privacy_settings, default_retention_policy,
    default_speaker_analysis_settings, default_video_bitrate, AccessSettings, AiRuntimeSettings,
    CaptureErrorResponse, CaptureOutputFiles, CaptureSources, InactivityActivityMode,
    MicrophoneControllerState, MicrophoneDisconnectPolicy, MicrophonePreference,
    MicrophonePreferenceMode, RecordingSettings, ScreenResolution, ScreenResolutionPreset,
    SourceSessionMeta, SourceSessions, UpdateRecordingSettingsRequest, UserContextSettings,
    VideoBitrateMode, VideoBitratePreset, VideoBitrateSettings,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("native-capture-{label}-{unique}"));

        fs::create_dir_all(&path).expect("test directory should be created");

        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn run_async_test(test: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build")
        .block_on(test);
}

#[cfg(target_os = "macos")]
fn write_openable_screen_file(path: &Path) {
    fs::write(path, b"\0\0\0\x14ftypqt  \0\0\0\0qt  \0\0\0\x10moovtrak")
        .expect("screen artifact should exist");
}

#[cfg(target_os = "macos")]
fn write_existing_audio_placeholder(path: &Path) -> String {
    fs::write(path, b"placeholder audio").expect("placeholder audio file should exist");
    path.to_string_lossy().to_string()
}

// A real, positive-duration AAC/m4a file so that the AVFoundation duration
// validation inside `finalize_capture_outputs` accepts it as usable captured
// audio (mirrors the helper in `native_capture_output.rs`'s tests).
#[cfg(target_os = "macos")]
fn write_valid_m4a_audio_file(path: &Path) {
    use cidre::{av, ns};

    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    let path_str = path.to_string_lossy().to_string();
    let url = ns::Url::with_fs_path_str(&path_str, false);
    let sample_rate = 48_000.0_f64;
    let format_id = ns::Number::with_u32(u32::from_be_bytes(*b"aac "));
    let sample_rate_value = ns::Number::with_f64(sample_rate);
    let channel_count_value = ns::Number::with_i64(1);
    let settings: cidre::arc::R<ns::Dictionary<ns::String, ns::Id>> =
        ns::Dictionary::with_keys_values(
            &[
                av::audio::all_formats_keys::id(),
                av::audio::all_formats_keys::sample_rate(),
                av::audio::all_formats_keys::number_of_channels(),
            ],
            &[
                format_id.as_id_ref(),
                sample_rate_value.as_id_ref(),
                channel_count_value.as_id_ref(),
            ],
        );
    let mut file = av::AudioFile::open_write_common_format(
        &url,
        &settings,
        av::AudioCommonFormat::PcmF32,
        false,
    )
    .expect("writable test audio file should open");
    let processing_format = file.processing_format();
    let frames: u32 = 24_000; // 0.5s @ 48kHz — comfortably positive duration.
    let mut buffer = av::AudioPcmBuf::with_format(&processing_format, frames)
        .expect("test audio buffer should allocate");
    buffer
        .set_frame_len(frames)
        .expect("frame length should set");
    if let Some(samples) = buffer.data_f32_mut_at(0) {
        for (index, sample) in samples.iter_mut().enumerate() {
            *sample = ((index as f32) * 0.05).sin() * 0.1;
        }
    }
    file.write(&buffer).expect("test audio should write");
    file.close();
}

fn recording_settings_fixture() -> RecordingSettings {
    RecordingSettings {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: "/tmp".to_string(),
        auto_start: false,
        developer_options_enabled: false,
        preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
        follow_timeline_live: false,
        retention_policy: default_retention_policy(),
        appearance: default_appearance(),
        time_format: Default::default(),
        ocr: default_ocr_settings(),
        transcription: default_audio_transcription_settings(),
        speaker_analysis: default_speaker_analysis_settings(),
        audio_speech_detection: default_audio_speech_detection_settings(),
        metadata: default_metadata_settings(),
        privacy: default_privacy_settings(),
        access: AccessSettings::default(),
        ai_runtime: AiRuntimeSettings::default(),
        user_context: UserContextSettings::default(),
        semantic_search: capture_types::default_semantic_search_settings(),
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        microphone_activity_sensitivity: 50,
        system_audio_activity_sensitivity: 50,
        microphone_vad_adapter: default_microphone_vad_adapter(),
        inactivity_activity_mode: default_inactivity_activity_mode(),
    }
}

fn update_recording_settings_request_fixture() -> UpdateRecordingSettingsRequest {
    let settings = recording_settings_fixture();
    UpdateRecordingSettingsRequest {
        capture_screen: settings.capture_screen,
        capture_microphone: settings.capture_microphone,
        capture_system_audio: settings.capture_system_audio,
        segment_duration_seconds: settings.segment_duration_seconds,
        screen_frame_rate: settings.screen_frame_rate,
        screen_resolution: settings.screen_resolution,
        video_bitrate: settings.video_bitrate,
        save_directory: settings.save_directory,
        auto_start: settings.auto_start,
        developer_options_enabled: settings.developer_options_enabled,
        preview_cache_ttl_seconds: settings.preview_cache_ttl_seconds,
        follow_timeline_live: settings.follow_timeline_live,
        retention_policy: settings.retention_policy,
        appearance: settings.appearance,
        time_format: settings.time_format,
        ocr: settings.ocr,
        transcription: settings.transcription,
        speaker_analysis: settings.speaker_analysis,
        audio_speech_detection: settings.audio_speech_detection,
        metadata: settings.metadata,
        privacy: settings.privacy,
        access: settings.access,
        ai_runtime: settings.ai_runtime,
        user_context: settings.user_context,
        semantic_search: settings.semantic_search,
        pause_capture_on_inactivity: settings.pause_capture_on_inactivity,
        idle_timeout_seconds: settings.idle_timeout_seconds,
        microphone_activity_sensitivity: settings.microphone_activity_sensitivity,
        system_audio_activity_sensitivity: settings.system_audio_activity_sensitivity,
        microphone_vad_adapter: settings.microphone_vad_adapter,
        inactivity_activity_mode: settings.inactivity_activity_mode,
    }
}

#[cfg(target_os = "macos")]
fn running_runtime_controller() -> RuntimeController {
    let mut controller = RuntimeController::default();
    controller
        .apply(RuntimeSignal::StartRequested)
        .expect("idle runtime should start");
    controller
        .apply(RuntimeSignal::SourcesReady)
        .expect("starting runtime should become running");
    controller
}

#[cfg(target_os = "macos")]
fn paused_runtime_fixture() -> NativeCaptureRuntime {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        segment_loop_control: None,
        capture_clock: Some(CaptureClock::start_now()),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(60))),
        segment_planner: Some(SegmentPlanner::new(
            "/tmp/native-capture-tests",
            "native-session-resume",
        )),
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            is_paused: true,
            ..InactivityState::default()
        },
        ..Default::default()
    }
}

#[cfg(target_os = "macos")]
fn independent_source_sessions_fixture() -> SourceSessions {
    SourceSessions {
        screen: Some(SourceSessionMeta {
            session_id: "native-session-screen".to_string(),
            started_at_unix_ms: 123,
        }),
        microphone: Some(SourceSessionMeta {
            session_id: "native-session-microphone".to_string(),
            started_at_unix_ms: 123,
        }),
        system_audio: Some(SourceSessionMeta {
            session_id: "native-session-system-audio".to_string(),
            started_at_unix_ms: 123,
        }),
    }
}

#[cfg(target_os = "macos")]
fn resumed_segment_state_fixture(screen_file: String) -> StartedSegmentState {
    (
        CaptureOutputFiles {
            screen_file: Some(screen_file.clone()),
            screen_files: vec![screen_file.clone()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        },
        Some(screen_file),
        None,
        None,
        None,
    )
}

#[cfg(target_os = "macos")]
fn running_screen_capture_runtime_fixture() -> NativeCaptureRuntime {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        capture_clock: Some(CaptureClock::start_now()),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(60))),
        segment_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "native-session-wake-screen",
            "2026/04/23",
        )),
        microphone_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "native-session-wake-mic",
            "2026/04/23",
        )),
        system_audio_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "native-session-wake-system-audio",
            "2026/04/23",
        )),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: Some("/tmp/mic.m4a".to_string()),
            microphone_files: vec!["/tmp/mic.m4a".to_string()],
            system_audio_file: Some("/tmp/system-audio.m4a".to_string()),
            system_audio_files: vec!["/tmp/system-audio.m4a".to_string()],
        }),
        output_files: Some(CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: Some("/tmp/screen.mov".to_string()),
        microphone_recording_file: Some("/tmp/mic.m4a".to_string()),
        system_audio_recording_file: Some("/tmp/system-audio.m4a".to_string()),
        source_sessions: Some(independent_source_sessions_fixture()),
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            ..InactivityState::default()
        },
        ..Default::default()
    }
}

#[cfg(target_os = "macos")]
async fn insert_open_frame_batch_fixture(
    infra: &::app_infra::AppInfra,
    session_id: &str,
    frame_path: &str,
) {
    infra
        .capture_frame(
            &::app_infra::NewFrame::new(session_id, frame_path, "2026-05-02T13:40:00Z"),
            None,
        )
        .await
        .expect("first frame should persist");

    let open_before = infra
        .list_frame_batches(Some(session_id))
        .await
        .expect("frame batches should list")
        .into_iter()
        .find(|batch| batch.batch_started_at == "2026-05-02T13:40:00Z")
        .expect("first batch should exist");
    assert_eq!(open_before.status, ::app_infra::FrameBatchStatus::Open);
}

#[cfg(target_os = "macos")]
async fn assert_frame_batches_closed(infra: &::app_infra::AppInfra, session_id: &str) {
    let batches = infra
        .list_frame_batches(Some(session_id))
        .await
        .expect("frame batches should list after stop cleanup");
    assert!(batches
        .iter()
        .all(|batch| batch.status != ::app_infra::FrameBatchStatus::Open));
    assert!(batches
        .iter()
        .any(|batch| batch.status == ::app_infra::FrameBatchStatus::Closed));
    assert!(batches.iter().all(|batch| batch.finalize_job_id.is_some()));
}

#[cfg(target_os = "macos")]
fn audio_paused_runtime_fixture() -> NativeCaptureRuntime {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        segment_loop_control: None,
        capture_clock: Some(CaptureClock::start_now()),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(60))),
        segment_planner: Some(SegmentPlanner::new(
            "/tmp/native-capture-tests",
            "native-session-audio-pause",
        )),
        source_sessions: Some(independent_source_sessions_fixture()),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: Some("/tmp/screen.mov".to_string()),
        active_screen_session: None, // screen still conceptually active
        active_microphone_session: None,
        active_system_audio_session: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            microphone_paused: true,
            system_audio_paused: true,
            is_paused: true,
            ..InactivityState::default()
        },
        ..Default::default()
    }
}

// The away hold (founder ruling 2026-10-23: lock is away). The live lock and
// display state is read by the tick; its application is exercised here without
// an audio device.
/// A paused microphone with no session, whose planner date is stale and whose
/// file can't be opened: an attempt to reopen it refreshes the date and then
/// fails before any audio device is touched.
#[cfg(target_os = "macos")]
fn stranded_microphone_runtime_fixture() -> NativeCaptureRuntime {
    let mut runtime = lent_out_microphone_runtime_fixture();
    runtime.microphone_lent_out = false;
    mark_microphone_planner_stale(&mut runtime);
    runtime
}

#[cfg(target_os = "macos")]
fn mark_microphone_planner_stale(runtime: &mut NativeCaptureRuntime) {
    runtime
        .microphone_planner
        .as_mut()
        .expect("fixture has a microphone planner")
        .set_date_prefix("2000-01-01".to_string());
}

#[cfg(target_os = "macos")]
fn microphone_open_was_attempted(runtime: &NativeCaptureRuntime) -> bool {
    runtime
        .microphone_planner
        .as_ref()
        .expect("fixture has a microphone planner")
        .date_prefix()
        == current_date_prefix()
}

/// [`stranded_microphone_runtime_fixture`] with a running microphone: unpaused,
/// recording a file, its device standing in for a session.
#[cfg(target_os = "macos")]
fn recording_microphone_runtime_fixture(device: TestMicrophoneDevice) -> NativeCaptureRuntime {
    let mut runtime = stranded_microphone_runtime_fixture();
    runtime
        .inactivity
        .set_family_paused_states(false, false, false);
    let microphone_file = "/tmp/native-capture-tests/microphone-away.m4a".to_string();
    set_current_microphone_output_file(
        runtime.current_segment_output_files.as_mut().unwrap(),
        microphone_file.clone(),
    );
    runtime.microphone_recording_file = Some(microphone_file);
    runtime.test_microphone_device = Some(device);
    runtime
}

/// A microphone whose device went (a waited-for device unplugged, a dead
/// session dropped): no session, and the file its stop closed booked against
/// the live segment for the next commit.
#[cfg(target_os = "macos")]
fn stopped_microphone_with_booked_file_fixture(dir: &TestDir) -> (NativeCaptureRuntime, String) {
    let booked_file = dir.path().join("microphone-booked-by-stop.m4a");
    write_valid_m4a_audio_file(&booked_file);
    let booked_file = booked_file.to_string_lossy().into_owned();
    let mut runtime = stranded_microphone_runtime_fixture();
    runtime
        .inactivity
        .set_family_paused_states(false, false, false);
    runtime.output_files = Some(crate::native_capture::segments::empty_output_files());
    let outputs = runtime.current_segment_output_files.as_mut().unwrap();
    outputs.microphone_files = vec![booked_file.clone()];
    runtime.microphone_recording_file = None;
    (runtime, booked_file)
}

// Voice enrollment borrows the microphone mid-session. Its own speech reaches the
// process-global activity feed, so the inactivity tick used to "resume" a paused
// microphone by starting a second AVCaptureSession that recorded the take into
// the timeline. Until the microphone is handed back, capture must not start,
// resume or pause the family.
#[cfg(target_os = "macos")]
fn lent_out_microphone_runtime_fixture() -> NativeCaptureRuntime {
    let mut runtime = audio_paused_runtime_fixture();
    runtime.microphone_lent_out = true;
    // Opening a microphone file fails here, so any attempt to start a session
    // surfaces as an error before an audio device is touched.
    runtime.microphone_planner = Some(SegmentPlanner::with_date_prefix(
        "/dev/null/mnema-microphone-lent-out",
        "native-session-microphone",
        current_date_prefix(),
    ));
    runtime
}

#[cfg(target_os = "macos")]
fn screen_paused_runtime_fixture() -> NativeCaptureRuntime {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        segment_loop_control: None,
        capture_clock: Some(CaptureClock::start_now()),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(60))),
        segment_planner: Some(SegmentPlanner::new(
            "/tmp/native-capture-tests",
            "native-session-screen-pause",
        )),
        // Screen is paused but audio (mic) is still active, so
        // current_segment_output_files tracks the ongoing mic continuation.
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: Some("/tmp/native-capture-tests/.mnema/segments/native-session-screen-pause/1/audio/microphone.m4a".to_string()),
            microphone_files: vec!["/tmp/native-capture-tests/.mnema/segments/native-session-screen-pause/1/audio/microphone.m4a".to_string()],
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        microphone_recording_file: Some("/tmp/native-capture-tests/.mnema/segments/native-session-screen-pause/1/audio/microphone.m4a".to_string()),
        recording_file: None,
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            screen_paused: true,
            is_paused: true,
            ..InactivityState::default()
        },
        ..Default::default()
    }
}

#[cfg(target_os = "macos")]
fn screen_paused_with_system_audio_runtime_fixture(audio_paused: bool) -> NativeCaptureRuntime {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        segment_loop_control: None,
        capture_clock: Some(CaptureClock::start_now()),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(60))),
        segment_planner: Some(SegmentPlanner::new(
            "/tmp/native-capture-tests",
            "native-session-screen-audio-resume",
        )),
        current_segment_output_files: None,
        recording_file: None,
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            screen_paused: true,
            microphone_paused: audio_paused,
            system_audio_paused: audio_paused,
            is_paused: true,
            ..InactivityState::default()
        },
        ..Default::default()
    }
}

mod audio_pause;
mod audio_restart;
mod audio_segments;
mod continuation;
#[cfg(target_os = "macos")]
mod host_events;
mod inactivity_noop;
mod inactivity_resume;
#[cfg(target_os = "macos")]
mod low_disk;
mod microphone_away;
mod planners;
mod resume_failures;
mod screen_pause;
mod settings;
mod soft_pause;
mod stop_and_sleep;
mod wake_recovery;
