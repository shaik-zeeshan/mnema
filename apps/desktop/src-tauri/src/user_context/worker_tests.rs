use super::*;

#[test]
fn cold_store_window_reaches_back_initial_lookback() {
    let now = 10_000_000_000;
    let window = next_forward_window(now, None).expect("cold window");
    assert_eq!(window.0, now - INITIAL_LOOKBACK_MS);
    assert_eq!(window.1, (now - INITIAL_LOOKBACK_MS) + MAX_WINDOW_MS);
}

#[test]
fn resumes_from_last_covered_end() {
    let now = 10_000_000_000;
    let last_end = now - (5 * 60 * 1000);
    let window = next_forward_window(now, Some(last_end)).expect("forward window");
    assert_eq!(window.0, last_end);
    assert_eq!(window.1, now);
}

#[test]
fn waits_until_enough_new_time() {
    let now = 10_000_000_000;
    let last_end = now - (MIN_WINDOW_MS - 1);
    assert!(next_forward_window(now, Some(last_end)).is_none());
}

#[test]
fn clamps_long_gap_to_max_window() {
    let now = 10_000_000_000;
    let last_end = now - (5 * MAX_WINDOW_MS);
    let window = next_forward_window(now, Some(last_end)).expect("clamped window");
    assert_eq!(window.1 - window.0, MAX_WINDOW_MS);
}

// --- A: empty-window cursor jump -------------------------------------------

#[test]
fn empty_window_jump_target_leaps_gap_to_next_capture() {
    let end = 1_000;
    let now = 10_000;
    // A raw capture sits in the gap ahead: jump straight to it.
    let next = 5_000;
    assert_eq!(empty_window_jump_target(end, now, Some(next)), next);
}

#[test]
fn empty_window_jump_target_clamps_next_to_now() {
    let end = 1_000;
    let now = 4_000;
    // The next raw capture is beyond `now` (clock skew / racing capture):
    // clamp to `now`, never overshoot realtime.
    assert_eq!(empty_window_jump_target(end, now, Some(9_000)), now);
}

#[test]
fn empty_window_jump_target_holds_when_capture_inside_window() {
    let end = 5_000;
    let now = 10_000;
    // A raw capture sits within the just-read window (OCR pending): do NOT
    // leap past it — advance one window (stay at `end`).
    assert_eq!(empty_window_jump_target(end, now, Some(end)), end);
    assert_eq!(empty_window_jump_target(end, now, Some(end - 1)), end);
}

#[test]
fn empty_window_jump_target_none_jumps_to_now() {
    let end = 1_000;
    let now = 10_000;
    // No raw captures at/after `end`: safe to jump straight to `now`.
    assert_eq!(empty_window_jump_target(end, now, None), now);
}

#[test]
fn empty_window_jump_target_always_within_end_and_now() {
    let end = 2_000;
    let now = 8_000;
    for next in [None, Some(0), Some(2_000), Some(5_000), Some(99_999)] {
        let result = empty_window_jump_target(end, now, next);
        assert!(result >= end, "result {result} went backward before end {end}");
        assert!(result <= now, "result {result} overshot now {now}");
    }
}

// --- C: catch-up cadence ---------------------------------------------------

#[test]
fn is_catching_up_false_when_caught_up() {
    // Small backlog (within one window) => no catch-up cadence.
    assert!(!is_catching_up(true, MIN_WINDOW_MS));
    assert!(!is_catching_up(true, 0));
}

#[test]
fn is_catching_up_true_with_large_backlog_and_derived_window() {
    assert!(is_catching_up(true, CATCHUP_BACKLOG_MS + 1));
    assert!(is_catching_up(true, 10 * CATCHUP_BACKLOG_MS));
}

#[test]
fn is_catching_up_false_without_a_derived_window() {
    // Even a huge backlog does not trigger catch-up if the forward beat did
    // not actually cover a window this tick (e.g. it was idle/not-enough-yet).
    assert!(!is_catching_up(false, 10 * CATCHUP_BACKLOG_MS));
}

#[test]
fn empty_backfill_window_jumps_to_just_above_the_previous_capture() {
    // An older capture exists: coverage stops 1 ms above it, so the next
    // backward window [prev+1-30m, prev+1] still reads it.
    assert_eq!(backfill_jump_start(10_000, 0, Some(4_000)), 4_001);
    // Nothing older: jump straight to the floor (backfill complete).
    assert_eq!(backfill_jump_start(10_000, 2_000, None), 2_000);
    // A capture below the floor never drags coverage under it.
    assert_eq!(backfill_jump_start(10_000, 2_000, Some(500)), 2_000);
    // Never above the window's own start.
    assert_eq!(backfill_jump_start(10_000, 0, Some(10_000)), 10_000);
}

/// The worker ticks no faster than the Thorough interval, which is no shorter than
/// `MIN_WINDOW_MS`, so forward always has a window on a tick. Backfill must gate
/// on "caught up to realtime", not "no forward window", or it never runs.
#[test]
fn backfill_gate_is_open_after_a_caught_up_forward_window() {
    assert!(THOROUGH_INTERVAL.as_millis() as i64 >= MIN_WINDOW_MS);
    assert!(!is_catching_up(true, THOROUGH_INTERVAL.as_millis() as i64));
}

#[test]
fn is_catching_up_strictly_greater_than_threshold() {
    // Exactly at the threshold => not catching up (strictly greater).
    assert!(!is_catching_up(true, CATCHUP_BACKLOG_MS));
}

#[test]
fn cadence_never_distills_when_nothing_changed() {
    let now = 10_000_000_000;
    // Cold (no pass yet) and long overdue: still no pass without a change.
    assert!(!WorkerCadence::default().should_distill(now, false));
    let cadence = WorkerCadence {
        last_distillation_at_ms: Some(now - 10 * CONCLUSION_MIN_INTERVAL_MS),
        ..WorkerCadence::default()
    };
    assert!(!cadence.should_distill(now, false));
}

#[test]
fn cadence_distills_changes_at_most_once_per_interval() {
    let now = 10_000_000_000;
    // Cold: the first change distills right away.
    assert!(WorkerCadence::default().should_distill(now, true));
    let cadence = WorkerCadence {
        last_distillation_at_ms: Some(now),
        ..WorkerCadence::default()
    };
    // Changes keep arriving every tick, but the throttle holds for 30 minutes.
    assert!(!cadence.should_distill(now + THOROUGH_INTERVAL.as_millis() as i64, true));
    assert!(!cadence.should_distill(now + CONCLUSION_MIN_INTERVAL_MS - 1, true));
    assert!(cadence.should_distill(now + CONCLUSION_MIN_INTERVAL_MS, true));
}

#[test]
fn cadence_drains_a_backlog_back_to_back_then_throttles_and_failures_wait() {
    let now = 10_000_000_000;
    let next_tick = now + BALANCED_INTERVAL.as_millis() as i64;
    let mut cadence = WorkerCadence::default();

    // Backlog: the pass hit its cap, so the next tick distills without waiting.
    cadence.record_distillation(now, true, true);
    assert!(cadence.should_distill(next_tick, true));

    // Caught up: the next change waits out the full interval.
    cadence.record_distillation(next_tick, true, false);
    assert!(!cadence.should_distill(next_tick * 2 - now, true));
    assert!(cadence.should_distill(next_tick + CONCLUSION_MIN_INTERVAL_MS, true));

    // Failure: waits the full interval even with a backlog behind the watermark.
    cadence.record_distillation(now, false, true);
    assert!(!cadence.should_distill(next_tick, true));
    assert!(cadence.should_distill(now + CONCLUSION_MIN_INTERVAL_MS, true));
}

#[test]
fn cadence_decays_on_the_slowest_beat() {
    let mut cadence = WorkerCadence::default();
    // Below the M threshold => no decay yet.
    cadence.ticks_since_decay = CONFIDENCE_DECAY_EVERY_M_TICKS - 1;
    assert!(!cadence.should_decay());
    // At the threshold => decay.
    cadence.ticks_since_decay = CONFIDENCE_DECAY_EVERY_M_TICKS;
    assert!(cadence.should_decay());
}

#[test]
fn tier_paces_cloud_but_not_local() {
    assert_eq!(
        tick_interval(AiProviderKind::Anthropic, DerivationBudgetTier::Light),
        LIGHT_INTERVAL
    );
    assert_eq!(
        tick_interval(AiProviderKind::Anthropic, DerivationBudgetTier::Thorough),
        THOROUGH_INTERVAL
    );
    assert_eq!(
        tick_interval(AiProviderKind::Ollama, DerivationBudgetTier::Thorough),
        LOCAL_INTERVAL
    );
}

// --- #98 History Backfill window math --------------------------------------

#[test]
fn bounded_backfill_floor_is_now_minus_window_days() {
    let now = 10_000_000_000;
    // go-deeper OFF: floor is the bounded recent window, ignoring the earliest
    // capture even when there is much older history.
    let floor = backfill_floor_ms(now, 30, false, Some(0));
    assert_eq!(floor, now - 30 * 86_400_000);
}

#[test]
fn go_deeper_floor_reaches_earliest_capture() {
    let now = 10_000_000_000;
    let earliest = 1_234;
    // go-deeper ON: floor is the true earliest capture (all of history).
    assert_eq!(backfill_floor_ms(now, 30, true, Some(earliest)), earliest);
    // go-deeper ON with no captures collapses to `now` (backfill no-op).
    assert_eq!(backfill_floor_ms(now, 30, true, None), now);
}

#[test]
fn backfill_window_steps_one_window_older() {
    let oldest_covered = 10_000_000_000;
    let floor = oldest_covered - 10 * BACKFILL_WINDOW_MS;
    let (start, end) = next_backfill_window(floor, oldest_covered).expect("backfill window");
    // The window ends at the current trailing edge and reaches back exactly one
    // BACKFILL_WINDOW_MS (the floor is far below, so it does not clamp here).
    assert_eq!(end, oldest_covered);
    assert_eq!(start, oldest_covered - BACKFILL_WINDOW_MS);
}

#[test]
fn backfill_window_clamps_to_floor() {
    let oldest_covered = 10_000_000_000;
    // Floor is closer than one full window: the window start clamps to the floor.
    let floor = oldest_covered - (BACKFILL_WINDOW_MS / 2);
    let (start, end) = next_backfill_window(floor, oldest_covered).expect("clamped window");
    assert_eq!(start, floor);
    assert_eq!(end, oldest_covered);
}

#[test]
fn backfill_complete_when_coverage_reaches_floor() {
    let oldest_covered = 10_000_000_000;
    // Coverage already AT the floor → nothing to backfill.
    assert!(next_backfill_window(oldest_covered, oldest_covered).is_none());
    // Coverage already BELOW the floor (e.g. floor moved up) → nothing to do.
    assert!(next_backfill_window(oldest_covered + 1, oldest_covered).is_none());
}

#[test]
fn backfill_intensity_is_a_real_cloud_tier_knob() {
    // The named cloud tier is a real intensity knob (windows/tick), not just a
    // sleep: Light < Balanced < Thorough.
    assert_eq!(
        backfill_windows_per_tick(AiProviderKind::Anthropic, DerivationBudgetTier::Light),
        1
    );
    assert_eq!(
        backfill_windows_per_tick(AiProviderKind::Anthropic, DerivationBudgetTier::Balanced),
        2
    );
    assert_eq!(
        backfill_windows_per_tick(AiProviderKind::Anthropic, DerivationBudgetTier::Thorough),
        4
    );
    // Local ignores the tier entirely (fixed pacing): always one window/tick.
    for tier in [
        DerivationBudgetTier::Light,
        DerivationBudgetTier::Balanced,
        DerivationBudgetTier::Thorough,
    ] {
        assert_eq!(backfill_windows_per_tick(AiProviderKind::Ollama, tier), 1);
    }
}
