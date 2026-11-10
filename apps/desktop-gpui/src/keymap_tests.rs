use serde_json::json;

use super::*;

#[test]
fn settings_strings_become_gpui_keystrokes_and_labels() {
    assert_eq!(to_gpui("CommandOrControl+Alt+R"), "cmd-alt-r");
    assert_eq!(to_gpui("CommandOrControl+,"), "cmd-,");
    assert_eq!(to_gpui("Shift+ArrowLeft"), "shift-left");
    assert_eq!(to_gpui("CommandOrControl++"), "cmd-+");
    assert_eq!(display("CommandOrControl+Alt+Space"), "⌘⌥Space");
    assert_eq!(display("CommandOrControl+,"), "⌘,");
    assert_eq!(display("/"), "/");
    assert_eq!(to_gpui("1"), "1");
}

#[test]
fn reserved_and_missing_entries_keep_defaults() {
    let settings = json!({
        "globalShortcuts": {"bindings": {"toggleRecording": "CommandOrControl+Alt+X"}},
        "appShortcuts": {"toggleSourceScreen": "Escape", "toggleShortcutsHelp": "Shift+Tab"},
    });
    let b = Bindings::from_settings(&settings);
    assert_eq!(b.toggle_recording, "CommandOrControl+Alt+X");
    assert_eq!(b.sources[0], "1", "Escape is reserved");
    assert_eq!(b.toggle_shortcuts_help, "/", "Shift+Tab is reserved");
    assert!(is_reserved("shift+ArrowRight") && !is_reserved("ArrowUp"));
}

#[test]
fn help_sheet_is_built_from_state() {
    let b = Bindings::default();
    let stopped = HelpState {
        can_toggle_recording: true,
        can_toggle_sources: true,
        ..Default::default()
    };
    let groups = help_groups(&b, stopped, vec![]);
    let global: Vec<_> = groups[0].rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(global[0], "Start recording");
    assert!(global.contains(&"Toggle screen for the next recording"));
    assert!(!global.contains(&"Pause recording") && !global.contains(&"Open debug"));
    assert_eq!(groups[1].title, "Here");

    let recording = HelpState {
        capturing: true,
        user_paused: true,
        can_toggle_recording: true,
        developer_options: true,
        ..Default::default()
    };
    let extra = HelpGroup {
        title: "Timeline",
        rows: vec![],
    };
    let groups = help_groups(&b, recording, vec![extra]);
    let global: Vec<_> = groups[0].rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(&global[..2], ["Stop recording", "Resume recording"]);
    assert!(global.contains(&"Open debug"));
    assert_eq!(groups.len(), 2, "empty surface groups are dropped");
}

// ── Rebinding takes effect at once (the old key stops, gpui-kit's stay) ──

mod rebind {
    use app_core::keyboard_bindings::KeyboardBindingsSettings;
    use gpui_kit::{
        div, prelude::*, Context, Entity, FocusHandle, TestAppContext, VisualTestContext, Window,
    };
    use serde_json::Value;

    use super::super::*;
    use crate::timeline::audio::{PlayPause, SeekBack};

    actions!(keymap_test, [Quit]);

    /// The shell's key context with the drawer's nested inside; records what fires.
    struct Probe {
        shell: FocusHandle,
        drawer: FocusHandle,
        fired: Vec<&'static str>,
    }

    impl Render for Probe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let rec = |name: &'static str| move |p: &mut Probe| p.fired.push(name);
            div()
                .size_full()
                .key_context(SHELL_CONTEXT)
                .track_focus(&self.shell)
                .on_action(cx.listener({
                    let f = rec("settings");
                    move |p, _: &ToggleSettings, _, _| f(p)
                }))
                .on_action(cx.listener({
                    let f = rec("debug");
                    move |p, _: &OpenDebug, _, _| f(p)
                }))
                .on_action(cx.listener({
                    let f = rec("screen");
                    move |p, _: &ToggleSourceScreen, _, _| f(p)
                }))
                .on_action(cx.listener({
                    let f = rec("play");
                    move |p, _: &PlayPause, _, _| f(p)
                }))
                .on_action(cx.listener({
                    let f = rec("back");
                    move |p, _: &SeekBack, _, _| f(p)
                }))
                .child(
                    div()
                        .size_full()
                        .key_context("AudioDrawer")
                        .track_focus(&self.drawer),
                )
        }
    }

    fn open(cx: &mut TestAppContext) -> (Entity<Probe>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            bind(&Bindings::default(), cx);
        });
        let (probe, cx) = cx.add_window_view(|_, cx| Probe {
            shell: cx.focus_handle(),
            drawer: cx.focus_handle(),
            fired: Vec::new(),
        });
        focus(&probe, cx, false);
        (probe, cx)
    }

    fn focus(probe: &Entity<Probe>, cx: &mut VisualTestContext, drawer: bool) {
        cx.update(|window, cx| {
            let p = probe.read(cx);
            let h = if drawer { &p.drawer } else { &p.shell };
            h.clone().focus(window, cx);
        });
        cx.run_until_parked();
    }

    /// What one press fired.
    fn fired(probe: &Entity<Probe>, cx: &mut VisualTestContext, keys: &str) -> Vec<&'static str> {
        cx.simulate_keystrokes(keys);
        cx.run_until_parked();
        probe.update(cx, |p, _| std::mem::take(&mut p.fired))
    }

    fn defaults() -> Value {
        serde_json::to_value(KeyboardBindingsSettings::defaults()).unwrap()
    }

    /// The live save path: app-core validates (`CoreBackend::save_keyboard`), and
    /// only a valid save emits the event that rebinds (`bridge::events`).
    fn save(cx: &mut VisualTestContext, kb: &mut Value, edit: impl FnOnce(&mut Value)) -> bool {
        let mut next = kb.clone();
        edit(&mut next);
        let request: KeyboardBindingsSettings = serde_json::from_value(next).unwrap();
        let Ok(valid) = request.validated_for_update() else {
            return false;
        };
        *kb = serde_json::to_value(valid).unwrap();
        let payload = kb.clone();
        cx.update(|_, cx| rebind(&payload, cx));
        cx.run_until_parked();
        true
    }

    #[gpui_kit::test]
    fn a_rebind_moves_the_action_and_the_old_key_stops(cx: &mut TestAppContext) {
        let (probe, cx) = open(cx);
        let mut kb = defaults();
        assert_eq!(fired(&probe, cx, "cmd-,"), ["settings"]);
        assert!(save(cx, &mut kb, |k| {
            k["appShortcuts"]["openSettings"] = "CommandOrControl+Shift+O".into()
        }));
        assert!(fired(&probe, cx, "cmd-,").is_empty(), "the old key is gone");
        assert_eq!(fired(&probe, cx, "cmd-shift-o"), ["settings"]);
    }

    #[gpui_kit::test]
    fn a_cleared_binding_fires_nothing(cx: &mut TestAppContext) {
        let (probe, cx) = open(cx);
        let mut kb = defaults();
        assert!(save(cx, &mut kb, |k| k["appShortcuts"]["openSettings"] =
            "".into()));
        assert!(fired(&probe, cx, "cmd-,").is_empty());
        assert_eq!(fired(&probe, cx, "cmd-d"), ["debug"], "the rest stay");
    }

    #[gpui_kit::test]
    fn reset_to_defaults_leaves_only_the_defaults(cx: &mut TestAppContext) {
        let (probe, cx) = open(cx);
        let mut kb = defaults();
        for (group, key, value) in [
            ("appShortcuts", "openSettings", "CommandOrControl+Shift+O"),
            ("appShortcuts", "openDebug", "CommandOrControl+Shift+D"),
            ("appShortcuts", "toggleSourceScreen", "9"),
            ("audioDrawerShortcuts", "playPause", "K"),
        ] {
            assert!(save(cx, &mut kb, |k| k[group][key] = value.into()));
        }
        assert!(save(cx, &mut kb, |k| *k = defaults()));
        for old in ["cmd-shift-o", "cmd-shift-d", "9"] {
            assert!(fired(&probe, cx, old).is_empty(), "{old} still fires");
        }
        assert_eq!(fired(&probe, cx, "cmd-,"), ["settings"]);
        assert_eq!(fired(&probe, cx, "cmd-d"), ["debug"]);
        assert_eq!(fired(&probe, cx, "1"), ["screen"]);
        focus(&probe, cx, true);
        assert!(fired(&probe, cx, "k").is_empty());
        assert_eq!(fired(&probe, cx, "space"), ["play"]);
    }

    #[gpui_kit::test]
    fn swapping_two_keys_never_fires_both_or_the_wrong_one(cx: &mut TestAppContext) {
        let (probe, cx) = open(cx);
        let mut kb = defaults();
        let expect = |cx: &mut VisualTestContext, comma: &[&str], d: &[&str], x: &[&str]| {
            assert_eq!(fired(&probe, cx, "cmd-,"), comma);
            assert_eq!(fired(&probe, cx, "cmd-d"), d);
            assert_eq!(fired(&probe, cx, "cmd-x"), x);
        };
        // Taking the other action's key in one step is a conflict: refused, nothing moves.
        assert!(!save(cx, &mut kb, |k| {
            k["appShortcuts"]["openSettings"] = "CommandOrControl+D".into()
        }));
        expect(cx, &["settings"], &["debug"], &[]);
        // So a swap goes through a free key; each step moves exactly one action.
        assert!(save(cx, &mut kb, |k| {
            k["appShortcuts"]["openSettings"] = "CommandOrControl+X".into()
        }));
        expect(cx, &[], &["debug"], &["settings"]);
        assert!(save(cx, &mut kb, |k| {
            k["appShortcuts"]["openDebug"] = "CommandOrControl+,".into()
        }));
        expect(cx, &["debug"], &[], &["settings"]);
        assert!(save(cx, &mut kb, |k| {
            k["appShortcuts"]["openSettings"] = "CommandOrControl+D".into()
        }));
        expect(cx, &["debug"], &["settings"], &[]);
    }

    #[gpui_kit::test]
    fn a_reserved_or_invalid_key_keeps_the_previous_binding(cx: &mut TestAppContext) {
        let (probe, cx) = open(cx);
        let mut kb = defaults();
        assert!(save(cx, &mut kb, |k| {
            k["appShortcuts"]["openSettings"] = "CommandOrControl+Shift+O".into();
            k["appShortcuts"]["toggleSourceScreen"] = "9".into();
        }));
        // app-core refuses these saves (RESERVED_SHORTCUT_BINDINGS, the parser), so
        // no event comes and the current keys stay.
        for (key, bad) in [
            ("toggleSourceScreen", "Tab"),
            ("toggleSourceScreen", "ArrowLeft"),
            ("openSettings", "Escape"),
            ("openSettings", "Alt+Foo+Bar"),
        ] {
            assert!(!save(cx, &mut kb, |k| k["appShortcuts"][key] = bad.into()));
        }
        // The keymap's own guard agrees: a payload carrying them keeps the current
        // key, not the default.
        let mut payload = kb.clone();
        payload["appShortcuts"]["openSettings"] = "Escape".into();
        payload["appShortcuts"]["toggleSourceScreen"] = "Alt+Foo+Bar".into();
        payload["audioDrawerShortcuts"]["playPause"] = "Tab".into();
        cx.update(|_, cx| rebind(&payload, cx));
        cx.run_until_parked();
        assert_eq!(fired(&probe, cx, "cmd-shift-o"), ["settings"]);
        assert!(fired(&probe, cx, "cmd-,").is_empty());
        assert_eq!(fired(&probe, cx, "9"), ["screen"]);
        assert!(fired(&probe, cx, "1").is_empty());
        focus(&probe, cx, true);
        assert_eq!(fired(&probe, cx, "space"), ["play"]);
    }

    #[gpui_kit::test]
    fn audio_drawer_keys_rebind_the_same_way(cx: &mut TestAppContext) {
        let (probe, cx) = open(cx);
        let mut kb = defaults();
        focus(&probe, cx, true);
        assert_eq!(fired(&probe, cx, "left"), ["back"]);
        assert!(save(cx, &mut kb, |k| {
            k["audioDrawerShortcuts"]["playPause"] = "K".into();
            k["audioDrawerShortcuts"]["seekBack"] = "".into();
        }));
        assert!(fired(&probe, cx, "space").is_empty());
        assert_eq!(fired(&probe, cx, "k"), ["play"]);
        assert!(fired(&probe, cx, "left").is_empty(), "cleared");
        // Outside the drawer its keys don't apply.
        focus(&probe, cx, false);
        assert!(fired(&probe, cx, "k").is_empty());
    }

    #[gpui_kit::test]
    fn rebinding_keeps_every_other_binding_and_never_grows(cx: &mut TestAppContext) {
        let (_probe, cx) = open(cx);
        // Bound after ours, as main.rs binds ⌘Q.
        cx.update(|_, cx| cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]));
        let snapshot = |cx: &mut VisualTestContext| {
            cx.update(|_, cx| {
                let keymap = cx.key_bindings();
                let keymap = keymap.borrow();
                let foreign: Vec<String> = keymap
                    .bindings()
                    .filter(|b| b.meta() != Some(OURS))
                    .map(|b| b.action().name().to_string())
                    .collect();
                let len = keymap.bindings().len();
                (foreign, len)
            })
        };
        let (before, len) = snapshot(cx);
        assert!(
            before.iter().any(|n| n.starts_with("input::")),
            "kit bindings"
        );
        assert!(before.iter().any(|n| n == "keymap_test::Quit"));
        let mut kb = defaults();
        for key in ["CommandOrControl+Shift+O", "CommandOrControl+Shift+P"] {
            assert!(save(cx, &mut kb, |k| k["appShortcuts"]["openSettings"] =
                key.into()));
        }
        assert!(save(cx, &mut kb, |k| *k = defaults()));
        assert_eq!(snapshot(cx), (before, len));
    }
}
