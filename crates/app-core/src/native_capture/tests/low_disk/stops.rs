use super::*;

/// A running screen+mic+system-audio runtime with a live mic recording file and
/// a committed-outputs slot, used to drive the graceful-stop transition.
fn stoppable_low_disk_runtime() -> NativeCaptureRuntime {
    let root = std::env::temp_dir().to_string_lossy().into_owned();
    NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_index: 2,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        effective_screen_bitrate_bps: Some(SCREEN_BITRATE_BPS),
        output_files: Some(crate::native_capture::segments::empty_output_files()),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(
            SEGMENT_SECONDS,
        ))),
        segment_planner: Some(SegmentPlanner::with_date_prefix(
            root,
            "native-session-low-disk-stop",
            "2026/06/25",
        )),
        free_space_probe: Some(scripted_probe),
        ..Default::default()
    }
}

#[test]
fn graceful_stop_below_critical_floor_ends_session() {
    // The graceful-stop helper (with no app handle, so the notification path is
    // a no-op) must end the session: runtime no longer running, suspension
    // cleared, state Failed.
    let mut runtime = stoppable_low_disk_runtime();
    runtime.capture_suspension = Some(CaptureSuspension::with_kind(
        CaptureSuspensionKind::LowDisk,
        &CaptureErrorResponse {
            code: "capture_low_disk".to_string(),
            message: "low disk".to_string(),
        },
    ));

    crate::native_capture::segments::graceful_stop_for_low_disk(
        None,
        &mut runtime,
        disk_space::RESERVE_FLOOR_BYTES - 1,
        false,
    );

    assert!(
        !runtime.is_running,
        "a below-critical graceful stop must end the session"
    );
    assert!(
        runtime.capture_suspension.is_none(),
        "graceful stop must clear the suspension slot"
    );
}

#[test]
fn boundary_below_critical_floor_drives_graceful_stop() {
    // A scripted probe yielding free < CRITICAL_FLOOR_BYTES must drive the
    // boundary path to a graceful stop (Stopped outcome + session ended),
    // rather than a suspension. No app handle is available in a unit test, so
    // this asserts the runtime-state transition (the notification path is
    // covered by push_error_app_notification existing; it no-ops here).
    //
    // The boundary entry requires a tauri::AppHandle, so exercise the decision
    // + graceful-stop seam it composes directly: a Critical classification at
    // the boundary leads to graceful_stop_for_low_disk(commit=true).
    let mut runtime = stoppable_low_disk_runtime();
    script_free_bytes(&[disk_space::RESERVE_FLOOR_BYTES - 1]);

    let decision = crate::native_capture::segments::low_disk_decision_at_boundary(&runtime)
        .expect("a measurable below-floor reading must classify");
    assert_eq!(
        decision,
        disk_space::LowDiskDecision::Critical,
        "free below the reserve floor must classify Critical at the boundary"
    );

    // The Critical boundary arm performs a graceful stop. Drive that arm's
    // effect (no app handle -> notification no-op) and assert the session ends.
    crate::native_capture::segments::graceful_stop_for_low_disk(
        None,
        &mut runtime,
        disk_space::RESERVE_FLOOR_BYTES - 1,
        true,
    );
    assert!(
        !runtime.is_running,
        "the Critical boundary path must stop the session"
    );
}

// --- Commit the current segment's mic audio on a low-disk suspend (b) ---

#[test]
fn low_disk_boundary_suspend_commits_current_segment_microphone() {
    // ADR 0040 ("the current segment is still healthy ... so commit it before
    // ending"): at a healthy rotation boundary a low-disk Pause must NOT
    // silently drop the current segment's already-finalized microphone audio.
    // The mic `.m4a` lives in current_segment_output_files (set at segment
    // start) and is a valid closed file on disk; the suspend entry must commit
    // it into runtime.output_files just as a normal rotation / inactivity
    // pause would, else up to ~5 minutes of microphone audio is orphaned (no
    // audio_segment row) on every low-disk pause.
    //
    // Mic-only session so the screen/system-audio commit path is a no-op and
    // the assertion isolates the microphone commit.
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let mic_path = temp.path().join("segment-0001-mic.m4a");
    write_valid_m4a_audio_file(&mic_path);
    let mic = mic_path.to_string_lossy().into_owned();

    let mut current_segment_output_files = crate::native_capture::segments::empty_output_files();
    set_current_microphone_output_file(&mut current_segment_output_files, mic.clone());

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: false,
            microphone: true,
            system_audio: false,
        }),
        current_segment_index: 1,
        microphone_recording_file: Some(mic.clone()),
        current_segment_output_files: Some(current_segment_output_files),
        output_files: Some(crate::native_capture::segments::empty_output_files()),
        ..Default::default()
    };

    crate::native_capture::segments::suspend_screen_capture(
        None,
        &mut runtime,
        &CaptureErrorResponse {
            code: "capture_low_disk".to_string(),
            message: "low disk".to_string(),
        },
        CaptureSuspensionKind::LowDisk,
    )
    .expect("low-disk suspend should succeed with no live screen/mic session");

    let committed = runtime
        .output_files
        .as_ref()
        .expect("committed output_files slot should exist");
    let committed_mic = committed
        .microphone_file
        .iter()
        .chain(committed.microphone_files.iter())
        .any(|file| file == &mic);
    assert!(
        committed_mic,
        "low-disk boundary suspend must commit the current segment's finalized \
         microphone audio into runtime.output_files (got {committed:?}); otherwise \
         up to 5 minutes of mic audio is silently lost on every low-disk pause"
    );
}

#[test]
fn low_disk_suspend_commits_the_current_segment_system_audio() {
    // The system-audio sibling of the microphone test above: LowDisk is the
    // one suspension kind that stops the tap (ADR 0040/0052), so the suspend
    // entry must pass `system_audio_stops_with_suspension` = true into
    // `commit_suspended_screen_system_outputs`, committing the tap's
    // finalized `.m4a` instead of stripping it as a still-live file — and it
    // must leave no live continuation file behind (the tap is down).
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let sys_path = temp.path().join("segment-0001-sys.m4a");
    write_valid_m4a_audio_file(&sys_path);
    let sys = sys_path.to_string_lossy().into_owned();

    let mut current_segment_output_files = crate::native_capture::segments::empty_output_files();
    crate::native_capture_output::set_current_system_audio_output_file(
        &mut current_segment_output_files,
        sys.clone(),
    );

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: false,
            microphone: false,
            system_audio: true,
        }),
        current_segment_index: 1,
        system_audio_recording_file: Some(sys.clone()),
        current_segment_output_files: Some(current_segment_output_files),
        output_files: Some(crate::native_capture::segments::empty_output_files()),
        ..Default::default()
    };

    crate::native_capture::segments::suspend_screen_capture(
        None,
        &mut runtime,
        &CaptureErrorResponse {
            code: "capture_low_disk".to_string(),
            message: "low disk".to_string(),
        },
        CaptureSuspensionKind::LowDisk,
    )
    .expect("low-disk suspend should succeed with no live screen/mic session");

    let committed = runtime
        .output_files
        .as_ref()
        .expect("committed output_files slot should exist");
    let committed_sys = committed
        .system_audio_file
        .iter()
        .chain(committed.system_audio_files.iter())
        .any(|file| file == &sys);
    assert!(
        committed_sys,
        "low-disk suspend must commit the current segment's finalized \
         system-audio `.m4a` into runtime.output_files (got {committed:?}); \
         LowDisk stops the tap, so skipping the commit orphans the segment"
    );
    assert!(
        runtime.system_audio_recording_file.is_none(),
        "LowDisk stops the tap — no live system-audio continuation file may \
         survive the suspend"
    );
}

// --- Graceful stop commit gating (d) ---
//
// Closes the proxy-test trap the existing stop tests fall into (they assert
// only `!is_running`, so a regression that silently dropped the committed
// outputs would still pass). These drive a system-audio-only segment — a real,
// AVFoundation-valid `.m4a` — so the commit runs through `finalize_capture_outputs`
// without needing a live native session or a valid screen `.mov`.
//
// NB: the *screen-session-ordering* half of the graceful-stop fix (stop the
// ScreenCaptureKit session before reading its `.mov`) cannot be unit-tested:
// `capture-screen` exposes no constructor for a live `ActiveCaptureSession`.

/// A running system-audio-only runtime whose current segment carries a real,
/// committable `.m4a` at `sys_path`, used to drive the graceful-stop commit
/// gating. No live native session is needed: the system-audio commit path only
/// reads/validates the file.
fn system_audio_segment_runtime(sys_path: &str, segment_index: u64) -> NativeCaptureRuntime {
    let mut current = crate::native_capture::segments::empty_output_files();
    crate::native_capture_output::set_current_system_audio_output_file(
        &mut current,
        sys_path.to_string(),
    );
    NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: false,
            microphone: false,
            system_audio: true,
        }),
        current_segment_index: segment_index,
        system_audio_recording_file: Some(sys_path.to_string()),
        current_segment_output_files: Some(current),
        output_files: Some(crate::native_capture::segments::empty_output_files()),
        ..Default::default()
    }
}

#[test]
fn graceful_stop_commit_true_appends_current_segment_outputs() {
    // A boundary/recovery graceful stop (commit_current_segment = true) must
    // actually COMMIT the healthy current segment into runtime.output_files,
    // not merely end the session (ADR 0040 "clean-finalize what is safely
    // closable"). Asserting only `!is_running` would pass even if the commit
    // silently dropped every byte.
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let sys_path = temp.path().join("segment-0002-sys.m4a");
    write_valid_m4a_audio_file(&sys_path);
    let sys = sys_path.to_string_lossy().into_owned();

    let mut runtime = system_audio_segment_runtime(&sys, 2);
    crate::native_capture::segments::graceful_stop_for_low_disk(
        None,
        &mut runtime,
        disk_space::RESERVE_FLOOR_BYTES - 1,
        true,
    );

    let committed = runtime
        .output_files
        .as_ref()
        .expect("committed output_files slot should exist");
    let committed_sys = committed
        .system_audio_file
        .iter()
        .chain(committed.system_audio_files.iter())
        .any(|file| file == &sys);
    assert!(
        committed_sys,
        "commit_current_segment=true graceful stop must append the healthy \
         current segment's system-audio output (got {committed:?})"
    );
    assert!(!runtime.is_running, "graceful stop must end the session");
}

#[test]
fn graceful_stop_commit_false_drops_current_segment_outputs() {
    // The mid-segment-fill caller passes commit_current_segment = false because
    // it already discarded its partial; graceful stop must then commit NOTHING,
    // so no half-written file is recorded. This pins the `commit` flag actually
    // gating the commit (not a vacuous parameter).
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let sys_path = temp.path().join("segment-0003-sys.m4a");
    write_valid_m4a_audio_file(&sys_path);
    let sys = sys_path.to_string_lossy().into_owned();

    let mut runtime = system_audio_segment_runtime(&sys, 3);
    crate::native_capture::segments::graceful_stop_for_low_disk(
        None,
        &mut runtime,
        disk_space::RESERVE_FLOOR_BYTES - 1,
        false,
    );

    let committed = runtime
        .output_files
        .as_ref()
        .expect("committed output_files slot should exist");
    assert!(
        committed.system_audio_file.is_none() && committed.system_audio_files.is_empty(),
        "commit_current_segment=false graceful stop must NOT append outputs \
         (the partial was already discarded), got {committed:?}"
    );
    assert!(!runtime.is_running, "graceful stop must end the session");
}

// --- Mid-segment write-failure classification (c) ---
//
// `classify_write_failure_disk_full` is the guard that decides whether a writer
// append/finalize failure coincides with low disk. It is pure over the injected
// probe, so it is testable end to end (unlike `handle_mid_segment_write_failure_*`,
// which needs a `&tauri::AppHandle` for the notification path and is harness-blocked).

#[test]
fn classify_write_failure_disk_full_passes_through_on_healthy_disk() {
    // A writer failure on a HEALTHY disk must return None so the existing fatal-
    // failure behavior is preserved unchanged. If this inverted, every unrelated
    // write failure would be silently swallowed into a low-disk suspension.
    let estimate = fixture_estimate();
    let resume = disk_space::resume_threshold_bytes(estimate);
    let runtime = low_disk_runtime_fixture();

    script_free_bytes(&[resume + 1]);
    assert_eq!(
        crate::native_capture::segments::classify_write_failure_disk_full(&runtime),
        None,
        "a write failure on a healthy disk must not be treated as disk-full"
    );

    // Unmeasurable -> None too (best-effort: never force a disk-full stop on a
    // volume we cannot stat).
    let mut unmeasurable = low_disk_runtime_fixture();
    unmeasurable.free_space_probe = Some(erroring_probe);
    assert_eq!(
        crate::native_capture::segments::classify_write_failure_disk_full(&unmeasurable),
        None,
        "an unmeasurable probe must not force a disk-full classification"
    );
}

#[test]
fn classify_write_failure_disk_full_detects_pause_and_critical() {
    let estimate = fixture_estimate();
    let pause = disk_space::pause_threshold_bytes(estimate);
    let runtime = low_disk_runtime_fixture();

    // Below the pause threshold but at/above the reserve floor -> Pause
    // (discard the partial and suspend).
    script_free_bytes(&[pause - 1]);
    let pause_decision =
        crate::native_capture::segments::classify_write_failure_disk_full(&runtime);
    assert!(
        matches!(
            pause_decision,
            Some((disk_space::LowDiskDecision::Pause, _))
        ),
        "free below pause but above the floor must classify the failure as Pause \
         (got {pause_decision:?})"
    );

    // Below the reserve floor -> Critical (discard the partial and stop).
    script_free_bytes(&[disk_space::RESERVE_FLOOR_BYTES - 1]);
    let critical_decision =
        crate::native_capture::segments::classify_write_failure_disk_full(&runtime);
    assert!(
        matches!(
            critical_decision,
            Some((disk_space::LowDiskDecision::Critical, _))
        ),
        "free below the reserve floor must classify the failure as Critical \
         (got {critical_decision:?})"
    );
}
