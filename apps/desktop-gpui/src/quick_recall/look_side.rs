//! Quick Look's side panel (`QuickLookSide.svelte`): what the stage shows, why
//! it matched, its text (screen) or transcript (audio), and the actions: Open
//! this frame / at this line (↵), Copy (⌘C), Page (⌘↵).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, Div};

use super::look::{clock, highlight, residual_terms};
use super::look_view::{marked_text, SIDE_W};
use super::model::{ms, Ranked};
use super::view::act;
use super::QuickRecall;
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::Button;

impl QuickRecall {
    pub(super) fn look_side(&self, cx: &mut Context<Self>, p: &Palette) -> Div {
        let l = self.look.as_ref().expect("look");
        let terms = residual_terms(&self.results.residual);
        let now = self.now(cx);
        let side = div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(SIDE_W))
            .gap(px(10.))
            .pt(px(14.))
            .px(px(14.))
            .pb(px(12.))
            .border_l_1()
            .border_color(p.hairline);
        let head = |name: String, ch: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap(space::S2)
                .text_size(text::BASE)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_muted)
                .child(self.tile(&l.result, p))
                .child(name)
                .child(div().ml_auto().child(ch))
        };
        let title = |t: String| {
            div()
                .text_size(text::MD)
                .font_weight(theme::W_SEMIBOLD)
                .line_height(relative(1.3))
                .text_color(p.text_strong)
                .child(t)
        };
        let when = |a: String, b: String| {
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .text_color(p.text)
                .child(a)
                .child(div().text_size(text::XS).text_color(p.text_subtle).child(b))
        };
        let text_box = |label: &str, n: usize, body: Div| {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .pb(px(6.))
                        .child(
                            div()
                                .text_size(text::SM)
                                .font_weight(theme::W_MEDIUM)
                                .text_color(p.text_subtle)
                                .child(label.to_string()),
                        )
                        .child(
                            div()
                                .font_family(FONT_MONO)
                                .text_size(text::XS)
                                .text_color(p.text_subtle)
                                .child(format!("{n} lines")),
                        ),
                )
                .child(crate::ui::scroll::area(
                    div()
                        .id("qr-look-lines")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .py(px(8.))
                        .px(px(10.))
                        .rounded(radius::MD)
                        .bg(p.surface_subtle)
                        .border_1()
                        .border_color(p.hairline)
                        .child(body),
                ))
        };
        let copied = l.copied;
        match &l.result {
            Ranked::Frame(f) => {
                let frame_ms = l.frame().map_or(0, |x| x.1);
                let at = crate::chat::format::local(frame_ms, now);
                let mins = ((ms(&f.group_end_at) - ms(&f.group_start_at)) as f64 / 60_000.)
                    .round()
                    .max(1.);
                let text = l.frame().and_then(|x| l.ocr.get(&x.0).cloned());
                let lines: Vec<Vec<(String, bool)>> = text
                    .clone()
                    .flatten()
                    .unwrap_or_default()
                    .lines()
                    .filter(|x| !x.trim().is_empty())
                    .map(|x| highlight(x, &terms))
                    .collect();
                let hits: usize = lines.iter().map(|s| s.iter().filter(|r| r.1).count()).sum();
                let gone = l.preview_id().is_some_and(|id| l.gone.contains_key(&id));
                let body = if lines.is_empty() {
                    div()
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .child(if text.is_none() {
                            "Reading…"
                        } else {
                            "No text was read off this frame."
                        })
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .text_size(text::SM)
                        .line_height(relative(1.45))
                        .text_color(p.text_muted)
                        .children(lines.iter().map(|s| div().child(marked_text(s.clone(), p))))
                };
                side.child(head(
                    f.app_name.clone().unwrap_or_else(|| "Unknown app".into()),
                    chip("screen", ChipKind::Screen, p).into_any_element(),
                ))
                .child(title(
                    f.window_title
                        .clone()
                        .or_else(|| f.app_name.clone())
                        .unwrap_or_else(|| "Untitled".into()),
                ))
                .child(when(
                    format!(
                        "{} · {}",
                        super::rules::day_comma(at.date()),
                        super::rules::clock_secs(at)
                    ),
                    f.url
                        .clone()
                        .unwrap_or_else(|| format!("{mins} min on screen")),
                ))
                .child(if f.found_by_meaning {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .child(chip("by meaning", ChipKind::Neutral, p))
                        .child("no exact words")
                } else {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .child(chip("keyword", ChipKind::Accent, p))
                        .child(format!("{hits} on this frame"))
                })
                .child(text_box("Text on this frame", lines.len(), body))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(
                            Button::new(
                                "qr-look-open",
                                if gone {
                                    "Open in Timeline"
                                } else {
                                    "Open this frame"
                                },
                            )
                            .primary()
                            .sm()
                            .icon("icons/chart-no-axes-gantt.svg")
                            .kbd("↵")
                            .on_click(act(cx, |qr, cx| qr.look_open_here(cx))),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(6.))
                                .child(
                                    Button::new(
                                        "qr-look-copy",
                                        if copied { "Copied" } else { "Copy text" },
                                    )
                                    .sm()
                                    .icon(IconName::Copy.path())
                                    .kbd("⌘C")
                                    .on_click(act(cx, |qr, cx| qr.look_copy(cx))),
                                )
                                .when_some(l.result.page_frame(), |d, id| {
                                    d.child(
                                        Button::new("qr-look-page", "Page")
                                            .ghost()
                                            .sm()
                                            .icon(IconName::ExternalLink.path())
                                            .kbd("⌘↵")
                                            .on_click(act(cx, move |qr, cx| qr.open_page(id, cx))),
                                    )
                                }),
                        ),
                )
            }
            Ranked::Audio(a) => {
                let turns = l.turns.clone().unwrap_or_default();
                let start = crate::chat::format::local(
                    ms(&a.audio_segment.started_at)
                        + l.turn().map_or(a.span_start_ms, |t| t.start_ms) as i64,
                    now,
                );
                let seg_ms =
                    (ms(&a.audio_segment.ended_at) - ms(&a.audio_segment.started_at)).max(0);
                let title_s = a
                    .aligned_frame
                    .as_ref()
                    .and_then(|f| f.window_title.clone().or_else(|| f.app_name.clone()))
                    .unwrap_or_else(|| {
                        if a.mic() {
                            "Microphone".into()
                        } else {
                            "System audio".into()
                        }
                    });
                let body = if turns.is_empty() {
                    div()
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .child("Reading…")
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .children(turns.iter().enumerate().map(|(j, t)| {
                            let cur = j as i32 == l.k;
                            div()
                                .flex()
                                .gap(px(8.))
                                .px(px(4.))
                                .py(px(2.))
                                .rounded(radius::SM)
                                .when(cur, |d| d.bg(p.wash_strong))
                                .text_size(text::SM)
                                .line_height(relative(1.45))
                                .child(
                                    div()
                                        .flex_none()
                                        .font_family(FONT_MONO)
                                        .text_size(text::XS)
                                        .text_color(p.text_subtle)
                                        .child(clock(t.start_ms as i64)),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .text_color(p.text_muted)
                                        .when(!t.speaker.is_empty(), |d| {
                                            d.child(
                                                div()
                                                    .text_color(p.text_strong)
                                                    .font_weight(theme::W_SEMIBOLD)
                                                    .child(t.speaker.clone()),
                                            )
                                        })
                                        .child(marked_text(highlight(&t.text, &terms), p)),
                                )
                        }))
                };
                side.child(head(
                    if a.mic() {
                        "Microphone".into()
                    } else {
                        "System audio".into()
                    },
                    chip(
                        if a.mic() { "mic" } else { "system" },
                        if a.mic() {
                            ChipKind::Mic
                        } else {
                            ChipKind::SysAudio
                        },
                        p,
                    )
                    .into_any_element(),
                ))
                .child(title(title_s))
                .child(when(
                    format!(
                        "{} · {}",
                        super::rules::day_comma(start.date()),
                        crate::chat::format::clock(start)
                    ),
                    format!("{} recording", clock(seg_ms)),
                ))
                .child(text_box("Transcript", turns.len(), body))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(
                            Button::new("qr-look-open", "Open at this line")
                                .primary()
                                .sm()
                                .icon("icons/chart-no-axes-gantt.svg")
                                .kbd("↵")
                                .on_click(act(cx, |qr, cx| qr.look_open_here(cx))),
                        )
                        .child(
                            Button::new(
                                "qr-look-copy",
                                if copied { "Copied" } else { "Copy quote" },
                            )
                            .sm()
                            .icon(IconName::Copy.path())
                            .kbd("⌘C")
                            .on_click(act(cx, |qr, cx| qr.look_copy(cx))),
                        ),
                )
            }
        }
    }
}
