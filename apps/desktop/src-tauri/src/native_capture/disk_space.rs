//! Disk-space safety primitives for the capture pipeline.
//!
//! Self-contained, side-effect-free module owning the low-disk threshold math and
//! the free-space measurement seam used by capture preflight, rotation, and
//! suspend/resume. Design of record: [ADR 0040](../../../docs/adr/0040-low-disk-safety-is-a-transient-liveness-capture-suspension-kind.md).
//!
//! Low disk is modeled as a transient-liveness Capture Suspension. Free space is
//! checked exactly when a new segment file is about to be opened (preflight =
//! first segment, rotation = each next segment), never on a continuous poll. The
//! thresholds are settings-derived with fixed coefficients (no user setting):
//!
//! - `pause`   = `RESERVE_FLOOR_BYTES + next_segment_estimate`
//! - `resume`  = `RESERVE_FLOOR_BYTES + 2 * next_segment_estimate` (hysteresis)
//! - `critical` = `RESERVE_FLOOR_BYTES` (the app's own SQLite DB / OCR / OS are at
//!   risk below this, so the session stops gracefully rather than waiting)
//!
//! Reserving `floor + one-segment estimate` (rather than a flat byte constant) is
//! correct across the per-segment range, which runs from ~5 MB (audio-only) to
//! ~4.5 GB (120 Mbps screen × 5 min).
//!
//! All arithmetic is saturating: absurd inputs (e.g. `u64::MAX`) clamp instead of
//! panicking. Measurement is best-effort — an inability to *measure* never blocks
//! capture; only a measured shortfall acts.

use std::path::Path;

/// The reserve floor that protects the app's own storage (SQLite DB, OCR output,
/// previews) and the OS. 1 GiB.
pub(crate) const RESERVE_FLOOR_BYTES: u64 = 1024 * 1024 * 1024;

/// Per-second audio byte rate folded into the next-segment estimate when *any*
/// audio source (microphone and/or system audio) is requested. ~48 KB/s ≈
/// 384 kbps — a deliberately generous single headroom figure across mic + system
/// audio combined (real AAC is ~128 kbps/source), so audio-only capture reserves
/// almost nothing while the screen bitrate dominates whenever screen is on. When
/// neither audio source is requested the estimate uses `0` instead.
pub(crate) const AUDIO_BYTES_PER_SEC: u64 = 48_000;

/// The audio byte rate to fold into [`next_segment_estimate_bytes`] for a given
/// audio-source request: a single [`AUDIO_BYTES_PER_SEC`] headroom figure when
/// any audio source is on, `0` when neither is. (We do not scale by source count;
/// the constant already covers mic + system audio together.)
pub(crate) fn audio_bytes_per_sec_for_sources(
    capture_microphone: bool,
    capture_system_audio: bool,
) -> u64 {
    if capture_microphone || capture_system_audio {
        AUDIO_BYTES_PER_SEC
    } else {
        0
    }
}

/// Format a byte count as a human-readable, one-decimal string for user-facing
/// disk-space messages. Uses binary units (GiB/MiB/KiB) to match the reserve
/// floor and threshold math, which are all powers of two. There is no shared
/// byte formatter in the codebase, so this small helper lives beside the
/// thresholds it formats.
pub(crate) fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes_f = bytes as f64;
    if bytes_f >= GIB {
        format!("{:.1} GiB", bytes_f / GIB)
    } else if bytes_f >= MIB {
        format!("{:.1} MiB", bytes_f / MIB)
    } else if bytes_f >= KIB {
        format!("{:.1} KiB", bytes_f / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// The critical floor below which the session can no longer protect its own
/// storage and must stop gracefully. Equal to the reserve floor by design — the
/// reserve floor *is* the critical threshold.
pub(crate) const CRITICAL_FLOOR_BYTES: u64 = RESERVE_FLOOR_BYTES;

/// Estimated size in bytes of the next segment file, given the effective screen
/// bitrate, the audio byte rate, and the segment duration:
///
/// `estimate = (effective_screen_bitrate_bps / 8 + audio_bytes_per_sec) * segment_duration_seconds`
///
/// Saturating throughout so implausible inputs clamp to `u64::MAX` instead of
/// overflowing. For audio-only capture pass `effective_screen_bitrate_bps = 0`.
pub(crate) fn next_segment_estimate_bytes(
    effective_screen_bitrate_bps: u64,
    audio_bytes_per_sec: u64,
    segment_duration_seconds: u64,
) -> u64 {
    let screen_bytes_per_sec = effective_screen_bitrate_bps / 8;
    let bytes_per_sec = screen_bytes_per_sec.saturating_add(audio_bytes_per_sec);
    bytes_per_sec.saturating_mul(segment_duration_seconds)
}

/// Pause/preflight threshold: free space must stay at or above
/// `RESERVE_FLOOR_BYTES + estimate` for a new segment to be opened. Saturating.
pub(crate) fn pause_threshold_bytes(estimate_bytes: u64) -> u64 {
    RESERVE_FLOOR_BYTES.saturating_add(estimate_bytes)
}

/// Resume threshold (hysteresis): a suspended session only resumes once free
/// space reaches `RESERVE_FLOOR_BYTES + 2 * estimate`. The extra estimate of
/// headroom over the pause threshold prevents flapping between pause and resume.
/// Saturating.
pub(crate) fn resume_threshold_bytes(estimate_bytes: u64) -> u64 {
    RESERVE_FLOOR_BYTES.saturating_add(estimate_bytes.saturating_mul(2))
}

/// Critical threshold: below `CRITICAL_FLOOR_BYTES` the session stops gracefully.
/// Exposed as a helper so call sites stay uniform with the other thresholds.
pub(crate) fn critical_threshold_bytes() -> u64 {
    CRITICAL_FLOOR_BYTES
}

/// The decision a call site should take for a given measured free-space reading.
///
/// - `Sufficient`: free space is at or above the pause threshold; open the segment.
/// - `Pause`: free space is below the pause threshold but at or above the critical
///   floor; suspend and wait for recovery.
/// - `Critical`: free space is below the critical floor; stop gracefully (app
///   storage is now at risk).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LowDiskDecision {
    Sufficient,
    Pause,
    Critical,
}

/// Classify a measured free-space reading against an estimate's thresholds.
///
/// `Critical` if `free < critical`; else `Pause` if `free < pause_threshold`;
/// else `Sufficient`. (The critical floor is at or below the pause threshold,
/// so the ordering of these checks is total.)
pub(crate) fn classify_free_space(free_bytes: u64, estimate_bytes: u64) -> LowDiskDecision {
    if free_bytes < critical_threshold_bytes() {
        LowDiskDecision::Critical
    } else if free_bytes < pause_threshold_bytes(estimate_bytes) {
        LowDiskDecision::Pause
    } else {
        LowDiskDecision::Sufficient
    }
}

/// Whether a suspended session may resume: `free >= resume_threshold`. The
/// resume threshold sits one estimate above the pause threshold (hysteresis), so
/// a reading that merely cleared the pause threshold does not yet resume.
pub(crate) fn can_resume(free_bytes: u64, estimate_bytes: u64) -> bool {
    free_bytes >= resume_threshold_bytes(estimate_bytes)
}

/// A free-space probe seam: maps a path to the bytes available to a
/// non-privileged process on that path's volume. Defaults to
/// [`default_free_space_probe`]; tests inject a stub so suspend/resume/preflight
/// logic is exercisable without a real full disk.
pub(crate) type FreeSpaceProbe = fn(&Path) -> std::io::Result<u64>;

/// The production [`FreeSpaceProbe`]: `fs2::available_space`, which reports the
/// space available to a non-privileged process — the right bound for capture
/// writing into a user-space directory.
pub(crate) fn default_free_space_probe(path: &Path) -> std::io::Result<u64> {
    fs2::available_space(path)
}

/// Best-effort free-space measurement for the recordings root.
///
/// The recordings `save_directory` may not exist yet (first run, freshly
/// reconfigured path), so this probes the nearest existing ancestor of `root` —
/// mirroring the model-download disk preflight in `semantic_search_models.rs`.
///
/// Best-effort semantics: an inability to *measure* returns `None` and must never
/// block capture. That covers both "no existing ancestor to stat" and "the probe
/// returned `Err`". Only a `Some(free)` reading that a threshold then finds short
/// is allowed to act.
pub(crate) fn measure_free_space(root: &Path, probe: FreeSpaceProbe) -> Option<u64> {
    let probe_path = root.ancestors().find(|ancestor| ancestor.exists())?;
    probe(probe_path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // Representative byte rates used across the estimate tests.
    const SEGMENT_SECONDS: u64 = 300; // 5 minutes, the capped segment duration.
    const AUDIO_BYTES_PER_SEC: u64 = 16_000; // ~128 kbps AAC, audio-only baseline.

    #[test]
    fn audio_only_estimate_is_a_few_megabytes() {
        // bitrate 0, small audio rate, 300s -> a few MB.
        let estimate = next_segment_estimate_bytes(0, AUDIO_BYTES_PER_SEC, SEGMENT_SECONDS);
        assert_eq!(estimate, AUDIO_BYTES_PER_SEC * SEGMENT_SECONDS);
        assert_eq!(estimate, 4_800_000);
        // Comfortably in the single-digit-MB band.
        assert!(estimate < 8 * 1024 * 1024);
    }

    #[test]
    fn default_preset_estimate_is_reasonable() {
        // A representative default screen preset: ~8 Mbps screen + audio over 300s.
        let screen_bitrate_bps = 8_000_000;
        let estimate =
            next_segment_estimate_bytes(screen_bitrate_bps, AUDIO_BYTES_PER_SEC, SEGMENT_SECONDS);
        let expected = (screen_bitrate_bps / 8 + AUDIO_BYTES_PER_SEC) * SEGMENT_SECONDS;
        assert_eq!(estimate, expected);
        assert_eq!(estimate, 304_800_000); // ~290 MB
        // Between a few hundred MB and a GB for an 8 Mbps preset.
        assert!(estimate > 256 * 1024 * 1024);
        assert!(estimate < 1024 * 1024 * 1024);
    }

    #[test]
    fn max_bitrate_estimate_is_about_four_and_a_half_gigabytes() {
        // 120 Mbps screen, no audio, 300s -> 120_000_000 / 8 * 300 = 4.5 GB.
        let estimate = next_segment_estimate_bytes(120_000_000, 0, SEGMENT_SECONDS);
        assert_eq!(estimate, 120_000_000 / 8 * 300);
        assert_eq!(estimate, 4_500_000_000);
    }

    #[test]
    fn estimate_saturates_on_absurd_inputs() {
        // u64::MAX everywhere must not panic and must clamp to u64::MAX.
        let estimate = next_segment_estimate_bytes(u64::MAX, u64::MAX, u64::MAX);
        assert_eq!(estimate, u64::MAX);

        // A saturating-add overflow in the per-second rate also clamps.
        let estimate = next_segment_estimate_bytes(u64::MAX, u64::MAX, 1);
        assert_eq!(estimate, u64::MAX);

        // A saturating-mul overflow in the duration also clamps.
        let estimate = next_segment_estimate_bytes(0, 1_000_000_000, u64::MAX);
        assert_eq!(estimate, u64::MAX);
    }

    #[test]
    fn thresholds_derive_from_estimate_and_floor() {
        let estimate = 2_000_000_000; // 2 GB
        assert_eq!(pause_threshold_bytes(estimate), RESERVE_FLOOR_BYTES + estimate);
        assert_eq!(
            resume_threshold_bytes(estimate),
            RESERVE_FLOOR_BYTES + 2 * estimate
        );
        assert_eq!(critical_threshold_bytes(), RESERVE_FLOOR_BYTES);
        // critical == reserve floor by design.
        assert_eq!(CRITICAL_FLOOR_BYTES, RESERVE_FLOOR_BYTES);
    }

    #[test]
    fn thresholds_saturate_on_absurd_estimate() {
        // An estimate near u64::MAX must not overflow the floor addition / doubling.
        assert_eq!(pause_threshold_bytes(u64::MAX), u64::MAX);
        assert_eq!(resume_threshold_bytes(u64::MAX), u64::MAX);
        // Doubling alone can overflow before the floor is added.
        assert_eq!(resume_threshold_bytes(u64::MAX / 2 + 1), u64::MAX);
    }

    #[test]
    fn classify_critical_below_floor() {
        let estimate = 500_000_000;
        // Strictly below the reserve floor -> Critical regardless of estimate.
        assert_eq!(
            classify_free_space(RESERVE_FLOOR_BYTES - 1, estimate),
            LowDiskDecision::Critical
        );
        assert_eq!(classify_free_space(0, estimate), LowDiskDecision::Critical);
    }

    #[test]
    fn classify_pause_between_floor_and_pause_threshold() {
        let estimate = 500_000_000;
        let pause = pause_threshold_bytes(estimate);
        // At the floor (>= critical) but below the pause threshold -> Pause.
        assert_eq!(
            classify_free_space(RESERVE_FLOOR_BYTES, estimate),
            LowDiskDecision::Pause
        );
        assert_eq!(
            classify_free_space(pause - 1, estimate),
            LowDiskDecision::Pause
        );
    }

    #[test]
    fn classify_sufficient_at_or_above_pause_threshold() {
        let estimate = 500_000_000;
        let pause = pause_threshold_bytes(estimate);
        assert_eq!(classify_free_space(pause, estimate), LowDiskDecision::Sufficient);
        assert_eq!(
            classify_free_space(pause + 1, estimate),
            LowDiskDecision::Sufficient
        );
    }

    #[test]
    fn hysteresis_between_pause_and_resume_does_not_flap() {
        let estimate = 500_000_000;
        let pause = pause_threshold_bytes(estimate);
        let resume = resume_threshold_bytes(estimate);
        assert!(resume > pause, "resume must sit above pause for hysteresis");

        // A reading strictly between the pause and resume thresholds: it cleared
        // pause (so it is Sufficient for opening a new segment) but a suspended
        // session must NOT resume yet — this is the band that prevents flapping.
        let between = (pause + resume) / 2;
        assert!(between > pause && between < resume);
        assert!(
            !can_resume(between, estimate),
            "must not resume in the hysteresis band"
        );

        // Just below resume still does not resume.
        assert!(!can_resume(resume - 1, estimate));
    }

    #[test]
    fn resume_at_or_above_resume_threshold_is_sufficient_and_resumable() {
        let estimate = 500_000_000;
        let resume = resume_threshold_bytes(estimate);
        // At/above resume: both resumable and classified Sufficient.
        assert!(can_resume(resume, estimate));
        assert!(can_resume(resume + 1, estimate));
        assert_eq!(
            classify_free_space(resume, estimate),
            LowDiskDecision::Sufficient
        );
    }

    #[test]
    fn audio_bytes_per_sec_for_sources_folds_in_only_when_audio_requested() {
        // NB: the test module shadows the production constant with a smaller
        // audio-only baseline, so reference the production figure by its path.
        let headroom = crate::native_capture::disk_space::AUDIO_BYTES_PER_SEC;
        assert_eq!(audio_bytes_per_sec_for_sources(false, false), 0);
        assert_eq!(audio_bytes_per_sec_for_sources(true, false), headroom);
        assert_eq!(audio_bytes_per_sec_for_sources(false, true), headroom);
        // Both sources still folds in a single headroom figure, not double.
        assert_eq!(audio_bytes_per_sec_for_sources(true, true), headroom);
    }

    #[test]
    fn human_bytes_formats_binary_units_with_one_decimal() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(human_bytes(RESERVE_FLOOR_BYTES), "1.0 GiB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024 / 2), "1.5 GiB");
    }

    #[test]
    fn measure_returns_none_when_probe_errors() {
        // A probe that always errors -> measurement is None and never blocks.
        fn erroring_probe(_path: &Path) -> std::io::Result<u64> {
            Err(std::io::Error::new(std::io::ErrorKind::Other, "boom"))
        }
        // Use an existing ancestor so we reach the probe (and the probe is what fails).
        let root = std::env::temp_dir().join("mnema-disk-space-test-leaf");
        assert_eq!(measure_free_space(&root, erroring_probe), None);
    }

    #[test]
    fn measure_walks_to_existing_ancestor_when_leaf_missing() {
        // A probe that succeeds only for an existing path; the leaf does not exist
        // but its ancestor (the system temp dir) does, so we get the probed value.
        fn existing_ancestor_probe(path: &Path) -> std::io::Result<u64> {
            if path.exists() {
                Ok(4_242_424_242)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "path does not exist",
                ))
            }
        }

        let temp = std::env::temp_dir();
        assert!(temp.exists(), "system temp dir is expected to exist");
        // A leaf path that almost certainly does not exist on disk.
        let leaf: PathBuf = temp
            .join("mnema-disk-space-nonexistent")
            .join("save")
            .join("recordings");
        assert!(!leaf.exists());

        assert_eq!(
            measure_free_space(&leaf, existing_ancestor_probe),
            Some(4_242_424_242)
        );
    }

    #[test]
    fn measure_returns_none_when_no_ancestor_exists() {
        // No ancestor exists -> None (never blocks). A relative path whose
        // components don't exist on the filesystem exercises the ancestor-walk
        // miss; the empty path's only ancestor ("") does not exist either.
        fn never_called_probe(_path: &Path) -> std::io::Result<u64> {
            panic!("probe must not be called when no ancestor exists");
        }
        let root = Path::new("this-relative-path/almost-certainly/does-not-exist-xyz");
        assert_eq!(measure_free_space(root, never_called_probe), None);
    }
}
