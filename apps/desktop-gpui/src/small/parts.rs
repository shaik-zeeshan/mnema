//! Pieces the small windows share: the 12 px-corner frame, kit headings,
//! the compact `.mx-notice` with an inline paragraph, the inset panel, the
//! `.mx-spec` rows and the developer window's tone dot.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Div, FontWeight, Hsla, SharedString};

use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::Tracking as _;
use crate::ui::{alpha, icon, mix, Tone};

/// One inline run of a paragraph: text and an optional `(weight, colour)`.
pub type Run<'a> = (&'a str, Option<(FontWeight, Hsla)>);

/// The window body: surface fill, 12 px corners (the native mask in Tauri).
pub fn frame(p: &Palette) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .rounded(radius::LG)
        .bg(p.surface)
        .font_family(theme::FONT_UI)
        .text_color(p.text)
}

/// `.mx-h2`: 600 20/1.25 strong.
pub fn h2(s: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .font_weight(theme::W_SEMIBOLD)
        .text_size(text::XL)
        .tracking(-0.018)
        .line_height(relative(1.25))
        .text_color(p.text_strong)
        .child(s.into())
}

/// `.mx-body-sm`: 400 12/1.5 muted.
pub fn body_sm(s: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .text_size(text::BASE)
        .line_height(relative(1.5))
        .text_color(p.text_muted)
        .child(s.into())
}

/// Strong run (`<b>` inside notice / prose text).
pub fn b(p: &Palette) -> Option<(FontWeight, Hsla)> {
    Some((theme::W_SEMIBOLD, p.text_strong))
}

/// One wrapping paragraph of inline runs.
pub fn para(runs: &[Run], size: gpui_kit::Pixels, color: Hsla) -> Div {
    div()
        .text_size(size)
        .line_height(relative(1.45))
        .text_color(color)
        .child(crate::chat::inline_runs(runs))
}

/// The small windows' `.mx-notice` (24 px icon box, `padding: 8px 10px`) with a
/// paragraph instead of a title.
pub fn note(tone: Tone, icon_path: &'static str, text_el: impl IntoElement, p: &Palette) -> Div {
    let (t, bg, bd) = match tone {
        Tone::Neutral => (p.text_muted, p.wash, p.border),
        _ => tone.colors(p),
    };
    div()
        .flex()
        .items_start()
        .gap(space::S3)
        .px(px(10.))
        .py(px(8.))
        .rounded(radius::MD)
        .border_1()
        .border_color(mix(bd, p.border, 0.7))
        .bg(mix(bg, p.surface, 0.6))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(24.))
                .rounded(radius::SM)
                .text_color(t)
                .bg(alpha(t, 0.12))
                .child(icon(icon_path, px(13.))),
        )
        .child(div().flex_1().min_w_0().pt(px(3.)).child(text_el))
}

/// `.mx-panel.mx-panel--inset`.
pub fn inset_panel(p: &Palette) -> Div {
    div()
        .rounded(radius::LG)
        .border_1()
        .border_color(p.border)
        .bg(p.surface_subtle)
}

/// One `.mx-spec` row: label, value, optional mono note on the right.
pub fn spec_row(
    first: bool,
    label: impl Into<SharedString>,
    value: impl IntoElement,
    note: Option<SharedString>,
    label_w: gpui_kit::Pixels,
    p: &Palette,
) -> Div {
    div()
        .flex()
        .items_start()
        .py(px(7.))
        .when(!first, |d| d.border_t_1().border_color(p.hairline))
        .child(
            div()
                .flex_none()
                .w(label_w)
                .pr(space::S4)
                .font_weight(theme::W_MEDIUM)
                .text_size(text::BASE)
                .line_height(relative(1.4))
                .text_color(p.text_subtle)
                .child(label.into()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(text::BASE)
                .line_height(relative(1.4))
                .text_color(p.text_strong)
                .child(value),
        )
        .children(note.map(|n| {
            div()
                .flex_none()
                .pl(space::S4)
                .text_size(text::SM)
                .line_height(relative(1.4))
                .text_color(p.text_subtle)
                .child(n)
        }))
}

/// `.dv-dot`: 7 px tone dot (ok = soft accent, off = hollow ring, problems glow).
pub fn dot(tone: Tone, size: gpui_kit::Pixels, p: &Palette) -> Div {
    let d = div().flex_none().size(size).rounded_full();
    match tone {
        Tone::Ok => d.bg(mix(p.accent, p.surface, 0.62)),
        Tone::Off => d.border_1().border_color(p.text_subtle),
        Tone::Neutral => d.bg(p.chart_grey_3),
        // The 3 px glow as a circle behind the dot: GPUI's shadow spread
        // doesn't grow the corner radius, so a spread shadow reads square.
        _ => {
            let c = tone.colors(p).0;
            div()
                .flex_none()
                .relative()
                .size(size)
                .child(
                    div()
                        .absolute()
                        .top(px(-3.))
                        .left(px(-3.))
                        .size(size + px(6.))
                        .rounded_full()
                        .bg(alpha(c, 0.18)),
                )
                .child(div().absolute().inset_0().rounded_full().bg(c))
        }
    }
}

/// Mono numbers (`.num`).
pub fn num(s: impl Into<SharedString>, size: gpui_kit::Pixels, color: Hsla) -> Div {
    div()
        .font_family(FONT_MONO)
        .text_size(size)
        .tracking(-0.01)
        .line_height(relative(1.))
        .text_color(color)
        .whitespace_nowrap()
        .child(s.into())
}

/// A finished element list helper for `.children(..)` call sites.
pub fn any(el: impl IntoElement) -> AnyElement {
    el.into_any_element()
}
