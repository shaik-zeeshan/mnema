//! The 30 px status bar (`.mx-statusbar`): REC pill + clock, pause/stop or
//! Record, the three source lanes (toggles while stopped, the status popover's
//! trigger while recording), one flag, the engine pill only when unhealthy,
//! today's time, storage, "local only" and the shortcuts `?`.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, Context, Div, Hsla, MouseButton, Role, SharedString, Window};

use super::model::{self, FlagKind, Lane, LaneMod, RecMod, Tone};
use super::{Overlay, Shell};
use crate::theme::{self, breakpoint, radius, size, space, text, Palette, FONT_MONO, FONT_UI};
use crate::ui::Tracking as _;
use crate::ui::{clock, icon, icon_button, sep, with_tip};

fn lane_color(lane: Lane, p: &Palette) -> Hsla {
    match lane {
        Lane::Screen => p.source_screen,
        Lane::Microphone => p.source_mic,
        Lane::SystemAudio => p.source_sysaudio,
    }
}

/// `.mx-lane`: a 5px mark (35% unless lit) then the label. `blocked`
/// (`.mx-lane--blocked`, no screen permission): a round lit mark, and warn ink
/// unless the lane is selected (the source colour outranks it in kit.css).
fn lane(lane: Lane, kind: LaneMod, small: Option<&'static str>, blocked: bool, p: &Palette) -> Div {
    let (fg, mark) = match kind {
        LaneMod::On => (lane_color(lane, p), 1.),
        LaneMod::Paused => (p.status_paused_fg, 1.),
        LaneMod::Starting | LaneMod::Waiting => (p.text_muted, 1.),
        LaneMod::Error => (p.danger_text, 1.),
        LaneMod::Off if blocked => (p.warn, 1.),
        LaneMod::Off => (p.text_subtle, 0.35),
    };
    let mark = div()
        .flex_none()
        .size(px(5.))
        .rounded(px(1.))
        .map(|m| match kind {
            LaneMod::Starting | LaneMod::Waiting => m.border_1().border_color(fg),
            LaneMod::Error => m.rounded_full().bg(fg),
            _ if blocked => m.rounded_full().bg(fg),
            // Two 2 px bars, 1 px apart, 6 px tall (the paused mark's gradient).
            LaneMod::Paused => m
                .h(px(6.))
                .rounded(px(0.))
                .flex()
                .justify_between()
                .child(div().w(px(2.)).h_full().bg(fg))
                .child(div().w(px(2.)).h_full().bg(fg)),
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
        // `.ss-rectrig[aria-expanded="true"]`: the fill stays while its popover is open.
        .when(shell.overlay == Some(Overlay::Status), |el| {
            el.bg(p.wash_strong)
        })
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
    let mut out = Vec::new();
    if m.running {
        // Pause is a whole-session control (it stays on through an idle pause);
        // Resume shows for a user pause or low disk, which resumes on its own.
        let show_resume = m.user_paused || m.low_disk;
        let off = m.loading_pause || (m.low_disk && !m.user_paused);
        let title = model::pause_title(m);
        let glyph = if show_resume {
            crate::assets::FILL_PLAY
        } else {
            crate::assets::FILL_PAUSE
        };
        out.push(
            with_tip(
                // `mx-btn--icon` without `--sm`: the 6 px corner.
                icon_button("status-pause", title, size::H_MINI, false, p).rounded(radius::SM),
                title,
            )
            .child(icon(glyph, size::ICON_XS))
            .when(off, |b| b.opacity(theme::DISABLED_OPACITY).cursor_default())
            .when(!off, |b| {
                b.on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx)))
            })
            .into_any_element(),
        );
    }
    let key = crate::keymap::display(&shell.bindings.toggle_recording);
    if m.running || m.loading_start {
        // Shown (disabled) while starting, so the bar doesn't jump when it lands.
        let off = !m.running || m.loading_stop;
        out.push(
            with_tip(
                icon_button("status-stop", "Stop recording", size::H_MINI, false, p)
                    .rounded(radius::SM),
                format!("Stop recording  {key}"),
            )
            .child(icon(crate::assets::FILL_STOP, size::ICON_XS))
            .when(off, |b| b.opacity(theme::DISABLED_OPACITY).cursor_default())
            .when(!off, |b| {
                b.on_click(cx.listener(|this, _, _, cx| this.stop_recording(cx)))
            })
            .into_any_element(),
        );
        return out;
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
        .tracking(-0.003)
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
    out.push(with_tip(record, format!("Start recording  {key}")).into_any_element());
    out
}

/// `terse`: without each lane's "· word" (the mark and the tooltip still say it).
fn lanes(shell: &Shell, terse: bool, p: &Palette, cx: &mut Context<Shell>) -> gpui_kit::AnyElement {
    let m = &shell.model.status;
    if m.running || m.loading_start {
        let live = model::live_lanes(m);
        let open = shell.overlay == Some(Overlay::Status);
        let aria = Lane::ALL
            .iter()
            .zip(live)
            .map(|(l, (kind, small))| format!("{}: {}", l.name(), model::lane_tip(kind, small)))
            .collect::<Vec<_>>()
            .join(", ");
        return div()
            .id("status-lanes")
            .role(Role::Button)
            .aria_label(SharedString::from(format!(
                "What Mnema is doing \u{2014} {aria}"
            )))
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
                    .enumerate()
                    .map(|(i, (l, (kind, small)))| {
                        let el = div().id(("status-live-lane", i)).child(lane(
                            *l,
                            kind,
                            small.filter(|_| !terse),
                            false,
                            p,
                        ));
                        // The popover says it all while open (`use:tip={statusPop.open ? null : …}`).
                        if open {
                            el.into_any_element()
                        } else {
                            let tip = format!("{}: {}", l.name(), model::lane_tip(kind, small));
                            with_tip(el, tip).into_any_element()
                        }
                    }),
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
        let screen_blocked = i == 0 && blocked;
        let small = screen_blocked.then_some("no permission");
        let tip = if screen_blocked {
            "Screen Recording is off for Mnema in System Settings".to_string()
        } else {
            format!(
                "{}: {} ({})",
                l.name(),
                if on {
                    "on \u{2014} click to skip on next recording"
                } else {
                    "off \u{2014} click to include in next recording"
                },
                crate::keymap::display(&shell.bindings.sources[i])
            )
        };
        let off = m.read_only || m.loading_start || shell.saving_source[i];
        let button = div()
            .id(("status-lane", i))
            .test_support()
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
            .when(off, |el| el.opacity(theme::DISABLED_OPACITY))
            .child(lane(l, kind, small.filter(|_| !terse), screen_blocked, p))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_source(i, cx)));
        row = row.child(with_tip(button, tip));
    }
    row.into_any_element()
}

/// A pill: the status flag (`.mx-flag[data-tone]`) or the engine (`.mx-engine`).
/// `hover_bg`: the fill under the pointer (transparent for most; `None` for a
/// `span.mx-flag`, which has no hover rule). No `lead` and no `bold` leaves the dot.
fn pill(
    id: &'static str,
    lead: &str,
    bold: String,
    tone: Option<(Hsla, Hsla)>,
    hover_bg: Option<Hsla>,
    p: &Palette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let (bd, strong_ink) = (p.border_hover, p.text_strong);
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
        // The bold part keeps its own colour (`.mx-flag[data-tone] b`).
        .when_some(hover_bg, |el, bg| {
            el.hover(move |s| s.bg(bg).border_color(bd).text_color(strong_ink))
        })
        // `.mx-flag[data-tone]::before`: a 3 px halo of the tone at 18%.
        .child(
            div()
                .flex_none()
                .size(px(6.))
                .rounded_full()
                .bg(dot)
                .when(tone.is_some(), |d| {
                    d.shadow(vec![crate::ui::ring(crate::ui::alpha(dot, 0.18))])
                }),
        )
        // The lead text gives way first (with an ellipsis), the action only once
        // the lead is gone, the bar's `?` never.
        .min_w_0()
        .when(!lead.is_empty(), |el| {
            el.child(
                div()
                    .min_w_0()
                    .flex_shrink(1e6)
                    .truncate()
                    .child(lead.to_string()),
            )
        })
        .when(!bold.is_empty(), |el| {
            el.child(div().min_w_0().truncate().text_color(strong).child(bold))
        })
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
        let (tone, tone_bg) = match f.tone {
            Tone::Info => ((p.info, p.info_border), p.info_bg),
            Tone::Warn => ((p.warn, p.warn_border), p.warn_bg),
            Tone::Danger => ((p.danger_text, p.danger_border), p.danger_bg),
        };
        let restart = f.kind == FlagKind::PrivacyRestart;
        let action = if restart && shell.restarting {
            "Restarting\u{2026}".to_string()
        } else {
            f.action.clone()
        };
        let hover_bg = match f.kind {
            FlagKind::PrivacyRetry => None,
            // `button.mx-flag.ss-trig-flag`: the tone fill on hover and while open.
            FlagKind::DiskSoon => Some(tone_bg),
            _ => Some(gpui_kit::transparent_black()),
        };
        let el = pill("status-flag", &f.text, action, Some(tone), hover_bg, p);
        let full = format!("{} {}", f.text, f.action).trim().to_string();
        // A narrow bar cuts the lead text short, so the tip carries all of it
        // (not over the open status popover, as the lanes).
        let tip = f
            .tip
            .clone()
            .or_else(|| (shell.overlay != Some(Overlay::Status)).then(|| full.clone()));
        // "retrying" is status, not a button (`<span role="status">`).
        let el = if f.kind == FlagKind::PrivacyRetry {
            el.role(Role::Status).cursor_default()
        } else if restart && (shell.restarting || m.loading_stop) {
            el.opacity(theme::BUSY_OPACITY).cursor_default()
        } else {
            let flag = f.clone();
            let open = shell.overlay == Some(Overlay::Status);
            el.when(f.kind == FlagKind::DiskSoon, |el| {
                el.aria_expanded(open).when(open, |el| el.bg(tone_bg))
            })
            .on_click(cx.listener(move |this, _, _, cx| this.run_flag(&flag, cx)))
        }
        .aria_label(SharedString::from(full))
        .test_support();
        match tip {
            Some(tip) => with_tip(el, tip).into_any_element(),
            None => el.into_any_element(),
        }
    });
    // Under 900 px a flag, the lanes' words and the engine pill don't fit together
    // (the web lets them overlap). The flag is the one to read, so the lanes drop
    // their words and the engine pill shrinks to its dot; both keep their tooltip.
    let compact = !wide && flag.is_some();
    let resting =
        (flag.is_none() && rec.kind == RecMod::Paused && rec.clock.is_none()).then(|| {
            let t = "resumes when you’re back";
            with_tip(div().id("status-resting").min_w_0().truncate().child(t), t).into_any_element()
        });
    let engine = m.engine.as_ref().map(|e| {
        let tone = e.error.then_some((p.warn, p.warn_border));
        div()
            .flex()
            .items_center()
            .gap(space::S3)
            .min_w_0()
            // Down to its dot, it has nothing left to give: the flag's text does.
            .when(compact, |d| d.flex_none())
            .child(with_tip(
                pill(
                    "status-engine",
                    if compact { "" } else { "engine" },
                    (!compact).then(|| e.label.to_string()).unwrap_or_default(),
                    tone,
                    Some(gpui_kit::transparent_black()),
                    p,
                )
                .test_support()
                .on_click(
                    // `openSettings("intelligence")`, as the Svelte engine pill.
                    |_, _, cx| {
                        cx.defer(|cx| crate::window::open_settings(Some("intelligence"), cx))
                    },
                ),
                if compact {
                    format!("engine {} \u{2014} {}", e.label, e.tip)
                } else {
                    e.tip.clone()
                },
            ))
            .child(sep(p))
    });

    // Below 900 px storage and "local only" go (`.ss-wide`). Whatever still
    // doesn't fit drops out whole, last first: each readout (with the separator
    // before it) wraps onto a second line the box's one-line height clips, where
    // the web clips the last one mid-word. A zero-width first item keeps a line
    // from ever holding just an overflowing readout. The `?` never shrinks.
    let mut texts: Vec<String> = Vec::new();
    texts.extend(
        m.tracked_ms
            .map(|ms| format!("today {}", model::format_tracked(ms))),
    );
    if wide {
        texts.extend(
            m.storage_bytes
                .filter(|b| *b > 0)
                .map(|bytes| format!("{} stored", model::format_bytes(bytes))),
        );
        texts.extend(m.local_only.then(|| "local only".to_string()));
    }
    let readouts = div()
        .id("status-readouts")
        .test_support()
        .flex()
        .flex_wrap()
        .justify_end()
        .h_full()
        .min_w_0()
        // The readouts give way before the flag's, engine's or resting text, the
        // only other items that shrink: flex hands out the shortfall by shrink ×
        // width, so this weight leaves their share far below an f32 step until the
        // readouts are gone.
        .flex_shrink(1e12)
        .overflow_hidden()
        .child(div().h_full())
        .children(texts.into_iter().enumerate().map(|(i, text)| {
            div()
                .id(("status-readout", i))
                .test_support()
                .flex()
                .items_center()
                .gap(space::S3)
                .h_full()
                .when(i > 0, |d| d.pl(space::S3).child(sep(p)))
                .child(text)
        }));

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
        .tracking(0.02)
        .text_color(p.status_fg)
        .whitespace_nowrap()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(rec_pill(shell, p, cx))
        .children(record_controls(shell, p, cx))
        .child(sep(p))
        .child(lanes(shell, compact, p, cx))
        .child(div().flex_1())
        .children(flag)
        .children(resting)
        .children(engine)
        .child(readouts)
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
