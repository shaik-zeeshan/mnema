#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

#[cfg(target_os = "windows")]
pub(crate) fn current_system_idle_ms() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    let mut last_input = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };

    if unsafe { GetLastInputInfo(&mut last_input) } == 0 {
        if INVALID_IDLE_READING_LOGGED
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            super::debug_log::log("failed to read Windows system idle time via GetLastInputInfo");
        }
        return None;
    }

    // LASTINPUTINFO::dwTime is a 32-bit tick value; subtract in that domain so
    // uptime wraparound matches the Win32 GetTickCount/GetLastInputInfo contract.
    let now_tick_ms = unsafe { GetTickCount64() };
    Some(u64::from((now_tick_ms as u32).wrapping_sub(last_input.dwTime)))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn current_system_idle_ms() -> Option<u64> {
    None
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::current_system_idle_ms;
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;

    #[test]
    fn windows_system_idle_reading_is_available_and_sane() {
        let idle_ms = current_system_idle_ms()
            .expect("Windows GetLastInputInfo should return a system idle reading");
        let uptime_ms = unsafe { GetTickCount64() };
        let max_sane_idle_ms = uptime_ms.min(u64::from(u32::MAX));

        assert!(
            idle_ms <= max_sane_idle_ms,
            "idle reading {idle_ms}ms must not exceed uptime/window tick range {max_sane_idle_ms}ms"
        );
    }
}
