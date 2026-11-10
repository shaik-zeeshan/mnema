//! The graphical answer blocks (`ChatTurn.svelte` + the Insights charts):
//! bars (MiniBars), dossier (chips + ConfidenceBar) and timeline. They carry
//! already-parsed data; nothing here parses.

use capture_types::{BarsItem, DossierItem, TimelineItem};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Div, Hsla, SharedString};

use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::mix;

/// `.mx-panel.ch-block` with an optional `h5` head and right-hand label.
pub fn panel(title: Option<&str>, label: Option<&str>, body: AnyElement, p: &Palette) -> Div {
    let head = (title.is_some() || label.is_some()).then(|| {
        div()
            .flex()
            .items_baseline()
            .gap(space::S3)
            .mb(space::S3)
            .children(title.map(|t| {
                div()
                    .font_weight(theme::W_SEMIBOLD)
                    .text_size(text::MD)
                    .line_height(relative(1.3))
                    .text_color(p.text_strong)
                    .child(SharedString::from(t.to_string()))
            }))
            .children(label.map(|l| {
                div()
                    .ml_auto()
                    .font_family(FONT_MONO)
                    .text_size(text::XS)
                    .text_color(p.text_subtle)
                    .child(SharedString::from(l.to_string()))
            }))
    });
    div()
        .px(space::S5)
        .py(space::S4)
        .mb(space::S4)
        .bg(p.surface)
        .border_1()
        .border_color(p.border)
        .rounded(radius::LG)
        .children(head)
        .child(body)
}

/// MiniBars: a rank ramp from pure accent toward the track (magnitude, not category).
pub fn bars(items: &[BarsItem], p: &Palette) -> Div {
    let max = items.iter().fold(0f64, |m, i| m.max(i.value));
    let steps = items.len().saturating_sub(1).max(1) as f32;
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .children(items.iter().enumerate().map(|(i, it)| {
            let w = if max > 0. {
                (it.value / max).clamp(0., 1.) as f32
            } else {
                0.
            };
            let fade = (i as f32 / steps * 58.).round() / 100.;
            let fill = mix(p.accent, p.surface_hover, 1. - fade);
            let dominant = max > 0. && it.value == max;
            let val = it
                .sublabel
                .clone()
                .unwrap_or_else(|| format!("{}", it.value));
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .w(px(110.))
                        .flex_none()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(11.5))
                        .text_color(if dominant { p.text } else { p.text_muted })
                        .child(SharedString::from(it.label.clone())),
                )
                .child(
                    div()
                        .flex_1()
                        .h(px(10.))
                        .rounded_full()
                        .bg(p.surface_hover)
                        .border_1()
                        .border_color(p.border)
                        .overflow_hidden()
                        .child(
                            div()
                                .h_full()
                                .w(relative(w))
                                .rounded_full()
                                .bg(fill)
                                .opacity(0.92),
                        ),
                )
                .child(
                    div()
                        .w(px(56.))
                        .flex_none()
                        .text_right()
                        .text_size(text::XS)
                        .text_color(p.text_muted)
                        .child(SharedString::from(val)),
                )
        }))
}

/// ConfidenceBar: 56×5 accent fill + the percentage.
pub fn confidence(c: f64, p: &Palette) -> Div {
    let c = if c.is_finite() { c.clamp(0., 1.) } else { 0. };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(
            div()
                .relative()
                .w(px(56.))
                .h(px(5.))
                .flex_none()
                .rounded_full()
                .bg(p.surface_hover)
                .border_1()
                .border_color(p.border)
                .overflow_hidden()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .h_full()
                        .w(relative(c as f32))
                        .rounded_full()
                        .bg(p.accent),
                ),
        )
        .child(
            div()
                .text_size(px(10.5))
                .text_color(p.text_muted)
                .child(SharedString::from(format!("{}%", (c * 100.).round()))),
        )
}

pub fn dossier(items: &[DossierItem], p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .children(items.iter().enumerate().map(|(i, it)| {
            div()
                .flex()
                .items_center()
                .gap(space::S3)
                .py(px(9.))
                .when(i == 0, |d| d.pt_0())
                .when(i > 0, |d| d.border_t_1().border_color(p.hairline))
                .child(
                    div().w(px(96.)).flex_none().children(
                        it.subject
                            .as_ref()
                            .map(|s| chip(s.clone(), ChipKind::Neutral, p)),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(text::BASE)
                        .line_height(relative(1.5))
                        .text_color(p.text_strong)
                        .child(SharedString::from(it.statement.clone())),
                )
                .child(
                    div()
                        .w(px(70.))
                        .flex_none()
                        .child(confidence(it.confidence, p)),
                )
        }))
}

fn category_color(category: Option<&str>, p: &Palette) -> Hsla {
    match category {
        Some("research") => p.cat_research,
        Some("creating") => p.cat_creating,
        Some("communication") => p.cat_communication,
        Some("learning") => p.cat_learning,
        Some("personal") => p.cat_personal,
        _ => p.chart_grey_3,
    }
}

/// Timeline: one row per interval (category dot, label, range, app).
/// shortcut: a list, not the Insights rail with app icons (`insights/charts`
/// has the pieces; the block doesn't use them yet).
pub fn timeline(title: Option<&str>, items: &[TimelineItem], p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .children(title.map(|t| {
            div()
                .mb(px(4.))
                .font_weight(theme::W_SEMIBOLD)
                .text_size(text::MD)
                .text_color(p.text_strong)
                .child(SharedString::from(t.to_string()))
        }))
        .children(items.iter().map(|it| {
            let range = match &it.end {
                Some(e) => format!("{} – {e}", it.start),
                None => it.start.clone(),
            };
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(text::BASE)
                .child(
                    div()
                        .size(px(8.))
                        .flex_none()
                        .rounded_full()
                        .bg(category_color(it.category.as_deref(), p)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(p.text_strong)
                        .child(SharedString::from(it.label.clone())),
                )
                .children(it.app.as_ref().map(|a| {
                    div()
                        .text_color(p.text_muted)
                        .child(SharedString::from(a.clone()))
                }))
                .child(
                    div()
                        .font_family(FONT_MONO)
                        .text_size(text::XS)
                        .text_color(p.text_subtle)
                        .child(SharedString::from(range)),
                )
        }))
}
