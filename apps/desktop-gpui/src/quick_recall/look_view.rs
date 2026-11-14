//! Quick Look's drawing (`QuickLook.svelte` + `QuickLookSide.svelte`): a fixed
//! 476 px body so scrubbing never resizes the bar. Screen: the frame with its
//! matches outlined, a 7-frame strip 15 s apart, the frame's text and actions.
//! Audio: the line set large over a waveform, the turns as the strip, the transcript.

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, img, px, relative, Context, Div, ObjectFit, Resource, Role, StyledText, Window,
};

use super::look::{
    clock, highlight, missing_copy, residual_terms, signed_clock, strip_window, Look, RANGE,
    STEP_MS, STRIP_CELLS,
};
use super::model::{ms, Ranked};
use super::results::match_box;
use super::QuickRecall;
use crate::theme::{self, radius, size, text, Palette, FONT_MONO};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::Tracking as _;
use crate::ui::{alpha, icon, spread, Tone};

pub(super) const SIDE_W: f32 = 256.;

pub(super) fn marked_text(segs: Vec<(String, bool)>, p: &Palette) -> StyledText {
    let mut s = String::new();
    let mut hl = Vec::new();
    for (t, m) in segs {
        let start = s.len();
        s.push_str(&t);
        if m {
            hl.push((start..s.len(), super::results::mark_style(p)));
        }
    }
    StyledText::new(s).with_highlights(hl)
}

fn tag(p: &Palette) -> Div {
    div()
        .absolute()
        .top(px(10.))
        .flex()
        .items_center()
        .gap(px(7.))
        .h(px(22.))
        .px(px(8.))
        .rounded(radius::SM)
        .bg(p.overlay_bg_strong)
        .border_1()
        .border_color(p.overlay_border)
        .whitespace_nowrap()
}

impl QuickRecall {
    pub(super) fn look_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> Div {
        let side = self.look_side(cx, p);
        let (stage, strip) = match self.look.as_ref().map(Look::is_screen) {
            Some(true) => (self.screen_stage(window, cx, p), self.screen_strip(cx, p)),
            _ => (self.audio_stage(p), self.turn_strip(cx, p)),
        };
        div()
            .flex()
            .h(px(super::LOOK_H))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(stage.flex_1().min_h_0())
                    .child(strip),
            )
            .child(side)
    }

    fn screen_stage(&mut self, window: &mut Window, cx: &mut Context<Self>, p: &Palette) -> Div {
        let l = self.look.as_ref().expect("look");
        let Ranked::Frame(hit) = &l.result else {
            return div();
        };
        let frame = l.frame();
        let frame_ms = frame.map_or(0, |f| f.1);
        let match_ms = l
            .strip
            .get(RANGE as usize)
            .copied()
            .flatten()
            .map_or(frame_ms, |f| f.1);
        let offset = if l.k == 0 { 0 } else { frame_ms - match_ms };
        let at = crate::chat::format::local(frame_ms, self.now(cx));
        let stamp = format!("· {}", super::rules::clock_secs(at));
        let lead = if l.k == 0 {
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .child(div().size(px(6.)).rounded_full().bg(p.accent))
                .child(
                    div()
                        .text_color(p.text_strong)
                        .font_weight(theme::W_MEDIUM)
                        .child("matched frame"),
                )
        } else {
            div()
                .flex()
                .gap(px(7.))
                .child(
                    div()
                        .text_color(p.text_strong)
                        .font_weight(theme::W_MEDIUM)
                        .child(signed_clock(offset)),
                )
                .child(if offset >= 0 {
                    "after the match"
                } else {
                    "before the match"
                })
        };
        let boxes = frame
            .and_then(|f| l.boxes.get(&f.0))
            .cloned()
            .unwrap_or_default();
        let keyword = boxes
            .iter()
            .any(|b| b.kind == app_infra::MatchBoxKind::Keyword);
        let pixel = l.preview_id();
        let gone = pixel.and_then(|id| l.gone.get(&id)).cloned();
        let path = pixel
            .and_then(|id| l.stage.get(&id).or_else(|| l.thumbs.get(&id)))
            .cloned();
        let (fw, fh) = (
            hit.representative_frame.width.unwrap_or(16) as f32,
            hit.representative_frame.height.unwrap_or(10) as f32,
        );
        let mut stage = div()
            .relative()
            .overflow_hidden()
            .bg(alpha(p.surface_subtle, 0.6));
        stage = stage.child(
            tag(p)
                .left(px(12.))
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .text_color(p.text_muted)
                .child(lead)
                .child(stamp),
        );
        if keyword || hit.found_by_meaning {
            let legend = div()
                .flex()
                .gap(px(12.))
                .text_size(text::XS)
                .text_color(p.text_subtle)
                .when(keyword, |d| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            // A 14×8 swatch with a 1.5 px *outline* (outside it, no layout space).
                            .child(
                                div()
                                    .w(px(17.))
                                    .h(px(11.))
                                    .m(px(-1.5))
                                    .rounded(px(3.5))
                                    .bg(p.accent_bg)
                                    .border(px(1.5))
                                    .border_color(p.accent),
                            )
                            .child("keyword"),
                    )
                })
                .when(hit.found_by_meaning, |d| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .w(px(16.))
                                    .h(px(10.))
                                    .m(px(-1.))
                                    .rounded(px(3.))
                                    .border_1()
                                    .border_dashed()
                                    .border_color(p.text_subtle),
                            )
                            .child("meaning"),
                    )
                });
            stage = stage.child(tag(p).right(px(12.)).child(legend));
        }
        if let Some(reason) = gone {
            let (title, body) = missing_copy(&reason);
            return stage.child(
                div()
                    .absolute()
                    .top(px(40.))
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(empty(
                        EmptyLayout::Center,
                        Tone::Warn,
                        "icons/image-off.svg",
                        title,
                        body,
                        Vec::new(),
                        p,
                    )),
            );
        }
        if let Some(path) = path {
            // Fit inside the stage under the tag row: 28 px sides, 40 top, 12 bottom.
            let natural = {
                let res = Resource::Path(path.clone().into());
                self.images
                    .update(cx, |c, cx| gpui_kit::ImageCache::load(c, &res, window, cx))
                    .and_then(Result::ok)
                    .map(|i| i.size(0))
                    .map(|s| (s.width.0 as f32, s.height.0 as f32))
                    .unwrap_or((fw, fh))
            };
            let (sw, sh) = (super::WIDTH - SIDE_W - 2., super::LOOK_H - 74.);
            let (w, h) = (sw - 28., sh - 40. - 12.);
            let z = (w / natural.0).min(h / natural.1).max(0.);
            let (dw, dh) = (natural.0 * z, natural.1 * z);
            stage = stage.child(
                div()
                    .absolute()
                    .left(px((sw - dw) / 2.))
                    .top(px(40. + (h - dh) / 2.))
                    .w(px(dw))
                    .h(px(dh))
                    .overflow_hidden()
                    .rounded(radius::MD)
                    .shadow(vec![spread(p.border_strong, px(1.))])
                    .child(
                        img(path)
                            .image_cache(&self.images)
                            .size_full()
                            .rounded(radius::MD)
                            .object_fit(ObjectFit::Fill),
                    )
                    .children(boxes.iter().map(|b| match_box(b, p))),
            );
        }
        stage
    }

    fn screen_strip(&self, cx: &mut Context<Self>, p: &Palette) -> Div {
        let l = self.look.as_ref().expect("look");
        let match_ms = l.strip.get(RANGE as usize).copied().flatten().map(|f| f.1);
        let cells = strip_window(l.k, -RANGE, RANGE, STRIP_CELLS)
            .into_iter()
            .map(|j| {
                let f = l.strip.get((j + RANGE) as usize).copied().flatten();
                let px_id = f.map(|f| l.pixel_id(f));
                let off = f.map_or(j as i64 * STEP_MS, |f| f.1 - match_ms.unwrap_or(f.1));
                let current = j == l.k;
                let gone = px_id.is_some_and(|id| l.gone.contains_key(&id));
                let thumb = px_id.and_then(|id| l.thumbs.get(&id)).cloned();
                let pic = div()
                    .relative()
                    .w_full()
                    .aspect_ratio(8. / 5.)
                    .overflow_hidden()
                    .rounded(px(4.))
                    .shadow(vec![spread(p.border, px(1.))])
                    .bg(if gone { p.wash } else { p.surface_subtle })
                    .when(!current, |d| d.opacity(0.6))
                    .when(gone, |d| {
                        d.flex()
                            .items_center()
                            .justify_center()
                            .text_color(p.text_faint)
                            .child(icon("icons/image-off.svg", size::ICON_SM))
                    })
                    .when_some(thumb.filter(|_| !gone), |d, t| {
                        d.child(
                            img(t)
                                .image_cache(&self.images)
                                .size_full()
                                .rounded(px(4.))
                                .object_fit(ObjectFit::Cover),
                        )
                    });
                let label = div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(5.))
                    .h(px(12.))
                    .when(j == 0, |d| {
                        d.child(div().size(px(5.)).rounded_full().bg(p.accent))
                    })
                    .child(if j == 0 {
                        "match".to_string()
                    } else {
                        signed_clock(off)
                    });
                div()
                    .id(("qr-film", (j + RANGE) as usize))
                    .test_support()
                    .role(Role::Button)
                    .aria_label(if j == 0 {
                        "Matched frame".to_string()
                    } else {
                        format!("{} from the match", signed_clock(off))
                    })
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(5.))
                    .p(px(4.))
                    .rounded(radius::MD)
                    .font_family(FONT_MONO)
                    .text_size(text::XS)
                    .text_color(if current {
                        p.text_strong
                    } else {
                        p.text_subtle
                    })
                    .when(current, |d| d.bg(p.wash_strong))
                    .when(!current, |d| d.hover(|s| s.bg(p.wash).text_color(p.text)))
                    .cursor_pointer()
                    .child(pic)
                    .child(label)
                    .on_click(cx.listener(move |qr, _, _, cx| {
                        qr.look_go(j, cx);
                        qr.refresh(cx);
                    }))
            });
        div()
            .flex()
            .gap(px(6.))
            .min_w_0()
            .pt(px(8.))
            .px(px(10.))
            .pb(px(9.))
            .border_t_1()
            .border_color(p.hairline)
            .children(cells)
    }

    fn audio_stage(&self, p: &Palette) -> Div {
        let l = self.look.as_ref().expect("look");
        let Ranked::Audio(a) = &l.result else {
            return div();
        };
        let s = if a.mic() {
            p.source_mic
        } else {
            p.source_sysaudio
        };
        let terms = residual_terms(&self.results.residual);
        let stage = div()
            .relative()
            .overflow_hidden()
            .bg(alpha(p.surface_subtle, 0.6));
        let (Some(turns), Some(t)) = (l.turns.as_ref(), l.turn()) else {
            return stage;
        };
        let match_k = super::look::match_turn_index(turns, a.span_start_ms).unwrap_or(0) as i32;
        let lead = if l.k == match_k {
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .child(div().size(px(6.)).rounded_full().bg(p.accent))
                .child(
                    div()
                        .text_color(p.text_strong)
                        .font_weight(theme::W_MEDIUM)
                        .child("matched line"),
                )
        } else {
            div()
                .flex()
                .gap(px(7.))
                .child(
                    div()
                        .text_color(p.text_strong)
                        .font_weight(theme::W_MEDIUM)
                        .child(if l.k > match_k { "after" } else { "before" }),
                )
                .child("the match")
        };
        let seg_ms = (ms(&a.audio_segment.ended_at) - ms(&a.audio_segment.started_at)).max(0);
        let cells = strip_window(l.k, 0, turns.len() as i32 - 1, 4);
        let from = (turns[cells[0] as usize].start_ms as i64 - 20_000).max(0);
        let end = turns[*cells.last().expect("cells") as usize].end_ms as i64 + 20_000;
        let to = (from + 1).max(if seg_ms > 0 { seg_ms.min(end) } else { end });
        let bars = super::results::seeded_bars(&a.group_key, 120, 14, 80);
        let x = |m: i64| ((m - from) as f32 / (to - from) as f32).clamp(0., 1.);
        let wave = div()
            .relative()
            .flex()
            .items_center()
            .gap(px(2.))
            .h(px(64.))
            .children(bars.into_iter().enumerate().map(|(i, v)| {
                let at = from as f32 + (i as f32 + 0.5) / 120. * (to - from) as f32;
                let on = at >= t.start_ms as f32 && at <= t.end_ms as f32;
                div()
                    .flex_1()
                    .h(relative(v as f32 / 100.))
                    .min_h(px(2.))
                    .rounded(px(1.))
                    .bg(if on { alpha(s, 0.7) } else { alpha(p.fg, 0.18) })
            }))
            .child(
                div()
                    .absolute()
                    .top(px(-6.))
                    .bottom(px(-6.))
                    .left(relative(x(turns[match_k as usize].start_ms as i64)))
                    .w(px(1.5))
                    .bg(p.accent),
            )
            .when(l.k != match_k, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(-6.))
                        .bottom(px(-6.))
                        .left(relative(x(t.start_ms as i64)))
                        .w(px(1.5))
                        .bg(p.text_strong),
                )
            });
        let title = a
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
        stage
            .child(
                tag(p)
                    .left(px(12.))
                    .font_family(FONT_MONO)
                    .text_size(text::SM)
                    .text_color(p.text_muted)
                    .child(lead)
                    .child(format!("· {} into the recording", clock(t.start_ms as i64))),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(18.))
                    .pt(px(44.))
                    .px(px(36.))
                    .pb(px(20.))
                    .when(!t.speaker.is_empty(), |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(9.))
                                .text_size(text::MD)
                                .font_weight(theme::W_MEDIUM)
                                .text_color(p.text_muted)
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(24.))
                                        .rounded_full()
                                        .bg(crate::ui::mix(s, p.surface, 0.26))
                                        .text_color(p.text_strong)
                                        .text_size(text::SM)
                                        .font_weight(theme::W_SEMIBOLD)
                                        .child(
                                            t.speaker
                                                .chars()
                                                .next()
                                                .map(|c| c.to_uppercase().to_string())
                                                .unwrap_or_default(),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_color(p.text_strong)
                                        .font_weight(theme::W_SEMIBOLD)
                                        .child(t.speaker.clone()),
                                )
                                .child(format!("in {title}")),
                        )
                    })
                    .child(
                        div()
                            .relative()
                            .pl(px(38.))
                            .text_size(px(24.))
                            .tracking(-0.018)
                            .line_height(relative(1.34))
                            .font_weight(theme::W_MEDIUM)
                            .text_color(p.text_strong)
                            .line_clamp(5)
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top(px(-6.))
                                    .text_size(px(56.))
                                    .font_weight(gpui_kit::FontWeight::BOLD)
                                    .text_color(alpha(s, 0.72))
                                    .child("\u{201C}"),
                            )
                            .child(marked_text(highlight(&t.text, &terms), p)),
                    )
                    .child(wave)
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .font_family(FONT_MONO)
                            .text_size(text::XS)
                            .text_color(p.text_subtle)
                            .child(clock(from))
                            .child(format!("of {}", clock(seg_ms)))
                            .child(clock(to)),
                    ),
            )
    }

    fn turn_strip(&self, cx: &mut Context<Self>, p: &Palette) -> Div {
        let l = self.look.as_ref().expect("look");
        let row = div()
            .flex()
            .gap(px(6.))
            .min_w_0()
            .pt(px(8.))
            .px(px(10.))
            .pb(px(9.))
            .border_t_1()
            .border_color(p.hairline);
        let (Some(turns), Ranked::Audio(a)) = (l.turns.as_ref(), &l.result) else {
            return row;
        };
        let match_k = super::look::match_turn_index(turns, a.span_start_ms).unwrap_or(0) as i32;
        let cells = strip_window(l.k, 0, turns.len() as i32 - 1, 4)
            .into_iter()
            .map(|j| {
                let t = &turns[j as usize];
                let current = j == l.k;
                div()
                    .id(("qr-turn", j as usize))
                    .role(Role::Button)
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(5.))
                    .p(px(4.))
                    .rounded(radius::MD)
                    .cursor_pointer()
                    .when(current, |d| d.bg(p.wash_strong))
                    .child(
                        // `box-shadow: 0 0 0 1px` as a real border just outside the 52 px
                        // cell: a shadow under an unfilled box paints it solid.
                        div()
                            .h(px(54.))
                            .m(px(-1.))
                            .py(px(6.))
                            .px(px(8.))
                            .rounded(px(5.))
                            .border_1()
                            .border_color(p.border)
                            .overflow_hidden()
                            .text_size(text::SM)
                            .line_height(relative(1.35))
                            .text_color(p.text_muted)
                            .line_clamp(3)
                            .child(format!(
                                "{}{}",
                                if t.speaker.is_empty() {
                                    String::new()
                                } else {
                                    format!("{} ", t.speaker)
                                },
                                t.text
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .pl(px(2.))
                            .font_family(FONT_MONO)
                            .text_size(text::XS)
                            .text_color(if current {
                                p.text_strong
                            } else {
                                p.text_subtle
                            })
                            .when(j == match_k, |d| {
                                d.child(div().size(px(5.)).rounded_full().bg(p.accent))
                            })
                            .child(clock(t.start_ms as i64)),
                    )
                    .on_click(cx.listener(move |qr, _, _, cx| {
                        qr.look_go(j, cx);
                        qr.refresh(cx);
                    }))
            });
        row.children(cells)
    }
}
