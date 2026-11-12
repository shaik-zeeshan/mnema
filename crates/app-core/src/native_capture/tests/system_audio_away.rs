// System audio stops while the Mac is away (founder ruling 2026-11-12): a running
// tap holds coreaudiod's idle-sleep assertion. Tests cannot build a real tap, so
// they cover the hold's bookkeeping and the decisions around the start.

use super::*;
use crate::native_capture::lifecycle::should_retry_system_audio_start;
use crate::native_capture::segments::{
    first_system_audio_output_file, start_system_audio_family_for_runtime,
};
use crate::native_capture::system_audio_away::{apply_system_audio_away, system_audio_tap_age_ms};

/// A system-audio-only session recording `file` in segment 3, with no tap: the
/// tap's stand-in is the file booked as the one it writes.
fn recording_system_audio_runtime(dir: &TestDir, file: Option<&str>) -> NativeCaptureRuntime {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();
    let mut outputs = crate::native_capture::segments::empty_output_files();
    if let Some(file) = file {
        outputs.system_audio_file = Some(file.to_string());
        outputs.system_audio_files = vec![file.to_string()];
    }
    NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: false,
            microphone: false,
            system_audio: true,
        }),
        current_segment_index: 3,
        output_files: Some(crate::native_capture::segments::empty_output_files()),
        current_segment_output_files: Some(outputs),
        system_audio_recording_file: file.map(str::to_string),
        system_audio_planner: Some(SegmentPlanner::new(
            dir.path().to_string_lossy().to_string(),
            "sysaudio_away",
        )),
        source_sessions: Some(independent_source_sessions_fixture()),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(60))),
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
#[test]
fn going_away_commits_the_file_the_tap_was_writing_and_holds_it_down() {
    let dir = TestDir::new("sysaudio-away-commit");
    let file = dir.path().join("sysaudio-segment-0003.m4a");
    write_valid_m4a_audio_file(&file);
    let file = file.to_string_lossy().into_owned();
    let mut runtime = recording_system_audio_runtime(&dir, Some(&file));

    assert!(!apply_system_audio_away(&mut runtime, true, None));

    assert!(runtime.system_audio_away_hold);
    assert!(runtime.active_system_audio_session.is_none());
    assert_eq!(runtime.system_audio_recording_file, None, "no live file");
    let live = runtime.current_segment_output_files.as_ref().unwrap();
    assert!(
        live.system_audio_file.is_none() && live.system_audio_files.is_empty(),
        "booked once, at the commit, never again at the segment boundary"
    );
    assert_eq!(
        runtime.output_files.as_ref().unwrap().system_audio_files,
        vec![file.clone()],
        "the finished file is committed with its segment"
    );
    assert!(Path::new(&file).is_file(), "the audio is kept on disk");

    // Later away ticks find nothing left to stop or commit.
    assert!(!apply_system_audio_away(&mut runtime, true, None));
    assert_eq!(
        runtime.output_files.as_ref().unwrap().system_audio_files,
        vec![file]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn nothing_starts_the_tap_while_the_mac_is_away() {
    let dir = TestDir::new("sysaudio-away-no-start");
    let mut runtime = recording_system_audio_runtime(&dir, None);
    runtime.away_probe = Some(|| true);

    // Session start, user resume and low-disk recovery all go through this.
    let started = start_system_audio_family_for_runtime(&mut runtime, Vec::new(), 4, "test")
        .expect("an away start declines, it does not fail");
    assert_eq!(started, None);
    assert!(runtime.active_system_audio_session.is_none());
    assert!(
        runtime.system_audio_away_hold,
        "the tick restarts it on return"
    );

    // The hold alone keeps it down until the tick sees the Mac back.
    runtime.away_probe = Some(|| panic!("a held start must not probe"));
    assert_eq!(
        start_system_audio_family_for_runtime(&mut runtime, Vec::new(), 4, "test")
            .expect("declines"),
        None
    );
    assert!(!should_retry_system_audio_start(&runtime), "nor the retry");
}

#[cfg(target_os = "macos")]
#[test]
fn coming_back_wants_the_tap_again_on_a_collision_safe_file() {
    let dir = TestDir::new("sysaudio-away-back");
    let mut runtime = recording_system_audio_runtime(&dir, None);
    runtime.system_audio_away_hold = true;

    assert!(
        apply_system_audio_away(&mut runtime, false, None),
        "came back"
    );
    assert!(!runtime.system_audio_away_hold);
    assert!(should_retry_system_audio_start(&runtime));
    // Only the tick that saw the Mac come back reports it.
    assert!(!apply_system_audio_away(&mut runtime, false, None));

    // The segment already holds the audio recorded before the Mac went away.
    let planner = runtime.system_audio_planner.clone().unwrap();
    let canonical = planner.system_audio_file(3);
    fs::create_dir_all(canonical.parent().unwrap()).unwrap();
    write_existing_audio_placeholder(&canonical);

    let rebuilt = first_system_audio_output_file(&planner, 3, true, false)
        .expect("an unpaused rebuild writes");
    assert_ne!(rebuilt, canonical, "never the segment's canonical name");
    assert!(rebuilt
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("sysaudio_away-segment-0003-")));
    // A fresh segment (session start, user resume, low-disk recovery) keeps it.
    assert_eq!(
        first_system_audio_output_file(&planner, 4, false, false),
        Some(planner.system_audio_file(4))
    );
}

#[cfg(target_os = "macos")]
#[test]
fn coming_back_paused_for_inactivity_wants_a_listening_tap() {
    let dir = TestDir::new("sysaudio-away-back-paused");
    let mut runtime = recording_system_audio_runtime(&dir, None);
    runtime.system_audio_away_hold = true;
    runtime
        .inactivity
        .set_family_paused_states(false, false, true);

    assert!(apply_system_audio_away(&mut runtime, false, None));
    assert!(should_retry_system_audio_start(&runtime));
    let planner = runtime.system_audio_planner.clone().unwrap();
    assert_eq!(
        first_system_audio_output_file(&planner, 3, true, true),
        None,
        "alive and listening, writing nothing until sound resumes it"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_session_without_system_audio_takes_no_hold() {
    let dir = TestDir::new("sysaudio-away-unrequested");
    let mut runtime = recording_system_audio_runtime(&dir, None);
    runtime.requested_sources = Some(CaptureSources {
        screen: true,
        microphone: true,
        system_audio: false,
    });

    assert!(!apply_system_audio_away(&mut runtime, true, None));
    assert!(!runtime.system_audio_away_hold);
}

// Silence is judged over the tap's own life. A session started hours ago whose
// tap was rebuilt a moment ago (the Mac came back) has not had a minute to hear
// anything yet.
#[cfg(target_os = "macos")]
#[test]
fn permission_silence_is_judged_from_the_current_tap_not_the_recording() {
    let now = 10_000_000;
    let runtime = NativeCaptureRuntime {
        recording_started_at_unix_ms: Some(now - 2 * 60 * 60 * 1000),
        system_audio_tap_started_at_unix_ms: Some(now - 5_000),
        ..Default::default()
    };
    assert_eq!(system_audio_tap_age_ms(&runtime, now), 5_000);

    let no_tap = NativeCaptureRuntime {
        recording_started_at_unix_ms: Some(now - 2 * 60 * 60 * 1000),
        ..Default::default()
    };
    assert_eq!(system_audio_tap_age_ms(&no_tap, now), 0);
}
