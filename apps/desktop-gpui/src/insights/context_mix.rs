//! Context's "What the dossier is built from" panel (`ContextSide.svelte`
//! `.in-mix`): authored vs inferred as one split bar with its legend, then
//! the authored statements per topic.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, Div};

use super::charts::{hatch, swatch};
use super::context::topics;
use super::parts::panel;
use crate::theme::{self, space, text, Palette, FONT_MONO};
use crate::ui::Tracking as _;
use crate::ui::{alpha, kicker};

type Row = app_core::user_context::commands::AuthoredContextRow;

/// The `.in-mix` panel over the authored `rows` and `ni` inferred views.
pub(super) fn mix(rows: &[Row], ni: usize, p: &Palette) -> Div {
    let n = rows.len();
    let mut by_topic: Vec<(&str, gpui_kit::Hsla, usize)> = topics(p)
        .into_iter()
        .map(|(t, c)| {
            (
                t,
                c,
                rows.iter()
                    .filter(|r| r.item.topic.as_deref() == Some(t))
                    .count(),
            )
        })
        .collect();
    let other = n - by_topic.iter().map(|t| t.2).sum::<usize>();
    if other > 0 {
        by_topic.push(("Other", p.chart_grey_3, other));
    }
    let max_t = by_topic.iter().map(|t| t.2).max().unwrap_or(1).max(1);
    // `b` in a `.num`: 500 SM mono, strong.
    let count = |k: usize| {
        div()
            .font_family(FONT_MONO)
            .text_size(text::SM)
            .font_weight(theme::W_MEDIUM)
            .tracking(-0.01)
            .text_color(p.text_strong)
            .child(k.to_string())
    };
    // `.in-mix__total`: 10 px, 2 px apart.
    let total = div()
        .flex()
        .gap(px(2.))
        .h(px(10.))
        .mt(space::S3)
        .child(
            div()
                .flex_grow(n.max(1) as f32 * if n == 0 { 0.001 } else { 1. })
                .flex_basis(px(0.))
                .rounded(px(2.))
                .bg(p.text_strong),
        )
        .child(
            div()
                .flex_grow(ni.max(1) as f32 * if ni == 0 { 0.001 } else { 1. })
                .flex_basis(px(0.))
                .rounded(px(2.))
                .bg(p.chart_grey_3),
        );
    panel(p)
        .p(space::S5)
        .child(kicker("What the dossier is built from", p))
        .child(total)
        .child(
            // `.in-mix__l`: 500 SM/1 subtle, the counts bold mono strong.
            div()
                .flex()
                .gap(space::S4)
                .mt(space::S2)
                .text_size(text::SM)
                .font_weight(theme::W_MEDIUM)
                .line_height(relative(1.))
                .text_color(p.text_subtle)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(swatch(p.text_strong, 8.))
                        .child("authored")
                        .child(count(n)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        // `.in-sw--hatch`: 1.5 px stripes every 4 px over the colour at 25%.
                        .child(
                            div()
                                .flex_none()
                                .size(px(8.))
                                .rounded(px(2.))
                                .overflow_hidden()
                                .child(
                                    hatch(p.chart_grey_4, alpha(p.chart_grey_4, 0.25), 1.5, 4.)
                                        .size_full(),
                                ),
                        )
                        .child("inferred")
                        .child(count(ni)),
                ),
        )
        .when(n > 0, |d| {
            d.child(
                div()
                    .mt(space::S4)
                    .children(by_topic.into_iter().map(|(t, c, k)| {
                        // `.in-mix__row`: 96 / 1fr / 56, 500 BASE/1.
                        div()
                            .flex()
                            .items_center()
                            .gap(space::S3)
                            .py(px(8.))
                            .border_t_1()
                            .border_color(p.hairline)
                            .text_size(text::BASE)
                            .font_weight(theme::W_MEDIUM)
                            .line_height(relative(1.))
                            .text_color(p.text)
                            .child(
                                div()
                                    .w(px(96.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(swatch(c, 8.))
                                    .child(t),
                            )
                            .child(
                                div().flex_1().child(
                                    div()
                                        .h(px(14.))
                                        .w(relative(k as f32 / max_t as f32))
                                        .rounded(px(2.))
                                        .bg(c),
                                ),
                            )
                            .child(
                                div()
                                    .w(px(56.))
                                    .flex_none()
                                    .flex()
                                    .justify_end()
                                    .child(count(k)),
                            )
                    })),
            )
        })
}
