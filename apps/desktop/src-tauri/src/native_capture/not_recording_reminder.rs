//! "Mnema isn't recording" reminder: one in-app notification (plus a native
//! macOS one, `macos_notification`) when capture has been fully stopped for a
//! while and the user is plainly at the Mac.
//!
//! A User Capture Pause keeps the session running (`is_running` stays true), so
//! a deliberate pause never counts as "off". Away = display asleep or the lock
//! screen in front (ADR 0021's display-unavailable, plus `loginwindow`).

use super::{AppNotification, AppNotificationAction, AppNotificationsState};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::{Emitter, Manager};

#[cfg(target_os = "macos")]
mod macos_notification;

/// Asks the main window's bell to open, so the reminder row (Start / Snooze for
/// today) is in view.
const OPEN_APP_NOTIFICATIONS_EVENT: &str = "open_app_notifications";

pub(crate) const NOT_RECORDING_REMINDER_NOTIFICATION_ID: &str = "not-recording-reminder";
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

#[tauri::command]
pub fn snooze_not_recording_reminder(until_unix_ms: u64, app_handle: tauri::AppHandle) {
    snooze_until(&app_handle, until_unix_ms);
}

fn snooze_until(app_handle: &tauri::AppHandle, until_unix_ms: u64) {
    SNOOZED_UNTIL_MS.store(until_unix_ms, Ordering::Relaxed);
    clear_reminder(app_handle);
}

/// Removes both the in-app row and the native banner.
fn clear_reminder(app_handle: &tauri::AppHandle) {
    super::clear_app_notification_by_id(app_handle, NOT_RECORDING_REMINDER_NOTIFICATION_ID);
    #[cfg(target_os = "macos")]
    macos_notification::remove();
}

/// The native banner's Start button: same start seam as the tray.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn start_from_reminder(app_handle: &tauri::AppHandle) {
    match super::start_native_capture_from_app_handle("not-recording-reminder", app_handle) {
        Ok(_) => clear_reminder(app_handle),
        Err(error) => {
            super::debug_log::log_warn(format!(
                "start from not-recording reminder failed: [{}]",
                error.code
            ));
            // The row is still there; show it so the user sees why.
            show_reminder_in_app(app_handle);
        }
    }
}

/// Clicking the native banner: bring up Main with the bell open.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn show_reminder_in_app(app_handle: &tauri::AppHandle) {
    // ponytail: a cold-built Main misses this event (no listener yet); the
    // bell's badge still points at the row.
    let _ = crate::windows::open_main_window(app_handle);
    let _ = app_handle.emit(OPEN_APP_NOTIFICATIONS_EVENT, ());
}

pub(crate) fn spawn_not_recording_reminder(app_handle: tauri::AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("mnema-not-recording-reminder".to_string())
        .spawn(move || {
            let mut runtime = ReminderRuntime::default();
            loop {
                std::thread::sleep(POLL_INTERVAL);
                tick(&app_handle, &mut runtime);
            }
        });
    if let Err(error) = spawned {
        super::debug_log::log_warn(format!("failed to spawn not-recording reminder: {error}"));
    }
}

fn tick(app_handle: &tauri::AppHandle, runtime: &mut ReminderRuntime) {
    // Before onboarding, or when the license refuses a start, "Start" can't work.
    if !crate::windows::is_onboarding_complete(app_handle) {
        return;
    }
    let license_now_ms = crate::licensing::now_ms();
    if crate::licensing::license_block(
        crate::licensing::cached_status(app_handle).as_ref(),
        license_now_ms,
    )
    .is_some()
    {
        return;
    }

    let capture_off = !super::current_native_capture_session(app_handle).is_running;
    let now_ms = super::runtime::now_unix_ms();
    let step = runtime.observe(
        now_ms,
        capture_off,
        super::system_idle::current_system_idle_ms(),
        capture_off && user_away(),
        SNOOZED_UNTIL_MS.load(Ordering::Relaxed),
    );
    match step {
        ReminderStep::Nothing => {}
        ReminderStep::Clear => clear_reminder(app_handle),
        ReminderStep::Fire => {
            super::debug_log::log_info("not-recording reminder fired");
            let Some(state) = app_handle.try_state::<AppNotificationsState>() else {
                return;
            };
            super::push_app_notification(
                app_handle,
                state.inner(),
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
            #[cfg(target_os = "macos")]
            macos_notification::post(app_handle);
        }
    }
}

#[cfg(target_os = "macos")]
fn user_away() -> bool {
    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    core_graphics::display::CGDisplay::main().is_asleep()
        || super::metadata::frontmost_app_is_lock_screen()
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
}
