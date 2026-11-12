//! Every copy affordance in the developer window, by a real click on the
//! visible control: what lands on the clipboard and the "Copied" the user sees.

use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{TestAppContext, VisualTestContext};

use super::debug::logs::{line_copy_id, line_id};
use super::debug::model::Tab;
use super::debug::readouts::Level;
use super::debug::{DebugWindow, LINE_COPIED_FOR};
use super::tests::{click, debug_map, drill, has};
use crate::ui::copy::COPIED_FOR;
use crate::ui::nid;

const ERROR_ENTRY: &str = "[2026-11-04][09:01:44][app_infra::processing::ocr][ERROR] [ocr] frame 88412 failed: image decode error\n    at app_infra::processing::ocr::run (ocr.rs:212)";

fn clipboard(vcx: &mut VisualTestContext) -> Option<String> {
    vcx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()))
}

fn label(vcx: &mut VisualTestContext, id: impl Into<gpui_kit::ElementId>) -> String {
    let id = id.into();
    vcx.update(|window, cx| {
        window.render_frame(cx);
        window.find(id).label().unwrap_or_default().to_string()
    })
}

fn logs(cx: &mut TestAppContext) -> (gpui_kit::Entity<DebugWindow>, &mut VisualTestContext) {
    let (view, vcx, _) = drill(cx, debug_map());
    view.update(vcx, |w, cx| {
        w.follow = false;
        w.go(Tab::Logs, cx);
    });
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    (view, vcx)
}

#[gpui_kit::test]
fn clicking_a_log_line_copies_it_and_marks_it_copied_for_1200ms(cx: &mut TestAppContext) {
    let (view, vcx) = logs(cx);
    click(vcx, line_id(ERROR_ENTRY, 0));
    assert_eq!(
        clipboard(vcx).as_deref(),
        Some(ERROR_ENTRY),
        "the line and its backtrace"
    );
    let copied = |vcx: &mut VisualTestContext| view.read_with(vcx, |w, _| w.copied_line.clone());
    assert_eq!(copied(vcx).as_deref(), Some(ERROR_ENTRY));
    vcx.executor()
        .advance_clock(LINE_COPIED_FOR - Duration::from_millis(1));
    vcx.run_until_parked();
    assert!(copied(vcx).is_some(), "still marked at 1199 ms");
    vcx.executor().advance_clock(Duration::from_millis(1));
    vcx.run_until_parked();
    assert_eq!(copied(vcx), None, "unmarked at 1200 ms");
}

#[gpui_kit::test]
fn the_hovered_log_line_shows_a_copy_button_that_copies_it(cx: &mut TestAppContext) {
    let (view, vcx) = logs(cx);
    assert!(!has(vcx, line_copy_id(ERROR_ENTRY)), "hidden until hovered");
    vcx.update(|window, cx| window.hover(line_id(ERROR_ENTRY, 0), cx));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(
        has(vcx, line_copy_id(ERROR_ENTRY)),
        "shown on the hovered line"
    );
    assert_eq!(
        view.read_with(vcx, |w, _| w.hovered_line.clone())
            .as_deref(),
        Some(ERROR_ENTRY),
        "only that line"
    );
    click(vcx, line_copy_id(ERROR_ENTRY));
    assert_eq!(clipboard(vcx).as_deref(), Some(ERROR_ENTRY));
    assert_eq!(label(vcx, line_copy_id(ERROR_ENTRY)), "Copied");
    assert_eq!(
        view.read_with(vcx, |w, _| w.copied_line.clone()).as_deref(),
        Some(ERROR_ENTRY),
        "the line reads copied too"
    );
}

#[gpui_kit::test]
fn copy_visible_lines_copies_what_the_filters_leave(cx: &mut TestAppContext) {
    let (view, vcx) = logs(cx);
    click(vcx, "dv-copy-visible".into());
    let all = clipboard(vcx).unwrap_or_default();
    assert_eq!(
        all.lines().count(),
        8,
        "every line, a backtrace included: {all}"
    );
    assert!(all.starts_with("[2026-11-04][08:58:02]"), "oldest first");
    assert_eq!(label(vcx, "dv-copy-visible"), "Copied");
    vcx.executor().advance_clock(COPIED_FOR);
    vcx.run_until_parked();
    view.update(vcx, |w, cx| {
        w.level = Level::Error;
        cx.notify();
    });
    vcx.run_until_parked();
    click(vcx, "dv-copy-visible".into());
    assert_eq!(clipboard(vcx).as_deref(), Some(ERROR_ENTRY), "Errors only");
}

#[gpui_kit::test]
fn a_warning_row_copies_its_message_without_opening_its_tab(cx: &mut TestAppContext) {
    let (view, vcx, _) = drill(cx, debug_map());
    view.update(vcx, |w, cx| w.go(Tab::Overview, cx));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    // "Needs a look" first; the running-fine rows have no copy button.
    assert!(has(vcx, "dv-row-1-copy"));
    // Rows number 1..=n down the groups; the last one runs fine.
    let last = view.read_with(vcx, |w, _| w.data.health.len());
    assert!(!has(
        vcx,
        gpui_kit::SharedString::from(format!("dv-row-{last}-copy"))
    ));
    click(vcx, "dv-row-1-copy".into());
    let text = clipboard(vcx).unwrap_or_default();
    assert!(text.starts_with("Reading: 1 frame couldn"), "{text}");
    assert!(text.ends_with("\u{b7} 1 failed"), "{text}");
    assert_eq!(
        view.read_with(vcx, |w, _| w.tab),
        Tab::Overview,
        "the row's own click didn't run"
    );
    assert_eq!(label(vcx, "dv-row-1-copy"), "Copied");
}

#[gpui_kit::test]
fn a_failed_jobs_error_and_its_id_copy(cx: &mut TestAppContext) {
    let (view, vcx, _) = drill(cx, debug_map());
    click(vcx, nid("dv-proc", 1));
    click(vcx, nid("dv-job", 48213871));
    let error = view.read_with(vcx, |w, _| {
        w.detail.inspected.as_ref().unwrap()["lastError"]
            .as_str()
            .unwrap()
            .to_string()
    });
    click(vcx, "dv-insp-error-copy".into());
    assert_eq!(clipboard(vcx), Some(error), "the error in full");
    assert_eq!(label(vcx, "dv-insp-error-copy"), "Copied");
    click(vcx, "dv-copy-id".into());
    assert_eq!(clipboard(vcx).as_deref(), Some("48213871"));
    assert!(
        view.read_with(vcx, |w, _| w.detail.copied),
        "Copy id → Copied"
    );
}

#[gpui_kit::test]
fn copy_diagnostics_copies_the_report_and_says_so(cx: &mut TestAppContext) {
    let (view, vcx, _) = drill(cx, debug_map());
    click(vcx, "dv-more".into());
    click(vcx, "dv-copy".into());
    let report = clipboard(vcx).unwrap_or_default();
    assert!(report.starts_with("Mnema "), "{report}");
    assert!(
        report.contains("# Health") && report.contains("# Queues"),
        "{report}"
    );
    assert!(!view.read_with(vcx, |w, _| w.menu_open), "the menu closed");
    assert_eq!(
        view.read_with(vcx, |w, cx| w.toasts.read(cx).len()),
        1,
        "the toast"
    );
    assert!(has(vcx, nid("toast", 0)));
}

#[gpui_kit::test]
fn the_more_menu_closes_on_a_press_outside_and_toggles_on_its_button(cx: &mut TestAppContext) {
    let (view, vcx, _) = drill(cx, debug_map());
    let open = |vcx: &mut gpui_kit::VisualTestContext| view.read_with(vcx, |w, _| w.menu_open);
    click(vcx, "dv-more".into());
    assert!(open(vcx), "the button opens it");
    click(vcx, "dv-more".into());
    assert!(!open(vcx), "the button closes it again");
    click(vcx, "dv-more".into());
    click(vcx, nid("dv-proc", 1));
    assert!(!open(vcx), "a press outside closes it (DevStrip.svelte)");
}
