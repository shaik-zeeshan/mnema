//! `TranscriptReader`: one row per speaker turn (a speaker column beside the
//! prose), word-level karaoke while playing, and follow mode (a wheel in the
//! reader detaches it; "jump to playhead" or Esc re-attaches).
//!
//! Karaoke is one `StyledText` per paragraph with highlight ranges, so a frame
//! re-render costs a text layout per paragraph, not a box per word. With no word
//! timings the whole paragraph is the seek target instead.

use std::ops::Range;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyElement, Context, ElementId, HighlightStyle, InteractiveText, Role, SharedString,
    StyledText,
};

use super::header::mark;
use super::view_model::{self as vm, Group};
use super::AudioDrawer;
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::timeline::model::{clock_label, local};
use crate::ui::{alpha, Button};

/// The speaker column and the repair slide-over's reserved width.
const GUTTER_W: f32 = 176.;
pub const REPAIR_W: f32 = 352.;
/// Everyone but the selected voice recedes to this.
const RECEDE: f32 = 0.38;

fn words_text(words: &[crate::timeline::audio::model::Run]) -> (String, Vec<Range<usize>>) {
    let mut s = String::new();
    let mut ranges = Vec::with_capacity(words.len());
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let start = s.len();
        s.push_str(&w.text);
        ranges.push(start..s.len());
    }
    (s, ranges)
}

fn gutter(
    d: &AudioDrawer,
    ix: usize,
    g: &Group,
    stacked: bool,
    p: &Palette,
    cx: &mut Context<AudioDrawer>,
) -> AnyElement {
    let profiles = &d.data.profiles;
    let unnamed = vm::is_unnamed(g);
    let repairable = g.cluster_id >= 0;
    let name = vm::persisted_name(g, profiles);
    let m = d.marks.get(&g.cluster_id);
    let chip = (!d.soft_dismissed.contains(&g.cluster_id))
        .then(|| vm::suggestion_chip(&d.groups, ix, profiles, d.developer))
        .flatten();
    let needs = repairable && (g.person_id.is_none() || g.suggested_person_id.is_some());
    let who = if repairable {
        let tip = name.clone();
        let el = div()
            .id(ElementId::NamedInteger("drawer-who".into(), ix as u64))
            .role(Role::Button)
            .aria_label(SharedString::from(format!("Repair speaker {name}")))
            .group("who")
            .flex()
            .items_center()
            .gap(px(7.))
            .max_w_full()
            .ml(px(-9.))
            .h(theme::size::H_SM)
            .px(px(9.))
            .rounded(radius::BTN_SM)
            .cursor_pointer()
            .hover(|el| el.bg(p.wash_strong).text_color(p.text_strong))
            .text_size(text::BASE)
            .font_weight(theme::W_MEDIUM)
            .text_color(if unnamed { p.text_muted } else { p.text_strong })
            .child(mark(m, unnamed, p))
            .child(
                div()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(name),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(p.text_subtle)
                    .when(!needs, |el| {
                        el.invisible().group_hover("who", |s| s.visible())
                    })
                    .child(crate::ui::icon(
                        IconName::Ellipsis.path(),
                        theme::size::ICON_SM,
                    )),
            )
            .on_click(cx.listener(move |d, _, window, cx| d.open_repair(ix, window, cx)));
        crate::ui::with_tip(el, tip).into_any_element()
    } else {
        div()
            .flex()
            .items_center()
            .gap(px(7.))
            .text_size(text::MD)
            .font_weight(theme::W_MEDIUM)
            .text_color(if unnamed { p.text_muted } else { p.text_strong })
            .child(mark(m, unnamed, p))
            .child(name)
            .into_any_element()
    };
    let note = if unnamed && repairable && chip.is_none() {
        Some("name this voice")
    } else if g.overlaps {
        Some("overlapping speech")
    } else {
        None
    };
    let busy = d.busy_cluster.is_some();
    let cluster = g.cluster_id;
    let suggestion = chip.map(|(name, meta)| {
        let first = name.split(' ').next().unwrap_or(&name).to_string();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(2.))
            .text_size(text::SM)
            .text_color(p.text_muted)
            .child("maybe ")
            .child(
                div()
                    .font_weight(theme::W_MEDIUM)
                    .text_color(p.text)
                    .child(first),
            )
            .children(meta.map(|m| {
                div()
                    .mr(px(2.))
                    .font_family(FONT_MONO)
                    .text_color(p.text_subtle)
                    .child(format!(" · {m}"))
            }))
            .child(
                Button::icon_only(
                    ElementId::NamedInteger("drawer-confirm".into(), ix as u64),
                    IconName::Check.path(),
                    format!("Confirm {name} — links this voice and saves a sample"),
                )
                .ghost()
                .sm()
                .disabled(busy)
                .tip("Confirm — links + saves a sample")
                .on_click(cx.listener(move |d, _, _, cx| {
                    d.write(cluster, super::data::Write::Confirm { cluster }, false, cx)
                })),
            )
            .child(
                Button::icon_only(
                    ElementId::NamedInteger("drawer-hide".into(), ix as u64),
                    IconName::X.path(),
                    "Hide this suggestion — changes nothing, ask me later",
                )
                .ghost()
                .sm()
                .tip("Hide — writes nothing")
                .on_click(cx.listener(move |d, _, _, cx| {
                    d.soft_dismissed.push(cluster);
                    cx.notify();
                })),
            )
    });
    let offset = crate::ui::clock::now(cx).offset();
    div()
        .flex_none()
        .map(|el| {
            if stacked {
                el.w_full()
            } else {
                el.w(px(GUTTER_W))
            }
        })
        .min_w_0()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(6.))
        .child(who)
        .children(note.map(|n| div().text_size(text::SM).text_color(p.text_muted).child(n)))
        .children(suggestion)
        .when(d.show_timestamps, |el| {
            el.child(
                div()
                    .font_family(FONT_MONO)
                    .text_size(text::XS)
                    .text_color(p.text_subtle)
                    .child(clock_label(
                        local(d.segment.start_ms + g.start_ms, offset),
                        true,
                    )),
            )
        })
        .into_any_element()
}

fn prose(
    d: &AudioDrawer,
    ix: usize,
    g: &Group,
    active: bool,
    playing: bool,
    p: &Palette,
    cx: &mut Context<AudioDrawer>,
) -> AnyElement {
    let now = d.transport.current_ms();
    let base = div()
        .flex_1()
        .min_w_0()
        .text_size(text::BASE)
        .line_height(px(12. * 1.65));
    let this = cx.entity().downgrade();
    match d.karaoke.get(ix).cloned().flatten() {
        Some(words) => {
            let (s, ranges) = words_text(&words);
            let current = active.then(|| vm::active_word(&words, now)).flatten();
            let mut hl: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
            if playing {
                for (wi, (w, r)) in words.iter().zip(&ranges).enumerate() {
                    let style = if Some(wi) == current {
                        HighlightStyle {
                            color: Some(p.accent),
                            background_color: Some(p.accent_bg),
                            ..Default::default()
                        }
                    } else if w.end_ms <= now {
                        HighlightStyle {
                            color: Some(p.text),
                            ..Default::default()
                        }
                    } else {
                        continue;
                    };
                    hl.push((r.clone(), style));
                }
            }
            let (gs, ge) = (g.start_ms, g.end_ms);
            let text = InteractiveText::new(
                ElementId::NamedInteger("drawer-para".into(), ix as u64),
                StyledText::new(s).with_highlights(hl),
            )
            .on_click(ranges, move |wi, _, cx| {
                let Some(w) = words.get(wi) else { return };
                let ms = vm::clamp_seek(w.start_ms, gs, ge);
                this.update(cx, |d, cx| {
                    d.seek_to_ms(ms);
                    cx.notify();
                })
                .ok();
            });
            // Hybrid: only while playing do the unplayed words dim.
            base.text_color(if playing { p.text_subtle } else { p.text })
                .child(text)
                .into_any_element()
        }
        None => {
            let seek = vm::segment_seek(&d.data.runs, g);
            // `formatTranscriptSegmentTitle`: "0:12–0:31", or the start alone.
            let mut range = vm::player_time(g.start_ms as f64 / 1000.);
            if g.end_ms > g.start_ms {
                range = format!("{range}–{}", vm::player_time(g.end_ms as f64 / 1000.));
            }
            base.child(crate::ui::with_tip(
                div()
                    .id(ElementId::NamedInteger("drawer-para".into(), ix as u64))
                    .role(Role::Button)
                    .aria_label(SharedString::from(format!(
                        "Jump to {}",
                        vm::player_time(g.start_ms as f64 / 1000.)
                    )))
                    .cursor_pointer()
                    .text_color(if active { p.accent } else { p.text })
                    .hover(|el| el.text_color(p.text_strong))
                    .child(g.text.clone())
                    .on_click(cx.listener(move |d, _, _, cx| {
                        d.seek_to_ms(seek);
                        cx.notify();
                    })),
                format!("Jump to {range}"),
            ))
            .into_any_element()
        }
    }
}

/// `stacked`: the window is ≤820 px wide, so each turn puts its speaker over
/// its text (`TranscriptReader.svelte`'s one-column `.turn`).
pub fn render(
    d: &AudioDrawer,
    stacked: bool,
    p: &Palette,
    cx: &mut Context<AudioDrawer>,
) -> AnyElement {
    let active = d.active_group();
    let playing = active.is_some();
    let sel = d.selected_cluster;
    let fixing = d.repair.as_ref().map(|r| r.index);
    let rows: Vec<AnyElement> = d
        .groups
        .iter()
        .enumerate()
        .filter(|(_, g)| !(d.only_selected && sel.is_some() && Some(g.cluster_id) != sel))
        .map(|(ix, g)| {
            let tint = d
                .marks
                .get(&g.cluster_id)
                .map_or(p.text_subtle, |m| m.hue.color(p));
            let is_active = active == Some(ix);
            div()
                .flex()
                .map(|el| {
                    if stacked {
                        el.flex_col().gap(px(4.))
                    } else {
                        el.gap(space::S5)
                    }
                })
                .pl(px(18.))
                .pr(px(22.))
                .py(space::S3)
                .when(ix > 0, |el| el.border_t_1().border_color(p.hairline))
                .when(is_active, |el| el.bg(alpha(tint, 0.08)))
                .when(sel.is_some_and(|s| s != g.cluster_id), |el| {
                    el.opacity(RECEDE)
                })
                .when(sel == Some(g.cluster_id) && !is_active, |el| el.bg(p.wash))
                .when(fixing == Some(ix), |el| el.bg(p.wash_strong))
                .child(gutter(d, ix, g, stacked, p, cx))
                .map(|el| {
                    let text = prose(d, ix, g, is_active, playing, p, cx);
                    // `.turn--overlap .para`: a 2 px rule 16 px left of the text.
                    let overlap = |w: gpui_kit::Div| {
                        w.when(g.overlaps, |w| {
                            w.ml(px(-16.))
                                .pl(px(14.))
                                .border_l_2()
                                .border_color(p.warn_border)
                        })
                    };
                    // Stacked, the text's `flex_1` must not grow along the column.
                    if stacked {
                        el.child(overlap(div().w_full()).child(text))
                    } else if g.overlaps {
                        el.child(overlap(div().flex_1().min_w_0()).child(text))
                    } else {
                        el.child(text)
                    }
                })
                .into_any_element()
        })
        .collect();
    let note = if let Some(e) = &d.data.speakers_error {
        Some((e.clone(), true))
    } else {
        d.data.speakers_notice.clone().map(|n| (n, false))
    };
    let correction = d.correction_error.clone().filter(|_| d.repair.is_none());
    let foot = |msg: String, error: bool| {
        div()
            .px(px(14.))
            .pt(space::S1)
            .pb(space::S2)
            .text_size(text::SM)
            .when(error, |el| {
                el.font_family(FONT_MONO).text_color(p.danger_text)
            })
            .when(!error, |el| el.italic().text_color(p.text_muted))
            .child(msg)
    };
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .when(d.repair.is_some(), |el| el.pr(px(REPAIR_W)))
        .child(
            div()
                .id("drawer-reader")
                .role(Role::List)
                .flex_1()
                .min_h_0()
                .py(space::S1)
                .overflow_y_scroll()
                .track_scroll(&d.scroll)
                .on_scroll(cx.listener(|d, _, _, cx| {
                    if !d.follow_detached {
                        d.follow_detached = true;
                        cx.notify();
                    }
                }))
                .children(rows)
                .child(crate::ui::scroll::bar(&d.scroll)),
        )
        .children(note.map(|(m, e)| foot(m, e)))
        .children(correction.map(|m| foot(m, true)))
        .when(d.follow_detached, |el| {
            el.child(
                div()
                    .absolute()
                    .bottom(px(10.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    // `.jump`: the popover shadow, on a box hugging the button.
                    .child(
                        div()
                            .rounded(radius::BTN_SM)
                            .shadow(crate::ui::popover::popover_shadow(p))
                            .child(
                                Button::new("drawer-jump", "jump to playhead")
                                    .sm()
                                    .icon(IconName::ArrowDown.path())
                                    .kbd("esc")
                                    .on_click(cx.listener(|d, _, _, cx| {
                                        d.follow_detached = false;
                                        d.reattach();
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
        })
        .into_any_element()
}
