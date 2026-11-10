//! Feedback states that stay put: `.mx-notice` (a callout inside content),
//! `.mx-banner` (window-wide, under the titlebar; one at a time) and
//! `.mx-inline` (one line under a field). Flat: tinted fill + tone border.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Div, SharedString};

use super::{alpha, icon, mix, spread, Tone};
use crate::theme::{self, radius, size, space, text, Palette};

/// `.mx-notice[data-tone]`: icon box, title, optional text, actions on the right.
pub fn notice(
    tone: Tone,
    icon_path: impl Into<SharedString>,
    title: impl Into<SharedString>,
    body: Option<SharedString>,
    acts: Vec<AnyElement>,
    p: &Palette,
) -> Div {
    let body = body.map(|b| {
        div()
            .text_size(text::BASE)
            .line_height(gpui_kit::relative(1.45))
            .text_color(p.text_muted)
            .child(b)
            .into_any_element()
    });
    notice_el(tone, icon_path, title, body, acts, p)
}

/// [`notice`] with a body the caller builds (mixed runs, a raw error box).
pub fn notice_el(
    tone: Tone,
    icon_path: impl Into<SharedString>,
    title: impl Into<SharedString>,
    body: Option<AnyElement>,
    acts: Vec<AnyElement>,
    p: &Palette,
) -> Div {
    let (t, bg, bd) = tone.colors(p);
    let off = tone == Tone::Off;
    div()
        .flex()
        .items_center()
        .gap(space::S3)
        .px(px(12.))
        .py(px(10.))
        .rounded(radius::MD)
        .border_1()
        .border_color(mix(bd, p.border, 0.7))
        .when(off, |d| d.border_dashed())
        .when(!off, |d| d.bg(mix(bg, p.surface, 0.6)))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded(radius::SM)
                .text_color(t)
                .bg(alpha(t, 0.12))
                .child(icon(icon_path, size::BTN_ICON_MD)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .font_weight(theme::W_SEMIBOLD)
                        .text_size(text::MD)
                        .line_height(gpui_kit::relative(1.35))
                        .text_color(p.text_strong)
                        .child(title.into()),
                )
                .children(body),
        )
        .child(div().flex().flex_none().gap(px(6.)).children(acts))
}

/// `.mx-banner[data-tone]`: dot, message (bold runs via `strong`), actions.
pub fn banner(tone: Tone, message: AnyElement, acts: Vec<AnyElement>, p: &Palette) -> Div {
    let (t, bg, bd) = match tone {
        Tone::Neutral => (p.text_subtle, p.surface_subtle, p.border),
        _ => tone.colors(p),
    };
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .min_h(size::BANNER_MIN_H)
        .pl(space::S4)
        .pr(px(8.))
        .py(px(5.))
        .border_b_1()
        .border_color(mix(bd, p.border, 0.7))
        .bg(mix(bg, p.bg, 0.7))
        .text_size(text::MD)
        .line_height(gpui_kit::relative(1.4))
        .text_color(p.text)
        .child(
            div()
                .flex_none()
                .size(px(7.))
                .rounded_full()
                .bg(t)
                .shadow(vec![spread(alpha(t, 0.2), px(3.))]),
        )
        .child(div().flex_1().min_w_0().child(message))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .children(acts),
        )
}

/// A bold run inside banner / inline text (`<b>`).
pub fn strong(s: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .font_weight(theme::W_SEMIBOLD)
        .text_color(p.text_strong)
        .child(s.into())
}

/// `.mx-inline[data-tone]`: 13 px icon (or a spinner) + text (+ a ghost sm action).
pub fn inline(tone: Tone, lead: Option<AnyElement>, parts: Vec<AnyElement>, p: &Palette) -> Div {
    let t = tone.colors(p).0;
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .text_size(text::BASE)
        .line_height(gpui_kit::relative(1.45))
        .text_color(t)
        .children(lead)
        .children(parts)
}
