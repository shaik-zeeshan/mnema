use super::*;

#[test]
fn privacy_failure_without_microphone_commits_current_screen_output_before_runtime_failure() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let screen_path = temp_dir.path().join("screen-segment.mov");
    std::fs::write(&screen_path, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
        .expect("fake openable mov should be written");
    let screen_path = screen_path.to_string_lossy().into_owned();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
        output_files: Some(empty_output_files()),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some(screen_path.clone()),
            screen_files: vec![screen_path.clone()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: Some(screen_path.clone()),
        ..Default::default()
    };
    let error = CaptureErrorResponse {
        code: "privacy_update_failed".to_string(),
        message: "privacy update failed".to_string(),
    };

    suspend_screen_capture(
        None,
        &mut runtime,
        &error,
        CaptureSuspensionKind::PrivacyFilter,
    )
    .expect("privacy suspension should succeed");
    assert!(
        runtime.current_segment_output_files.is_none(),
        "without microphone continuation, suspended screen/system outputs should already be committed and detached"
    );
    mark_runtime_session_failed(&mut runtime);

    assert!(!runtime.is_running);
    assert!(runtime.current_segment_output_files.is_none());
    let output_files = runtime
        .output_files
        .expect("committed output files should be preserved after runtime failure");
    assert_eq!(
        output_files.screen_file.as_deref(),
        Some(screen_path.as_str())
    );
    assert_eq!(output_files.screen_files, vec![screen_path]);
}

#[test]
fn display_unavailable_suspension_commits_tail_segment_and_stays_running() {
    // A display-unavailable suspension used to skip committing the in-flight
    // segment because the dead stream left it truncated. The stop path now
    // finalizes the writers even for a delegate-terminated stream, so the
    // tail `.mov` is openable — the suspend path must commit it (not orphan
    // it) and must not fail the session, so a screen-only recording resumes
    // automatically when the display returns (ADR 0021 amendment).
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let screen_path = temp_dir.path().join("screen-segment.mov");
    std::fs::write(&screen_path, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
        .expect("fake openable mov should be written");
    let screen_path = screen_path.to_string_lossy().into_owned();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
        output_files: Some(empty_output_files()),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some(screen_path.clone()),
            screen_files: vec![screen_path.clone()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: Some(screen_path.clone()),
        ..Default::default()
    };
    let error = CaptureErrorResponse {
        code: "privacy_filter_display_unavailable".to_string(),
        message: "no display".to_string(),
    };

    suspend_screen_capture(
        None,
        &mut runtime,
        &error,
        CaptureSuspensionKind::DisplayUnavailable,
    )
    .expect("display-unavailable suspension should succeed");

    // The session is kept alive for recovery rather than failed.
    assert!(runtime.is_running);
    assert_eq!(
        runtime
            .capture_suspension
            .as_ref()
            .map(|suspension| suspension.kind),
        Some(CaptureSuspensionKind::DisplayUnavailable)
    );
    // The finalized in-flight tail segment is committed, not orphaned.
    assert!(runtime.current_segment_output_files.is_none());
    assert!(runtime.recording_file.is_none());
    let output_files = runtime
        .output_files
        .expect("output files collection should be preserved");
    assert!(
        output_files.screen_files.contains(&screen_path),
        "the finalized in-flight tail segment must be committed"
    );
}

/// Build a running screen-only runtime whose in-flight segment points at a
/// fake but openable `.mov`, mirroring the display-unavailable fixture.
fn running_screen_runtime_with_tail(screen_path: &str) -> NativeCaptureRuntime {
    NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
        output_files: Some(empty_output_files()),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some(screen_path.to_string()),
            screen_files: vec![screen_path.to_string()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: Some(screen_path.to_string()),
        ..Default::default()
    }
}

#[test]
fn license_block_boundary_stop_commits_current_segment_and_ends_session() {
    // The rotation-boundary trial/revoked stop has a HEALTHY current segment
    // (only opening the *next* file is refused), so it must commit the tail —
    // not orphan a user's final recording — and end the session. Both the
    // trial-lapse and revoked variants pass `commit = true`; assert the
    // mechanics are identical. `None` core skips the dialog/notification.
    for revoked in [false, true] {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let screen_path = temp_dir.path().join("screen-segment.mov");
        std::fs::write(&screen_path, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
            .expect("fake openable mov should be written");
        let screen_path = screen_path.to_string_lossy().into_owned();

        let mut runtime = running_screen_runtime_with_tail(&screen_path);
        graceful_stop_for_license_block(None, &mut runtime, revoked);

        assert!(!runtime.is_running, "session must end (revoked={revoked})");
        assert!(
            runtime.current_segment_output_files.is_none(),
            "in-flight tail must be committed and detached (revoked={revoked})"
        );
        let output_files = runtime
            .output_files
            .expect("committed output files should be preserved");
        assert!(
            output_files.screen_files.contains(&screen_path),
            "the healthy current segment must be committed, not orphaned (revoked={revoked})"
        );
    }
}

#[test]
fn boundary_stop_without_commit_discards_the_partial_segment() {
    // The mid-fill low-disk caller passes `commit = false`: a half-written
    // partial must NOT be committed. Locks the shared-fn contract the trial/
    // revoked stop now depends on.
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let screen_path = temp_dir.path().join("partial-segment.mov");
    std::fs::write(&screen_path, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
        .expect("fake mov should be written");
    let screen_path = screen_path.to_string_lossy().into_owned();

    let mut runtime = running_screen_runtime_with_tail(&screen_path);
    graceful_stop_at_boundary(None, &mut runtime, false);

    assert!(!runtime.is_running);
    let output_files = runtime
        .output_files
        .expect("output files collection should be preserved");
    assert!(
        !output_files.screen_files.contains(&screen_path),
        "a non-committed partial must never be added to the committed set"
    );
}

// Every source paused or suspended: the subject here is the stale-output
// fallback in `current_segment_sources_for_runtime`, so system audio is paused
// in the fixture — a screen suspension alone no longer pauses it (ADR 0052).
#[test]
fn privacy_failure_with_paused_audio_keeps_suspended_sources_explicit() {
    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
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
        inactivity: crate::native_capture_inactivity::InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            microphone_paused: true,
            system_audio_paused: true,
            is_paused: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let error = CaptureErrorResponse {
        code: "privacy_update_failed".to_string(),
        message: "privacy update failed".to_string(),
    };

    suspend_screen_capture(
        None,
        &mut runtime,
        &error,
        CaptureSuspensionKind::PrivacyFilter,
    )
    .expect("privacy suspension should succeed");

    assert_eq!(
        runtime.current_segment_sources,
        Some(CaptureSources {
            screen: false,
            microphone: false,
            system_audio: false,
        })
    );
    assert!(
        crate::native_capture::runtime::current_segment_sources_for_runtime(&runtime)
            .is_none(),
        "explicit all-paused privacy suspension must not fall back to stale screen/system-audio outputs"
    );
}

// ADR 0021 (amended): a display sleeping, locking or disconnecting is a
// screen-only condition. System audio runs on a process tap with no display
// dependency, so it records straight through it — exactly as the microphone
// does. Before the swap, this suspension took system audio down with the
// screen and the recording lost audio for the whole sleep.
#[test]
fn display_unavailable_suspension_keeps_system_audio_recording() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let screen_path = temp_dir.path().join("screen-segment.mov");
    std::fs::write(&screen_path, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
        .expect("fake openable mov should be written");
    let screen_path = screen_path.to_string_lossy().into_owned();
    let system_audio_path = "/tmp/live-system-audio.m4a".to_string();
    let sources = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: true,
    };

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(sources.clone()),
        current_segment_sources: Some(sources),
        output_files: Some(empty_output_files()),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some(screen_path.clone()),
            screen_files: vec![screen_path.clone()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: Some(system_audio_path.clone()),
            system_audio_files: vec![system_audio_path.clone()],
        }),
        recording_file: Some(screen_path.clone()),
        system_audio_recording_file: Some(system_audio_path.clone()),
        ..Default::default()
    };

    suspend_screen_capture(
        None,
        &mut runtime,
        &CaptureErrorResponse {
            code: "privacy_filter_display_unavailable".to_string(),
            message: "Failed to find any displays or windows to capture (code: -3815)".to_string(),
        },
        CaptureSuspensionKind::DisplayUnavailable,
    )
    .expect("display-unavailable suspension should succeed");

    assert!(runtime.recording_file.is_none(), "the screen is suspended");
    assert_eq!(
        runtime.system_audio_recording_file.as_deref(),
        Some(system_audio_path.as_str()),
        "the tap keeps writing the same file through a display sleep"
    );
    assert_eq!(
        runtime.current_segment_sources,
        Some(CaptureSources {
            screen: false,
            microphone: false,
            system_audio: true,
        }),
        "system audio alone keeps the session recording with no display"
    );

    // The tap's in-flight file must stay tracked, or nothing ever commits it.
    let current_outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("the system-audio continuation must remain current");
    assert!(current_outputs.screen_file.is_none());
    assert_eq!(
        current_outputs.system_audio_file.as_deref(),
        Some(system_audio_path.as_str())
    );

    // ...and it must NOT have been committed: it is still being written.
    let committed = runtime
        .output_files
        .as_ref()
        .expect("committed outputs should exist");
    assert!(
        committed.system_audio_files.is_empty(),
        "committing a file the tap is still writing would truncate the segment"
    );
    assert!(
        committed.screen_files.contains(&screen_path),
        "the screen's tail segment is committed, since the screen did stop"
    );
}

// Low disk is not about the screen: every source writes to the same volume,
// so it is the one suspension that stops the tap too (ADR 0040).
#[test]
fn low_disk_suspension_stops_system_audio_with_everything_else() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let screen_path = temp_dir.path().join("screen-segment.mov");
    std::fs::write(&screen_path, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
        .expect("fake openable mov should be written");
    let screen_path = screen_path.to_string_lossy().into_owned();
    let sources = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: true,
    };

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(sources.clone()),
        current_segment_sources: Some(sources),
        output_files: Some(empty_output_files()),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some(screen_path.clone()),
            screen_files: vec![screen_path.clone()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: Some("/tmp/low-disk-system-audio.m4a".to_string()),
            system_audio_files: vec!["/tmp/low-disk-system-audio.m4a".to_string()],
        }),
        recording_file: Some(screen_path.clone()),
        system_audio_recording_file: Some("/tmp/low-disk-system-audio.m4a".to_string()),
        ..Default::default()
    };

    suspend_screen_capture(
        None,
        &mut runtime,
        &CaptureErrorResponse {
            code: "capture_low_disk".to_string(),
            message: "recordings volume low on free space".to_string(),
        },
        CaptureSuspensionKind::LowDisk,
    )
    .expect("low-disk suspension should succeed");

    assert!(runtime.system_audio_recording_file.is_none());
    assert_eq!(
        runtime.current_segment_sources,
        Some(CaptureSources {
            screen: false,
            microphone: false,
            system_audio: false,
        }),
        "low disk stops the tap along with the screen"
    );
}

#[test]
fn privacy_failure_with_active_microphone_detaches_suspended_screen_state() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let screen_path = temp_dir.path().join("screen-segment.mov");
    std::fs::write(&screen_path, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
        .expect("fake openable mov should be written");
    let screen_path = screen_path.to_string_lossy().into_owned();
    let microphone_path = temp_dir
        .path()
        .join("microphone.m4a")
        .to_string_lossy()
        .into_owned();

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
        output_files: Some(empty_output_files()),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some(screen_path.clone()),
            screen_files: vec![screen_path.clone()],
            microphone_file: Some(microphone_path.clone()),
            microphone_files: vec![microphone_path.clone()],
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: Some(screen_path.clone()),
        microphone_recording_file: Some(microphone_path.clone()),
        system_audio_recording_file: Some("/tmp/stale-system-audio.m4a".to_string()),
        ..Default::default()
    };
    let error = CaptureErrorResponse {
        code: "privacy_update_failed".to_string(),
        message: "privacy update failed".to_string(),
    };

    suspend_screen_capture(
        None,
        &mut runtime,
        &error,
        CaptureSuspensionKind::PrivacyFilter,
    )
    .expect("privacy suspension should succeed");

    assert!(runtime.recording_file.is_none());
    assert!(runtime.system_audio_recording_file.is_none());
    assert_eq!(
        runtime.current_segment_sources,
        Some(CaptureSources {
            screen: false,
            microphone: true,
            system_audio: false,
        })
    );

    let current_outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("microphone continuation should remain current");
    assert!(current_outputs.screen_file.is_none());
    assert!(current_outputs.screen_files.is_empty());
    assert_eq!(
        current_outputs.microphone_file.as_deref(),
        Some(microphone_path.as_str())
    );
    assert_eq!(current_outputs.microphone_files, vec![microphone_path]);

    let committed = runtime
        .output_files
        .as_ref()
        .expect("suspended screen output should be committed");
    assert_eq!(committed.screen_file.as_deref(), Some(screen_path.as_str()));
    assert_eq!(committed.screen_files, vec![screen_path]);
}
