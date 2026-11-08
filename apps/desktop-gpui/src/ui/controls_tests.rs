//! Layer 2 for the shared controls: clicks, keys and every timing constant,
//! each asserted on both sides of its boundary.

use std::time::Duration;

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    div, point, px, size, AnyElement, AppContext as _, Bounds, Context, ElementId, Entity,
    IntoElement, ParentElement, Render, ScrollDelta, Styled, TestAppContext, Window,
    WindowAppearance, WindowBounds, WindowHandle, WindowOptions,
};

use super::segmented::Segmented;
use super::toast::{Toast, Toasts};
use super::{nid, Button};
use crate::theme::{self, motion, Palette, TOAST_TTL};

type Build = Box<dyn Fn(usize, &mut Window, &mut Context<Harness>) -> AnyElement>;

/// One window holding whatever the test builds, plus a counter it can bump.
struct Harness {
    build: Build,
    selected: usize,
}

impl Render for Harness {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p(px(80.))
            .child((self.build)(self.selected, window, cx))
    }
}

fn open(
    cx: &mut TestAppContext,
    build: impl Fn(usize, &mut Window, &mut Context<Harness>) -> AnyElement + 'static,
) -> (WindowHandle<Root>, Entity<Harness>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(800.), px(500.)),
            })),
            ..Default::default()
        };
        let (window, view) = gpui_kit::open_window(options, cx, |_, cx| {
            cx.new(|_| Harness {
                build: Box::new(build),
                selected: 0,
            })
        })
        .expect("window");
        (window.downcast::<Root>().expect("root"), view)
    })
}

fn act(
    cx: &mut TestAppContext,
    window: WindowHandle<Root>,
    f: impl FnOnce(&mut Window, &mut gpui_kit::App),
) {
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        f(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn present(cx: &mut TestAppContext, window: WindowHandle<Root>, id: impl Into<ElementId>) -> bool {
    let id = id.into();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id.clone()).is_some()
    })
    .unwrap()
}

fn bounds(
    cx: &mut TestAppContext,
    window: WindowHandle<Root>,
    id: impl Into<ElementId>,
) -> Bounds<gpui_kit::Pixels> {
    let id = id.into();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find(id.clone()).bounds()
    })
    .unwrap()
}

fn advance(cx: &mut TestAppContext, d: Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

fn tipped(_: usize, _: &mut Window, _: &mut Context<Harness>) -> AnyElement {
    div()
        .flex()
        .gap(px(20.))
        .child(Button::new("first", "First"))
        .child(Button::new("tipped", "Export").tip("Export the week"))
        .into_any_element()
}

#[gpui_kit::test]
fn tooltip_dwells_350ms_not_349(cx: &mut TestAppContext) {
    let (window, _) = open(cx, tipped);
    act(cx, window, |w, cx| w.hover("tipped", cx));
    advance(cx, motion::TOOLTIP_DWELL - Duration::from_millis(1));
    assert!(!present(cx, window, "tooltip"), "no tooltip at 349 ms");
    advance(cx, Duration::from_millis(1));
    assert!(present(cx, window, "tooltip"), "tooltip at 350 ms");
}

#[gpui_kit::test]
fn tooltip_is_instant_on_keyboard_focus(cx: &mut TestAppContext) {
    let (window, _) = open(cx, tipped);
    let focused = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.find(id).focused()
        })
        .unwrap()
    };
    // A click focuses the first button (pointer: no ring, no tip); Tab then
    // moves focus by keyboard. No clock moves, so the tip is there at once.
    act(cx, window, |w, cx| w.click("first", cx));
    assert_eq!(focused(cx, "first"), Some(true));
    assert!(!present(cx, window, "tooltip"));
    act(cx, window, |w, cx| w.press("tab", cx));
    assert_eq!(
        focused(cx, "tipped"),
        Some(true),
        "Tab moves to the tipped button"
    );
    assert!(present(cx, window, "tooltip"), "tip shows with no dwell");
}

#[gpui_kit::test]
fn tooltip_hides_on_scroll(cx: &mut TestAppContext) {
    let (window, _) = open(cx, tipped);
    act(cx, window, |w, cx| w.hover("tipped", cx));
    advance(cx, motion::TOOLTIP_DWELL);
    assert!(present(cx, window, "tooltip"));
    act(cx, window, |w, cx| {
        w.scroll("tipped", ScrollDelta::Pixels(point(px(0.), px(-24.))), cx)
    });
    assert!(!present(cx, window, "tooltip"), "scroll hides the tip");
}

#[gpui_kit::test]
fn segmented_thumb_slides_420ms_expo(cx: &mut TestAppContext) {
    let (window, view) = open(cx, |selected, _, cx| {
        let this = cx.weak_entity();
        Segmented::new("seg", "Range", ["Day", "Week", "Month"], selected)
            .on_change(move |ix, _, cx| {
                this.update(cx, |h, cx| {
                    h.selected = ix;
                    cx.notify();
                })
                .ok();
            })
            .into_any_element()
    });
    let start = bounds(cx, window, "seg-thumb");
    let target = bounds(cx, window, nid("seg-opt", 2));
    act(cx, window, |w, cx| w.click(nid("seg-opt", 2), cx));
    assert_eq!(cx.read(|cx| view.read(cx).selected), 2);
    let x = |cx: &mut TestAppContext| f32::from(bounds(cx, window, "seg-thumb").origin.x);
    let (x0, x1) = (f32::from(start.origin.x), f32::from(target.origin.x));
    // The click frame starts the clock; mid-way the expo curve is well past linear.
    advance(cx, motion::SLOW / 2);
    let mid = x(cx);
    let want = x0 + (x1 - x0) * motion::expo(0.5);
    assert!((mid - want).abs() < 1.5, "mid {mid} vs expo {want}");
    assert!(mid > x0 + (x1 - x0) * 0.5, "ease-out leads linear");
    // expo's tail is sub-pixel past ~400 ms, so the far side of the 420 ms
    // boundary is checked as "landed exactly", the near side as "mid-way".
    advance(cx, motion::SLOW / 2);
    assert!((x(cx) - x1).abs() < 0.5, "lands on the option at 420 ms");
    let end = bounds(cx, window, "seg-thumb");
    assert!((f32::from(end.size.width) - f32::from(target.size.width)).abs() < 0.5);
}

#[gpui_kit::test]
fn toast_rises_8px_in_220ms_and_drains_over_its_ttl(cx: &mut TestAppContext) {
    let toasts = cx.new(|_| Toasts::default());
    let held = toasts.clone();
    let (window, _) = open(cx, move |_, _, _| {
        div()
            .relative()
            .size_full()
            .child(held.clone())
            .into_any_element()
    });
    toasts.update(cx, |t, cx| {
        t.push(
            Toast::ok("Deleted 3 moments").action("Undo", true, |_, _| {}),
            cx,
        );
    });
    cx.run_until_parked();
    let y = |cx: &mut TestAppContext| f32::from(bounds(cx, window, nid("toast", 0)).origin.y);
    let y0 = y(cx);
    advance(cx, motion::MED - Duration::from_millis(1));
    let y_late = y(cx);
    assert!(y_late > y0 - 8.5 && y_late < y0, "still rising at 219 ms");
    advance(cx, Duration::from_millis(1));
    let rest = y(cx);
    assert!((y0 - rest - 8.).abs() < 0.5, "rose 8 px: {y0} -> {rest}");

    let full = f32::from(bounds(cx, window, nid("toast", 0)).size.width);
    advance(cx, TOAST_TTL / 2 - motion::MED);
    let half = f32::from(bounds(cx, window, nid("toast-drain", 0)).size.width);
    assert!(
        (half / full - 0.5).abs() < 0.02,
        "half drained at ttl/2: {half}/{full}"
    );
    advance(cx, TOAST_TTL / 2 - Duration::from_millis(1));
    assert!(
        present(cx, window, nid("toast", 0)),
        "still up at ttl - 1 ms"
    );
    assert_eq!(cx.read(|cx| toasts.read(cx).len()), 1);
    advance(cx, Duration::from_millis(1));
    assert!(cx.read(|cx| toasts.read(cx).is_empty()), "gone at the ttl");
}

#[test]
fn disabled_and_busy_opacity_are_theme_independent() {
    assert_eq!(theme::DISABLED_OPACITY, 0.4);
    assert_eq!(theme::BUSY_OPACITY, 0.6);
    // Neither comes from a palette, so both modes (and the cross-fade) share them.
    assert_ne!(Palette::dark(), Palette::light());
}

#[gpui_kit::test]
fn disabled_buttons_swallow_clicks_and_busy_ones_too(cx: &mut TestAppContext) {
    let (window, view) = open(cx, |_, _, cx| {
        let bump = |cx: &mut Context<Harness>| {
            let this = cx.weak_entity();
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                this.update(cx, |h, _| h.selected += 1).ok();
            }
        };
        div()
            .flex()
            .gap(px(10.))
            .child(Button::new("live", "Live").on_click(bump(cx)))
            .child(Button::new("off", "Off").disabled(true).on_click(bump(cx)))
            .child(Button::new("busy", "Busy").busy(true).on_click(bump(cx)))
            .into_any_element()
    });
    for id in ["off", "busy", "live"] {
        act(cx, window, |w, cx| w.click(id, cx));
    }
    assert_eq!(
        cx.read(|cx| view.read(cx).selected),
        1,
        "only the live one fired"
    );
}
