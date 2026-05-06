#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_os = "macos")]
static INVALID_IDLE_READING_LOGGED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
pub(crate) fn current_system_idle_ms() -> Option<u64> {
    use core_graphics::event::CGEventType;
    use core_graphics::event_source::CGEventSourceStateID;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(
            state_id: CGEventSourceStateID,
            event_type: CGEventType,
        ) -> f64;
    }

    let seconds = unsafe {
        CGEventSourceSecondsSinceLastEventType(
            CGEventSourceStateID::CombinedSessionState,
            // CoreGraphics uses `kCGAnyInputEventType` (-1) for system-wide user input idle time.
            // In `core-graphics`, that value is represented by `TapDisabledByUserInput`.
            CGEventType::TapDisabledByUserInput,
        )
    };

    let max_seconds = (u64::MAX as f64) / 1000.0;
    if !seconds.is_finite() || seconds.is_sign_negative() || seconds > max_seconds {
        if INVALID_IDLE_READING_LOGGED
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            super::debug_log::log(format!(
                "received invalid system idle reading from CoreGraphics: {seconds}"
            ));
        }
        return None;
    }

    Some((seconds * 1000.0) as u64)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn current_system_idle_ms() -> Option<u64> {
    None
}
