//! Change settings › Models (`ModelPickers.svelte`): per picker a build group,
//! the family's note and a Model / Download / Memory strip; then the read-only
//! budget (total, stacked bar with the free-space marker, legend) and the
//! over-disk notice. Every figure comes from `onboarding::budget`.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, Hsla, Pixels, Window};

use super::change::{kicker_note, provider_id};
use super::{button, on_ix};
use crate::onboarding::budget::{self, Budget, DiskVerdict, Pick};
use crate::onboarding::flow::Feature;
use crate::onboarding::parts::{self, fine};
use crate::onboarding::Onboarding;
use crate::settings::model::format_bytes;
use crate::theme::{self, leading, radius, size, space, text, Palette, FONT_MONO, FONT_UI};
use crate::ui::baseline::{self, Line};
use crate::ui::notice::notice;
use crate::ui::segmented::Segmented;
use crate::ui::Tracking as _;
use crate::ui::{alpha, kicker, text_width, Tone};

/// The big download total beside its fine print (`.ob-total b`, 22 px).
const TOTAL: Pixels = px(22.);
/// `.ob-strip div`: 10 px block padding, 6 px between label and value.
const CELL_PAD_Y: Pixels = px(10.);
const CELL_GAP: Pixels = px(6.);
/// `.ob-bar-stack`: 6 px tall, 3 px corners, 2 px between parts.
const BAR_H: Pixels = px(6.);
const BAR_GAP: Pixels = px(2.);
/// `.ob-room`: 14 px label row 6 px under the bar; the marker is 2 × 16 px.
const ROOM_H: Pixels = px(14.);
const MARKER_W: Pixels = px(2.);
const MARKER_H: Pixels = px(16.);
/// `.ob-legend li::before`: an 8 px swatch, 6 px from its label.
const SWATCH: Pixels = px(8.);
const LEGEND_GAP: Pixels = px(6.);

/// `(model id, display name, approx bytes, installed, multilingual)`.
type SemanticModel = (String, String, u64, bool, bool);

/// Indices into `semantic` of one language group, in catalog order.
fn family_builds(semantic: &[SemanticModel], multilingual: bool) -> Vec<usize> {
    (0..semantic.len())
        .filter(|&i| semantic[i].4 == multilingual)
        .collect()
}

/// `.ob-strip`: three cells, `1.6fr 1fr 1fr`, hairline-separated.
fn strip(cells: [(&str, String, bool); 3], p: &Palette) -> AnyElement {
    div()
        .flex()
        .w_full()
        .border_1()
        .border_color(p.border)
        .rounded(radius::MD)
        .bg(p.surface)
        .children(
            cells
                .into_iter()
                .enumerate()
                .map(|(i, (label, value, num))| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(CELL_GAP)
                        .min_w_0()
                        .w(relative(if i == 0 { 1.6 / 3.6 } else { 1. / 3.6 }))
                        .py(CELL_PAD_Y)
                        .px(space::S3)
                        .when(i > 0, |d| d.border_l_1().border_color(p.hairline))
                        .child(
                            div()
                                .font_family(FONT_MONO)
                                .text_size(text::XS)
                                .font_weight(theme::W_MEDIUM)
                                .tracking(0.1)
                                .line_height(relative(1.))
                                .text_color(p.text_subtle)
                                .child(label.to_uppercase()),
                        )
                        .child(
                            div()
                                .font_family(if num { FONT_MONO } else { FONT_UI })
                                .font_weight(if num {
                                    theme::W_REGULAR
                                } else {
                                    theme::W_MEDIUM
                                })
                                .text_size(text::MD)
                                .when(num, |d| d.tracking(-0.01))
                                .line_height(relative(1.2))
                                .text_color(p.text_strong)
                                .child(value),
                        )
                }),
        )
        .into_any_element()
}

/// `.ob-picker__ctl`: the build group(s) and the family's note, height-reserved.
fn ctl(groups: Vec<AnyElement>, note: &'static str, p: &Palette) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(space::S3)
        .min_h(size::H_SM)
        .children(groups)
        .child(fine(note, p))
        .into_any_element()
}

fn legend_item(label: &str, value: String, c: Hsla, p: &Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(LEGEND_GAP)
        .text_size(text::BASE)
        .line_height(relative(1.))
        .text_color(p.text_muted)
        .child(div().size(SWATCH).rounded(px(2.)).bg(c))
        .child(label.to_string())
        .child(
            div()
                .font_family(FONT_MONO)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_strong)
                .child(value),
        )
        .into_any_element()
}

impl Onboarding {
    pub fn set_transcription_model(&mut self, ix: usize, cx: &mut Context<Self>) {
        let id = provider_id(self.draft.transcription.provider);
        if let Some(m) = self
            .engines
            .iter()
            .find(|e| e.provider == id)
            .and_then(|e| e.models.get(ix))
        {
            self.draft.transcription.model_id = m.0.clone();
            self.refresh_facts();
            cx.notify();
        }
    }

    /// Picks the first build in the English or Multilingual group.
    pub fn set_semantic_family(&mut self, multilingual: bool, cx: &mut Context<Self>) {
        if let Some(&ix) = family_builds(&self.semantic, multilingual).first() {
            self.set_semantic_model(ix, cx);
        }
    }

    pub fn set_semantic_model(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(m) = self.semantic.get(ix) {
            self.draft.semantic_search.model_id = Some(m.0.clone());
            self.refresh_facts();
            cx.notify();
        }
    }

    /// The Models section's figures: both picks (semantic `None` while the
    /// feature is off) and the budget they add up to.
    pub fn models_budget(&self) -> (Option<Pick>, Option<Pick>, Budget) {
        let provider = provider_id(self.draft.transcription.provider);
        let t = self
            .engines
            .iter()
            .find(|e| e.provider == provider)
            .and_then(|e| {
                e.models
                    .iter()
                    .find(|m| m.0 == self.draft.transcription.model_id)
                    .or(e.models.first())
            })
            .map(|m| budget::transcription_pick(provider, m));
        let s = self
            .semantic
            .iter()
            .find(|m| Some(&m.0) == self.draft.semantic_search.model_id.as_ref())
            .filter(|_| self.features.semantic_search)
            .map(budget::semantic_pick);
        let speaker = self
            .models
            .iter()
            .find(|(sub, _)| *sub == crate::onboarding::flow::Subsystem::Speaker)
            .map(|(_, m)| m);
        let f = &self.features;
        let b = Budget {
            speaker: speaker
                .filter(|m| f.speaker_separation && !m.installed)
                .map_or(0, |m| m.bytes),
            transcription: budget::pick_bytes(t.as_ref(), f.transcription),
            semantic: budget::pick_bytes(s.as_ref(), f.semantic_search),
            semantic_approx: s.as_ref().is_some_and(|p| p.approx),
        };
        (t, s, b)
    }

    fn speaker_installed(&self) -> bool {
        self.models
            .iter()
            .any(|(sub, m)| *sub == crate::onboarding::flow::Subsystem::Speaker && m.installed)
    }

    /// What "Download anyway" keys its dismissal on: a new figure re-asks.
    fn budget_key(&self, b: &Budget) -> (u64, Option<f64>, f64, f64) {
        let free = self.probe.as_ref().and_then(|p| p.free);
        (b.bytes(), free, self.interval_s(), self.pixels())
    }

    /// The over-disk verdict for the current picks (drives the bar's hatch).
    pub fn budget_verdict(&self) -> Option<DiskVerdict> {
        let (_, _, b) = self.models_budget();
        let free = self.probe.as_ref().and_then(|p| p.free);
        budget::disk_verdict(&b, free, self.interval_s(), self.pixels())
    }

    /// The verdict the notice shows: hidden once dismissed at these figures.
    pub fn budget_notice(&self) -> Option<DiskVerdict> {
        let (_, _, b) = self.models_budget();
        self.budget_verdict()
            .filter(|_| self.budget_dismissed != Some(self.budget_key(&b)))
    }

    pub fn dismiss_budget(&mut self, cx: &mut Context<Self>) {
        let (_, _, b) = self.models_budget();
        self.budget_dismissed = Some(self.budget_key(&b));
        cx.notify();
    }

    pub(super) fn models_pane(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> Vec<AnyElement> {
        let provider = provider_id(self.draft.transcription.provider);
        let engine = self.engines.iter().find(|e| e.provider == provider);
        let (t_pick, s_pick, b) = self.models_budget();
        let f = &self.features;

        // ── Transcription ──
        let t_labels: Vec<String> = engine.map_or(vec![], |e| {
            e.models
                .iter()
                .map(|m| budget::short_label(m.0.as_deref().unwrap_or(""), &m.1))
                .collect()
        });
        let t_sel = engine
            .and_then(|e| {
                e.models
                    .iter()
                    .position(|m| m.0 == self.draft.transcription.model_id)
            })
            .unwrap_or(0);
        let t_group = (t_labels.len() > 1).then(|| {
            Segmented::new("ob-t-model", "Transcription build", t_labels, t_sel)
                .on_change(on_ix(cx, |this, ix, cx| {
                    this.set_transcription_model(ix, cx)
                }))
                .into_any_element()
        });
        let transcription = div()
            .flex()
            .flex_col()
            .gap(space::S3)
            .child(kicker_note(
                "Transcription",
                fine(
                    format!(
                        "Engine: {} \u{2014} chosen on Engines. This screen picks its build.",
                        engine.map_or("Whisper", |e| e.name.as_str())
                    ),
                    p,
                ),
                p,
                cx,
            ))
            .child(ctl(
                t_group.into_iter().collect(),
                budget::family_note(provider),
                p,
            ))
            .child(strip(
                [
                    (
                        "Model",
                        t_pick.as_ref().map_or("None".into(), |m| m.name.clone()),
                        false,
                    ),
                    ("Download", budget::size_cell(t_pick.as_ref(), ""), true),
                    (
                        "Memory",
                        t_pick.as_ref().map_or("\u{2014}", |m| m.detail).into(),
                        true,
                    ),
                ],
                p,
            ));

        // ── Semantic Search ──
        let s_sel = self
            .semantic
            .iter()
            .position(|m| Some(&m.0) == self.draft.semantic_search.model_id.as_ref())
            .unwrap_or(0);
        let mut s_groups = Vec::new();
        if !self.semantic.is_empty() {
            // A language group, then that group's builds under short labels.
            let multi = self.semantic.get(s_sel).is_some_and(|m| m.4);
            let builds = family_builds(&self.semantic, multi);
            let at = builds.iter().position(|&i| i == s_sel).unwrap_or(0);
            let labels: Vec<String> = builds
                .iter()
                .map(|&i| budget::short_label(&self.semantic[i].0, &self.semantic[i].1))
                .collect();
            s_groups.push(
                Segmented::new(
                    "ob-s-family",
                    "Semantic Search languages",
                    ["English", "Multilingual"],
                    multi as usize,
                )
                .on_change(on_ix(cx, |this, ix, cx| {
                    this.set_semantic_family(ix == 1, cx)
                }))
                .into_any_element(),
            );
            if builds.len() > 1 {
                s_groups.push(
                    Segmented::new("ob-s-model", "Semantic Search model", labels, at)
                        .on_change(on_ix(cx, move |this, ix, cx| {
                            if let Some(&i) = builds.get(ix) {
                                this.set_semantic_model(i, cx)
                            }
                        }))
                        .into_any_element(),
                );
            }
        }
        let s_family = match (&s_pick, self.semantic.get(s_sel)) {
            (Some(_), Some(m)) if m.4 => budget::SEMANTIC_MULTILINGUAL,
            (Some(_), _) => budget::SEMANTIC_ENGLISH,
            (None, _) => budget::SEMANTIC_OFF,
        };
        let semantic = div()
            .flex()
            .flex_col()
            .gap(space::S3)
            .child(kicker("Semantic Search", p))
            .child(ctl(s_groups, budget::family_note(s_family), p))
            .child(strip(
                [
                    (
                        "Model",
                        s_pick.as_ref().map_or("Off".into(), |m| m.name.clone()),
                        false,
                    ),
                    (
                        "Download",
                        budget::size_cell(s_pick.as_ref(), "about "),
                        true,
                    ),
                    (
                        "Languages",
                        s_pick
                            .as_ref()
                            .map_or("keyword search only", |m| m.detail)
                            .into(),
                        false,
                    ),
                ],
                p,
            ));

        // ── The budget ──
        let free = self.probe.as_ref().and_then(|p| p.free);
        let (interval, pixels) = (self.interval_s(), self.pixels());
        let verdict = self.budget_verdict();
        let shown = self.budget_notice();
        let room = budget::room_for_downloads(free, interval, pixels);
        // One axis for bytes AND room; 6% headroom keeps the marker off the edge.
        let axis = (b.bytes() as f64).max(room.unwrap_or(0.)).max(1.) * 1.06;
        let frac = |bytes: f64| (bytes / axis) as f32;
        let room_frac = room.map_or(0., |r| frac(r).min(1.));
        let total = baseline::row(vec![
            (
                Line::new(FONT_MONO, TOTAL, leading::ONE),
                parts::mono(
                    budget::total_label(b.bytes(), b.approx()),
                    TOTAL,
                    if shown.is_some() {
                        p.danger_text
                    } else {
                        p.text_strong
                    },
                )
                .tracking(-0.04)
                .into_any_element(),
            ),
            (
                Line::new(FONT_UI, text::BASE, parts::FINE_LH),
                fine("to download before any of this works", p).into_any_element(),
            ),
        ])
        .gap(space::S3);
        let parts_bar = [
            (b.speaker, p.source_mic),
            (b.transcription, p.chart_grey_4),
            (b.semantic, p.info),
        ];
        let bar = div()
            .relative()
            .flex()
            .gap(BAR_GAP)
            .h(BAR_H)
            .rounded(BAR_H / 2.)
            .overflow_hidden()
            .bg(p.surface_hover)
            .children({
                // GPUI clips to a rectangle: the end segments round themselves.
                let parts: Vec<_> = parts_bar
                    .into_iter()
                    .filter(|(bytes, _)| *bytes > 0)
                    .collect();
                let last = parts.len().saturating_sub(1);
                parts.into_iter().enumerate().map(move |(i, (bytes, c))| {
                    div()
                        .h_full()
                        .w(relative(frac(bytes as f64)))
                        .bg(c)
                        .when(i == 0, |d| d.rounded_l(BAR_H / 2.))
                        .when(i == last, |d| d.rounded_r(BAR_H / 2.))
                })
            })
            // Past the marker the downloads don't fit. shortcut: a flat tint, not
            // the CSS 135° hatch; draw stripes on a canvas if the founder asks.
            .when(verdict.is_some(), |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .left(relative(room_frac))
                        .bg(alpha(p.danger_border, 0.5)),
                )
            });
        let room_row = room.map(|r| {
            let label = if r > 0. {
                format!("{} left for downloads", format_bytes(r))
            } else {
                "no room left for downloads".into()
            };
            // Centred on the marker, except near either end (it would clip).
            let lw = text_width(&label, FONT_MONO, theme::W_REGULAR, text::SM, 0., window);
            let shift = if room_frac > 0.78 {
                lw
            } else if room_frac < 0.12 {
                px(0.)
            } else {
                lw / 2.
            };
            div()
                .relative()
                .h(ROOM_H)
                .mt(CELL_GAP)
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .line_height(ROOM_H)
                .text_color(p.text_subtle)
                .child(
                    div()
                        .absolute()
                        .top(-MARKER_H)
                        .left(relative(room_frac))
                        .ml(-MARKER_W / 2.)
                        .w(MARKER_W)
                        .h(MARKER_H)
                        .rounded(MARKER_W / 2.)
                        .bg(p.text_muted),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(2.))
                        .left(relative(room_frac))
                        .ml(-shift)
                        .whitespace_nowrap()
                        .child(label),
                )
        });
        let off = |on: bool, v: String| if on { v } else { "off".into() };
        let speaker_value = if !f.speaker_separation {
            "off".into()
        } else if self.speaker_installed() {
            "already on this Mac".into()
        } else {
            format_bytes(b.speaker as f64)
        };
        let legend = div()
            .flex()
            .flex_wrap()
            .gap_x(space::S5)
            .gap_y(space::S2)
            .child(legend_item(
                "Who's speaking",
                speaker_value,
                p.source_mic,
                p,
            ))
            .child(legend_item(
                "Transcription",
                off(f.transcription, budget::size_cell(t_pick.as_ref(), "")),
                p.chart_grey_4,
                p,
            ))
            .child(legend_item(
                "Semantic Search",
                off(
                    f.semantic_search,
                    budget::size_cell(s_pick.as_ref(), "about "),
                ),
                p.info,
                p,
            ));
        let totals = div()
            .flex()
            .flex_col()
            .gap(space::S2)
            .child(total)
            .child(div().child(bar).children(room_row))
            .child(legend);

        let mut col = div()
            .flex()
            .flex_col()
            .gap(space::S4)
            .child(transcription)
            .child(div().h(px(1.)).bg(p.hairline))
            .child(semantic)
            .child(totals);
        if let Some(v) = shown {
            let mut acts = Vec::new();
            if v.escape.is_some() {
                acts.push(
                    button(
                        "ob-budget-off",
                        "Turn Semantic Search off",
                        cx,
                        |this, _, cx| this.toggle(Feature::SemanticSearch, cx),
                    )
                    .sm()
                    .into_any_element(),
                );
            }
            acts.push(
                button("ob-budget-anyway", "Download anyway", cx, |this, _, cx| {
                    this.dismiss_budget(cx)
                })
                .ghost()
                .sm()
                .into_any_element(),
            );
            col = col.child(notice(
                Tone::Danger,
                "icons/hard-drive.svg",
                v.message,
                None,
                acts,
                p,
            ));
        }
        vec![col.into_any_element()]
    }
}
