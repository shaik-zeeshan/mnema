//! OS observers and system facts. Sleep/wake come from GPUI's
//! `on_system_sleep`/`on_system_wake` (the same NSWorkspace notifications the
//! Tauri app observes). Screens sleep/wake, focus/app changes and the Core
//! Graphics display callback have no GPUI equivalent with the flags capture
//! needs, so they use app-core's shared `os_notifiers`. Lock state stays
//! app-core's (`native_capture_system_idle::session_is_away`).

use std::sync::Arc;

use app_core::native_capture::{os_notifiers, system_wake};
use app_core::AppCore;
use gpui_kit::App;
use objc2_foundation::NSTimeZone;

/// `AppCore::start`'s `register_native_notifiers` step.
pub fn register_core_notifiers(core: &Arc<AppCore>) {
    os_notifiers::start_system_wake_notifier(core, false);
    os_notifiers::start_display_reconfiguration_notifier(core);
    os_notifiers::start_metadata_notifier(core);
}

/// The sleep/wake half, on GPUI's hooks.
pub fn observe_sleep_wake(core: &Arc<AppCore>, cx: &mut App) {
    let sleep_core = core.clone();
    cx.on_system_sleep(move |_| system_wake::handle_system_will_sleep(&sleep_core))
        .detach();
    let wake_core = core.clone();
    cx.on_system_wake(move |_| {
        system_wake::recover_screen_capture_after_system_wake(wake_core.clone())
    })
    .detach();
}

/// The Mac's current UTC offset in seconds, read natively (`time`'s
/// `current_local_offset` refuses in a multi-threaded process).
pub fn local_offset_seconds() -> i32 {
    NSTimeZone::localTimeZone().secondsFromGMT() as i32
}

pub fn local_offset() -> time::UtcOffset {
    time::UtcOffset::from_whole_seconds(local_offset_seconds()).unwrap_or(time::UtcOffset::UTC)
}

/// Unix ms of the next local midnight (Snooze for today).
pub fn next_local_midnight_ms() -> u64 {
    next_midnight_ms(super::now_unix_ms(), local_offset_seconds())
}

fn next_midnight_ms(now_ms: u64, offset_s: i32) -> u64 {
    const DAY_MS: i64 = 86_400_000;
    let local = now_ms as i64 + offset_s as i64 * 1000;
    let next = (local.div_euclid(DAY_MS) + 1) * DAY_MS;
    (next - offset_s as i64 * 1000) as u64
}

#[cfg(test)]
mod tests {
    #[test]
    fn next_midnight_is_local_and_within_a_day() {
        // 2026-11-04 14:32 IST (+05:30) → 2026-11-05 00:00 IST.
        let now = 1_793_782_920_000;
        let ist = 5 * 3600 + 1800;
        assert_eq!(super::next_midnight_ms(now, ist), 1_793_817_000_000);
        let midnight = super::next_local_midnight_ms();
        let now = super::super::now_unix_ms();
        assert!(midnight > now && midnight <= now + 25 * 3_600_000);
    }
}
