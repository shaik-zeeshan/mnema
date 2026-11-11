use super::*;

#[cfg(target_os = "macos")]
#[test]
fn audio_segment_window_uses_actual_media_duration() {
    let (started_at, ended_at) = audio_segment_window_from_duration_ms(1_774_000_000_000, 9_000);

    assert_eq!(started_at, "2026-03-20T09:46:40Z");
    assert_eq!(ended_at, "2026-03-20T09:46:49Z");
}

#[cfg(target_os = "macos")]
#[test]
fn audio_duration_time_to_ms_preserves_fractional_seconds() {
    assert_eq!(
        audio_duration_time_to_ms(cidre::cm::Time::new(333, 1_000)),
        Some(333)
    );
    assert_eq!(
        audio_duration_time_to_ms(cidre::cm::Time::new(29_999, 10_000)),
        Some(3_000)
    );
    assert_eq!(
        audio_duration_time_to_ms(cidre::cm::Time::new(1, 3)),
        Some(333)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn audio_duration_time_to_ms_rejects_non_positive_and_non_numeric_duration() {
    assert_eq!(
        audio_duration_time_to_ms(cidre::cm::Time::new(0, 1_000)),
        None
    );
    assert_eq!(
        audio_duration_time_to_ms(cidre::cm::Time::new(-1, 1_000)),
        None
    );
    assert_eq!(
        audio_duration_time_to_ms(cidre::cm::Time::indefinit()),
        None
    );
}

#[cfg(target_os = "macos")]
#[test]
fn audio_segment_start_uses_embedded_restart_timestamp() {
    let source_session = SourceSessionMeta {
        session_id: "mic-source".to_string(),
        started_at_unix_ms: 1_700_000_000_000,
    };
    let schedule = SegmentSchedule::new(std::time::Duration::from_secs(60));

    assert_eq!(
        audio_segment_started_at_unix_ms_for_file(
            &source_session,
            2,
            &schedule,
            "/tmp/audio/microphone-mic-source-segment-0002-1700000099999.m4a",
        ),
        1_700_000_099_999
    );
}

#[cfg(target_os = "macos")]
#[test]
fn audio_segment_start_falls_back_to_scheduled_boundary_for_base_file() {
    let source_session = SourceSessionMeta {
        session_id: "mic-source".to_string(),
        started_at_unix_ms: 1_700_000_000_000,
    };
    let schedule = SegmentSchedule::new(std::time::Duration::from_secs(60));

    assert_eq!(
        audio_segment_started_at_unix_ms_for_file(
            &source_session,
            2,
            &schedule,
            "/tmp/audio/microphone-mic-source-segment-0002.m4a",
        ),
        1_700_000_060_000
    );
}

#[cfg(target_os = "macos")]
#[test]
fn trimmed_microphone_finalization_keeps_segment_index_in_timestamped_filename() {
    let dir = TestDir::new("trimmed-microphone-filename");
    let microphone_file = write_existing_audio_placeholder(
        &dir.path()
            .join("microphone-native-session-microphone-segment-0001.m4a"),
    );
    let source_sessions = independent_source_sessions_fixture();
    let schedule = SegmentSchedule::new(std::time::Duration::from_secs(60));
    let shifted_started_at_unix_ms = 1_123;
    let finalization = capture_microphone::MicrophoneOutputFinalization {
        source_file: Some(microphone_file.clone()),
        output_file: Some(microphone_file.clone()),
        speech_detected: true,
        start_offset_ms: 1_000,
        discard_reason: None,
    };
    let mut output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: Some(microphone_file.clone()),
        microphone_files: vec![microphone_file],
        system_audio_file: None,
        system_audio_files: Vec::new(),
    };

    apply_microphone_output_finalization(
        Some(&mut output_files),
        &finalization,
        Some(&source_sessions),
        Some(&schedule),
        1,
    );

    let renamed_file = output_files
        .microphone_file
        .as_deref()
        .expect("trimmed microphone output should remain present");
    assert!(
        renamed_file.ends_with("microphone-native-session-microphone-segment-0001-1123.m4a"),
        "trimmed microphone filename must preserve the segment index"
    );
    assert_eq!(
        audio_segment_started_at_unix_ms_for_file(
            source_sessions.microphone.as_ref().unwrap(),
            1,
            &schedule,
            renamed_file,
        ),
        shifted_started_at_unix_ms
    );
}

// A stop renamed the microphone's file (a resume's queued audio pulled its
// start earlier), then the display went: the suspension, which commits screen
// and system audio only, must carry the renamed file for the next rotation,
// not the stale name the session was recording to.
#[cfg(target_os = "macos")]
#[test]
fn a_stopped_microphone_file_renamed_by_its_finalization_survives_a_display_suspension() {
    let dir = TestDir::new("stopped-microphone-display-suspension");
    let screen_file = dir.path().join("screen-segment.mov");
    fs::write(&screen_file, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
        .expect("fake openable mov should be written");
    let screen_file = screen_file.to_string_lossy().into_owned();
    let microphone_file = write_existing_audio_placeholder(
        &dir.path()
            .join("microphone-native-session-microphone-segment-0001-1700000030000.m4a"),
    );
    let schedule = SegmentSchedule::new(std::time::Duration::from_secs(60));
    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_index: 1,
        segment_schedule: Some(schedule),
        source_sessions: Some(independent_source_sessions_fixture()),
        output_files: Some(crate::native_capture::segments::empty_output_files()),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some(screen_file.clone()),
            screen_files: vec![screen_file.clone()],
            microphone_file: Some(microphone_file.clone()),
            microphone_files: vec![microphone_file.clone()],
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: Some(screen_file),
        microphone_recording_file: Some(microphone_file.clone()),
        ..Default::default()
    };
    apply_stopped_microphone_finalization(
        &mut runtime,
        &capture_microphone::MicrophoneOutputFinalization {
            source_file: Some(microphone_file.clone()),
            output_file: Some(microphone_file.clone()),
            speech_detected: true,
            start_offset_ms: -1_365,
            discard_reason: None,
        },
    );

    crate::native_capture::segments::suspend_screen_capture(
        None,
        &mut runtime,
        &CaptureErrorResponse {
            code: "display_unavailable".to_string(),
            message: "display asleep".to_string(),
        },
        CaptureSuspensionKind::DisplayUnavailable,
    )
    .expect("display suspension should succeed");

    let carried = runtime
        .current_segment_output_files
        .as_ref()
        .expect("the closed microphone file is still owed a commit");
    let [renamed_file] = carried.microphone_files.as_slice() else {
        panic!("expected one file: {:?}", carried.microphone_files);
    };
    assert_ne!(renamed_file, &microphone_file);
    assert!(Path::new(renamed_file).is_file());
    assert!(
        runtime.microphone_recording_file.is_none(),
        "nothing records"
    );
}

// A stop's finalization (here, a resume's queued audio pulling the start
// 1.365 s earlier) is booked against the live segment: the segment's file is
// renamed to the corrected stamp and stays the one the commit picks up.
#[cfg(target_os = "macos")]
#[test]
fn stopped_microphone_finalization_restamps_the_live_segment_file() {
    let dir = TestDir::new("stopped-microphone-finalization");
    let microphone_file = write_existing_audio_placeholder(
        &dir.path()
            .join("microphone-native-session-microphone-segment-0001-1700000030000.m4a"),
    );
    let source_sessions = independent_source_sessions_fixture();
    let schedule = SegmentSchedule::new(std::time::Duration::from_secs(60));
    let mut runtime = NativeCaptureRuntime {
        current_segment_index: 1,
        segment_schedule: Some(schedule.clone()),
        source_sessions: Some(source_sessions.clone()),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: Some(microphone_file.clone()),
            microphone_files: vec![microphone_file.clone()],
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        ..Default::default()
    };
    let finalization = capture_microphone::MicrophoneOutputFinalization {
        source_file: Some(microphone_file.clone()),
        output_file: Some(microphone_file),
        speech_detected: true,
        start_offset_ms: -1_365,
        discard_reason: None,
    };

    apply_stopped_microphone_finalization(&mut runtime, &finalization);

    let output_files = runtime
        .current_segment_output_files
        .as_ref()
        .expect("live segment outputs should remain");
    let [restamped_file] = output_files.microphone_files.as_slice() else {
        panic!("expected one file: {:?}", output_files.microphone_files);
    };
    assert_eq!(output_files.microphone_file.as_ref(), Some(restamped_file));
    assert_eq!(
        audio_segment_started_at_unix_ms_for_file(
            source_sessions.microphone.as_ref().unwrap(),
            1,
            &schedule,
            restamped_file,
        ),
        1_700_000_028_635
    );
    assert!(Path::new(restamped_file).is_file());
}

#[cfg(target_os = "macos")]
#[test]
fn microphone_finalization_preserves_prior_rotated_outputs() {
    let first_file = "/tmp/microphone-native-session-microphone-segment-0001-1000.m4a".to_string();
    let current_file =
        "/tmp/microphone-native-session-microphone-segment-0001-2000.m4a".to_string();
    let finalized_file =
        "/tmp/microphone-native-session-microphone-segment-0001-2000-final.m4a".to_string();
    let finalization = capture_microphone::MicrophoneOutputFinalization {
        source_file: Some(current_file.clone()),
        output_file: Some(finalized_file.clone()),
        speech_detected: true,
        start_offset_ms: 0,
        discard_reason: None,
    };
    let mut output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: Some(current_file.clone()),
        microphone_files: vec![first_file.clone(), current_file],
        system_audio_file: None,
        system_audio_files: Vec::new(),
    };

    apply_microphone_output_finalization(
        Some(&mut output_files),
        &finalization,
        Some(&independent_source_sessions_fixture()),
        Some(&SegmentSchedule::new(std::time::Duration::from_secs(60))),
        1,
    );

    assert_eq!(output_files.microphone_file, Some(finalized_file.clone()));
    assert_eq!(
        output_files.microphone_files,
        vec![first_file, finalized_file]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn discarded_microphone_finalization_preserves_prior_rotated_outputs() {
    let first_file = "/tmp/microphone-native-session-microphone-segment-0001-1000.m4a".to_string();
    let current_file =
        "/tmp/microphone-native-session-microphone-segment-0001-2000.m4a".to_string();
    let finalization = capture_microphone::MicrophoneOutputFinalization {
        source_file: Some(current_file.clone()),
        output_file: None,
        speech_detected: false,
        start_offset_ms: 0,
        discard_reason: Some("no_vad_speech".to_string()),
    };
    let mut output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: Some(current_file.clone()),
        microphone_files: vec![first_file.clone(), current_file],
        system_audio_file: None,
        system_audio_files: Vec::new(),
    };

    apply_microphone_output_finalization(
        Some(&mut output_files),
        &finalization,
        Some(&independent_source_sessions_fixture()),
        Some(&SegmentSchedule::new(std::time::Duration::from_secs(60))),
        1,
    );

    assert_eq!(output_files.microphone_file, Some(first_file.clone()));
    assert_eq!(output_files.microphone_files, vec![first_file]);
}

#[cfg(target_os = "macos")]
#[test]
fn audio_segment_start_uses_reanchored_session_timing_for_contiguous_late_segment() {
    let mut runtime = NativeCaptureRuntime {
        current_segment_index: 5,
        capture_clock: Some(CaptureClock::start_now()),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(60))),
        source_sessions: Some(SourceSessions {
            screen: None,
            microphone: Some(SourceSessionMeta {
                session_id: "mic-source".to_string(),
                started_at_unix_ms: 1_700_000_000_000,
            }),
            system_audio: Some(SourceSessionMeta {
                session_id: "system-source".to_string(),
                started_at_unix_ms: 1_700_000_000_000,
            }),
        }),
        ..Default::default()
    };

    reanchor_active_segment_timing(&mut runtime, "test reanchor")
        .expect("active segment timing should reanchor");

    let source_sessions = runtime
        .source_sessions
        .as_ref()
        .expect("source sessions should remain present");
    let schedule = runtime
        .segment_schedule
        .as_ref()
        .expect("schedule should remain present");
    let dir = TestDir::new("audio-segment-reanchored-files");
    let microphone_file = write_existing_audio_placeholder(
        &dir.path().join("microphone-mic-source-segment-0005.m4a"),
    );
    let system_audio_file = write_existing_audio_placeholder(
        &dir.path()
            .join("system-audio-system-source-segment-0005.m4a"),
    );
    let output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: Some(microphone_file.clone()),
        microphone_files: vec![microphone_file],
        system_audio_file: Some(system_audio_file.clone()),
        system_audio_files: vec![system_audio_file],
    };

    let segments = committed_audio_segments_for_output_files(
        Some(source_sessions),
        Some(schedule),
        runtime.current_segment_index,
        Some(&output_files),
    );

    assert_eq!(segments.len(), 2);
    assert_ne!(
        segments[0].started_at, "2023-11-14T22:17:20Z",
        "segment 5 must not be anchored to the original session start after reanchor"
    );
    for segment in segments {
        assert_eq!(
            segment.started_at,
            audio_segment_window_from_duration_ms(
                audio_segment_started_at_unix_ms_for_file(
                    match segment.source_kind {
                        ::app_infra::AudioSegmentSourceKind::Microphone => {
                            source_sessions.microphone.as_ref().unwrap()
                        }
                        ::app_infra::AudioSegmentSourceKind::SystemAudio => {
                            source_sessions.system_audio.as_ref().unwrap()
                        }
                    },
                    runtime.current_segment_index,
                    schedule,
                    &segment.file_path,
                ),
                60_000,
            )
            .0
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn reanchoring_moves_the_schedule_anchor_but_not_the_recording_start() {
    let started = 1_700_000_000_000;
    let mut runtime = NativeCaptureRuntime {
        current_segment_index: 5,
        capture_clock: Some(CaptureClock::start_now()),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(60))),
        source_sessions: Some(SourceSessions {
            screen: Some(SourceSessionMeta {
                session_id: "screen-source".to_string(),
                started_at_unix_ms: started,
            }),
            microphone: None,
            system_audio: None,
        }),
        recording_started_at_unix_ms: Some(started),
        ..Default::default()
    };

    // The re-anchor a screen resume after an inactivity pause performs.
    reanchor_active_segment_timing(&mut runtime, "resuming screen from inactivity")
        .expect("active segment timing should reanchor");

    let anchor = runtime
        .source_sessions
        .as_ref()
        .unwrap()
        .screen
        .as_ref()
        .unwrap();
    assert_ne!(
        anchor.started_at_unix_ms, started,
        "the schedule anchor moves"
    );
    assert_eq!(runtime.recording_started_at_unix_ms, Some(started));
    assert_eq!(
        crate::native_capture::runtime::session_from_runtime(&runtime).recording_started_at_unix_ms,
        Some(started)
    );
    assert_eq!(
        crate::native_capture::capture_health::read_locked(&runtime)
            .dto
            .started_at_ms,
        Some(started)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn fresh_start_inactivity_empty_audio_outputs_leave_no_output_or_db_payloads() {
    let output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: None,
        microphone_files: Vec::new(),
        system_audio_file: None,
        system_audio_files: Vec::new(),
    };

    let segments = committed_audio_segments_for_output_files(
        Some(&independent_source_sessions_fixture()),
        Some(&SegmentSchedule::new(std::time::Duration::from_secs(60))),
        1,
        Some(&output_files),
    );

    assert!(output_files.microphone_file.is_none());
    assert!(output_files.microphone_files.is_empty());
    assert!(output_files.system_audio_file.is_none());
    assert!(output_files.system_audio_files.is_empty());
    assert!(segments.is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn committed_audio_segments_skip_missing_output_files() {
    let output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: Some("/tmp/missing-microphone-output.m4a".to_string()),
        microphone_files: vec!["/tmp/missing-microphone-output.m4a".to_string()],
        system_audio_file: Some("/tmp/missing-system-audio-output.m4a".to_string()),
        system_audio_files: vec!["/tmp/missing-system-audio-output.m4a".to_string()],
    };

    let segments = committed_audio_segments_for_output_files(
        Some(&independent_source_sessions_fixture()),
        Some(&SegmentSchedule::new(std::time::Duration::from_secs(60))),
        1,
        Some(&output_files),
    );

    assert!(
        segments.is_empty(),
        "missing finalized audio artifacts must not become persisted Audio Segment payloads"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn fresh_start_inactivity_valid_active_audio_outputs_survive_db_payload_planning() {
    let dir = TestDir::new("startup-active-audio-files");
    let microphone_file =
        write_existing_audio_placeholder(&dir.path().join("startup-active-microphone.m4a"));
    let system_audio_file =
        write_existing_audio_placeholder(&dir.path().join("startup-active-system-audio.m4a"));
    let output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: Some(microphone_file.clone()),
        microphone_files: vec![microphone_file.clone()],
        system_audio_file: Some(system_audio_file.clone()),
        system_audio_files: vec![system_audio_file.clone()],
    };

    let segments = committed_audio_segments_for_output_files(
        Some(&independent_source_sessions_fixture()),
        Some(&SegmentSchedule::new(std::time::Duration::from_secs(60))),
        1,
        Some(&output_files),
    );

    assert_eq!(segments.len(), 2);
    assert!(segments
        .iter()
        .any(|segment| segment.file_path == microphone_file));
    assert!(segments
        .iter()
        .any(|segment| segment.file_path == system_audio_file));
}
