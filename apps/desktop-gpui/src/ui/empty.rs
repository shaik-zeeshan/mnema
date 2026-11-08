//! `.mx-empty`: glyph + title + one line + optional actions. Panel-sized
//! (left-aligned), `Center` for a whole pane, `Compact` for a list/panel row.
//! A tone (danger = error + Retry, warn = offline) turns the dashed glyph solid.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Div, SharedString};

use super::{icon, Tone};
use crate::theme::{self, radius, space, text, Palette};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EmptyLayout {
    #[default]
    Panel,
    Center,
    Compact,
}

pub fn empty(
    layout: EmptyLayout,
    tone: Tone,
    icon_path: impl Into<SharedString>,
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    acts: Vec<AnyElement>,
    p: &Palette,
) -> Div {
    let compact = layout == EmptyLayout::Compact;
    let center = layout == EmptyLayout::Center;
    let (glyph_px, icon_px, r) = if compact {
        (px(28.), px(14.), radius::SM)
    } else {
        (px(40.), px(18.), radius::MD)
    };
    let glyph = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(glyph_px)
        .rounded(r)
        .border_1()
        .map(|g| match tone {
            Tone::Neutral => g
                .border_dashed()
                .border_color(p.border_strong)
                .text_color(p.text_subtle),
            _ => {
                let (t, bg, bd) = tone.colors(p);
                g.border_color(bd).bg(bg).text_color(t)
            }
        })
        .child(icon(icon_path, icon_px));
    let title = div()
        .font_weight(theme::W_SEMIBOLD)
        .text_size(if compact { text::MD } else { text::LG })
        .line_height(gpui_kit::relative(1.3))
        .text_color(p.text_strong)
        .child(title.into());
    let body = div()
        .text_size(if compact { text::BASE } else { text::MD })
        .line_height(gpui_kit::relative(1.55))
        .text_color(p.text_muted)
        .child(body.into());
    let acts = div().flex().flex_wrap().gap(space::S2).children(acts);
    if compact {
        return div()
            .flex()
            .items_center()
            .gap(space::S3)
            .px(space::S4)
            .py(space::S3)
            .child(glyph)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(title)
                    .child(body),
            )
            .child(acts);
    }
    div()
        .flex()
        .flex_col()
        .when(center, |d| d.items_center().text_center().mx_auto())
        .when(!center, |d| d.items_start())
        .gap(space::S3)
        .p(space::S6)
        .max_w(px(440.))
        .child(glyph)
        .child(title)
        // `margin: -6px 0 0` on the text: 12 - 6 = 6 px under the title.
        .child(body.mt(px(-6.)).max_w(px(52. * 7.)))
        .child(acts.mt(px(2.)).when(center, |d| d.justify_center()))
}
