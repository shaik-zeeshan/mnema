//! Layer 2 for Chat: the versioned stream, the timing constants on both sides
//! of their boundary, Enter / suggestions, and the pure helpers.

use std::time::Duration;

use capture_types::{AnswerBlock, AppearanceSetting, TurnUpdate};
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    px, size, Bounds, Entity, Point, TestAppContext, WindowAppearance, WindowBounds, WindowHandle,
    WindowOptions,
};

use super::format::{close_dangling_bold, model_display_name};
use super::model::{receive, Availability};
use super::*;
use crate::{keymap, theme};

fn open(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<Chat>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        keymap::bind(&keymap::Bindings::default(), cx);
        clock::freeze(Some(app_core::fixtures::fixture_now()), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1280.), px(800.)),
            })),
            ..Default::default()
        };
        let (window, chat) =
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Chat::new(window, cx)))
                .expect("window");
        (window.downcast::<Root>().expect("root"), chat)
    })
}

fn ev(version: u64, turn_index: i64, update: TurnUpdate) -> AskAiUpdateEvent {
    AskAiUpdateEvent {
        conversation_id: "c1".into(),
        version,
        turn_index,
        update,
    }
}

fn prose(text: &str) -> TurnUpdate {
    TurnUpdate::AppendProse { text: text.into() }
}

/// A chat `c1` with one live turn at version 1.
fn live(cx: &mut TestAppContext, chat: &Entity<Chat>) {
    chat.update(cx, |c, cx| {
        c.active = Some("c1".into());
        c.pane = Pane::Thread;
        let mut t = ChatTurn::new(0, "What did I ship?", "streaming");
        t.version = 1;
        c.turns.push(t);
        c.streaming = true;
        cx.notify();
    });
}

fn answer(chat: &Entity<Chat>, cx: &mut TestAppContext) -> String {
    cx.read(|cx| chat.read(cx).turns[0].plain_text())
}

#[test]
fn only_the_exactly_next_version_applies() {
    let mut t = ChatTurn::new(0, "q", "thinking");
    assert_eq!(receive(&mut t, 1, &prose("Hel"), 0), Receive::Applied);
    assert_eq!(receive(&mut t, 2, &prose("lo"), 0), Receive::Applied);
    // A stale or duplicate version is ignored, never re-applied.
    assert_eq!(receive(&mut t, 2, &prose("lo"), 0), Receive::Stale);
    assert_eq!(receive(&mut t, 1, &prose("X"), 0), Receive::Stale);
    assert_eq!(t.plain_text(), "Hello");
    assert_eq!(t.version, 2);
    // A skipped version is a gap: nothing applies, the caller re-snapshots.
    assert_eq!(receive(&mut t, 4, &prose("!"), 0), Receive::Gap);
    assert_eq!(t.plain_text(), "Hello");
    assert_eq!(t.version, 2);
}

#[test]
fn append_prose_coalesces_like_the_rust_reducer() {
    let mut t = ChatTurn::new(0, "q", "streaming");
    t.apply(&prose("a"), 0);
    t.apply(&prose("b"), 0);
    t.apply(
        &TurnUpdate::OpenBlock {
            block: AnswerBlock::Bars {
                title: None,
                items: vec![],
            },
        },
        0,
    );
    t.apply(&prose("c"), 0);
    assert_eq!(t.view.blocks.len(), 3);
    assert_eq!(
        t.view.blocks[0],
        AnswerBlock::Prose {
            markdown: "ab".into()
        }
    );
    t.started_at_ms = Some(1_000);
    t.apply(&TurnUpdate::Done, 4_400);
    assert_eq!(t.view.phase, "done");
    assert_eq!(t.elapsed_ms, Some(3_400));
}

#[gpui_kit::test]
fn the_pane_ignores_stale_versions_and_other_chats(cx: &mut TestAppContext) {
    let (_w, chat) = open(cx);
    live(cx, &chat);
    let r = chat.update(cx, |c, cx| c.on_update(ev(2, 0, prose("One")), cx));
    assert_eq!(r, Some(Receive::Applied));
    let r = chat.update(cx, |c, cx| c.on_update(ev(2, 0, prose("One")), cx));
    assert_eq!(r, Some(Receive::Stale));
    let r = chat.update(cx, |c, cx| c.on_update(ev(1, 0, prose("Zero")), cx));
    assert_eq!(r, Some(Receive::Stale));
    assert_eq!(answer(&chat, cx), "One");
    // Another conversation's stream never touches this one.
    let mut other = ev(3, 0, prose("X"));
    other.conversation_id = "c2".into();
    assert_eq!(chat.update(cx, |c, cx| c.on_update(other, cx)), None);
    // Done settles streaming.
    chat.update(cx, |c, cx| c.on_update(ev(3, 0, TurnUpdate::Done), cx));
    assert!(!cx.read(|cx| chat.read(cx).streaming));
}

#[gpui_kit::test]
fn copied_reverts_at_1600ms(cx: &mut TestAppContext) {
    let (_w, chat) = open(cx);
    live(cx, &chat);
    chat.update(cx, |c, cx| {
        c.on_update(ev(2, 0, prose("Shipped the parser.")), cx);
        c.on_update(ev(3, 0, TurnUpdate::Done), cx);
        c.copy_answer(0, cx);
    });
    assert_eq!(COPIED_REVERT, Duration::from_millis(1600));
    cx.executor()
        .advance_clock(COPIED_REVERT - Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| chat.read(cx).copied),
        Some(0),
        "still Copied at 1199 ms"
    );
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| chat.read(cx).copied),
        None,
        "reverted at 1600 ms"
    );
    assert_eq!(
        cx.read(|cx| cx.read_from_clipboard().and_then(|c| c.text())),
        Some("Shipped the parser.".into())
    );
}

#[gpui_kit::test]
fn deleted_toast_lasts_4s(cx: &mut TestAppContext) {
    let (_w, chat) = open(cx);
    chat.update(cx, |c, cx| c.show_deleted(cx));
    let toasts = cx.read(|cx| chat.read(cx).toasts.clone());
    assert_eq!(DELETED_TOAST_TTL, Duration::from_secs(4));
    cx.executor()
        .advance_clock(DELETED_TOAST_TTL - Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| toasts.read(cx).len()), 1, "showing at 3999 ms");
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| toasts.read(cx).len()), 0, "gone at 4000 ms");
}

#[gpui_kit::test]
fn pin_failure_toast_with_retry_lasts_5s(cx: &mut TestAppContext) {
    let (_w, chat) = open(cx);
    chat.update(cx, |c, cx| c.pin_failed("c1".into(), true, cx));
    let toasts = cx.read(|cx| chat.read(cx).toasts.clone());
    assert_eq!(PIN_FAIL_TOAST_TTL, Duration::from_secs(5));
    cx.executor()
        .advance_clock(PIN_FAIL_TOAST_TTL - Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| toasts.read(cx).len()), 1, "showing at 4999 ms");
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| toasts.read(cx).len()), 0, "gone at 5000 ms");
}

#[gpui_kit::test]
fn enter_sends_and_send_becomes_stop(cx: &mut TestAppContext) {
    let (w, chat) = open(cx);
    chat.update(cx, |c, cx| {
        c.data.availability = Some(Availability {
            available: true,
            reason: None,
        });
        cx.notify();
    });
    cx.update_window(w.into(), |_, window, cx| {
        chat.update(cx, |c, cx| {
            c.composer
                .update(cx, |s, cx| s.set_value("What did I ship?", window, cx));
            c.composer.update(cx, |s, cx| s.focus(window, cx));
        });
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let (turns, streaming, left) = cx.read(|cx| {
        let c = chat.read(cx);
        (
            c.turns.len(),
            c.streaming,
            c.composer.read(cx).value().to_string(),
        )
    });
    assert_eq!((turns, streaming, left.as_str()), (1, true, ""));
    let stop = cx
        .update_window(w.into(), |_, window, cx| {
            window.render_frame(cx);
            window.find("chat-send").label().map(str::to_string)
        })
        .unwrap();
    assert_eq!(stop.as_deref(), Some("Stop"));
}

#[gpui_kit::test]
fn suggestions_fill_the_composer_and_never_send(cx: &mut TestAppContext) {
    let (w, chat) = open(cx);
    chat.update(cx, |c, cx| {
        c.data.availability = Some(Availability {
            available: true,
            reason: None,
        });
        cx.notify();
    });
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(crate::ui::nid("chat-suggest", 1), cx);
    })
    .unwrap();
    cx.run_until_parked();
    let (turns, text, scope) = cx.read(|cx| {
        let c = chat.read(cx);
        (
            c.turns.len(),
            c.composer.read(cx).value().to_string(),
            c.scope,
        )
    });
    assert_eq!(turns, 0);
    assert_eq!(text, "Summarize my calls this week");
    let today = app_core::fixtures::fixture_now().date();
    assert_eq!(scope.label(today), "this week");
}

#[test]
fn dangling_bold_closes_only_while_open() {
    assert_eq!(close_dangling_bold("a **b"), "a **b**");
    assert_eq!(close_dangling_bold("a **b**"), "a **b**");
    assert_eq!(close_dangling_bold("a **b*"), "a **b**");
    assert_eq!(close_dangling_bold("a **"), "a ");
    // Inside an open fence, and in inline code, it's left alone.
    assert_eq!(close_dangling_bold("```\n**x"), "```\n**x");
    assert_eq!(close_dangling_bold("`**` ok"), "`**` ok");
    // Only the last paragraph counts.
    assert_eq!(close_dangling_bold("**a\n\nb"), "**a\n\nb");
}

#[test]
fn caret_is_appended_while_streaming_but_not_inside_code() {
    assert_eq!(
        prose::display_source("Hi **there", true),
        "Hi **there**[](mnema:caret)"
    );
    assert_eq!(prose::display_source("Hi", false), "Hi");
    assert_eq!(prose::display_source("```\nfn", true), "```\nfn");
}

#[test]
fn model_names_read_like_the_svelte_ones() {
    assert_eq!(
        model_display_name("claude-sonnet-4-5-20250929"),
        "Claude Sonnet 4.5"
    );
    assert_eq!(
        model_display_name("claude-3-5-sonnet-latest"),
        "Claude Sonnet 3.5"
    );
    assert_eq!(model_display_name("gpt-5-mini"), "GPT-5 mini");
    assert_eq!(model_display_name("gpt-4o"), "GPT-4o");
    assert_eq!(
        model_display_name("accounts/x/models/qwen2.5:14b"),
        "qwen2.5:14b"
    );
}

#[test]
fn reason_codes_are_detected() {
    assert!(turn::is_reason_code("no_providers"));
    assert!(turn::is_reason_code("needs_reconnect:chatgpt"));
    assert!(!turn::is_reason_code("The engine broke."));
}

#[cfg(feature = "fixtures")]
#[test]
fn every_parity_fixture_reads_through_the_real_types() {
    for state in fixture::STATES {
        let f = fixture::chat_fixture(state).unwrap_or_else(|e| panic!("{state}: {e:#}"));
        let opens = matches!(state, "mid-stream" | "done" | "error");
        assert_eq!(f.open.is_some(), opens, "{state}");
        assert_eq!(f.snapshot.is_some(), state == "mid-stream", "{state}");
    }
}

#[test]
fn picker_rows_quick_view_and_typed_id() {
    use crate::ui::model_picker::{Mode, Pick, PoolModel, Row, Spec};
    let provider = capture_types::AiProviderConfig {
        id: "anthropic".into(),
        kind: capture_types::AiProviderKind::Anthropic,
        label: String::new(),
        base_url: String::new(),
    };
    let pool = ["claude-sonnet-4-5", "claude-haiku-4-5"].map(|id| PoolModel {
        id: id.into(),
        provider: "anthropic".into(),
        context_window: None,
    });
    let fav = ("anthropic".to_string(), "claude-haiku-4-5".to_string());
    let spec = Spec {
        providers: vec![provider],
        pool: pool.to_vec(),
        favorites: vec![fav.clone()],
        sentinel: Some(("Default".into(), None)),
        sentinel_selected: true,
        can_star: true,
        ..Default::default()
    };
    let groups = Default::default();
    // Quick: Default, the starred model numbered 1, then "Browse all 2 models".
    let quick = spec.rows("", Mode::Quick, &groups);
    let opts: Vec<_> = quick
        .iter()
        .filter_map(|r| match r {
            Row::Option(o) => Some((o.pick.clone(), o.num)),
            _ => None,
        })
        .collect();
    assert_eq!(
        opts,
        vec![(Pick::Sentinel, None), (Pick::Pool(fav), Some(1))]
    );
    assert!(
        matches!(quick.last(), Some(Row::Browse { label, .. }) if label == "Browse all 2 models")
    );
    // A typed id nobody lists is offered per connected provider, never guessed.
    let search = spec.rows("gpt-6", Mode::Quick, &groups);
    assert!(search.iter().any(|r| matches!(r, Row::Note { .. })));
    let exact = Pick::Exact(("anthropic".into(), "gpt-6".into()));
    assert!(search
        .iter()
        .any(|r| matches!(r, Row::Option(o) if o.pick == exact)));
}
