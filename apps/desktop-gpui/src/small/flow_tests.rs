//! Flows found missing against Svelte: the access request swapping to a waiting
//! successor, the update window's out-of-window Refresh / Renew lines, and the
//! developer window's failed reads.

use std::sync::Arc;

use capture_types::{Activation, LicenseStatus};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{TestAppContext, VisualTestContext};
use serde_json::{json, Value};

use super::access::{AccessBackend, AccessRequest, Call, FixtureAccess, Phase};
use super::debug::model::Tab;
use super::tests::{click, debug_map, drill, init};
use super::update::{RenewNote, UpdateWindow};

fn request(id: &str, tool: &str) -> Value {
    json!({
        "requestId": id, "client": { "label": tool, "source": "inferred" },
        "command": "search --query \"x\"", "minimumScope": "lastDay",
        "preferredScope": "last7Days", "currentScope": null, "createdAt": "2026-11-04T09:01:30Z"
    })
}

#[gpui_kit::test]
fn access_a_gone_request_gives_way_to_the_one_now_waiting(cx: &mut TestAppContext) {
    init(cx);
    let backend = Arc::new(FixtureAccess {
        fail: Some("no pending CLI Access request matches this approval".into()),
        successor: Some(request("req-2", "Codex")),
        ..FixtureAccess::new(Some(request("req-1", "Claude Code")))
    });
    let b: Arc<dyn AccessBackend> = backend.clone();
    let (view, vcx) = cx.add_window_view(|window, cx| AccessRequest::new(b, window, cx));
    vcx.run_until_parked();
    view.update_in(vcx, |v, window, cx| v.approve(window, cx));
    vcx.run_until_parked();
    view.update(vcx, |v, _| {
        let Phase::Request(r) = &v.phase else {
            panic!("the successor is shown: {:?}", v.phase);
        };
        assert_eq!(r.id, "req-2");
        assert!(!v.gone, "the new request can be answered");
        assert_eq!(v.error, None);
    });
    // The dead request was the one answered.
    assert!(backend.calls().contains(&Call::Approve(
        "req-1".into(),
        super::access::model::Scope::LastDay
    )));
}

fn licensed(in_window: bool) -> LicenseStatus {
    LicenseStatus::Licensed {
        update_through_ms: 1_731_536_000_000,
        in_window,
        email: "owner@example.com".into(),
        name: "Ada Lovelace".into(),
        activation: Activation::Activated,
    }
}

fn expired(
    cx: &mut TestAppContext,
    refresh: super::update::LicenseRefresh,
) -> (gpui_kit::Entity<UpdateWindow>, &mut VisualTestContext) {
    init(cx);
    cx.update(|cx| crate::ui::clock::freeze(Some(app_core::fixtures::fixture_now()), cx));
    let status = json!({
        "state": "availableOutOfWindow",
        "app": { "version": "0.1.23" },
        "update": { "version": "0.2.0", "notes": "" }
    });
    let (view, vcx) = cx.add_window_view(|_, _| UpdateWindow {
        refresh_license: Some(refresh),
        ..UpdateWindow::new(Some(status), Some(licensed(false)))
    });
    vcx.run_until_parked();
    (view, vcx)
}

#[gpui_kit::test]
fn refresh_license_says_what_it_found(cx: &mut TestAppContext) {
    // Still out of window: the line keeps the end date.
    let (view, vcx) = expired(cx, Arc::new(|| Ok(Some(licensed(false)))));
    click(vcx, "up-refresh".into());
    view.update(vcx, |w, _| {
        assert!(!w.refreshing);
        assert_eq!(w.renew_note, Some(RenewNote::Checked));
    });
    let line = view.update(vcx, |w, cx| w.renew_line(cx));
    assert_eq!(
        line.as_deref(),
        Some("Checked just now \u{2014} your window still ends Nov 14, 2024.")
    );
}

#[gpui_kit::test]
fn a_found_renewal_says_so_and_a_failed_refresh_says_why(cx: &mut TestAppContext) {
    let (view, vcx) = expired(cx, Arc::new(|| Ok(Some(licensed(true)))));
    click(vcx, "up-refresh".into());
    let line = view.update(vcx, |w, cx| w.renew_line(cx));
    assert_eq!(
        line.as_deref(),
        Some("Renewal found \u{2014} checking for the update now.")
    );

    let (view, vcx) = expired(cx, Arc::new(|| Err("license server unreachable".into())));
    click(vcx, "up-refresh".into());
    view.update(vcx, |w, _| {
        assert_eq!(
            w.action_error.as_deref(),
            Some("license server unreachable")
        );
        assert_eq!(w.renew_note, None);
        assert!(!w.refreshing, "the button comes back");
    });
}

#[gpui_kit::test]
fn the_ai_tab_says_when_its_lists_fail_to_load(cx: &mut TestAppContext) {
    let mut map = debug_map();
    map["list_user_context_derivation_runs"] = json!({ "__reject": "database is locked" });
    map["list_cli_access_history"] = json!({ "__reject": "audit file unreadable" });
    let (view, vcx, _) = drill(cx, map);
    view.update(vcx, |w, cx| w.go(Tab::Ai, cx));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    view.update(vcx, |w, _| {
        assert_eq!(w.data.runs_error.as_deref(), Some("database is locked"));
        assert_eq!(w.data.cli_error.as_deref(), Some("audit file unreadable"));
    });
}
