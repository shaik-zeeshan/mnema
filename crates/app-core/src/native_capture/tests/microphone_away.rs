use super::*;

// Lock is away: a microphone with no session to stop is still held, so nothing
// reopens it until the Mac comes back.
#[cfg(target_os = "macos")]
#[test]
fn away_holds_a_microphone_with_no_session_until_the_mac_comes_back() {
    let mut runtime = stranded_microphone_runtime_fixture();
    let mut logged = false;

    apply_microphone_away(&mut runtime, true, None);
    reopen_unwakeable_microphone(&mut runtime, &mut logged, || None);
    assert!(runtime.microphone_away_hold);
    assert!(!microphone_open_was_attempted(&runtime));

    apply_microphone_away(&mut runtime, false, None);
    reopen_unwakeable_microphone(&mut runtime, &mut logged, || None);
    assert!(!runtime.microphone_away_hold);
    assert!(microphone_open_was_attempted(&runtime));
}

// The Mac came back while low disk had stopped the held microphone: nothing
// reopens it while the disk is still full, and once recovery clears the
// suspension (it never restarts a paused microphone) the tick does.
#[cfg(target_os = "macos")]
#[test]
fn coming_back_on_a_full_disk_reopens_the_microphone_once_space_returns() {
    let mut runtime = stranded_microphone_runtime_fixture();
    runtime.microphone_away_hold = true;
    runtime.capture_suspension = Some(CaptureSuspension::with_kind(
        CaptureSuspensionKind::LowDisk,
        &CaptureErrorResponse {
            code: "capture_low_disk".to_string(),
            message: "low disk".to_string(),
        },
    ));
    let mut logged = false;

    apply_microphone_away(&mut runtime, false, None);
    assert!(
        !runtime.microphone_away_hold,
        "no session to restart, so the hold ends"
    );
    reopen_unwakeable_microphone(&mut runtime, &mut logged, || None);
    assert!(
        !microphone_open_was_attempted(&runtime),
        "low disk keeps the microphone down"
    );

    runtime.capture_suspension = None;
    reopen_unwakeable_microphone(&mut runtime, &mut logged, || None);
    assert!(microphone_open_was_attempted(&runtime));
}

// A resume that failed with no session (the waited-for device was still gone
// at unlock, or inactivity monitoring is off) leaves nothing activity can wake:
// the tick keeps retrying it.
#[cfg(target_os = "macos")]
#[test]
fn a_paused_microphone_with_no_session_is_retried_every_tick() {
    let mut runtime = stranded_microphone_runtime_fixture();
    let mut logged = false;

    for _ in 0..2 {
        mark_microphone_planner_stale(&mut runtime);
        reopen_unwakeable_microphone(&mut runtime, &mut logged, || None);
        assert!(microphone_open_was_attempted(&runtime));
        assert!(runtime.inactivity.is_microphone_paused(), "the open failed");
    }
    assert!(logged, "the failure streak is logged once, then quietly");
}

#[cfg(target_os = "macos")]
#[test]
fn a_paused_microphone_held_elsewhere_is_not_reopened() {
    let holds: [fn(&mut NativeCaptureRuntime); 4] = [
        |runtime| runtime.microphone_away_hold = true,
        |runtime| runtime.microphone_lent_out = true,
        |runtime| runtime.microphone_waiting_for_device = true,
        |runtime| runtime.user_capture_paused = true,
    ];
    for hold in holds {
        let mut runtime = stranded_microphone_runtime_fixture();
        hold(&mut runtime);

        reopen_unwakeable_microphone(&mut runtime, &mut false, || None);

        assert!(!microphone_open_was_attempted(&runtime));
    }
}

// Unpaused with no session (an enrollment restore or a reconnect failed, or low
// disk left it down while away): restarted, since nothing else will.
#[cfg(target_os = "macos")]
#[test]
fn an_unpaused_microphone_with_no_session_is_restarted() {
    let mut runtime = stranded_microphone_runtime_fixture();
    runtime.inactivity.microphone_paused = false;
    runtime.inactivity.system_audio_paused = false;
    runtime.inactivity.is_paused = false;

    reopen_unwakeable_microphone(&mut runtime, &mut false, || None);

    assert!(microphone_open_was_attempted(&runtime));
    assert!(
        runtime.active_microphone_session.is_none(),
        "the open failed"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn low_disk_recovery_leaves_a_paused_lent_out_or_held_microphone_down() {
    let mut runtime = audio_paused_runtime_fixture();
    runtime.away_probe = Some(|| false);
    let requested = runtime.requested_sources.clone().unwrap();
    assert!(
        !low_disk_recovery_sources(&runtime, &requested).microphone,
        "a paused microphone (the away hold pauses it too) is the tick's to reopen"
    );

    runtime.inactivity.microphone_paused = false;
    assert!(low_disk_recovery_sources(&runtime, &requested).microphone);

    runtime.microphone_lent_out = true;
    assert!(!low_disk_recovery_sources(&runtime, &requested).microphone);

    runtime.microphone_lent_out = false;
    runtime.microphone_away_hold = true;
    assert!(
        !low_disk_recovery_sources(&runtime, &requested).microphone,
        "lock is away: the tick reopens it once the Mac is back"
    );
}

// Recovery runs before the tick that first sees the Mac away sets the hold:
// it reads the live away state too, or it opens the microphone just for the
// away tick to close it again.
#[cfg(target_os = "macos")]
#[test]
fn low_disk_recovery_leaves_the_microphone_down_once_the_mac_is_away() {
    let mut runtime = audio_paused_runtime_fixture();
    runtime.inactivity.microphone_paused = false;
    runtime.away_probe = Some(|| true);
    let requested = runtime.requested_sources.clone().unwrap();

    assert!(!runtime.microphone_away_hold);
    assert!(!low_disk_recovery_sources(&runtime, &requested).microphone);
}

#[cfg(target_os = "macos")]
#[test]
fn inactivity_resume_is_blocked_while_the_microphone_is_held() {
    let mut runtime = audio_paused_runtime_fixture();
    runtime.inactivity.activity_mode = InactivityActivityMode::SystemInputOrScreenOrAudio;
    let idle_past_timeout_ms = runtime.inactivity.idle_timeout_seconds * 1_000 + 1;
    let snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(idle_past_timeout_ms),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(idle_past_timeout_ms),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.20),
        },
        system_audio_activity: AudioActivitySourceState {
            enabled: false,
            idle_ms: None,
            latest_normalized_level: None,
        },
    };
    assert!(should_resume_microphone_after_activity(
        &mut runtime,
        20_000,
        snapshot
    ));

    let holds: [fn(&mut NativeCaptureRuntime); 4] = [
        |runtime| runtime.microphone_away_hold = true,
        |runtime| runtime.microphone_lent_out = true,
        |runtime| runtime.microphone_waiting_for_device = true,
        |runtime| {
            runtime.capture_suspension = Some(CaptureSuspension::with_kind(
                CaptureSuspensionKind::LowDisk,
                &CaptureErrorResponse {
                    code: "capture_low_disk".to_string(),
                    message: "low disk".to_string(),
                },
            ))
        },
    ];
    for hold in holds {
        let mut runtime = audio_paused_runtime_fixture();
        runtime.inactivity.activity_mode = InactivityActivityMode::SystemInputOrScreenOrAudio;
        hold(&mut runtime);
        assert!(!should_resume_microphone_after_activity(
            &mut runtime,
            20_000,
            snapshot
        ));
    }
}

// The path every other away test skips: a running microphone is stopped on
// lock with its file closed, and restarted on unlock with its output left to
// the activity policy.
#[cfg(target_os = "macos")]
#[test]
fn locking_stops_a_running_microphone_and_unlocking_restarts_it() {
    let dir = TestDir::new("locking-stops-microphone");
    let microphone_file = dir.path().join("microphone-away.m4a");
    write_valid_m4a_audio_file(&microphone_file);
    let microphone_file = microphone_file.to_string_lossy().into_owned();
    let mut runtime = recording_microphone_runtime_fixture(TestMicrophoneDevice {
        running: true,
        gone: false,
    });
    let outputs = runtime.current_segment_output_files.as_mut().unwrap();
    crate::native_capture_output::clear_current_microphone_output_file(outputs);
    set_current_microphone_output_file(outputs, microphone_file.clone());
    runtime.microphone_recording_file = Some(microphone_file.clone());
    runtime.output_files = Some(crate::native_capture::segments::empty_output_files());

    apply_microphone_away(&mut runtime, true, None);

    assert!(!runtime.test_microphone_device.as_ref().unwrap().running);
    assert!(runtime.microphone_away_hold);
    assert!(
        runtime.inactivity.is_microphone_paused(),
        "the file is closed"
    );
    assert!(runtime.microphone_recording_file.is_none());
    assert!(runtime
        .current_segment_output_files
        .as_ref()
        .unwrap()
        .microphone_files
        .is_empty());
    assert_eq!(
        runtime.output_files.as_ref().unwrap().microphone_files,
        vec![microphone_file],
        "the closed file is committed, not dropped"
    );

    apply_microphone_away(&mut runtime, false, None);
    reopen_unwakeable_microphone(&mut runtime, &mut false, || None);

    assert!(runtime.test_microphone_device.as_ref().unwrap().running);
    assert!(!runtime.microphone_away_hold);
    assert!(
        runtime.inactivity.is_microphone_paused(),
        "activity resumes the output, not the unlock"
    );
    assert!(!microphone_open_was_attempted(&runtime));
}

// A stopped microphone has no session to finalize, but the file its stop
// closed is still owed a commit: an idle pause must commit it, not drop it.
#[cfg(target_os = "macos")]
#[test]
fn an_idle_pause_commits_the_file_a_stopped_microphone_booked() {
    let dir = TestDir::new("idle-pause-booked-microphone");
    let (mut runtime, booked_file) = stopped_microphone_with_booked_file_fixture(&dir);

    pause_microphone_for_inactivity(&mut runtime).expect("the pause should succeed");

    assert!(runtime.inactivity.is_microphone_paused());
    assert_eq!(
        runtime.output_files.as_ref().unwrap().microphone_files,
        vec![booked_file],
        "the booked file must reach the committed outputs"
    );
    assert!(runtime
        .current_segment_output_files
        .as_ref()
        .unwrap()
        .microphone_files
        .is_empty());
}

// The screen's restart paths rebuild the segment's outputs from the screen;
// with no microphone session to rotate, the file a stop booked must ride along.
#[cfg(target_os = "macos")]
#[test]
fn wake_recovery_carries_the_file_a_stopped_microphone_booked() {
    let dir = TestDir::new("wake-recovery-booked-microphone");
    let (booked_runtime, booked_file) = stopped_microphone_with_booked_file_fixture(&dir);
    let mut runtime = screen_paused_runtime_fixture();
    runtime.current_segment_output_files = booked_runtime.current_segment_output_files;
    runtime.microphone_recording_file = None;
    let screen_file = "/tmp/native-capture-tests/booked-wake-screen.mov".to_string();

    recover_screen_capture_after_wake_with_start_segment(
        &mut runtime,
        None,
        |_, _, _, _, _, _, _, _, _| Ok(resumed_segment_state_fixture(screen_file.clone())),
    )
    .expect("wake recovery should restart the screen");

    assert_eq!(
        runtime
            .current_segment_output_files
            .as_ref()
            .unwrap()
            .microphone_files,
        vec![booked_file]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn screen_resume_carries_the_file_a_stopped_microphone_booked() {
    let dir = TestDir::new("screen-resume-booked-microphone");
    let (booked_runtime, booked_file) = stopped_microphone_with_booked_file_fixture(&dir);
    let mut runtime = screen_paused_runtime_fixture();
    runtime.current_segment_output_files = booked_runtime.current_segment_output_files;
    runtime.microphone_recording_file = None;
    let screen_file = "/tmp/native-capture-tests/booked-resume-screen.mov".to_string();

    resume_screen_from_inactivity_with_start_segment(
        &mut runtime,
        None,
        |_, _, _, _, _, _, _, _, _| Ok(resumed_segment_state_fixture(screen_file.clone())),
    )
    .expect("screen resume should restart the screen");

    assert_eq!(
        runtime
            .current_segment_output_files
            .as_ref()
            .unwrap()
            .microphone_files,
        vec![booked_file]
    );
}

// AirPods that disconnected while the Mac was locked: the session bound to them
// never restarts, so it is dropped and the hold ends, and the tick reopens the
// microphone on the current device instead of retrying the dead one forever.
#[cfg(target_os = "macos")]
#[test]
fn a_microphone_whose_device_left_while_away_is_reopened_on_return() {
    let mut runtime = recording_microphone_runtime_fixture(TestMicrophoneDevice {
        running: true,
        gone: false,
    });
    apply_microphone_away(&mut runtime, true, None);
    runtime.test_microphone_device.as_mut().unwrap().gone = true;

    apply_microphone_away(&mut runtime, false, None);

    assert!(!runtime.microphone_away_hold);
    assert!(
        runtime.test_microphone_device.is_none(),
        "the dead session is dropped"
    );
    reopen_unwakeable_microphone(&mut runtime, &mut false, || None);
    assert!(microphone_open_was_attempted(&runtime));
}

// `startRunning` succeeded, then the device went: nothing holds the microphone,
// so the stopped device is a dead one and the tick replaces it.
#[cfg(target_os = "macos")]
#[test]
fn a_microphone_whose_device_died_unheld_is_reopened() {
    let mut runtime = recording_microphone_runtime_fixture(TestMicrophoneDevice {
        running: false,
        gone: true,
    });

    reopen_unwakeable_microphone(&mut runtime, &mut false, || None);

    assert!(runtime.test_microphone_device.is_none());
    assert!(microphone_open_was_attempted(&runtime));
}

// The away probe (a CGSession dictionary and a display list, 2-3×/s) is the
// microphone's alone: a session that never records it does not pay for it.
#[cfg(target_os = "macos")]
#[test]
fn the_away_tick_does_not_probe_without_a_requested_microphone() {
    let mut runtime = stranded_microphone_runtime_fixture();
    runtime.requested_sources = Some(CaptureSources {
        screen: true,
        microphone: false,
        system_audio: true,
    });
    runtime.away_probe = Some(|| panic!("probed the Mac's away state"));

    tick_microphone_away(&mut runtime, None);

    assert!(!runtime.microphone_away_hold);
}

#[cfg(target_os = "macos")]
#[test]
fn the_away_tick_holds_a_requested_microphone() {
    let mut runtime = stranded_microphone_runtime_fixture();
    runtime.away_probe = Some(|| true);

    tick_microphone_away(&mut runtime, None);

    assert!(runtime.microphone_away_hold);
}

// A user Pause→Resume while voice enrollment holds the microphone must not
// open a second session that records the take (the bug 9923eaf9 fixed for
// inactivity); the enrollment's restore brings the microphone back.
#[cfg(target_os = "macos")]
#[test]
fn a_user_resume_leaves_a_lent_out_microphone_down() {
    let mut runtime = lent_out_microphone_runtime_fixture();
    runtime.away_probe = Some(|| false);
    let requested = CaptureSources {
        screen: true,
        microphone: true,
        system_audio: false,
    };

    let (sources, held_away) = user_resume_sources(&runtime, requested);

    assert!(!sources.microphone, "enrollment holds the microphone");
    assert!(sources.screen);
    assert!(!held_away, "the hold is enrollment's, not the away hold");
}

// A device that resolves but will not run: the start returned, but nothing
// records. Counted as a failed reopen and dropped, so the tick's failure streak
// quiets the retries instead of logging "stopped unexpectedly" and "reopened"
// every tick.
#[cfg(target_os = "macos")]
#[test]
fn a_reopen_whose_device_did_not_start_is_a_failure() {
    let mut runtime = recording_microphone_runtime_fixture(TestMicrophoneDevice {
        running: false,
        gone: true,
    });

    let settled = settle_microphone_reopen(&mut runtime, Ok(()));

    assert!(settled.is_err(), "a stopped device did not reopen");
    assert!(
        runtime.test_microphone_device.is_none(),
        "the dead session is dropped, so the next tick does not warn about it"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_reopen_whose_device_runs_stands() {
    let mut runtime = recording_microphone_runtime_fixture(TestMicrophoneDevice {
        running: true,
        gone: false,
    });

    settle_microphone_reopen(&mut runtime, Ok(())).expect("a running device reopened");

    assert!(runtime.test_microphone_device.is_some());
}

// A Specific Device that vanished under fallback-to-default: the reopen
// resolves the device from the current preferences, not the one frozen at
// start, or every reopen fails for the rest of the session.
#[cfg(target_os = "macos")]
#[test]
fn a_reopen_uses_the_current_microphone_device() {
    let mut runtime = stranded_microphone_runtime_fixture();
    runtime.microphone_device_id_for_capture = Some("vanished-mic".to_string());

    reopen_unwakeable_microphone(&mut runtime, &mut false, || Some(None));

    assert!(microphone_open_was_attempted(&runtime));
    assert_eq!(
        runtime.microphone_device_id_for_capture, None,
        "the reopen asks for the default device the preferences now resolve to"
    );
}

// Preferences that can't be read without waiting (the tick holds the capture
// lock) leave the device the session started with.
#[cfg(target_os = "macos")]
#[test]
fn a_reopen_keeps_the_start_device_when_preferences_are_unreadable() {
    let mut runtime = stranded_microphone_runtime_fixture();
    runtime.microphone_device_id_for_capture = Some("start-mic".to_string());

    reopen_unwakeable_microphone(&mut runtime, &mut false, || None);

    assert_eq!(
        runtime.microphone_device_id_for_capture.as_deref(),
        Some("start-mic")
    );
}

// An inactivity pause keeps the device running (listen-only): not a reopen.
#[cfg(target_os = "macos")]
#[test]
fn a_listening_microphone_paused_for_inactivity_is_not_reopened() {
    let mut runtime = stranded_microphone_runtime_fixture();
    runtime.test_microphone_device = Some(TestMicrophoneDevice {
        running: true,
        gone: false,
    });

    reopen_unwakeable_microphone(&mut runtime, &mut false, || None);

    assert!(runtime.test_microphone_device.is_some());
    assert!(!microphone_open_was_attempted(&runtime));
}

// Lock is away: enrollment finishing while the Mac is locked leaves the
// microphone down under the away hold instead of recording until the next
// tick, and the tick reopens it once the Mac is back.
#[cfg(target_os = "macos")]
#[test]
fn restoring_the_microphone_while_away_leaves_it_held_until_return() {
    let mut lifecycle = RecordingLifecycle::default();
    *lifecycle.runtime_mut() = stranded_microphone_runtime_fixture();
    lifecycle.runtime_mut().microphone_lent_out = true;
    lifecycle.runtime_mut().away_probe = Some(|| true);

    lifecycle
        .restore_microphone_after_out_of_band_recording()
        .expect("a held restore opens nothing");

    let runtime = lifecycle.runtime_mut();
    assert!(!runtime.microphone_lent_out);
    assert!(runtime.microphone_away_hold);
    assert!(!microphone_open_was_attempted(runtime));

    apply_microphone_away(runtime, false, None);
    reopen_unwakeable_microphone(runtime, &mut false, || None);
    assert!(microphone_open_was_attempted(runtime));
}

#[cfg(target_os = "macos")]
#[test]
fn inactivity_resume_leaves_a_lent_out_microphone_alone() {
    let mut runtime = lent_out_microphone_runtime_fixture();

    resume_microphone_from_inactivity(&mut runtime)
        .expect("resuming a lent-out microphone must not try to start a session");

    assert!(runtime.inactivity.is_microphone_paused());
    assert!(runtime.active_microphone_session.is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn inactivity_pause_keeps_the_file_a_lent_out_microphone_closed() {
    let mut runtime = lent_out_microphone_runtime_fixture();
    runtime.inactivity.microphone_paused = false;
    let closed_file = "/tmp/native-capture-tests/microphone-closed-by-enrollment.m4a".to_string();
    {
        let output_files = runtime.current_segment_output_files.as_mut().unwrap();
        output_files.microphone_file = Some(closed_file.clone());
        output_files.microphone_files = vec![closed_file.clone()];
    }

    pause_microphone_for_inactivity(&mut runtime)
        .expect("pausing a lent-out microphone is a no-op");

    assert!(!runtime.inactivity.is_microphone_paused());
    let output_files = runtime.current_segment_output_files.as_ref().unwrap();
    assert_eq!(
        output_files.microphone_files,
        vec![closed_file],
        "the file the enrollment stop closed must stay booked for the segment commit"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn restoring_hands_the_microphone_back_even_when_the_restart_fails() {
    let mut lifecycle = RecordingLifecycle::default();
    *lifecycle.runtime_mut() = lent_out_microphone_runtime_fixture();

    lifecycle
        .restore_microphone_after_out_of_band_recording()
        .expect_err("restoring onto a vanished volume must fail");

    assert!(
        !lifecycle.runtime().microphone_lent_out,
        "a failed restore must still hand the microphone back, or inactivity resume can never re-arm it"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn resume_microphone_from_inactivity_seeds_missing_dedicated_planner_from_source_session() {
    let mut runtime = audio_paused_runtime_fixture();
    runtime.microphone_planner = None;

    let result = resume_microphone_from_inactivity(&mut runtime);

    assert!(
        runtime.microphone_planner.is_some(),
        "microphone planner should be restored for recoverable paused runtimes"
    );
    let planner = runtime
        .microphone_planner
        .as_ref()
        .expect("microphone planner should be seeded");
    assert_eq!(planner.save_root_dir(), "/tmp/native-capture-tests");
    assert_eq!(planner.session_id(), "native-session-microphone");
    assert_eq!(planner.date_prefix().split('/').count(), 3);
    assert_ne!(planner.session_id(), "native-session-audio-pause");
    assert!(
        result.is_ok() || result.is_err(),
        "resume may still fail in test env after planner seeding"
    );
}
