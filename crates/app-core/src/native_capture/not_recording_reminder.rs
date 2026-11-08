//! "Mnema isn't recording" reminder: one in-app notification (plus the host's
//! native banner, [`WindowSignal::NotRecordingReminder`]) when capture has been fully stopped for a
//! while and the user is plainly at the Mac.
//!
//! A User Capture Pause keeps the session running (`is_running` stays true), so
//! a deliberate pause never counts as "off". Away = display asleep or the lock
//! screen in front (ADR 0021's display-unavailable, plus `loginwindow`).

use crate::native_capture::app_notifications::{
    clear_app_notification_by_id, push_app_notification, AppNotification, AppNotificationAction,
    AppNotificationsState,
};
use crate::{AppCore, CoreEvent, WindowRequest, WindowSignal};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const NOT_RECORDING_REMINDER_NOTIFICATION_ID: &str = "not-recording-reminder";
/// How long capture must have been off before the reminder fires.
const REMIND_AFTER_OFF_MS: u64 = 30 * 60 * 1000;
/// Keyboard/mouse input this recent counts as "actively using the Mac".
const ACTIVE_INPUT_WITHIN_MS: u64 = 60 * 1000;
const POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Unix ms the user's "Snooze for today" lasts until (their local midnight,
/// computed by the frontend). 0 = not snoozed.
static SNOOZED_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, PartialEq, Eq)]
enum ReminderStep {
    Nothing,
    Fire,
    Clear,
}

/// One off-stretch's bookkeeping, owned by the poll thread.
#[derive(Debug, Default)]
struct ReminderRuntime {
    off_since_ms: Option<u64>,
    fired: bool,
}

impl ReminderRuntime {
    fn observe(
        &mut self,
        now_ms: u64,
        capture_off: bool,
        user_idle_ms: Option<u64>,
        away: bool,
        snoozed_until_ms: u64,
    ) -> ReminderStep {
        if !capture_off {
            // Recording (or user-paused) ends the stretch and re-arms.
            self.off_since_ms = None;
            return if std::mem::take(&mut self.fired) {
                ReminderStep::Clear
            } else {
                ReminderStep::Nothing
            };
        }
        let off_since_ms = *self.off_since_ms.get_or_insert(now_ms);
        if now_ms < snoozed_until_ms {
            // A snooze ends the current reminder; after it lapses the same
            // stretch may remind again (once).
            self.fired = false;
            return ReminderStep::Nothing;
        }
        let user_active = user_idle_ms.is_some_and(|idle| idle < ACTIVE_INPUT_WITHIN_MS);
        if self.fired
            || away
            || !user_active
            || now_ms.saturating_sub(off_since_ms) < REMIND_AFTER_OFF_MS
        {
            return ReminderStep::Nothing;
        }
        self.fired = true;
        ReminderStep::Fire
    }
}

pub fn snooze_not_recording_reminder(until_unix_ms: u64, core: &AppCore) {
    snooze_until(core, until_unix_ms);
}

/// Also the native banner's "Snooze for today" action.
pub fn snooze_until(core: &AppCore, until_unix_ms: u64) {
    SNOOZED_UNTIL_MS.store(until_unix_ms, Ordering::Relaxed);
    clear_reminder(core);
}

/// Removes both the in-app row and the native banner.
fn clear_reminder(core: &AppCore) {
    clear_app_notification_by_id(core, NOT_RECORDING_REMINDER_NOTIFICATION_ID);
    core.host()
        .window_signal(WindowSignal::NotRecordingReminder { post: false });
}

/// The native banner's Start button: same start seam as the tray.
pub fn start_from_reminder(core: &Arc<AppCore>) {
    match super::control::start_native_capture_from_core("not-recording-reminder", core) {
        Ok(_) => clear_reminder(core),
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "start from not-recording reminder failed: [{}]",
                error.code
            ));
            // The row is still there; show it so the user sees why.
            show_reminder_in_app(core);
        }
    }
}

/// Clicking the native banner: bring up Main with the bell open.
pub fn show_reminder_in_app(core: &AppCore) {
    // ponytail: a cold-built Main misses this event (no listener yet); the
    // bell's badge still points at the row.
    let _ = core.host().open_window(WindowRequest::Main);
    core.host().emit(CoreEvent::OpenAppNotifications);
}

pub fn spawn_not_recording_reminder(core: Arc<AppCore>) {
    let spawned = std::thread::Builder::new()
        .name("mnema-not-recording-reminder".to_string())
        .spawn(move || {
            let mut runtime = ReminderRuntime::default();
            loop {
                std::thread::sleep(POLL_INTERVAL);
                tick(&core, &mut runtime);
            }
        });
    if let Err(error) = spawned {
        crate::debug_log::log_warn(format!("failed to spawn not-recording reminder: {error}"));
    }
}

fn tick(core: &AppCore, runtime: &mut ReminderRuntime) {
    // Before onboarding, or when the license refuses a start, "Start" can't work.
    if !crate::onboarding_state::current_onboarding_state(
        core,
        &core.state::<crate::onboarding_state::OnboardingStateStore>(),
    )
    .is_complete()
    {
        return;
    }
    let license_now_ms = crate::licensing::now_ms();
    if crate::licensing::license_block(
        crate::licensing::cached_status(core).as_ref(),
        license_now_ms,
    )
    .is_some()
    {
        return;
    }

    let capture_off = !super::control::current_native_capture_session(core).is_running;
    let now_ms = crate::native_capture::runtime::now_unix_ms();
    let step = runtime.observe(
        now_ms,
        capture_off,
        crate::native_capture_system_idle::current_system_idle_ms(),
        capture_off && user_away(),
        SNOOZED_UNTIL_MS.load(Ordering::Relaxed),
    );
    match step {
        ReminderStep::Nothing => {}
        ReminderStep::Clear => clear_reminder(core),
        ReminderStep::Fire => {
            crate::debug_log::log_info("not-recording reminder fired");
            let Some(state) = core.try_state::<AppNotificationsState>() else {
                return;
            };
            push_app_notification(
                core,
                &state,
                AppNotification {
                    id: NOT_RECORDING_REMINDER_NOTIFICATION_ID.to_string(),
                    severity: "warning".to_string(),
                    title: "Mnema isn't recording".to_string(),
                    message: "Recording has been off for a while. Start it so today isn't lost."
                        .to_string(),
                    created_at_unix_ms: now_ms,
                    action: Some(AppNotificationAction::NotRecordingReminder),
                },
            );
            core.host()
                .window_signal(WindowSignal::NotRecordingReminder { post: true });
        }
    }
}

#[cfg(target_os = "macos")]
fn user_away() -> bool {
    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    core_graphics::display::CGDisplay::main().is_asleep()
        || crate::native_capture::metadata::frontmost_app_is_lock_screen()
}

#[cfg(not(target_os = "macos"))]
fn user_away() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60 * 1000;
    const ACTIVE: Option<u64> = Some(5_000);

    /// Off since t=0, observed again at `at_ms` with the given conditions.
    fn after_off(at_ms: u64, idle: Option<u64>, away: bool, snooze: u64) -> ReminderStep {
        let mut r = ReminderRuntime::default();
        assert_eq!(r.observe(0, true, ACTIVE, false, 0), ReminderStep::Nothing);
        r.observe(at_ms, true, idle, away, snooze)
    }

    #[test]
    fn fires_only_after_thirty_minutes_off_while_active() {
        assert_eq!(after_off(29 * MIN, ACTIVE, false, 0), ReminderStep::Nothing);
        assert_eq!(after_off(30 * MIN, ACTIVE, false, 0), ReminderStep::Fire);
    }

    #[test]
    fn idle_unknown_away_or_snoozed_never_fire() {
        assert_eq!(
            after_off(40 * MIN, Some(5 * MIN), false, 0),
            ReminderStep::Nothing
        );
        assert_eq!(after_off(40 * MIN, None, false, 0), ReminderStep::Nothing);
        assert_eq!(after_off(40 * MIN, ACTIVE, true, 0), ReminderStep::Nothing);
        assert_eq!(
            after_off(40 * MIN, ACTIVE, false, 50 * MIN),
            ReminderStep::Nothing
        );
    }

    #[test]
    fn paused_or_recording_never_counts_as_off() {
        let mut r = ReminderRuntime::default();
        assert_eq!(r.observe(0, false, ACTIVE, false, 0), ReminderStep::Nothing);
        assert_eq!(
            r.observe(60 * MIN, false, ACTIVE, false, 0),
            ReminderStep::Nothing
        );
        // The stretch starts when capture is first seen off, not at launch.
        assert_eq!(
            r.observe(61 * MIN, true, ACTIVE, false, 0),
            ReminderStep::Nothing
        );
        assert_eq!(
            r.observe(90 * MIN, true, ACTIVE, false, 0),
            ReminderStep::Nothing
        );
        assert_eq!(
            r.observe(91 * MIN, true, ACTIVE, false, 0),
            ReminderStep::Fire
        );
    }

    #[test]
    fn fires_once_per_stretch_and_rearms_when_recording_starts() {
        let mut r = ReminderRuntime::default();
        r.observe(0, true, ACTIVE, false, 0);
        assert_eq!(
            r.observe(30 * MIN, true, ACTIVE, false, 0),
            ReminderStep::Fire
        );
        assert_eq!(
            r.observe(90 * MIN, true, ACTIVE, false, 0),
            ReminderStep::Nothing
        );
        assert_eq!(
            r.observe(91 * MIN, false, ACTIVE, false, 0),
            ReminderStep::Clear
        );
        assert_eq!(
            r.observe(92 * MIN, false, ACTIVE, false, 0),
            ReminderStep::Nothing
        );
        r.observe(93 * MIN, true, ACTIVE, false, 0);
        assert_eq!(
            r.observe(123 * MIN, true, ACTIVE, false, 0),
            ReminderStep::Fire
        );
    }

    #[test]
    fn snooze_lapsing_at_midnight_lets_the_same_stretch_remind_again() {
        let mut r = ReminderRuntime::default();
        r.observe(0, true, ACTIVE, false, 0);
        assert_eq!(
            r.observe(30 * MIN, true, ACTIVE, false, 0),
            ReminderStep::Fire
        );
        let midnight = 120 * MIN;
        assert_eq!(
            r.observe(60 * MIN, true, ACTIVE, false, midnight),
            ReminderStep::Nothing
        );
        assert_eq!(
            r.observe(midnight, true, ACTIVE, false, midnight),
            ReminderStep::Fire
        );
    }

    mod host {
        use super::super::*;
        use crate::native_capture::app_notifications::AppNotificationsState;
        use crate::test_support::{core_in, FakeHost};

        fn reminder_core(host: &Arc<FakeHost>, dir: &std::path::Path) -> AppCore {
            let core = core_in(host.clone(), dir);
            core.manage(AppNotificationsState::default());
            core
        }

        #[test]
        fn snoozing_withdraws_the_native_banner_and_the_in_app_row() {
            let dir = tempfile::tempdir().unwrap();
            let host = Arc::new(FakeHost::default());
            let core = reminder_core(&host, dir.path());

            snooze_not_recording_reminder(42, &core);
            SNOOZED_UNTIL_MS.store(0, Ordering::Relaxed);

            assert_eq!(
                *host.window_signals.lock().unwrap(),
                vec![WindowSignal::NotRecordingReminder { post: false }]
            );
            assert!(matches!(
                host.events.lock().unwrap().as_slice(),
                [CoreEvent::AppNotificationsChanged(_)]
            ));
        }

        #[test]
        fn clicking_the_banner_opens_main_with_the_bell() {
            let dir = tempfile::tempdir().unwrap();
            let host = Arc::new(FakeHost::default());
            let core = reminder_core(&host, dir.path());

            show_reminder_in_app(&core);

            assert_eq!(
                *host.opened_windows.lock().unwrap(),
                vec![WindowRequest::Main]
            );
            assert_eq!(
                *host.events.lock().unwrap(),
                vec![CoreEvent::OpenAppNotifications]
            );
        }
    }
}
