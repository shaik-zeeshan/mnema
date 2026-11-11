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

pub(super) fn init(cx: &mut TestAppContext) {
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

/// Real rust.log lines (our own writer's shape), many more than the window holds.
struct LogTail;

impl DebugBackend for LogTail {
    fn fetch(&self, s: Section, _: i64) -> DebugData {
        let mut d = DebugData::default();
        if s == Section::Logs {
            let lines: Vec<String> = (0..300)
                .map(|i| {
                    format!(
                        "[2026-11-10T13:48:{:02}.527282Z][INFO][mnema_gpui] line {i}",
                        i % 60
                    )
                })
                .collect();
            d.log_tail = Some(json!({ "exists": true, "lines": lines }));
        }
        d
    }
}

#[gpui_kit::test]
fn the_log_tail_is_one_row_per_line_and_scrolls(cx: &mut TestAppContext) {
    use gpui_kit::{point, px, ScrollDelta, ScrollWheelEvent};
    init(cx);
    let b: Arc<dyn DebugBackend> = Arc::new(LogTail);
    let (view, vcx) = cx.add_window_view(|window, cx| DebugWindow::new(b, window, cx));
    view.update(vcx, |w, cx| {
        w.go(super::debug::model::Tab::Logs, cx);
        w.follow = false;
    });
    vcx.run_until_parked();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let scroll = view.read_with(vcx, |w, _| w.log_scroll.clone());
    // 300 rows + the footer (a merged tail was 1 row + footer).
    assert!(scroll.bounds_for_item(300).is_some(), "one row per line");
    assert_eq!(scroll.offset().y, px(0.));
    vcx.simulate_event(ScrollWheelEvent {
        position: scroll.bounds().center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-400.))),
        ..Default::default()
    });
    vcx.run_until_parked();
    assert!(
        scroll.offset().y < px(0.),
        "the wheel moves the list: {:?}",
        scroll.offset().y
    );
}

// ── The processor drill-in (Pipeline › <processor> › #job) ─────────────────

use super::debug::data::{FixtureDebug, JobCall};
use super::debug::detail::SPECS;
use super::debug::model::Tab;
use crate::ui::nid;

pub(super) fn debug_map() -> Value {
    serde_json::from_str(include_str!(
        "../../../desktop/parity/fixtures/debug/default.json"
    ))
    .unwrap()
}

pub(super) fn drill(
    cx: &mut TestAppContext,
    map: Value,
) -> (
    Entity<DebugWindow>,
    &mut VisualTestContext,
    Arc<FixtureDebug>,
) {
    init(cx);
    cx.update(|cx| crate::ui::clock::freeze(Some(app_core::fixtures::fixture_now()), cx));
    let backend = Arc::new(FixtureDebug::new(map));
    let b: Arc<dyn DebugBackend> = backend.clone();
    let (view, vcx) = cx.add_window_view(|window, cx| DebugWindow::new(b, window, cx));
    // Tall enough that every row is laid out (hit-tested) without scrolling.
    vcx.simulate_resize(gpui_kit::size(gpui_kit::px(520.), gpui_kit::px(1600.)));
    view.update(vcx, |w, cx| w.go(Tab::Pipeline, cx));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    (view, vcx, backend)
}

pub(super) fn click(vcx: &mut VisualTestContext, id: gpui_kit::ElementId) {
    vcx.update(|window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    });
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
}

pub(super) fn has(vcx: &mut VisualTestContext, id: impl Into<gpui_kit::ElementId>) -> bool {
    let id = id.into();
    vcx.update(|window, _| window.try_find(id).is_some())
}

fn lists(b: &FixtureDebug) -> Vec<JobCall> {
    b.calls()
        .into_iter()
        .filter(|c| matches!(c, JobCall::List { .. }))
        .collect()
}

#[gpui_kit::test]
fn every_processor_row_drills_into_its_own_lane(cx: &mut TestAppContext) {
    let (view, vcx, backend) = drill(cx, debug_map());
    for (i, spec) in SPECS.iter().enumerate() {
        let before = lists(&backend).len();
        click(vcx, nid("dv-proc", i as u64));
        assert_eq!(view.read_with(vcx, |w, _| w.detail.spec), Some(*spec));
        assert!(
            has(vcx, "dv-crumb-pipeline"),
            "{}: the breadcrumb",
            spec.key
        );
        match spec.processor {
            Some(processor) => {
                assert_eq!(
                    lists(&backend)[before..].first(),
                    Some(&JobCall::List {
                        processor: processor.into(),
                        status: None,
                        limit: 25
                    }),
                    "{}: one page of its own lane",
                    spec.key
                );
                assert!(has(vcx, "dv-job-filter"));
                assert!(has(vcx, nid("dv-job", 48213907)), "{}: the rows", spec.key);
            }
            None => {
                assert_eq!(
                    lists(&backend).len(),
                    before,
                    "the meaning index has no jobs"
                );
                assert!(!has(vcx, "dv-job-filter"));
            }
        }
        click(vcx, "dv-back".into());
        assert!(!view.read_with(vcx, |w, _| w.drilled()));
        assert!(
            has(vcx, nid("dv-proc", i as u64)),
            "back on the Pipeline tab"
        );
    }
}

#[gpui_kit::test]
fn the_jobs_poll_runs_every_2s_and_a_filter_refetches(cx: &mut TestAppContext) {
    let (view, vcx, backend) = drill(cx, debug_map());
    click(vcx, nid("dv-proc", 1));
    assert_eq!(lists(&backend).len(), 1);
    vcx.executor().advance_clock(Duration::from_millis(1999));
    vcx.run_until_parked();
    assert_eq!(lists(&backend).len(), 1, "not again before 2 s");
    vcx.executor().advance_clock(Duration::from_millis(1));
    vcx.run_until_parked();
    assert_eq!(lists(&backend).len(), 2, "again at 2 s");
    // Retrying shares the wire status with queued: a wider page, split here.
    click(vcx, nid("dv-job-filter-opt", 3));
    assert_eq!(
        lists(&backend).last(),
        Some(&JobCall::List {
            processor: "audio_transcription".into(),
            status: Some("queued".into()),
            limit: 100
        })
    );
    let shown: Vec<i64> = view.read_with(vcx, |w, cx| {
        let now = crate::ui::clock::now_ms(cx);
        w.detail
            .visible(now)
            .iter()
            .map(|j| j["id"].as_i64().unwrap())
            .collect()
    });
    assert_eq!(shown, vec![48213907], "only the job serving a backoff");
}

#[gpui_kit::test]
fn zero_jobs_say_so(cx: &mut TestAppContext) {
    let mut map = debug_map();
    map["list_processing_jobs_by_processor"] = json!({ "jobs": [], "total": 0 });
    let (_, vcx, _) = drill(cx, map);
    click(vcx, nid("dv-proc", 2));
    assert!(has(vcx, "dv-jobs-empty"));
    assert!(!has(vcx, nid("dv-job", 48213907)));
}

#[gpui_kit::test]
fn a_failed_read_shows_its_error_in_place(cx: &mut TestAppContext) {
    let mut map = debug_map();
    map["list_processing_jobs_by_processor"] =
        json!({ "__reject": "failed to list processing jobs: database is locked" });
    let (view, vcx, _) = drill(cx, map);
    click(vcx, nid("dv-proc", 0));
    assert!(has(vcx, "dv-jobs-empty"));
    assert_eq!(
        view.read_with(vcx, |w, _| w.detail.error.clone())
            .as_deref(),
        Some("failed to list processing jobs: database is locked")
    );
}

#[gpui_kit::test]
fn a_failed_job_at_its_ceiling_offers_reprocess_not_retry(cx: &mut TestAppContext) {
    let (view, vcx, backend) = drill(cx, debug_map());
    click(vcx, nid("dv-proc", 1));
    click(vcx, nid("dv-job", 48213871));
    assert_eq!(
        view.read_with(vcx, |w, _| w
            .detail
            .inspected
            .as_ref()
            .map(|j| j["id"].clone())),
        Some(json!(48213871))
    );
    assert!(has(vcx, "dv-insp-error"), "the multi-line error, in full");
    assert!(has(vcx, "dv-reprocess"));
    assert!(!has(vcx, "dv-retry-now"), "a failed job is terminal");
    click(vcx, "dv-reprocess".into());
    assert!(backend.calls().contains(&JobCall::Reprocess {
        processor: "audio_transcription".into(),
        subject_id: 2290871
    }));
    assert_eq!(
        view.read_with(vcx, |w, _| w.detail.message.clone()),
        Some((true, "Queued again".into()))
    );
}

#[gpui_kit::test]
fn retry_now_calls_the_backend_once_and_refreshes(cx: &mut TestAppContext) {
    let (view, vcx, backend) = drill(cx, debug_map());
    click(vcx, nid("dv-proc", 1));
    click(vcx, nid("dv-job", 48213907));
    assert!(!has(vcx, "dv-reprocess"), "a waiting job isn't reprocessed");
    let before = lists(&backend).len();
    click(vcx, "dv-retry-now".into());
    let retries: Vec<_> = backend
        .calls()
        .into_iter()
        .filter(|c| matches!(c, JobCall::RetryNow(_)))
        .collect();
    assert_eq!(retries, vec![JobCall::RetryNow(48213907)]);
    assert_eq!(lists(&backend).len(), before + 1, "the page is re-read");
    assert_eq!(
        view.read_with(vcx, |w, _| w.detail.message.clone()),
        Some((true, "Will try again now".into()))
    );
    assert!(has(vcx, "dv-insp-message"));
}

#[gpui_kit::test]
fn a_refused_retry_shows_the_backend_words_in_place(cx: &mut TestAppContext) {
    let mut map = debug_map();
    let refusal = "Only a waiting job can be retried now. Use Reprocess for a failed job.";
    map["retry_processing_job_now"] = json!({ "__reject": refusal });
    let (view, vcx, _) = drill(cx, map);
    click(vcx, nid("dv-proc", 1));
    click(vcx, nid("dv-job", 48213907));
    click(vcx, "dv-retry-now".into());
    assert_eq!(
        view.read_with(vcx, |w, _| w.detail.message.clone()),
        Some((false, refusal.into()))
    );
    assert!(has(vcx, "dv-insp-message"));
    assert!(
        view.read_with(vcx, |w, _| w.detail.inspected.is_some()),
        "still on the job"
    );
}

#[gpui_kit::test]
fn enter_drills_in_and_esc_walks_back_to_the_row_it_came_from(cx: &mut TestAppContext) {
    let (view, vcx, _) = drill(cx, debug_map());
    view.update_in(vcx, |w, window, cx| window.focus(&w.proc_focus[1], cx));
    press(vcx, "enter");
    assert_eq!(
        view.read_with(vcx, |w, _| w.detail.spec.map(|s| s.key)),
        Some("transcription")
    );
    click(vcx, nid("dv-job", 48213871));
    press(vcx, "escape");
    view.read_with(vcx, |w, _| {
        assert!(w.detail.inspected.is_none(), "Esc closes the job first");
        assert!(w.drilled(), "and stays on the processor");
    });
    press(vcx, "escape");
    view.update_in(vcx, |w, window, _| {
        assert!(!w.drilled());
        assert_eq!(w.tab, Tab::Pipeline);
        assert!(
            w.proc_focus[1].is_focused(window),
            "focus is back on its row"
        );
    });
}

#[gpui_kit::test]
fn an_unreachable_cloud_provider_says_so_and_tests_it(cx: &mut TestAppContext) {
    let mut map = debug_map();
    map["get_recording_settings"]["transcription"]["provider"] = json!("deepgram");
    if let Some(jobs) = map["list_processing_jobs_by_processor"]["jobs"].as_array_mut() {
        jobs[0]["lastError"] = json!("deepgram: connection refused");
        jobs[0]["payloadJson"] = json!("{\"provider\":\"deepgram\"}");
    }
    let (view, vcx, backend) = drill(cx, map);
    // Settings arrive with the 30 s housekeeping read.
    vcx.run_until_parked();
    click(vcx, nid("dv-proc", 1));
    assert!(has(vcx, "dv-test-provider"));
    click(vcx, "dv-test-provider".into());
    assert!(backend.calls().contains(&JobCall::TestDeepgram));
    assert_eq!(
        view.read_with(vcx, |w, _| w.detail.test.clone()),
        Some(Ok(()))
    );
    assert!(has(vcx, "dv-test-result"));
}
