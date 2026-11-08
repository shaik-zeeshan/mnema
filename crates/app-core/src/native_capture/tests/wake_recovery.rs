use super::*;

#[cfg(target_os = "macos")]
#[test]
fn inactivity_resume_soft_pause_is_noop_without_family_pause_state() {
    let mut runtime = paused_runtime_fixture();

    resume_runtime_from_inactivity(&mut runtime)
        .expect("soft-resume should tolerate legacy paused state without restart");

    assert!(runtime.is_running);
    assert_eq!(runtime.runtime_state, RuntimeState::Running);
    assert!(!runtime.inactivity.is_paused);
    assert_eq!(runtime.current_segment_index, 1);
    assert!(runtime.current_segment_output_files.is_none());
    assert!(runtime.recording_file.is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn inactivity_resume_soft_pause_clears_paused_state_without_restarting_outputs() {
    let mut runtime = paused_runtime_fixture();
    resume_runtime_from_inactivity(&mut runtime).expect("legacy soft-resume should succeed");

    assert!(runtime.is_running);
    assert_eq!(runtime.runtime_state, RuntimeState::Running);
    assert!(!runtime.inactivity.is_paused);
    assert_eq!(runtime.current_segment_index, 1);
    assert!(runtime.recording_file.is_none());
    assert!(runtime.current_segment_output_files.is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn wake_recovery_restarts_screen_capture_and_preserves_live_microphone_output() {
    let mut runtime = running_screen_capture_runtime_fixture();
    let expected_screen_file =
        "/tmp/native-capture-tests/2026/04/23/native-session-wake-screen-segment-0002.mov"
            .to_string();

    let recovered = recover_screen_capture_after_wake_with_start_segment(
        &mut runtime,
        None,
        |segment_dir,
         screen_output,
         sources,
         frame_rate,
         resolution,
         bitrate,
         microphone_device_id,
         frame_tx,
         microphone_output_path| {
            assert_eq!(
                sources,
                &CaptureSources {
                    screen: true,
                    microphone: false,
                    system_audio: false,
                },
                "a wake restarts the screen alone; the tap is untouched by it"
            );
            assert_eq!(frame_rate, 5.0);
            assert_eq!(resolution, &ScreenResolution::default());
            assert_eq!(bitrate, None);
            assert_eq!(microphone_device_id, None);
            assert!(frame_tx.is_none());
            assert!(microphone_output_path.is_none());
            assert_eq!(
                segment_dir,
                std::path::Path::new(
                    "/tmp/native-capture-tests/2026/04/23/.native-session-wake-screen-segment-0002"
                )
            );
            assert_eq!(
                screen_output,
                Some(std::path::Path::new(
                    "/tmp/native-capture-tests/2026/04/23/native-session-wake-screen-segment-0002.mov"
                ))
            );

            Ok(resumed_segment_state_fixture(expected_screen_file.clone()))
        },
    )
    .expect("wake recovery should restart screen capture");

    assert!(recovered);
    assert_eq!(runtime.current_segment_index, 2);
    assert_eq!(runtime.recording_file, Some(expected_screen_file.clone()));
    // The tap kept writing across the wake, so its file is neither replanned nor
    // finalized here — it stays exactly as it was.
    assert_eq!(
        runtime.system_audio_recording_file.as_deref(),
        Some("/tmp/system-audio.m4a")
    );
    let outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("wake recovery should refresh current segment outputs");
    assert_eq!(outputs.screen_file, Some(expected_screen_file));
    assert_eq!(outputs.microphone_file.as_deref(), Some("/tmp/mic.m4a"));
    assert_eq!(outputs.microphone_files, vec!["/tmp/mic.m4a".to_string()]);
    assert_eq!(
        runtime.current_segment_sources,
        Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        })
    );
}

// Regression: the wake path rebuilt `current_segment_output_files` from the fresh
// screen start and merged only the microphone continuation, so the file the tap
// had been writing straight through the sleep ended up tracked nowhere. The next
// rotation's `finalize_capture_outputs` then read a segment whose
// `system_audio_file` was `None`, concluded the tap's path was an untracked
// intermediate, and deleted it — up to five minutes of system audio gone on the
// first rotation after every wake, with no `audio_segment` row, no transcription
// and no error. The two sibling restart paths merged both families already.
#[cfg(target_os = "macos")]
#[test]
fn wake_recovery_carries_the_live_system_audio_file_into_the_refreshed_outputs() {
    let mut lifecycle = RecordingLifecycle::default();
    *lifecycle.runtime_mut() = running_screen_capture_runtime_fixture();
    assert!(
        lifecycle.handle_system_will_sleep(),
        "the sleep handler should clear screen state"
    );

    let expected_screen_file =
        "/tmp/native-capture-tests/2026/04/23/native-session-wake-screen-segment-0002.mov"
            .to_string();
    let recovered = recover_screen_capture_after_wake_with_start_segment(
        lifecycle.runtime_mut(),
        None,
        |_segment_dir,
         _screen_output,
         _sources,
         _frame_rate,
         _resolution,
         _bitrate,
         _microphone_device_id,
         _frame_tx,
         _microphone_output_path| {
            Ok(resumed_segment_state_fixture(expected_screen_file.clone()))
        },
    )
    .expect("wake recovery should restart screen capture");

    assert!(recovered);
    let runtime = lifecycle.runtime();
    assert_eq!(
        runtime.system_audio_recording_file.as_deref(),
        Some("/tmp/system-audio.m4a"),
        "the tap never stopped, so the runtime still names its file"
    );
    let outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("wake recovery should refresh current segment outputs");
    assert_eq!(
        outputs.system_audio_file.as_deref(),
        Some("/tmp/system-audio.m4a"),
        "and the segment the next rotation finalizes has to name it too, or finalize deletes it"
    );
    assert_eq!(
        outputs.system_audio_files,
        vec!["/tmp/system-audio.m4a".to_string()]
    );
}

// Regression: a mid-segment tap rebuild finalizes an earlier system-audio
// generation and opens a fresh one, both tracked in the current segment's
// `system_audio_files`. Wake recovery rebuilt the previous segment's outputs by
// clearing *all* system audio and carrying only the live file forward — so the
// finalized earlier generation was committed nowhere: a real `.m4a` on disk with
// no `audio_segment` row, never transcribed, invisible to retention. The suspend
// path already strips only the live file for exactly this reason; the wake path
// must mirror it.
#[cfg(target_os = "macos")]
#[test]
fn wake_recovery_commits_earlier_system_audio_rebuild_generations() {
    let dir = TestDir::new("wake-recovery-system-audio-rebuild-generations");
    let stale_screen_file = dir.path().join("stale-sleep-screen.mov");
    write_openable_screen_file(&stale_screen_file);
    // A finalized earlier tap generation — a real, positive-duration m4a.
    let finalized_generation = dir.path().join("system-audio-gen1.m4a");
    write_valid_m4a_audio_file(&finalized_generation);
    let finalized_generation = finalized_generation.to_string_lossy().to_string();
    // The file the tap is still writing across the wake.
    let live_system_audio_file = "/tmp/system-audio-live.m4a".to_string();

    let mut runtime = running_screen_capture_runtime_fixture();
    runtime.current_segment_output_files = Some(CaptureOutputFiles {
        screen_file: Some(stale_screen_file.to_string_lossy().to_string()),
        screen_files: vec![stale_screen_file.to_string_lossy().to_string()],
        microphone_file: None,
        microphone_files: Vec::new(),
        system_audio_file: Some(live_system_audio_file.clone()),
        system_audio_files: vec![finalized_generation.clone(), live_system_audio_file.clone()],
    });
    runtime.recording_file = Some(stale_screen_file.to_string_lossy().to_string());
    runtime.microphone_recording_file = None;
    runtime.system_audio_recording_file = Some(live_system_audio_file.clone());

    let mut lifecycle = RecordingLifecycle::default();
    *lifecycle.runtime_mut() = runtime;
    assert!(
        lifecycle.handle_system_will_sleep(),
        "the sleep handler should clear screen state"
    );

    let expected_screen_file =
        "/tmp/native-capture-tests/2026/04/23/native-session-wake-screen-segment-0002.mov"
            .to_string();
    let recovered = recover_screen_capture_after_wake_with_start_segment(
        lifecycle.runtime_mut(),
        None,
        |_segment_dir,
         _screen_output,
         _sources,
         _frame_rate,
         _resolution,
         _bitrate,
         _microphone_device_id,
         _frame_tx,
         _microphone_output_path| {
            Ok(resumed_segment_state_fixture(expected_screen_file.clone()))
        },
    )
    .expect("wake recovery should restart screen capture");

    assert!(recovered);
    let runtime = lifecycle.runtime();
    // The live file is still carried into the refreshed segment for the next
    // rotation (the previously fixed bug must stay fixed).
    let outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("wake recovery should refresh current segment outputs");
    assert_eq!(
        outputs.system_audio_file.as_deref(),
        Some(live_system_audio_file.as_str()),
    );
    // ...and the finalized earlier generation is committed with the previous
    // segment rather than orphaned on disk.
    let committed = runtime
        .output_files
        .as_ref()
        .expect("previous segment outputs should be committed during wake recovery");
    assert!(
        committed.system_audio_files.contains(&finalized_generation),
        "the finalized earlier tap generation must be committed, not orphaned: {:?}",
        committed.system_audio_files
    );
    // The valid earlier generation must still exist on disk (it is real audio).
    assert!(std::path::Path::new(&finalized_generation).exists());
}

#[cfg(target_os = "macos")]
#[test]
fn wake_recovery_restarts_the_screen_alone_while_system_audio_is_paused() {
    let mut runtime = running_screen_capture_runtime_fixture();
    runtime
        .inactivity
        .set_family_paused_states(false, false, true);
    runtime.system_audio_recording_file = None;
    if let Some(outputs) = runtime.current_segment_output_files.as_mut() {
        outputs.system_audio_file = None;
        outputs.system_audio_files.clear();
    }
    runtime.current_segment_sources = Some(CaptureSources {
        screen: true,
        microphone: true,
        system_audio: false,
    });

    let expected_screen_file =
        "/tmp/native-capture-tests/2026/04/23/native-session-wake-screen-segment-0002.mov"
            .to_string();

    let recovered = recover_screen_capture_after_wake_with_start_segment(
        &mut runtime,
        None,
        |_segment_dir,
         _screen_output,
         sources,
         _frame_rate,
         _resolution,
         _bitrate,
         _microphone_device_id,
         _frame_tx,
         _microphone_output_path| {
            assert_eq!(
                sources,
                &CaptureSources {
                    screen: true,
                    microphone: false,
                    system_audio: false,
                },
                "the screen stream carries no audio; the tap delivers its own activity and resumes itself"
            );
            Ok(resumed_segment_state_fixture(expected_screen_file.clone()))
        },
    )
    .expect("wake recovery should restart screen capture");

    assert!(recovered);
    assert!(!runtime.inactivity.is_screen_paused());
    assert!(runtime.inactivity.is_system_audio_paused());
    assert!(runtime.system_audio_recording_file.is_none());
    assert_eq!(
        runtime.current_segment_sources,
        Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        })
    );
}

#[cfg(target_os = "macos")]
#[test]
fn system_sleep_clears_live_screen_state_but_preserves_microphone_continuation() {
    let mut lifecycle = RecordingLifecycle::default();
    *lifecycle.runtime_mut() = running_screen_capture_runtime_fixture();

    let handled = lifecycle.handle_system_will_sleep();

    assert!(handled);
    let runtime = lifecycle.runtime();
    assert!(runtime.is_running);
    assert!(runtime.active_screen_session.is_none());
    assert!(runtime.recording_file.is_none());
    // The tap has no display dependency: a sleep leaves it recording, so its
    // in-flight file must stay tracked rather than be orphaned.
    assert_eq!(
        runtime.system_audio_recording_file.as_deref(),
        Some("/tmp/system-audio.m4a")
    );
    assert_eq!(
        runtime.microphone_recording_file.as_deref(),
        Some("/tmp/mic.m4a")
    );
    let outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("microphone continuation should remain trackable");
    assert_eq!(outputs.screen_file.as_deref(), Some("/tmp/screen.mov"));
    assert_eq!(outputs.screen_files, vec!["/tmp/screen.mov".to_string()]);
    assert_eq!(
        outputs.system_audio_file.as_deref(),
        Some("/tmp/system-audio.m4a")
    );
    assert_eq!(
        outputs.system_audio_files,
        vec!["/tmp/system-audio.m4a".to_string()]
    );
    assert_eq!(outputs.microphone_file.as_deref(), Some("/tmp/mic.m4a"));
    assert_eq!(outputs.microphone_files, vec!["/tmp/mic.m4a".to_string()]);
    // Both audio families survive the sleep; only the screen stops.
    assert_eq!(
        runtime.current_segment_sources,
        Some(CaptureSources {
            screen: false,
            microphone: true,
            system_audio: true,
        })
    );

    let session = lifecycle.session();
    assert!(!session.is_running);
    assert_eq!(
        session.requested_sources,
        Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        })
    );
}

#[cfg(target_os = "macos")]
#[test]
fn system_sleep_is_ignored_when_screen_capture_is_not_live() {
    let mut lifecycle = RecordingLifecycle::default();
    *lifecycle.runtime_mut() = paused_runtime_fixture();

    let handled = lifecycle.handle_system_will_sleep();

    assert!(!handled);
    let runtime = lifecycle.runtime();
    assert!(runtime.is_running);
    assert!(runtime.recording_file.is_none());
    assert!(runtime.current_segment_output_files.is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn system_sleep_clears_paused_screen_state_so_wake_can_restart_capture() {
    let mut lifecycle = RecordingLifecycle::default();
    *lifecycle.runtime_mut() = screen_paused_runtime_fixture();
    lifecycle.runtime_mut().requested_sources = Some(CaptureSources {
        screen: true,
        microphone: true,
        system_audio: true,
    });
    lifecycle.runtime_mut().current_segment_sources = Some(CaptureSources {
        screen: false,
        microphone: true,
        system_audio: false,
    });
    lifecycle.runtime_mut().system_audio_planner = Some(SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-screen-pause-system-audio",
        "2026/04/23",
    ));
    lifecycle.runtime_mut().recording_file = Some("/tmp/stale-paused-screen.mov".to_string());
    lifecycle.runtime_mut().system_audio_recording_file =
        Some("/tmp/stale-paused-system-audio.m4a".to_string());

    let handled = lifecycle.handle_system_will_sleep();

    assert!(handled);
    let runtime = lifecycle.runtime();
    assert!(runtime.active_screen_session.is_none());
    assert!(runtime.recording_file.is_none());
    let outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("microphone continuation should remain trackable while screen is paused");
    assert!(outputs.screen_file.is_none());
    assert!(outputs.screen_files.is_empty());
    assert!(outputs.system_audio_file.is_none());
    assert!(outputs.system_audio_files.is_empty());
    assert_eq!(
        outputs.microphone_file.as_deref(),
        Some(
            "/tmp/native-capture-tests/.mnema/segments/native-session-screen-pause/1/audio/microphone.m4a"
        )
    );
    assert_eq!(
        runtime.current_segment_sources,
        Some(CaptureSources {
            screen: false,
            microphone: true,
            system_audio: true,
        })
    );
}

#[cfg(target_os = "macos")]
#[test]
fn system_sleep_handler_matches_broken_screen_session_shape() {
    let mut lifecycle = RecordingLifecycle::default();
    *lifecycle.runtime_mut() = running_screen_capture_runtime_fixture();

    assert!(lifecycle.handle_system_will_sleep());

    let session = lifecycle.session();
    assert!(!session.is_running);
    assert_eq!(
        lifecycle.runtime().current_segment_sources,
        Some(CaptureSources {
            screen: false,
            microphone: true,
            system_audio: true,
        })
    );
    assert!(
        !crate::native_capture::runtime::system_audio_writer_active_for_runtime(
            lifecycle.runtime()
        )
    );
}

#[cfg(target_os = "macos")]
#[test]
fn wake_recovery_failure_clears_screen_bookkeeping_but_preserves_live_microphone() {
    let mut runtime = running_screen_capture_runtime_fixture();

    let error = recover_screen_capture_after_wake_with_start_segment(
        &mut runtime,
        None,
        |_, _, _, _, _, _, _, _, _| {
            Err(CaptureErrorResponse {
                code: "capture_stream_start_failed".to_string(),
                message: "wake restart failed".to_string(),
            })
        },
    )
    .expect_err("wake recovery failure should bubble");

    assert_eq!(error.code, "capture_stream_start_failed");
    assert_eq!(runtime.current_segment_index, 1);
    assert!(runtime.recording_file.is_none());
    let outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("audio continuation should remain trackable");
    assert!(outputs.screen_file.is_none());
    // Both audio families outlive a failed screen restart, so both keep their
    // in-flight files tracked for the next boundary to commit.
    assert_eq!(
        outputs.system_audio_file.as_deref(),
        Some("/tmp/system-audio.m4a")
    );
    assert_eq!(outputs.microphone_file.as_deref(), Some("/tmp/mic.m4a"));
    assert_eq!(outputs.microphone_files, vec!["/tmp/mic.m4a".to_string()]);
    assert!(runtime.is_running);
    assert_eq!(
        runtime.current_segment_sources,
        Some(CaptureSources {
            screen: false,
            microphone: true,
            system_audio: true,
        })
    );
}

#[cfg(target_os = "macos")]
#[test]
fn wake_recovery_restarts_screen_capture_after_sleep_while_screen_was_paused() {
    let mut runtime = screen_paused_runtime_fixture();
    runtime.requested_sources = Some(CaptureSources {
        screen: true,
        microphone: true,
        system_audio: true,
    });
    runtime.current_segment_sources = Some(CaptureSources {
        screen: false,
        microphone: true,
        system_audio: false,
    });
    runtime.system_audio_planner = Some(SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-screen-pause-system-audio",
        "2026/04/23",
    ));

    let expected_date_prefix = current_date_prefix();
    let expected_screen_file = format!(
        "/tmp/native-capture-tests/{expected_date_prefix}/native-session-screen-pause-segment-0002.mov"
    );
    let recovered = recover_screen_capture_after_wake_with_start_segment(
        &mut runtime,
        None,
        |segment_dir,
         screen_output_file,
         sources,
         screen_frame_rate,
         _screen_resolution,
         _effective_screen_bitrate_bps,
         _microphone_device_id,
         _frame_artifact_tx,
         _microphone_output_path| {
            assert_eq!(
                segment_dir.to_string_lossy(),
                format!(
                    "/tmp/native-capture-tests/{expected_date_prefix}/.native-session-screen-pause-segment-0002"
                )
            );
            assert_eq!(
                screen_output_file.expect("screen output should be planned").to_string_lossy(),
                expected_screen_file
            );
            assert_eq!(
                *sources,
                CaptureSources {
                    screen: true,
                    microphone: false,
                    system_audio: false,
                },
                "the screen restart never plans a system-audio file; the tap owns its own"
            );
            assert_eq!(screen_frame_rate, 5.0);

            Ok(resumed_segment_state_fixture(expected_screen_file.clone()))
        },
    )
    .expect("wake recovery should restart paused screen capture");

    assert!(recovered);
    assert_eq!(runtime.current_segment_index, 2);
    assert_eq!(runtime.recording_file, Some(expected_screen_file.clone()));
    let outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("wake recovery should refresh current segment outputs");
    assert_eq!(outputs.screen_file, Some(expected_screen_file));
    assert_eq!(
        outputs.microphone_file.as_deref(),
        Some(
            "/tmp/native-capture-tests/.mnema/segments/native-session-screen-pause/1/audio/microphone.m4a"
        )
    );
    assert!(
        outputs.system_audio_file.is_none(),
        "the screen restart contributes no system-audio file; the tap owns its own"
    );
    assert_eq!(
        runtime.current_segment_sources,
        Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        })
    );
    assert!(!runtime.inactivity.is_screen_paused());
}

#[cfg(target_os = "macos")]
#[test]
fn wake_recovery_finalizes_stale_screen_output_after_sleep_even_when_recording_file_was_cleared() {
    let dir = TestDir::new("wake-recovery-sleep-stale-screen");
    let stale_screen_file = dir.path().join("stale-sleep-screen.mov");
    write_openable_screen_file(&stale_screen_file);

    let mut runtime = running_screen_capture_runtime_fixture();
    runtime.requested_sources = Some(CaptureSources {
        screen: true,
        microphone: true,
        system_audio: false,
    });
    runtime.current_segment_sources = Some(CaptureSources {
        screen: true,
        microphone: true,
        system_audio: false,
    });
    runtime.system_audio_planner = None;
    runtime.current_segment_output_files = Some(CaptureOutputFiles {
        screen_file: Some(stale_screen_file.to_string_lossy().to_string()),
        screen_files: vec![stale_screen_file.to_string_lossy().to_string()],
        microphone_file: Some("/tmp/mic.m4a".to_string()),
        microphone_files: vec!["/tmp/mic.m4a".to_string()],
        system_audio_file: None,
        system_audio_files: Vec::new(),
    });
    runtime.recording_file = Some(stale_screen_file.to_string_lossy().to_string());
    runtime.system_audio_recording_file = None;

    let mut lifecycle = RecordingLifecycle::default();
    *lifecycle.runtime_mut() = runtime;
    assert!(lifecycle.handle_system_will_sleep());

    let expected_screen_file =
        format!("/tmp/native-capture-tests/2026/04/23/native-session-wake-screen-segment-0002.mov");

    let recovered = recover_screen_capture_after_wake_with_start_segment(
        lifecycle.runtime_mut(),
        None,
        |segment_dir,
         screen_output_file,
         sources,
         screen_frame_rate,
         _screen_resolution,
         _effective_screen_bitrate_bps,
         _microphone_device_id,
         _frame_artifact_tx,
         _microphone_output_path| {
            assert_eq!(
                segment_dir.to_string_lossy(),
                "/tmp/native-capture-tests/2026/04/23/.native-session-wake-screen-segment-0002"
            );
            assert_eq!(
                screen_output_file
                    .expect("screen output should be planned")
                    .to_string_lossy(),
                expected_screen_file
            );
            assert_eq!(
                *sources,
                CaptureSources {
                    screen: true,
                    microphone: false,
                    system_audio: false,
                }
            );
            assert_eq!(screen_frame_rate, 5.0);

            Ok(resumed_segment_state_fixture(expected_screen_file.clone()))
        },
    )
    .expect("wake recovery should restart screen capture");

    assert!(recovered);
    let runtime = lifecycle.runtime();
    let committed = runtime
        .output_files
        .as_ref()
        .expect("stale output files should be committed during wake recovery");
    assert_eq!(
        committed.screen_file.as_deref(),
        Some(stale_screen_file.to_string_lossy().as_ref())
    );
    assert_eq!(
        committed.screen_files,
        vec![stale_screen_file.to_string_lossy().to_string()]
    );
    assert!(committed.system_audio_file.is_none());
    assert!(committed.system_audio_files.is_empty());
    assert_eq!(runtime.current_segment_index, 2);
    assert_eq!(runtime.recording_file, Some(expected_screen_file));
    assert!(runtime.system_audio_recording_file.is_none());
}
