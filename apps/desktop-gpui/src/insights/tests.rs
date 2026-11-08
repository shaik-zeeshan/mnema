//! Layer 2 for Insights: the delayed-commit Dismiss at 4999 / 5000 ms through
//! the view with `advance_clock`, Undo, navigating away (tab switch, back from
//! the subject, the shell leaving Insights) committing at once, `[` / `]` day
//! stepping, and ⌘J's question.

use std::sync::Arc;
use std::time::Duration;

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::{
    point, px, size, AppContext as _, Bounds, Entity, TestAppContext, WindowAppearance,
    WindowBounds, WindowHandle, WindowOptions,
};

use super::fixture::FixtureBackend;
use super::undo::DISMISS_UNDO;
use super::{Insights, Tab};
use crate::theme;
use crate::ui::clock;

const MS: Duration = Duration::from_millis(1);

fn open(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<Insights>, Arc<FixtureBackend>) {
    let backend = Arc::new(FixtureBackend::load("populated").expect("fixture"));
    let b = backend.clone();
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        clock::freeze(Some(app_core::fixtures::fixture_now()), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(1280.), px(800.)),
            })),
            ..Default::default()
        };
        let (window, view) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                let mut i = Insights::new(b, window, cx);
                i.reload(cx);
                i
            })
        })
        .expect("window");
        (window.downcast::<Root>().expect("root"), view)
    });
    cx.run_until_parked();
    (window, view, backend)
}

fn advance(cx: &mut TestAppContext, d: Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

fn dismisses(b: &FixtureBackend) -> usize {
    b.calls()
        .iter()
        .filter(|c| c.starts_with("dismiss:"))
        .count()
}

/// Open the first subject and dismiss its lead conclusion; returns its id.
fn dismiss_lead(cx: &mut TestAppContext, v: &Entity<Insights>) -> i64 {
    let subject = v
        .read_with(cx, |i, _| {
            i.subjects
                .as_ref()
                .map(|s| s.conclusions[0].subject.clone())
        })
        .unwrap_or_default();
    v.update(cx, |i, cx| {
        i.set_tab(Tab::Subjects, cx);
    });
    cx.run_until_parked();
    let subject = if subject.is_empty() {
        v.read_with(cx, |i, _| {
            i.subjects.as_ref().unwrap().conclusions[0].subject.clone()
        })
    } else {
        subject
    };
    v.update(cx, |i, cx| i.open_subject(subject, cx));
    cx.run_until_parked();
    let id = v.read_with(cx, |i, _| {
        i.detail
            .as_ref()
            .unwrap()
            .view
            .as_ref()
            .unwrap()
            .conclusions[0]
            .id
    });
    v.update(cx, |i, cx| i.dismiss(id, cx));
    cx.run_until_parked();
    id
}

#[gpui_kit::test]
fn dismiss_commits_at_5000ms_not_4999(cx: &mut TestAppContext) {
    let (_, v, b) = open(cx);
    let id = dismiss_lead(cx, &v);
    assert!(
        v.read_with(cx, |i, _| i.undo.has(id)),
        "the row collapses to Dismissed · Undo"
    );
    advance(cx, DISMISS_UNDO - MS);
    assert_eq!(
        dismisses(&b),
        0,
        "nothing reaches the backend inside the window"
    );
    advance(cx, MS);
    assert_eq!(
        b.calls()
            .iter()
            .filter(|c| **c == format!("dismiss:{id}"))
            .count(),
        1
    );
    assert!(!v.read_with(cx, |i, _| i.undo.has(id)));
}

#[gpui_kit::test]
fn undo_inside_the_window_never_commits(cx: &mut TestAppContext) {
    let (_, v, b) = open(cx);
    let id = dismiss_lead(cx, &v);
    advance(cx, DISMISS_UNDO - MS);
    v.update(cx, |i, cx| i.undo_dismiss(id, cx));
    advance(cx, DISMISS_UNDO * 2);
    assert_eq!(dismisses(&b), 0);
    assert_eq!(
        v.read_with(cx, |i, _| i.open_conclusion),
        Some(id),
        "Undo reopens the row"
    );
}

#[gpui_kit::test]
fn navigating_away_commits_at_once(cx: &mut TestAppContext) {
    // Back from the subject page.
    let (_, v, b) = open(cx);
    dismiss_lead(cx, &v);
    advance(cx, Duration::from_millis(100));
    v.update(cx, |i, cx| i.close_subject(cx));
    cx.run_until_parked();
    assert_eq!(dismisses(&b), 1, "leaving the subject commits now");
    advance(cx, DISMISS_UNDO);
    assert_eq!(dismisses(&b), 1, "and only once");

    // Another tab.
    let (_, v, b) = open(cx);
    dismiss_lead(cx, &v);
    v.update(cx, |i, cx| i.set_tab(Tab::Context, cx));
    cx.run_until_parked();
    assert_eq!(dismisses(&b), 1);

    // The shell leaving Insights calls `leave`.
    let (_, v, b) = open(cx);
    dismiss_lead(cx, &v);
    v.update(cx, |i, cx| i.leave(cx));
    cx.run_until_parked();
    assert_eq!(dismisses(&b), 1);
}

#[gpui_kit::test]
fn brackets_step_days_never_past_today(cx: &mut TestAppContext) {
    let (_, v, _) = open(cx);
    v.update(cx, |i, cx| i.set_tab(Tab::Journal, cx));
    let today = v.read_with(cx, |i, _| i.day);
    v.update(cx, |i, cx| super::journal::on_key(i, "]", cx));
    assert_eq!(v.read_with(cx, |i, _| i.day), today, "] stops at today");
    v.update(cx, |i, cx| super::journal::on_key(i, "[", cx));
    assert_eq!(
        v.read_with(cx, |i, _| i.day),
        today - time::Duration::days(1)
    );
    v.update(cx, |i, cx| super::journal::on_key(i, "]", cx));
    assert_eq!(v.read_with(cx, |i, _| i.day), today);
    v.update(cx, |i, cx| i.set_tab(Tab::Overview, cx));
    assert!(
        !v.update(cx, |i, cx| super::journal::on_key(i, "[", cx)),
        "only on the Journal"
    );
}

#[gpui_kit::test]
fn cmd_j_asks_about_the_period_or_the_subject(cx: &mut TestAppContext) {
    let (_, v, _) = open(cx);
    let q = v.update(cx, |i, cx| i.ask_query(cx));
    assert_eq!(q.as_deref(), Some("What did this week add up to?"));
    dismiss_lead(cx, &v);
    let (q, s) = v.update(cx, |i, cx| (i.ask_query(cx), i.subject.clone()));
    assert_eq!(q, Some(format!("What do you know about {}?", s.unwrap())));
}
