//! The MCP modals through the fixture backend: Custom add (verify, rollback,
//! validation, no secret in any saved settings), Custom edit, per-tool
//! curation, and the in-modal OAuth stages. No network, no vault, no child process.

use std::sync::Arc;

use gpui_kit::base::Root;
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle};
use serde_json::json;

use super::logic::Stage;
use super::{form, oauth, tools, Step};
use crate::settings::autosave::{Unit, RECORDING_DEBOUNCE};
use crate::settings::fixture::{Call, FixtureBackend};
use crate::settings::tests::{frame, open_at};
use crate::settings::Settings;

const SECRET: &str = "sk-fixture-never-saved";

type Open = (WindowHandle<Root>, Entity<Settings>, Arc<FixtureBackend>);

fn open(cx: &mut TestAppContext) -> Open {
    open_at(cx, false, (1280., 800.))
}

fn with<R>(
    cx: &mut TestAppContext,
    (w, v, _): &Open,
    f: impl FnOnce(&mut Settings, &mut gpui_kit::Window, &mut gpui_kit::Context<Settings>) -> R,
) -> R {
    let r = cx
        .update_window((*w).into(), |_, window, cx| {
            v.update(cx, |s, cx| f(s, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    frame(cx, *w);
    r
}

fn click(cx: &mut TestAppContext, w: WindowHandle<Root>, id: &'static str) {
    frame(cx, w);
    cx.update_window(w.into(), |_, window, cx| window.click(id, cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, w);
}

fn type_into(
    cx: &mut TestAppContext,
    o: &Open,
    pick: fn(&form::Form) -> Entity<InputState>,
    text: &str,
) {
    let text = text.to_string();
    with(cx, o, move |s, window, cx| {
        let Step::Form(f) = &s.ui.mcp.step else {
            panic!("no form")
        };
        pick(f).update(cx, |st, cx| st.set_value(text, window, cx));
    });
}

fn servers(cx: &mut TestAppContext, v: &Entity<Settings>) -> Vec<capture_types::McpServerConfig> {
    v.read_with(cx, |s, _| s.d().ai_runtime.mcp_servers)
}

fn mcp_calls(b: &FixtureBackend) -> Vec<Call> {
    b.calls()
        .into_iter()
        .filter(|c| !matches!(c, Call::McpCheckNode))
        .collect()
}

#[gpui_kit::test]
fn a_custom_http_connector_is_added_verified_and_its_secret_kept_out_of_settings(
    cx: &mut TestAppContext,
) {
    let o = open(cx);
    let (w, v, b) = (o.0, o.1.clone(), o.2.clone());
    *b.mcp_tools.lock().unwrap() = vec![("search".into(), None), ("read".into(), None)];
    with(cx, &o, |s, window, cx| form::open(s, None, window, cx));
    type_into(cx, &o, |f| f.name.clone(), "My API");
    type_into(cx, &o, |f| f.url.clone(), "https://api.example/mcp");
    type_into(cx, &o, |f| f.secret.clone(), SECRET);
    click(cx, w, "mcp-form-submit");

    assert_eq!(
        mcp_calls(&b),
        vec![
            Call::Domain(Unit::AiRuntime),
            Call::McpSetSecret("my-api".into()),
            Call::McpTools("my-api".into()),
        ],
        "persist, then the secret, then the verify"
    );
    let list = servers(cx, &v);
    assert_eq!(list[0].id, "my-api", "prepended");
    assert!(list[0].enabled);
    let sent = serde_json::to_string(&b.last_sent(Unit::AiRuntime).unwrap()).unwrap();
    assert!(
        !sent.contains(SECRET),
        "the secret never rides the settings"
    );
    v.read_with(cx, |s, _| {
        assert!(!s.ui.mcp_picker_open, "closed on success");
        assert_eq!(s.ui.mcp.counts.get("my-api"), Some(&2));
        assert!(s.data.mcp_secrets.contains("my-api"));
        assert!(
            !s.gate(Unit::AiRuntime).dirty,
            "the saved settings were adopted"
        );
    });
}

#[gpui_kit::test]
fn add_is_refused_until_the_transport_has_its_target(cx: &mut TestAppContext) {
    let o = open(cx);
    let (w, b) = (o.0, o.2.clone());
    with(cx, &o, |s, window, cx| form::open(s, None, window, cx));
    type_into(cx, &o, |f| f.name.clone(), "No URL");
    click(cx, w, "mcp-form-submit");
    assert!(mcp_calls(&b).is_empty(), "an HTTP connector needs a URL");
    // stdio: a command is needed; the URL no longer counts.
    with(cx, &o, |s, _, _| {
        if let Step::Form(f) = &mut s.ui.mcp.step {
            f.stdio = true;
        }
    });
    type_into(cx, &o, |f| f.url.clone(), "https://ignored");
    click(cx, w, "mcp-form-submit");
    assert!(
        mcp_calls(&b).is_empty(),
        "a stdio connector needs a command"
    );
}

#[gpui_kit::test]
fn a_failed_verify_rolls_the_connector_and_its_secret_back(cx: &mut TestAppContext) {
    let o = open(cx);
    let (w, v, b) = (o.0, o.1.clone(), o.2.clone());
    *b.mcp_error.lock().unwrap() = Some("401 Unauthorized".into());
    let before = servers(cx, &v);
    with(cx, &o, |s, window, cx| form::open(s, None, window, cx));
    type_into(cx, &o, |f| f.name.clone(), "Bad");
    type_into(cx, &o, |f| f.url.clone(), "https://bad.example/mcp");
    type_into(cx, &o, |f| f.secret.clone(), SECRET);
    click(cx, w, "mcp-form-submit");

    assert_eq!(
        mcp_calls(&b),
        vec![
            Call::Domain(Unit::AiRuntime),
            Call::McpSetSecret("bad".into()),
            Call::McpClearSecret("bad".into()),
            Call::Domain(Unit::AiRuntime),
        ]
    );
    assert_eq!(servers(cx, &v), before, "no orphan connector");
    v.read_with(cx, |s, _| {
        assert!(s.ui.mcp_picker_open, "stays open with the reason");
        let Step::Form(f) = &s.ui.mcp.step else {
            panic!("form")
        };
        assert_eq!(f.error.as_deref(), Some("401 Unauthorized"));
        assert!(!f.connecting);
    });
    // The pasted secret was cleared from the field when it was sent.
    let left = v.read_with(cx, |s, cx| match &s.ui.mcp.step {
        Step::Form(f) => f.secret.read(cx).value().to_string(),
        _ => String::new(),
    });
    assert!(left.is_empty());
}

#[gpui_kit::test]
fn a_custom_stdio_connector_keeps_args_env_and_secret_env(cx: &mut TestAppContext) {
    let o = open(cx);
    let (w, v) = (o.0, o.1.clone());
    with(cx, &o, |s, window, cx| form::open(s, None, window, cx));
    with(cx, &o, |s, _, _| {
        if let Step::Form(f) = &mut s.ui.mcp.step {
            f.stdio = true;
        }
    });
    type_into(cx, &o, |f| f.name.clone(), "Local");
    type_into(cx, &o, |f| f.command.clone(), "my-server");
    type_into(cx, &o, |f| f.secret_env.clone(), "TOKEN");
    click(cx, w, "mcp-form-arg-add");
    type_into(cx, &o, |f| f.args[0].clone(), "--stdio");
    click(cx, w, "mcp-form-env-add");
    type_into(cx, &o, |f| f.env[0].0.clone(), "MODE");
    type_into(cx, &o, |f| f.env[0].1.clone(), "fast");
    click(cx, w, "mcp-form-submit");
    let m = servers(cx, &v)
        .into_iter()
        .find(|m| m.id == "local")
        .unwrap();
    assert_eq!(m.command.as_deref(), Some("my-server"));
    assert_eq!(m.args, vec!["--stdio".to_string()]);
    assert_eq!(m.env[0].name, "MODE");
    assert_eq!(m.secret_env_name.as_deref(), Some("TOKEN"));
    assert_eq!(m.url, None);
}

#[gpui_kit::test]
fn editing_a_custom_connector_without_a_secret_rides_autosave(cx: &mut TestAppContext) {
    let o = open(cx);
    let (w, v, b) = (o.0, o.1.clone(), o.2.clone());
    let mut custom = super::logic::Custom {
        label: "Mine".into(),
        url: "https://a.example/mcp".into(),
        ..Default::default()
    }
    .build(&[]);
    custom.enabled_tools = Some(vec!["x".into()]);
    let keep = custom.clone();
    with(cx, &o, move |s, _, cx| {
        s.edit(move |d| d.ai_runtime.mcp_servers.push(keep), cx)
    });
    cx.executor().advance_clock(RECORDING_DEBOUNCE);
    cx.run_until_parked();
    let saves = b.calls().len();
    with(cx, &o, |s, window, cx| {
        form::open(s, Some("mine"), window, cx)
    });
    type_into(cx, &o, |f| f.url.clone(), "https://b.example/mcp");
    click(cx, w, "mcp-form-submit");
    assert_eq!(
        b.calls().len(),
        saves,
        "nothing verified, nothing saved yet"
    );
    let m = servers(cx, &v)
        .into_iter()
        .find(|m| m.id == "mine")
        .unwrap();
    assert_eq!(m.url.as_deref(), Some("https://b.example/mcp"));
    assert_eq!(m.enabled_tools, custom.enabled_tools, "curation kept");
    assert!(!v.read_with(cx, |s, _| s.ui.mcp_picker_open));
    cx.executor().advance_clock(RECORDING_DEBOUNCE);
    cx.run_until_parked();
    assert!(b.calls()[saves..].contains(&Call::Domain(Unit::AiRuntime)));
}

#[gpui_kit::test]
fn tool_curation_writes_the_connector_draft(cx: &mut TestAppContext) {
    let o = open(cx);
    let v = o.1.clone();
    *o.2.mcp_tools.lock().unwrap() = vec![
        ("a".into(), Some("first".into())),
        ("b".into(), None),
        ("c".into(), None),
    ];
    with(cx, &o, |s, window, cx| tools::open(s, "github", window, cx));
    let enabled = |cx: &mut TestAppContext| servers(cx, &v)[0].enabled_tools.clone();
    assert_eq!(enabled(cx), None, "uncurated: the default offer");
    v.read_with(cx, |s, _| {
        let list = s.ui.mcp.tools.as_ref().unwrap();
        assert!(!list.loading);
        assert_eq!(list.tools.len(), 3);
        assert_eq!(s.ui.mcp.counts.get("github"), Some(&3));
    });
    click(cx, o.0, "mcp-tool-1");
    assert_eq!(
        enabled(cx),
        Some(vec!["a".into(), "c".into()]),
        "materialized"
    );
    click(cx, o.0, "mcp-tools-none");
    assert_eq!(enabled(cx), Some(vec![]));
    click(cx, o.0, "mcp-tools-all");
    assert_eq!(enabled(cx), Some(vec!["a".into(), "b".into(), "c".into()]));
    click(cx, o.0, "mcp-tools-reset");
    assert_eq!(enabled(cx), None);
}

#[gpui_kit::test]
fn a_tool_list_failure_shows_the_reason_and_retries(cx: &mut TestAppContext) {
    let o = open(cx);
    let (v, b) = (o.1.clone(), o.2.clone());
    *b.mcp_error.lock().unwrap() = Some("connect failed".into());
    with(cx, &o, |s, window, cx| tools::open(s, "github", window, cx));
    let err = v.read_with(cx, |s, _| s.ui.mcp.tools.as_ref().unwrap().error.clone());
    assert_eq!(err.as_deref(), Some("connect failed"));
    *b.mcp_error.lock().unwrap() = None;
    click(cx, o.0, "mcp-tools-retry");
    let err = v.read_with(cx, |s, _| s.ui.mcp.tools.as_ref().unwrap().error.clone());
    assert_eq!(err, None);
    assert_eq!(
        b.calls()
            .iter()
            .filter(|c| matches!(c, Call::McpTools(_)))
            .count(),
        2
    );
}

fn stage(cx: &mut TestAppContext, v: &Entity<Settings>) -> Stage {
    v.read_with(cx, |s, _| match &s.ui.mcp.step {
        Step::OAuth(f) => oauth::stage(s, f),
        _ => panic!("not on the OAuth step"),
    })
}

#[gpui_kit::test]
fn the_oauth_preset_is_added_on_connect_and_follows_the_live_state(cx: &mut TestAppContext) {
    let o = open(cx);
    let (w, v, b) = (o.0, o.1.clone(), o.2.clone());
    *b.mcp_oauth.lock().unwrap() = vec![json!({ "id": "notion", "state": "authorizing" })];
    let i = super::PRESETS.iter().position(|p| p.oauth).unwrap();
    with(cx, &o, move |s, window, cx| {
        oauth::open_preset(s, i, window, cx)
    });
    assert_eq!(stage(cx, &v), Stage::Idle);
    assert!(
        servers(cx, &v).iter().all(|m| m.id != "notion"),
        "nothing added yet"
    );
    click(cx, w, "mcp-oauth-connect");
    assert_eq!(
        mcp_calls(&b),
        vec![
            Call::Domain(Unit::AiRuntime),
            Call::McpOAuthBegin("notion".into()),
            Call::McpOAuthStatuses,
        ]
    );
    assert_eq!(servers(cx, &v)[0].id, "notion");
    assert_eq!(stage(cx, &v), Stage::Authorizing);
    // The browser round trip is denied: the state falls back to none.
    with(cx, &o, |s, _, _| {
        s.data.mcp_oauth = vec![json!({ "id": "notion", "state": "none" })]
    });
    assert_eq!(stage(cx, &v), Stage::Denied);
    click(cx, w, "mcp-oauth-retry");
    assert_eq!(stage(cx, &v), Stage::Idle);
    // Approved: the callback's refresh lands `authorized`.
    with(cx, &o, |s, _, _| {
        s.data.mcp_oauth = vec![json!({ "id": "notion", "state": "authorized" })]
    });
    assert_eq!(stage(cx, &v), Stage::Authorized);
    click(cx, w, "mcp-oauth-done");
    assert!(!v.read_with(cx, |s, _| s.ui.mcp_picker_open));
}

#[gpui_kit::test]
fn a_failed_begin_is_denied_and_named_on_the_row(cx: &mut TestAppContext) {
    let o = open(cx);
    let (w, v, b) = (o.0, o.1.clone(), o.2.clone());
    *b.mcp_error.lock().unwrap() = Some("no dynamic client registration".into());
    with(cx, &o, |s, window, cx| {
        oauth::open_for(s, "github", true, window, cx)
    });
    click(cx, w, "mcp-oauth-connect");
    assert_eq!(stage(cx, &v), Stage::Denied);
    v.read_with(cx, |s, _| {
        assert_eq!(
            s.ui.mcp.oauth_errors.get("github").map(String::as_str),
            Some("no dynamic client registration")
        );
    });
}

#[gpui_kit::test]
fn a_missing_node_marks_stdio_rows(cx: &mut TestAppContext) {
    let backend = Arc::new(FixtureBackend {
        no_node: true,
        ..Default::default()
    });
    let o = open(cx);
    let (w, v) = (o.0, o.1.clone());
    with(cx, &o, move |s, _, cx| {
        s.set_backend(backend);
        s.open_tab("intelligence", cx);
        let m = super::logic::from_preset(&super::PRESETS[4], &[], true);
        s.edit(move |d| d.ai_runtime.mcp_servers.push(m), cx)
    });
    frame(cx, w);
    assert_eq!(v.read_with(cx, |s, _| s.ui.mcp.node.clone()), Some(None));
    let found = cx
        .update_window(w.into(), |_, window, _| {
            window.try_find("mcp-node-filesystem").is_some()
        })
        .unwrap();
    assert!(found, "the nodejs.org link shows on the stdio row");
}
