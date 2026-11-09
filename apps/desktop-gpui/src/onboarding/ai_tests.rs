//! Onboarding's AI setup: ready means a live listing (never "a key was
//! saved"), a rejected key and an unreachable engine read differently, the
//! key verifies itself 650 ms after the last keystroke, the ChatGPT outcome
//! lands on the flow, and skipping AI still finishes.

use std::sync::Arc;
use std::time::Duration;

use capture_types::{AiEngineRef, AiProviderConfig, AiProviderKind, AppearanceSetting};
use gpui_kit::{Entity, TestAppContext, VisualTestContext, WindowAppearance};
use serde_json::{json, Value};

use super::ai::{self, Check, Failure};
use super::data::Backend;
use super::fixture::FixtureBackend;
use super::flow::Step;
use super::Onboarding;
use crate::theme;

const KEY: &str = "sk-ant-test-0123456789";

fn open(
    cx: &mut TestAppContext,
    verify: Option<Value>,
) -> (
    Entity<Onboarding>,
    &mut VisualTestContext,
    Arc<FixtureBackend>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
    });
    let mut map = crate::shell::fixture::load("onboarding", "granted").unwrap();
    if let Some(v) = verify {
        map["verify_ai_provider"] = v;
    }
    let backend = Arc::new(FixtureBackend::new(map));
    let b: Arc<dyn Backend> = backend.clone();
    let (view, vcx) = cx.add_window_view(move |_, cx| Onboarding::new(b.clone(), |_, _| {}, cx));
    vcx.run_until_parked();
    view.update_in(vcx, |o, window, cx| {
        o.go(Step::Change, cx);
        o.tab = 3;
        o.ai_inputs(window, cx);
        o.ai_set_mode(true, cx);
        o.ai_set_kind(AiProviderKind::Anthropic, cx);
    });
    vcx.run_until_parked();
    (view, vcx, backend)
}

/// Type the key, then press Verify now.
fn connect(view: &Entity<Onboarding>, vcx: &mut VisualTestContext) {
    view.update_in(vcx, |o, window, cx| {
        let key = o.ai.key.clone().unwrap();
        key.update(cx, |s, cx| s.set_value(KEY, window, cx));
        o.ai_connect_cloud(window, cx);
    });
    vcx.run_until_parked();
}

#[gpui_kit::test]
fn a_verified_key_flips_to_ready(cx: &mut TestAppContext) {
    let (view, vcx, backend) = open(cx, None);
    view.update(vcx, |o, _| assert!(!o.ai_ready()));
    connect(&view, vcx);
    view.update(vcx, |o, cx| {
        assert!(matches!(
            o.ai.checks.get("anthropic"),
            Some(Check::Live { .. })
        ));
        assert_eq!(
            o.draft.ai_runtime.default_model,
            Some(AiEngineRef {
                provider: "anthropic".into(),
                model: "claude-sonnet-4-5".into()
            }),
            "the first engine to answer seeds the default"
        );
        assert!(o.ai_ready());
        assert!(o.features.ai_features, "becoming ready switches the row on");
        assert!(o.ai.keys_saved.contains("anthropic"));
        assert_eq!(
            o.ai.key.as_ref().unwrap().read(cx).value().as_ref(),
            "",
            "the field forgets the key"
        );
    });
    let calls = backend.calls();
    assert!(
        calls.contains(&"save_key:anthropic".to_string()),
        "{calls:?}"
    );
    assert!(calls.contains(&"verify:anthropic".to_string()), "{calls:?}");
    assert!(
        !calls.iter().any(|c| c.contains(KEY)),
        "the key is never recorded"
    );
}

#[gpui_kit::test]
fn a_rejected_key_shows_the_error(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx, Some(json!({ "__reject": "authentication failed" })));
    connect(&view, vcx);
    view.update(vcx, |o, _| {
        assert_eq!(
            o.ai.checks.get("anthropic"),
            Some(&Check::Failed(Failure::Rejected))
        );
        assert!(!o.ai_ready(), "a stored key that was refused is not ready");
        assert!(!o.features.ai_features);
        let why = o.ai_missing().unwrap();
        assert!(why.contains("key rejected"), "{why}");
    });
}

#[gpui_kit::test]
fn an_unreachable_provider_reads_cant_reach_not_invalid(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx, Some(json!({ "__reject": "unreachable" })));
    connect(&view, vcx);
    view.update(vcx, |o, _| {
        assert_eq!(
            o.ai.checks.get("anthropic"),
            Some(&Check::Failed(Failure::Unreachable))
        );
        assert!(!o.ai_ready());
        let why = o.ai_missing().unwrap();
        assert!(why.contains("Can\u{2019}t reach"), "{why}");
        assert!(!why.contains("reject") && !why.contains("invalid"), "{why}");
    });
}

#[test]
fn reasons_split_by_what_the_user_should_do() {
    use Failure::*;
    assert_eq!(Failure::classify("authentication failed"), Rejected);
    assert_eq!(Failure::classify("unreachable"), Unreachable);
    // The ChatGPT codes: unreachable is a liveness problem, needs_reconnect a sign-in.
    assert_eq!(
        Failure::classify("provider_unreachable:chatgpt"),
        Unreachable
    );
    assert_eq!(
        Failure::classify("Provider_unreachable:chatgpt-2"),
        Unreachable
    );
    assert_eq!(Failure::classify("needs_reconnect:chatgpt"), NeedsSignIn);
    assert_eq!(
        Failure::classify("missing API key"),
        Other("missing API key".into())
    );
}

#[test]
fn a_saved_key_and_a_default_model_are_not_ready_without_a_live_listing() {
    let providers = vec![AiProviderConfig {
        id: "openai".into(),
        kind: AiProviderKind::Openai,
        label: String::new(),
        base_url: String::new(),
    }];
    let default = AiEngineRef {
        provider: "openai".into(),
        model: "gpt-5-mini".into(),
    };
    let mut checks = std::collections::HashMap::new();
    assert!(ai::missing(&providers, &checks, Some(&default)).is_some());
    checks.insert(
        "openai".into(),
        Check::Live {
            models: vec!["gpt-5".into()],
            latency_ms: 1,
        },
    );
    assert!(
        ai::missing(&providers, &checks, Some(&default))
            .unwrap()
            .contains("no longer lists"),
        "a live listing without the chosen model isn't ready"
    );
    checks.insert(
        "openai".into(),
        Check::Live {
            models: vec!["gpt-5-mini".into()],
            latency_ms: 1,
        },
    );
    assert_eq!(ai::missing(&providers, &checks, Some(&default)), None);
}

#[gpui_kit::test]
fn a_typed_key_verifies_itself_650_ms_after_the_last_keystroke(cx: &mut TestAppContext) {
    let (view, vcx, backend) = open(cx, None);
    view.update_in(vcx, |o, window, cx| {
        let key = o.ai.key.clone().unwrap();
        key.update(cx, |s, cx| s.focus(window, cx));
    });
    vcx.run_until_parked();
    vcx.simulate_input(KEY);
    vcx.run_until_parked();
    vcx.executor()
        .advance_clock(ai::KEY_DEBOUNCE - Duration::from_millis(1));
    vcx.run_until_parked();
    assert!(
        !backend.calls().iter().any(|c| c.starts_with("save_key")),
        "not at 649 ms"
    );
    vcx.executor().advance_clock(Duration::from_millis(1));
    vcx.run_until_parked();
    assert!(
        backend.calls().contains(&"save_key:anthropic".to_string()),
        "at 650 ms"
    );
}

#[gpui_kit::test]
fn the_chatgpt_sign_in_shows_a_code_then_verifies_on_the_outcome(cx: &mut TestAppContext) {
    let (view, vcx, backend) = open(cx, None);
    view.update(vcx, |o, cx| {
        o.ai_set_kind(AiProviderKind::Chatgpt, cx);
        o.ai_chatgpt_sign_in(cx);
    });
    vcx.run_until_parked();
    view.update(vcx, |o, _| {
        assert!(matches!(&o.ai.login, Some((id, ai::Login::Waiting { code, .. })) if id == "chatgpt" && code == "WXYZ-1234"));
        assert!(!o.ai_ready());
    });
    assert!(backend
        .calls()
        .iter()
        .any(|c| c.starts_with("open_url:https://auth.openai.com")));
    view.update(vcx, |o, cx| {
        o.on_chatgpt_update(&json!({ "providerId": "chatgpt", "connected": true }), cx)
    });
    vcx.run_until_parked();
    view.update(vcx, |o, _| {
        assert!(o.ai.login.is_none());
        assert!(o.ai.keys_saved.contains("chatgpt"));
        assert!(o.ai_ready(), "the token set verified live");
    });
}

#[gpui_kit::test]
fn skipping_ai_still_finishes(cx: &mut TestAppContext) {
    let (view, vcx, backend) = open(cx, None);
    view.update(vcx, |o, cx| {
        o.ai_set_mode(false, cx);
        o.go(Step::Finale, cx);
    });
    vcx.run_until_parked();
    view.update(vcx, |o, _| {
        assert!(o.completed, "AI is never a hold");
        assert!(!o.draft.ai_runtime.enabled);
    });
    assert!(backend.calls().iter().any(|c| c.starts_with("finish")));
}
