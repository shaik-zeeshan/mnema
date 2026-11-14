//! Layer 2 for Quick Recall: every §5 constant on both sides of its boundary
//! through the real view (typing goes through the keystroke interceptor and the
//! InputState), plus the keymap's main paths. Data: the parity fixtures.

use std::sync::Arc;
use std::time::Duration;

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    point, px, size, AppContext as _, Bounds, Entity, TestAppContext, WindowAppearance,
    WindowBounds, WindowHandle, WindowOptions,
};
use time::macros::datetime;

use super::fixture::{Call, FixtureBackend};
use super::tokens::{Forced, Pill};
use super::*;
use crate::theme;

const MS: Duration = Duration::from_millis(1);

fn open(
    cx: &mut TestAppContext,
    state: &str,
) -> (WindowHandle<Root>, Entity<QuickRecall>, Arc<FixtureBackend>) {
    let backend = Arc::new(FixtureBackend::load(state).expect("fixture"));
    let b = backend.clone();
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        crate::ui::clock::freeze(Some(datetime!(2026-11-04 14:32 +05:30)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(WIDTH), px(CAP_H)),
            })),
            ..Default::default()
        };
        let (window, view) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| QuickRecall::new(b, window, cx))
        })
        .expect("window");
        (window.downcast::<Root>().expect("root"), view)
    });
    cx.run_until_parked();
    frame(cx, window);
    let input = view.read_with(cx, |qr, _| qr.input.clone());
    cx.update_window(window.into(), |_, window, cx| {
        input.update(cx, |i, cx| i.focus(window, cx))
    })
    .unwrap();
    frame(cx, window);
    (window, view, backend)
}

fn frame(cx: &mut TestAppContext, window: WindowHandle<Root>) {
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

fn advance(cx: &mut TestAppContext, d: Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

/// Type `text` key by key, as the user would.
fn typed(cx: &mut TestAppContext, window: WindowHandle<Root>, text: &str) {
    let keys: Vec<String> = text
        .chars()
        .map(|c| {
            if c == ' ' {
                "space".into()
            } else {
                c.to_string()
            }
        })
        .collect();
    cx.simulate_keystrokes(window.into(), &keys.join(" "));
    cx.run_until_parked();
}

fn keys(cx: &mut TestAppContext, window: WindowHandle<Root>, k: &str) {
    cx.simulate_keystrokes(window.into(), k);
    cx.run_until_parked();
}

fn searches(b: &FixtureBackend) -> Vec<String> {
    b.calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::Search(q) => Some(q),
            _ => None,
        })
        .collect()
}

fn asks(b: &FixtureBackend) -> usize {
    b.calls()
        .iter()
        .filter(|c| matches!(c, Call::AskStart(_)))
        .count()
}

#[gpui_kit::test]
fn search_runs_250ms_after_the_last_keystroke(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "results");
    typed(cx, w, "pricing");
    assert_eq!(v.read_with(cx, |qr, _| qr.text.clone()), "pricing");
    advance(cx, SEARCH_DEBOUNCE - MS);
    assert!(searches(&b).is_empty(), "249 ms: no search yet");
    advance(cx, MS);
    assert_eq!(searches(&b), ["pricing"], "250 ms: the search runs");
    assert!(v.read_with(cx, |qr, _| qr.results.has_results()));
    // A keystroke restarts the wait.
    typed(cx, w, "s");
    advance(cx, SEARCH_DEBOUNCE - MS);
    assert_eq!(searches(&b).len(), 1);
    advance(cx, MS);
    assert_eq!(searches(&b).last().map(String::as_str), Some("pricings"));
}

#[gpui_kit::test]
fn phrases_lift_into_pills_700ms_after_typing_pauses(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "results");
    typed(cx, w, "notes in Arc");
    advance(cx, COMMIT - MS);
    v.read_with(cx, |qr, _| {
        assert!(qr.pills.is_empty(), "699 ms: still tinted text");
        assert_eq!(qr.parse.spans.len(), 1);
        assert_eq!(qr.operator_query(), "notes app:Arc");
    });
    advance(cx, MS);
    v.read_with(cx, |qr, _| {
        assert_eq!(qr.pills, vec![Pill::App("Arc".into())], "700 ms: a pill");
        assert_eq!(qr.text, "notes ");
        assert_eq!(qr.operator_query(), "notes app:Arc");
    });
}

#[gpui_kit::test]
fn a_question_answers_750ms_after_typing_pauses(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "ask-done");
    typed(cx, w, "what did ana say");
    assert_eq!(v.read_with(cx, |qr, _| qr.mode()), Mode::Ask);
    advance(cx, ASK_PAUSE - MS);
    assert_eq!(asks(&b), 0, "749 ms: not asked yet");
    advance(cx, MS);
    assert_eq!(asks(&b), 1, "750 ms: asked");
    assert_eq!(
        v.read_with(cx, |qr, _| qr.ask.phase().map(String::from)),
        Some("thinking".into())
    );
    // An edit cancels the turn in flight and waits again.
    typed(cx, w, "?");
    advance(cx, ASK_PAUSE);
    assert_eq!(asks(&b), 1, "the final ? asks the same question: no re-ask");
    typed(cx, w, " no");
    advance(cx, ASK_PAUSE - MS);
    assert!(
        b.calls()
            .iter()
            .any(|c| matches!(c, Call::AskCancel(_, true))),
        "the edit discards the old turn"
    );
    assert_eq!(asks(&b), 1);
    advance(cx, MS);
    assert_eq!(asks(&b), 2);
}

#[gpui_kit::test]
fn a_forced_ask_waits_for_return(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "results");
    typed(cx, w, "pricing page");
    keys(cx, w, "tab");
    assert_eq!(v.read_with(cx, |qr, _| qr.forced), Some(Forced::Ask));
    advance(cx, ASK_PAUSE * 2);
    assert_eq!(asks(&b), 0, "Tab-forced: nothing sends on its own");
    keys(cx, w, "enter");
    assert_eq!(asks(&b), 1);
}

#[gpui_kit::test]
fn blur_within_300ms_of_summon_keeps_the_panel(cx: &mut TestAppContext) {
    let (_, v, _) = open(cx, "idle");
    let summon = |cx: &mut TestAppContext| {
        v.update(cx, |qr, cx| {
            qr.summoned(cx.background_executor().now());
            qr.on_activation(true, cx);
        })
    };
    let blur = |cx: &mut TestAppContext| v.update(cx, |qr, cx| qr.on_activation(false, cx));
    summon(cx);
    advance(cx, BLUR_GRACE - MS);
    blur(cx);
    assert_eq!(
        v.read_with(cx, |qr, _| (qr.dismissals, qr.visible)),
        (0, true),
        "299 ms: setup blur"
    );
    summon(cx);
    advance(cx, BLUR_GRACE);
    blur(cx);
    assert_eq!(
        v.read_with(cx, |qr, _| (qr.dismissals, qr.visible)),
        (1, false),
        "300 ms: dismissed"
    );
    // A one-shot suppress keeps it through the next blur only.
    summon(cx);
    advance(cx, BLUR_GRACE * 2);
    v.update(cx, |qr, _| qr.suppress_next_blur());
    blur(cx);
    assert_eq!(v.read_with(cx, |qr, _| qr.dismissals), 1);
    summon(cx);
    advance(cx, BLUR_GRACE * 2);
    blur(cx);
    assert_eq!(v.read_with(cx, |qr, _| qr.dismissals), 2);
}

#[gpui_kit::test]
fn state_resets_5s_after_hide(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "results");
    typed(cx, w, "pricing");
    advance(cx, SEARCH_DEBOUNCE);
    v.update(cx, |qr, cx| qr.on_activation(false, cx));
    advance(cx, RESET_AFTER_HIDE - MS);
    v.read_with(cx, |qr, _| {
        assert_eq!(qr.text, "pricing", "4999 ms: kept for a quick resume");
        assert!(qr.results.has_results());
    });
    advance(cx, MS);
    v.read_with(cx, |qr, _| {
        assert_eq!(qr.text, "", "5000 ms: back to the empty bar");
        assert!(!qr.results.has_results());
        assert!(!qr.show_body());
    });
    // Coming back before 5 s keeps everything.
    typed(cx, w, "pricing");
    advance(cx, SEARCH_DEBOUNCE);
    v.update(cx, |qr, cx| qr.on_activation(false, cx));
    advance(cx, RESET_AFTER_HIDE - MS);
    v.update(cx, |qr, cx| qr.on_activation(true, cx));
    advance(cx, RESET_AFTER_HIDE);
    assert_eq!(v.read_with(cx, |qr, _| qr.text.clone()), "pricing");
}

#[gpui_kit::test]
fn down_on_the_empty_bar_opens_recent_then_returns_uses_a_row(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "recent");
    keys(cx, w, "down");
    v.read_with(cx, |qr, _| {
        assert!(qr.show_recent());
        assert_eq!(qr.recent.list.len(), 3);
        assert_eq!(qr.recent_count(), 6, "3 recent + 3 try");
    });
    keys(cx, w, "down enter");
    assert_eq!(
        v.read_with(cx, |qr, _| qr.text.clone()),
        "what did Ana say about retention?"
    );
    assert!(!v.read_with(cx, |qr, _| qr.show_recent()));
}

#[gpui_kit::test]
fn esc_clears_then_closes_and_ctrl_f_opens_the_pill(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "results");
    v.update(cx, |qr, _| qr.visible = true);
    typed(cx, w, "notes in Arc");
    advance(cx, COMMIT);
    keys(cx, w, "ctrl-f");
    assert!(
        v.read_with(cx, |qr, _| qr.filters.popover),
        "Ctrl+F: the pill's popover"
    );
    keys(cx, w, "escape");
    assert!(!v.read_with(cx, |qr, _| qr.filters.popover));
    assert_eq!(
        v.read_with(cx, |qr, _| qr.filters.pill),
        Some(0),
        "the pill stays pressed"
    );
    keys(cx, w, "escape");
    assert_eq!(
        v.read_with(cx, |qr, _| qr.filters.pill),
        None,
        "then lets go"
    );
    keys(cx, w, "escape");
    v.read_with(cx, |qr, _| {
        assert!(qr.text.is_empty() && qr.pills.is_empty())
    });
    keys(cx, w, "escape");
    assert_eq!(
        v.read_with(cx, |qr, _| qr.dismissals),
        1,
        "Esc on the empty bar closes it"
    );
}

#[gpui_kit::test]
fn arrows_move_and_space_looks(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "results");
    typed(cx, w, "pricing page");
    advance(cx, SEARCH_DEBOUNCE);
    assert_eq!(
        v.read_with(cx, |qr, _| qr.results.selected),
        Some(0),
        "the top hit is lit"
    );
    keys(cx, w, "down");
    assert_eq!(v.read_with(cx, |qr, _| qr.results.selected), Some(1));
    keys(cx, w, "space");
    assert!(
        v.read_with(cx, |qr, _| qr.look.is_some()),
        "Space looks once arrowed in"
    );
    keys(cx, w, "escape");
    assert!(v.read_with(cx, |qr, _| qr.look.is_none()));
}

#[gpui_kit::test]
fn the_panel_warms_mcp_once_and_only_with_ai_and_a_connector_on(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
    });
    for (ai, connector, want) in [(true, true, 1), (false, true, 0), (true, false, 0)] {
        let mut backend = FixtureBackend::load("idle").expect("fixture");
        crate::chat::fixture::with_mcp(&mut backend.map["get_recording_settings"], ai, connector);
        let backend = Arc::new(backend);
        let b = backend.clone();
        let view = cx.update(|cx| {
            let (_, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| QuickRecall::new(b, window, cx))
            })
            .expect("window");
            view
        });
        cx.run_until_parked();
        // A later summon re-reads the bar but doesn't warm again (Svelte: on mount).
        view.update(cx, |qr, cx| qr.on_activation(true, cx));
        cx.run_until_parked();
        let warms = backend
            .calls()
            .iter()
            .filter(|c| **c == Call::WarmMcp)
            .count();
        assert_eq!(warms, want, "ai {ai}, connector {connector}");
    }
}

#[gpui_kit::test]
fn quick_looks_copy_button_copies_the_frame_text_and_says_copied(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "results");
    typed(cx, w, "pricing page");
    advance(cx, SEARCH_DEBOUNCE);
    keys(cx, w, "down");
    keys(cx, w, "space");
    frame(cx, w);
    let is_screen = v.read_with(cx, |qr, _| qr.look.as_ref().map(|l| l.is_screen()));
    assert!(is_screen.is_some(), "Quick Look is open");
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("qr-look-copy", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let clip = cx.read(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    let expected = if is_screen == Some(true) {
        b.ocr_text(0)
    } else {
        v.read_with(cx, |qr, _| {
            qr.look
                .as_ref()
                .and_then(|l| l.turn())
                .map(|t| t.text.clone())
        })
    };
    assert!(
        clip.as_deref().is_some_and(|t| !t.is_empty()),
        "something real: {clip:?}"
    );
    assert_eq!(clip, expected);
    assert!(
        v.read_with(cx, |qr, _| qr.look.as_ref().unwrap().copied),
        "the label reads Copied"
    );
    advance(cx, COPIED_LOOK);
    assert!(
        !v.read_with(cx, |qr, _| qr.look.as_ref().unwrap().copied),
        "reverted at 1.4 s"
    );
}

#[gpui_kit::test]
fn an_answer_link_keeps_the_panel_up_while_the_browser_opens(cx: &mut TestAppContext) {
    let (_, v, _) = open(cx, "idle");
    v.update(cx, |qr, cx| {
        qr.summoned(cx.background_executor().now());
        qr.on_activation(true, cx);
    });
    advance(cx, BLUR_GRACE * 2);
    // `openAnswerLink`: the prose's link handler suppresses the next blur.
    let open_link = v.read_with(cx, |qr, _| qr.link_opener());
    cx.update(|cx| {
        let window = cx.windows()[0];
        window
            .update(cx, |_, window, cx| {
                let click = gpui_kit::ClickEvent::Keyboard(Default::default());
                open_link(&"https://example.com/notes".into(), &click, window, cx);
            })
            .unwrap();
    });
    v.update(cx, |qr, cx| qr.on_activation(false, cx));
    assert_eq!(
        v.read_with(cx, |qr, _| (qr.dismissals, qr.visible)),
        (0, true),
        "the browser's focus isn't a click away"
    );
    // A link that can't open suppresses nothing.
    cx.update(|cx| {
        let window = cx.windows()[0];
        window
            .update(cx, |_, window, cx| {
                let click = gpui_kit::ClickEvent::Keyboard(Default::default());
                open_link(&"file:///etc/passwd".into(), &click, window, cx);
            })
            .unwrap();
    });
    v.update(cx, |qr, cx| {
        qr.on_activation(true, cx);
        qr.on_activation(false, cx);
    });
    assert_eq!(v.read_with(cx, |qr, _| qr.dismissals), 1);
}

#[gpui_kit::test]
fn a_long_answer_points_to_chat(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "ask-done");
    typed(cx, w, "what did ana say");
    advance(cx, ASK_PAUSE);
    let set = |cx: &mut TestAppContext, text: &str| {
        v.update(cx, |qr, cx| {
            let mut t = crate::chat::ChatTurn::new(0, "what did ana say", "streaming");
            t.apply(
                &capture_types::TurnUpdate::AppendProse { text: text.into() },
                0,
            );
            t.view.phase = "done".into();
            qr.ask.turn = Some(t);
            qr.ask.thread_id = Some("c1".into());
            cx.notify();
        });
        // One frame measures the answer, the next draws what that implies.
        frame(cx, w);
        frame(cx, w);
        cx.update_window(w.into(), |_, window, _| {
            window.try_find("qr-ans-more").is_some()
        })
        .unwrap()
    };
    assert!(
        !set(cx, "Ana said the launch moved to Friday."),
        "a short answer fits"
    );
    let long = (0..14)
        .map(|i| format!("Line {i} of a model that ignored the two-sentence brief."))
        .collect::<Vec<_>>()
        .join("\n\n");
    assert!(
        set(cx, &long),
        "past the clip: Read the full answer in Chat"
    );
    v.update(cx, |qr, _| qr.visible = true);
    let before = v.read_with(cx, |qr, _| qr.dismissals);
    cx.update_window(w.into(), |_, window, cx| window.click("qr-ans-more", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        v.read_with(cx, |qr, _| qr.dismissals),
        before + 1,
        "Chat opens the thread and the bar closes"
    );
}

/// A newer search drops the one still queued: only its fetch runs.
#[gpui_kit::test]
fn a_newer_search_cancels_the_one_still_in_flight(cx: &mut TestAppContext) {
    let (_w, v, b) = open(cx, "results");
    let before = searches(&b).len();
    v.update(cx, |qr, cx| {
        qr.run_search("pricing".into(), cx);
        qr.run_search("pricing page".into(), cx);
    });
    cx.run_until_parked();
    assert_eq!(searches(&b)[before..], ["pricing page".to_string()]);
}

#[test]
fn a_snippet_with_line_breaks_stays_on_one_line() {
    assert_eq!(
        model::snippet_runs("…the parent's <mark>name</mark> opens.\n\nMove  to…"),
        vec![
            ("…the parent's ".to_string(), false),
            ("name".to_string(), true),
            (" opens. Move to…".to_string(), false),
        ]
    );
}
