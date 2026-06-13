//! Shared on-device smoke invariant checks for the Windows capture passes
//! (GitHub issue #84).
//!
//! Issue #84 is the human-in-the-loop slice: an operator physically locks
//! (Win+L), sleeps/wakes, and idles/resumes the machine while a real native
//! capture runs (see `windows_transient_liveness_smoke` and
//! `windows_inactivity_smoke`). Those harnesses already drive the scenarios and
//! verify the runtime pause/resume bookkeeping. This module adds the two *new*
//! capture invariants this milestone introduced so that the operator's pass
//! checks them automatically against the artifacts the run produces:
//!
//!   1. **Frame-index sidecar (from #73).** Every finalized screen segment must
//!      carry a monotonic frame-index sidecar. We reuse
//!      [`capture_screen::screen_segment_frame_index_path`],
//!      [`capture_screen::decode_screen_segment_frame_index`], and
//!      [`capture_screen::screen_segment_frame_index_offsets_are_monotonic`].
//!
//!   2. **Inactivity tail hold-back (from #74).** A capture stopped *while
//!      inactivity-paused* must commit `.m4a` audio measurably **shorter** than
//!      the wall-clock capture window (the idle tail is discarded), while a
//!      **normal** stop must **not** be meaningfully shorter (the tail drains).
//!      We read committed `.m4a` duration through the `media-decode` MF seam.
//!
//! The pure comparison/decision logic lives in plain functions that take values
//! (durations, decoded sidecar bytes) so it is unit-testable on any platform;
//! the thin filesystem/Media-Foundation wrappers that feed those functions are
//! Windows-gated.

#[cfg(target_os = "windows")]
use std::path::Path;
#[cfg(target_os = "windows")]
use std::time::Duration;

/// A finalized screen segment fails the #73 invariant unless its sidecar both
/// loads and is monotonic. This decodes the sidecar payload and applies
/// [`capture_screen::screen_segment_frame_index_offsets_are_monotonic`].
///
/// Pure over the sidecar bytes so it is unit-testable without a real capture.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn frame_index_sidecar_payload_is_monotonic(bytes: &[u8]) -> Result<(), String> {
    let index = capture_screen::decode_screen_segment_frame_index(bytes)
        .map_err(|error| format!("frame-index sidecar failed to decode: {error}"))?;
    if index.entries.is_empty() {
        return Err("frame-index sidecar decoded to zero entries".to_string());
    }
    if !capture_screen::screen_segment_frame_index_offsets_are_monotonic(&index.entries) {
        return Err(format!(
            "frame-index sidecar video offsets are not monotonic across {} entries",
            index.entries.len()
        ));
    }
    Ok(())
}

/// Wall-clock vs committed-`.m4a`-duration verdict for a stop.
///
/// `tolerance_ms` absorbs codec priming / frame quantization so a *normal* stop
/// that legitimately commits ~the full window is not flagged as "shorter".
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) enum StopKind {
    /// Capture stopped while inactivity-paused: #74 discards the idle tail, so
    /// the committed `.m4a` must be shorter than the wall-clock window.
    Inactivity,
    /// Capture stopped normally: the tail drains, so the committed `.m4a` must
    /// not be meaningfully shorter than the wall-clock window.
    Normal,
}

/// Apply the #74 inactivity-tail invariant to one committed audio file.
///
/// `committed_audio_ms` is the decoded duration of the committed `.m4a`;
/// `wall_clock_ms` is the elapsed capture window the operator/harness observed.
/// For an [`StopKind::Inactivity`] stop the committed audio must be shorter than
/// the window by at least `tolerance_ms`; for [`StopKind::Normal`] it must not
/// fall short of the window by more than `tolerance_ms`.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn audio_tail_holdback_verdict(
    stop_kind: StopKind,
    committed_audio_ms: u64,
    wall_clock_ms: u64,
    tolerance_ms: u64,
) -> Result<(), String> {
    let shortfall = wall_clock_ms.saturating_sub(committed_audio_ms);
    match stop_kind {
        StopKind::Inactivity => {
            if shortfall > tolerance_ms {
                Ok(())
            } else {
                Err(format!(
                    "inactivity stop did not hold back the idle tail: committed audio {committed_audio_ms}ms vs wall-clock {wall_clock_ms}ms (shortfall {shortfall}ms <= tolerance {tolerance_ms}ms); the committed .m4a should be measurably shorter than the capture window"
                ))
            }
        }
        StopKind::Normal => {
            if shortfall <= tolerance_ms {
                Ok(())
            } else {
                Err(format!(
                    "normal stop committed audio that is shorter than the capture window: committed audio {committed_audio_ms}ms vs wall-clock {wall_clock_ms}ms (shortfall {shortfall}ms > tolerance {tolerance_ms}ms); a normal stop must drain the tail"
                ))
            }
        }
    }
}

/// Read a finalized screen segment's frame-index sidecar and assert it is
/// monotonic (#73). Resolves the sidecar path with
/// [`capture_screen::screen_segment_frame_index_path`].
#[cfg(target_os = "windows")]
pub(crate) fn assert_screen_segment_has_monotonic_sidecar(
    screen_video_path: &Path,
) -> Result<(), String> {
    let sidecar_path = capture_screen::screen_segment_frame_index_path(screen_video_path);
    let bytes = std::fs::read(&sidecar_path).map_err(|error| {
        format!(
            "finalized screen segment {} is missing its frame-index sidecar {}: {error}",
            screen_video_path.display(),
            sidecar_path.display()
        )
    })?;
    frame_index_sidecar_payload_is_monotonic(&bytes).map_err(|error| {
        format!(
            "frame-index sidecar {} for screen segment {} failed the #73 invariant: {error}",
            sidecar_path.display(),
            screen_video_path.display()
        )
    })
}

/// Assert every finalized screen segment in `screen_files` carries a monotonic
/// frame-index sidecar (#73). Errors name the first offending segment.
#[cfg(target_os = "windows")]
pub(crate) fn assert_all_screen_segments_have_monotonic_sidecars(
    screen_files: &[String],
) -> Result<(), String> {
    if screen_files.is_empty() {
        return Err(
            "no finalized screen segments were recorded; cannot verify frame-index sidecars"
                .to_string(),
        );
    }
    for screen_file in screen_files {
        assert_screen_segment_has_monotonic_sidecar(Path::new(screen_file))?;
    }
    Ok(())
}

/// Decode a committed `.m4a`'s duration in milliseconds through the Media
/// Foundation `media-decode` seam (ADR 0024). This is the same MF backend the
/// finalized-audio validation uses; duration is `samples / sample_rate`.
#[cfg(target_os = "windows")]
pub(crate) fn committed_audio_duration_ms(audio_path: &Path) -> Result<u64, String> {
    let decoded = media_decode::decode_to_mono_f32(audio_path).map_err(|error| {
        format!(
            "failed to decode committed audio {} for the #74 tail invariant: {error}",
            audio_path.display()
        )
    })?;
    if decoded.sample_rate_hz == 0 {
        return Err(format!(
            "committed audio {} decoded with a zero sample rate",
            audio_path.display()
        ));
    }
    let duration = Duration::from_secs_f64(
        decoded.samples.len() as f64 / f64::from(decoded.sample_rate_hz),
    );
    Ok(duration.as_millis() as u64)
}

/// Assert the #74 tail-holdback invariant against the committed microphone /
/// system-audio files of a stop. Every committed audio file in the pass must
/// satisfy [`audio_tail_holdback_verdict`] for `stop_kind`.
#[cfg(target_os = "windows")]
pub(crate) fn assert_audio_tail_holdback(
    stop_kind: StopKind,
    committed_audio_files: &[String],
    wall_clock_ms: u64,
    tolerance_ms: u64,
) -> Result<(), String> {
    if committed_audio_files.is_empty() {
        return Err(
            "no committed audio files were recorded; cannot verify the inactivity tail invariant"
                .to_string(),
        );
    }
    for audio_file in committed_audio_files {
        let committed_ms = committed_audio_duration_ms(Path::new(audio_file))?;
        audio_tail_holdback_verdict(stop_kind, committed_ms, wall_clock_ms, tolerance_ms).map_err(
            |error| format!("committed audio {audio_file} failed the #74 invariant: {error}"),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_screen::{
        encode_screen_segment_frame_index, ScreenSegmentFrameIndex, ScreenSegmentFrameIndexEntry,
    };

    fn entry(captured_at_unix_ms: u64, frame_index: u64, video_offset_ms: u64) -> ScreenSegmentFrameIndexEntry {
        ScreenSegmentFrameIndexEntry {
            captured_at_unix_ms,
            frame_index,
            video_offset_ms,
        }
    }

    fn encode(entries: Vec<ScreenSegmentFrameIndexEntry>) -> Vec<u8> {
        encode_screen_segment_frame_index(&ScreenSegmentFrameIndex {
            version: 1,
            entries,
        })
    }

    #[test]
    fn monotonic_sidecar_payload_passes() {
        let bytes = encode(vec![
            entry(1_000, 0, 0),
            entry(2_000, 1, 1_000),
            entry(3_000, 2, 2_000),
        ]);
        assert!(frame_index_sidecar_payload_is_monotonic(&bytes).is_ok());
    }

    #[test]
    fn non_monotonic_sidecar_payload_fails() {
        let bytes = encode(vec![
            entry(1_000, 0, 0),
            entry(2_000, 1, 2_000),
            entry(3_000, 2, 1_000),
        ]);
        let error = frame_index_sidecar_payload_is_monotonic(&bytes)
            .expect_err("non-monotonic offsets must fail the #73 invariant");
        assert!(error.contains("not monotonic"), "unexpected error: {error}");
    }

    #[test]
    fn empty_sidecar_payload_fails() {
        let bytes = encode(vec![]);
        let error = frame_index_sidecar_payload_is_monotonic(&bytes)
            .expect_err("an empty sidecar must fail the #73 invariant");
        assert!(error.contains("zero entries"), "unexpected error: {error}");
    }

    #[test]
    fn corrupt_sidecar_payload_fails_to_decode() {
        let error = frame_index_sidecar_payload_is_monotonic(b"not a sidecar")
            .expect_err("a corrupt sidecar must fail the #73 invariant");
        assert!(error.contains("failed to decode"), "unexpected error: {error}");
    }

    #[test]
    fn inactivity_stop_shorter_than_wall_clock_passes() {
        // Committed 4s of audio across a 10s wall-clock window: the idle tail
        // was held back, so the inactivity invariant is satisfied.
        assert!(audio_tail_holdback_verdict(StopKind::Inactivity, 4_000, 10_000, 500).is_ok());
    }

    #[test]
    fn inactivity_stop_full_length_fails() {
        // Committed ~the whole window: the tail was NOT held back.
        let error = audio_tail_holdback_verdict(StopKind::Inactivity, 9_800, 10_000, 500)
            .expect_err("a full-length inactivity stop must fail the #74 invariant");
        assert!(error.contains("did not hold back"), "unexpected error: {error}");
    }

    #[test]
    fn normal_stop_full_length_passes() {
        // A normal stop drains the tail, so committed ~= wall-clock.
        assert!(audio_tail_holdback_verdict(StopKind::Normal, 9_800, 10_000, 500).is_ok());
    }

    #[test]
    fn normal_stop_short_audio_fails() {
        // A normal stop that commits far less than the window indicates the tail
        // was wrongly discarded.
        let error = audio_tail_holdback_verdict(StopKind::Normal, 4_000, 10_000, 500)
            .expect_err("a short normal stop must fail the #74 invariant");
        assert!(error.contains("must drain the tail"), "unexpected error: {error}");
    }

    #[test]
    fn tolerance_absorbs_small_shortfall() {
        // A 300ms shortfall under a 500ms tolerance is "not shorter" for either
        // stop kind: normal passes, inactivity fails (it needed to be shorter).
        assert!(audio_tail_holdback_verdict(StopKind::Normal, 9_700, 10_000, 500).is_ok());
        assert!(audio_tail_holdback_verdict(StopKind::Inactivity, 9_700, 10_000, 500).is_err());
    }
}
