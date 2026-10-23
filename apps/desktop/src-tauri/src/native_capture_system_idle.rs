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

/// Whether the login session's screen is locked. `CGSSessionScreenIsLocked` is
/// only present in the session dictionary while locked.
#[cfg(target_os = "macos")]
pub(crate) fn session_screen_is_locked() -> bool {
    use cidre::{arc, cf};

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGSessionCopyCurrentDictionary() -> Option<arc::R<cf::Dictionary>>;
    }

    let Some(session) = (unsafe { CGSessionCopyCurrentDictionary() }) else {
        return false;
    };
    session
        .value(&cf::String::from_str("CGSSessionScreenIsLocked"))
        .is_some_and(|value| value.equal(cf::Boolean::value_true()))
}
