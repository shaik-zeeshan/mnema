//! Cross-platform frame-pacing logic for the capture backends.
//!
//! This module is deliberately free of any platform API: it is pure arithmetic
//! over 100-nanosecond ticks (the unit used by both Windows Media Foundation
//! sample times and `Direct3D11CaptureFrame::SystemRelativeTime`). Keeping it
//! platform-agnostic lets the frame-rate cap and the per-segment timestamp
//! baseline be unit-tested on every CI target (macOS *and* Windows), not just on
//! real Windows hardware.
//!
//! The Windows backend ([`crate::windows_capture`]) drives capture at the
//! source's variable, change-driven rate; these helpers decide which arriving
//! frames to keep so the encoded stream honors `screen_frame_rate`, and rebase
//! each segment's timestamps to start at zero across rotation.

/// Number of 100ns ticks in one second.
const TICKS_PER_SECOND: i64 = 10_000_000;

/// Minimum spacing, in 100ns ticks, between two kept frames for a given target
/// frame rate. Fractional rates are supported (e.g. `0.5` fps → a 2s interval).
///
/// A non-positive or non-finite `frame_rate` means "no cap" and returns `0`, so
/// [`should_drop_frame`] keeps every frame.
pub fn frame_cap_min_interval_ticks(frame_rate: f64) -> i64 {
    if !frame_rate.is_finite() || frame_rate <= 0.0 {
        return 0;
    }
    (TICKS_PER_SECOND as f64 / frame_rate) as i64
}

/// Decide whether an arriving frame should be dropped to honor the frame-rate
/// cap.
///
/// - The first frame of a segment (`last_kept_ticks == None`) is always kept.
/// - With no cap (`min_interval_ticks <= 0`) every frame is kept.
/// - Otherwise the frame is dropped when it arrives less than
///   `min_interval_ticks` after the last kept frame.
pub fn should_drop_frame(
    last_kept_ticks: Option<i64>,
    candidate_ticks: i64,
    min_interval_ticks: i64,
) -> bool {
    if min_interval_ticks <= 0 {
        return false;
    }
    match last_kept_ticks {
        None => false,
        Some(last) => candidate_ticks - last < min_interval_ticks,
    }
}

/// Duration for a sample once the following kept frame is known.
///
/// Media Foundation timestamps are segment-relative. With one-frame lookahead,
/// the current sample lasts until the next kept sample time. Out-of-order or
/// duplicate timestamps fall back to the nominal frame duration so callers do
/// not write zero/negative durations.
pub fn lookahead_sample_duration_ticks(
    sample_ticks: i64,
    next_sample_ticks: i64,
    fallback_duration_ticks: i64,
) -> i64 {
    (next_sample_ticks - sample_ticks).max(fallback_duration_ticks.max(1))
}

/// Duration for the held final sample when a segment is closed at a known
/// boundary.
///
/// The duration is clamped so the sample does not extend past the segment
/// boundary. If the held sample is already at or beyond the boundary, there is
/// no positive-duration sample to write.
pub fn boundary_clamped_lookahead_duration_ticks(
    sample_ticks: i64,
    boundary_ticks: i64,
) -> Option<i64> {
    (boundary_ticks - sample_ticks)
        .checked_sub(0)
        .filter(|duration| *duration > 0)
}

/// Rebases absolute capture timestamps so each segment's first kept frame starts
/// at tick zero.
///
/// The capture source reports monotonically-increasing absolute times
/// (`SystemRelativeTime`); the sink writer wants per-segment-relative sample
/// times. The first observed absolute time sets the baseline `t0`; subsequent
/// frames report `absolute - t0`. After a rotation the runtime calls
/// [`SegmentTimeline::reset`] so the next segment rebaselines from its own first
/// frame.
#[derive(Debug, Default, Clone)]
pub struct SegmentTimeline {
    baseline: Option<i64>,
}

impl SegmentTimeline {
    /// Construct a timeline with no baseline yet (the first frame sets it).
    pub fn new() -> Self {
        Self { baseline: None }
    }

    /// Map an absolute tick value to a segment-relative one, clamped to be
    /// non-negative. The first call latches the baseline and returns `0`.
    pub fn relative_ticks(&mut self, absolute_ticks: i64) -> i64 {
        let baseline = *self.baseline.get_or_insert(absolute_ticks);
        (absolute_ticks - baseline).max(0)
    }

    /// Clear the baseline so the next frame rebaselines the timeline. Called on
    /// segment rotation.
    pub fn reset(&mut self) {
        self.baseline = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_frame_rate_disables_the_cap() {
        assert_eq!(frame_cap_min_interval_ticks(0.0), 0);
        // With no cap, every candidate is kept regardless of spacing.
        assert!(!should_drop_frame(Some(0), 1, 0));
        assert!(!should_drop_frame(None, 0, 0));
    }

    #[test]
    fn interval_ticks_match_frame_rate() {
        assert_eq!(frame_cap_min_interval_ticks(1.0), 10_000_000);
        assert_eq!(frame_cap_min_interval_ticks(30.0), 333_333);
        assert_eq!(frame_cap_min_interval_ticks(60.0), 166_666);
    }

    #[test]
    fn fractional_frame_rate_stretches_the_interval_past_one_second() {
        // The 0.5 fps default keeps one frame every 2 seconds.
        assert_eq!(frame_cap_min_interval_ticks(0.5), 20_000_000);
        assert_eq!(frame_cap_min_interval_ticks(0.25), 40_000_000);
        // Degenerate rates disable the cap instead of overflowing.
        assert_eq!(frame_cap_min_interval_ticks(-1.0), 0);
        assert_eq!(frame_cap_min_interval_ticks(f64::NAN), 0);
        assert_eq!(frame_cap_min_interval_ticks(f64::INFINITY), 0);
    }

    #[test]
    fn first_frame_is_always_kept() {
        let interval = frame_cap_min_interval_ticks(30.0);
        assert!(!should_drop_frame(None, 0, interval));
        assert!(!should_drop_frame(None, 5, interval));
    }

    #[test]
    fn sub_interval_frame_is_dropped() {
        let interval = frame_cap_min_interval_ticks(30.0);
        // Arrives only half an interval after the last kept frame.
        assert!(should_drop_frame(
            Some(1_000_000),
            1_000_000 + interval / 2,
            interval
        ));
    }

    #[test]
    fn frame_at_or_above_interval_is_kept() {
        let interval = frame_cap_min_interval_ticks(30.0);
        assert!(!should_drop_frame(
            Some(1_000_000),
            1_000_000 + interval,
            interval
        ));
        assert!(!should_drop_frame(
            Some(1_000_000),
            1_000_000 + interval + 1,
            interval
        ));
    }

    #[test]
    fn timeline_baselines_on_first_frame() {
        let mut timeline = SegmentTimeline::new();
        assert_eq!(timeline.relative_ticks(5_000_000), 0);
        assert_eq!(timeline.relative_ticks(5_500_000), 500_000);
        assert_eq!(timeline.relative_ticks(6_000_000), 1_000_000);
    }

    #[test]
    fn timeline_clamps_out_of_order_frames_to_zero() {
        let mut timeline = SegmentTimeline::new();
        assert_eq!(timeline.relative_ticks(5_000_000), 0);
        // A frame reporting a time earlier than the baseline clamps to 0 rather
        // than producing a negative sample time.
        assert_eq!(timeline.relative_ticks(4_000_000), 0);
    }

    #[test]
    fn reset_rebaselines_after_rotation() {
        let mut timeline = SegmentTimeline::new();
        assert_eq!(timeline.relative_ticks(5_000_000), 0);
        assert_eq!(timeline.relative_ticks(8_000_000), 3_000_000);

        timeline.reset();
        // The next segment rebaselines from its own first frame.
        assert_eq!(timeline.relative_ticks(20_000_000), 0);
        assert_eq!(timeline.relative_ticks(21_000_000), 1_000_000);
    }

    #[test]
    fn lookahead_duration_uses_next_frame_delta() {
        assert_eq!(
            lookahead_sample_duration_ticks(1_000_000, 1_400_000, frame_cap_min_interval_ticks(30.0)),
            400_000
        );
    }

    #[test]
    fn lookahead_duration_falls_back_for_non_increasing_timestamps() {
        let fallback = frame_cap_min_interval_ticks(30.0);
        assert_eq!(
            lookahead_sample_duration_ticks(1_000_000, 1_000_000, fallback),
            fallback
        );
        assert_eq!(
            lookahead_sample_duration_ticks(1_000_000, 900_000, fallback),
            fallback
        );
    }

    #[test]
    fn boundary_clamped_lookahead_duration_stops_at_segment_boundary() {
        assert_eq!(
            boundary_clamped_lookahead_duration_ticks(59_900_000, 60_000_000),
            Some(100_000)
        );
        assert_eq!(
            boundary_clamped_lookahead_duration_ticks(60_000_000, 60_000_000),
            None
        );
        assert_eq!(
            boundary_clamped_lookahead_duration_ticks(60_100_000, 60_000_000),
            None
        );
    }
}
