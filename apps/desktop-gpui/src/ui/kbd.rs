//! `kbd` / `.mx-kbd` and the `.mx-keys__k` chord row.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, Div, SharedString};

use crate::theme::{self, radius, size, text, Palette, FONT_MONO};

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

/// One chord as separate keys, 3 px apart (`⌘` `K`).
pub fn keys<S: Into<SharedString> + Clone>(chord: &[S], p: &Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .gap(px(3.))
        .children(chord.iter().cloned().map(|k| kbd(k, p)))
}
