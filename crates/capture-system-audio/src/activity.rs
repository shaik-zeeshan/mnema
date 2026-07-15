//! The system-audio activity signal the inactivity machinery reads.
//!
//! Lifted out of `capture-screen` with the tap swap (ADR 0052). Nothing about the
//! numbers changed — same RMS probe, same window-peak accumulator, same
//! sentinels — but the source did: levels now come from the tap's own
//! deliveries, so the signal keeps flowing through display sleep and outlives any
//! screen stream. That is what lets system audio pause and resume for inactivity
//! with no screen at all.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

static LAST_ACTIVITY_LEVEL_BITS: AtomicU32 = AtomicU32::new(0);
static LAST_ACTIVITY_MONOTONIC_MS: AtomicU64 = AtomicU64::new(0);
static LAST_ACTIVITY_UNIX_MS: AtomicU64 = AtomicU64::new(0);
static ACTIVITY_WINDOW_PEAK_LEVEL_BITS: AtomicU32 = AtomicU32::new(0);
static ACTIVITY_WINDOW_SAMPLE_COUNT: AtomicU64 = AtomicU64::new(0);
static SOUND_OBSERVED: AtomicBool = AtomicBool::new(false);

fn monotonic_epoch() -> &'static Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now)
}

fn now_monotonic_marker_ms() -> u64 {
    // 0 stays reserved as the "no sample observed yet" sentinel.
    (monotonic_epoch().elapsed().as_millis().min(u128::from(u64::MAX)) as u64).saturating_add(1)
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Records one delivery's level. Zero-level samples count: they are what proves
/// the tap is still delivering, and the inactivity policy applies its own
/// threshold to the peak.
pub(crate) fn record_delivery(level: f32) {
    store_activity(level, now_monotonic_marker_ms(), now_unix_ms());
}

fn store_activity(level: f32, now_monotonic_ms: u64, now_unix_ms: u64) {
    let level = level.clamp(0.0, 1.0);
    LAST_ACTIVITY_LEVEL_BITS.store(level.to_bits(), Ordering::Relaxed);
    LAST_ACTIVITY_MONOTONIC_MS.store(now_monotonic_ms, Ordering::Relaxed);
    LAST_ACTIVITY_UNIX_MS.store(now_unix_ms, Ordering::Relaxed);
    if level > 0.0 {
        SOUND_OBSERVED.store(true, Ordering::Relaxed);
    }
    record_window_peak(level);
}

fn record_window_peak(level: f32) {
    ACTIVITY_WINDOW_SAMPLE_COUNT.fetch_add(1, Ordering::Relaxed);

    let level_bits = level.to_bits();
    let mut observed_bits = ACTIVITY_WINDOW_PEAK_LEVEL_BITS.load(Ordering::Relaxed);
    while f32::from_bits(observed_bits) < level {
        match ACTIVITY_WINDOW_PEAK_LEVEL_BITS.compare_exchange_weak(
            observed_bits,
            level_bits,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(next_bits) => observed_bits = next_bits,
        }
    }
}

pub fn last_system_audio_activity_unix_ms() -> Option<u64> {
    let ts = LAST_ACTIVITY_UNIX_MS.load(Ordering::Relaxed);
    (ts > 0).then_some(ts)
}

pub fn system_audio_activity_idle_ms() -> Option<u64> {
    let ts = LAST_ACTIVITY_MONOTONIC_MS.load(Ordering::Relaxed);
    (ts > 0).then_some(now_monotonic_marker_ms().saturating_sub(ts))
}

pub fn system_audio_activity_level() -> Option<f32> {
    last_system_audio_activity_unix_ms()
        .map(|_| f32::from_bits(LAST_ACTIVITY_LEVEL_BITS.load(Ordering::Relaxed)))
}

pub fn take_system_audio_activity_window_peak_level() -> Option<f32> {
    let sample_count = ACTIVITY_WINDOW_SAMPLE_COUNT.swap(0, Ordering::Relaxed);
    let level_bits = ACTIVITY_WINDOW_PEAK_LEVEL_BITS.swap(0, Ordering::Relaxed);
    (sample_count > 0).then_some(f32::from_bits(level_bits))
}

pub fn peek_system_audio_activity_window_peak_level() -> Option<f32> {
    let sample_count = ACTIVITY_WINDOW_SAMPLE_COUNT.load(Ordering::Relaxed);
    let level_bits = ACTIVITY_WINDOW_PEAK_LEVEL_BITS.load(Ordering::Relaxed);
    (sample_count > 0).then_some(f32::from_bits(level_bits))
}

/// Whether this session's tap has ever delivered a sound — the one fact the
/// permission heuristic is built from (ADR 0052). Distinct from the readings
/// above, which are all "how loud, how recently": a denied tap keeps those
/// flowing (zeros, on schedule) and this is what it can never do.
pub fn system_audio_sound_observed() -> bool {
    SOUND_OBSERVED.load(Ordering::Relaxed)
}

pub fn record_system_audio_activity_for_tests(level: f32, now_monotonic_ms: u64, now_unix_ms: u64) {
    store_activity(level, now_monotonic_ms, now_unix_ms);
}

/// Clears the signal so a new recording session cannot inherit the last one's
/// idle reading. The sibling of `capture_microphone::reset_last_microphone_activity_unix_ms`.
pub fn reset_system_audio_activity() {
    LAST_ACTIVITY_LEVEL_BITS.store(0, Ordering::Relaxed);
    LAST_ACTIVITY_MONOTONIC_MS.store(0, Ordering::Relaxed);
    LAST_ACTIVITY_UNIX_MS.store(0, Ordering::Relaxed);
    ACTIVITY_WINDOW_PEAK_LEVEL_BITS.store(0, Ordering::Relaxed);
    ACTIVITY_WINDOW_SAMPLE_COUNT.store(0, Ordering::Relaxed);
    // Session-scoped like the rest: the *persisted* evidence is what remembers
    // across sessions, and it only ever strengthens, so clearing here cannot
    // un-prove a grant.
    SOUND_OBSERVED.store(false, Ordering::Relaxed);
}

// Moved here with the signal itself (ADR 0052); they read the same global state,
// so they serialize against each other.
#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use super::*;

    fn activity_state_test_guard() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn reset_clears_every_reading() {
        let _guard = activity_state_test_guard();
        reset_system_audio_activity();

        store_activity(0.6, 10_000, 20_000);

        assert_eq!(last_system_audio_activity_unix_ms(), Some(20_000));
        assert_eq!(system_audio_activity_level(), Some(0.6));

        reset_system_audio_activity();

        assert_eq!(last_system_audio_activity_unix_ms(), None);
        assert_eq!(system_audio_activity_level(), None);
        assert_eq!(system_audio_activity_idle_ms(), None);
    }

    // The inactivity policy polls once a second; a peak that decayed to the latest
    // sample would miss every burst shorter than the poll.
    #[test]
    fn window_peak_tracks_the_max_until_taken() {
        let _guard = activity_state_test_guard();
        reset_system_audio_activity();

        store_activity(0.02, 10_000, 20_000);
        store_activity(0.60, 10_010, 20_010);
        store_activity(0.08, 10_020, 20_020);

        assert_eq!(take_system_audio_activity_window_peak_level(), Some(0.60));
        assert_eq!(take_system_audio_activity_window_peak_level(), None);
        assert_eq!(system_audio_activity_level(), Some(0.08));

        reset_system_audio_activity();
    }

    #[test]
    fn peek_preserves_the_window_peak_until_taken() {
        let _guard = activity_state_test_guard();
        reset_system_audio_activity();

        store_activity(0.15, 10_000, 20_000);
        store_activity(0.70, 10_010, 20_010);

        assert_eq!(peek_system_audio_activity_window_peak_level(), Some(0.70));
        assert_eq!(peek_system_audio_activity_window_peak_level(), Some(0.70));
        assert_eq!(take_system_audio_activity_window_peak_level(), Some(0.70));
        assert_eq!(peek_system_audio_activity_window_peak_level(), None);

        reset_system_audio_activity();
    }

    // Zero-level deliveries still count: they are what proves the tap is alive,
    // and the inactivity policy applies its own threshold to the peak.
    #[test]
    fn a_silent_delivery_is_still_a_delivery() {
        let _guard = activity_state_test_guard();
        reset_system_audio_activity();

        record_delivery(0.0);

        assert!(last_system_audio_activity_unix_ms().is_some());
        assert_eq!(take_system_audio_activity_window_peak_level(), Some(0.0));

        reset_system_audio_activity();
    }
}
