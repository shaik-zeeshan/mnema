//! The developer window's frame pieces (`DevStrip.svelte`, `Verdict.svelte`,
//! `MiniView.svelte`): the strip with pause · keep on top · mini · ⋯, the
//! verdict line, the tab bar, the ⋯ menu, and the mini view.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context};

use super::model::{self, Tab, MINI_DOTS};
use super::{DebugWindow, Mode};
use crate::small::parts::dot;
use crate::theme::{self, size, space, text, Palette, FONT_MONO};
use crate::ui::menu::{menu, menu_item};
use crate::ui::segmented::Segmented;
use crate::ui::window_strip::WindowStrip;
use crate::ui::{icon, Button, Force, Tone};

fn small_icon(id: &'static str, path: &'static str, aria: &'static str) -> Button {
    Button::icon_only(id, path, aria).ghost().sm().h(px(22.))
}

pub fn strip(w: &DebugWindow, cx: &mut Context<DebugWindow>, p: &Palette) -> AnyElement {
    let pause_label = if w.live {
        "Pause live updates"
    } else {
        "Resume live updates"
    };
    let pause = div()
        .relative()
        .child(
            small_icon(
                "dv-pause",
                if w.live {
                    crate::assets::FILL_LUCIDE_PAUSE
                } else {
                    crate::assets::FILL_LUCIDE_PLAY
                },
                pause_label,
            )
            .pressed(!w.live)
            .tip(pause_label)
            .on_click(cx.listener(|w, _, _, cx| w.toggle_live(cx))),
        )
        .when(w.live, |d| {
            d.child(
                div()
                    .absolute()
                    .top(px(3.))
                    .right(px(3.))
                    .size(px(5.))
                    .rounded_full()
                    .bg(p.accent)
                    .shadow(vec![crate::ui::spread(p.accent_glow, px(2.))]),
            )
        });
    WindowStrip::new("Developer")
        .action(pause)
        .action(
            small_icon("dv-pin", "icons/pin.svg", "Keep on top")
                .pressed(w.pinned)
                .tip("Keep on top")
                .on_click(cx.listener(|w, _, window, cx| w.set_pinned(!w.pinned, window, cx))),
        )
        .action(
            small_icon("dv-mini-btn", "icons/minimize-2.svg", "Mini mode")
                .tip("Mini mode")
                .on_click(cx.listener(|w, _, window, cx| w.to_mini(window, cx))),
        )
        .action(
            small_icon("dv-more", "icons/ellipsis.svg", "More")
                .pressed(w.menu_open)
                .tip("More")
                .on_click(cx.listener(|w, _, _, cx| {
                    w.menu_open = !w.menu_open;
                    cx.notify();
                })),
        )
        .action(div().w(space::S1))
        .into_any_element()
}

pub fn more_menu(
    w: &DebugWindow,
    cx: &mut Context<DebugWindow>,
    p: &Palette,
) -> Option<AnyElement> {
    if !w.menu_open {
        return None;
    }
    let log_path = w
        .data
        .log_tail
        .as_ref()
        .and_then(|t| t["path"].as_str())
        .map(str::to_string);
    let items = vec![
        menu_item(
            "dv-copy",
            "Copy diagnostics",
            Some("\u{21e7}\u{2318}C".into()),
            Force::None,
            cx.listener(|w, _, _, cx| w.copy_diagnostics(cx)),
            p,
        )
        .into_any_element(),
        menu_item(
            "dv-reveal",
            "Reveal logs in Finder",
            None,
            Force::None,
            cx.listener(move |w, _, _, cx| {
                w.menu_open = false;
                if let Some(path) = &log_path {
                    cx.reveal_path(std::path::Path::new(path));
                }
                cx.notify();
            }),
            p,
        )
        .into_any_element(),
        menu_item(
            "dv-open-log",
            "Open app log",
            None,
            Force::None,
            cx.listener(|w, _, _, cx| {
                w.menu_open = false;
                if let Some(core) = crate::chat::core(cx) {
                    if let Err(e) = app_core::general_app_log::open(&core) {
                        log::warn!("debug: open app log failed: {}", e.code);
                    }
                }
                cx.notify();
            }),
            p,
        )
        .into_any_element(),
    ];
    Some(
        div()
            .absolute()
            .top(size::TITLEBAR + px(4.))
            .right(space::S2)
            .w(px(236.))
            .child(menu(items, p))
            .into_any_element(),
    )
}

pub fn verdict(w: &DebugWindow, cx: &mut Context<DebugWindow>, p: &Palette) -> AnyElement {
    let d = &w.data;
    let v = model::health_verdict(&d.health, d.health_error.as_deref());
    let tail = if d.version.is_empty() {
        String::new()
    } else {
        format!(" \u{b7} {}", d.version)
    };
    let recording = d.capture.as_ref().map_or_else(
        || model::is_recording(&d.health),
        |c| c["recording"].as_bool().unwrap_or(false),
    );
    let started = d.capture.as_ref().and_then(|c| c["startedAtMs"].as_i64());
    let right = match (w.paused_at_ms, recording, started) {
        (Some(at), _, _) => format!(
            "paused at {}",
            crate::timeline::model::clock_label(
                crate::insights::stats::local(at, crate::ui::clock::now(cx).offset()),
                true,
            )
        ),
        (None, true, Some(s)) => format!(
            "recording {}{tail}",
            model::format_running(d.fetched_at_ms.unwrap_or(s) - s)
        ),
        (None, true, None) => format!("recording{tail}"),
        (None, false, _) => format!("not recording{tail}"),
    };
    div()
        .id("dv-verdict")
        .flex()
        .items_center()
        .gap(space::S2)
        .min_w_0()
        .px(space::S4)
        .pt(px(11.))
        .pb(px(9.))
        .child(dot(v.tone, px(7.), p))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .font_weight(theme::W_MEDIUM)
                .text_size(text::MD)
                .line_height(relative(1.3))
                .text_color(p.text_strong)
                .child(v.text),
        )
        .child(
            div()
                .ml_auto()
                .flex_none()
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .text_color(p.text_subtle)
                .whitespace_nowrap()
                .child(right),
        )
        .into_any_element()
}

pub fn tab_bar(w: &DebugWindow, cx: &mut Context<DebugWindow>, p: &Palette) -> AnyElement {
    let selected = Tab::ALL.iter().position(|t| *t == w.tab).unwrap_or(0);
    let this = cx.entity().downgrade();
    div()
        .flex_none()
        .px(space::S4)
        .pt(px(2.))
        .pb(px(10.))
        .border_b_1()
        .border_color(p.hairline)
        .child(
            Segmented::new("dv-tabs", "Section", Tab::ALL.map(|t| t.label()), selected)
                .fill()
                .on_change(move |ix, _, cx| {
                    let _ = this.update(cx, |w, cx| w.go(Tab::ALL[ix], cx));
                }),
        )
        .into_any_element()
}

/// The 300×120 mini frame: a 28 px strip, eight dots, the worst row, the queue.
pub fn mini(w: &DebugWindow, cx: &mut Context<DebugWindow>, p: &Palette) -> AnyElement {
    let rows = &w.data.health;
    let foot = model::mini_footer(rows, w.data.health_error.as_deref());
    let queued: Option<i64> = (!w.data.lanes.is_empty() || w.data.semantic.is_some()).then(|| {
        w.data
            .lanes
            .iter()
            .map(|l| l["queued"].as_i64().unwrap_or(0))
            .sum::<i64>()
            + w.data
                .semantic
                .as_ref()
                .and_then(|s| s["backlogCount"].as_i64())
                .unwrap_or(0)
    });
    // `grid-template-columns: repeat(4, auto); justify-content: space-between`:
    // four content-sized columns (item i over item i + 4), spread apart.
    let item = |(feature, label): &(&str, &'static str)| {
        let tone = model::severity_for(rows, feature).map_or(Tone::Off, model::tone_of);
        div()
            .flex()
            .items_center()
            .gap(px(5.))
            .whitespace_nowrap()
            .font_weight(theme::W_MEDIUM)
            .text_size(text::XS)
            .line_height(relative(1.))
            .text_color(p.text_muted)
            .child(dot(tone, px(6.), p))
            .child(*label)
    };
    let dots = div()
        .flex()
        .justify_between()
        .gap_x(px(10.))
        .children((0..4).map(|c| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(item(&MINI_DOTS[c]))
                .child(item(&MINI_DOTS[c + 4]))
        }));
    let foot = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .min_w_0()
        .pt(px(7.))
        .border_t_1()
        .border_color(p.hairline)
        .child(dot(foot.tone, px(7.), p))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(text::SM)
                .line_height(relative(1.2))
                .text_color(p.text)
                .child(foot.text),
        )
        .children(queued.map(|q| {
            div()
                .flex_none()
                .font_family(FONT_MONO)
                .font_weight(theme::W_MEDIUM)
                .text_size(text::XS)
                .text_color(p.text_subtle)
                .child(format!("{} queued", model::count(q)))
        }));
    let strip = div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(28.))
        .pl(px(9.))
        .pr(px(4.))
        .border_b_1()
        .border_color(p.hairline)
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, window, _| {
            window.start_window_move()
        })
        .child(
            div()
                .flex()
                .gap(px(6.))
                .w(px(54.))
                .children((0..3).map(|i| {
                    let lit = [p.status_running_dot, p.status_paused_dot, p.source_mic][i];
                    div()
                        .id(("dv-mini-light", i))
                        .size(px(9.))
                        .rounded_full()
                        .bg(p.border_strong)
                        .hover(move |s| s.bg(lit))
                        .when(i == 0, |d| {
                            d.role(gpui_kit::Role::Button)
                                .aria_label("Close window")
                                .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })
                                .on_click(|_, window, _| window.remove_window())
                        })
                })),
        )
        .child(
            div()
                .flex_1()
                .text_center()
                .font_weight(theme::W_MEDIUM)
                .text_size(text::SM)
                .text_color(p.text_subtle)
                .child("Developer"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .text_color(p.text_subtle)
                .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation()
                })
                .child(icon("icons/pin.svg", px(12.)))
                .child(
                    Button::icon_only("dv-expand", "icons/maximize-2.svg", "Expand")
                        .ghost()
                        .sm()
                        .h(px(20.))
                        .on_click(cx.listener(|w, _, window, cx| w.expand(window, cx))),
                ),
        );
    div()
        .id("dv-mini")
        .size_full()
        .flex()
        .flex_col()
        .child(strip)
        .child(
            div()
                .id("dv-mini-body")
                .role(gpui_kit::Role::Button)
                .aria_label("Expand developer window")
                .flex_1()
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(9.))
                .px(px(12.))
                .cursor_pointer()
                .on_click(cx.listener(|w, _, window, cx| {
                    if w.mode == Mode::Mini {
                        w.expand(window, cx);
                    }
                }))
                .child(dots)
                .child(foot),
        )
        .into_any_element()
}
