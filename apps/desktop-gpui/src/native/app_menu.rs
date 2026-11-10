//! The macOS menu bar. The Tauri app never built one, so it shipped Tauri's
//! default menu (`tauri::menu::Menu::default`, muda's predefined items); this is
//! that menu with the same titles and keys, plus Settings… and Show All, which
//! the standard app menu has. GPUI reads each item's shortcut from the keymap
//! when the menus are set, so [`crate::keymap::bind`] calls [`refresh`] and the
//! Settings item shows the user's current key.

use gpui_kit::base::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::{actions, App, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType};
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{msg_send, sel};

use crate::keymap::ToggleSettings;

actions!(
    app,
    [
        Quit,
        About,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom,
        ToggleFullScreen,
        CloseWindow,
    ]
);

/// Tauri's menu (macOS branch of `Menu::default`), titles as muda writes them.
pub fn menus() -> Vec<Menu> {
    vec![
        Menu::new("Mnema").items([
            MenuItem::action("About Mnema", About),
            MenuItem::separator(),
            MenuItem::action("Settings…", ToggleSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Mnema", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Mnema", Quit),
        ]),
        Menu::new("File").items([MenuItem::action("Close Window", CloseWindow)]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("View").items([MenuItem::action("Toggle Full Screen", ToggleFullScreen)]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
            MenuItem::separator(),
            MenuItem::action("Close Window", CloseWindow),
        ]),
        // Empty on macOS in Tauri too (its About lives here only off macOS).
        Menu::new("Help"),
    ]
}

/// Set the menus again from the current keymap.
pub fn refresh(cx: &mut App) {
    cx.set_menus(menus());
}

/// Once at startup, after the keymap: the items' keys (muda's accelerators;
/// Undo/Redo/Cut/Copy/Paste/Select All come from gpui-kit's input bindings),
/// their handlers, then the menus. Quit's handler is `main.rs`'s (it owns the core).
pub fn install(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
    ]);
    cx.on_action(|_: &About, cx| send_later(cx, Target::App, sel!(orderFrontStandardAboutPanel:)));
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| send_later(cx, Target::KeyWindow, sel!(performMiniaturize:)));
    cx.on_action(|_: &Zoom, cx| send_later(cx, Target::KeyWindow, sel!(performZoom:)));
    cx.on_action(|_: &ToggleFullScreen, cx| {
        send_later(cx, Target::KeyWindow, sel!(toggleFullScreen:))
    });
    cx.on_action(|_: &CloseWindow, cx| send_later(cx, Target::KeyWindow, sel!(performClose:)));
    // Reached only when no window handled it: the menu (or its ⌘,) while main
    // isn't the key window. Open main on Settings, as the tray's Settings does.
    cx.on_action(|_: &ToggleSettings, cx| crate::window::open_settings(None, cx));
    refresh(cx);
}

#[derive(Clone, Copy)]
enum Target {
    App,
    KeyWindow,
}

/// muda's selectors, sent outside GPUI's update: `performClose:` runs the
/// window's should-close hook and the About panel takes key status, and both
/// re-enter GPUI, which panics if app state is still borrowed.
fn send_later(cx: &mut App, target: Target, selector: Sel) {
    cx.spawn(async move |_| {
        let Some(class) = AnyClass::get(c"NSApplication") else {
            return;
        };
        // SAFETY: main thread (GPUI's foreground executor); NSApp and its key
        // window are live AppKit objects, and each selector takes one `id` sender.
        unsafe {
            let app: *mut AnyObject = msg_send![class, sharedApplication];
            let receiver: *mut AnyObject = match target {
                Target::App => app,
                Target::KeyWindow => msg_send![app, keyWindow],
            };
            if let Some(receiver) = receiver.as_ref() {
                let _: *mut AnyObject = msg_send![receiver, performSelector: selector, withObject: std::ptr::null_mut::<AnyObject>()];
            }
        }
    })
    .detach();
}

#[cfg(test)]
#[path = "app_menu_tests.rs"]
mod tests;
