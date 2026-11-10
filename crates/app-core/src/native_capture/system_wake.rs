//! System sleep/wake and display reconfiguration: the handlers the host's
//! native notifiers call (NSWorkspace sleep/wake, screens sleep/wake, the Core
//! Graphics display-reconfiguration callback) and the states holding their
//! registrations. The registrations themselves stay host-side.

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "macos")]
use std::sync::{Arc, Mutex, OnceLock, Weak};
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
use capture_types::CaptureErrorResponse;

use crate::debug_log;
use crate::native_capture::lifecycle::NativeCaptureState;
use crate::native_capture::log_format::{
    format_optional_capture_source_flags, runtime_log_session_id,
};
use crate::{AppCore, CoreEvent, WindowSignal};

#[cfg(target_os = "macos")]
#[derive(Default)]
pub struct SystemWakeNotifierState(pub std::sync::Mutex<Vec<cidre::ns::NotificationGuard>>);

#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct SystemWakeNotifierState(pub std::sync::Mutex<Vec<()>>);

// Holds the registration for the Core Graphics display-reconfiguration callback
// used as the primary, polling-free wake-recovery signal (dark/deep-idle wakes
// and monitor reconnects power the panel back on without posting
// `NSWorkspaceDidWake`). The guard removes the callback on drop for symmetry.
#[cfg(target_os = "macos")]
#[derive(Default)]
pub struct DisplayReconfigurationNotifierState(
    // The guard is held only so its `Drop` deregisters the callback on teardown.
    #[allow(dead_code)] pub std::sync::Mutex<Option<DisplayReconfigurationCallbackGuard>>,
);

#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct DisplayReconfigurationNotifierState(pub std::sync::Mutex<Option<()>>);

#[cfg(target_os = "macos")]
// ScreenCaptureKit can report no displays for several seconds after macOS
// wake; keep recovery alive long enough that the backend does not depend on a
// later frontend permissions poll to restart capture.
pub(crate) const SYSTEM_WAKE_RECOVERY_RETRY_DELAYS_MS: &[u64] = &[
    500, 1_500, 3_000, 5_000, 10_000, 10_000, 10_000, 10_000, 10_000,
];

fn emit_system_did_wake(core: &AppCore) {
    // Every wake path funnels through here, so this is where hidden webviews
    // come back — a missed `screens_did_wake` must never leave the app blank.
    core.host()
        .window_signal(crate::WindowSignal::HideAllViews(false));
    core.host().emit(CoreEvent::SystemDidWake);
}

#[cfg(target_os = "macos")]
pub fn handle_system_will_sleep(core: &AppCore) {
    let state = core.state::<NativeCaptureState>();
    let mut runtime = match state.lock() {
        Ok(runtime) => runtime,
        Err(_) => return,
    };

    if runtime.handle_system_will_sleep() {
        let runtime_state = runtime.runtime();
        debug_log::log_info(format!(
            "marked screen capture inactive for system sleep (session_id='{}', requested_sources={})",
            runtime_log_session_id(runtime_state),
            format_optional_capture_source_flags(runtime_state.requested_sources.as_ref())
        ));
    }
}

#[cfg(target_os = "macos")]
fn recover_screen_capture_after_system_wake_once(
    core: &Arc<AppCore>,
) -> Result<bool, CaptureErrorResponse> {
    let state = core.state::<NativeCaptureState>();
    let mut runtime = state.lock().map_err(|_| CaptureErrorResponse {
        code: "native_capture_state_poisoned".to_string(),
        message: "Native capture state is unavailable while recovering after system wake"
            .to_string(),
    })?;

    let outcome = runtime.recover_after_wake(Some(core));
    let runtime_state = runtime.runtime();
    match &outcome {
        Ok(true) => {
            debug_log::log_info(format!(
                "recovered screen capture after system wake (session_id='{}', requested_sources={})",
                runtime_log_session_id(runtime_state),
                format_optional_capture_source_flags(runtime_state.requested_sources.as_ref())
            ));
        }
        Ok(false) => {}
        Err(error) => {
            debug_log::log_error(format!(
                "failed to recover screen capture after system wake (session_id='{}', requested_sources={}): [{}] {}",
                runtime_log_session_id(runtime_state),
                format_optional_capture_source_flags(runtime_state.requested_sources.as_ref()),
                error.code,
                error.message
            ));
        }
    }

    outcome
}

#[cfg(target_os = "macos")]
pub(crate) fn is_recover_after_wake_retryable_error(error: &CaptureErrorResponse) -> bool {
    matches!(
        error.code.as_str(),
        "capture_stream_start_failed"
            | "capture_stream_start_timeout"
            | "capture_shareable_content_failed"
            | "capture_shareable_content_timeout"
            | "capture_shareable_content_unavailable"
            | "capture_display_unavailable"
    ) || error
        .message
        .contains("Failed to find any displays or windows")
        || error.message.contains("code: -3815")
}

#[cfg(target_os = "macos")]
fn log_scheduled_system_wake_recovery_retry(error: &CaptureErrorResponse, delay_ms: u64) {
    debug_log::log_warn(format!(
        "screen capture wake recovery hit a transient ScreenCaptureKit error; retrying in {}ms: [{}] {}",
        delay_ms, error.code, error.message
    ));
}

#[cfg(target_os = "macos")]
fn system_wake_recovery_in_progress() -> &'static AtomicBool {
    static IN_PROGRESS: OnceLock<AtomicBool> = OnceLock::new();
    IN_PROGRESS.get_or_init(|| AtomicBool::new(false))
}

#[cfg(target_os = "macos")]
fn begin_system_wake_recovery() -> bool {
    system_wake_recovery_in_progress()
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

#[cfg(target_os = "macos")]
fn finish_system_wake_recovery() {
    system_wake_recovery_in_progress().store(false, Ordering::Release);
}

#[cfg(target_os = "macos")]
fn retry_screen_capture_recovery_after_system_wake(
    core: Arc<AppCore>,
    mut last_error: CaptureErrorResponse,
) {
    std::thread::spawn(move || {
        for delay_ms in SYSTEM_WAKE_RECOVERY_RETRY_DELAYS_MS {
            log_scheduled_system_wake_recovery_retry(&last_error, *delay_ms);
            std::thread::sleep(Duration::from_millis(*delay_ms));

            match recover_screen_capture_after_system_wake_once(&core) {
                Ok(_) => {
                    finish_system_wake_recovery();
                    emit_system_did_wake(&core);
                    return;
                }
                Err(error) if is_recover_after_wake_retryable_error(&error) => {
                    last_error = error;
                }
                Err(_) => {
                    finish_system_wake_recovery();
                    emit_system_did_wake(&core);
                    return;
                }
            }
        }

        finish_system_wake_recovery();
        emit_system_did_wake(&core);
    });
}

#[cfg(target_os = "macos")]
pub fn recover_screen_capture_after_system_wake(core: Arc<AppCore>) {
    // A launch that found no display starts recording now (ADR 0021).
    crate::native_capture::auto_start::retry_pending_auto_start(&core);
    if !begin_system_wake_recovery() {
        return;
    }

    match recover_screen_capture_after_system_wake_once(&core) {
        Ok(_) => {
            finish_system_wake_recovery();
            emit_system_did_wake(&core);
        }
        Err(error) if is_recover_after_wake_retryable_error(&error) => {
            retry_screen_capture_recovery_after_system_wake(core, error);
        }
        Err(_) => {
            finish_system_wake_recovery();
            emit_system_did_wake(&core);
        }
    }
}

#[cfg(target_os = "macos")]
pub fn recover_screen_capture_after_possible_missed_wake(core: Arc<AppCore>) {
    let state = core.state::<NativeCaptureState>();
    let should_recover = state
        .lock()
        .map(|runtime| runtime.should_attempt_recovery_after_possible_wake())
        .unwrap_or(false);

    if !should_recover {
        return;
    }

    debug_log::log_info(
        "attempting screen capture recovery during session resync after possible missed system wake notification"
            .to_string(),
    );
    recover_screen_capture_after_system_wake(core);
}

// `NSWorkspaceDidWake` is only posted on a *full* wake; dark wakes, Power Nap,
// and "Wake from Deep Idle" never post it, so capture would silently stay
// paused until the next frontend permissions poll. The display panel powering
// back up *does* drive a Core Graphics display reconfiguration, which we listen
// on as the definitive, polling-free re-arm signal. This also covers external
// monitor disconnect/reconnect — the other half of ADR 0021's
// "display-unavailable as transient liveness".
#[cfg(target_os = "macos")]
fn display_reconfiguration_recovery_core() -> &'static Mutex<Option<Weak<AppCore>>> {
    static CORE: OnceLock<Mutex<Option<Weak<AppCore>>>> = OnceLock::new();
    CORE.get_or_init(|| Mutex::new(None))
}

/// The C callback can't capture state, so the host stashes the core where it
/// can reach it, before registering [`display_reconfiguration_callback`] so no
/// early callback races a missing core.
#[cfg(target_os = "macos")]
pub fn set_display_reconfiguration_recovery_core(core: &Arc<AppCore>) {
    if let Ok(mut slot) = display_reconfiguration_recovery_core().lock() {
        *slot = Some(Arc::downgrade(core));
    }
}

// Decide whether a reconfiguration callback's flags represent a display coming
// (back) online at the *end* of a configuration pass — the moment recovery
// should fire. We deliberately ignore the begin-configuration notification (the
// pre-change half of every begin/end pair) and pure offline transitions
// (remove/disable with nothing bringing a display online), so recovery runs
// exactly once per reconfiguration and never on a display going away.
#[cfg(target_os = "macos")]
pub(crate) fn display_reconfiguration_flags_indicate_display_online(flags: u32) -> bool {
    use core_graphics::display::CGDisplayChangeSummaryFlags as F;

    let flags = F::from_bits_retain(flags);

    // The begin-configuration notification is the "about to change" half; wait
    // for the matching end notification (begin flag clear) before re-arming.
    if flags.contains(F::kCGDisplayBeginConfigurationFlag) {
        return false;
    }

    // A display is now present/active if it was added, enabled, became main, or
    // (re)acquired a mode. A reconfiguration that only removes/disables a
    // display is a display going away, not a wake — skip it.
    flags.intersects(
        F::kCGDisplayAddFlag
            | F::kCGDisplayEnabledFlag
            | F::kCGDisplaySetMainFlag
            | F::kCGDisplaySetModeFlag,
    )
}

#[cfg(target_os = "macos")]
pub unsafe extern "C" fn display_reconfiguration_callback(
    _display: core_graphics::display::CGDirectDisplayID,
    flags: u32,
    _user_info: *const std::ffi::c_void,
) {
    use core_graphics::display::CGDisplayChangeSummaryFlags as F;

    // Only act on the end-configuration half of each begin/end pair.
    if F::from_bits_retain(flags).contains(F::kCGDisplayBeginConfigurationFlag) {
        return;
    }

    let core = display_reconfiguration_recovery_core()
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().and_then(Weak::upgrade));
    let Some(core) = core else {
        return;
    };

    // Every end-of-configuration re-decides every window's parked view: a
    // display coming back must reveal the windows on it even when the wake
    // recovery below is already running (it returns early) and no window event
    // fires, and one going away hides the windows it stranded right away.
    // The host hops to the main thread itself.
    core.host().window_signal(WindowSignal::RefreshParkedViews);

    if display_reconfiguration_flags_indicate_display_online(flags) {
        // Funnel through the shared recovery path: it is atomic-guarded
        // (`begin/finish_system_wake_recovery`) and re-checks `!session_is_live`,
        // so firing alongside the `NSWorkspaceDidWake` fallback is idempotent.
        recover_screen_capture_after_system_wake(core);
    }
}

#[cfg(target_os = "macos")]
pub struct DisplayReconfigurationCallbackGuard;

#[cfg(target_os = "macos")]
impl Drop for DisplayReconfigurationCallbackGuard {
    fn drop(&mut self) {
        unsafe {
            core_graphics::display::CGDisplayRemoveReconfigurationCallback(
                display_reconfiguration_callback,
                std::ptr::null(),
            );
        }
    }
}

/// The screens went to sleep. Hide the webview VIEWS (not windows) while no
/// display is lit: the lock/sleep-entry transition otherwise repaints the whole
/// layer tree into permanently stranded IOSurfaces, and a hidden view is the one
/// state whose backing stores WebKit marks volatile.
pub fn handle_screens_did_sleep(core: &AppCore) {
    core.host().window_signal(WindowSignal::HideAllViews(true));
    core.host().emit(CoreEvent::ScreensDidSleep);
}

pub fn handle_screens_did_wake(core: &std::sync::Arc<AppCore>) {
    core.host().window_signal(WindowSignal::HideAllViews(false));
    core.host().emit(CoreEvent::ScreensDidWake);
    crate::native_capture::auto_start::retry_pending_auto_start(core);
}
