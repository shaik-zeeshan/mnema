//! The OS observers capture needs, registered by each host from
//! `AppCore::start`'s `register_native_notifiers` callback: NSWorkspace
//! sleep/wake and screens sleep/wake, focus/app changes (privacy filter
//! refresh), and the Core Graphics display-reconfiguration callback. Their
//! guards live in app-core states, so they last for the process.

use std::sync::Arc;

use crate::AppCore;

/// Focus, Space and app launch/quit changes re-check the privacy filter.
#[cfg(target_os = "macos")]
pub fn start_metadata_notifier(core: &Arc<AppCore>) {
    use crate::native_capture::privacy::PrivacyRefreshReason;
    use cidre::ns;

    let mut center = ns::Workspace::shared().notification_center();
    let mut guards = Vec::new();
    for (notification, reason) in [
        (
            ns::workspace::notification::did_activate_app(),
            PrivacyRefreshReason::WorkspaceFocusChanged,
        ),
        (
            ns::workspace::notification::did_deactivate_app(),
            PrivacyRefreshReason::WorkspaceFocusChanged,
        ),
        (
            ns::workspace::notification::active_space_did_change(),
            PrivacyRefreshReason::WorkspaceFocusChanged,
        ),
        (
            ns::workspace::notification::did_launch_app(),
            PrivacyRefreshReason::WorkspaceAppChanged,
        ),
        (
            ns::workspace::notification::did_terminate_app(),
            PrivacyRefreshReason::WorkspaceAppChanged,
        ),
        (
            ns::workspace::notification::did_hide_app(),
            PrivacyRefreshReason::WorkspaceAppChanged,
        ),
        (
            ns::workspace::notification::did_unhide_app(),
            PrivacyRefreshReason::WorkspaceAppChanged,
        ),
    ] {
        guards.push(center.add_observer_guard(notification, None, None, {
            let core = core.clone();
            move |_notification| {
                crate::native_capture::privacy::refresh::request_privacy_filter_refresh(
                    &core, reason,
                );
            }
        }));
    }

    core.state::<crate::native_capture::metadata::MetadataNotifierState>()
        .replace(guards);
}

#[cfg(not(target_os = "macos"))]
pub fn start_metadata_notifier(_core: &Arc<AppCore>) {}

/// Screens sleep/wake, plus NSWorkspace will-sleep/did-wake when
/// `system_sleep_wake` (a host with its own sleep/wake hooks, like GPUI's
/// `on_system_sleep`/`on_system_wake`, passes `false` and calls
/// [`system_wake::handle_system_will_sleep`] /
/// [`system_wake::recover_screen_capture_after_system_wake`] from them).
///
/// [`system_wake::handle_system_will_sleep`]: crate::native_capture::system_wake::handle_system_will_sleep
/// [`system_wake::recover_screen_capture_after_system_wake`]: crate::native_capture::system_wake::recover_screen_capture_after_system_wake
#[cfg(target_os = "macos")]
pub fn start_system_wake_notifier(core: &Arc<AppCore>, system_sleep_wake: bool) {
    use crate::native_capture::system_wake::{self, SystemWakeNotifierState};
    use cidre::ns;

    let mut center = ns::Workspace::shared().notification_center();
    let mut guards = Vec::new();
    if system_sleep_wake {
        guards.push(center.add_observer_guard(
            ns::workspace::notification::will_sleep(),
            None,
            None,
            {
                let core = core.clone();
                move |_notification| {
                    system_wake::handle_system_will_sleep(&core);
                }
            },
        ));
        guards.push(center.add_observer_guard(
            ns::workspace::notification::did_wake(),
            None,
            None,
            {
                let core = core.clone();
                move |_notification| {
                    system_wake::recover_screen_capture_after_system_wake(core.clone());
                }
            },
        ));
    }
    guards.push(center.add_observer_guard(
        ns::workspace::notification::screens_did_sleep(),
        None,
        None,
        {
            let core = core.clone();
            move |_notification| {
                system_wake::handle_screens_did_sleep(&core);
            }
        },
    ));
    guards.push(center.add_observer_guard(
        ns::workspace::notification::screens_did_wake(),
        None,
        None,
        {
            let core = core.clone();
            move |_notification| {
                system_wake::handle_screens_did_wake(&core);
            }
        },
    ));

    let notifier_state = core.state::<SystemWakeNotifierState>();
    let mut notifier_slot = notifier_state
        .0
        .lock()
        .expect("system wake notifier state poisoned");
    notifier_slot.clear();
    notifier_slot.extend(guards);
}

#[cfg(not(target_os = "macos"))]
pub fn start_system_wake_notifier(_core: &Arc<AppCore>, _system_sleep_wake: bool) {}

// `NSWorkspaceDidWake` is only posted on a *full* wake; dark wakes, Power Nap,
// and "Wake from Deep Idle" never post it. The display reconfiguration callback
// is the polling-free re-arm signal (`crate::native_capture::system_wake`).
#[cfg(target_os = "macos")]
pub fn start_display_reconfiguration_notifier(core: &Arc<AppCore>) {
    use crate::native_capture::system_wake::{
        display_reconfiguration_callback, set_display_reconfiguration_recovery_core,
        DisplayReconfigurationCallbackGuard, DisplayReconfigurationNotifierState,
    };
    use core_graphics::base::{kCGErrorSuccess, CGError};

    // The C callback can't capture state, so stash the core where it can reach
    // it. Set before registering so no early callback races a missing core.
    set_display_reconfiguration_recovery_core(core);

    let error: CGError = unsafe {
        core_graphics::display::CGDisplayRegisterReconfigurationCallback(
            display_reconfiguration_callback,
            std::ptr::null(),
        )
    };
    if error != kCGErrorSuccess {
        crate::debug_log::log_warn(format!(
            "failed to register display reconfiguration callback for wake recovery (CGError {error}); \
             relying on NSWorkspaceDidWake fallback"
        ));
        return;
    }

    let notifier_state = core.state::<DisplayReconfigurationNotifierState>();
    let mut slot = notifier_state
        .0
        .lock()
        .expect("display reconfiguration notifier state poisoned");
    *slot = Some(DisplayReconfigurationCallbackGuard);
}

#[cfg(not(target_os = "macos"))]
pub fn start_display_reconfiguration_notifier(_core: &Arc<AppCore>) {}
