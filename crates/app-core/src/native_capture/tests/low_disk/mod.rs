// ---------------------------------------------------------------------------
// Low-disk safety (ADR 0040): preflight refusal + suspend/resume transitions.
//
// `disk_space::FreeSpaceProbe` is a bare `fn` pointer, so a scripted free-space
// sequence is driven through a thread_local queue: each probe call pops the next
// reading (and repeats the last one once the queue is drained), letting a single
// test feed a decreasing-then-increasing sequence to drive the enter/stay/resume
// decisions without a real full disk.
// ---------------------------------------------------------------------------

use super::*;
use crate::disk_space;

thread_local! {
    static SCRIPTED_FREE_BYTES: std::cell::RefCell<Vec<u64>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Load a scripted free-space sequence (consumed front-to-back). The probe
/// repeats the final value after the sequence is drained so callers that probe
/// more often than they scripted still get a stable reading.
fn script_free_bytes(sequence: &[u64]) {
    SCRIPTED_FREE_BYTES.with(|cell| {
        // Reverse so we can cheaply `pop` from the back in call order.
        *cell.borrow_mut() = sequence.iter().rev().copied().collect();
    });
}

/// A [`disk_space::FreeSpaceProbe`] that returns the next scripted reading.
/// Ignores the probed path (the scripted sequence is what drives the test).
fn scripted_probe(_path: &std::path::Path) -> std::io::Result<u64> {
    SCRIPTED_FREE_BYTES.with(|cell| {
        let mut queue = cell.borrow_mut();
        if queue.len() > 1 {
            Ok(queue.pop().expect("queue is non-empty"))
        } else {
            // Repeat the last scripted value indefinitely.
            queue
                .last()
                .copied()
                .ok_or_else(|| std::io::Error::other("no scripted free-space reading"))
        }
    })
}

/// A probe that always errors — exercises the best-effort "unmeasurable never
/// blocks / never suspends" path.
fn erroring_probe(_path: &std::path::Path) -> std::io::Result<u64> {
    Err(std::io::Error::other("disk probe unavailable"))
}

const SEGMENT_SECONDS: u64 = 300;
const SCREEN_BITRATE_BPS: u32 = 8_000_000;

/// The next-segment estimate matching the fixtures below: 8 Mbps screen +
/// audio over 300s. Used to derive the pause/resume thresholds for assertions.
fn fixture_estimate() -> u64 {
    disk_space::next_segment_estimate_bytes(
        SCREEN_BITRATE_BPS as u64,
        disk_space::audio_bytes_per_sec_for_sources(true, true),
        SEGMENT_SECONDS,
    )
}

/// A running screen+mic+system-audio runtime whose planner roots at the system
/// temp dir (so `measure_free_space`'s nearest-existing-ancestor walk reaches a
/// real, statable path) and whose free-space probe is the scripted stub.
fn low_disk_runtime_fixture() -> NativeCaptureRuntime {
    let root = std::env::temp_dir().to_string_lossy().into_owned();
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
        effective_screen_bitrate_bps: Some(SCREEN_BITRATE_BPS),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(
            SEGMENT_SECONDS,
        ))),
        segment_planner: Some(SegmentPlanner::with_date_prefix(
            root,
            "native-session-low-disk",
            "2026/06/25",
        )),
        free_space_probe: Some(scripted_probe),
        ..Default::default()
    }
}

// --- Preflight (a) ---

#[test]
fn preflight_refuses_below_pause_threshold() {
    let estimate = disk_space::next_segment_estimate_bytes(
        SCREEN_BITRATE_BPS as u64,
        disk_space::audio_bytes_per_sec_for_sources(true, false),
        SEGMENT_SECONDS,
    );
    let pause = disk_space::pause_threshold_bytes(estimate);
    script_free_bytes(&[pause - 1]);

    let sources = CaptureSources {
        screen: true,
        microphone: true,
        system_audio: false,
    };
    let result = crate::native_capture::segments::preflight_disk_space_check(
        &std::env::temp_dir(),
        Some(SCREEN_BITRATE_BPS),
        &sources,
        SEGMENT_SECONDS,
        scripted_probe,
    );
    let error = result.expect_err("preflight must refuse below the pause threshold");
    assert_eq!(error.code, "insufficient_disk_space");
}

#[test]
fn preflight_proceeds_at_or_above_pause_threshold() {
    let estimate = disk_space::next_segment_estimate_bytes(
        SCREEN_BITRATE_BPS as u64,
        disk_space::audio_bytes_per_sec_for_sources(true, false),
        SEGMENT_SECONDS,
    );
    let pause = disk_space::pause_threshold_bytes(estimate);
    let sources = CaptureSources {
        screen: true,
        microphone: true,
        system_audio: false,
    };

    // Exactly at the threshold proceeds (the refusal is strict `<`).
    script_free_bytes(&[pause]);
    crate::native_capture::segments::preflight_disk_space_check(
        &std::env::temp_dir(),
        Some(SCREEN_BITRATE_BPS),
        &sources,
        SEGMENT_SECONDS,
        scripted_probe,
    )
    .expect("preflight must proceed at the pause threshold");

    // Comfortably above proceeds too.
    script_free_bytes(&[pause + 10 * 1024 * 1024 * 1024]);
    crate::native_capture::segments::preflight_disk_space_check(
        &std::env::temp_dir(),
        Some(SCREEN_BITRATE_BPS),
        &sources,
        SEGMENT_SECONDS,
        scripted_probe,
    )
    .expect("preflight must proceed above the pause threshold");
}

#[test]
fn preflight_does_not_block_when_unmeasurable() {
    let sources = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: false,
    };
    // An erroring probe -> unmeasurable -> must NOT block the start.
    crate::native_capture::segments::preflight_disk_space_check(
        &std::env::temp_dir(),
        Some(SCREEN_BITRATE_BPS),
        &sources,
        SEGMENT_SECONDS,
        erroring_probe,
    )
    .expect("an unmeasurable probe must never block the start");
}

// --- Suspend / resume transitions (b) ---

#[test]
fn boundary_decision_enters_low_disk_below_pause() {
    let estimate = fixture_estimate();
    let pause = disk_space::pause_threshold_bytes(estimate);
    let mut runtime = low_disk_runtime_fixture();

    // Below the pause threshold -> the boundary would suspend (Pause/Critical).
    script_free_bytes(&[pause - 1]);
    let decision = crate::native_capture::segments::low_disk_decision_at_boundary(&runtime)
        .expect("a measurable reading must classify");
    assert_ne!(
        decision,
        disk_space::LowDiskDecision::Sufficient,
        "below the pause threshold the boundary must not be Sufficient"
    );

    // Strictly below the reserve floor classifies Critical (still suspends).
    script_free_bytes(&[disk_space::RESERVE_FLOOR_BYTES - 1]);
    assert_eq!(
        crate::native_capture::segments::low_disk_decision_at_boundary(&runtime),
        Some(disk_space::LowDiskDecision::Critical)
    );

    // At/above the pause threshold the boundary opens the next segment.
    script_free_bytes(&[pause]);
    assert_eq!(
        crate::native_capture::segments::low_disk_decision_at_boundary(&runtime),
        Some(disk_space::LowDiskDecision::Sufficient)
    );

    // Unmeasurable -> None (best-effort: the rotation proceeds, no suspend).
    runtime.free_space_probe = Some(erroring_probe);
    assert_eq!(
        crate::native_capture::segments::low_disk_decision_at_boundary(&runtime),
        None
    );
}

#[test]
fn hysteresis_stays_suspended_between_pause_and_resume_then_resumes() {
    let estimate = fixture_estimate();
    let pause = disk_space::pause_threshold_bytes(estimate);
    let resume = disk_space::resume_threshold_bytes(estimate);
    assert!(resume > pause, "resume must sit above pause for hysteresis");
    let runtime = low_disk_runtime_fixture();

    // A reading in the hysteresis band (cleared pause, not yet resume): a
    // suspended session must NOT resume — this prevents flapping.
    let between = (pause + resume) / 2;
    script_free_bytes(&[between]);
    assert_eq!(
        crate::native_capture::segments::low_disk_can_resume(&runtime),
        Some(false),
        "must stay suspended in the hysteresis band"
    );

    // Just below resume still does not resume.
    script_free_bytes(&[resume - 1]);
    assert_eq!(
        crate::native_capture::segments::low_disk_can_resume(&runtime),
        Some(false)
    );

    // At/above the resume threshold the session resumes.
    script_free_bytes(&[resume]);
    assert_eq!(
        crate::native_capture::segments::low_disk_can_resume(&runtime),
        Some(true)
    );
}

#[test]
fn decreasing_then_increasing_sequence_drives_enter_then_resume() {
    let estimate = fixture_estimate();
    let pause = disk_space::pause_threshold_bytes(estimate);
    let resume = disk_space::resume_threshold_bytes(estimate);
    let runtime = low_disk_runtime_fixture();

    // Decreasing sequence ending below pause -> would enter LowDisk.
    script_free_bytes(&[resume + 1, pause + 1, pause - 1]);
    // Drain the first two (still sufficient) then land below pause.
    let _ = crate::native_capture::segments::low_disk_decision_at_boundary(&runtime);
    let _ = crate::native_capture::segments::low_disk_decision_at_boundary(&runtime);
    assert_ne!(
        crate::native_capture::segments::low_disk_decision_at_boundary(&runtime),
        Some(disk_space::LowDiskDecision::Sufficient),
        "the decreasing sequence must end below the pause threshold (enter LowDisk)"
    );

    // Increasing sequence reaching >= resume -> resumes.
    script_free_bytes(&[pause + 1, resume - 1, resume + 1]);
    let _ = crate::native_capture::segments::low_disk_can_resume(&runtime);
    let _ = crate::native_capture::segments::low_disk_can_resume(&runtime);
    assert_eq!(
        crate::native_capture::segments::low_disk_can_resume(&runtime),
        Some(true),
        "the increasing sequence must reach the resume threshold"
    );
}

#[test]
fn low_disk_suspend_stops_microphone_but_other_kinds_keep_it() {
    let low_disk_error = CaptureErrorResponse {
        code: "capture_low_disk".to_string(),
        message: "low disk".to_string(),
    };

    // LowDisk suspends the mic too: with no live session but a live mic
    // recording file, the suspend entry must clear the mic recording file.
    let mut low_disk_runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        microphone_recording_file: Some("/tmp/low-disk-mic.m4a".to_string()),
        ..Default::default()
    };
    crate::native_capture::segments::suspend_screen_capture(
        None,
        &mut low_disk_runtime,
        &low_disk_error,
        CaptureSuspensionKind::LowDisk,
    )
    .expect("low-disk suspend should succeed with no live screen session");
    assert!(
        low_disk_runtime.microphone_recording_file.is_none(),
        "LowDisk must stop the microphone (clear its recording file)"
    );
    assert_eq!(
        low_disk_runtime
            .capture_suspension
            .as_ref()
            .map(|suspension| suspension.kind),
        Some(CaptureSuspensionKind::LowDisk)
    );

    // PrivacyFilter keeps the mic alive: the mic continuation is preserved.
    let privacy_error = CaptureErrorResponse {
        code: "privacy_filter_apply_failed".to_string(),
        message: "privacy".to_string(),
    };
    let mut privacy_runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        microphone_recording_file: Some("/tmp/privacy-mic.m4a".to_string()),
        ..Default::default()
    };
    crate::native_capture::segments::suspend_screen_capture(
        None,
        &mut privacy_runtime,
        &privacy_error,
        CaptureSuspensionKind::PrivacyFilter,
    )
    .expect("privacy suspend should succeed with no live screen session");
    assert_eq!(
        privacy_runtime.microphone_recording_file.as_deref(),
        Some("/tmp/privacy-mic.m4a"),
        "non-LowDisk kinds must keep the microphone alive"
    );
}

// --- Mid-segment disk-full: partial discard + graceful stop (c) ---

#[test]
fn discard_partial_on_disk_full_deletes_final_paths_and_commits_no_row() {
    // Create real partial files at their final paths; the discard routine must
    // delete each one and report no committed segment row (returns false).
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let screen_path = temp.path().join("segment-0001.mov");
    let mic_path = temp.path().join("segment-0001-mic.m4a");
    let sys_path = temp.path().join("segment-0001-sys.m4a");
    for path in [&screen_path, &mic_path, &sys_path] {
        std::fs::write(path, b"partial-corrupt-bytes").expect("partial file should be written");
        assert!(path.exists(), "partial fixture must exist before discard");
    }
    let screen = screen_path.to_string_lossy().into_owned();
    let mic = mic_path.to_string_lossy().into_owned();
    let sys = sys_path.to_string_lossy().into_owned();

    let output_files = CaptureOutputFiles {
        screen_file: Some(screen.clone()),
        screen_files: vec![screen.clone()],
        microphone_file: Some(mic.clone()),
        microphone_files: vec![mic.clone()],
        system_audio_file: Some(sys.clone()),
        system_audio_files: vec![sys.clone()],
    };

    let committed = crate::native_capture::segments::discard_partial_segment_on_disk_full(
        Some(&output_files),
        Some(&screen),
        Some(&mic),
        Some(&sys),
    );

    assert!(
        !committed,
        "the disk-full discard must commit NO segment row (returns false)"
    );
    assert!(!screen_path.exists(), "screen partial must be deleted");
    assert!(!mic_path.exists(), "microphone partial must be deleted");
    assert!(!sys_path.exists(), "system-audio partial must be deleted");
}

#[test]
fn discard_partial_on_disk_full_is_best_effort_for_missing_paths() {
    // A path that does not exist (NotFound) must be tolerated silently and the
    // routine must still report no committed row.
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let missing = temp.path().join("never-written.mov");
    let missing = missing.to_string_lossy().into_owned();
    let committed = crate::native_capture::segments::discard_partial_segment_on_disk_full(
        None,
        Some(&missing),
        None,
        None,
    );
    assert!(!committed, "a missing partial still commits no row");
}

/// Produces a real, positive-duration AAC/`.m4a` so the AVFoundation duration
/// validation inside `finalize_capture_outputs` accepts it as usable captured
/// audio (a junk byte file would be legitimately dropped as unusable, which
/// would mask the data-loss this test guards against). Mirrors the helper in
/// `native_capture_output.rs`'s tests.
pub(super) fn write_valid_m4a_audio_file(path: &std::path::Path) {
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

mod stops;
