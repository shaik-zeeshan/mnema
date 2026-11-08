//! `.mx-pop`: raised surface, overlay border, radius 12, popover shadow, and the
//! open motion. The CSS blur(22) is a solid surface here (`POP_SURFACE_ALPHA`).

use gpui_kit::prelude::*;
use gpui_kit::AnimationExt as _;
use gpui_kit::{div, px, BoxShadow, Div, Pixels, Stateful};

use crate::theme::{self, radius, text, Palette};

/// `0 8px 24px rgba(0,0,0,.22)`.
pub fn popover_shadow(p: &Palette) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: p.shadow_popover,
        offset: gpui_kit::point(px(0.), px(8.)),
        blur_radius: px(24.),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// The `.mx-pop` surface. Callers position it (`absolute` + edges) and pass it
/// to [`pop_in`].
pub fn popover(id: &'static str, label: &'static str, p: &Palette) -> Stateful<Div> {
    surface(p)
        .id(id)
        .role(gpui_kit::Role::Dialog)
        .aria_label(label)
        .absolute()
}

/// The popover look without positioning or a11y (menus, lists, static demos).
pub fn surface(p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .bg(p.pop_bg)
        .border_1()
        .border_color(p.overlay_border)
        .rounded(radius::LG)
        .shadow(popover_shadow(p))
        .occlude()
        .font_family(theme::FONT_UI)
        .text_size(text::BASE)
        .font_weight(theme::W_REGULAR)
        .text_color(p.text)
        .whitespace_normal()
}

/// Open motion: fade in (120 ms quart) while rising 4 px into place (220 ms expo).
/// `up` = opens upward from a bottom-anchored trigger. GPUI can't scale a box, so
/// the CSS `scale(.98)` is dropped. Reduce Motion skips straight to the end.
pub fn pop_in(el: Stateful<Div>, edge: Pixels, up: bool) -> impl IntoElement {
    let fade = theme::motion::FAST.as_secs_f32() / theme::motion::MED.as_secs_f32();
    el.with_animation(
        "pop-in",
        gpui_kit::Animation::new(theme::motion::MED).with_easing(theme::motion::expo),
        move |el, t| {
            let lift = px(4.) * (1. - t);
            let el = el.opacity((t / fade).min(1.));
            if up {
                el.bottom(edge - lift)
            } else {
                el.top(edge - lift)
            }
        },
    )
}
