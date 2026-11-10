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
    open_at(cx, (1280., 800.))
}

fn open_at(
    cx: &mut TestAppContext,
    (w, h): (f32, f32),
) -> (WindowHandle<Root>, Entity<Insights>, Arc<FixtureBackend>) {
    let backend = Arc::new(FixtureBackend::load("populated").expect("fixture"));
    let b = backend.clone();
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        clock::freeze(Some(app_core::fixtures::fixture_now()), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(w), px(h)),
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
    let id = open_lead(cx, v);
    v.update(cx, |i, cx| i.dismiss(id, cx));
    cx.run_until_parked();
    id
}

/// Open the first subject; returns its lead conclusion's id.
fn open_lead(cx: &mut TestAppContext, v: &Entity<Insights>) -> i64 {
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

fn bounds(
    cx: &mut TestAppContext,
    window: WindowHandle<Root>,
    id: &'static str,
) -> Bounds<gpui_kit::Pixels> {
    use gpui_kit::test::TestWindowExt;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find(id).bounds()
    })
    .expect("window")
}

#[gpui_kit::test]
fn at_the_minimum_window_the_bar_controls_wrap_inside_it(cx: &mut TestAppContext) {
    let (w, h) = crate::window::MAIN_MIN;
    let (window, _, _) = open_at(cx, (w, h));
    let ask = bounds(cx, window, "insights-ask");
    let tab = bounds(cx, window, "insights-tab-overview");
    assert!(
        ask.right() <= px(w),
        "Ask ends at {:?}, past the {w} px window",
        ask.right()
    );
    assert!(
        ask.top() >= tab.bottom(),
        "the controls drop under the tabs (`.in-bar__in` wraps)"
    );
    // A wide window keeps them on the tab row.
    let (window, _, _) = open(cx);
    let ask = bounds(cx, window, "insights-ask");
    let tab = bounds(cx, window, "insights-tab-overview");
    assert!(ask.top() < tab.bottom(), "one row at 1280 px");
}

#[gpui_kit::test]
fn the_evidence_drawer_overlays_and_the_page_stays_put(cx: &mut TestAppContext) {
    // `SubjectDetail.svelte`: on a wide window the drawer covers the pane's right
    // 44% and the centred page keeps its left edge; below 700 px it is a sheet.
    let (window, v, _) = open_at(cx, (2400., 900.));
    let id = open_lead(cx, &v);
    let back = bounds(cx, window, "insights-subject-back");
    v.update(cx, |i, cx| {
        i.open_conclusion = Some(id);
        cx.notify();
    });
    cx.run_until_parked();
    let pushed = bounds(cx, window, "insights-subject-back");
    assert_eq!(
        pushed.left(),
        back.left(),
        "opening the drawer must not move the page"
    );
    // (2400 + 1280) / 2 − 48 − 2400 × 0.56 + 24 = 472
    let push = v.read_with(cx, |i, _| super::subjects::drawer_push(i));
    assert_eq!(push, px(472.));
    v.update(cx, |i, _| i.width = px(699.));
    assert_eq!(
        v.read_with(cx, |i, _| super::subjects::drawer_push(i)),
        px(0.),
        "a sheet pushes nothing"
    );
}
