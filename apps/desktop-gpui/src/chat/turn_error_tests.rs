//! `turnErrorCopy` for every reason code and failure kind, and the inline
//! ChatGPT "Sign in again" through `FixtureSources`.

use capture_types::{AiEngineRef, AiProviderConfig, AiProviderKind, RecordingSettings};

use super::{error_copy, Action, ErrorCopy};
use crate::ui::Tone;

/// The parity fixture's settings with ChatGPT, a labelled OpenAI and Ollama
/// (the global default) connected.
fn settings() -> RecordingSettings {
    let map: serde_json::Value = serde_json::from_str(include_str!(
        "../../../desktop/parity/fixtures/chat/error.json"
    ))
    .unwrap();
    let mut s: RecordingSettings =
        serde_json::from_value(map["get_recording_settings"].clone()).unwrap();
    let pv = |id: &str, kind, label: &str| AiProviderConfig {
        id: id.into(),
        kind,
        label: label.into(),
        base_url: String::new(),
    };
    s.ai_runtime.providers.extend([
        pv("chatgpt", AiProviderKind::Chatgpt, ""),
        pv("openai-2", AiProviderKind::Openai, "Work"),
        pv("ollama", AiProviderKind::Ollama, ""),
    ]);
    s.ai_runtime.default_model = Some(AiEngineRef {
        provider: "ollama".into(),
        model: "qwen3:8b".into(),
    });
    s
}

fn code(raw: &str) -> ErrorCopy {
    error_copy(Some(raw), None, Some(&settings()))
}

#[test]
fn a_rejected_chatgpt_login_names_the_provider_and_offers_sign_in_only() {
    let c = code("needs_reconnect:chatgpt");
    assert_eq!(c.action, Action::Reconnect("chatgpt".into()));
    assert_eq!(
        (c.tone, c.title),
        (Tone::Danger, "Sign in again to keep chatting")
    );
    assert_eq!(
        c.text,
        "ChatGPT signed you out. Sign in again to keep chatting — retrying won't help until then."
    );
    assert!(!c.unreachable);
}

#[test]
fn an_unreachable_provider_never_asks_for_a_sign_in() {
    let c = code("provider_unreachable:chatgpt");
    assert_eq!(c.action, Action::Retry);
    assert_eq!(
        (c.tone, c.title),
        (Tone::Warn, "Can’t reach the AI provider right now")
    );
    assert_eq!(
        c.text,
        "Can't reach ChatGPT right now — your sign-in is fine."
    );
    assert!(c.unreachable, "the cloud-off icon");
    assert!(!c.text.to_lowercase().contains("sign in again"));

    let local = code("local_endpoint_unreachable");
    assert_eq!(local.action, Action::Retry);
    assert_eq!(
        local.text,
        "Can't reach Ollama at localhost:11434. Make sure it's running."
    );
    assert_eq!(local.tone, Tone::Warn);
}

#[test]
fn a_locked_keychain_retries_without_naming_settings() {
    let c = code("vault_denied");
    assert_eq!(c.action, Action::Retry);
    assert_eq!(
        (c.tone, c.title),
        (Tone::Danger, "Couldn’t answer this one")
    );
    assert_eq!(
        c.text,
        "Mnema couldn't read your saved keys from the keychain. Allow access, then try again."
    );
}

#[test]
fn a_setup_code_opens_settings_with_the_instance_label() {
    let c = code("no_provider_key:openai-2");
    assert_eq!(c.action, Action::Settings);
    assert_eq!(
        (c.tone, c.title),
        (Tone::Danger, "The AI settings need a look")
    );
    assert_eq!(c.text, "No API key saved for Work.");
    // An instance no longer in settings falls back to its kind's name.
    assert_eq!(
        code("no_provider_key:anthropic").text,
        "No API key saved for Anthropic."
    );
    assert_eq!(
        code("no_default_model").text,
        "Choose a global default model."
    );
    // No settings yet: the kind label still names it.
    assert_eq!(
        error_copy(Some("needs_reconnect:chatgpt"), None, None).text,
        "ChatGPT signed you out. Sign in again to keep chatting — retrying won't help until then."
    );
}

#[test]
fn a_sentence_keeps_its_text_and_its_kind_picks_the_action() {
    let s = Some("The provider said no.");
    let at = |kind| error_copy(s, kind, None);
    let quota = at(Some("quota"));
    assert_eq!(
        (quota.action, quota.tone, quota.title),
        (
            Action::Retry,
            Tone::Warn,
            "The provider is limiting requests"
        )
    );
    assert_eq!(quota.text, "The provider said no.");
    let long = at(Some("context_too_long"));
    assert_eq!(
        (long.action, long.tone, long.title),
        (
            Action::NewChat,
            Tone::Warn,
            "This chat is too long for the model"
        )
    );
    for kind in ["settings", "auth"] {
        let c = at(Some(kind));
        assert_eq!(
            (c.action, c.tone, c.title),
            (
                Action::Settings,
                Tone::Danger,
                "The AI settings need a look"
            )
        );
    }
    let off = at(Some("unreachable"));
    assert_eq!(
        (off.action, off.tone, off.unreachable),
        (Action::Retry, Tone::Warn, true)
    );
    let other = at(Some("retryable"));
    assert_eq!(
        (other.action, other.tone, other.title),
        (Action::Retry, Tone::Danger, "Couldn’t answer this one")
    );
    let empty = error_copy(None, None, None);
    assert_eq!(empty.text, "The engine couldn't answer.");
    assert_eq!(empty.action, Action::Retry);
}

#[cfg(feature = "fixtures")]
mod sign_in {
    use std::sync::Arc;

    use gpui_kit::base::Root;
    use gpui_kit::prelude::*;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{Entity, TestAppContext, WindowHandle};
    use serde_json::json;

    use super::super::Login;
    use crate::chat::fixture::FixtureSources;
    use crate::chat::tests::open;
    use crate::chat::{Chat, ChatTurn, Pane};

    const URL: &str = "https://auth.openai.com/codex/device";

    /// A chat whose trailing turn failed with a rejected ChatGPT login.
    fn setup(
        cx: &mut TestAppContext,
        login: Option<(String, String)>,
    ) -> (WindowHandle<Root>, Entity<Chat>, Arc<FixtureSources>) {
        let (w, chat) = open(cx);
        let src = Arc::new(FixtureSources {
            login,
            ..Default::default()
        });
        chat.update(cx, |c, cx| {
            c.sources = src.clone();
            c.data.settings = Some(super::settings());
            c.active = Some("c1".into());
            c.pane = Pane::Thread;
            let mut t = ChatTurn::new(0, "What did I ship?", "error");
            t.view.error_message = Some("needs_reconnect:chatgpt".into());
            c.turns.push(t);
            cx.notify();
        });
        (w, chat, src)
    }

    fn click(cx: &mut TestAppContext, w: WindowHandle<Root>, id: &'static str) {
        cx.update_window(w.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn present(cx: &mut TestAppContext, w: WindowHandle<Root>, id: &'static str) -> bool {
        cx.update_window(w.into(), |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    fn login(cx: &mut TestAppContext, chat: &Entity<Chat>) -> Option<(String, Login)> {
        chat.read_with(cx, |c, _| c.login.clone())
    }

    #[gpui_kit::test]
    fn sign_in_again_shows_the_code_and_cancel_tells_the_backend(cx: &mut TestAppContext) {
        let (w, chat, src) = setup(cx, Some(("ABCD-1234".into(), URL.into())));
        assert!(present(cx, w, "chat-signin"));
        assert!(!present(cx, w, "chat-err-retry"), "retrying can't help yet");
        assert!(!present(cx, w, "chat-err-settings"));
        click(cx, w, "chat-signin");
        assert_eq!(
            login(cx, &chat),
            Some((
                "chatgpt".into(),
                Login::Waiting {
                    code: "ABCD-1234".into(),
                    url: URL.into()
                }
            ))
        );
        assert!(present(cx, w, "chat-signin-code"));
        click(cx, w, "chat-signin-cancel");
        assert_eq!(login(cx, &chat), None);
        assert!(present(cx, w, "chat-signin"));
        assert_eq!(
            src.calls(),
            ["chatgpt_begin chatgpt", "chatgpt_cancel chatgpt"]
        );
    }

    #[gpui_kit::test]
    fn the_login_outcome_settles_the_inline_sign_in(cx: &mut TestAppContext) {
        let (w, chat, _) = setup(cx, Some(("ABCD-1234".into(), URL.into())));
        click(cx, w, "chat-signin");
        // Another provider's outcome is not ours.
        chat.update(cx, |c, cx| {
            c.on_chatgpt_update(&json!({ "providerId": "other", "connected": true }), cx)
        });
        assert!(matches!(login(cx, &chat), Some((_, Login::Waiting { .. }))));
        chat.update(cx, |c, cx| {
            c.on_chatgpt_update(
                &json!({ "providerId": "chatgpt", "connected": false, "error": "The code expired." }),
                cx,
            )
        });
        assert_eq!(
            login(cx, &chat),
            Some(("chatgpt".into(), Login::Failed("The code expired.".into())))
        );
        click(cx, w, "chat-signin");
        chat.update(cx, |c, cx| {
            c.on_chatgpt_update(&json!({ "providerId": "chatgpt", "connected": true }), cx)
        });
        assert_eq!(login(cx, &chat), None);
    }

    #[gpui_kit::test]
    fn a_failed_start_shows_why_and_keeps_the_button(cx: &mut TestAppContext) {
        let (w, chat, _) = setup(cx, None);
        click(cx, w, "chat-signin");
        assert_eq!(
            login(cx, &chat),
            Some((
                "chatgpt".into(),
                Login::Failed("OpenAI didn't answer.".into())
            ))
        );
        assert!(present(cx, w, "chat-signin"));
    }
}
