//! Flow parity, part two: the receipt's cited audio, the day spine's link to
//! the cards and its capture runs, Subjects' refresh pill and filter wait, the
//! evidence frame peek, and "Recording since".

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{px, AppContext as _, ElementId, Entity, TestAppContext, WindowHandle};

use super::journal::audio::SegRow;
use super::tests::{advance, open, open_at, open_lead};
use super::{Insights, Tab};
use crate::timeline::audio::fixture::{FakePlayer, FixtureAudio};
use crate::timeline::audio::Deps;
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

/// The newest card's receipt, with segment `seg` (drawer fixture) in its span.
fn open_receipt(
    cx: &mut TestAppContext,
    v: &Entity<Insights>,
    b: &super::fixture::FixtureBackend,
    seg: i64,
) -> FakePlayer {
    let player = FakePlayer::new(50.);
    let audio = Arc::new(FixtureAudio::load("drawer").expect("drawer fixture"));
    v.update(cx, |i, cx| i.set_tab(Tab::Journal, cx));
    cx.run_until_parked();
    let (id, span) = v.read_with(cx, |i, _| {
        let a = i
            .journal
            .as_ref()
            .unwrap()
            .activities
            .iter()
            .max_by_key(|a| a.started_at_ms)
            .unwrap();
        (a.id, (a.started_at_ms, a.ended_at_ms))
    });
    *b.segments.lock().unwrap() = vec![SegRow {
        id: seg,
        start: span.0,
        end: span.0 + 50_000,
        mic: true,
    }];
    v.update(cx, |i, cx| {
        i.audio = Deps {
            backend: audio,
            open_player: player.factory(),
        };
        i.toggle_receipt(id, span, cx);
    });
    cx.run_until_parked();
    player
}

#[gpui_kit::test]
fn the_receipt_lists_spoken_turns_and_a_click_plays_that_turn(cx: &mut TestAppContext) {
    // 1× played nothing: the cited audio was never loaded.
    let (w, v, b) = open_at(cx, (1280., 2600.));
    let player = open_receipt(cx, &v, &b, 901);
    v.read_with(cx, |i, _| {
        let rc = i.receipt.as_ref().unwrap();
        assert_eq!(
            rc.audio.turns().len(),
            5,
            "the drawer fixture's five spoken turns"
        );
        assert_eq!(rc.speed, 1, "audio makes 1× the default");
        assert_eq!(
            rc.audio.turns()[0].speaker,
            "Ana Ruiz",
            "named live from the profiles"
        );
    });
    assert!(shows(cx, w, nid("insights-turn", 1)));
    click(cx, w, nid("insights-turn", 1));
    let (pos, playing) = player.state();
    assert!(playing, "selecting a turn relives it");
    // `clipStartOffsetSec` with no seek: the clip plays from its segment's start.
    assert_eq!(pos, 0.);
    let (sel, key) = v.read_with(cx, |i, _| {
        let a = &i.receipt.as_ref().unwrap().audio;
        (a.selected, a.turns()[1].key)
    });
    assert_eq!(sel, Some(key));
    // The frames follow the audio clock.
    player.advance(20.);
    v.update(cx, |_, cx| cx.notify());
    cx.update_window(w.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.executor()
        .advance_clock(crate::timeline::audio::transport::PLAY_POLL);
    cx.run_until_parked();
    let idx = v.read_with(cx, |i, _| i.receipt.as_ref().unwrap().index);
    assert!(idx > 0, "the strip moved with the clip");
    // The play button pauses the clip.
    click(cx, w, "insights-receipt-play");
    assert!(!player.state().1);
}

#[gpui_kit::test]
fn a_clip_whose_segment_is_gone_says_so(cx: &mut TestAppContext) {
    let (w, v, b) = open_at(cx, (1280., 2600.));
    // The fixture only knows segment 901: 902's row is gone.
    let player = open_receipt(cx, &v, &b, 902);
    click(cx, w, nid("insights-turn", 0));
    assert!(!player.state().1);
    let e = v.read_with(cx, |i, _| {
        i.receipt.as_ref().unwrap().audio.media_error.clone()
    });
    assert_eq!(e.as_deref(), Some("This clip's audio is no longer stored."));
}

#[gpui_kit::test]
fn hovering_a_spine_block_lights_its_card_and_a_click_scrolls_to_it(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx);
    v.update(cx, |i, cx| i.set_tab(Tab::Journal, cx));
    cx.run_until_parked();
    cx.update_window(w.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    // A shown card (folded sub-lines have no block of their own).
    let id = v.read_with(cx, |i, _| *i.card_bounds.borrow().keys().min().unwrap());
    let block = nid("insights-spine-block", id as u64);
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.hover(block.clone(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        v.read_with(cx, |i, _| i.hot),
        Some(id),
        "the card lights with its block"
    );
    let before = v.read_with(cx, |i, _| i.scroll.offset().y);
    click(cx, w, block);
    let after = v.read_with(cx, |i, _| i.scroll.offset().y);
    assert!(
        after < before,
        "the page scrolled down to the card ({before:?} → {after:?})"
    );
}

#[gpui_kit::test]
fn a_capture_only_day_draws_runs_with_their_top_apps(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx);
    v.update(cx, |i, cx| i.set_tab(Tab::Journal, cx));
    cx.run_until_parked();
    v.update(cx, |i, cx| {
        std::rc::Rc::make_mut(i.journal.as_mut().unwrap())
            .activities
            .clear();
        i.status.ctx.as_mut().unwrap().engine_available = false;
        cx.notify();
    });
    assert!(shows(cx, w, nid("insights-spine-run", 0)));
    cx.run_until_parked();
    let apps = v.read_with(cx, |i, _| i.run_apps.clone());
    assert!(
        apps.is_some_and(|(_, a)| a.first().is_some_and(|x| !x.is_empty())),
        "each run is labelled by its apps"
    );
}

#[gpui_kit::test]
fn a_changed_list_waits_behind_the_pill_while_scrolled_down(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx);
    v.update(cx, |i, cx| i.set_tab(Tab::Subjects, cx));
    cx.run_until_parked();
    // What's shown lacks one subject the fresh read has; the reader is scrolled down.
    v.update(cx, |i, cx| {
        let d = i.subjects.as_mut().unwrap();
        let gone = d.conclusions[0].subject.clone();
        d.conclusions.retain(|c| c.subject != gone);
        i.scroll.set_offset(gpui_kit::point(px(0.), px(-400.)));
        i.refresh_subjects(cx);
    });
    advance(cx, Duration::from_millis(499));
    assert!(
        v.read_with(cx, |i, _| i.subj_ui.staged.is_none()),
        "inside the 500 ms wait"
    );
    advance(cx, Duration::from_millis(1));
    assert_eq!(
        v.read_with(cx, |i, _| i.subj_ui.staged.as_ref().map(|s| s.1)),
        Some(1)
    );
    assert!(
        shows(cx, w, "insights-subjects-refresh"),
        "1 view updated · Refresh"
    );
    click(cx, w, "insights-subjects-refresh");
    assert!(v.read_with(cx, |i, _| i.subj_ui.staged.is_none()));
}

#[gpui_kit::test]
fn at_the_top_a_changed_list_swaps_in_on_its_own(cx: &mut TestAppContext) {
    let (_, v, _) = open(cx);
    v.update(cx, |i, cx| i.set_tab(Tab::Subjects, cx));
    cx.run_until_parked();
    let n = v.update(cx, |i, cx| {
        let d = i.subjects.as_mut().unwrap();
        let n = d.conclusions.len();
        let gone = d.conclusions[0].subject.clone();
        d.conclusions.retain(|c| c.subject != gone);
        i.refresh_subjects(cx);
        n
    });
    advance(cx, Duration::from_millis(500));
    v.read_with(cx, |i, _| {
        assert!(i.subj_ui.staged.is_none());
        assert_eq!(i.subjects.as_ref().unwrap().conclusions.len(), n);
    });
}

#[gpui_kit::test]
fn the_filter_ranks_200ms_after_typing(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx);
    v.update(cx, |i, cx| i.set_tab(Tab::Subjects, cx));
    cx.run_until_parked();
    cx.update_window(w.into(), |_, window, cx| {
        v.update(cx, |i, cx| i.filter.update(cx, |s, cx| s.focus(window, cx)));
        window.render_frame(cx);
        window.input("rust", cx);
    })
    .unwrap();
    advance(cx, Duration::from_millis(199));
    assert_eq!(v.read_with(cx, |i, _| i.subj_ui.applied.clone()), "");
    advance(cx, Duration::from_millis(1));
    assert_eq!(v.read_with(cx, |i, _| i.subj_ui.applied.clone()), "rust");
}

#[gpui_kit::test]
fn an_evidence_row_opens_its_frame_in_the_peek(cx: &mut TestAppContext) {
    let (w, v, _) = open_at(cx, (2400., 1400.));
    let id = open_lead(cx, &v);
    v.update(cx, |i, cx| {
        i.open_conclusion = Some(id);
        cx.notify();
    });
    cx.run_until_parked();
    let aid = v.read_with(cx, |i, _| {
        let d = i.detail.as_ref().unwrap();
        let c = d
            .view
            .as_ref()
            .unwrap()
            .conclusions
            .iter()
            .find(|c| c.id == id)
            .unwrap();
        c.evidence
            .iter()
            .map(|e| e.activity_id)
            .find(|a| {
                d.activities.get(a).is_some_and(|a| {
                    a.evidence
                        .first()
                        .is_some_and(|r| r.subject_type == "frame")
                })
            })
            .expect("a resolved activity citing a frame")
    });
    click(cx, w, nid("insights-ev", aid as u64));
    assert!(v
        .read_with(cx, |i, _| i.peek.src.as_ref().and_then(|s| s.frame_id))
        .is_some());
    assert!(
        shows(cx, w, "chat-frame-open"),
        "show this frame in the timeline, inside the peek"
    );
    v.update(cx, |i, cx| assert!(i.close_top(cx)));
    assert!(
        v.read_with(cx, |i, _| i.peek.src.is_none()),
        "Esc closes the peek first"
    );
    assert!(v.read_with(cx, |i, _| i.subject.is_some()));
}

#[gpui_kit::test]
fn recording_today_with_no_cards_writes_up_from_the_session_start(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx);
    v.update(cx, |i, cx| i.set_tab(Tab::Journal, cx));
    cx.run_until_parked();
    let since = v.update(cx, |i, cx| {
        std::rc::Rc::make_mut(i.journal.as_mut().unwrap())
            .activities
            .clear();
        i.recording = (true, false);
        let start = crate::insights::stats::window(
            i.day,
            crate::ui::calendar::Unit::Day,
            crate::ui::clock::now(cx).offset(),
        )
        .0;
        i.recording_since = Some(start + 3_600_000);
        cx.notify();
        start + 3_600_000
    });
    assert!(
        shows(cx, w, "insights-pending"),
        "the first stretch shows before any card"
    );
    let _ = since;
}

#[gpui_kit::test]
fn the_journal_builds_its_day_once_per_load_not_per_render(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx);
    v.update(cx, |i, cx| i.set_tab(Tab::Journal, cx));
    cx.run_until_parked();
    let view = |cx: &mut TestAppContext| {
        cx.update_window(w.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
        v.read_with(cx, |i, _| i.journal_day.as_ref().map(|m| m.view.clone()))
            .expect("the Journal drew its day")
    };
    let first = view(cx);
    assert!(std::rc::Rc::ptr_eq(&first, &view(cx)), "a redraw reuses it");
    v.update(cx, |i, cx| {
        let j = i.journal.as_deref().cloned().unwrap();
        i.journal = Some(std::rc::Rc::new(j)); // a new load
        cx.notify();
    });
    assert!(
        !std::rc::Rc::ptr_eq(&first, &view(cx)),
        "a new load rebuilds it"
    );
}
