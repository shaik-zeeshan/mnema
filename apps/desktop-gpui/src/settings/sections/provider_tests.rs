//! Settings › Intelligence › Providers, Ask AI and User Context driven by
//! clicks and typing on the real view over the fixture backend: Manage on
//! every kind, the key's save / clear (with its confirm), Remove provider, the
//! ChatGPT sign-in, Test connection and the gating Svelte applies.

use std::sync::Arc;

use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle};
use serde_json::json;

use super::pickers_tests::confirm_core as bare_core;
use crate::settings::fixture::{Call, FixtureBackend, CHATGPT_CODE};
use crate::settings::tests::{frame, open_at};
use crate::settings::Settings;

const TALL: (f32, f32) = (1280., 4000.);

/// A core that answers confirms, plus the tokio bridge a landed action's
/// status re-read runs on.
fn confirm_core(
    cx: &mut TestAppContext,
    answer: bool,
) -> (Arc<super::pickers_tests::ConfirmHost>, tempfile::TempDir) {
    let core = bare_core(cx, answer);
    cx.update(crate::bridge::tokio::init);
    core
}

fn open(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<Settings>, Arc<FixtureBackend>) {
    let (w, v, b) = open_at(cx, false, TALL);
    v.update(cx, |s, cx| s.open_tab("intelligence", cx));
    frame(cx, w);
    (w, v, b)
}

fn on_window<R>(
    cx: &mut TestAppContext,
    w: WindowHandle<Root>,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    let r = cx
        .update_window(w.into(), |_, window, cx| {
            window.render_frame(cx);
            f(window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    r
}

fn click(cx: &mut TestAppContext, w: WindowHandle<Root>, id: &str) {
    let id = gpui_kit::ElementId::Name(id.to_string().into());
    on_window(cx, w, |window, cx| window.click(id, cx));
}

fn present(cx: &mut TestAppContext, w: WindowHandle<Root>, id: &str) -> bool {
    let id = gpui_kit::ElementId::Name(id.to_string().into());
    on_window(cx, w, |window, _| window.try_find(id).is_some())
}

fn label(cx: &mut TestAppContext, w: WindowHandle<Root>, id: &str) -> String {
    let id = gpui_kit::ElementId::Name(id.to_string().into());
    on_window(cx, w, |window, _| {
        window.find(id).label().unwrap_or_default().to_string()
    })
}

/// Click a field, then type into it.
fn type_into(cx: &mut TestAppContext, w: WindowHandle<Root>, id: &str, text: &str) {
    click(cx, w, id);
    on_window(cx, w, |window, cx| window.input(text, cx));
}

fn has(b: &FixtureBackend, c: &Call) -> bool {
    b.calls().contains(c)
}

#[gpui_kit::test]
fn manage_on_ollama_opens_label_endpoint_and_remove(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx);
    assert!(!present(cx, w, "pv-endpoint-ollama"), "closed: no fields");
    click(cx, w, "pv-manage-ollama");
    let open = v.read_with(cx, |s, _| s.ui.provider_open.clone());
    assert_eq!(open.as_deref(), Some("ollama"), "Manage reads Done now");
    for id in ["pv-label-ollama", "pv-endpoint-ollama", "pv-remove-ollama"] {
        assert!(present(cx, w, id), "{id} is drawn");
    }
    assert!(!present(cx, w, "pv-key-ollama"), "a local kind has no key");
    click(cx, w, "pv-manage-ollama");
    assert!(!present(cx, w, "pv-endpoint-ollama"), "Done closes it");
}

#[gpui_kit::test]
fn one_row_open_at_a_time(cx: &mut TestAppContext) {
    let (w, _, _) = open(cx);
    click(cx, w, "pv-manage-ollama");
    click(cx, w, "pv-manage-anthropic");
    assert!(present(cx, w, "pv-key-anthropic"));
    assert!(!present(cx, w, "pv-endpoint-ollama"));
}

#[gpui_kit::test]
fn typing_the_endpoint_and_label_edits_the_draft(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx);
    click(cx, w, "pv-manage-ollama");
    type_into(cx, w, "pv-endpoint-ollama", "http://127.0.0.1:9");
    type_into(cx, w, "pv-label-ollama", "Box");
    let pv = v.read_with(cx, |s, _| s.d().ai_runtime.providers[1].clone());
    assert_eq!(pv.base_url, "http://127.0.0.1:9");
    assert_eq!(pv.label, "Box");
}

#[gpui_kit::test]
fn a_typed_key_pins_the_row_until_saved(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx);
    click(cx, w, "pv-manage-anthropic");
    type_into(cx, w, "pv-key-anthropic", "sk-ant-test");
    assert!(
        !present(cx, w, "pv-manage-anthropic"),
        "an unsaved key hides Done: the row stays open"
    );
    click(cx, w, "pv-save-key-anthropic");
    assert!(has(&b, &Call::SaveKey("anthropic".into())));
    frame(cx, w);
    let typed = v.read_with(cx, |s, cx| s.typed_key("anthropic", cx));
    assert_eq!(typed, "", "the field forgets the key once it is vaulted");
    assert!(present(cx, w, "pv-manage-anthropic"), "Done is back");
}

#[gpui_kit::test]
fn clear_asks_first_and_keep_key_keeps_it(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx);
    let (host, _dir) = confirm_core(cx, false);
    click(cx, w, "pv-manage-anthropic");
    click(cx, w, "pv-clear-key-anthropic");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Delete this API key?".to_string()]
    );
    assert!(!has(&b, &Call::ClearKey("anthropic".into())));
    v.read_with(cx, |s, _| {
        assert!(s.data.keys_saved.contains("anthropic"));
        assert!(s.ui.pv.saving.is_none(), "a cancel releases the latch");
    });
}

#[gpui_kit::test]
fn a_confirmed_clear_deletes_the_key(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx);
    let _core = confirm_core(cx, true);
    click(cx, w, "pv-manage-anthropic");
    click(cx, w, "pv-clear-key-anthropic");
    assert!(has(&b, &Call::ClearKey("anthropic".into())));
    assert!(!v.read_with(cx, |s, _| s.data.keys_saved.contains("anthropic")));
}

#[gpui_kit::test]
fn removing_a_local_provider_needs_no_confirm(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx);
    click(cx, w, "pv-manage-ollama");
    click(cx, w, "pv-remove-ollama");
    let ids: Vec<String> = v.read_with(cx, |s, _| {
        s.d()
            .ai_runtime
            .providers
            .iter()
            .map(|p| p.id.clone())
            .collect()
    });
    assert_eq!(ids, vec!["anthropic".to_string()]);
    assert!(b.calls().iter().all(|c| !matches!(c, Call::ClearKey(_))));
}

#[gpui_kit::test]
fn removing_the_default_s_provider_clears_the_default_and_its_key(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx);
    let (host, _dir) = confirm_core(cx, true);
    v.update(cx, |s, cx| {
        s.edit(
            |d| {
                d.access.ask_ai_model = Some("claude-haiku".into());
                d.access.ask_ai_model_provider = Some("anthropic".into());
            },
            cx,
        )
    });
    click(cx, w, "pv-manage-anthropic");
    click(cx, w, "pv-remove-anthropic");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Remove this provider?".to_string()]
    );
    assert!(has(&b, &Call::ClearKey("anthropic".into())));
    let d = v.read_with(cx, |s, _| s.d());
    assert!(d.ai_runtime.providers.iter().all(|p| p.id != "anthropic"));
    assert_eq!(d.ai_runtime.default_model, None);
    assert_eq!(d.access.ask_ai_model, None);
    assert_eq!(d.access.ask_ai_model_provider, None);
}

#[gpui_kit::test]
fn an_added_provider_opens_its_own_row(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx);
    click(cx, w, "pv-add");
    click(cx, w, "pv-kind-OpenAI-compatible");
    let added = v.read_with(cx, |s, _| s.ui.provider_open.clone());
    assert_eq!(added.as_deref(), Some("openai_compatible"));
    assert!(
        !present(cx, w, "pv-test-openai_compatible"),
        "no key yet: nothing to test"
    );
    assert!(present(cx, w, "pv-base-url-openai_compatible"));
    assert!(present(cx, w, "pv-key-openai_compatible"));
    // No base URL pins it open: no Done to collapse it.
    assert!(!present(cx, w, "pv-manage-openai_compatible"));
}

#[gpui_kit::test]
fn chatgpt_signs_in_with_a_device_code(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx);
    click(cx, w, "pv-add");
    click(cx, w, "pv-kind-ChatGPT");
    assert!(present(cx, w, "pv-cg-connect-chatgpt"), "Connect ChatGPT");
    assert!(!present(cx, w, "pv-cg-disconnect-chatgpt"));
    assert!(present(cx, w, "pv-remove-chatgpt"), "the new row opened");
    click(cx, w, "pv-cg-connect-chatgpt");
    assert!(has(&b, &Call::ChatgptBegin("chatgpt".into())));
    assert_eq!(label(cx, w, "pv-cg-code-chatgpt"), "Click to copy");
    assert!(present(cx, w, "pv-cg-copy-chatgpt"));
    let code = v.read_with(cx, |s, _| s.ui.pv.login.get("chatgpt").cloned());
    assert!(
        matches!(code, Some(super::provider_row::Login::Waiting { code, .. }) if code == CHATGPT_CODE)
    );
    click(cx, w, "pv-cg-cancel-chatgpt");
    assert!(has(&b, &Call::ChatgptCancel("chatgpt".into())));
    assert!(present(cx, w, "pv-cg-connect-chatgpt"), "back to Connect");
}

#[gpui_kit::test]
fn a_landed_sign_in_reads_connected_with_disconnect(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx);
    click(cx, w, "pv-add");
    click(cx, w, "pv-kind-ChatGPT");
    click(cx, w, "pv-cg-connect-chatgpt");
    v.update(cx, |s, cx| {
        s.on_chatgpt_update(&json!({ "providerId": "chatgpt", "connected": true }), cx)
    });
    assert!(present(cx, w, "pv-cg-disconnect-chatgpt"));
    v.update(cx, |s, cx| {
        s.on_chatgpt_update(
            &json!({ "providerId": "chatgpt", "connected": false, "error": "expired" }),
            cx,
        )
    });
    assert!(present(cx, w, "pv-cg-error-chatgpt"));
}

#[gpui_kit::test]
fn disconnect_asks_with_stay_connected(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx);
    let (host, _dir) = confirm_core(cx, false);
    click(cx, w, "pv-add");
    click(cx, w, "pv-kind-ChatGPT");
    v.update(cx, |s, cx| {
        s.on_chatgpt_update(&json!({ "providerId": "chatgpt", "connected": true }), cx)
    });
    click(cx, w, "pv-cg-disconnect-chatgpt");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Disconnect ChatGPT?".to_string()]
    );
    assert!(!has(&b, &Call::ClearKey("chatgpt".into())));
    assert!(v.read_with(cx, |s, _| s.ui.pv.disconnecting.is_none()));
}

#[gpui_kit::test]
fn test_connection_reports_then_the_master_switch_clears_it(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx);
    click(cx, w, "ai-test");
    assert!(has(&b, &Call::TestConnection));
    assert!(v.read_with(cx, |s, _| matches!(s.ui.pv.test.result, Some(Ok(_)))));
    assert!(present(cx, w, "ai-test-raw"), "the raw reply can be opened");
    click(cx, w, "ai-enabled");
    assert!(v.read_with(cx, |s, _| s.ui.pv.test.result.is_none()));
}

#[gpui_kit::test]
fn ask_ai_off_disables_its_model_picker(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx);
    v.update(cx, |s, cx| s.edit(|d| d.access.ask_ai_enabled = false, cx));
    click(cx, w, "askai-model");
    let open = on_window(cx, w, |window, _| window.find("askai-model").expanded());
    assert_ne!(open, Some(true), "a disabled picker doesn't open");
}
