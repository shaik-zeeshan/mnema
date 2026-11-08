use super::*;

fn app_notification_fixture(id: &str, title: &str, created_at_unix_ms: u64) -> AppNotification {
    AppNotification {
        id: id.to_string(),
        severity: "warning".to_string(),
        title: title.to_string(),
        message: format!("{title} message"),
        created_at_unix_ms,
        action: None,
    }
}

#[test]
fn app_notifications_runtime_lists_session_notifications() {
    let mut runtime = AppNotificationsRuntime::default();

    runtime.push_session_notification(app_notification_fixture("one", "One", 1));
    runtime.push_session_notification(app_notification_fixture("two", "Two", 2));

    let notifications = runtime.list();

    assert_eq!(notifications.len(), 2);
    assert_eq!(notifications[0].id, "one");
    assert_eq!(notifications[1].id, "two");
}

#[test]
fn app_notifications_runtime_replaces_existing_notification_id_once() {
    let mut runtime = AppNotificationsRuntime::default();

    runtime.push_session_notification(app_notification_fixture("vad-fallback", "Old", 1));
    let notifications =
        runtime.push_session_notification(app_notification_fixture("vad-fallback", "Updated", 2));

    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].id, "vad-fallback");
    assert_eq!(notifications[0].title, "Updated");
    assert_eq!(notifications[0].created_at_unix_ms, 2);
}

#[test]
fn app_update_notification_replacement_uses_one_notification_id() {
    let mut runtime = AppNotificationsRuntime::default();

    runtime.push_session_notification(AppNotification {
        id: "app-update-available".to_string(),
        severity: "info".to_string(),
        title: "Old update".to_string(),
        message: "Old".to_string(),
        created_at_unix_ms: 1,
        action: Some(AppNotificationAction::OpenSettingsTab {
            tab: "about".to_string(),
        }),
    });
    let notifications = runtime.push_session_notification(AppNotification {
        id: "app-update-available".to_string(),
        severity: "info".to_string(),
        title: "New update".to_string(),
        message: "New".to_string(),
        created_at_unix_ms: 2,
        action: Some(AppNotificationAction::OpenSettingsTab {
            tab: "about".to_string(),
        }),
    });

    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].id, "app-update-available");
    assert_eq!(notifications[0].title, "New update");
    assert_eq!(notifications[0].created_at_unix_ms, 2);
}

#[test]
fn app_notifications_runtime_clears_one_notification_by_id() {
    let mut runtime = AppNotificationsRuntime::default();
    runtime.push_session_notification(app_notification_fixture("one", "One", 1));
    runtime.push_session_notification(app_notification_fixture("two", "Two", 2));

    let notifications = runtime.clear_one("one");

    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].id, "two");
}

#[test]
fn app_notifications_runtime_clear_one_is_noop_for_missing_id() {
    let mut runtime = AppNotificationsRuntime::default();
    runtime.push_session_notification(app_notification_fixture("one", "One", 1));

    let notifications = runtime.clear_one("missing");

    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].id, "one");
}

#[test]
fn app_notifications_runtime_clears_all_session_notifications() {
    let mut runtime = AppNotificationsRuntime::default();
    runtime.push_session_notification(app_notification_fixture("one", "One", 1));
    runtime.push_session_notification(app_notification_fixture("two", "Two", 2));

    assert!(runtime.clear_all().is_empty());
    assert!(runtime.list().is_empty());
}

#[test]
fn open_settings_tab_about_notification_action_serializes() {
    let notification = AppNotification {
        id: "app-update-available".to_string(),
        severity: "info".to_string(),
        title: "Mnema update available".to_string(),
        message: "Version 0.3.0 is ready to install from Settings.".to_string(),
        created_at_unix_ms: 1234,
        action: Some(AppNotificationAction::OpenSettingsTab {
            tab: "about".to_string(),
        }),
    };

    let payload = serde_json::to_value(&notification).expect("notification should serialize");
    assert_eq!(payload["action"]["type"], "open_settings_tab");
    assert_eq!(payload["action"]["tab"], "about");
}
