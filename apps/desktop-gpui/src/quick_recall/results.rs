//! The ranked list (`ResultsList` / `BestMoment` / `SearchResultCard`): the
//! first-search skeleton, the best moment (a crop zoomed onto its first match
//! at the preview's native pixels, or the matched stretch of audio), then
//! one-line rows. Focus stays in the field; a click selects, ↵ opens.

use std::collections::HashSet;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, img, px, relative, AnyElement, Context, Div, FontWeight, HighlightStyle, Hsla, ObjectFit,
    Resource, Role, SharedString, StyledText, Window,
};

use super::model::{ms, snippet_runs, Ranked};
use super::view::{act, section};
use super::QuickRecall;
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::Tracking as _;
use crate::ui::{
    alpha,
    chip::{chip, ChipKind},
    icon, inset, mix, skeleton, Button,
};

/// The best moment's crop (`.rc-crop`): 268 × 168.
const CROP_W: f32 = 268.;
const CROP_H: f32 = 168.;
const ROW_H: f32 = 34.;

/// A snippet with its `<mark>` runs; audio reads as a quote.
pub(super) fn marked(snippet: &str, quote: bool, p: &Palette) -> StyledText {
    let mut s = String::new();
    let mut hl = Vec::new();
    if quote {
        s.push('“');
    }
    for (t, m) in snippet_runs(snippet) {
        let start = s.len();
        s.push_str(&t);
        if m {
            hl.push((start..s.len(), mark_style(p)));
        }
    }
    if quote {
        s.push('”');
    }
    StyledText::new(s).with_highlights(hl)
}

/// `mark` (kit-extra.css): accent ink on the accent tint with a 1 px accent-border
/// underline. Its 2 px corner and side padding can't exist inside a text run.
pub(super) fn mark_style(p: &Palette) -> HighlightStyle {
    HighlightStyle {
        color: Some(p.accent),
        background_color: Some(p.accent_bg),
        underline: Some(gpui_kit::UnderlineStyle {
            thickness: px(1.),
            color: Some(p.accent_border),
            wavy: false,
        }),
        ..Default::default()
    }
}

impl QuickRecall {
    /// The app icon (letter fallback) for a frame; the source glyph for audio.
    pub(super) fn tile(&self, r: &Ranked, p: &Palette) -> AnyElement {
        match r {
            Ranked::Frame(f) => {
                let name = f.app_name.clone().unwrap_or_else(|| "?".into());
                self.app_glyph(
                    f.app_bundle_id.as_deref(),
                    &name,
                    18.,
                    Some(p.text_muted),
                    p,
                )
            }
            Ranked::Audio(a) => source_glyph(a.mic(), p),
        }
    }

    pub(super) fn result_row(
        &self,
        id: (&'static str, usize),
        r: &Ranked,
        selected: bool,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> impl IntoElement {
        let time = super::rules::moment_time(ms(r.start()), self.now(cx));
        let (i, kind) = (id.1, id.0);
        let r2 = r.clone();
        let snippet = marked(r.snippet(), matches!(r, Ranked::Audio(_)), p);
        div()
            .id(id)
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .group("qr-row")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(ROW_H))
            .pl(px(10.))
            .pr(px(4.))
            .rounded(radius::MD)
            .text_color(p.text)
            .when(selected, |d| d.bg(p.wash_strong))
            .when(!selected, |d| d.hover(|s| s.bg(p.wash)))
            .child(div().flex_none().w(px(18.)).child(self.tile(r, p)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .flex()
                    .gap(space::S2)
                    .text_size(text::MD)
                    .child(
                        div()
                            .flex_none()
                            .font_weight(theme::W_MEDIUM)
                            .text_color(p.text_strong)
                            .child(r.title()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(p.text_muted)
                            .child(snippet),
                    ),
            )
            .when(r.meaning(), |d| {
                d.child(
                    div()
                        .font_family(FONT_MONO)
                        .text_size(text::XS)
                        .text_color(p.text_subtle)
                        .child("meaning"),
                )
            })
            .child(
                div()
                    .min_w(px(64.))
                    .flex()
                    .justify_end()
                    .font_family(FONT_MONO)
                    .text_size(text::SM)
                    .text_color(if selected { p.text } else { p.text_subtle })
                    .child(time),
            )
            .child(
                div()
                    .w(px(26.))
                    .flex()
                    .justify_center()
                    .invisible()
                    .when(selected, |d| d.visible())
                    .group_hover("qr-row", |s| s.visible())
                    .child(
                        Button::icon_only((kind, i + 1000), IconName::Eye.path(), "Quick Look")
                            .ghost()
                            .sm()
                            .on_click(act(cx, move |qr, cx| qr.look_show(r2.clone(), cx))),
                    ),
            )
            .on_click(cx.listener(move |qr, _, _, cx| {
                if kind == "qr-opt" {
                    qr.select_result(i);
                } else {
                    qr.ask.sel = i;
                    qr.ask.cite = 0;
                    qr.ask.nav = true;
                }
                qr.refresh(cx);
            }))
    }

    pub(super) fn results_list(
        &mut self,
        rows_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let r = &self.results;
        let list = div()
            .id("quick-recall-results")
            .role(Role::ListBox)
            .aria_label("Search results")
            .flex()
            .flex_col();
        if r.loading && !r.has_results() {
            return div().child(skeletons(p)).into_any_element();
        }
        let visible = self.visible_results().to_vec();
        let total = self.results.ranked.len();
        let capped = self.results.capped();
        let first = if rows_only { 0 } else { 1 };
        let mut col = div()
            .flex()
            .flex_col()
            .when(self.results.loading, |d| d.opacity(0.55));
        if let (Some(best), false) = (visible.first(), rows_only) {
            col =
                col.child(self.best_moment(best, self.results.selected == Some(0), window, cx, p));
        }
        if visible.len() > first {
            let days: HashSet<time::Date> = self
                .results
                .ranked
                .iter()
                .map(|r| crate::chat::format::local(ms(r.start()), self.now(cx)).date())
                .collect();
            let sec = if rows_only {
                section(
                    "Moments",
                    Some(super::rules::count_label(total, capped)),
                    None,
                    p,
                )
            } else {
                let breakdown = super::rules::moment_breakdown(
                    self.results.frames.len(),
                    self.results.audio.len(),
                    days.len(),
                    self.results.has_more,
                );
                section(
                    "More moments",
                    Some(super::rules::count_label(total - 1, capped)),
                    Some(breakdown),
                    p,
                )
            };
            col = col.child(sec);
            for (k, res) in visible.iter().enumerate().skip(first) {
                let sel = self.results.selected == Some(k);
                col = col.child(self.result_row(("qr-opt", k), res, sel, cx, p));
            }
        }
        if let Some(label) = super::rules::more_row_label(total, self.results.expanded) {
            col = col.child(
                div()
                    .id("qr-more")
                    .role(Role::Button)
                    .w_full()
                    .py(px(8.))
                    .flex()
                    .justify_center()
                    .rounded(px(7.))
                    .text_size(text::SM)
                    .text_color(p.text_subtle)
                    .cursor_pointer()
                    .hover(|s| s.text_color(p.accent).bg(p.surface_hover))
                    .child(label)
                    .on_click(cx.listener(|qr, _, _, cx| {
                        qr.toggle_expanded();
                        qr.refresh(cx);
                    })),
            );
        }
        list.child(col).into_any_element()
    }

    /// `behind` = the opaque colour under the crop (the row's fill over the shell).
    fn crop(
        &self,
        r: &Ranked,
        behind: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let base = div()
            .relative()
            .flex_none()
            .w(px(CROP_W))
            .h(px(CROP_H))
            .overflow_hidden()
            .rounded(radius::MD)
            .bg(p.surface_subtle);
        let ring = div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .rounded(radius::MD)
            .shadow(vec![inset(p.border, px(1.))]);
        match r {
            Ranked::Frame(f) => {
                let start = crate::chat::format::clock(crate::chat::format::local(
                    ms(&f.group_start_at),
                    self.now(cx),
                ));
                let mut el = base;
                if let Some(path) = self.thumbs.get(&f.thumbnail_frame_id).cloned() {
                    let res = Resource::Path(path.clone().into());
                    let natural = self
                        .images
                        .update(cx, |c, cx| gpui_kit::ImageCache::load(c, &res, window, cx))
                        .and_then(Result::ok)
                        .map(|i| i.size(0))
                        .map(|s| (s.width.0 as f32, s.height.0 as f32));
                    let focus = f.match_boxes.first();
                    let frame = match (natural, focus) {
                        (Some((nw, nh)), Some(b)) => {
                            // Centre the first box at native pixels (a box wider than the crop centres on its start).
                            let clamp = |v: f32, lo: f32| v.max(lo).min(0.);
                            let cxp = b.x as f32 * nw + (b.w as f32 * nw).min(CROP_W - 40.) / 2.;
                            let cyp = (b.y as f32 + b.h as f32 / 2.) * nh;
                            let (ox, oy) = (
                                clamp(CROP_W / 2. - cxp, CROP_W - nw),
                                clamp(CROP_H / 2. - cyp, CROP_H - nh),
                            );
                            div()
                                .absolute()
                                .left(px(ox))
                                .top(px(oy))
                                .w(px(nw))
                                .h(px(nh))
                                .child(img(path).image_cache(&self.images).size_full())
                                .children(f.match_boxes.iter().map(|b| match_box(b, p)))
                        }
                        _ => {
                            // Contained: round it where it reaches the crop's corners,
                            // less its letterbox gap (GPUI clips to a rectangle).
                            let gap = natural.map_or(0., |(nw, nh)| {
                                let z = (CROP_W / nw).min(CROP_H / nh);
                                ((CROP_W - nw * z) / 2.).max((CROP_H - nh * z) / 2.)
                            });
                            div().absolute().top_0().left_0().size_full().child(
                                img(path)
                                    .image_cache(&self.images)
                                    .size_full()
                                    .rounded((radius::MD - px(gap)).max(px(0.)))
                                    .object_fit(ObjectFit::Contain),
                            )
                        }
                    };
                    el = el.child(frame);
                    if natural.is_some() && focus.is_some() {
                        // The zoomed frame overflows the crop and GPUI clips to a rectangle:
                        // paint what's behind over the corner wedges (wide enough to reach
                        // the corner, r·(√2−1); the part outside the crop is clipped).
                        el = el.child(crate::ui::ring_box(behind, radius::MD, radius::MD));
                    }
                }
                el.child(ring)
                    .child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .bottom(px(8.))
                            .px(px(6.))
                            .py(px(3.))
                            .rounded(radius::XS)
                            .bg(p.overlay_bg_strong)
                            .border_1()
                            .border_color(p.overlay_border)
                            .font_family(FONT_MONO)
                            .text_size(text::XS)
                            .font_weight(theme::W_MEDIUM)
                            .text_color(p.text_strong)
                            .child(start),
                    )
                    .into_any_element()
            }
            Ranked::Audio(a) => {
                let s = if a.mic() {
                    p.source_mic
                } else {
                    p.source_sysaudio
                };
                let seg_ms =
                    (ms(&a.audio_segment.ended_at) - ms(&a.audio_segment.started_at)).max(0);
                let from = (a.span_start_ms as i64 - 20_000).max(0);
                let end = a.span_end_ms as i64 + 20_000;
                let to = (from + 1).max(if seg_ms > 0 { seg_ms.min(end) } else { end });
                // shortcut: bars are seeded from the group key, not real peaks (as the Svelte crop).
                let bars = seeded_bars(&a.group_key, 44, 16, 85);
                let wave = div().flex().flex_1().items_center().gap(px(2.)).children(
                    bars.into_iter().enumerate().map(|(i, v)| {
                        let at = from as f32 + (i as f32 + 0.5) / 44. * (to - from) as f32;
                        let on = at >= a.span_start_ms as f32 && at <= a.span_end_ms as f32;
                        div()
                            .flex_1()
                            .h(relative(v as f32 / 100.))
                            .min_h(px(2.))
                            .rounded(px(1.))
                            .bg(if on { p.accent } else { alpha(p.fg, 0.18) })
                    }),
                );
                let clk = super::look::clock;
                base.child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .pt(px(12.))
                        .px(px(12.))
                        .pb(px(10.))
                        .rounded(radius::MD)
                        .bg(mix(s, p.surface_subtle, 0.07))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .font_family(FONT_MONO)
                                .text_size(text::XS)
                                .font_weight(theme::W_MEDIUM)
                                .tracking(0.02)
                                .text_color(s)
                                .child(icon(
                                    if a.mic() {
                                        IconName::Mic.path()
                                    } else {
                                        "icons/volume-2.svg".into()
                                    },
                                    px(12.),
                                ))
                                .child(format!(
                                    "{} · {}",
                                    if a.mic() {
                                        "microphone"
                                    } else {
                                        "system audio"
                                    },
                                    clk(seg_ms)
                                )),
                        )
                        .child(wave)
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .font_family(FONT_MONO)
                                .text_size(text::XS)
                                .text_color(p.text_subtle)
                                .child(clk(from))
                                .child(
                                    div()
                                        .text_color(p.accent)
                                        .font_weight(theme::W_MEDIUM)
                                        .child(clk(a.span_start_ms as i64)),
                                )
                                .child(clk(to)),
                        ),
                )
                .child(ring)
                .into_any_element()
            }
        }
    }

    fn best_moment(
        &mut self,
        r: &Ranked,
        selected: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> impl IntoElement {
        let now = self.now(cx);
        let (start, end) = match r {
            Ranked::Frame(f) => (ms(&f.group_start_at), ms(&f.group_end_at)),
            Ranked::Audio(a) => (ms(&a.absolute_start_at), ms(&a.absolute_end_at)),
        };
        let fmt = crate::chat::format::local;
        let (s, e) = (fmt(start, now), fmt(end, now));
        let clock = crate::chat::format::clock;
        let day = super::rules::day_comma(s.date());
        let range = super::rules::clock_range(&clock(s), &clock(e));
        let span = match r {
            Ranked::Frame(_) => format!(
                "{} min on screen",
                ((end - start) as f64 / 60_000.).round().max(1.)
            ),
            Ranked::Audio(a) => format!(
                "{} recording",
                super::look::clock(ms(&a.audio_segment.ended_at) - ms(&a.audio_segment.started_at))
            ),
        };
        let where_ = match r {
            Ranked::Frame(f) => f
                .url
                .as_ref()
                .and_then(|u| u.split('/').next().map(String::from))
                .or_else(|| f.app_name.clone())
                .unwrap_or_default(),
            Ranked::Audio(_) => String::new(),
        };
        let terms: Vec<String> = {
            let mut seen = Vec::new();
            for (t, m) in snippet_runs(r.snippet()) {
                let t = t.to_lowercase();
                if m && !seen.contains(&t) {
                    seen.push(t);
                }
            }
            seen
        };
        let count = match r {
            Ranked::Frame(f) => f.match_count,
            Ranked::Audio(a) => a.match_count,
        };
        let why = if r.meaning() {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(chip("by meaning", ChipKind::Neutral, p).h(px(18.)))
                .child("no exact words")
        } else {
            let words = terms
                .iter()
                .map(|t| format!("“{t}”"))
                .collect::<Vec<_>>()
                .join(" + ");
            let times = if count > 1 {
                format!(" ×{count}")
            } else {
                String::new()
            };
            let place = if matches!(r, Ranked::Frame(_)) {
                "on screen"
            } else {
                "in the transcript"
            };
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(chip("keyword", ChipKind::Accent, p).h(px(18.)))
                .child(format!("{words}{times} {place}"))
        };
        let r_look = r.clone();
        let page = r.page_frame();
        // The selected fill over the shell's `bg`, for the crop's corner cover.
        let solid = Hsla {
            a: 1.,
            ..p.wash_strong
        };
        let behind = if selected {
            mix(solid, p.bg, p.wash_strong.a)
        } else {
            p.bg
        };
        div()
            .id(("qr-opt", 0usize))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .flex()
            .gap(px(18.))
            .p(px(10.))
            .rounded(radius::LG)
            .when(selected, |d| d.bg(p.wash_strong))
            .child(self.crop(r, behind, window, cx, p))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(6.))
                    .pt(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(crate::ui::kicker("best moment", p))
                            .child(
                                div()
                                    .ml_auto()
                                    .font_family(FONT_MONO)
                                    .text_size(text::XS)
                                    .text_color(p.text_subtle)
                                    .child(span),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(space::S2)
                            .mt(px(2.))
                            .text_size(text::LG)
                            .font_weight(theme::W_SEMIBOLD)
                            .tracking(-0.012)
                            .text_color(p.text_strong)
                            .child(self.tile(r, p))
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(r.title()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_x(px(10.))
                            .font_family(FONT_MONO)
                            .text_size(text::SM)
                            .text_color(p.text_subtle)
                            .child(
                                div()
                                    .flex()
                                    .child(
                                        div()
                                            .text_color(p.text)
                                            .font_weight(theme::W_MEDIUM)
                                            .child(day),
                                    )
                                    .child(format!(" · {range}")),
                            )
                            .child(super::rules::relative_time(
                                start,
                                (now.unix_timestamp_nanos() / 1_000_000) as i64,
                            ))
                            .when(!where_.is_empty(), |d| d.child(where_)),
                    )
                    .child(
                        div()
                            .mt(px(2.))
                            .text_size(text::MD)
                            .line_height(relative(1.55))
                            .text_color(p.text_muted)
                            .line_clamp(2)
                            .child(marked(r.snippet(), matches!(r, Ranked::Audio(_)), p)),
                    )
                    .child(why.text_size(text::SM).text_color(p.text_subtle))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(4.))
                            .mt(px(6.))
                            .child(
                                Button::new("qr-best-open", "Open in Timeline")
                                    .sm()
                                    .when(selected, |b| b.primary())
                                    .icon("icons/chart-no-axes-gantt.svg")
                                    .kbd("↵")
                                    .on_click(act(cx, |qr, cx| qr.open_result_at(0, cx))),
                            )
                            .child(
                                Button::new("qr-best-look", "Quick Look")
                                    .sm()
                                    .ghost()
                                    .icon(IconName::Eye.path())
                                    .on_click(act(cx, move |qr, cx| {
                                        qr.select_result(0);
                                        qr.look_show(r_look.clone(), cx);
                                    })),
                            )
                            .when_some(page, |d, id| {
                                d.child(
                                    Button::new("qr-best-page", "Open page")
                                        .sm()
                                        .ghost()
                                        .icon(IconName::ExternalLink.path())
                                        .kbd("⌘↵")
                                        .on_click(act(cx, move |qr, cx| qr.open_page(id, cx))),
                                )
                            }),
                    ),
            )
            .on_click(cx.listener(|qr, _, _, cx| {
                qr.select_result(0);
                qr.refresh(cx);
            }))
    }
}

/// A match outline: solid accent = keyword, dashed = by meaning.
pub(super) fn match_box(b: &app_infra::MatchBox, p: &Palette) -> Div {
    let meaning = b.kind == app_infra::MatchBoxKind::Meaning;
    div()
        .absolute()
        .left(relative(b.x as f32))
        .top(relative(b.y as f32))
        .w(relative(b.w as f32))
        .h(relative(b.h as f32))
        .rounded(px(3.))
        .map(|d| {
            if meaning {
                d.bg(alpha(p.fg, 0.07))
                    .border_1()
                    .border_dashed()
                    .border_color(p.text_subtle)
            } else {
                d.bg(alpha(p.accent, 0.12))
                    .border(px(1.5))
                    .border_color(p.accent)
            }
        })
}

pub(super) fn source_glyph(mic: bool, p: &Palette) -> AnyElement {
    let c = if mic { p.source_mic } else { p.source_sysaudio };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(18.))
        .rounded(px(5.))
        .text_color(c)
        .bg(mix(c, p.surface, 0.18))
        .shadow(vec![inset(alpha(c, 0.3), px(1.))])
        .child(icon(
            if mic {
                IconName::Mic.path()
            } else {
                SharedString::from("icons/volume-2.svg")
            },
            size::ICON_XS,
        ))
        .into_any_element()
}

/// Decorative bar heights seeded from a key (the Svelte crops' Park–Miller run).
pub(super) fn seeded_bars(key: &str, n: usize, lo: u64, span: u64) -> Vec<u64> {
    let mut s: u64 = 0;
    for ch in key.encode_utf16() {
        s = (s.wrapping_mul(31) + ch as u64) & 0xffff_ffff;
    }
    s = (s % 2_147_483_646) + 1;
    (0..n)
        .map(|_| {
            s = (s * 16807) % 2_147_483_647;
            lo + s % span
        })
        .collect()
}

fn skeletons(p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .gap(px(18.))
                .p(px(10.))
                .child(
                    skeleton::skel(p)
                        .w(px(CROP_W))
                        .h(px(CROP_H))
                        .rounded(radius::MD),
                )
                .child(
                    div()
                        .flex_1()
                        .pt(px(4.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(skeleton::title(p))
                        .child(skeleton::line(0.6, p))
                        .child(skeleton::line(1., p))
                        .child(skeleton::line(0.4, p)),
                ),
        )
        .children((0..3).map(|_| {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .py(px(6.))
                .px(px(10.))
                .child(skeleton::skel(p).size(px(18.)))
                .child(div().flex_1().child(skeleton::line(1., p)))
                .child(skeleton::skel(p).w(px(56.)).h(px(10.)))
        }))
        .font_weight(FontWeight::NORMAL)
}
