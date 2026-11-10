//! Layer 2: the shell's keys, clicks and timing constants.

use std::time::Duration;

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    px, size, Bounds, ElementId, Entity, Focusable, Point, TestAppContext, WindowAppearance,
    WindowBounds, WindowHandle, WindowOptions,
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

fn click(cx: &mut TestAppContext, window: WindowHandle<Root>, id: impl Into<ElementId>) {
    let id = id.into();
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

#[gpui_kit::test]
fn the_live_feed_polls_only_while_recording_or_the_status_popover_is_open(cx: &mut TestAppContext) {
    let (_window, shell) = open(cx);
    let live = |cx: &mut TestAppContext| cx.read(|cx| shell.read(cx).live.is_some());
    assert!(!live(cx), "idle and not recording: nothing polls");

    shell.update(cx, |s, cx| s.toggle_overlay(Overlay::Status, cx));
    cx.run_until_parked();
    assert!(
        live(cx),
        "the popover reads health and the backlog even when stopped"
    );
    // Closing through the toggle stops it at once and forgets the health reading.
    shell.update(cx, |s, cx| {
        s.model.status.health = Some(health::Health::default());
        s.toggle_overlay(Overlay::Status, cx)
    });
    assert!(!live(cx));
    assert_eq!(
        cx.read(|cx| shell.read(cx).model.status.health.clone()),
        None
    );

    // Closed another way (the backdrop): the next tick finds nothing wants it.
    shell.update(cx, |s, cx| s.toggle_overlay(Overlay::Status, cx));
    cx.run_until_parked();
    shell.update(cx, |s, cx| {
        s.overlay = None;
        cx.notify();
    });
    cx.executor()
        .advance_clock(capture::LIVE_POLL - Duration::from_millis(1));
    cx.run_until_parked();
    assert!(live(cx));
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert!(!live(cx), "stops on the 2 s tick");
}

/// Settings, then its "Back to app" button: the button held keyboard focus and is
/// gone, the state the real app was in when the titlebar went dead.
fn back_from_settings(cx: &mut TestAppContext, start: &str) -> (WindowHandle<Root>, Entity<Shell>) {
    let (window, shell) = open(cx);
    press(cx, window, start);
    press(cx, window, "cmd-,");
    click(cx, window, "settings-back");
    assert_ne!(cx.read(|cx| shell.read(cx).surface), Surface::Settings);
    (window, shell)
}

#[gpui_kit::test]
fn titlebar_clicks_work_right_after_back_to_app(cx: &mut TestAppContext) {
    for (start, id, want) in [
        ("cmd-1", "titlebar-chats", Surface::Chat),
        ("cmd-1", "titlebar-settings", Surface::Settings),
        ("cmd-1", "nav-insights", Surface::Insights),
        ("cmd-2", "nav-timeline", Surface::Timeline),
    ] {
        let (window, shell) = back_from_settings(cx, start);
        click(cx, window, id);
        assert_eq!(cx.read(|cx| shell.read(cx).surface), want, "{id}");
    }
    let (window, shell) = back_from_settings(cx, "cmd-1");
    let screen = |cx: &mut TestAppContext| cx.read(|cx| shell.read(cx).model.status.selected[0]);
    let before = screen(cx);
    click(cx, window, ("status-lane", 0usize));
    assert_ne!(screen(cx), before, "the screen lane toggles");
}

#[gpui_kit::test]
fn shell_shortcuts_work_right_after_back_to_app(cx: &mut TestAppContext) {
    let (window, shell) = back_from_settings(cx, "cmd-1");
    let surface = |cx: &mut TestAppContext| cx.read(|cx| shell.read(cx).surface);
    press(cx, window, "cmd-k");
    assert!(field_focused(cx, window, &shell), "⌘K");
    press(cx, window, "escape");
    press(cx, window, "cmd-j");
    assert_eq!(
        cx.read(|cx| shell.read(cx).command.read(cx).mode),
        Mode::Ask,
        "⌘J"
    );
    press(cx, window, "escape");
    press(cx, window, "cmd-2");
    assert_eq!(surface(cx), Surface::Insights, "⌘2");
    press(cx, window, "cmd-1");
    assert_eq!(surface(cx), Surface::Timeline, "⌘1");
    press(cx, window, "/");
    assert_eq!(
        cx.read(|cx| shell.read(cx).overlay),
        Some(Overlay::Shortcuts),
        "/"
    );
    press(cx, window, "escape");
    press(cx, window, "cmd-,");
    assert_eq!(surface(cx), Surface::Settings, "⌘,");
}

/// The populated recording bar plus the engine pill and an idle lane: the real
/// app's row that ran out of room at 640 px.
fn full_bar_model() -> ShellModel {
    let mut m = fixture::shell_model(&fixture::load("shell", "populated").unwrap()).unwrap();
    let empty = fixture::shell_model(&fixture::load("shell", "empty").unwrap()).unwrap();
    m.status.engine = empty.status.engine;
    if let Some(rt) = m.status.runtime.as_mut() {
        rt[2].paused = true;
    }
    m
}

/// Which readouts show whole at `width`; panics if one shows only in part.
fn readouts_at(cx: &mut TestAppContext, width: f32) -> Vec<usize> {
    let model = full_bar_model();
    let (window, _) = cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        keymap::bind(&Bindings::default(), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(width), px(480.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Shell::new(model, window, cx))
        })
        .expect("window")
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let help = window.find("status-shortcuts").bounds();
        assert!(help.right() <= px(width), "the ? stays inside at {width}");
        let row = window.find("status-readouts").bounds();
        let mut shown = Vec::new();
        for i in 0..3usize {
            let Some(el) = window.try_find(("status-readout", i)) else {
                break;
            };
            let b = el.bounds();
            if b.top() >= row.bottom() {
                continue; // wrapped onto the clipped line: gone whole
            }
            assert!(
                b.left() >= row.left() && b.right() <= row.right() && b.right() <= help.left(),
                "readout {i} is cut at {width}: {b:?} in {row:?}"
            );
            shown.push(i);
        }
        shown
    })
    .unwrap()
}

#[gpui_kit::test]
fn status_readouts_drop_out_whole_never_cut(cx: &mut TestAppContext) {
    assert_eq!(
        readouts_at(cx, 640.),
        [0usize; 0],
        "640: 'today' doesn't fit"
    );
    // Storage and "local only" go below 900 px.
    assert_eq!(readouts_at(cx, 800.), [0], "800: today only");
    assert_eq!(
        readouts_at(cx, 1280.),
        [0, 1, 2],
        "wide: today, storage, local only"
    );
}
