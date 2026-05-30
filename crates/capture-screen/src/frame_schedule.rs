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
/// frame rate.
///
/// A `frame_rate` of `0` means "no cap" and returns `0`, so
/// [`should_drop_frame`] keeps every frame.
pub fn frame_cap_min_interval_ticks(frame_rate: u32) -> i64 {
    if frame_rate == 0 {
        return 0;
    }
    TICKS_PER_SECOND / frame_rate as i64
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
        assert_eq!(frame_cap_min_interval_ticks(0), 0);
        // With no cap, every candidate is kept regardless of spacing.
        assert!(!should_drop_frame(Some(0), 1, 0));
        assert!(!should_drop_frame(None, 0, 0));
    }

    #[test]
    fn interval_ticks_match_frame_rate() {
        assert_eq!(frame_cap_min_interval_ticks(1), 10_000_000);
        assert_eq!(frame_cap_min_interval_ticks(30), 333_333);
        assert_eq!(frame_cap_min_interval_ticks(60), 166_666);
    }

    #[test]
    fn first_frame_is_always_kept() {
        let interval = frame_cap_min_interval_ticks(30);
        assert!(!should_drop_frame(None, 0, interval));
        assert!(!should_drop_frame(None, 5, interval));
    }

    #[test]
    fn sub_interval_frame_is_dropped() {
        let interval = frame_cap_min_interval_ticks(30);
        // Arrives only half an interval after the last kept frame.
        assert!(should_drop_frame(Some(1_000_000), 1_000_000 + interval / 2, interval));
    }

    #[test]
    fn frame_at_or_above_interval_is_kept() {
        let interval = frame_cap_min_interval_ticks(30);
        assert!(!should_drop_frame(Some(1_000_000), 1_000_000 + interval, interval));
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
}
