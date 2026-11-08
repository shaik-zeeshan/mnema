//! `DrawerHeader`: the identity strip (source, index, clock, duration, model,
//! file), the live status pill and the chrome actions; under it the speaker strip
//! (one toggle chip per voice; a selection dims every other voice).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{
    canvas, div, point, px, AnyElement, Context, Div, ElementId, PathBuilder, Role, SharedString,
};

use super::view_model::{self as vm, Mark, Shape, Tone, TranscriptStatus};
use super::AudioDrawer;
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::timeline::model::{clock_label, local};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::{progress, with_tip, Button};

/// `SpeakerMark`: 12 px, colour AND shape (the accessibility half of the pair).
/// Unnamed voices are hollow.
pub fn mark(m: Option<&Mark>, ghosted: bool, p: &Palette) -> AnyElement {
    const SIZE: f32 = 12.;
    let c = m.map_or(p.text_subtle, |m| m.hue.color(p));
    let shape = m.map_or(Shape::Circle, |m| m.shape);
    let base = div().flex_none().size(px(SIZE));
    let el = match shape {
        Shape::Circle | Shape::Square => {
            let r = if shape == Shape::Circle {
                radius::PILL
            } else {
                px(1.)
            };
            let b = base.rounded(r);
            if ghosted {
                // CSS `2px dashed` at 12 px renders as a solid ring in WebKit; match that.
                b.border_2().border_color(c)
            } else {
                b.bg(c)
            }
        }
        // No clip-path in GPUI: the triangle / diamond are filled paths; the
        // CSS hatch for an unnamed one is drawn as a 1.5 px outline instead.
        Shape::Triangle | Shape::Diamond => base.child(
            canvas(
                |_, _, _| {},
                move |b, _, window, _| {
                    let (l, t, s) = (b.left(), b.top(), b.size.width);
                    let pts = if shape == Shape::Triangle {
                        vec![point(l + s / 2., t), point(l + s, t + s), point(l, t + s)]
                    } else {
                        let h = s / 2.;
                        vec![
                            point(l + h, t),
                            point(l + s, t + h),
                            point(l + h, t + s),
                            point(l, t + h),
                        ]
                    };
                    let mut path = if ghosted {
                        PathBuilder::stroke(px(1.5))
                    } else {
                        PathBuilder::fill()
                    };
                    path.add_polygon(&pts, true);
                    if let Ok(path) = path.build() {
                        window.paint_path(path, c);
                    }
                },
            )
            .size_full(),
        ),
    };
    el.into_any_element()
}

fn sep(p: &Palette) -> Div {
    div().text_color(p.text_faint).child("·")
}

pub fn render(d: &AudioDrawer, p: &Palette, cx: &mut Context<AudioDrawer>) -> AnyElement {
    let s = &d.segment;
    let offset = crate::ui::clock::now(cx).offset();
    let at = |ms| clock_label(local(ms, offset), true);
    let (tone, label, busy) = vm::status_pill(
        s.source,
        d.data.status,
        d.data.speakers_running,
        d.data.speakers_failed,
        d.strip.len(),
    );
    let pill_kind = match tone {
        Tone::Warn => ChipKind::Warn,
        Tone::Bad => ChipKind::Danger,
        _ => ChipKind::Neutral,
    };
    let status = d.data.status;
    let action = if status == TranscriptStatus::Missing {
        "run"
    } else {
        "rerun"
    };
    let action_disabled = d.rerun_loading
        || matches!(
            status,
            TranscriptStatus::Loading | TranscriptStatus::Running
        );
    let action_tip = match status {
        TranscriptStatus::Loading => "Transcript is still loading".to_string(),
        TranscriptStatus::Running => "Transcription is queued or still processing".to_string(),
        _ if s.source == super::model::Src::Sys => {
            format!(
                "{} speech detection and transcription with current settings",
                cap(action)
            )
        }
        _ => format!("{} transcription with current settings", cap(action)),
    };
    let file = with_tip(
        div()
            .id("drawer-file")
            .max_w(px(130.))
            .ml(space::S1)
            .overflow_hidden()
            .text_ellipsis()
            .whitespace_nowrap()
            .font_family(FONT_MONO)
            .text_size(text::XS)
            .font_weight(theme::W_REGULAR)
            .text_color(p.text_subtle)
            .child(s.file_name()),
        s.file_path.to_string_lossy().to_string(),
    );
    let ghost = |id: &'static str, label: &str| Button::new(id, label.to_string()).ghost().sm();
    let head = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(space::S2)
        .pl(px(14.))
        .pr(px(10.))
        .py(px(10.))
        .border_b_1()
        .border_color(p.hairline)
        .text_size(text::SM)
        .font_weight(theme::W_MEDIUM)
        .text_color(p.text_muted)
        .child(chip(
            s.source.label(),
            if s.source == super::model::Src::Sys {
                ChipKind::SysAudio
            } else {
                ChipKind::Mic
            },
            p,
        ))
        .child(
            div()
                .font_family(FONT_MONO)
                .child(format!("#{}", s.segment_index)),
        )
        .child(sep(p))
        .child(
            div()
                .font_family(FONT_MONO)
                .child(format!("{} → {}", at(s.start_ms), at(s.end_ms))),
        )
        .child(sep(p))
        .child(
            div()
                .font_family(FONT_MONO)
                .child(vm::player_time(s.duration_s().round())),
        )
        .children(d.data.model.as_ref().map(|m| {
            div()
                .flex()
                .items_center()
                .gap(space::S2)
                .child(sep(p))
                .child(with_tip(
                    div().id("drawer-model").child(m.label.clone()),
                    m.tip.clone(),
                ))
        }))
        .child(file)
        .child(div().flex_1())
        .child(
            chip(label, pill_kind, p)
                .id("drawer-status")
                .role(Role::Status)
                .when(busy, |c| c.child(progress::spinner(theme::size::SPIN_SM))),
        )
        .child(
            ghost(
                "drawer-rerun",
                if d.rerun_loading {
                    "starting…"
                } else {
                    action
                },
            )
            .disabled(action_disabled)
            .tip(action_tip)
            .on_click(cx.listener(|d, _, _, cx| d.rerun(cx))),
        )
        .child(
            ghost("drawer-timestamps", "timestamps")
                .pressed(d.show_timestamps)
                .on_click(cx.listener(|d, _, _, cx| {
                    d.show_timestamps = !d.show_timestamps;
                    cx.notify();
                })),
        )
        .child(
            ghost(
                "drawer-expand-toggle",
                if d.expanded { "collapse" } else { "expand" },
            )
            .pressed(d.expanded)
            .on_click(cx.listener(|d, _, _, cx| {
                d.expanded = !d.expanded;
                cx.notify();
            })),
        )
        .child(
            Button::icon_only("drawer-close", IconName::X.path(), "Close audio player")
                .ghost()
                .sm()
                .tip("Close  esc")
                .on_click(cx.listener(|d, _, _, cx| d.close(cx))),
        );
    let strip = (!d.strip.is_empty()).then(|| speaker_strip(d, p, cx));
    div()
        .flex()
        .flex_col()
        .child(head)
        .children(strip)
        .into_any_element()
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn speaker_strip(d: &AudioDrawer, p: &Palette, cx: &mut Context<AudioDrawer>) -> AnyElement {
    let has_sel = d.selected_cluster.is_some();
    let chips: Vec<AnyElement> = d
        .strip
        .iter()
        .map(|s| {
            let id = s.cluster_id;
            let pressed = d.selected_cluster == Some(id);
            let aria = format!(
                "{}{}, {} talk time",
                s.name,
                if s.state.is_empty() {
                    String::new()
                } else {
                    format!(", {}", s.state)
                },
                vm::compact_duration(s.talk_ms)
            );
            div()
                .id(ElementId::NamedInteger("drawer-speaker".into(), id as u64))
                .role(Role::Button)
                .aria_label(SharedString::from(aria))
                .flex()
                .items_center()
                .gap(px(7.))
                .h(theme::size::H_SM)
                .px(px(9.))
                .rounded(radius::SM)
                .cursor_pointer()
                .when(pressed, |el| el.bg(p.wash_strong))
                .hover(|el| el.bg(p.wash))
                .text_size(text::BASE)
                .font_weight(theme::W_MEDIUM)
                .text_color(if s.unnamed { p.text_muted } else { p.text })
                .child(mark(d.marks.get(&id), s.unnamed, p))
                .child(s.name.clone())
                .when(!s.state.is_empty(), |el| {
                    el.child(
                        div()
                            .font_weight(theme::W_REGULAR)
                            .text_color(p.text_subtle)
                            .child(s.state.clone()),
                    )
                })
                .child(
                    div()
                        .font_family(FONT_MONO)
                        .text_size(text::SM)
                        .font_weight(theme::W_REGULAR)
                        .text_color(p.text_subtle)
                        .child(format!("{}%", s.share_pct)),
                )
                .on_click(cx.listener(move |d, _, _, cx| {
                    d.selected_cluster = (d.selected_cluster != Some(id)).then_some(id);
                    if d.selected_cluster.is_none() {
                        d.only_selected = false;
                    }
                    cx.notify();
                }))
                .into_any_element()
        })
        .collect();
    div()
        .id("drawer-speakers")
        .role(Role::Toolbar)
        .aria_label("Speakers in this segment")
        .flex()
        .items_center()
        .gap(px(2.))
        .min_w_0()
        .pl(px(14.))
        .pr(px(10.))
        .py(px(5.))
        .border_b_1()
        .border_color(p.hairline)
        .child(
            div()
                .mr(space::S2)
                .text_size(text::BASE)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_subtle)
                .child("Speakers"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .min_w_0()
                .overflow_hidden()
                .children(chips),
        )
        .child(div().flex_1())
        .child(
            Button::new("drawer-only-voice", "only this voice")
                .ghost()
                .sm()
                .pressed(d.only_selected && has_sel)
                .disabled(!has_sel)
                .on_click(cx.listener(|d, _, _, cx| {
                    d.only_selected = !d.only_selected;
                    cx.notify();
                })),
        )
        .into_any_element()
}
