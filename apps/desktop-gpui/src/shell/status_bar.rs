//! The 30 px status bar (`.mx-statusbar`): REC pill + clock, pause/stop or
//! Record, the three source lanes (toggles while stopped, the status popover's
//! trigger while recording), one flag, the engine pill only when unhealthy,
//! today's time, storage, "local only" and the shortcuts `?`.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, Context, Div, Hsla, MouseButton, Role, SharedString, Window};

use super::model::{self, Lane, LaneMod, RecMod, Tone};
use super::{Overlay, Shell};
use crate::keymap::{ToggleSourceMicrophone, ToggleSourceScreen, ToggleSourceSystemAudio};
use crate::theme::{self, breakpoint, radius, size, space, text, Palette, FONT_MONO, FONT_UI};
use crate::ui::{clock, icon, icon_button, sep, with_tip};

fn lane_color(lane: Lane, p: &Palette) -> Hsla {
    match lane {
        Lane::Screen => p.source_screen,
        Lane::Microphone => p.source_mic,
        Lane::SystemAudio => p.source_sysaudio,
    }
}

/// `.mx-lane`: a 5px mark (35% unless lit) then the label.
fn lane(lane: Lane, kind: LaneMod, small: Option<&'static str>, p: &Palette) -> Div {
    let (fg, mark) = match kind {
        LaneMod::On => (lane_color(lane, p), 1.),
        LaneMod::Paused => (p.status_paused_fg, 1.),
        LaneMod::Starting | LaneMod::Waiting => (p.text_muted, 1.),
        LaneMod::Error => (p.danger_text, 1.),
        LaneMod::Off => (p.text_subtle, 0.35),
    };
    let mark = div()
        .flex_none()
        .size(px(5.))
        .rounded(px(1.))
        .map(|m| match kind {
            LaneMod::Starting | LaneMod::Waiting => m.border_1().border_color(fg),
            LaneMod::Error => m.rounded_full().bg(fg),
            _ => m.bg(fg).opacity(mark),
        });
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .text_color(fg)
        .child(mark)
        .child(
            div()
                .when(kind == LaneMod::Off, |t| {
                    t.line_through().text_decoration_color(p.text_faint)
                })
                .child(lane.label()),
        )
        .children(small.map(|s| div().text_color(p.text_subtle).child(format!("· {s}"))))
}

fn rec_pill(shell: &Shell, p: &Palette, cx: &mut Context<Shell>) -> impl IntoElement {
    let m = &shell.model.status;
    let view = model::rec_view(m);
    let (fg, border, dot, glow) = match view.kind {
        RecMod::Running => (
            p.status_running_fg,
            p.status_running_border,
            p.status_running_dot,
            Some(p.status_running_dot_glow),
        ),
        RecMod::Paused => (
            p.status_paused_fg,
            p.status_paused_border,
            p.status_paused_dot,
            Some(p.status_paused_dot_glow),
        ),
        RecMod::Suspended => (p.warn, p.warn_border, p.warn, None),
        RecMod::Stopped => (p.danger_text, p.danger_border, p.danger, None),
        RecMod::Starting => (p.text_muted, p.border_strong, p.status_dot, None),
        RecMod::Off => (
            p.status_fg,
            gpui_kit::transparent_black(),
            p.status_dot,
            None,
        ),
    };
    let clock = match (view.clock, m.session_start_ms) {
        (Some(model::Clock::Elapsed), Some(start)) => {
            Some(model::format_clock(clock::now_ms(cx) - start, true))
        }
        (Some(model::Clock::Paused), _) => shell
            .pause_clock
            .elapsed(cx.background_executor().now())
            .map(|d| model::format_clock(d.as_millis() as i64, false)),
        _ => None,
    };
    let label = match &clock {
        Some(c) => format!("{} {c}", view.label),
        None => view.label.clone(),
    };
    // Steady dot + static 3px glow ring (no looping motion anywhere).
    let dot = div()
        .flex_none()
        .size(px(7.))
        .rounded_full()
        .bg(dot)
        .when_some(glow, |d, glow| d.shadow(vec![crate::ui::ring(glow)]));
    div()
        .id("status-rec")
        .role(Role::Button)
        .aria_label(SharedString::from(label.clone()))
        .aria_expanded(shell.overlay == Some(Overlay::Status))
        .flex()
        .items_center()
        .gap(px(7.))
        .h(size::REC_PILL_H)
        .pl(px(7.))
        .pr(px(9.))
        .rounded(radius::PILL)
        .border_1()
        .border_color(border)
        .text_color(fg)
        .cursor_pointer()
        .hover(|s| s.bg(p.wash_strong))
        .child(dot)
        .child(label)
        .on_click(cx.listener(|this, _, _, cx| this.toggle_overlay(Overlay::Status, cx)))
}

fn record_controls(
    shell: &Shell,
    p: &Palette,
    cx: &mut Context<Shell>,
) -> Vec<gpui_kit::AnyElement> {
    let m = &shell.model.status;
    if m.running {
        let pause_label = if m.user_paused {
            "Resume recording"
        } else {
            "Pause recording"
        };
        let glyph = if m.user_paused {
            crate::assets::FILL_PLAY
        } else {
            crate::assets::FILL_PAUSE
        };
        return vec![
            with_tip(
                icon_button("status-pause", pause_label, size::H_MINI, false, p),
                pause_label,
            )
            .child(icon(glyph, size::ICON_XS))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx)))
            .into_any_element(),
            with_tip(
                icon_button("status-stop", "Stop recording", size::H_MINI, false, p),
                "Stop recording",
            )
            .child(icon(crate::assets::FILL_STOP, size::ICON_XS))
            .on_click(cx.listener(|this, _, _, cx| this.stop_recording(cx)))
            .into_any_element(),
        ];
    }
    let off = m.read_only;
    let record = div()
        .id("status-record")
        .role(Role::Button)
        .aria_label("Start recording")
        .flex()
        .items_center()
        .gap(px(6.))
        .h(size::H_MINI)
        .px(px(9.))
        .rounded(radius::BTN_SM)
        .bg(p.surface_raised)
        .border_1()
        .border_color(p.border_strong)
        .font_family(FONT_UI)
        .text_size(text::BASE)
        .text_color(p.text_strong)
        .cursor_pointer()
        .hover(|s| s.bg(p.surface_hover))
        .when(off, |el| el.opacity(theme::DISABLED_OPACITY))
        .child(
            div()
                .size(px(5.))
                .rounded_full()
                .bg(if off { p.text_strong } else { p.danger }),
        )
        .child("Record")
        .on_click(cx.listener(|this, _, _, cx| this.start_recording(cx)))
        .test_support();
    vec![with_tip(
        record,
        format!(
            "Start recording  {}",
            crate::keymap::display(&shell.bindings.toggle_recording)
        ),
    )
    .into_any_element()]
}

fn lanes(shell: &Shell, p: &Palette, cx: &mut Context<Shell>) -> gpui_kit::AnyElement {
    let m = &shell.model.status;
    if m.running || m.loading_start {
        let live = model::live_lanes(m);
        return div()
            .id("status-lanes")
            .role(Role::Button)
            .aria_label("What Mnema is doing")
            .aria_expanded(shell.overlay == Some(Overlay::Status))
            .flex()
            .items_center()
            .gap(space::S3)
            .h(size::H_MINI)
            .mx(px(-6.))
            .px(px(6.))
            .rounded(radius::SM)
            .cursor_pointer()
            .hover(|s| s.bg(p.wash_strong))
            .when(shell.overlay == Some(Overlay::Status), |el| {
                el.bg(p.wash_strong)
            })
            .children(
                Lane::ALL
                    .iter()
                    .zip(live)
                    .map(|(l, (kind, small))| lane(*l, kind, small, p)),
            )
            .child(
                div()
                    .ml(px(-4.))
                    .text_color(p.text_faint)
                    .child(icon(IconName::ChevronUp.path(), px(10.))),
            )
            .on_click(cx.listener(|this, _, _, cx| this.toggle_overlay(Overlay::Status, cx)))
            .into_any_element();
    }
    let blocked = model::screen_permission_missing(m);
    let mut row = div().flex().items_center().gap(space::S3);
    for (i, l) in Lane::ALL.into_iter().enumerate() {
        let on = m.selected[i];
        let kind = if on { LaneMod::On } else { LaneMod::Off };
        let small = (i == 0 && blocked).then_some("no permission");
        let tip = format!(
            "{}: {} ({})",
            l.name(),
            if on {
                "on — click to skip on next recording"
            } else {
                "off — click to include in next recording"
            },
            i + 1
        );
        let button = div()
            .id(("status-lane", i))
            .role(Role::Button)
            .aria_label(format!("{} for the next recording", l.name()))
            .aria_toggled(if on {
                gpui_kit::Toggled::True
            } else {
                gpui_kit::Toggled::False
            })
            .h(px(20.))
            .mx(px(-4.))
            .px(px(5.))
            .flex()
            .items_center()
            .rounded(radius::XS)
            .cursor_pointer()
            .hover(|s| s.bg(p.wash_strong))
            .when(m.read_only, |el| el.opacity(theme::DISABLED_OPACITY))
            .child(lane(l, kind, small, p))
            .on_click(cx.listener(move |_, _, window, cx| match i {
                0 => window.dispatch_action(Box::new(ToggleSourceScreen), cx),
                1 => window.dispatch_action(Box::new(ToggleSourceMicrophone), cx),
                _ => window.dispatch_action(Box::new(ToggleSourceSystemAudio), cx),
            }));
        row = row.child(with_tip(button, tip));
    }
    row.into_any_element()
}

/// A pill: the status flag (`.mx-flag[data-tone]`) or the engine (`.mx-engine`).
fn pill(
    id: &'static str,
    lead: &str,
    bold: String,
    tone: Option<(Hsla, Hsla)>,
    p: &Palette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let (fg, border, dot, strong) = match tone {
        Some((tone, border)) => (tone, border, tone, tone),
        None => (p.text_muted, p.border_strong, p.status_dot, p.text),
    };
    div()
        .id(id)
        .role(Role::Button)
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(20.))
        .pl(px(7.))
        .pr(px(8.))
        .rounded(radius::PILL)
        .border_1()
        .border_color(border)
        .text_color(fg)
        .cursor_pointer()
        .hover(|s| s.border_color(p.border_hover))
        .child(div().size(px(6.)).rounded_full().bg(dot))
        .when(!lead.is_empty(), |el| el.child(lead.to_string()))
        .child(div().text_color(strong).child(bold))
}

pub fn render(
    shell: &Shell,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Shell>,
) -> impl IntoElement {
    let m = &shell.model.status;
    let wide = window.viewport_size().width >= breakpoint::STATUS_WIDE;
    let rec = model::rec_view(m);
    let flag = model::status_flag(m).map(|f| {
        let tone = match f.tone {
            Tone::Info => (p.info, p.info_border),
            Tone::Warn => (p.warn, p.warn_border),
            Tone::Danger => (p.danger, p.danger_border),
        };
        let flag = f.clone();
        pill("status-flag", f.text, f.action, Some(tone), p)
            .on_click(cx.listener(move |this, _, _, cx| this.run_flag(&flag, cx)))
            .into_any_element()
    });
    let resting = (flag.is_none() && rec.kind == RecMod::Paused && rec.clock.is_none())
        .then(|| div().child("resumes when you’re back").into_any_element());
    let engine = m.engine.as_ref().map(|e| {
        let tone = e.error.then_some((p.warn, p.warn_border));
        div()
            .flex()
            .items_center()
            .gap(space::S3)
            .child(with_tip(
                pill("status-engine", "engine", e.label.to_string(), tone, p).on_click(
                    // `openSettings("intelligence")`, as the Svelte engine pill.
                    |_, _, cx| {
                        cx.defer(|cx| crate::window::open_settings(Some("intelligence"), cx))
                    },
                ),
                e.tip.clone(),
            ))
            .child(sep(p))
    });

    let mut readouts: Vec<gpui_kit::AnyElement> = Vec::new();
    if let Some(ms) = m.tracked_ms {
        readouts.push(
            div()
                .child(format!("today {}", model::format_tracked(ms)))
                .into_any_element(),
        );
    }
    if wide {
        if let Some(bytes) = m.storage_bytes.filter(|b| *b > 0) {
            if !readouts.is_empty() {
                readouts.push(sep(p).into_any_element());
            }
            readouts.push(
                div()
                    .child(format!("{} stored", model::format_bytes(bytes)))
                    .into_any_element(),
            );
        }
        if m.local_only {
            if !readouts.is_empty() {
                readouts.push(sep(p).into_any_element());
            }
            readouts.push(div().child("local only").into_any_element());
        }
    }

    div()
        .id("status-bar")
        .flex()
        .flex_none()
        .items_center()
        .gap(space::S3)
        .h(size::STATUS_BAR)
        .px(space::S3)
        .min_w_0()
        .overflow_hidden()
        .bg(p.status_bg)
        .border_t_1()
        .border_color(p.status_border)
        .font_family(FONT_MONO)
        .text_size(text::XS)
        .font_weight(theme::W_MEDIUM)
        .text_color(p.status_fg)
        .whitespace_nowrap()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(rec_pill(shell, p, cx))
        .children(record_controls(shell, p, cx))
        .child(sep(p))
        .child(lanes(shell, p, cx))
        .child(div().flex_1())
        .children(flag)
        .children(resting)
        .children(engine)
        .children(readouts)
        .child(
            with_tip(
                icon_button(
                    "status-shortcuts",
                    "Keyboard shortcuts",
                    size::H_MINI,
                    false,
                    p,
                ),
                format!(
                    "Keyboard shortcuts · {}",
                    crate::keymap::display(&shell.bindings.toggle_shortcuts_help)
                ),
            )
            .aria_expanded(shell.overlay == Some(Overlay::Shortcuts))
            .child(icon(IconName::CircleQuestionMark.path(), px(14.)))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_overlay(Overlay::Shortcuts, cx))),
        )
}
