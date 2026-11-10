//! `ui::scroll::bar`: it adds nothing to the area's scroll range, and its thumb
//! drags. (That the thumb shows only on overflow, and where, is checked on real
//! pixels by the `scroll-bar` check in `tests/visual/scroll.rs`.)

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::{
    div, point, px, size, AppContext as _, Bounds, Context, InteractiveElement, IntoElement,
    Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Point, Render, ScrollHandle, StatefulInteractiveElement, Styled, TestAppContext, Window,
    WindowAppearance, WindowBounds, WindowHandle, WindowOptions,
};

use gpui_kit::test::TestWindowExt;

use super::bar;
use crate::theme;

/// A 200 px tall area with 8 px padding over `rows` 40 px rows.
struct Area {
    handle: ScrollHandle,
    rows: usize,
}

impl Render for Area {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            div()
                .id("area")
                .w(px(200.))
                .h(px(200.))
                .p(px(8.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .overflow_y_scroll()
                .track_scroll(&self.handle)
                .children((0..self.rows).map(|_| div().flex_none().h(px(40.))))
                .child(bar(&self.handle)),
        )
    }
}

fn open(cx: &mut TestAppContext, rows: usize) -> (WindowHandle<Root>, ScrollHandle) {
    let handle = ScrollHandle::new();
    let h = handle.clone();
    let w = cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        // Snaps the bar's fade, so it shows on the frame that asks for it.
        cx.set_reduce_motion(true);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(400.), px(400.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Area { handle: h, rows }))
            .expect("window")
            .0
            .downcast::<Root>()
            .expect("root")
    });
    frame(cx, w);
    (w, handle)
}

fn frame(cx: &mut TestAppContext, w: WindowHandle<Root>) {
    cx.update_window(w.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

fn send(cx: &mut TestAppContext, w: WindowHandle<Root>, e: impl gpui_kit::InputEvent) {
    cx.update_window(w.into(), |_, window, cx| {
        window.dispatch_event(e.to_platform_input(), cx);
    })
    .unwrap();
    frame(cx, w);
}

fn moved(at: Point<Pixels>, pressed: Option<MouseButton>) -> MouseMoveEvent {
    MouseMoveEvent {
        position: at,
        modifiers: Modifiers::default(),
        pressed_button: pressed,
    }
}

#[gpui_kit::test]
fn the_bar_adds_nothing_to_the_scroll_range(cx: &mut TestAppContext) {
    // 4 rows + gaps + padding = 188 px: fits, so nothing to scroll.
    let (_, short) = open(cx, 4);
    assert_eq!(short.max_offset().y, px(0.));
    // 10 rows: 400 + 36 gaps + 16 padding = 452, so 252 past a 200 px area.
    let (_, long) = open(cx, 10);
    assert_eq!(long.max_offset().y, px(252.));
}

#[gpui_kit::test]
fn dragging_the_thumb_scrolls(cx: &mut TestAppContext) {
    let (w, handle) = open(cx, 10);
    let b = handle.bounds();
    // The thumb is 200 / 452 of the track (88.5 px) and starts at the top.
    let x = b.right() - px(6.);
    let grab = point(x, b.top() + px(40.));
    // The first move over the area attaches the bar; on the edge, the next
    // reveals it (hover mode); then press and drag.
    send(cx, w, moved(grab, None));
    send(cx, w, moved(grab, None));
    send(
        cx,
        w,
        MouseDownEvent {
            position: grab,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        },
    );
    assert_eq!(
        handle.offset().y,
        px(0.),
        "pressing the thumb keeps the place"
    );
    let to = point(x, grab.y + px(40.));
    send(cx, w, moved(to, Some(MouseButton::Left)));
    send(
        cx,
        w,
        MouseUpEvent {
            position: to,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
        },
    );
    // 40 px of thumb travel over (200 - 88.5) px of track = 252 * 40 / 111.5.
    let y = -handle.offset().y;
    let want = 252. * 40. / (200. - 200. * 200. / 452.);
    assert!(
        (f32::from(y) - want).abs() < 1.,
        "scrolled {y:?}, want ~{want}"
    );
}
