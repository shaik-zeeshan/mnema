//! Shared bits the shell needs now. S6 grows this into the full control set
//! (one file per control); keep each helper token-only.

pub mod clock;

use gpui_kit::prelude::*;
use gpui_kit::AnimationExt as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, px, svg, AnyView, App, BoxShadow, Context, Div, SharedString, Stateful, Window,
};

use crate::theme::{self, leading, radius, size, space, text, Palette, FONT_MONO};

/// The `mx-tip` tooltip body. Attach with [`with_tip`] so the 350 ms dwell applies.
pub struct Tip(pub SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::tokens(cx);
        div()
            .id("tooltip")
            .test_support()
            .max_w(px(260.))
            .px(px(8.))
            .py(px(5.))
            .rounded(radius::SM)
            .bg(p.overlay_bg_strong)
            .border_1()
            .border_color(p.overlay_border)
            .shadow(popover_shadow(&p))
            .text_size(text::SM)
            .font_weight(theme::W_MEDIUM)
            .line_height(gpui_kit::relative(leading::SNUG))
            .text_color(p.text_strong)
            .child(self.0.clone())
    }
}

/// A tooltip after the §5 dwell (350 ms).
pub fn with_tip<E: StatefulInteractiveElement>(el: E, tip: impl Into<SharedString>) -> E {
    let tip = tip.into();
    el.tooltip(move |_, cx: &mut App| -> AnyView { cx.new(|_| Tip(tip.clone())).into() })
        .tooltip_show_delay(theme::motion::TOOLTIP_DWELL)
}

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

/// The `.mx-pop` surface: raised glass (96%), overlay border, radius 12, shadow.
/// Callers position it (`absolute` + edges) and pass it to [`pop_in`].
pub fn popover(id: &'static str, label: &'static str, p: &Palette) -> Stateful<Div> {
    div()
        .id(id)
        .role(gpui_kit::Role::Dialog)
        .aria_label(label)
        .absolute()
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
pub fn pop_in(el: Stateful<Div>, edge: gpui_kit::Pixels, up: bool) -> impl IntoElement {
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

/// The kit `kbd`: mono 10, hairline box, 18 px tall.
pub fn kbd(label: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(size::KBD)
        .min_w(size::KBD)
        .px(px(5.))
        .rounded(radius::XS)
        .border_1()
        .border_color(p.hairline)
        .font_family(FONT_MONO)
        .text_size(text::XS)
        .font_weight(theme::W_MEDIUM)
        .text_color(p.text_subtle)
        .child(label.into())
}

/// The kit kicker: mono 10 uppercase, tracked, green `//`, muted text.
pub fn kicker(label: &str, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(space::S2)
        .font_family(FONT_MONO)
        .text_size(text::XS)
        .font_weight(theme::W_MEDIUM)
        .text_color(p.text_subtle)
        .child(div().text_color(p.accent).child("//"))
        // No letter-spacing in GPUI: the 0.12em tracking is a native gap.
        .child(SharedString::from(label.to_uppercase()))
}

/// The 1×12 vertical separator (`mx-sep`).
pub fn sep(p: &Palette) -> Div {
    div().flex_none().w(px(1.)).h(px(12.)).bg(p.border_strong)
}

/// An icon by asset path at `size`, painted in the inherited text colour (so a
/// parent's hover / current colour reaches it; GPUI's `svg` doesn't inherit).
#[derive(IntoElement)]
pub struct Icon {
    path: SharedString,
    size: gpui_kit::Pixels,
}

pub fn icon(path: impl Into<SharedString>, size: gpui_kit::Pixels) -> Icon {
    Icon {
        path: path.into(),
        size,
    }
}

impl RenderOnce for Icon {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        svg()
            .path(self.path)
            .flex_none()
            .size(self.size)
            .text_color(window.text_style().color)
    }
}

/// An interactive element the helpers return. Its concrete type changes when
/// gpui-kit's `test-support` wraps it (`.test_support()` -> `Observed<_>`).
pub trait Control: StatefulInteractiveElement + ParentElement + Styled + IntoElement {}
impl<T: StatefulInteractiveElement + ParentElement + Styled + IntoElement> Control for T {}

/// The kit's ghost icon button (`mx-btn--ghost mx-btn--icon`), `h` square.
/// `current` = `aria-current` / pressed: the selected fill. Mouse-down stops here
/// so a titlebar drag never starts from a button.
pub fn icon_button(
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<SharedString>,
    h: gpui_kit::Pixels,
    current: bool,
    p: &Palette,
) -> impl Control {
    let (fg, bg) = if current {
        (p.text_strong, Some(p.wash_strong))
    } else {
        (p.text_muted, None)
    };
    let hover = (p.wash_strong, p.text_strong);
    div()
        .id(id)
        .test_support()
        .role(gpui_kit::Role::Button)
        .aria_label(label)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(h)
        .rounded(px(5.))
        .text_color(fg)
        .when_some(bg, |el, bg| el.bg(bg))
        .cursor_pointer()
        .hover(move |s| s.bg(hover.0).text_color(hover.1))
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation()
        })
}
