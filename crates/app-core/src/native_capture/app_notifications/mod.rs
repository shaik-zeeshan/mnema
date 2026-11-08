//! The bell's session notifications: the list, and the push/clear helpers that
//! broadcast it (`app_notifications_changed`).

use crate::native_capture::runtime;
use crate::{debug_log, AppCore, CoreEvent};
use std::sync::{Arc, Mutex};

pub const APP_NOTIFICATIONS_CHANGED_EVENT: &str = "app_notifications_changed";
const PRIVACY_RECOVERY_RESTART_REQUIRED_NOTIFICATION_ID: &str = "privacy-recovery-restart-required";

#[derive(Debug, Clone, serde::Serialize)]
#[allow(dead_code)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppNotificationAction {
    OpenSettingsTab {
        tab: String,
    },
    /// "Start" + "Snooze for today" (see `not_recording_reminder`).
    NotRecordingReminder,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppNotification {
    pub id: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    pub created_at_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<AppNotificationAction>,
}

#[derive(Debug, Default)]
pub struct AppNotificationsRuntime {
    notifications: Vec<AppNotification>,
}

impl AppNotificationsRuntime {
    pub fn list(&self) -> Vec<AppNotification> {
        self.notifications.clone()
    }

    pub fn push_session_notification(
        &mut self,
        notification: AppNotification,
    ) -> Vec<AppNotification> {
        self.notifications.retain(|item| item.id != notification.id);
        self.notifications.push(notification);
        self.list()
    }

    pub fn clear_one(&mut self, id: &str) -> Vec<AppNotification> {
        self.notifications.retain(|item| item.id != id);
        self.list()
    }

    pub fn clear_all(&mut self) -> Vec<AppNotification> {
        self.notifications.clear();
        self.list()
    }
}

/// One list, shared by app-core and the host (both manage the same `Arc`).
pub type AppNotificationsState = Arc<Mutex<AppNotificationsRuntime>>;

pub fn emit_app_notifications_changed(core: &AppCore, notifications: &[AppNotification]) {
    core.host().emit(CoreEvent::AppNotificationsChanged(
        serde_json::to_value(notifications).unwrap_or(serde_json::Value::Null),
    ));
}

pub fn push_app_notification(
    core: &AppCore,
    state: &AppNotificationsState,
    notification: AppNotification,
) {
    let notifications = {
        let mut runtime = state.lock().expect("app notifications state poisoned");
        runtime.push_session_notification(notification)
    };
    emit_app_notifications_changed(core, &notifications);
}

pub fn push_privacy_recovery_restart_required_notification(core: &AppCore) {
    let Some(state) = core.try_state::<AppNotificationsState>() else {
        debug_log::log_warn(
            "app notifications state unavailable while reporting privacy recovery restart requirement",
        );
        return;
    };
    push_app_notification(
        core,
        &state,
        AppNotification {
            id: PRIVACY_RECOVERY_RESTART_REQUIRED_NOTIFICATION_ID.to_string(),
            severity: "warning".to_string(),
            title: "Screen capture paused for privacy".to_string(),
            message: "Screen and system audio capture were paused after privacy filter recovery failed. Stop and start recording to resume those sources.".to_string(),
            created_at_unix_ms: runtime::now_unix_ms(),
            action: None,
        },
    );
}

pub fn push_warning_app_notification(
    core: &AppCore,
    id: &str,
    title: &str,
    message: &str,
    settings_tab: Option<&str>,
    created_at_unix_ms: u64,
) {
    let action = settings_tab.map(|tab| AppNotificationAction::OpenSettingsTab {
        tab: tab.to_string(),
    });
    push_app_notification(
        core,
        &core.state::<AppNotificationsState>(),
        AppNotification {
            id: id.to_string(),
            severity: "warning".to_string(),
            title: title.to_string(),
            message: message.to_string(),
            created_at_unix_ms,
            action,
        },
    );
}

/// Clear (remove) a single app notification by its stable id and broadcast the
/// updated list. Mirrors [`push_warning_app_notification`] for the suspend/resume
/// pair: the low-disk warning is pushed on suspend and cleared here on resume.
/// Best-effort — silently no-ops if the notifications state is unavailable.
pub fn clear_app_notification_by_id(core: &AppCore, id: &str) {
    let Some(state) = core.try_state::<AppNotificationsState>() else {
        return;
    };
    let notifications = {
        let mut runtime = state.lock().expect("app notifications state poisoned");
        runtime.clear_one(id)
    };
    emit_app_notifications_changed(core, &notifications);
}

pub fn push_info_app_notification(
    core: &AppCore,
    id: &str,
    title: &str,
    message: &str,
    settings_tab: Option<&str>,
    created_at_unix_ms: u64,
) {
    let action = settings_tab.map(|tab| AppNotificationAction::OpenSettingsTab {
        tab: tab.to_string(),
    });
    push_app_notification(
        core,
        &core.state::<AppNotificationsState>(),
        AppNotification {
            id: id.to_string(),
            severity: "info".to_string(),
            title: title.to_string(),
            message: message.to_string(),
            created_at_unix_ms,
            action,
        },
    );
}

/// Push an `error`-severity app notification. Mirrors
/// [`push_warning_app_notification`] but with `severity: "error"`; used for the
/// low-disk graceful-stop notice (ADR 0040), where capture has stopped and the
/// user must free space before recording can be restarted. The frontend already
/// types `severity` as `"info" | "warning" | "error"` and styles the `error`
/// case; an unstyled severity would degrade to the neutral base card.
pub fn push_error_app_notification(
    core: &AppCore,
    id: &str,
    title: &str,
    message: &str,
    settings_tab: Option<&str>,
    created_at_unix_ms: u64,
) {
    let action = settings_tab.map(|tab| AppNotificationAction::OpenSettingsTab {
        tab: tab.to_string(),
    });
    push_app_notification(
        core,
        &core.state::<AppNotificationsState>(),
        AppNotification {
            id: id.to_string(),
            severity: "error".to_string(),
            title: title.to_string(),
            message: message.to_string(),
            created_at_unix_ms,
            action,
        },
    );
}

#[cfg(test)]
mod tests;
