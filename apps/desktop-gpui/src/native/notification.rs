//! The native "Mnema isn't recording" banner (`WindowSignal::NotRecordingReminder`)
//! with Start / Snooze for today, through GPUI's `UNUserNotificationCenter`
//! wrapper (it already skips a binary that isn't in an `.app`, shows banners
//! while Mnema is frontmost, and registers the action category). Chosen over
//! porting `macos_notification.rs`: same behaviour, no second delegate.

use std::sync::Arc;

use app_core::native_capture::not_recording_reminder as reminder;
use app_core::AppCore;
use gpui_kit::{App, SystemNotification, SystemNotificationAction, SystemNotificationResponse};

const TAG: &str = "day.mnema.not-recording-reminder";
const START: &str = "start";
const SNOOZE: &str = "snooze-today";

pub fn post(cx: &App) {
    cx.show_system_notification(SystemNotification {
        tag: TAG.into(),
        title: "Mnema isn't recording".into(),
        body: "Recording has been off for over 30 minutes.".into(),
        actions: vec![
            SystemNotificationAction {
                id: START.into(),
                label: "Start".into(),
            },
            SystemNotificationAction {
                id: SNOOZE.into(),
                label: "Snooze for today".into(),
            },
        ],
    });
}

/// Withdraw it once recording starts or the reminder is snoozed.
pub fn remove(cx: &App) {
    cx.dismiss_system_notification(TAG);
}

/// Register once: a click on the banner or one of its buttons.
pub fn on_response(core: Arc<AppCore>, cx: &mut App) {
    cx.on_system_notification_response(move |response: SystemNotificationResponse, _| {
        if response.tag.as_ref() != TAG {
            return;
        }
        let core = core.clone();
        let action = response.action_id.map(|a| a.to_string());
        // Off the main thread: a start opens capture streams.
        std::thread::spawn(move || match action.as_deref() {
            Some(START) => reminder::start_from_reminder(&core),
            Some(SNOOZE) => reminder::snooze_until(&core, super::sys::next_local_midnight_ms()),
            None => reminder::show_reminder_in_app(&core),
            Some(_) => {}
        });
    });
}
