//! The bell's row actions against a real core: Open … settings opens its tab
//! and clears the row; a refused Start from the reminder keeps both.

use app_core::native_capture::app_notifications::{
    push_app_notification, AppNotification, AppNotificationAction, AppNotificationsState,
};
use gpui_kit::test::TestWindowExt;

use super::*;
use crate::shell::Surface;

fn rows(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Vec<String> {
    cx.read(|cx| {
        shell
            .read(cx)
            .model
            .notifications
            .iter()
            .map(|n| n.id.clone())
            .collect()
    })
}

fn click_row(cx: &mut TestAppContext, window: WindowHandle<Root>, id: &'static str) {
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click((id, 0usize), cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_rows_open_settings_action_opens_the_tab_and_clears_the_row(cx: &mut TestAppContext) {
    let mut live = live();
    let (window, shell) = open_shell(cx, &live, None);
    wait_for(cx, &mut live, &shell, "the first load", |_| true);
    cx.update(|cx| crate::window::adopt_main(window.into(), &shell, cx));
    bell::push_warning_app_notification(
        &live.core,
        "w-keys",
        "Global shortcuts unavailable",
        "Another app holds it.",
        Some("shortcuts"),
        1,
    );
    wait_for(cx, &mut live, &shell, "the row", |cx| {
        rows(cx, &shell) == ["w-keys".to_string()]
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.toggle_overlay(Overlay::Notifications, cx)));
    click_row(cx, window, "notif-action");
    wait_for(cx, &mut live, &shell, "the row cleared", |cx| {
        rows(cx, &shell).is_empty()
    });
    cx.read(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.overlay, None, "the popover closed");
        assert_eq!(s.surface, Surface::Settings, "on its tab");
    });
    assert!(bell::get_app_notifications(&live.core).is_empty());
}

#[gpui_kit::test]
fn a_refused_start_from_the_reminder_keeps_the_row_and_the_popover(cx: &mut TestAppContext) {
    let mut live = live();
    let (window, shell) = open_shell(cx, &live, None);
    wait_for(cx, &mut live, &shell, "the first load", |_| true);
    let state = live.core.state::<AppNotificationsState>();
    push_app_notification(
        &live.core,
        &state,
        AppNotification {
            id: "not_recording_reminder".into(),
            severity: "info".into(),
            title: "Mnema isn't recording".into(),
            message: "Start recording?".into(),
            created_at_unix_ms: 1,
            action: Some(AppNotificationAction::NotRecordingReminder),
        },
    );
    wait_for(cx, &mut live, &shell, "the reminder", |cx| {
        rows(cx, &shell).len() == 1
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.toggle_overlay(Overlay::Notifications, cx)));
    click_row(cx, window, "notif-action");
    // Every source is off, so app-core refuses the start.
    wait_for(cx, &mut live, &shell, "the refused start", |cx| {
        cx.read(|cx| shell.read(cx).model.status.start_error.is_some())
    });
    // The failure's own bell row lands after the error: the reminder stays beside it.
    wait_for(cx, &mut live, &shell, "the failure row", |cx| {
        rows(cx, &shell).len() == 2
    });
    assert!(rows(cx, &shell).contains(&"not_recording_reminder".to_string()));
    assert_eq!(
        cx.read(|cx| shell.read(cx).overlay),
        Some(Overlay::Notifications),
        "the popover stays open"
    );
}

/// Hidden Settings doesn't re-read on a change event (a full read reaches the
/// licence server); the next open re-reads, and a shown page follows events.
#[gpui_kit::test]
fn hidden_settings_waits_for_the_next_open_to_re_read(cx: &mut TestAppContext) {
    let mut live = live();
    let (_window, shell) = open_shell(cx, &live, None);
    wait_for(cx, &mut live, &shell, "the first load", |_| true);
    let settings = cx.read(|cx| shell.read(cx).settings.clone());
    let reading = |cx: &mut TestAppContext| cx.read(|cx| settings.read(cx).ui.models_loading);
    let changed = |cx: &mut TestAppContext| {
        cx.update(|cx| settings.update(cx, |s, cx| s.on_event(&CoreEvent::UserContextChanged, cx)))
    };
    changed(cx);
    assert!(!reading(cx), "hidden: no re-read");
    cx.update(|cx| shell.update(cx, |s, cx| s.go(Surface::Settings, cx)));
    assert!(reading(cx), "opening re-reads");
    wait_for(cx, &mut live, &shell, "the open's read", |cx| !reading(cx));
    changed(cx);
    assert!(reading(cx), "shown: an event re-reads");
}
