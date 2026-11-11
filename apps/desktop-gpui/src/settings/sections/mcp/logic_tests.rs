//! The Svelte `.test.ts` cases for the MCP rules: same inputs, same answers.

use super::*;

fn many(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("t{i}")).collect()
}

fn v(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

// ── newMcpServerId (ai-providers.test.ts) ──────────────────────────────────

#[test]
fn ids_slug_a_label_to_the_load_bearing_charset() {
    assert_eq!(new_server_id("GitHub", &[]), "github");
    assert_eq!(new_server_id("My Cool Server!", &[]), "my-cool-server");
    assert_eq!(new_server_id("  spaced  ", &[]), "spaced");
}

#[test]
fn ids_fall_back_to_connector() {
    assert_eq!(new_server_id("", &[]), "connector");
    assert_eq!(new_server_id("!!!", &[]), "connector");
}

#[test]
fn ids_suffix_on_collision() {
    assert_eq!(new_server_id("GitHub", &v(&["github"])), "github-2");
    assert_eq!(
        new_server_id("GitHub", &v(&["github", "github-2"])),
        "github-3"
    );
    assert_eq!(new_server_id("", &v(&["connector"])), "connector-2");
}

// ── buildCustomMcpDraft / presetDisplayLabel (mcp-presets.test.ts) ─────────

fn model() -> Custom {
    Custom {
        label: "My API".into(),
        url: "https://api.example/mcp".into(),
        ..Default::default()
    }
}

#[test]
fn an_http_model_builds_an_http_bearer_draft_with_stdio_fields_cleared() {
    let d = model().build(&[]);
    assert_eq!(d.transport, McpTransport::Http);
    assert_eq!(d.auth_mode, McpAuthMode::Bearer);
    assert_eq!(d.url.as_deref(), Some("https://api.example/mcp"));
    assert_eq!(d.command, None);
    assert!(d.args.is_empty());
    assert_eq!(d.secret_env_name, None);
    assert!(d.enabled);
    assert_eq!(d.enabled_tools, None);
}

#[test]
fn an_http_oauth_model_keeps_oauth() {
    let m = Custom {
        oauth: true,
        ..model()
    };
    assert_eq!(m.build(&[]).auth_mode, McpAuthMode::OAuth);
}

#[test]
fn a_stdio_model_drops_the_url_and_blank_args_and_env() {
    let m = Custom {
        stdio: true,
        command: " my-server ".into(),
        args: v(&["--flag", "  ", ""]),
        env: vec![("TOKEN".into(), "x".into()), ("  ".into(), String::new())],
        url: "https://ignored".into(),
        ..model()
    };
    let d = m.build(&[]);
    assert_eq!(d.transport, McpTransport::Stdio);
    // `authMode: undefined` in Svelte = serde's default, Bearer.
    assert_eq!(d.auth_mode, McpAuthMode::Bearer);
    assert_eq!(d.command.as_deref(), Some("my-server"));
    assert_eq!(d.args, v(&["--flag"]));
    assert_eq!(
        d.env,
        vec![McpEnvVar {
            name: "TOKEN".into(),
            value: "x".into()
        }]
    );
    assert_eq!(d.url, None);
}

#[test]
fn a_custom_id_slugs_off_the_trimmed_label_avoiding_existing_ids() {
    assert_eq!(model().build(&v(&["my-api"])).id, "my-api-2");
}

#[test]
fn display_labels_take_the_first_free_suffix() {
    assert_eq!(preset_display_label("Linear", &[]), "Linear");
    assert_eq!(
        preset_display_label("Linear", &v(&["Linear"])),
        "Linear (2)"
    );
    assert_eq!(
        preset_display_label("Linear", &v(&["Linear", "Linear (2)"])),
        "Linear (3)"
    );
    assert_eq!(
        preset_display_label("Linear", &v(&["Linear", "Linear (3)"])),
        "Linear (2)"
    );
}

#[test]
fn validation_needs_a_url_for_http_and_a_command_for_stdio() {
    assert!(!model().missing_target());
    let blank = Custom {
        url: "   ".into(),
        ..model()
    };
    assert!(blank.missing_target());
    let stdio = Custom {
        stdio: true,
        ..model()
    };
    assert!(stdio.missing_target(), "stdio ignores the url");
    let ok = Custom {
        command: "npx".into(),
        ..stdio
    };
    assert!(!ok.missing_target());
}

#[test]
fn editing_round_trips_and_keeps_id_enabled_and_curation() {
    let mut d = Custom {
        stdio: true,
        command: "srv".into(),
        args: v(&["-v"]),
        env: vec![("A".into(), "1".into())],
        secret_env: "TOKEN".into(),
        ..model()
    }
    .build(&[]);
    d.enabled = false;
    d.enabled_tools = Some(v(&["a"]));
    let back = Custom::from_server(&d);
    let again = back.to_config(d.id.clone(), d.enabled, d.enabled_tools.clone());
    assert_eq!(again, d);
}

#[test]
fn presets_map_back_by_id_and_transport() {
    let gh = from_preset(&PRESETS[0], &[], false);
    assert_eq!(preset_for(&gh).map(|p| p.id), Some("github"));
    let gh2 = from_preset(&PRESETS[0], std::slice::from_ref(&gh), false);
    assert_eq!(gh2.id, "github-2");
    assert_eq!(gh2.label, "GitHub (2)");
    assert_eq!(preset_for(&gh2).map(|p| p.id), Some("github"));
    // Not a numeric suffix, or the wrong transport: a custom connector.
    let custom = model().to_config("github-enterprise".into(), true, None);
    assert!(preset_for(&custom).is_none());
    let stdio_notion = Custom {
        stdio: true,
        command: "npx".into(),
        ..model()
    }
    .to_config("notion".into(), true, None);
    assert!(preset_for(&stdio_notion).is_none());
}

#[test]
fn hosted_presets_are_https_and_help_links_are_https() {
    for p in &PRESETS {
        assert!(p.url.is_some() != p.command.is_some(), "{}", p.id);
        if let Some(u) = p.url {
            assert!(u.starts_with("https://"), "{}", p.id);
        }
        if let Some(h) = p.help {
            assert!(h.starts_with("https://"), "{}", p.id);
        }
    }
}

// ── mcp-tool-curation.test.ts ──────────────────────────────────────────────

#[test]
fn uncurated_offers_the_first_cap_tools_in_server_order() {
    let names = active_tool_names(&many(40), None);
    assert_eq!(names.len(), DEFAULT_TOOL_CAP);
    assert_eq!(names[0], "t0");
    assert_eq!(
        names[DEFAULT_TOOL_CAP - 1],
        format!("t{}", DEFAULT_TOOL_CAP - 1)
    );
}

#[test]
fn under_the_cap_every_tool_is_active() {
    assert_eq!(active_tool_names(&v(&["a", "b"]), None), v(&["a", "b"]));
}

#[test]
fn curated_offers_only_names_that_still_exist_in_server_order() {
    let list = v(&["c", "a", "gone"]);
    assert_eq!(
        active_tool_names(&v(&["a", "b", "c"]), Some(&list)),
        v(&["a", "c"])
    );
}

#[test]
fn curated_empty_offers_nothing() {
    assert!(active_tool_names(&v(&["a", "b"]), Some(&[])).is_empty());
}

#[test]
fn the_first_toggle_materializes_the_default() {
    let next = toggle_tool(&many(40), None, "t5", false);
    assert!(!next.contains(&"t5".to_string()));
    assert_eq!(next.len(), DEFAULT_TOOL_CAP - 1);
    assert!(next.contains(&"t0".to_string()));
    assert!(!next.contains(&"t32".to_string()));
}

#[test]
fn checking_a_tool_beyond_the_cap_adds_it() {
    let next = toggle_tool(&many(40), None, "t35", true);
    assert!(next.contains(&"t35".to_string()));
    assert_eq!(next.len(), DEFAULT_TOOL_CAP + 1);
}

#[test]
fn toggles_keep_server_order_and_drop_drift() {
    let abc = v(&["a", "b", "c"]);
    assert_eq!(
        toggle_tool(&abc, Some(&v(&["a"])), "c", true),
        v(&["a", "c"])
    );
    assert_eq!(
        toggle_tool(&abc, Some(&v(&["a", "c"])), "a", false),
        v(&["c"])
    );
    assert!(toggle_tool(&v(&["a"]), Some(&v(&["a"])), "a", false).is_empty());
    assert_eq!(
        toggle_tool(&v(&["a", "b"]), Some(&v(&["a", "gone"])), "b", true),
        v(&["a", "b"])
    );
}

#[test]
fn active_counts() {
    assert_eq!(active_tool_count(40, None), DEFAULT_TOOL_CAP);
    assert_eq!(active_tool_count(5, None), 5);
    assert_eq!(active_tool_count(40, Some(&v(&["a", "b"]))), 2);
    assert_eq!(active_tool_count(40, Some(&[])), 0);
}

#[test]
fn row_captions() {
    assert_eq!(tool_caption(Some(40), None), "40 tools \u{b7} 32 active");
    assert_eq!(tool_caption(None, Some(&v(&["a", "b"]))), "2 active");
    assert_eq!(tool_caption(None, None), "see tools");
}

// ── mcp-oauth-stage.test.ts ────────────────────────────────────────────────

#[test]
fn resting_is_idle() {
    assert_eq!(oauth_stage(Some("none"), false, false, false), Stage::Idle);
    assert_eq!(
        oauth_stage(Some("reconnect"), false, false, false),
        Stage::Idle
    );
    assert_eq!(oauth_stage(None, false, false, false), Stage::Idle);
}

#[test]
fn just_clicked_with_begin_in_flight_waits_optimistically() {
    assert_eq!(
        oauth_stage(Some("none"), true, false, false),
        Stage::Authorizing
    );
    assert_eq!(oauth_stage(None, true, false, false), Stage::Authorizing);
}

#[test]
fn live_authorizing_shows_authorizing_even_before_an_attempt() {
    assert_eq!(
        oauth_stage(Some("authorizing"), false, false, false),
        Stage::Authorizing
    );
    assert_eq!(
        oauth_stage(Some("authorizing"), true, false, false),
        Stage::Authorizing
    );
}

#[test]
fn authorized_is_terminal() {
    assert_eq!(
        oauth_stage(Some("authorized"), false, false, false),
        Stage::Authorized
    );
    assert_eq!(
        oauth_stage(Some("authorized"), true, false, true),
        Stage::Authorized
    );
}

#[test]
fn a_begin_error_is_denied() {
    assert_eq!(oauth_stage(Some("none"), true, true, false), Stage::Denied);
}

#[test]
fn falling_back_after_authorizing_is_denied() {
    assert_eq!(oauth_stage(Some("none"), true, false, true), Stage::Denied);
    assert_eq!(
        oauth_stage(Some("reconnect"), true, false, true),
        Stage::Denied
    );
}

#[test]
fn retry_resets_to_idle() {
    assert_eq!(oauth_stage(Some("none"), false, true, false), Stage::Idle);
}

// ── mcp-connector-row.test.ts ──────────────────────────────────────────────

const CONFIGURE: Actions = Actions {
    connect: false,
    reconnect: false,
    disconnect: false,
    configure: true,
};

#[test]
fn bearer_rows() {
    let v = row_view(false, true, true, None);
    assert_eq!(
        (v.badge, v.actions, v.dimmed),
        (Badge::Secret, CONFIGURE, false)
    );
    let v = row_view(false, true, false, None);
    assert_eq!((v.badge, v.actions), (Badge::None, CONFIGURE));
    assert!(row_view(false, false, true, None).dimmed);
    // A stray oauth state never leaks onto a bearer / stdio row.
    let v = row_view(false, true, true, Some("authorized"));
    assert_eq!((v.badge, v.actions), (Badge::Secret, CONFIGURE));
}

#[test]
fn oauth_rows() {
    let v = row_view(true, true, false, Some("none"));
    assert_eq!(v.badge, Badge::NotConnected);
    assert_eq!(
        v.actions,
        Actions {
            connect: true,
            ..Default::default()
        }
    );
    assert!(!v.dimmed);
    assert_eq!(row_view(true, true, false, None).badge, Badge::NotConnected);
    let v = row_view(true, true, false, Some("authorizing"));
    assert_eq!(
        (v.badge, v.actions),
        (Badge::Authorizing, Actions::default())
    );
    let v = row_view(true, true, false, Some("reconnect"));
    assert_eq!(v.badge, Badge::Reconnect);
    assert!(v.actions.reconnect && v.actions.disconnect);
    assert!(!v.actions.connect && !v.actions.configure);
    let v = row_view(true, true, false, Some("authorized"));
    assert_eq!(v.badge, Badge::Authorized);
    assert_eq!(
        v.actions,
        Actions {
            disconnect: true,
            configure: true,
            ..Default::default()
        }
    );
    let v = row_view(true, false, false, Some("authorized"));
    assert_eq!(
        (v.badge, v.actions, v.dimmed),
        (Badge::AuthorizedMuted, CONFIGURE, true)
    );
}

fn preset(id: &str) -> &'static Preset {
    PRESETS.iter().find(|p| p.id == id).unwrap()
}

/// `presetOverrides` applied over `presetToDraft`.
fn overridden(id: &str, adv: Advanced, node_missing: bool) -> McpServerConfig {
    let p = preset(id);
    let mut m = from_preset(p, &[], true);
    apply_overrides(p, &adv, node_missing, &mut m);
    m
}

#[test]
fn preset_overrides_carry_only_what_changed() {
    let base = from_preset(preset("github"), &[], true);
    assert_eq!(
        overridden("github", Advanced::default(), false),
        base,
        "no edits"
    );
    let same_name = Advanced {
        name: "GitHub".into(),
        ..Default::default()
    };
    assert_eq!(overridden("github", same_name, false), base);
    let m = overridden(
        "github",
        Advanced {
            name: "Work GH".into(),
            url: "https://gh.example/mcp".into(),
            ..Default::default()
        },
        true,
    );
    assert_eq!(
        (m.label.as_str(), m.url.as_deref()),
        ("Work GH", Some("https://gh.example/mcp"))
    );
    assert!(m.enabled, "Node is irrelevant to a hosted preset");
}

#[test]
fn a_local_preset_takes_command_and_split_args_and_lands_off_without_node() {
    let fs = preset("filesystem");
    let defaults = Advanced {
        command: "npx".into(),
        args: fs.command.unwrap().1.join(" "),
        ..Default::default()
    };
    assert!(!overridden("filesystem", defaults.clone(), true).enabled);
    let m = overridden(
        "filesystem",
        Advanced {
            command: "node".into(),
            args: "server.js ~/Docs".into(),
            ..defaults.clone()
        },
        false,
    );
    assert_eq!(m.command.as_deref(), Some("node"));
    assert_eq!(m.args, v(&["server.js", "~/Docs"]));
    assert!(overridden(
        "filesystem",
        Advanced {
            args: String::new(),
            ..defaults
        },
        false
    )
    .args
    .is_empty());
}

#[test]
fn catalog_search_and_the_added_badge() {
    assert!(matches(preset("github"), "  PULL "), "tagline, case-blind");
    assert!(!matches(preset("github"), "stripe"));
    assert!(matches(preset("stripe"), ""));
    let gh2 = McpServerConfig {
        id: "github-2".into(),
        ..from_preset(preset("github"), &[], true)
    };
    assert!(added(preset("github"), &[gh2.clone()]));
    let other = McpServerConfig {
        id: "github-x".into(),
        ..gh2
    };
    assert!(!added(preset("github"), &[other]));
    assert!(
        PRESETS
            .iter()
            .all(|p| p.secret.is_some() == p.help.is_some()),
        "secret ⇔ help link"
    );
}
