//! Layer 2: the shell's keys, clicks and timing constants.

use std::time::Duration;

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    px, size, Bounds, Entity, Focusable, Point, TestAppContext, WindowAppearance, WindowBounds,
    WindowHandle, WindowOptions,
};

use super::command_field::Mode;
use super::*;
use crate::theme::{Mode as ThemeMode, Palette};

fn open(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<Shell>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        keymap::bind(&Bindings::default(), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1280.), px(800.)),
            })),
            ..Default::default()
        };
        let (window, shell) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Shell::new(ShellModel::default(), window, cx))
        })
        .expect("window");
        (window.downcast::<Root>().expect("root"), shell)
    })
}

fn press(cx: &mut TestAppContext, window: WindowHandle<Root>, keys: &str) {
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press(keys, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn click(cx: &mut TestAppContext, window: WindowHandle<Root>, id: &'static str) {
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn field_focused(
    cx: &mut TestAppContext,
    window: WindowHandle<Root>,
    shell: &Entity<Shell>,
) -> bool {
    cx.update_window(window.into(), |_, window, cx| {
        let field = shell.read(cx).command.clone();
        field.focus_handle(cx).contains_focused(window, cx)
    })
    .unwrap()
}

#[gpui_kit::test]
fn cmd_k_and_cmd_j_focus_the_field_in_their_mode(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    assert!(!field_focused(cx, window, &shell));
    press(cx, window, "cmd-j");
    assert!(field_focused(cx, window, &shell));
    assert_eq!(
        cx.read(|cx| shell.read(cx).command.read(cx).mode),
        Mode::Ask
    );
    press(cx, window, "cmd-k");
    assert_eq!(
        cx.read(|cx| shell.read(cx).command.read(cx).mode),
        Mode::Recall
    );
}

#[gpui_kit::test]
fn tab_flips_the_field_and_esc_returns_to_recall(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let mode = |cx: &mut TestAppContext| cx.read(|cx| shell.read(cx).command.read(cx).mode);
    press(cx, window, "cmd-k");
    press(cx, window, "tab");
    assert_eq!(mode(cx), Mode::Ask);
    press(cx, window, "tab");
    assert_eq!(mode(cx), Mode::Recall);
    press(cx, window, "tab");
    press(cx, window, "escape");
    assert_eq!(mode(cx), Mode::Recall);
    assert!(!field_focused(cx, window, &shell), "Esc blurs the field");
}

#[gpui_kit::test]
fn ask_with_text_opens_chat_with_the_question(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    press(cx, window, "cmd-j");
    cx.update_window(window.into(), |_, window, cx| {
        window.input("what did I read", cx)
    })
    .unwrap();
    press(cx, window, "enter");
    cx.read(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.surface, Surface::Chat);
        assert_eq!(s.chat_query.as_deref(), Some("what did I read"));
    });
}

#[gpui_kit::test]
fn cmd_1_and_cmd_2_switch_surfaces(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let surface = |cx: &mut TestAppContext| cx.read(|cx| shell.read(cx).surface);
    press(cx, window, "cmd-2");
    assert_eq!(surface(cx), Surface::Insights);
    press(cx, window, "cmd-1");
    assert_eq!(surface(cx), Surface::Timeline);
    click(cx, window, "nav-insights");
    assert_eq!(surface(cx), Surface::Insights);
}

#[gpui_kit::test]
fn gear_toggles_settings_and_returns_to_where_it_came_from(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    let surface = |cx: &mut TestAppContext| cx.read(|cx| shell.read(cx).surface);
    press(cx, window, "cmd-2");
    click(cx, window, "titlebar-settings");
    assert_eq!(surface(cx), Surface::Settings);
    press(cx, window, "cmd-,");
    assert_eq!(surface(cx), Surface::Insights, "⌘, toggles back");
    click(cx, window, "titlebar-settings");
    press(cx, window, "escape");
    assert_eq!(surface(cx), Surface::Insights, "Esc is 'Back to app'");
}

#[gpui_kit::test]
fn shortcuts_sheet_and_sources_follow_app_keys_but_not_while_typing(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    press(cx, window, "/");
    assert_eq!(
        cx.read(|cx| shell.read(cx).overlay),
        Some(Overlay::Shortcuts)
    );
    press(cx, window, "escape");
    assert_eq!(cx.read(|cx| shell.read(cx).overlay), None);
    press(cx, window, "1");
    assert!(cx.read(|cx| shell.read(cx).model.status.selected[0]));
    // Typing a digit into the field must not toggle a source.
    press(cx, window, "cmd-k");
    press(cx, window, "1");
    assert!(cx.read(|cx| shell.read(cx).model.status.selected[0]));
    assert_eq!(cx.read(|cx| shell.read(cx).command.read(cx).query(cx)), "1");
}

#[gpui_kit::test]
fn bell_opens_notifications_and_a_second_click_closes(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    click(cx, window, "titlebar-bell");
    assert_eq!(
        cx.read(|cx| shell.read(cx).overlay),
        Some(Overlay::Notifications)
    );
    click(cx, window, "overlay-backdrop");
    assert_eq!(cx.read(|cx| shell.read(cx).overlay), None);
}

#[gpui_kit::test]
fn tooltip_shows_after_350ms_not_349(cx: &mut TestAppContext) {
    let (window, _shell) = open(cx);
    let tooltip_shown = |cx: &mut TestAppContext| {
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.try_find("tooltip").is_some()
        })
        .unwrap()
    };
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.hover("titlebar-settings", cx);
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_millis(349));
    cx.run_until_parked();
    assert!(!tooltip_shown(cx), "no tooltip at 349 ms");
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert!(tooltip_shown(cx), "tooltip at 350 ms");
}

#[gpui_kit::test]
fn theme_switch_repaints_and_cross_fades_in_180ms(cx: &mut TestAppContext) {
    let (_window, _shell) = open(cx);
    cx.update(|cx| {
        theme::set_setting(AppearanceSetting::Light, cx);
        assert_eq!(theme::mode(cx), ThemeMode::Light);
        assert_ne!(theme::tokens(cx), Palette::light(), "mid-fade");
    });
    cx.executor()
        .advance_clock(theme::motion::THEME_FADE - Duration::from_millis(1));
    cx.update(|cx| assert!(theme::fading(cx)));
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!theme::fading(cx));
        assert_eq!(theme::tokens(cx), Palette::light());
    });
}

#[gpui_kit::test]
fn alt_chords_match_the_physical_key_not_the_typed_character(cx: &mut TestAppContext) {
    let (window, shell) = open(cx);
    cx.update(|cx| {
        let settings = serde_json::json!({"appShortcuts": {"toggleShortcutsHelp": "Alt+K"}});
        keymap::bind(&Bindings::from_settings(&settings), cx);
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        // ⌥K types "˚" on a US layout; the binding still matches the K key.
        let mut keystroke = gpui_kit::Keystroke::parse("alt-k").unwrap();
        keystroke.key_char = Some("˚".into());
        window.dispatch_keystroke(keystroke, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| shell.read(cx).overlay),
        Some(Overlay::Shortcuts)
    );
}
