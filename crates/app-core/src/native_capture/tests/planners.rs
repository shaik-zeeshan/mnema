use super::*;

#[test]
fn lock_runtime_for_idle_debug_recovers_poisoned_state() {
    let mut lifecycle = RecordingLifecycle::default();
    lifecycle.runtime_mut().is_running = true;
    lifecycle.runtime_mut().source_sessions = Some(SourceSessions {
        screen: Some(SourceSessionMeta {
            session_id: "session-1".to_string(),
            started_at_unix_ms: 123,
        }),
        microphone: None,
        system_audio: None,
    });
    let state = Arc::new(std::sync::Mutex::new(lifecycle));

    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _runtime = state.lock().expect("state should lock before poisoning");
        panic!("poison native capture state");
    }));

    assert!(state.is_poisoned());

    let runtime = lock_runtime_for_idle_debug(&state);
    let runtime = runtime.runtime();

    assert!(runtime.is_running);
    assert_eq!(
        runtime
            .source_sessions
            .as_ref()
            .and_then(|sessions| sessions.screen.as_ref())
            .map(|session| session.session_id.as_str()),
        Some("session-1")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn segment_planner_uses_session_level_audio_directories_for_audio_outputs() {
    let planner = SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-output-layout",
        "session-1",
        "2026/04/16",
    );

    assert_eq!(
        planner.segment_dir(4),
        std::path::PathBuf::from(
            "/tmp/native-capture-output-layout/2026/04/16/.session-1-segment-0004"
        )
    );
    assert_eq!(
        planner.audio_dir(),
        std::path::PathBuf::from("/tmp/native-capture-output-layout/2026/04/16/audio")
    );
    assert_eq!(
        planner.microphone_file(4),
        std::path::PathBuf::from(
            "/tmp/native-capture-output-layout/2026/04/16/audio/session-1-segment-0004.m4a"
        )
    );
    assert_eq!(
        planner.system_audio_file(4),
        std::path::PathBuf::from(
            "/tmp/native-capture-output-layout/2026/04/16/audio/session-1-segment-0004.m4a"
        )
    );
}

#[cfg(target_os = "macos")]
#[test]
fn per_source_planners_keep_screen_microphone_and_system_audio_paths_independent() {
    let screen_planner = SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-output-layout",
        "screen-session",
        "2026/04/16",
    );
    let microphone_planner = SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-output-layout",
        "microphone-session",
        "2026/04/16",
    );
    let system_audio_planner = SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-output-layout",
        "system-audio-session",
        "2026/04/16",
    );

    assert_eq!(
        screen_planner.segment_screen_output(4),
        std::path::PathBuf::from(
            "/tmp/native-capture-output-layout/2026/04/16/screen-session-segment-0004.mov"
        )
    );
    assert_eq!(
        microphone_planner.microphone_file(4),
        std::path::PathBuf::from(
            "/tmp/native-capture-output-layout/2026/04/16/audio/microphone-session-segment-0004.m4a"
        )
    );
    assert_eq!(
        system_audio_planner.system_audio_file(4),
        std::path::PathBuf::from(
            "/tmp/native-capture-output-layout/2026/04/16/audio/system-audio-session-segment-0004.m4a"
        )
    );
}

#[cfg(target_os = "macos")]
#[test]
fn cleanup_failed_segment_dirs_keeps_shared_dated_audio_directory() {
    let base_dir = std::env::temp_dir().join(format!(
        "native-capture-cleanup-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos()
    ));
    let segment_dir = base_dir.join("2026/04/16/.screen-session-segment-0001");
    let audio_dir = base_dir.join("2026/04/16/audio");

    std::fs::create_dir_all(&segment_dir).expect("segment dir should be created");
    std::fs::create_dir_all(&audio_dir).expect("audio dir should be created");
    std::fs::write(audio_dir.join("microphone-session.m4a"), b"audio")
        .expect("audio fixture should be written");

    cleanup_failed_segment_dirs(
        &segment_dir,
        Some(audio_dir.as_path()),
        Some(audio_dir.as_path()),
    );

    assert!(
        !segment_dir.exists(),
        "per-segment workspace should still be removed"
    );
    assert!(
        audio_dir.exists(),
        "shared dated audio directory must not be removed"
    );
    assert!(
        audio_dir.join("microphone-session.m4a").exists(),
        "existing shared audio outputs must be preserved"
    );

    std::fs::remove_dir_all(&base_dir).expect("temp cleanup should succeed");
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
fn rotation_keeps_emitted_segment_numbering_contiguous_when_schedule_jumps_ahead() {
    let scheduled_index = 10;

    assert!(should_rotate_segment(4, scheduled_index));
    assert_eq!(next_emitted_segment_index(4), 5);
}

#[cfg(target_os = "macos")]
#[test]
fn plan_live_rotation_segment_keeps_emitted_numbering_contiguous_when_schedule_jumps_ahead() {
    let runtime = NativeCaptureRuntime {
        current_segment_index: 4,
        ..Default::default()
    };
    let sources = CaptureSources {
        screen: true,
        microphone: true,
        system_audio: true,
    };
    let screen_planner = SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-screen-live",
        "2026/04/28",
    );
    let microphone_planner = SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-microphone-live",
        "2026/04/28",
    );
    let system_audio_planner = SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-system-audio-live",
        "2026/04/28",
    );
    let clock = CaptureClock::start_now();
    let schedule = SegmentSchedule::new(std::time::Duration::from_millis(1));

    std::thread::sleep(std::time::Duration::from_millis(20));

    let planned = plan_live_rotation_segment(
        &runtime,
        &sources,
        &screen_planner,
        Some(&microphone_planner),
        Some(&system_audio_planner),
        &schedule,
        &clock,
    )
    .expect("rotation should still be planned after schedule advances");

    assert_eq!(planned.next_index, 5);
    assert!(planned
        .screen_output_file
        .to_string_lossy()
        .ends_with("segment-0005.mov"));
    assert!(planned
        .microphone_output_path
        .as_ref()
        .expect("microphone path should be planned")
        .to_string_lossy()
        .ends_with("segment-0005.m4a"));
    assert!(planned
        .system_audio_output_path
        .as_ref()
        .expect("system audio path should be planned")
        .to_string_lossy()
        .ends_with("segment-0005.m4a"));
}

#[cfg(target_os = "macos")]
#[test]
fn plan_live_rotation_segment_skips_explicit_all_paused_sources() {
    let runtime = NativeCaptureRuntime {
        current_segment_index: 4,
        ..Default::default()
    };
    let sources = CaptureSources {
        screen: false,
        microphone: false,
        system_audio: false,
    };
    let screen_planner = SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-screen-live",
        "2026/04/28",
    );
    let clock = CaptureClock::start_now();
    let schedule = SegmentSchedule::new(std::time::Duration::from_millis(1));

    std::thread::sleep(std::time::Duration::from_millis(20));

    assert!(
        plan_live_rotation_segment(
            &runtime,
            &sources,
            &screen_planner,
            None,
            None,
            &schedule,
            &clock
        )
        .is_none(),
        "privacy suspension with all sources paused should preserve the explicit paused sentinel"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn rotation_seeds_missing_system_audio_planner_before_planning_output_path() {
    let mut runtime = NativeCaptureRuntime {
        current_segment_index: 4,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        }),
        segment_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "screen-session",
            "2026/04/28",
        )),
        source_sessions: Some(SourceSessions {
            screen: Some(SourceSessionMeta {
                session_id: "screen-session".to_string(),
                started_at_unix_ms: 123,
            }),
            microphone: None,
            system_audio: Some(SourceSessionMeta {
                session_id: "system-audio-session".to_string(),
                started_at_unix_ms: 123,
            }),
        }),
        ..Default::default()
    };
    let sources = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: true,
    };
    let screen_planner = runtime.segment_planner.clone().unwrap();
    let clock = CaptureClock::start_now();
    let schedule = SegmentSchedule::new(std::time::Duration::from_millis(1));

    std::thread::sleep(std::time::Duration::from_millis(20));

    let system_audio_planner =
        ensure_system_audio_planner_for_runtime(&mut runtime, "rotating segments")
            .expect("system-audio planner seeding should succeed");

    let planned = plan_live_rotation_segment(
        &runtime,
        &sources,
        &screen_planner,
        None,
        system_audio_planner.as_ref(),
        &schedule,
        &clock,
    )
    .expect("rotation should be planned after schedule advances");

    assert!(runtime.system_audio_planner.is_some());
    assert_eq!(
        planned
            .system_audio_output_path
            .as_ref()
            .expect("system audio path should be planned")
            .to_string_lossy(),
        "/tmp/native-capture-tests/2026/04/28/audio/system-audio-session-segment-0005.m4a"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn plan_live_rotation_segment_does_not_rotate_for_zero_duration_schedule() {
    let runtime = NativeCaptureRuntime {
        current_segment_index: 1,
        ..Default::default()
    };
    let sources = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: false,
    };
    let screen_planner = SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-screen-live",
        "2026/04/28",
    );
    let clock = CaptureClock::start_now();
    let schedule = SegmentSchedule::new(std::time::Duration::ZERO);

    std::thread::sleep(std::time::Duration::from_millis(10));

    assert!(
        plan_live_rotation_segment(
            &runtime,
            &sources,
            &screen_planner,
            None,
            None,
            &schedule,
            &clock,
        )
        .is_none(),
        "zero-duration schedules should keep rollover disabled"
    );
}

#[test]
fn segment_loop_sleep_duration_uses_idle_poll_interval_for_zero_duration_schedule() {
    let schedule = SegmentSchedule::new(std::time::Duration::ZERO);
    let clock = CaptureClock::start_now();

    assert_eq!(
        segment_loop_sleep_duration(&schedule, &clock),
        std::time::Duration::from_secs(1)
    );
}
