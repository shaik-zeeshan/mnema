use super::*;
use crate::native_capture::runtime::CaptureSuspension;
use capture_types::CaptureOutputFiles;

fn screen_only_sources() -> CaptureSources {
    CaptureSources {
        screen: true,
        microphone: false,
        system_audio: false,
    }
}

fn system_audio_only_sources() -> CaptureSources {
    CaptureSources {
        screen: false,
        microphone: false,
        system_audio: true,
    }
}

/// A running system-audio-only session whose tap failed to start: requested,
/// unpaused, and no session — the shape that used to report itself healthy
/// while recording nothing at all.
fn tapless_system_audio_runtime() -> NativeCaptureRuntime {
    NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(system_audio_only_sources()),
        active_system_audio_session: None,
        ..Default::default()
    }
}

// Regression: `start_system_audio_family_for_runtime` logs "continuing without
// it" on Err while `requested_sources.system_audio` stays true, and nothing
// retried — the rotation arm needs a session, the tick early-returned without
// one, and the zero-watchdog lives inside the session that was never created.
// So a transient start failure (coreaudiod restarting, an aggregate-UID
// collision) bought a whole session that recorded no system audio while the UI
// showed a healthy recording — and under ADR 0052's audio-only sessions, a
// recording that records literally nothing.
#[test]
fn a_tapless_session_that_still_wants_system_audio_is_retried() {
    assert!(should_retry_system_audio_start(
        &tapless_system_audio_runtime()
    ));
}

#[test]
fn a_low_disk_suspension_is_never_retried_back_into_a_tap() {
    let mut runtime = tapless_system_audio_runtime();
    runtime.capture_suspension = Some(CaptureSuspension::with_kind(
        CaptureSuspensionKind::LowDisk,
        &CaptureErrorResponse {
            code: "capture_low_disk".to_string(),
            message: "low disk".to_string(),
        },
    ));

    assert!(
        !should_retry_system_audio_start(&runtime),
        "low disk is the one suspension that stops the tap on purpose, and it restarts it itself (ADR 0040)"
    );
}

// A tap-less family paused for inactivity could never resume: the resume is
// "sound detected", and only a live tap hears sound. So the retry brings it up
// paused (listening, writing nothing) rather than leaving it stuck.
#[test]
fn a_paused_tapless_family_is_retried_into_a_listening_tap() {
    let mut paused = tapless_system_audio_runtime();
    paused
        .inactivity
        .set_family_paused_states(false, false, true);
    assert!(should_retry_system_audio_start(&paused));
}

#[test]
fn an_unrequested_or_stopped_system_audio_family_is_never_started_by_the_retry() {
    let mut unrequested = tapless_system_audio_runtime();
    unrequested.requested_sources = Some(screen_only_sources());
    assert!(!should_retry_system_audio_start(&unrequested));

    let mut stopped = tapless_system_audio_runtime();
    stopped.is_running = false;
    assert!(!should_retry_system_audio_start(&stopped));
}

// The tap-start retry rides the zero-watchdog's ladder: 30s, doubling, capped
// at 10 minutes. A start does synchronous Core Audio work, so an unbounded or
// un-paced retry would probe it on every tick.
#[test]
fn the_tap_start_retry_backs_off_from_thirty_seconds_to_a_ten_minute_cap() {
    let mut lifecycle = RecordingLifecycle::default();
    let mut delays = Vec::new();

    for _ in 0..8 {
        lifecycle.schedule_system_audio_start_retry(0);
        delays.push(lifecycle.system_audio_start_retry_delay_ms);
    }

    assert_eq!(
        delays,
        vec![30_000, 60_000, 120_000, 240_000, 480_000, 600_000, 600_000, 600_000]
    );
    assert_eq!(lifecycle.system_audio_start_retry_at_ms, Some(600_000));
}

// A running screen-only runtime as the delegate-stop reconcile sees it: the
// stream is dead, `active_screen_session` already `None`, with the in-flight
// segment's finalized `.mov` still tracked as the current output.
fn lifecycle_after_unexpected_screen_stop(screen_file: &str) -> RecordingLifecycle {
    RecordingLifecycle {
        runtime: NativeCaptureRuntime {
            is_running: true,
            requested_sources: Some(screen_only_sources()),
            current_segment_sources: Some(screen_only_sources()),
            output_files: Some(empty_output_files()),
            current_segment_output_files: Some(CaptureOutputFiles {
                screen_file: Some(screen_file.to_string()),
                screen_files: vec![screen_file.to_string()],
                microphone_file: None,
                microphone_files: Vec::new(),
                system_audio_file: None,
                system_audio_files: Vec::new(),
            }),
            recording_file: Some(screen_file.to_string()),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn write_openable_screen_segment(dir: &tempfile::TempDir) -> String {
    let path = dir.path().join("screen-segment.mov");
    std::fs::write(&path, b"\0\0\0\x18ftypqt  \0\0\0\x08moov")
        .expect("fake openable mov should be written");
    path.to_string_lossy().into_owned()
}

// Regression: a display sleep killed the ScreenCaptureKit stream (-3815),
// the reconcile cleared screen state without entering a suspension, and the
// next 60s rotation boundary rotated into the missing session — failing the
// entire recording session. The reconcile must instead enter a
// DisplayUnavailable suspension that keeps the session alive and commits
// the in-flight tail segment.
#[test]
fn unexpected_screen_stop_suspends_and_commits_tail_instead_of_failing() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let screen_file = write_openable_screen_segment(&temp_dir);
    let mut lifecycle = lifecycle_after_unexpected_screen_stop(&screen_file);
    let error = CaptureErrorResponse {
        code: "capture_stream_system_stopped".to_string(),
        message: "ScreenCaptureKit stream stopped unexpectedly: Failed to find any displays or windows to capture (code: -3815)".to_string(),
    };

    let outcome = lifecycle.suspend_screen_after_unexpected_stop(None, &error);

    assert_eq!(outcome, TickOutcome::SkipRotation);
    let runtime = lifecycle.runtime();
    assert!(
        runtime.is_running,
        "an unexpected stream stop must never end the recording session"
    );
    assert_eq!(
        runtime
            .capture_suspension
            .as_ref()
            .map(|suspension| suspension.kind),
        Some(CaptureSuspensionKind::DisplayUnavailable),
        "the stop must land in the DisplayUnavailable suspension so recovery owns the restart"
    );
    assert!(runtime.recording_file.is_none());
    let committed = runtime
        .output_files
        .as_ref()
        .expect("committed outputs should exist");
    assert!(
        committed.screen_files.contains(&screen_file),
        "the in-flight tail segment must be committed, not orphaned"
    );
}

// Regression backstop at the rotation seam: screen active, no live session,
// no suspension owner (e.g. a wake racing the will-sleep teardown) must
// suspend and skip — before this guard, rotation hit `invalid_runtime_state`
// and marked the whole session failed.
#[test]
fn rotation_with_missing_screen_session_suspends_instead_of_failing() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let screen_file = write_openable_screen_segment(&temp_dir);
    let mut lifecycle = lifecycle_after_unexpected_screen_stop(&screen_file);

    let outcome =
        lifecycle.suspend_if_screen_session_missing_at_rotation(None, &screen_only_sources());

    assert_eq!(outcome, Some(TickOutcome::SkipRotation));
    let runtime = lifecycle.runtime();
    assert!(runtime.is_running);
    assert_eq!(
        runtime
            .capture_suspension
            .as_ref()
            .map(|suspension| suspension.kind),
        Some(CaptureSuspensionKind::DisplayUnavailable),
    );
}

// License gate ≠ suspension (ADR 0021): a deny at either seam must refuse /
// stop outright while `capture_suspension` stays None — a license block
// never self-heals, so it must never enter the recovery machinery.

#[test]
fn license_block_at_start_refuses_with_the_gate_code_and_no_suspension() {
    let block = crate::licensing::license_block(Some(&capture_types::LicenseStatus::ReadOnly), 0);
    let refusal = RecordingLifecycle::refuse_start_for_license_block(block)
        .expect("read-only must refuse the start");
    assert_eq!(refusal.code, "capture_refused_read_only");

    let block = crate::licensing::license_block(Some(&capture_types::LicenseStatus::Revoked), 0);
    let refusal = RecordingLifecycle::refuse_start_for_license_block(block)
        .expect("revoked must refuse the start");
    assert_eq!(refusal.code, "capture_refused_revoked");
}

#[test]
fn license_block_at_start_never_refuses_on_unknown_status() {
    // The deferred gate hasn't run yet (cached status None) → allow.
    assert!(
        RecordingLifecycle::refuse_start_for_license_block(crate::licensing::license_block(
            None,
            i64::MAX
        ))
        .is_none()
    );
}

#[test]
fn license_block_at_rotation_stops_the_loop_without_a_suspension() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let screen_file = write_openable_screen_segment(&temp_dir);
    let mut lifecycle = lifecycle_after_unexpected_screen_stop(&screen_file);

    let block = crate::licensing::license_block(Some(&capture_types::LicenseStatus::ReadOnly), 0);
    let outcome = lifecycle.stop_for_license_block_at_rotation(None, block);

    assert_eq!(outcome, Some(TickOutcome::StopLoop));
    let runtime = lifecycle.runtime();
    assert!(!runtime.is_running, "the session must end at the boundary");
    assert!(
        runtime.capture_suspension.is_none(),
        "a license block is a stop, never a suspension — it cannot self-heal"
    );
    // The healthy in-flight tail segment is committed, not orphaned.
    let committed = runtime
        .output_files
        .as_ref()
        .expect("committed outputs should exist");
    assert!(committed.screen_files.contains(&screen_file));
}

#[test]
fn no_license_block_at_rotation_is_a_no_op() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let screen_file = write_openable_screen_segment(&temp_dir);
    let mut lifecycle = lifecycle_after_unexpected_screen_stop(&screen_file);

    let outcome = lifecycle
        .stop_for_license_block_at_rotation(None, crate::licensing::license_block(None, 0));

    assert_eq!(outcome, None);
    assert!(
        lifecycle.runtime().is_running,
        "an allowed status must not stop the session"
    );
}

// A system-audio-only session has no screen session, so the guard must not
// read "no screen session" as a fault: suspending here would stop a perfectly
// healthy audio-only recording at its first rotation (ADR 0052).
#[test]
fn rotation_guard_ignores_a_system_audio_only_session() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let screen_file = write_openable_screen_segment(&temp_dir);
    let mut lifecycle = lifecycle_after_unexpected_screen_stop(&screen_file);
    let sources = CaptureSources {
        screen: false,
        microphone: false,
        system_audio: true,
    };

    let outcome = lifecycle.suspend_if_screen_session_missing_at_rotation(None, &sources);

    assert_eq!(outcome, None);
    assert!(lifecycle.runtime().capture_suspension.is_none());
}

#[test]
fn rotation_guard_ignores_audio_only_sources() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let screen_file = write_openable_screen_segment(&temp_dir);
    let mut lifecycle = lifecycle_after_unexpected_screen_stop(&screen_file);
    let sources = CaptureSources {
        screen: false,
        microphone: true,
        system_audio: false,
    };

    let outcome = lifecycle.suspend_if_screen_session_missing_at_rotation(None, &sources);

    assert_eq!(outcome, None);
    assert!(lifecycle.runtime().capture_suspension.is_none());
}
