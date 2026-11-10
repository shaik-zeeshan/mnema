//! `ui::link`: the scheme gate, and the control's click / keys / role.

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    div, point, px, size, AppContext as _, Bounds, Context, IntoElement, ParentElement, Render,
    Styled, TestAppContext, Window, WindowAppearance, WindowBounds, WindowHandle, WindowOptions,
};

use super::{openable, Link};
use crate::theme;

#[test]
fn only_web_and_mail_urls_are_openable() {
    for ok in [
        "https://github.com/k2-fsa/sherpa-onnx",
        "HTTP://example.com",
        "mailto:hello@mnema.day",
    ] {
        assert!(openable(ok), "{ok}");
    }
    for bad in [
        "file:///etc/passwd",
        "javascript:alert(1)",
        "/etc/passwd",
        "https://",
        "https:///x",
        "https:example.com",
        "mailto:",
        "x-apple.systempreferences:com.apple.preference",
        "",
    ] {
        assert!(!openable(bad), "{bad}");
    }
}

struct Two;

impl Render for Two {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p(px(40.))
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(Link::new("ok", "Docs", "https://mnema.day/docs"))
            .child(Link::new("bad", "Passwords", "file:///etc/passwd").quiet())
    }
}

fn open(cx: &mut TestAppContext) -> WindowHandle<Root> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(400.), px(300.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Two))
            .expect("window")
            .0
            .downcast::<Root>()
            .expect("root")
    })
}

fn act(
    cx: &mut TestAppContext,
    w: WindowHandle<Root>,
    f: impl FnOnce(&mut Window, &mut gpui_kit::App),
) {
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        f(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_click_opens_the_url_and_the_link_is_a_named_link(cx: &mut TestAppContext) {
    let w = open(cx);
    act(cx, w, |window, _| {
        let el = window.find("ok");
        assert_eq!(el.role(), Some(gpui_kit::Role::Link));
    });
    act(cx, w, |window, cx| window.click("ok", cx));
    assert_eq!(cx.opened_url().as_deref(), Some("https://mnema.day/docs"));
}

fn keyboard_opens(key: &str, cx: &mut TestAppContext) {
    let w = open(cx);
    // A click on the refused link focuses it (and opens nothing); Shift-Tab
    // then moves focus back to the first link by keyboard.
    act(cx, w, |window, cx| window.click("bad", cx));
    act(cx, w, |window, cx| window.press("shift-tab", cx));
    act(cx, w, |window, _| {
        assert_eq!(
            window.find("ok").focused(),
            Some(true),
            "Shift-Tab reaches the link"
        )
    });
    assert_eq!(cx.opened_url(), None);
    act(cx, w, |window, cx| window.press(key, cx));
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://mnema.day/docs"),
        "{key}"
    );
}

#[gpui_kit::test]
fn keyboard_focus_then_enter_opens_it(cx: &mut TestAppContext) {
    keyboard_opens("enter", cx);
}

#[gpui_kit::test]
fn keyboard_focus_then_space_opens_it(cx: &mut TestAppContext) {
    keyboard_opens("space", cx);
}

#[gpui_kit::test]
fn a_disallowed_scheme_is_ignored(cx: &mut TestAppContext) {
    let w = open(cx);
    act(cx, w, |window, _| {
        assert_eq!(window.find("bad").role(), Some(gpui_kit::Role::Link))
    });
    act(cx, w, |window, cx| window.click("bad", cx));
    assert_eq!(cx.opened_url(), None, "file: never reaches the OS");
}
