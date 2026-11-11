//! What GPUI doesn't give us (BUILD-PROMPT §3.5): the tray, global shortcuts,
//! deep links, the Dock, the graceful quit, the native reminder, window chrome,
//! dialogs and the OS observers, plus the objc2 NSWindow helpers they share.

pub mod app_menu;
pub mod audio_player;
pub mod chrome;
pub mod deep_links;
pub mod dialogs;
pub mod dock;
pub mod hotkeys;
pub mod notification;
pub mod locale;
pub mod quit;
pub mod single_instance;
pub mod sys;
pub mod tray;

use std::sync::Arc;

use app_core::AppCore;
use capture_types::CaptureErrorResponse;
use gpui_kit::Window;
use objc2::msg_send;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSWindow, NSWindowSharingType};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The NSWindow behind a GPUI window. GPUI's handle is the content NSView.
pub fn ns_window(window: &Window) -> Option<&NSWindow> {
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    // SAFETY: GPUI's AppKit handle is its live content view; `-window` returns the
    // NSWindow that owns it, alive as long as the GPUI `Window` we borrow.
    unsafe {
        let view = handle.ns_view.as_ptr().cast::<AnyObject>();
        let ns_window: *mut NSWindow = msg_send![view, window];
        ns_window.as_ref()
    }
}

/// Show without becoming key or activating the app (the permission helper must
/// never take focus from System Settings).
pub fn order_front_regardless(window: &Window) {
    if let Some(ns_window) = ns_window(window) {
        ns_window.orderFrontRegardless();
    }
}

/// Hide the window without closing it (the main window's close = hide).
pub fn order_out(window: &Window) {
    if let Some(ns_window) = ns_window(window) {
        ns_window.orderOut(None);
    }
}

pub fn is_visible(window: &Window) -> bool {
    ns_window(window).is_some_and(|w| w.isVisible())
}

/// Keep above normal windows (the developer window's pin and mini mode).
pub fn set_always_on_top(window: &Window, on: bool) {
    if let Some(ns_window) = ns_window(window) {
        // NSFloatingWindowLevel = 3, NSNormalWindowLevel = 0.
        ns_window.setLevel(if on { 3 } else { 0 });
    }
}

/// The content min size, changed at runtime (mini mode lowers it before shrinking).
pub fn set_min_size(window: &Window, w: f32, h: f32) {
    if let Some(ns_window) = ns_window(window) {
        ns_window.setContentMinSize(objc2_foundation::NSSize::new(w as f64, h as f64));
    }
}

/// Mnema never records itself: `none`, or `read-only` with developer options
/// (so screenshots of the app work). Must run before the window's first show.
pub fn apply_sharing_type(window: &Window, developer_options: bool) {
    if let Some(ns_window) = ns_window(window) {
        ns_window.setSharingType(if developer_options {
            NSWindowSharingType::ReadOnly
        } else {
            NSWindowSharingType::None
        });
    }
}

/// Read before onboarding-gated work (tray menu, shortcuts).
pub fn onboarding_complete(core: &AppCore) -> bool {
    let store = core.state::<app_core::onboarding_state::OnboardingStateStore>();
    app_core::onboarding_state::current_onboarding_state(core, &store).is_complete()
}

/// Developer options, read live (a missing state reads as off: windows stay excluded).
pub fn developer_options(core: &AppCore) -> bool {
    core.try_state::<app_core::recording_settings::RecordingSettingsState>()
        .is_some_and(|s| {
            app_core::recording_settings::current_recording_settings(&s).developer_options_enabled
        })
}

pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// A capture command's failure: the dialog title and the error.
pub type CaptureResult = Result<(), (&'static str, CaptureErrorResponse)>;

/// The bell row a failed capture command leaves.
pub const CAPTURE_COMMAND_FAILED_ID: &str = "capture_command_failed";

/// A capture command failed: it must reach the user, never just the log. Always
/// a bell row carrying the error's message; `dialog` adds the "Recording error"
/// alert the Svelte controls show (not for a global shortcut pressed in another app).
pub fn report_capture_error(
    core: &AppCore,
    title: &str,
    error: &CaptureErrorResponse,
    dialog: bool,
) {
    log::error!("{title}: [{}]", error.code);
    if core
        .try_state::<app_core::native_capture::app_notifications::AppNotificationsState>()
        .is_some()
    {
        app_core::native_capture::app_notifications::push_error_app_notification(
            core,
            CAPTURE_COMMAND_FAILED_ID,
            title,
            &error.message,
            None,
            now_unix_ms(),
        );
    }
    if dialog {
        core.host().message(app_core::MessageDialog {
            title: "Recording error".into(),
            message: error.message.clone(),
            kind: app_core::DialogKind::Error,
        });
    }
}

/// Start or stop recording. Blocks (streams open/flush): call off the main thread.
pub fn toggle_recording(core: &Arc<AppCore>, origin: &str) -> CaptureResult {
    use app_core::native_capture::control;
    if control::current_native_capture_session(core).is_running {
        control::stop_native_capture_from_core(core)
            .map(|_| ())
            .map_err(|e| ("Recording could not stop", e))
    } else {
        control::start_native_capture_from_core(origin, core)
            .map(|_| ())
            .map_err(|e| ("Recording could not start", e))
    }
}

/// Pause or resume a running session. Blocks: call off the main thread.
pub fn toggle_pause(core: &Arc<AppCore>) -> CaptureResult {
    use app_core::native_capture::control;
    let session = control::current_native_capture_session(core);
    if !session.is_running {
        return Ok(());
    }
    if session.is_user_paused {
        control::resume_native_capture_from_core(core)
            .map(|_| ())
            .map_err(|e| ("Recording could not resume", e))
    } else {
        control::pause_native_capture_from_core(core)
            .map(|_| ())
            .map_err(|e| ("Recording could not pause", e))
    }
}
