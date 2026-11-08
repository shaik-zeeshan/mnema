use super::*;

// --- Slice 3b13: bookkeeping refinements ---

// Regression: the cold screen-inactivity pause (the screen session is already
// stopped — a sleep clears it, and the tick can reach the pause before the
// did-wake handler runs) still treated system audio as screen-owned. It passed
// `system_audio_recording_file` — which the tap is still writing — into
// `finalize_capture_outputs`, which reads an open `.m4a` as having no duration,
// judges it unusable, and deletes it out from under the running writer. Every
// explicit stop path tears the tap down before committing; this path cannot,
// because ADR 0052 requires a screen pause to leave the tap alive — so the live
// file has to stay out of the commit entirely.
#[cfg(target_os = "macos")]
#[test]
fn cold_screen_pause_never_finalizes_the_file_the_tap_is_still_writing() {
    let temp = tempfile::tempdir().expect("temp dir should be created");

    let screen_path = temp.path().join("segment-0001-screen.mov");
    std::fs::write(&screen_path, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
        .expect("openable screen mov should be written");
    let screen = screen_path.to_string_lossy().into_owned();

    // What a live AVAssetWriter's output looks like on disk: real bytes, no moov
    // atom yet, so nothing can read a duration off it.
    let system_audio_path = temp.path().join("segment-0001-system-audio.m4a");
    std::fs::write(&system_audio_path, b"open writer, no moov atom yet")
        .expect("in-flight system audio m4a should be written");
    let system_audio = system_audio_path.to_string_lossy().into_owned();

    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();
    let screen_and_system_audio = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: true,
    };

    let mut current_segment_output_files = crate::native_capture::segments::empty_output_files();
    crate::native_capture_output::set_current_screen_output_file(
        &mut current_segment_output_files,
        screen.clone(),
    );
    crate::native_capture_output::set_current_system_audio_output_file(
        &mut current_segment_output_files,
        system_audio.clone(),
    );

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(screen_and_system_audio.clone()),
        current_segment_sources: Some(screen_and_system_audio),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        current_segment_output_files: Some(current_segment_output_files),
        output_files: Some(crate::native_capture::segments::empty_output_files()),
        recording_file: Some(screen.clone()),
        system_audio_recording_file: Some(system_audio.clone()),
        // Cold: no live screen session to pause, so the pause runs its
        // stop-and-commit path rather than returning early.
        active_screen_session: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    pause_screen_for_inactivity(&mut runtime).expect("screen pause should succeed");

    assert!(
        system_audio_path.is_file(),
        "the tap is still writing this file — a screen pause must not finalize or delete it"
    );
    assert_eq!(
        runtime.system_audio_recording_file.as_deref(),
        Some(system_audio.as_str()),
        "and it stays tracked, so the tap's next boundary still commits it"
    );
    let committed = runtime
        .output_files
        .as_ref()
        .expect("committed output_files slot should exist");
    assert!(
        !committed
            .system_audio_file
            .iter()
            .chain(committed.system_audio_files.iter())
            .any(|file| file == &system_audio),
        "a file still being written must not be committed as a finished segment"
    );
}

// Sibling of the test above: keeping the live file out of the commit must not
// also drop the earlier-generation `.m4a` a mid-segment tap rebuild already
// finalized. That file is complete on disk and belongs to this segment; a cold
// screen pause that clears the whole system-audio list orphans it — no
// audio_segment row, never transcribed, invisible to retention — and the
// live-only continuation never carries it into a later commit either.
#[cfg(target_os = "macos")]
#[test]
fn cold_screen_pause_commits_the_generation_a_rebuild_already_finalized() {
    let temp = tempfile::tempdir().expect("temp dir should be created");

    let screen_path = temp.path().join("segment-0001-screen.mov");
    std::fs::write(&screen_path, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
        .expect("openable screen mov should be written");
    let screen = screen_path.to_string_lossy().into_owned();

    // Generation 1: finalized by a tap rebuild — a real, closed, readable m4a.
    let finalized_path = temp.path().join("segment-0001-system-audio-gen1.m4a");
    low_disk::write_valid_m4a_audio_file(&finalized_path);
    let finalized = finalized_path.to_string_lossy().into_owned();

    // Generation 2: the rebuilt tap's live file, still being written.
    let live_path = temp.path().join("segment-0001-system-audio-gen2.m4a");
    std::fs::write(&live_path, b"open writer, no moov atom yet")
        .expect("in-flight system audio m4a should be written");
    let live = live_path.to_string_lossy().into_owned();

    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();
    let screen_and_system_audio = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: true,
    };

    let mut current_segment_output_files = crate::native_capture::segments::empty_output_files();
    crate::native_capture_output::set_current_screen_output_file(
        &mut current_segment_output_files,
        screen.clone(),
    );
    crate::native_capture_output::set_current_system_audio_output_file(
        &mut current_segment_output_files,
        finalized.clone(),
    );
    crate::native_capture_output::set_current_system_audio_output_file(
        &mut current_segment_output_files,
        live.clone(),
    );

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(screen_and_system_audio.clone()),
        current_segment_sources: Some(screen_and_system_audio),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        current_segment_output_files: Some(current_segment_output_files),
        output_files: Some(crate::native_capture::segments::empty_output_files()),
        recording_file: Some(screen.clone()),
        system_audio_recording_file: Some(live.clone()),
        active_screen_session: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    pause_screen_for_inactivity(&mut runtime).expect("screen pause should succeed");

    let committed = runtime
        .output_files
        .as_ref()
        .expect("committed output_files slot should exist");
    assert!(
        committed
            .system_audio_files
            .iter()
            .any(|file| file == &finalized),
        "the generation a rebuild already finalized must commit with the segment (got {committed:?})"
    );
    assert!(
        !committed
            .system_audio_file
            .iter()
            .chain(committed.system_audio_files.iter())
            .any(|file| file == &live),
        "the file the tap is still writing must not be committed"
    );
    assert!(
        live_path.is_file(),
        "the live file must survive the pause untouched"
    );
    let continuation = runtime
        .current_segment_output_files
        .as_ref()
        .expect("continuation output files should exist for the live tap");
    assert_eq!(
        continuation.system_audio_files,
        vec![live.clone()],
        "the continuation carries only the live file — the finalized one is already committed"
    );
    assert_eq!(
        runtime.system_audio_recording_file.as_deref(),
        Some(live.as_str())
    );
}

#[cfg(target_os = "macos")]
#[test]
fn pause_screen_for_inactivity_keeps_continuation_for_system_audio_only() {
    // A system-audio-only continuation is a real continuation: the tap has no
    // screen dependency, so a screen pause leaves it recording and its file has to
    // stay tracked (ADR 0052). Before the CATap swap this asserted the opposite —
    // system audio rode the screen session, so a screen pause ended it.
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: Some("/tmp/system-audio.m4a".to_string()),
            system_audio_files: vec!["/tmp/system-audio.m4a".to_string()],
        }),
        recording_file: Some("/tmp/screen.mov".to_string()),
        system_audio_recording_file: Some("/tmp/system-audio.m4a".to_string()),
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    pause_screen_for_inactivity(&mut runtime).expect("screen pause should succeed");

    let output_files = runtime
        .current_segment_output_files
        .as_ref()
        .expect("a live tap is an audio continuation even with no microphone");
    assert!(output_files.screen_file.is_none());
    assert_eq!(
        output_files.system_audio_file.as_deref(),
        Some("/tmp/system-audio.m4a")
    );
    assert_eq!(
        runtime.system_audio_recording_file.as_deref(),
        Some("/tmp/system-audio.m4a")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn pause_audio_restart_screen_no_planner_clears_screen_output_files() {
    // When the screen restart fails due to missing planner, stale screen
    // entries in current_segment_output_files must also be cleared.
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
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
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: Some("/tmp/mic.m4a".to_string()),
            microphone_files: vec!["/tmp/mic.m4a".to_string()],
            system_audio_file: Some("/tmp/system-audio.m4a".to_string()),
            system_audio_files: vec!["/tmp/system-audio.m4a".to_string()],
        }),
        recording_file: Some("/tmp/screen.mov".to_string()),
        microphone_recording_file: Some("/tmp/mic.m4a".to_string()),
        system_audio_recording_file: Some("/tmp/system-audio.m4a".to_string()),
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        segment_planner: None, // no planner triggers the failure path
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    // With no active screen session, the function should succeed safely.
    pause_system_audio_for_inactivity(&mut runtime)
        .expect("should succeed safely when no active screen session");

    // System audio output file must be cleared.
    let output_files = runtime
        .current_segment_output_files
        .as_ref()
        .expect("output files struct should still exist");
    assert!(
        output_files.system_audio_file.is_none(),
        "system_audio_file should be cleared from output files"
    );
    // Screen and microphone output files should be untouched — no screen
    // session was stopped.
    assert!(
        output_files.screen_file.is_some(),
        "screen_file should be preserved when no screen session was active"
    );
    assert_eq!(
        output_files.microphone_file.as_deref(),
        Some("/tmp/mic.m4a"),
        "microphone_file should be preserved"
    );
}

// --- Slice 3b14: fatal error reconciliation and audio continuation guard ---

#[cfg(target_os = "macos")]
#[test]
fn pause_screen_for_inactivity_no_continuation_without_live_mic_session_or_file() {
    // When requested_sources has microphone but there is no active session
    // and no microphone_recording_file, the audio continuation guard should
    // NOT create a stub output-files struct.
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: Some("/tmp/screen.mov".to_string()),
        // No microphone session or recording file — mic was requested but
        // never successfully started or already stopped.
        active_microphone_session: None,
        active_system_audio_session: None,
        microphone_recording_file: None,
        active_screen_session: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    pause_screen_for_inactivity(&mut runtime).expect("screen pause should succeed");

    // Without a live microphone session or recording file, there is no real
    // audio continuation — output files should be cleared.
    assert!(
        runtime.current_segment_output_files.is_none(),
        "current_segment_output_files should be None when no live mic continuation exists"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn pause_screen_for_inactivity_continuation_with_mic_recording_file() {
    // When there is a microphone_recording_file (even without an active
    // session object in tests), the continuation should be preserved.
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: Some("/tmp/mic.m4a".to_string()),
            microphone_files: vec!["/tmp/mic.m4a".to_string()],
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: Some("/tmp/screen.mov".to_string()),
        microphone_recording_file: Some("/tmp/mic.m4a".to_string()),
        active_microphone_session: None,
        active_system_audio_session: None,
        active_screen_session: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    pause_screen_for_inactivity(&mut runtime).expect("screen pause should succeed");

    let output_files = runtime
        .current_segment_output_files
        .as_ref()
        .expect("output files should be preserved for mic recording file continuation");
    assert_eq!(
        output_files.microphone_file.as_deref(),
        Some("/tmp/mic.m4a")
    );
    assert!(output_files.screen_file.is_none());
}
