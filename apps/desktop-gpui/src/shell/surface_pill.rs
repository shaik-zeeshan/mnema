//! Timeline / Insights switch: two 32×26 icon tabs in a pill; the ink slides
//! under the current one (420 ms expo) with the 10×1.5 green tick (`.mx-nav`).

use gpui_kit::prelude::*;
use gpui_kit::AnimationExt as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, Animation, Context, ElementId, Pixels, Role};

use super::model::Surface;
use super::Shell;
use crate::assets::{INSIGHTS_GLYPH, TIMELINE_GLYPH};
use crate::theme::{motion, radius, size, Palette};
use crate::ui::{icon, with_tip};

// The pill's 1px border + 1px padding = the CSS 2px padding with an inset ring.
const PAD: f32 = 1.;

fn slot(surface: Surface) -> Option<f32> {
    match surface {
        Surface::Timeline => Some(0.),
        Surface::Insights => Some(1.),
        _ => None,
    }
}

fn ink_left(slot: f32) -> Pixels {
    // Absolute children start at the pill's padding box, 1px left of the tabs:
    // exactly where the CSS ink's outside 1px ring starts.
    px(slot * f32::from(size::NAV_TAB_W))
}

pub fn render(shell: &Shell, p: &Palette, cx: &mut Context<Shell>) -> impl IntoElement {
    let current = slot(shell.surface);
    let from = slot(shell.ink_from).or(current);
    let ink = current.map(|to| {
        let from = from.unwrap_or(to);
        let el = div()
            .absolute()
            .top_0()
            .h(size::NAV_TAB_H + px(2.))
            .w(size::NAV_TAB_W + px(2.))
            .left(ink_left(to))
            .rounded(radius::SM)
            .bg(p.surface_raised)
            .border_1()
            .border_color(p.border_strong);
        el.with_animation(
            ElementId::Name(format!("nav-ink-{from}-{to}").into()),
            Animation::new(motion::SLOW).with_easing(motion::expo),
            move |el, t| el.left(ink_left(from + (to - from) * t)),
        )
    });

    let tab = |id: &'static str, glyph: &'static str, tip: &'static str, here: bool| {
        let tick = here.then(|| {
            div()
                .absolute()
                .bottom(px(2.))
                .left(px((f32::from(size::NAV_TAB_W) - 10.) / 2.))
                .w(px(10.))
                .h(px(1.5))
                .rounded(px(1.))
                .bg(p.accent)
        });
        let (fg, hover) = if here {
            (p.text_strong, p.text_strong)
        } else {
            (p.text_subtle, p.text)
        };
        with_tip(
            div()
                .id(id)
                .test_support()
                .role(Role::Tab)
                .aria_label(if id == "nav-timeline" {
                    "Timeline"
                } else {
                    "Insights"
                })
                .aria_selected(here)
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .w(size::NAV_TAB_W)
                .h(size::NAV_TAB_H)
                .rounded(radius::SM)
                .text_color(fg)
                .cursor_pointer()
                .hover(move |s| s.text_color(hover))
                .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation()
                })
                .child(icon(glyph, size::ICON))
                .children(tick),
            tip,
        )
    };

    div()
        .id("nav")
        .role(Role::TabList)
        .aria_label("Main surface")
        .relative()
        .flex()
        .p(px(PAD))
        .rounded(radius::MD)
        .bg(p.wash)
        .border_1()
        .border_color(p.hairline)
        .children(ink)
        .child(
            tab(
                "nav-timeline",
                TIMELINE_GLYPH,
                "Timeline  ⌘1",
                shell.surface == Surface::Timeline,
            )
            .on_click(cx.listener(|this, _, _, cx| this.go(Surface::Timeline, cx))),
        )
        .child(
            tab(
                "nav-insights",
                INSIGHTS_GLYPH,
                "Insights  ⌘2",
                shell.surface == Surface::Insights,
            )
            .on_click(cx.listener(|this, _, _, cx| this.go(Surface::Insights, cx))),
        )
}
