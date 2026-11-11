//! A preset's step-2 Connect body through the fixture backend: the catalog's
//! search and Enter, verify-before-add with rollback, the Node-missing add,
//! Advanced overrides, Esc back to the catalog, and edit mode from Configure.

use std::sync::Arc;

use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};

use super::flow_tests::{click, mcp_calls, open, servers, with, Open, SECRET};
use super::{preset, Step};
use crate::settings::autosave::Unit;
use crate::settings::fixture::{Call, FixtureBackend};
use crate::settings::tests::frame;
use crate::settings::Settings;

fn set(
    cx: &mut TestAppContext,
    o: &Open,
    pick: fn(&preset::Connect) -> Entity<InputState>,
    text: &str,
) {
    let text = text.to_string();
    with(cx, o, move |s, window, cx| {
        let Step::Preset(c) = &s.ui.mcp.step else {
            panic!("not on a preset's Connect step")
        };
        pick(c).update(cx, |st, cx| st.set_value(text, window, cx));
    });
}

fn open_catalog(cx: &mut TestAppContext, o: &Open) {
    with(cx, o, |s, _, cx| {
        s.ui.mcp_picker_open = true;
        cx.notify();
    });
}

/// The open step: `(preset id, editing id)`, or `None` off the preset step.
fn on_preset(
    cx: &mut TestAppContext,
    v: &Entity<Settings>,
) -> Option<(&'static str, Option<String>)> {
    v.read_with(cx, |s, _| match &s.ui.mcp.step {
        Step::Preset(c) => Some((super::PRESETS[c.preset].id, c.edit.clone())),
        _ => None,
    })
}

#[gpui_kit::test]
fn a_bearer_preset_opens_its_connect_step_and_is_verified_before_it_lands(cx: &mut TestAppContext) {
    let o = open(cx);
    let (w, v, b) = (o.0, o.1.clone(), o.2.clone());
    *b.mcp_tools.lock().unwrap() = vec![("issues".into(), None)];
    open_catalog(cx, &o);
    click(cx, w, "mcp-preset-linear");
    assert_eq!(
        on_preset(cx, &v),
        Some(("linear", None)),
        "a tile opens step 2, nothing added"
    );
    assert!(servers(cx, &v).iter().all(|m| m.id != "linear"));
    click(cx, w, "mcp-connect-submit");
    assert!(mcp_calls(&b).is_empty(), "Add waits for the API key");
    set(cx, &o, |c| c.token.clone(), SECRET);
    click(cx, w, "mcp-connect-submit");
    assert_eq!(
        mcp_calls(&b),
        vec![
            Call::Domain(Unit::AiRuntime),
            Call::McpSetSecret("linear".into()),
            Call::McpTools("linear".into()),
        ]
    );
    let list = servers(cx, &v);
    assert_eq!(
        (list[0].id.as_str(), list[0].enabled),
        ("linear", true),
        "prepended, on"
    );
    let sent = serde_json::to_string(&b.last_sent(Unit::AiRuntime).unwrap()).unwrap();
    assert!(!sent.contains(SECRET), "the token never rides the settings");
    v.read_with(cx, |s, _| {
        assert!(!s.ui.mcp_picker_open);
        assert_eq!(s.ui.mcp.counts.get("linear"), Some(&1));
    });
}

#[gpui_kit::test]
fn a_rejected_token_rolls_the_preset_back_and_says_why(cx: &mut TestAppContext) {
    let o = open(cx);
    let (w, v, b) = (o.0, o.1.clone(), o.2.clone());
    *b.mcp_error.lock().unwrap() = Some("401 Unauthorized".into());
    open_catalog(cx, &o);
    click(cx, w, "mcp-preset-stripe");
    set(cx, &o, |c| c.token.clone(), SECRET);
    click(cx, w, "mcp-connect-submit");
    assert!(
        servers(cx, &v).iter().all(|m| m.id != "stripe"),
        "rolled back"
    );
    v.read_with(cx, |s, _| {
        assert!(s.ui.mcp_picker_open, "stays open");
        let Step::Preset(c) = &s.ui.mcp.step else {
            panic!()
        };
        assert_eq!(c.error.as_deref(), Some("401 Unauthorized"));
        assert!(!c.connecting);
        assert!(!s.data.mcp_secrets.contains("stripe"));
    });
}

#[gpui_kit::test]
fn a_local_preset_without_node_lands_switched_off_and_unverified(cx: &mut TestAppContext) {
    let backend = Arc::new(FixtureBackend {
        no_node: true,
        ..Default::default()
    });
    let o = open(cx);
    let (w, v) = (o.0, o.1.clone());
    let b = backend.clone();
    with(cx, &o, move |s, _, _| s.set_backend(backend));
    open_catalog(cx, &o);
    click(cx, w, "mcp-preset-filesystem");
    assert_eq!(
        v.read_with(cx, |s, _| s.ui.mcp.node.clone()),
        Some(None),
        "probed"
    );
    assert!(
        cx.update_window(w.into(), |_, window, _| window
            .try_find("mcp-connect-node")
            .is_some())
            .unwrap(),
        "the nodejs.org link shows"
    );
    click(cx, w, "mcp-connect-submit");
    assert_eq!(
        mcp_calls(&b),
        vec![Call::Domain(Unit::AiRuntime)],
        "no secret, no tool listing"
    );
    let m = &servers(cx, &v)[0];
    assert_eq!((m.id.as_str(), m.enabled), ("filesystem", false));
}

#[gpui_kit::test]
fn advanced_edits_ride_the_add_and_esc_steps_back(cx: &mut TestAppContext) {
    let o = open(cx);
    let (w, v) = (o.0, o.1.clone());
    open_catalog(cx, &o);
    click(cx, w, "mcp-preset-github");
    click(cx, w, "mcp-adv-toggle");
    set(cx, &o, |c| c.name.clone(), "Work GH");
    set(cx, &o, |c| c.url.clone(), "https://gh.example/mcp");
    // Esc on an add step returns to the catalog; picking again starts fresh.
    with(cx, &o, |s, _, cx| assert!(super::escape(s, cx)));
    assert!(matches!(v.read_with(cx, |s, _| s.ui.mcp_picker_open), true));
    assert_eq!(on_preset(cx, &v), None, "back on the catalog");
    click(cx, w, "mcp-preset-github");
    click(cx, w, "mcp-adv-toggle");
    set(cx, &o, |c| c.name.clone(), "Work GH");
    set(cx, &o, |c| c.url.clone(), "https://gh.example/mcp");
    set(cx, &o, |c| c.token.clone(), SECRET);
    click(cx, w, "mcp-connect-submit");
    let m = &servers(cx, &v)[0];
    assert_eq!(m.id, "github-2", "a second GitHub gets a suffixed id");
    assert_eq!(m.label, "Work GH");
    assert_eq!(m.url.as_deref(), Some("https://gh.example/mcp"));
}

#[gpui_kit::test]
fn search_filters_and_enter_picks_the_first_tile_else_custom(cx: &mut TestAppContext) {
    let o = open(cx);
    let (w, v) = (o.0, o.1.clone());
    let search = |cx: &mut TestAppContext, q: &str| {
        let q = q.to_string();
        with(cx, &o, move |s, window, cx| {
            let (st, _) = s.ui.mcp.search.as_ref().expect("the catalog's search");
            st.clone().update(cx, |st, cx| st.set_value(q, window, cx));
        });
    };
    open_catalog(cx, &o);
    search(cx, "invoices");
    let shown = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(w.into(), |_, window, _| window.try_find(id).is_some())
            .unwrap()
    };
    assert!(shown(cx, "mcp-preset-stripe") && !shown(cx, "mcp-preset-github"));
    cx.simulate_keystrokes(w.into(), "enter");
    frame(cx, w);
    assert_eq!(on_preset(cx, &v), Some(("stripe", None)));
    with(cx, &o, |s, _, cx| assert!(super::escape(s, cx)));
    frame(cx, w);
    search(cx, "zzz");
    cx.simulate_keystrokes(w.into(), "enter");
    frame(cx, w);
    assert!(
        v.read_with(
            cx,
            |s, _| matches!(&s.ui.mcp.step, Step::Form(f) if f.edit.is_none())
        ),
        "nothing matches: Enter opens Custom"
    );
}

#[gpui_kit::test]
fn configure_edits_a_preset_connector_in_place(cx: &mut TestAppContext) {
    let o = open(cx);
    let (w, v, b) = (o.0, o.1.clone(), o.2.clone());
    let github = super::PRESETS
        .iter()
        .position(|p| p.id == "github")
        .unwrap();
    with(cx, &o, move |s, window, cx| {
        preset::open(s, github, Some("github"), window, cx)
    });
    assert_eq!(on_preset(cx, &v), Some(("github", Some("github".into()))));
    click(cx, w, "mcp-adv-toggle");
    set(cx, &o, |c| c.url.clone(), "https://gh.example/mcp");
    // Typed (`set_value` stays silent): the field's Change is what writes through.
    with(cx, &o, |s, window, cx| {
        let Step::Preset(c) = &s.ui.mcp.step else {
            panic!()
        };
        c.url.clone().update(cx, |st, cx| st.focus(window, cx));
    });
    cx.update_window(w.into(), |_, window, cx| window.input("/v2", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        servers(cx, &v)[0].url.as_deref(),
        Some("https://gh.example/mcp/v2"),
        "an Advanced edit lands on the connector at once"
    );
    click(cx, w, "mcp-connect-submit");
    assert!(
        !v.read_with(cx, |s, _| s.ui.mcp_picker_open),
        "Save without a token closes"
    );
    assert!(
        mcp_calls(&b)
            .iter()
            .all(|c| !matches!(c, Call::McpTools(_))),
        "nothing to verify"
    );
    with(cx, &o, move |s, window, cx| {
        preset::open(s, github, Some("github"), window, cx)
    });
    set(cx, &o, |c| c.token.clone(), SECRET);
    click(cx, w, "mcp-connect-submit");
    let calls = mcp_calls(&b);
    assert!(calls.contains(&Call::McpSetSecret("github".into())));
    assert!(
        calls.contains(&Call::McpTools("github".into())),
        "a new token re-verifies"
    );
    assert!(
        servers(cx, &v).iter().any(|m| m.id == "github"),
        "an edit never rolls back"
    );
}

#[gpui_kit::test]
fn a_just_added_row_flashes_once(cx: &mut TestAppContext) {
    let o = open(cx);
    let (w, v) = (o.0, o.1.clone());
    open_catalog(cx, &o);
    click(cx, w, "mcp-preset-linear");
    set(cx, &o, |c| c.token.clone(), SECRET);
    click(cx, w, "mcp-connect-submit");
    assert_eq!(
        v.read_with(cx, |s, _| s.ui.mcp.flash.clone()),
        Some("linear".into())
    );
    cx.executor()
        .advance_clock(crate::theme::motion::FLASH - std::time::Duration::from_millis(1));
    cx.run_until_parked();
    assert!(
        v.read_with(cx, |s, _| s.ui.mcp.flash.is_some()),
        "1399 ms: still washing"
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(v.read_with(cx, |s, _| s.ui.mcp.flash.clone()), None);
}

#[gpui_kit::test]
fn an_authorizing_row_spins_beside_its_word(cx: &mut TestAppContext) {
    let o = open(cx);
    let (w, v) = (o.0, o.1.clone());
    with(cx, &o, |s, _, cx| {
        let mut m = super::logic::from_preset(&super::PRESETS[3], &[], true);
        m.id = "notion".into();
        s.edit(move |d| d.ai_runtime.mcp_servers.push(m), cx);
        s.data.mcp_oauth = vec![serde_json::json!({ "id": "notion", "state": "authorizing" })];
        s.open_tab("intelligence", cx);
    });
    frame(cx, w);
    let shown = cx
        .update_window(w.into(), |_, window, _| {
            window.try_find("mcp-authorizing-notion").is_some()
        })
        .unwrap();
    assert!(shown);
    let _ = v;
}

#[gpui_kit::test]
fn editing_a_custom_connector_saves_as_you_type(cx: &mut TestAppContext) {
    let o = open(cx);
    let (w, v) = (o.0, o.1.clone());
    with(cx, &o, |s, _, cx| {
        let m = super::logic::Custom {
            label: "My API".into(),
            url: "https://api.example/mcp".into(),
            ..Default::default()
        }
        .build(&[]);
        s.edit(move |d| d.ai_runtime.mcp_servers.push(m), cx);
    });
    with(cx, &o, |s, window, cx| {
        super::form::open(s, Some("my-api"), window, cx)
    });
    with(cx, &o, |s, window, cx| {
        let Step::Form(f) = &s.ui.mcp.step else {
            panic!()
        };
        f.name.clone().update(cx, |st, cx| {
            st.set_value("", window, cx);
            st.focus(window, cx)
        });
    });
    cx.update_window(w.into(), |_, window, cx| window.input("My API v2", cx))
        .unwrap();
    cx.run_until_parked();
    let label = |cx: &mut TestAppContext| {
        servers(cx, &v)
            .into_iter()
            .find(|m| m.id == "my-api")
            .unwrap()
            .label
    };
    assert_eq!(label(cx), "My API v2", "landed without Save");
    // Esc closes the edit; the edit stays (Svelte binds the live draft).
    with(cx, &o, |s, _, cx| assert!(super::escape(s, cx)));
    assert!(!v.read_with(cx, |s, _| s.ui.mcp_picker_open));
    assert_eq!(label(cx), "My API v2");
}

#[gpui_kit::test]
fn removing_a_connector_offers_keep_connector(cx: &mut TestAppContext) {
    let o = open(cx);
    let (host, _dir) = crate::settings::sections::pickers_tests::confirm_core(cx, false);
    let github = super::PRESETS
        .iter()
        .position(|p| p.id == "github")
        .unwrap();
    with(cx, &o, move |s, window, cx| {
        preset::open(s, github, Some("github"), window, cx)
    });
    click(cx, o.0, "mcp-connect-remove");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Remove this connector?".to_string()]
    );
    assert_eq!(
        *host.cancels.lock().unwrap(),
        vec!["Keep Connector".to_string()]
    );
    assert!(
        servers(cx, &o.1).iter().any(|m| m.id == "github"),
        "Keep keeps it"
    );
}
