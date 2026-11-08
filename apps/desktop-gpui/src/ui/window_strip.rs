//! `WindowStrip.svelte`: the small undecorated windows' 38 px strip. Three drawn
//! lights (grey; red / amber / green while the strip is hovered), the title
//! centred, optional right-aligned actions; the whole strip drags the window.
//! The first light IS the close button.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, App, SharedString, Window};

use super::{focus_handle, focus_visible, ring_box, Force};
use crate::theme::{self, size, text};

type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct WindowStrip {
    title: SharedString,
    actions: Vec<AnyElement>,
    on_close: Option<OnClose>,
    force: Force,
}

impl WindowStrip {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            actions: Vec::new(),
            on_close: None,
            force: Force::None,
        }
    }
    pub fn action(mut self, el: impl IntoElement) -> Self {
        self.actions.push(el.into_any_element());
        self
    }
    /// Instead of closing the window (the default).
    pub fn on_close(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(f));
        self
    }
    /// `Hover` lights the three dots; `Focus` rings the close light.
    pub fn force(mut self, f: Force) -> Self {
        self.force = f;
        self
    }
}

const GROUP: &str = "window-strip";

impl RenderOnce for WindowStrip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let handle = focus_handle("strip-close", window, cx);
        let ring_on = focus_visible(&handle, self.force, window);
        let lit = self.force == Force::Hover;
        let colors = [p.status_running_dot, p.status_paused_dot, p.source_mic];
        let light = |i: usize| {
            let c = colors[i];
            div()
                .relative()
                .flex_none()
                .size(size::STRIP_LIGHT)
                .rounded_full()
                .bg(if lit || (i == 0 && ring_on) {
                    c
                } else {
                    p.border_strong
                })
                .group_hover(GROUP, move |s| s.bg(c))
        };
        let close = self.on_close.clone();
        div()
            .id("window-strip")
            .group(GROUP)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .h(size::TITLEBAR)
            .border_b_1()
            .border_color(p.hairline)
            .on_mouse_down(gpui_kit::MouseButton::Left, |_, window, _| {
                window.start_window_move()
            })
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_family(theme::FONT_UI)
                    .font_weight(theme::W_MEDIUM)
                    .text_size(text::BASE)
                    .line_height(gpui_kit::relative(1.))
                    .text_color(p.text_subtle)
                    .child(self.title),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .gap(px(8.))
                    .w(size::TRAFFIC_LIGHTS)
                    .pl(px(13.))
                    .child(
                        light(0)
                            .id("strip-close")
                            .test_support()
                            .role(gpui_kit::Role::Button)
                            .aria_label("Close window")
                            .track_focus(&handle)
                            .when(ring_on, |el| {
                                el.child(ring_box(
                                    p.accent_glow,
                                    size::FOCUS_RING,
                                    theme::radius::PILL,
                                ))
                            })
                            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation()
                            })
                            .on_click(move |_, window, cx| match &close {
                                Some(f) => f(window, cx),
                                None => window.remove_window(),
                            }),
                    )
                    .child(light(1))
                    .child(light(2)),
            )
            .when(!self.actions.is_empty(), |el| {
                el.child(
                    div()
                        .relative()
                        .ml_auto()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation()
                        })
                        .children(self.actions),
                )
            })
    }
}
