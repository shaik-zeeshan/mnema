//! Capture auto-start at launch, and its retry when the launch found no display
//! (asleep or locked): ADR 0021's display-unavailable is transient liveness, so
//! a start refused for that alone waits for the display to return instead of
//! giving up. The display-return signals (did-wake, screens-did-wake, a display
//! coming online, the focus permissions poll) call [`retry_pending_auto_start`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use capture_types::CaptureErrorResponse;

use crate::debug_log;
use crate::recording_settings::{current_auto_start, RecordingSettingsState};
use crate::AppCore;

/// The ladder one display-return signal walks: ScreenCaptureKit can report no
/// displays for several seconds after a wake (the wake recovery's own ladder).
const RETRY_DELAYS_MS: &[u64] = &[
    0, 500, 1_500, 3_000, 5_000, 10_000, 10_000, 10_000, 10_000, 10_000,
];

/// Auto-start is waiting for a display (`pending`); a retry walk is running (`retrying`).
#[derive(Default)]
pub struct PendingAutoStartState {
    pending: AtomicBool,
    retrying: AtomicBool,
}

impl PendingAutoStartState {
    pub fn is_pending(&self) -> bool {
        self.pending.load(Ordering::SeqCst)
    }
}

/// A start refused only because ScreenCaptureKit saw no display.
pub fn is_display_unavailable(error: &CaptureErrorResponse) -> bool {
    error.code == "capture_display_unavailable"
        || error
            .message
            .contains("Failed to find any displays or windows")
        || error.message.contains("code: -3815")
}

/// Deferred startup's auto-start (only when the setting is on).
pub fn maybe_auto_start_native_capture(core: &Arc<AppCore>) {
    if !current_auto_start(&core.state::<RecordingSettingsState>()) {
        return;
    }
    let result = super::control::start_native_capture_from_core("auto-start", core).map(|_| ());
    note_auto_start_result(core, result);
}

/// Arms the retry when the start was refused for want of a display.
pub(crate) fn note_auto_start_result(core: &AppCore, result: Result<(), CaptureErrorResponse>) {
    let Err(error) = result else { return };
    if !is_display_unavailable(&error) {
        return;
    }
    if let Some(state) = core.try_state::<PendingAutoStartState>() {
        state.pending.store(true, Ordering::SeqCst);
        debug_log::log_warn(
            "auto-start found no display (asleep or locked); recording starts when a display returns",
        );
        // The session didn't change, but the hosts re-read and can say why it isn't recording.
        let session = super::control::current_native_capture_session(core);
        super::emit_native_capture_session_changed(core, &session);
    }
}

/// A display may be back: walk the retry ladder off the caller's thread (the
/// callers include the main thread's display callback). No-op unless pending.
pub fn retry_pending_auto_start(core: &Arc<AppCore>) {
    let Some(state) = core.try_state::<PendingAutoStartState>() else {
        return;
    };
    if !state.is_pending() || state.retrying.swap(true, Ordering::SeqCst) {
        return;
    }
    let core = Arc::clone(core);
    let spawned = std::thread::Builder::new()
        .name("mnema-auto-start-retry".into())
        .spawn(move || {
            let start = || {
                super::control::start_native_capture_from_core("auto-start retry", &core)
                    .map(|_| ())
            };
            run_retry(&core, &state, start, std::thread::sleep);
            state.retrying.store(false, Ordering::SeqCst);
        });
    if let Err(error) = spawned {
        debug_log::log_warn(format!("failed to spawn the auto-start retry: {error}"));
    }
}

/// One walk of the ladder. Stops (and forgets the pending start) on success, on
/// any failure other than a missing display, when auto-start was turned off, a
/// session is already running, or the app is quitting. Still no display after
/// the last step: stays pending for the next signal.
pub(crate) fn run_retry(
    core: &AppCore,
    state: &PendingAutoStartState,
    mut start: impl FnMut() -> Result<(), CaptureErrorResponse>,
    mut wait: impl FnMut(Duration),
) {
    for delay_ms in RETRY_DELAYS_MS {
        if *delay_ms > 0 {
            wait(Duration::from_millis(*delay_ms));
        }
        if !still_wanted(core) {
            state.pending.store(false, Ordering::SeqCst);
            return;
        }
        match start() {
            Ok(()) => {
                state.pending.store(false, Ordering::SeqCst);
                debug_log::log_info("auto-start began recording now that a display is available");
                return;
            }
            Err(error) if is_display_unavailable(&error) => {
                debug_log::log(format!(
                    "auto-start retry still found no display (after {delay_ms}ms)"
                ));
            }
            // `start_native_capture_from_core` already logged why; retrying it blindly won't help.
            Err(_) => {
                state.pending.store(false, Ordering::SeqCst);
                return;
            }
        }
    }
}

fn still_wanted(core: &AppCore) -> bool {
    if crate::exit::is_graceful_exit_in_progress(core) {
        return false;
    }
    let auto_start = core
        .try_state::<RecordingSettingsState>()
        .is_some_and(|s| current_auto_start(&s));
    let running = core
        .try_state::<crate::native_capture::lifecycle::NativeCaptureState>()
        .is_some_and(|s| s.lock().map(|r| r.session().is_running).unwrap_or(false));
    auto_start && !running
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;
    use crate::test_support::{core_in, FakeHost};

    fn display_gone() -> CaptureErrorResponse {
        CaptureErrorResponse {
            code: "capture_display_unavailable".into(),
            message: "No display available for ScreenCaptureKit capture".into(),
        }
    }

    fn core(dir: &std::path::Path, auto_start: bool) -> AppCore {
        core_with_host(dir, auto_start).0
    }

    fn core_with_host(dir: &std::path::Path, auto_start: bool) -> (AppCore, Arc<FakeHost>) {
        let host = Arc::new(FakeHost::default());
        let core = core_in(host.clone(), dir);
        crate::startup::manage_default_states(&core);
        core.state::<RecordingSettingsState>()
            .lock()
            .unwrap()
            .settings
            .auto_start = auto_start;
        (core, host)
    }

    #[test]
    fn a_start_refused_for_no_display_retries_on_the_return_signal_and_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let (core, host) = core_with_host(dir.path(), true);
        note_auto_start_result(&core, Err(display_gone()));
        let state = core.state::<PendingAutoStartState>();
        assert!(state.is_pending(), "armed by the refusal");
        assert!(
            host.events
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, crate::CoreEvent::NativeCaptureSessionChanged(_))),
            "the hosts re-read and show why it isn't recording"
        );

        // The display is back on the third attempt (0 ms, then 500 ms, 1500 ms).
        let attempts = Cell::new(0);
        let waits = RefCell::new(Vec::new());
        run_retry(
            &core,
            &state,
            || {
                attempts.set(attempts.get() + 1);
                if attempts.get() < 3 {
                    Err(display_gone())
                } else {
                    Ok(())
                }
            },
            |d| waits.borrow_mut().push(d.as_millis()),
        );
        assert_eq!(attempts.get(), 3);
        assert_eq!(*waits.borrow(), vec![500, 1_500]);
        assert!(
            !state.is_pending(),
            "recording started: nothing left to retry"
        );
    }

    #[test]
    fn other_start_failures_are_neither_armed_nor_retried() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path(), true);
        let denied = CaptureErrorResponse {
            code: "capture_permission_denied".into(),
            message: "Screen recording permission is required".into(),
        };
        note_auto_start_result(&core, Err(denied.clone()));
        let state = core.state::<PendingAutoStartState>();
        assert!(!state.is_pending(), "only a missing display waits");

        // Armed, then a different failure on the retry: one attempt, then forgotten.
        note_auto_start_result(&core, Err(display_gone()));
        let attempts = Cell::new(0);
        run_retry(
            &core,
            &state,
            || {
                attempts.set(attempts.get() + 1);
                Err(denied.clone())
            },
            |_| {},
        );
        assert_eq!(attempts.get(), 1);
        assert!(!state.is_pending());
    }

    #[test]
    fn still_no_display_after_the_ladder_stays_pending_for_the_next_signal() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path(), true);
        note_auto_start_result(&core, Err(display_gone()));
        let state = core.state::<PendingAutoStartState>();
        let attempts = Cell::new(0);
        run_retry(
            &core,
            &state,
            || {
                attempts.set(attempts.get() + 1);
                Err(display_gone())
            },
            |_| {},
        );
        assert_eq!(attempts.get(), RETRY_DELAYS_MS.len());
        assert!(state.is_pending());
    }

    #[test]
    fn turning_auto_start_off_meanwhile_cancels_the_retry() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path(), false);
        let state = core.state::<PendingAutoStartState>();
        state.pending.store(true, Ordering::SeqCst);
        run_retry(&core, &state, || panic!("must not start"), |_| {});
        assert!(!state.is_pending());
    }

    #[test]
    fn retry_is_a_no_op_unless_pending() {
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(core(dir.path(), true));
        retry_pending_auto_start(&core);
        let state = core.state::<PendingAutoStartState>();
        assert!(!state.retrying.load(Ordering::SeqCst), "no walk started");
    }
}
