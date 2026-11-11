//! Flow parity for Insights (Svelte = spec): each test clicks or types on the
//! real view and checks what the backend was asked and what is drawn.

use std::time::Duration;

use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, ElementId, Entity, TestAppContext, WindowHandle};

use super::read::Of;
use super::tests::{advance, open, open_at};

const TALL: (f32, f32) = (1280., 2600.);
use super::{Insights, Tab};
use crate::ui::nid;

fn click(cx: &mut TestAppContext, w: WindowHandle<Root>, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn shows(cx: &mut TestAppContext, w: WindowHandle<Root>, id: impl Into<ElementId>) -> bool {
    let id = id.into();
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

fn tab(cx: &mut TestAppContext, v: &Entity<Insights>, t: Tab) {
    v.update(cx, |i, cx| i.set_tab(t, cx));
    cx.run_until_parked();
}

fn count(b: &super::fixture::FixtureBackend, call: &str) -> usize {
    b.calls().iter().filter(|c| c.as_str() == call).count()
}

#[gpui_kit::test]
fn journal_reread_asks_for_a_fresh_digest(cx: &mut TestAppContext) {
    // It was drawn with no handler: clicking did nothing.
    let (w, v, b) = open(cx);
    tab(cx, &v, Tab::Journal);
    assert_eq!(count(&b, "digest:day:true"), 0);
    click(cx, w, "insights-journal-reread");
    assert_eq!(count(&b, "digest:day:true"), 1);
    assert!(!v.read_with(cx, |i, _| i.read_of(Of::Journal).regenerating));
}

#[gpui_kit::test]
fn a_failed_reread_keeps_the_read_and_offers_retry(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx);
    assert!(v.read_with(cx, |i, _| i.read_of(Of::Overview).digest.is_some()));
    b.fail("digest:week:true");
    click(cx, w, "insights-reread");
    v.read_with(cx, |i, _| {
        let r = i.read_of(Of::Overview);
        assert!(r.digest.is_some(), "the read we had stays (OV-06/07)");
        assert!(r.error.is_some() && !r.regenerating);
    });
    assert!(
        shows(cx, w, "insights-reread-retry"),
        "Couldn't re-read … Retry"
    );
    click(cx, w, "insights-reread-retry");
    assert_eq!(count(&b, "digest:week:true"), 2);
}

#[gpui_kit::test]
fn a_period_step_debounces_the_read_and_never_shows_the_old_numbers(cx: &mut TestAppContext) {
    let (_, v, b) = open(cx);
    let before = count(&b, "digest:week:false");
    v.update(cx, |i, cx| {
        let back = i.period.0 - time::Duration::days(7);
        i.set_period(back, crate::ui::calendar::Unit::Week, cx);
        assert!(
            i.overview.as_ref().is_some_and(|o| o.stale),
            "skeletons, not last week's numbers"
        );
        assert!(i.read_of(Of::Overview).digest.is_none() && i.read_of(Of::Overview).loading);
    });
    advance(cx, Duration::from_millis(499));
    assert_eq!(
        count(&b, "digest:week:false"),
        before,
        "still inside the debounce"
    );
    advance(cx, Duration::from_millis(1));
    assert_eq!(count(&b, "digest:week:false"), before + 1);
    assert!(v.read_with(cx, |i, _| i.overview.as_ref().is_some_and(|o| !o.stale)));
}

fn type_draft(cx: &mut TestAppContext, w: WindowHandle<Root>, v: &Entity<Insights>, text: &str) {
    cx.update_window(w.into(), |_, window, cx| {
        v.update(cx, |i, cx| {
            i.composer.update(cx, |s, cx| s.focus(window, cx))
        });
        window.render_frame(cx);
        window.input(text, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn draft(cx: &mut TestAppContext, v: &Entity<Insights>) -> String {
    v.read_with(cx, |i, cx| i.composer.read(cx).value().to_string())
}

#[gpui_kit::test]
fn a_failed_add_keeps_the_typed_statement(cx: &mut TestAppContext) {
    // The composer used to clear before the add landed, so "Your text is still
    // here" was false and the statement was lost.
    let (w, v, b) = open_at(cx, TALL);
    tab(cx, &v, Tab::Context);
    b.fail("add:");
    type_draft(cx, w, &v, "I review PRs on Fridays.");
    click(cx, w, "insights-add-context");
    assert_eq!(draft(cx, &v), "I review PRs on Fridays.");
    assert!(v.read_with(cx, |i, _| i.composer_error.is_some()));
    assert!(shows(cx, w, "insights-add-retry"));
}

#[gpui_kit::test]
fn cmd_enter_adds_the_statement_and_clears_the_draft(cx: &mut TestAppContext) {
    // ⌘↵ was drawn on the button but nothing handled it.
    let (w, v, b) = open_at(cx, TALL);
    tab(cx, &v, Tab::Context);
    type_draft(cx, w, &v, "Goals: ship the GPUI app.");
    cx.update_window(w.into(), |_, window, cx| window.press("cmd-enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(b
        .calls()
        .contains(&"add:Goals: ship the GPUI app.".to_string()));
    assert_eq!(draft(cx, &v), "");
}

#[gpui_kit::test]
fn the_draft_stops_at_280_characters(cx: &mut TestAppContext) {
    let (w, v, _) = open_at(cx, TALL);
    tab(cx, &v, Tab::Context);
    type_draft(cx, w, &v, &"x".repeat(300));
    assert_eq!(draft(cx, &v).chars().count(), 280);
}

#[gpui_kit::test]
fn edit_saves_in_place_and_cancel_drops_it(cx: &mut TestAppContext) {
    // There was no way to edit a statement.
    let (w, v, b) = open_at(cx, TALL);
    tab(cx, &v, Tab::Context);
    click(cx, w, nid("insights-stmt-edit", 1));
    assert_eq!(v.read_with(cx, |i, _| i.ctx_ui.editing), Some(1));
    click(cx, w, "insights-edit-cancel");
    assert_eq!(v.read_with(cx, |i, _| i.ctx_ui.editing), None);
    click(cx, w, nid("insights-stmt-edit", 1));
    cx.update_window(w.into(), |_, window, cx| {
        v.update(cx, |i, cx| {
            let ed = i.ctx_ui.editor.clone().unwrap();
            ed.update(cx, |s, cx| {
                s.set_value("I'm the only engineer.", window, cx)
            });
        })
    })
    .unwrap();
    click(cx, w, nid("insights-edit-topic", 3));
    click(cx, w, "insights-edit-save");
    assert!(b
        .calls()
        .contains(&"update:1:I'm the only engineer.".to_string()));
    assert_eq!(v.read_with(cx, |i, _| i.ctx_ui.editing), None);
}

#[gpui_kit::test]
fn a_failed_edit_stays_open(cx: &mut TestAppContext) {
    let (w, v, b) = open_at(cx, TALL);
    tab(cx, &v, Tab::Context);
    b.fail("update:");
    click(cx, w, nid("insights-stmt-edit", 1));
    click(cx, w, "insights-edit-save");
    v.read_with(cx, |i, _| {
        assert_eq!(i.ctx_ui.editing, Some(1), "the edit stays for another try");
        assert!(!i.ctx_ui.saving);
    });
}

#[gpui_kit::test]
fn a_failed_restore_says_why_on_the_archive(cx: &mut TestAppContext) {
    let (w, v, b) = open_at(cx, TALL);
    tab(cx, &v, Tab::Context);
    b.fail("restore:");
    click(cx, w, nid("insights-restore", 0));
    v.read_with(cx, |i, _| {
        assert!(i.ctx_ui.dismissed_error.is_some());
        assert!(i.ctx_ui.restoring.is_none());
    });
}

#[gpui_kit::test]
fn tagging_shows_on_success_and_a_failure_leaves_it_untagged(cx: &mut TestAppContext) {
    let (w, v, b) = open_at(cx, TALL);
    let untagged = |cx: &mut TestAppContext| {
        v.read_with(cx, |i, _| {
            let mut a: Vec<_> = i
                .overview
                .as_ref()
                .unwrap()
                .activities
                .iter()
                .filter(|a| a.category.is_none())
                .map(|a| (a.started_at_ms, a.id))
                .collect();
            a.sort();
            a.into_iter().rev().map(|x| x.1).collect::<Vec<_>>()
        })
    };
    let first = untagged(cx)[0];
    b.fail("category:");
    click(cx, w, nid(&format!("insights-tag-{first}"), 0));
    assert!(
        untagged(cx).contains(&first),
        "a failed correction isn't shown as done"
    );
    assert!(v.read_with(cx, |i, _| i.ov_ui.correcting.is_empty()));
}

#[gpui_kit::test]
fn the_category_modal_expands_threads_and_adjusts_focus(cx: &mut TestAppContext) {
    let (w, v, b) = open_at(cx, TALL);
    click(cx, w, nid("insights-cat", 0));
    assert!(shows(cx, w, "insights-modal"));
    click(cx, w, nid("insights-thread", 0));
    let id = v.update(cx, |i, cx| {
        let off = crate::ui::clock::now(cx).offset();
        let d = i.overview.clone().unwrap();
        let f = super::overview::figures(i, &d, cx);
        super::overview::threads(&f.acts, off)[0].acts[0].id
    });
    click(cx, w, nid("insights-adjust", id as u64));
    assert_eq!(v.read_with(cx, |i, _| i.ov_ui.adjusting), Some(id));
    // Esc closes the popover first, then the modal.
    v.update(cx, |i, cx| assert!(i.close_top(cx)));
    assert!(v.read_with(cx, |i, _| i.ov_ui.adjusting.is_none() && i.modal.is_some()));
    v.update(cx, |i, cx| {
        i.correct(id, super::overview::Fix::Focus(None), cx)
    });
    cx.run_until_parked();
    assert!(b.calls().contains(&format!("focus:{id}:None")));
}

#[gpui_kit::test]
fn the_ask_hand_off_scopes_chat_to_the_period(cx: &mut TestAppContext) {
    let (_, v, _) = open(cx);
    let (scope, period) = v.read_with(cx, |i, _| (i.ask_scope(), i.period));
    assert_eq!(
        scope,
        Some(crate::ui::calendar::period_of(period.0, period.1))
    );
    super::tests::open_lead(cx, &v);
    assert_eq!(
        v.read_with(cx, |i, _| i.ask_scope()),
        None,
        "a subject's Ask keeps Chat's scope"
    );
}

#[gpui_kit::test]
fn typing_in_the_filter_on_a_subject_page_returns_to_the_index(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx);
    super::tests::open_lead(cx, &v);
    cx.update_window(w.into(), |_, window, cx| {
        v.update(cx, |i, cx| i.filter.update(cx, |s, cx| s.focus(window, cx)));
        window.render_frame(cx);
        window.input("rust", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(v.read_with(cx, |i, _| i.subject.is_none()));
}

#[gpui_kit::test]
fn each_tier_pages_and_its_size_menu_keeps_the_first_row(cx: &mut TestAppContext) {
    // The list only ever drew a tier's first ten rows.
    let (w, v, _) = open_at(cx, TALL);
    tab(cx, &v, Tab::Subjects);
    v.update(cx, |i, cx| {
        let d = i.subjects.as_mut().unwrap();
        let lead = d
            .conclusions
            .iter()
            .max_by(|a, b| a.confidence.total_cmp(&b.confidence))
            .unwrap()
            .clone();
        for n in 0..14 {
            let mut c = lead.clone();
            c.id = 10_000 + n;
            c.subject = format!("Subject {n:02}");
            d.conclusions.push(c);
        }
        cx.notify();
    });
    cx.run_until_parked();
    let key = "Conviction:strong";
    let page =
        |cx: &mut TestAppContext| v.read_with(cx, |i, _| super::subjects::pager::state(i, key, 10));
    click(cx, w, format!("insights-pager-{key}-next"));
    assert_eq!(page(cx), (1, 10));
    // ← turns it back from the keyboard.
    v.update(cx, |i, cx| assert!(super::subjects::turn_list(i, -1, cx)));
    assert_eq!(page(cx).0, 0);
    click(cx, w, format!("insights-pager-{key}-range"));
    click(cx, w, nid(&format!("insights-pager-{key}-size"), 1));
    assert_eq!(page(cx), (0, 25));
    assert!(v.read_with(cx, |i, _| i.pager_menu.is_none()));
}

#[gpui_kit::test]
fn dismissing_the_open_conclusion_moves_to_its_neighbour(cx: &mut TestAppContext) {
    let (_, v, _) = open(cx);
    let id = super::tests::open_lead(cx, &v);
    let next = v.update(cx, |i, cx| {
        i.open_conclusion = Some(id);
        let flat = super::subjects::flat_conclusions(i);
        i.dismiss(id, cx);
        flat[1].id
    });
    assert_eq!(v.read_with(cx, |i, _| i.open_conclusion), Some(next));
    v.update(cx, |i, cx| i.undo_dismiss(id, cx));
    assert_eq!(
        v.read_with(cx, |i, _| i.open_conclusion),
        Some(id),
        "Undo reopens it"
    );
}
