//! The menu bar: Tauri's default menu, each item's shortcut, and each item
//! reaching a handler.

use gpui_kit::{Action, App, OperatingSystem, OwnedMenuItem, TestAppContext};
use serde_json::json;

use super::*;
use crate::keymap::{self, Bindings};

/// The shortcut GPUI's macOS menu shows: the first binding whose context is
/// empty, else the first binding (`create_menu_item`'s `find_or_first`).
fn shown_key(action: &dyn Action, cx: &App) -> String {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let all: Vec<_> = keymap.bindings_for_action(action).collect();
    all.iter()
        .find(|b| b.predicate().is_none())
        .or(all.first())
        .map(|b| b.keystrokes()[0].unparse(OperatingSystem::Mac))
        .unwrap_or_default()
}

/// `(menu, [(title, shortcut)])`, separators as `"—"`.
fn snapshot(cx: &App) -> Vec<(String, Vec<(String, String)>)> {
    cx.get_menus()
        .expect("menus")
        .into_iter()
        .map(|menu| {
            let items = menu
                .items
                .iter()
                .map(|item| match item {
                    OwnedMenuItem::Separator => ("—".into(), String::new()),
                    OwnedMenuItem::SystemMenu(m) => (m.name.to_string(), String::new()),
                    OwnedMenuItem::Submenu(m) => (m.name.to_string(), String::new()),
                    OwnedMenuItem::Action { name, action, .. } => {
                        (name.clone(), shown_key(action.as_ref(), cx))
                    }
                })
                .collect();
            (menu.name.to_string(), items)
        })
        .collect()
}

fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        keymap::bind(&Bindings::default(), cx);
        install(cx);
    });
}

fn expected(settings_key: &str) -> Vec<(String, Vec<(String, String)>)> {
    let menu = |name: &str, items: &[(&str, &str)]| {
        (
            name.to_string(),
            items
                .iter()
                .map(|(t, k)| (t.to_string(), k.to_string()))
                .collect::<Vec<_>>(),
        )
    };
    vec![
        menu(
            "Mnema",
            &[
                ("About Mnema", ""),
                ("—", ""),
                ("Settings…", settings_key),
                ("—", ""),
                ("Services", ""),
                ("—", ""),
                ("Hide Mnema", "cmd-h"),
                ("Hide Others", "alt-cmd-h"),
                ("Show All", ""),
                ("—", ""),
                ("Quit Mnema", "cmd-q"),
            ],
        ),
        menu("File", &[("Close Window", "cmd-w")]),
        menu(
            "Edit",
            &[
                ("Undo", "cmd-z"),
                ("Redo", "cmd-shift-z"),
                ("—", ""),
                ("Cut", "cmd-x"),
                ("Copy", "cmd-c"),
                ("Paste", "cmd-v"),
                ("Select All", "cmd-a"),
            ],
        ),
        menu("View", &[("Toggle Full Screen", "ctrl-cmd-f")]),
        menu(
            "Window",
            &[
                ("Minimize", "cmd-m"),
                ("Zoom", ""),
                ("—", ""),
                ("Close Window", "cmd-w"),
            ],
        ),
        menu("Help", &[]),
    ]
}

#[gpui_kit::test]
fn the_menu_bar_is_tauris_default_menu_with_its_keys(cx: &mut TestAppContext) {
    setup(cx);
    cx.update(|cx| assert_eq!(snapshot(cx), expected("cmd-,")));
}

#[gpui_kit::test]
fn a_rebound_settings_key_shows_in_the_menu(cx: &mut TestAppContext) {
    setup(cx);
    cx.update(|cx| {
        // macOS reads each item's key when the menus are set, so a rebind must
        // set them again: clear them here and expect them back.
        cx.set_menus([]);
        keymap::rebind(
            &json!({"appShortcuts": {"openSettings": "CommandOrControl+;"}}),
            cx,
        );
        assert_eq!(snapshot(cx), expected("cmd-;"));
    });
}

#[gpui_kit::test]
fn every_menu_item_reaches_a_handler(cx: &mut TestAppContext) {
    setup(cx);
    // `main.rs` owns Quit's handler (it holds the core).
    cx.update(|cx| {
        cx.on_action(|_: &Quit, _| {});
    });
    cx.update(|cx| {
        for menu in cx.get_menus().expect("menus") {
            for item in &menu.items {
                if let OwnedMenuItem::Action { name, action, .. } = item {
                    // The edit items go to the focused text input (gpui-kit's
                    // `Input` context); everything else has an app-level handler.
                    let input = action.name().starts_with("input::");
                    assert!(
                        input || cx.is_action_available(action.as_ref()),
                        "{} › {name} has no handler",
                        menu.name
                    );
                }
            }
        }
    });
}
