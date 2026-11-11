//! Chat flows found by the flow-parity pass (11 Nov), driven on the real view:
//! the list search, rename, failures that must say why, the composer's off
//! notice, the deferred `?q=` send, ⌘⇧M and the token bar's window.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use app_core::{
    AppCore, ConfirmDialog, CoreEvent, ExitKind, Host, MessageDialog, Paths, WindowRequest,
    WindowSignal,
};
use capture_types::{AiProviderConfig, AiProviderKind, ConversationSummary};
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, Entity, Focusable as _, TestAppContext, Window, WindowHandle};

use super::model::Availability;
use super::tests::open;
use super::*;
use crate::ui::Tone;

/// Records every alert Chat asks the host for.
#[derive(Default)]
struct Alerts(Mutex<Vec<(String, String)>>);

impl Host for Alerts {
    fn emit(&self, _: CoreEvent) {}
    fn message(&self, d: MessageDialog) {
        self.0.lock().unwrap().push((d.title, d.message));
    }
    fn confirm(&self, _: ConfirmDialog) -> tokio::sync::oneshot::Receiver<bool> {
        tokio::sync::oneshot::channel().1
    }
    fn open_url(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn refresh_tray(&self) {}
    fn open_window(&self, _: WindowRequest) -> Result<(), String> {
        Ok(())
    }
    fn window_signal(&self, _: WindowSignal) {}
    fn exit(&self, _: ExitKind) {}
}

/// A bare core (no capture index, no settings): every store write fails, so
/// the failure paths run for real. Nothing on disk outside a temp dir.
fn bare_core(cx: &mut TestAppContext) -> (Arc<Alerts>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("MNEMA_TRIPWIRE_NO_KEYCHAIN", "1");
    std::env::set_var(
        "MNEMA_LICENSE_TOKEN_DIR",
        std::env::temp_dir().join(format!("mnema-gpui-licence-{}", std::process::id())),
    );
    let host = Arc::new(Alerts::default());
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        cache: dir.path().join("cache"),
        log: dir.path().join("log"),
    };
    let core = Arc::new(AppCore::new(host.clone(), paths).unwrap());
    cx.executor().allow_parking();
    cx.update(|cx| {
        crate::bridge::tokio::init_from_handle(cx, core.runtime_handle());
        cx.set_global(crate::bridge::events::Core(core));
    });
    (host, dir)
}

/// Pump until `done` (backend answers come from tokio threads), up to 10 s.
fn wait(cx: &mut TestAppContext, what: &str, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !done(cx) {
        assert!(std::time::Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(5));
        cx.run_until_parked();
    }
}

fn act(cx: &mut TestAppContext, w: WindowHandle<Root>, f: impl FnOnce(&mut Window, &mut App)) {
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        f(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn row(id: &str, title: &str) -> ConversationSummary {
    serde_json::from_value(serde_json::json!({
        "conversationId": id, "title": title, "preview": "", "origin": "chat",
        "createdAtMs": 0, "updatedAtMs": 0, "turnCount": 1, "pinned": false
    }))
    .unwrap()
}

fn available(chat: &Entity<Chat>, cx: &mut TestAppContext, on: bool, reason: Option<&str>) {
    chat.update(cx, |c, cx| {
        c.data.availability = Some(Availability {
            available: on,
            reason: reason.map(str::to_string),
        });
        cx.notify();
    });
}

fn generation(chat: &Entity<Chat>, cx: &mut TestAppContext) -> u64 {
    chat.read_with(cx, |c, _| c.load_generation)
}

#[test]
fn the_off_notice_says_why_in_plain_words() {
    let (text, action, tone) = composer::off_copy("ask_ai_disabled", None);
    assert_eq!(
        (text.as_str(), action, tone),
        (
            "Ask AI is off. Past chats stay readable here.",
            "Turn on in Settings →",
            Tone::Off
        )
    );
    let (text, action, _) = composer::off_copy("local_endpoint_unreachable", None);
    assert!(text.starts_with("Can't reach "), "{text}");
    assert_eq!(action, "Check again");
    // A fix names it (never the old generic "needs attention" line).
    let (text, action, tone) = composer::off_copy("no_provider_key:anthropic", None);
    assert_eq!(text, "No API key saved for Anthropic.");
    assert_eq!((action, tone), ("Open settings", Tone::Warn));
}

#[gpui_kit::test]
fn the_off_action_checks_again_only_when_that_can_help(cx: &mut TestAppContext) {
    let (w, chat) = open(cx);
    available(&chat, cx, false, Some("vault_denied"));
    let before = generation(&chat, cx);
    act(cx, w, |w, cx| w.click("chat-off-action", cx));
    assert_eq!(
        generation(&chat, cx),
        before + 1,
        "Check again re-reads availability"
    );
    // "Turn on in Settings →" goes to Settings; it never just re-reads.
    available(&chat, cx, false, Some("ask_ai_disabled"));
    act(cx, w, |w, cx| w.click("chat-off-action", cx));
    assert_eq!(generation(&chat, cx), before + 1);
}

#[gpui_kit::test]
fn the_list_search_waits_220ms_after_the_last_key(cx: &mut TestAppContext) {
    let (w, chat) = open(cx);
    let search = chat.read_with(cx, |c, _| c.search.clone());
    act(cx, w, |w, cx| search.update(cx, |s, cx| s.focus(w, cx)));
    let before = generation(&chat, cx);
    act(cx, w, |w, cx| w.input("qu", cx));
    assert_eq!(SEARCH_DEBOUNCE, Duration::from_millis(220));
    cx.executor()
        .advance_clock(SEARCH_DEBOUNCE - Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(generation(&chat, cx), before, "nothing asked at 219 ms");
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(generation(&chat, cx), before + 1, "one search at 220 ms");
}

#[gpui_kit::test]
fn clear_search_lists_every_chat_again_and_keeps_the_caret(cx: &mut TestAppContext) {
    let (w, chat) = open(cx);
    let search = chat.read_with(cx, |c, _| c.search.clone());
    act(cx, w, |w, cx| {
        search.update(cx, |s, cx| {
            s.set_value("zebra", w, cx);
        })
    });
    chat.update(cx, |c, cx| {
        c.data.history = History::Rows(Vec::new());
        c.data.query = "zebra".into();
        cx.notify();
    });
    let before = generation(&chat, cx);
    act(cx, w, |w, cx| w.click("chat-clear-search", cx));
    // set_value emits no Change: before the fix the list stayed on the search's
    // empty answer and read "No chats yet".
    assert_eq!(generation(&chat, cx), before + 1, "re-listed");
    let (text, focused) = cx
        .update_window(w.into(), |_, window, cx| {
            let s = search.read(cx);
            (s.value().to_string(), s.focus_handle(cx).is_focused(window))
        })
        .unwrap();
    assert_eq!((text.as_str(), focused), ("", true));
}

#[gpui_kit::test]
fn escape_cancels_a_rename(cx: &mut TestAppContext) {
    let (w, chat) = open(cx);
    chat.update(cx, |c, cx| {
        c.data.history = History::Rows(vec![row("c1", "Quarterly numbers")]);
        cx.notify();
    });
    act(cx, w, |w, cx| {
        chat.update(cx, |c, cx| c.start_rename(Rename::Row("c1".into()), w, cx))
    });
    act(cx, w, |w, cx| w.input("Budget", cx));
    act(cx, w, |w, cx| w.press("escape", cx));
    let (renaming, title) = chat.read_with(cx, |c, _| {
        (c.renaming.clone(), c.row("c1").map(|r| r.title.clone()))
    });
    assert_eq!(renaming, None, "Esc ends the rename");
    assert_eq!(
        title.as_deref(),
        Some("Quarterly numbers"),
        "and saves nothing"
    );
}

#[gpui_kit::test]
fn a_failed_header_rename_says_why_and_keeps_the_old_title(cx: &mut TestAppContext) {
    let (w, chat) = open(cx);
    let (alerts, _dir) = bare_core(cx);
    chat.update(cx, |c, cx| {
        c.active = Some("c1".into());
        c.title = "Quarterly numbers".into();
        c.pane = Pane::Thread;
        c.turns.push(ChatTurn::new(0, "What changed?", "done"));
        cx.notify();
    });
    act(cx, w, |w, cx| {
        chat.update(cx, |c, cx| c.start_rename(Rename::Header, w, cx))
    });
    act(cx, w, |w, cx| w.input("Budget", cx));
    act(cx, w, |w, cx| w.press("enter", cx));
    wait(cx, "the alert", |_| !alerts.0.lock().unwrap().is_empty());
    let (title, detail) = alerts.0.lock().unwrap()[0].clone();
    assert_eq!(title, "Couldn’t rename this chat");
    assert!(!detail.is_empty());
    wait(cx, "the old title", |cx| {
        chat.read_with(cx, |c, _| c.title == "Quarterly numbers")
    });
}

#[gpui_kit::test]
fn a_failed_regenerate_says_why_and_keeps_the_answer(cx: &mut TestAppContext) {
    let (w, chat) = open(cx);
    let (alerts, _dir) = bare_core(cx);
    available(&chat, cx, true, None);
    chat.update(cx, |c, cx| {
        c.active = Some("c1".into());
        c.pane = Pane::Thread;
        let mut t = ChatTurn::new(0, "What did I ship?", "done");
        t.apply(
            &capture_types::TurnUpdate::AppendProse {
                text: "The parser.".into(),
            },
            0,
        );
        t.view.phase = "done".into();
        c.turns.push(t);
        cx.notify();
    });
    act(cx, w, |w, cx| w.click("chat-regenerate", cx));
    wait(cx, "the alert", |_| !alerts.0.lock().unwrap().is_empty());
    assert_eq!(
        alerts.0.lock().unwrap()[0].0,
        "Couldn’t regenerate this answer"
    );
    let (n, streaming, text) = chat.read_with(cx, |c, _| {
        (c.turns.len(), c.streaming, c.turns[0].plain_text())
    });
    assert_eq!((n, streaming, text.as_str()), (1, false, "The parser."));
}

#[gpui_kit::test]
fn a_titlebar_question_sends_once_availability_is_known(cx: &mut TestAppContext) {
    let (w, chat) = open(cx);
    // `?q=` before Ask AI's availability has loaded: nothing yet.
    act(cx, w, |w, cx| {
        chat.update(cx, |c, cx| {
            c.new_chat(Some("What did I ship?".into()), true, w, cx)
        })
    });
    assert_eq!(chat.read_with(cx, |c, _| c.turns.len()), 0);
    available(&chat, cx, true, None);
    act(cx, w, |_, _| {});
    let (n, q) = chat.read_with(cx, |c, _| {
        (
            c.turns.len(),
            c.turns.first().map(|t| t.view.question.clone()),
        )
    });
    assert_eq!(
        (n, q.as_deref()),
        (1, Some("What did I ship?")),
        "sent once"
    );
    act(cx, w, |_, _| {});
    assert_eq!(chat.read_with(cx, |c, _| c.turns.len()), 1, "and only once");
}

#[gpui_kit::test]
fn a_titlebar_question_waits_in_the_composer_when_ask_ai_is_off(cx: &mut TestAppContext) {
    let (w, chat) = open(cx);
    act(cx, w, |w, cx| {
        chat.update(cx, |c, cx| {
            c.new_chat(Some("What did I ship?".into()), true, w, cx)
        })
    });
    available(&chat, cx, false, Some("ask_ai_disabled"));
    act(cx, w, |_, _| {});
    let (n, text) = chat.read_with(cx, |c, cx| {
        (c.turns.len(), c.composer.read(cx).value().to_string())
    });
    assert_eq!((n, text.as_str()), (0, "What did I ship?"));
}

#[gpui_kit::test]
fn next_favorite_cycles_the_starred_models(cx: &mut TestAppContext) {
    let (_w, chat) = open(cx);
    let a = ("anthropic".to_string(), "claude-sonnet-4-5".to_string());
    let b = ("openai".to_string(), "gpt-5".to_string());
    assert!(
        !chat.update(cx, |c, cx| c.next_favorite(cx)),
        "nothing starred"
    );
    chat.update(cx, |c, _| {
        c.data.engine.favorites = vec![a.clone(), b.clone()];
        c.data.engine.default = Some(("ollama".into(), "qwen".into()));
    });
    // Not on a starred model: the first one.
    chat.update(cx, |c, cx| c.next_favorite(cx));
    assert_eq!(chat.read_with(cx, |c, _| c.pin.clone()), Some(a.clone()));
    chat.update(cx, |c, cx| c.next_favorite(cx));
    assert_eq!(chat.read_with(cx, |c, _| c.pin.clone()), Some(b));
    chat.update(cx, |c, cx| c.next_favorite(cx));
    assert_eq!(chat.read_with(cx, |c, _| c.pin.clone()), Some(a), "wraps");
}

#[test]
fn a_picked_model_goes_first_in_recents_without_the_starred_ones() {
    let e = |m: &str| ("p".to_string(), m.to_string());
    let recents = [e("a"), e("b"), e("c"), e("d"), e("e")];
    let next = data::next_recents(&recents, &e("c"), &[e("b")]);
    assert_eq!(next, [e("c"), e("a"), e("d"), e("e")]);
    let next = data::next_recents(&recents, &e("z"), &[]);
    assert_eq!(next.len(), data::RECENT_MODELS_CAP);
    assert_eq!(next[0], e("z"));
}

#[gpui_kit::test]
fn the_token_bar_uses_the_reported_window_and_never_guesses_a_local_one(cx: &mut TestAppContext) {
    let (_w, chat) = open(cx);
    let provider = |id: &str, kind| AiProviderConfig {
        id: id.into(),
        kind,
        label: String::new(),
        base_url: String::new(),
    };
    let turn = |provider: &str, model: &str| {
        let mut t = ChatTurn::new(0, "q", "done");
        t.view.provider = Some(provider.into());
        t.view.model = Some(model.into());
        t
    };
    chat.update(cx, |c, _| {
        c.data.engine.providers = vec![
            provider("anthropic", AiProviderKind::Anthropic),
            provider("ollama", AiProviderKind::Ollama),
        ];
    });
    let window =
        |cx: &mut TestAppContext, t: &ChatTurn| chat.read_with(cx, |c, _| c.context_window_of(t));
    // A local server picks its own window: tokens alone, never a guess.
    assert_eq!(window(cx, &turn("ollama", "claude-sonnet-4-5")), None);
    let cloud = turn("anthropic", "claude-sonnet-4-5");
    let guess = format::context_window_for_model("claude-sonnet-4-5");
    assert_eq!(window(cx, &cloud), guess);
    chat.update(cx, |c, _| {
        c.usage_models.insert(
            "anthropic".into(),
            vec![crate::ui::model_picker::PoolModel {
                id: "claude-sonnet-4-5".into(),
                provider: "anthropic".into(),
                context_window: Some(1_000_000),
            }],
        );
    });
    assert_eq!(
        window(cx, &cloud),
        Some(1_000_000),
        "the provider's word wins"
    );
}

#[test]
fn a_listing_failure_never_shows_its_code() {
    assert_eq!(
        data::listing_failure_copy("provider_unreachable:ollama"),
        "unreachable"
    );
    assert_eq!(
        data::listing_failure_copy("needs_reconnect:chatgpt"),
        "sign in with ChatGPT again"
    );
    assert_eq!(data::listing_failure_copy("HTTP 500"), "HTTP 500");
}
