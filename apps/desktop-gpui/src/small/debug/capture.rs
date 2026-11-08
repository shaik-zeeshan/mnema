//! The Capture tab (`tabs/CaptureTab.svelte`): the run line, three source lanes
//! with level meters, the suspension notice, the system-audio tap spec, what is
//! kept out of recordings, and disk headroom. Polled at 1 s.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, Hsla};
use serde_json::Value;

use super::readouts::{self, disk_at, gb, lane_detail, lane_word, meter_bars};
use super::tabs::{clock, h, sec};
use super::DebugWindow;
use crate::settings::model::Json;
use crate::small::parts::{dot, spec_row};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::{alpha, icon, notice, Button, Tone};

fn lane(
    name: &'static str,
    glyph: &'static str,
    c: Hsla,
    src: &Value,
    audio: bool,
    rate: Option<f64>,
    right: String,
    p: &Palette,
) -> AnyElement {
    let state = src.str_at(&["state"]).unwrap_or("off");
    let off = state == "off";
    let (word, tone) = lane_word(state);
    let level = src.num_at(&["levelDb"]);
    let live = state == "recording" && level.is_some();
    let lit = if live { meter_bars(level) } else { 0 };
    let icon_box = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(26.))
        .rounded(radius::SM)
        .border_1()
        .map(|d| {
            if off {
                d.text_color(p.text_subtle).border_color(p.border_strong)
            } else {
                d.text_color(c)
                    .bg(alpha(c, 0.12))
                    .border_color(alpha(c, 0.24))
            }
        })
        .child(icon(glyph, px(14.)));
    let right_el = if audio {
        div()
            .flex()
            .items_center()
            .gap(space::S2)
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(2.))
                    .h(px(12.))
                    .children((0..12).map(|i| {
                        div().w(px(3.)).h_full().rounded(px(1.)).bg(if i < lit {
                            c
                        } else {
                            p.wash_strong
                        })
                    })),
            )
            .child(
                div()
                    .min_w(px(52.))
                    .text_right()
                    .font_family(FONT_MONO)
                    .text_size(text::SM)
                    .text_color(p.text_muted)
                    .child(match (live, level) {
                        (true, Some(db)) => format!(
                            "{}{} dB",
                            if db < 0. { "\u{2212}" } else { "" },
                            db.abs().round()
                        ),
                        _ => "\u{2014}".into(),
                    }),
            )
    } else {
        div()
            .font_family(FONT_MONO)
            .text_size(text::SM)
            .text_color(p.text_muted)
            .child(right)
    };
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .py(px(8.))
        .px(px(2.))
        .border_b_1()
        .border_color(p.hairline)
        .child(icon_box)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .font_weight(theme::W_MEDIUM)
                        .text_size(text::MD)
                        .line_height(relative(1.25))
                        .text_color(p.text_strong)
                        .child(name),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .mt(px(2.))
                        .text_size(text::BASE)
                        .line_height(relative(1.3))
                        .text_color(p.text_subtle)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(
                            div()
                                .font_weight(theme::W_MEDIUM)
                                .text_color(tone.map_or(p.text, |t| t.colors(p).0))
                                .child(word),
                        )
                        .child(lane_detail(src, rate)),
                ),
        )
        .child(right_el)
        .into_any_element()
}

pub fn tab(w: &DebugWindow, cx: &mut Context<DebugWindow>, p: &Palette) -> Vec<AnyElement> {
    let d = &w.data;
    let Some(health) = d.capture.as_ref() else {
        // shortcut: get_capture_health needs the capture runtime managed in this
        // host (S4 startup); until then the tab waits here.
        return vec![empty(
            EmptyLayout::Compact,
            Tone::Neutral,
            "icons/inbox.svg",
            "Reading capture health\u{2026}",
            "The capture runtime answers once recording has started in this app.",
            vec![],
            p,
        )
        .px(px(0.))
        .into_any_element()];
    };
    let now_ms = crate::ui::clock::now_ms(cx);
    let recording = health.bool_at(&["recording"]);
    let started = health.num_at(&["startedAtMs"]).map(|n| n as i64);
    let tap = health.at(&["systemAudioTap"]);
    let rate = tap.num_at(&["sampleRateHz"]);
    let susp = health.at(&["suspension"]);
    let screen_right = {
        let s = health.at(&["screen"]);
        match (s.str_at(&["state"]), susp.num_at(&["sinceMs"])) {
            (Some("waiting"), Some(since)) => {
                format!("{} min", ((now_ms - since as i64) / 60_000).max(0))
            }
            (Some("recording"), _) => d
                .settings
                .as_ref()
                .and_then(|s| s.num_at(&["screenFrameRate"]))
                .filter(|f| *f > 0.)
                .map_or("\u{2014}".into(), |fps| {
                    let s = 1. / fps;
                    if s < 60. {
                        format!("every {} s", (s * 10.).round() / 10.)
                    } else {
                        format!("every {} min", (s / 60.).round())
                    }
                }),
            _ => "\u{2014}".into(),
        }
    };
    let run = div()
        .flex()
        .items_center()
        .gap(space::S2)
        .child(dot(if recording { Tone::Ok } else { Tone::Off }, px(7.), p))
        .child(
            div()
                .font_weight(theme::W_MEDIUM)
                .text_size(text::MD)
                .text_color(p.text_strong)
                .child(if recording {
                    "Recording"
                } else {
                    "Not recording"
                }),
        )
        .children(started.filter(|_| recording).map(|s| {
            div()
                .font_family(FONT_MONO)
                .text_size(text::BASE)
                .text_color(p.text_muted)
                .child(format!(
                    "{} \u{b7} since {}",
                    super::model::format_running(now_ms - s),
                    clock(s, cx)
                ))
        }))
        .child(div().flex_1())
        // shortcut: Record / Stop go through the recording lifecycle (S3/S4 in this host).
        .child(
            Button::new("dv-run", if recording { "Stop" } else { "Record" })
                .sm()
                .when(recording, |b| b.icon(crate::assets::STOP_SQUARE)),
        );
    let lanes = div()
        .flex()
        .flex_col()
        .border_t_1()
        .border_color(p.hairline)
        .child(lane(
            "Screen",
            "icons/monitor.svg",
            p.source_screen,
            health.at(&["screen"]),
            false,
            None,
            screen_right,
            p,
        ))
        .child(lane(
            "Microphone",
            "icons/mic.svg",
            p.source_mic,
            health.at(&["microphone"]),
            true,
            None,
            String::new(),
            p,
        ))
        .child(lane(
            "System audio",
            "icons/volume-2.svg",
            p.source_sysaudio,
            health.at(&["systemAudio"]),
            true,
            rate,
            String::new(),
            p,
        ));
    let mut out = vec![sec(p).child(run).child(lanes).into_any_element()];

    let free = health
        .num_at(&["disk", "freeBytes"])
        .or_else(|| d.storage.as_ref().and_then(|s| s.num_at(&["freeBytes"])));
    let floor = health.num_at(&["disk", "reserveFloorBytes"]);
    let pause_at = health.num_at(&["disk", "pauseThresholdBytes"]).or(floor);
    let resume_at = health.num_at(&["disk", "resumeThresholdBytes"]).or(floor);
    match susp.str_at(&["kind"]) {
        Some("displayUnavailable") => {
            let n = susp.num_at(&["recoveryAttempts"]).unwrap_or(0.) as i64;
            let since = susp.num_at(&["sinceMs"]).map_or("Checked".into(), |s| {
                format!("Since {}, checked", clock(s as i64, cx))
            });
            out.push(
                notice::notice(
                    Tone::Info,
                    "icons/monitor.svg",
                    "Resumes on its own when the display wakes",
                    Some(format!("{since} {n} {}. The last few minutes were saved; microphone and system audio keep recording.", if n == 1 { "time" } else { "times" }).into()),
                    vec![],
                    p,
                )
                .into_any_element(),
            );
        }
        Some("lowDisk") => out.push(
            notice::notice(
                Tone::Warn,
                "icons/triangle-alert.svg",
                format!(
                    "Recording paused until {} is free",
                    resume_at.map_or("more space".into(), |r| format!("{} GB", gb(r)))
                ),
                Some("It picks up on its own once there is room again.".into()),
                vec![],
                p,
            )
            .into_any_element(),
        ),
        Some("privacyFilter") if susp.bool_at(&["restartRequired"]) => out.push(
            notice::notice(
                Tone::Danger,
                "icons/triangle-alert.svg",
                "The privacy filter needs a restart",
                Some("The screen stays off until then. Press Stop, then Record.".into()),
                vec![],
                p,
            )
            .into_any_element(),
        ),
        _ => {}
    }
    if !recording {
        out.push(
            empty(
                EmptyLayout::Compact,
                Tone::Neutral,
                "icons/inbox.svg",
                "Nothing is being captured",
                "Earlier recordings are still being read and indexed. See Pipeline.",
                vec![],
                p,
            )
            .px(px(0.))
            .py(px(10.))
            .into_any_element(),
        );
    }
    if recording && !tap.is_null() {
        let n = tap.num_at(&["rebuildsToday"]).unwrap_or(0.) as i64;
        let reason =
            readouts::rebuild_reason(tap.str_at(&["lastReason"]), tap.str_at(&["lastDetail"]));
        let rows = [
            (
                "Rebuilt today",
                format!(
                    "{n} {}{}",
                    if n == 1 { "time" } else { "times" },
                    if health.str_at(&["systemAudio", "state"]) == Some("reconnecting") {
                        " \u{b7} rebuilding now"
                    } else {
                        ""
                    }
                ),
                Some("each opens a new file".to_string()),
            ),
            (
                "Last reason",
                reason,
                tap.num_at(&["lastRebuildAtMs"])
                    .map(|ms| clock(ms as i64, cx)),
            ),
            (
                "Silence check",
                tap.num_at(&["nextSilenceCheckInMs"])
                    .map_or("\u{2014}".into(), |ms| {
                        format!("next in {}", readouts::format_seconds(ms))
                    }),
                tap.num_at(&["watchdogDelayMs"])
                    .map(|ms| format!("every {}", readouts::format_seconds(ms))),
            ),
        ];
        out.push(
            sec(p)
                .child(h("System audio connection", None, p))
                .children(rows.into_iter().enumerate().map(|(i, (l, v, n))| {
                    spec_row(i == 0, l, div().child(v), n.map(Into::into), px(110.), p)
                }))
                .into_any_element(),
        );
    }
    if let Some(s) = &d.settings {
        let kept: Vec<String> = s
            .at(&["privacy", "excludedApps"])
            .as_array()
            .into_iter()
            .flatten()
            .filter(|a| a.bool_at(&["enabled"]))
            .filter_map(|a| a.str_at(&["displayName"]).map(str::to_string))
            .collect();
        let url = if s.bool_at(&["metadata", "enabled"]) {
            s.str_at(&["metadata", "browserUrlMode"]).unwrap_or("off")
        } else {
            "off"
        };
        let url = match url {
            "sanitized" => "saved without query strings",
            "full" => "saved in full",
            _ => "not saved",
        };
        let n = kept.len();
        out.push(
            sec(p)
                .child(h(
                    "Kept out of recordings",
                    Some(format!("{n} {}", if n == 1 { "app" } else { "apps" })),
                    p,
                ))
                .when(n > 0, |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(5.))
                            .children(kept.into_iter().map(|name| {
                                let initial = name
                                    .trim()
                                    .chars()
                                    .next()
                                    .map_or("?".into(), |c| c.to_uppercase().to_string());
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .h(px(22.))
                                    .pl(px(3.))
                                    .pr(px(8.))
                                    .rounded(radius::SM)
                                    .border_1()
                                    .border_color(p.border)
                                    .bg(p.surface_subtle)
                                    .text_size(text::BASE)
                                    .text_color(p.text)
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(16.))
                                            .rounded(px(4.))
                                            .bg(p.wash_strong)
                                            .font_weight(theme::W_SEMIBOLD)
                                            .text_size(text::XS)
                                            .text_color(p.text_muted)
                                            .child(initial),
                                    )
                                    .child(name)
                            })),
                    )
                })
                .child(
                    div()
                        .flex()
                        .gap(space::S2)
                        .text_size(text::BASE)
                        .text_color(p.text_subtle)
                        .child("Web addresses")
                        .child(
                            div()
                                .font_weight(theme::W_MEDIUM)
                                .text_color(p.text)
                                .child(url),
                        ),
                )
                .into_any_element(),
        );
    }
    let free_label = free.map_or("\u{2014}".into(), |f| format!("{} GB free", gb(f)));
    let mut disk = sec(p).child(
        h("Disk", None, p).child(div().flex_1()).child(
            div()
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .text_color(p.text)
                .child(free_label),
        ),
    );
    if let (Some(f), Some(pa), Some(ra)) = (free, pause_at, resume_at) {
        let warn = f < ra;
        let split = ra > pa;
        let tick = |at: f32, c: Hsla| {
            div()
                .absolute()
                .top(px(-3.))
                .bottom(px(-3.))
                .left(relative(at))
                .ml(px(-1.))
                .w(px(2.))
                .rounded(px(1.))
                .bg(c)
        };
        disk = disk
            .child(
                div()
                    .relative()
                    .h(px(8.))
                    .rounded(px(4.))
                    .bg(p.wash_strong)
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .w(relative(disk_at(f)))
                            .rounded(px(4.))
                            .bg(if warn {
                                p.warn
                            } else {
                                alpha(p.text_subtle, 0.5)
                            }),
                    )
                    .child(tick(disk_at(pa), p.text_muted))
                    .when(split, |d| d.child(tick(disk_at(ra), p.text_subtle))),
            )
            .child(
                div()
                    .relative()
                    .h(px(14.))
                    .mt(px(5.))
                    .font_family(FONT_MONO)
                    .text_size(text::XS)
                    .text_color(p.text_subtle)
                    .child(div().absolute().left_0().child(if disk_at(pa) < 0.12 {
                        format!("pause {}", gb(pa))
                    } else {
                        "0".into()
                    }))
                    .when(disk_at(pa) >= 0.12, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left(relative(disk_at(pa)))
                                .child(format!("pause {}", gb(pa))),
                        )
                    })
                    .when(split, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left(relative(disk_at(ra)))
                                .child(format!("resume {}", gb(ra))),
                        )
                    })
                    .child(div().absolute().right_0().child("25 GB +")),
            );
    }
    if let Some(s) = &d.storage {
        let n = |k: &str| s.num_at(&[k]).unwrap_or(0.);
        let used = n("screenBytes")
            + n("microphoneBytes")
            + n("systemAudioBytes")
            + n("indexBytes")
            + n("modelsBytes");
        disk = disk.child(
            div()
                .mt(px(2.))
                .text_size(text::BASE)
                .line_height(relative(1.5))
                .text_color(p.text_subtle)
                .child(format!(
                    "Mnema uses {} GB: screen {} \u{b7} audio {} \u{b7} meaning index {} \u{b7} models {}",
                    gb(used),
                    gb(n("screenBytes")),
                    gb(n("microphoneBytes") + n("systemAudioBytes")),
                    gb(n("indexBytes")),
                    gb(n("modelsBytes"))
                )),
        );
    }
    out.push(disk.into_any_element());
    out
}
