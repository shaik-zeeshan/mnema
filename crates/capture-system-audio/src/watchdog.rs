//! The zero-watchdog: the only way a wedged tap gets noticed.
//!
//! CATap's macOS-26 failure mode is that the IOProc keeps firing at the right
//! cadence while every sample is exactly zero, for stretches of a minute to a
//! quarter of an hour, with no error anywhere (Apple forums 825780). All-zero
//! delivery is indistinguishable from real silence, so this assumes guilt after
//! a while and rebuilds — lossless by construction, because a rebuild during
//! zeros cannot lose audio that was never delivered — and backs off, because
//! real silence would otherwise rebuild the tap every 30 seconds forever.
//!
//! One rule covers both shapes of failure: **how long since the last sound?**
//! That catches zeros arriving and, for free, deliveries stopping altogether —
//! the shape the ScreenCaptureKit tap died in.
//!
//! Time is injected rather than read, so the ladder below is a unit test rather
//! than a ten-minute one.

use std::time::{Duration, Instant};

/// Sustained silence for this long trips the first rebuild (ADR 0052).
pub const ZERO_WATCHDOG_INITIAL_DELAY: Duration = Duration::from_secs(30);

/// The backoff ceiling. A machine playing nothing at all delivers zeros
/// indefinitely and would otherwise rebuild forever; at the cap it costs six
/// rebuilds an hour, none of which can lose audio.
pub const ZERO_WATCHDOG_MAX_DELAY: Duration = Duration::from_secs(600);

/// Tracks how long a tap generation has gone without delivering sound.
///
/// Deliberately blind to whether system audio is paused for inactivity: the
/// resume trigger is "sound detected", so a wedged tap can never deliver the
/// sound that would wake it — which is exactly how the ScreenCaptureKit bug
/// trapped a live session for 34 minutes.
#[derive(Debug)]
pub struct ZeroWatchdog {
    last_sound: Instant,
    delay: Duration,
}

impl ZeroWatchdog {
    /// A fresh generation is given the benefit of the doubt: the clock starts now.
    pub fn new(now: Instant) -> Self {
        Self {
            last_sound: now,
            delay: ZERO_WATCHDOG_INITIAL_DELAY,
        }
    }

    /// Any non-zero sample: the tap is provably alive, so the timer and the
    /// backoff both go back to the start.
    pub fn observe_sound(&mut self, now: Instant) {
        self.last_sound = now;
        self.delay = ZERO_WATCHDOG_INITIAL_DELAY;
    }

    /// Driven from the caller's existing tick. `true` means rebuild now.
    pub fn poll(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.last_sound) < self.delay {
            return false;
        }

        // The next window is measured from this rebuild, not from the last real
        // sound: a rebuild that does not cure the wedge must wait out the longer
        // delay before trying again.
        self.last_sound = now;
        self.delay = (self.delay * 2).min(ZERO_WATCHDOG_MAX_DELAY);
        true
    }

    /// How long the next trip will take. Exposed for the rebuild log line.
    pub fn delay(&self) -> Duration {
        self.delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    #[test]
    fn silence_below_the_threshold_is_not_a_wedge() {
        let start = Instant::now();
        let mut watchdog = ZeroWatchdog::new(start);

        assert!(!watchdog.poll(start));
        assert!(!watchdog.poll(start + secs(29)));
    }

    #[test]
    fn sustained_silence_rebuilds() {
        let start = Instant::now();
        let mut watchdog = ZeroWatchdog::new(start);

        assert!(watchdog.poll(start + secs(30)));
    }

    #[test]
    fn one_sound_sample_resets_the_countdown() {
        let start = Instant::now();
        let mut watchdog = ZeroWatchdog::new(start);

        assert!(!watchdog.poll(start + secs(29)));
        watchdog.observe_sound(start + secs(29));
        assert!(!watchdog.poll(start + secs(58)));
        assert!(watchdog.poll(start + secs(59)));
    }

    #[test]
    fn the_backoff_grows_and_stops_at_the_cap() {
        let start = Instant::now();
        let mut watchdog = ZeroWatchdog::new(start);

        let mut elapsed = secs(0);
        for expected in [30, 60, 120, 240, 480, 600, 600, 600] {
            assert_eq!(watchdog.delay(), secs(expected), "delay before trip");

            // One tick short of the delay is still not a wedge...
            assert!(!watchdog.poll(start + elapsed + secs(expected - 1)));
            // ...and reaching it is.
            elapsed += secs(expected);
            assert!(watchdog.poll(start + elapsed), "trip after {expected}s");
        }

        assert_eq!(watchdog.delay(), ZERO_WATCHDOG_MAX_DELAY);
    }

    #[test]
    fn the_backoff_resets_once_sound_returns() {
        let start = Instant::now();
        let mut watchdog = ZeroWatchdog::new(start);

        assert!(watchdog.poll(start + secs(30)));
        assert!(watchdog.poll(start + secs(90)));
        assert_eq!(watchdog.delay(), secs(120));

        watchdog.observe_sound(start + secs(100));
        assert_eq!(watchdog.delay(), ZERO_WATCHDOG_INITIAL_DELAY);
        assert!(!watchdog.poll(start + secs(129)));
        assert!(watchdog.poll(start + secs(130)));
    }

    // A day of real silence must not rebuild the tap more than the cap allows.
    #[test]
    fn true_silence_is_bounded_by_the_cap() {
        let start = Instant::now();
        let mut watchdog = ZeroWatchdog::new(start);

        let mut rebuilds = 0;
        for second in 0..(24 * 60 * 60) {
            if watchdog.poll(start + secs(second)) {
                rebuilds += 1;
            }
        }

        // Five while the backoff climbs (30s, 90s, 210s, 450s, 930s), then six
        // an hour at the cap.
        assert_eq!(rebuilds, 147);
    }
}
