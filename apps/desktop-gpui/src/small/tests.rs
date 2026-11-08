//! Layer 2 for the small windows: the access request's anti-reflex keyboard
//! (focus on Deny, Esc denies, Enter presses nothing, closing answers
//! `closed`), the permission helper's 1 s poll and close beat, and the
//! developer window's poll cadence and pause.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use capture_types::AppearanceSetting;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, TestAppContext, VisualTestContext, WindowAppearance};
use serde_json::{json, Value};

use super::access::model::Scope;
use super::access::{self, AccessBackend, AccessRequest, Call, FixtureAccess, Phase};
use super::debug::data::{DebugBackend, DebugData, Section};
use super::debug::DebugWindow;
use super::helper::{GrantCheck, PermKind, PermissionHelper, CLOSE_AFTER_GRANT, POLL};
use crate::theme;

const MS: Duration = Duration::from_millis(1);

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
    });
}

fn request() -> Value {
    json!({
        "requestId": "req-1", "client": { "label": "Claude Code", "source": "inferred" },
        "command": "search --query \"pricing\"", "minimumScope": "lastDay",
        "preferredScope": "last7Days", "currentScope": null, "createdAt": "2026-11-04T09:01:30Z"
    })
}

fn access(
    cx: &mut TestAppContext,
) -> (
    Entity<AccessRequest>,
    &mut VisualTestContext,
    Arc<FixtureAccess>,
) {
    init(cx);
    let backend = Arc::new(FixtureAccess::new(Some(request())));
    let b: Arc<dyn AccessBackend> = backend.clone();
    let (view, vcx) = cx.add_window_view(|window, cx| {
        access::answer_closed_on_dismiss(b.clone(), window, cx);
        AccessRequest::new(b.clone(), window, cx)
    });
    vcx.run_until_parked();
    (view, vcx, backend)
}

fn answers(b: &FixtureAccess) -> Vec<Call> {
    b.calls()
        .into_iter()
        .filter(|c| *c != Call::Pending)
        .collect()
}

#[gpui_kit::test]
fn access_focus_starts_on_deny_and_the_narrowest_scope(cx: &mut TestAppContext) {
    let (view, vcx, _) = access(cx);
    view.update_in(vcx, |v, window, _| {
        assert!(matches!(v.phase, Phase::Request(_)));
        assert!(v.deny_focus.is_focused(window), "focus must land on Deny");
        assert_eq!(
            v.selected,
            Scope::LastDay,
            "pre-selects the minimum, never the ask"
        );
    });
}

/// A real key press on the focused element: down then up (keyboard click fires on up).
fn press(vcx: &mut VisualTestContext, key: &'static str) {
    let keystroke = gpui_kit::Keystroke::parse(key).unwrap();
    vcx.update(|window, cx| window.render_frame(cx));
    vcx.simulate_event(gpui_kit::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    vcx.simulate_event(gpui_kit::KeyUpEvent { keystroke });
    vcx.run_until_parked();
}

#[gpui_kit::test]
fn access_enter_presses_nothing_not_even_the_focused_deny(cx: &mut TestAppContext) {
    let (view, vcx, backend) = access(cx);
    press(vcx, "enter");
    assert!(
        answers(&backend).is_empty(),
        "Enter answered: {:?}",
        backend.calls()
    );
    view.update(vcx, |v, _| assert!(matches!(v.phase, Phase::Request(_))));
    // Control: the same press with Space does activate the focused Deny.
    press(vcx, "space");
    assert_eq!(answers(&backend), vec![Call::Deny { closed: false }]);
}

#[gpui_kit::test]
fn access_esc_denies(cx: &mut TestAppContext) {
    let (_, vcx, backend) = access(cx);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert_eq!(answers(&backend), vec![Call::Deny { closed: false }]);
}

#[gpui_kit::test]
fn access_closing_the_window_answers_closed(cx: &mut TestAppContext) {
    let (_, vcx, backend) = access(cx);
    assert!(vcx.simulate_close());
    vcx.run_until_parked();
    assert_eq!(answers(&backend), vec![Call::Deny { closed: true }]);
}

#[gpui_kit::test]
fn access_allow_sends_the_selected_scope_then_shows_the_receipt(cx: &mut TestAppContext) {
    let (view, vcx, backend) = access(cx);
    view.update_in(vcx, |v, window, cx| {
        v.select(Scope::Last7Days, cx);
        v.approve(window, cx);
    });
    vcx.run_until_parked();
    assert_eq!(
        answers(&backend),
        vec![Call::Approve("req-1".into(), Scope::Last7Days)]
    );
    view.update_in(vcx, |v, window, _| {
        assert!(matches!(v.phase, Phase::Receipt(_)));
        assert!(
            v.close_focus.is_focused(window),
            "focus follows into the receipt's Close"
        );
    });
    // The receipt's Esc closes; it never answers twice.
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert_eq!(answers(&backend).len(), 1);
}

#[gpui_kit::test]
fn access_a_stale_request_leaves_only_close(cx: &mut TestAppContext) {
    init(cx);
    let backend = Arc::new(FixtureAccess {
        fail: Some("no pending CLI Access request matches this approval".into()),
        ..FixtureAccess::new(Some(request()))
    });
    let b: Arc<dyn AccessBackend> = backend.clone();
    let (view, vcx) = cx.add_window_view(|window, cx| AccessRequest::new(b, window, cx));
    vcx.run_until_parked();
    view.update_in(vcx, |v, window, cx| v.approve(window, cx));
    vcx.run_until_parked();
    view.update(vcx, |v, _| {
        assert!(v.gone);
        assert!(v
            .error
            .as_deref()
            .unwrap_or("")
            .starts_with("This request is no longer valid"));
    });
}

struct Flip {
    granted: AtomicBool,
    checks: AtomicUsize,
}

impl GrantCheck for Flip {
    fn granted(&self, _: PermKind) -> Result<bool, String> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        Ok(self.granted.load(Ordering::SeqCst))
    }
}

#[gpui_kit::test]
fn helper_polls_every_second_and_closes_after_the_grant(cx: &mut TestAppContext) {
    init(cx);
    let check = Arc::new(Flip {
        granted: AtomicBool::new(false),
        checks: AtomicUsize::new(0),
    });
    let c: Arc<dyn GrantCheck> = check.clone();
    let closed = Arc::new(AtomicBool::new(false));
    let (view, vcx) = cx.add_window_view(|_, cx| PermissionHelper::new(PermKind::Screen, c, cx));
    let flag = closed.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, _: &super::helper::Closed, _| {
            flag.store(true, Ordering::SeqCst)
        })
        .detach()
    });
    vcx.run_until_parked();
    assert_eq!(check.checks.load(Ordering::SeqCst), 1, "checks on open");
    check.granted.store(true, Ordering::SeqCst);
    vcx.executor().advance_clock(POLL - MS);
    vcx.run_until_parked();
    assert_eq!(
        check.checks.load(Ordering::SeqCst),
        1,
        "no second check at 999 ms"
    );
    view.update(vcx, |h, _| assert!(!h.granted));
    vcx.executor().advance_clock(MS);
    vcx.run_until_parked();
    assert_eq!(
        check.checks.load(Ordering::SeqCst),
        2,
        "second check at 1000 ms"
    );
    view.update(vcx, |h, _| assert!(h.granted));
    vcx.executor().advance_clock(CLOSE_AFTER_GRANT - MS);
    vcx.run_until_parked();
    assert!(!closed.load(Ordering::SeqCst), "still open at 1399 ms");
    vcx.executor().advance_clock(MS);
    vcx.run_until_parked();
    assert!(
        closed.load(Ordering::SeqCst),
        "closes itself 1400 ms after the grant"
    );
    vcx.executor().advance_clock(POLL * 3);
    vcx.run_until_parked();
    assert_eq!(
        check.checks.load(Ordering::SeqCst),
        2,
        "polling stops once granted"
    );
}

#[derive(Default)]
struct Counting(std::sync::Mutex<Vec<Section>>);

impl DebugBackend for Counting {
    fn fetch(&self, s: Section, _: i64) -> DebugData {
        self.0.lock().unwrap().push(s);
        DebugData::default()
    }
}

impl Counting {
    fn n(&self, s: Section) -> usize {
        self.0.lock().unwrap().iter().filter(|x| **x == s).count()
    }
}

#[gpui_kit::test]
fn debug_health_polls_every_2s_and_pause_stops_it(cx: &mut TestAppContext) {
    init(cx);
    let backend = Arc::new(Counting::default());
    let b: Arc<dyn DebugBackend> = backend.clone();
    let (view, vcx) = cx.add_window_view(|window, cx| DebugWindow::new(b, window, cx));
    vcx.run_until_parked();
    assert_eq!(backend.n(Section::Health), 1, "health reads on open");
    vcx.executor().advance_clock(Duration::from_millis(1999));
    vcx.run_until_parked();
    assert_eq!(backend.n(Section::Health), 1, "not again before 2 s");
    vcx.executor().advance_clock(Duration::from_millis(1));
    vcx.run_until_parked();
    assert_eq!(backend.n(Section::Health), 2, "again at 2 s");
    view.update(vcx, |w, cx| w.go(super::debug::model::Tab::Capture, cx));
    vcx.run_until_parked();
    assert_eq!(
        backend.n(Section::Capture),
        1,
        "a tab's read starts when it opens"
    );
    view.update(vcx, |w, cx| w.toggle_live(cx));
    vcx.executor().advance_clock(Duration::from_secs(10));
    vcx.run_until_parked();
    assert_eq!(backend.n(Section::Health), 2, "paused: no polls");
    view.update(vcx, |w, cx| w.toggle_live(cx));
    vcx.run_until_parked();
    assert_eq!(backend.n(Section::Health), 3, "resume refreshes at once");
}
