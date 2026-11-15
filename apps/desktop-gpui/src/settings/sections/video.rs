//! Capture › Video + Video Bitrate (`Video.svelte`, `ScreenResolutionControl`,
//! `VideoBitrateControl`, `capture-rate.ts`).

use capture_types::{
    RecordingSettings, ScreenResolution, ScreenResolutionPreset, VideoBitrateMode,
    VideoBitratePreset,
};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Window};

use crate::settings::row::{hint, row, section};
use crate::settings::{icons, on, Settings};
use crate::theme::{self, radius, space, text, Palette};
use crate::ui::icon;
use crate::ui::input::{field_warning, TextInput};
use crate::ui::segmented::Segmented;
use crate::ui::select::Select;
use crate::ui::stepper::Stepper;
use crate::ui::Tracking as _;

/// `CAPTURE_INTERVAL_LADDER_S`.
const LADDER: [f64; 11] = [0.1, 0.5, 1., 2., 3., 5., 10., 15., 30., 45., 60.];
/// `CAPTURE_RATE_STOPS_S`.
const STOPS: [f64; 6] = [0.1, 1., 2., 5., 15., 60.];

/// `nearestLadderIndex`: the ladder interval closest (in log space) to `fps`.
pub fn ladder_interval(fps: f64) -> f64 {
    let target = if fps > 0. { 1. / fps } else { 2. }.ln();
    LADDER
        .iter()
        .copied()
        .min_by(|a, b| (a.ln() - target).abs().total_cmp(&(b.ln() - target).abs()))
        .unwrap_or(2.)
}

fn seg_label(i: f64) -> String {
    if i < 1. {
        format!("{}/s", (1. / i).round())
    } else if i == 1. {
        "1/s".into()
    } else if i == 60. {
        "1/min".into()
    } else {
        format!("{i}s")
    }
}

/// `captureRateShortLabel`.
fn short_label(fps: f64) -> String {
    let i = ladder_interval(fps);
    if i < 1. {
        format!("{} snapshots/sec", (1. / i).round())
    } else if i == 1. {
        "1 snapshot/sec".into()
    } else if i == 60. {
        "1 snapshot/min".into()
    } else {
        format!("1 snapshot every {i}s")
    }
}

const RES: [&str; 5] = ["Original", "1080p", "720p", "540p", "Custom"];
const PRESETS: [ScreenResolutionPreset; 4] = [
    ScreenResolutionPreset::Original,
    ScreenResolutionPreset::P1080,
    ScreenResolutionPreset::P720,
    ScreenResolutionPreset::P540,
];

fn res_ix(r: &ScreenResolution) -> usize {
    match r {
        ScreenResolution::Custom { .. } => 4,
        ScreenResolution::Preset { preset } => {
            PRESETS.iter().position(|p| p == preset).unwrap_or(0)
        }
    }
}

fn set_res(d: &mut RecordingSettings, i: usize) {
    d.screen_resolution = match i {
        4 => match d.screen_resolution {
            ScreenResolution::Custom { .. } => d.screen_resolution.clone(),
            _ => ScreenResolution::Custom {
                width: 1920,
                height: 1080,
            },
        },
        _ => ScreenResolution::Preset {
            preset: PRESETS[i].clone(),
        },
    };
}

/// "At 1 snapshot/sec / original resolution." after a bitrate hint.
fn at_rate(d: &RecordingSettings) -> String {
    match &d.screen_resolution {
        ScreenResolution::Custom { .. } => String::new(),
        ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        } => {
            format!(
                " At {} / original resolution.",
                short_label(d.screen_frame_rate)
            )
        }
        ScreenResolution::Preset { .. } => {
            format!(
                " At {} / {}.",
                short_label(d.screen_frame_rate),
                RES[res_ix(&d.screen_resolution)]
            )
        }
    }
}

/// `parseCustomDimension`: digits only.
fn dim(raw: &str) -> Option<u32> {
    (!raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()))
        .then(|| raw.parse().ok())
        .flatten()
}

/// `customResolutionErrors` over the two raw fields.
pub fn resolution_errors(w: &str, h: &str) -> Vec<&'static str> {
    let mut e = Vec::new();
    let (pw, ph) = (dim(w), dim(h));
    if !w.is_empty() && pw.is_none() {
        e.push("Width must be an integer.");
    }
    if !h.is_empty() && ph.is_none() {
        e.push("Height must be an integer.");
    }
    if pw.is_some_and(|v| !(16..=8192).contains(&v)) {
        e.push("Width must be between 16 and 8192.");
    }
    if ph.is_some_and(|v| !(16..=8192).contains(&v)) {
        e.push("Height must be between 16 and 8192.");
    }
    if w.is_empty() || h.is_empty() {
        e.push("Both width and height are required for custom mode.");
    }
    e
}

/// `customBitrateErrors` over the raw Mbps field.
pub fn bitrate_errors(raw: &str) -> Vec<&'static str> {
    let raw = raw.trim();
    if raw.is_empty() {
        return vec!["Custom bitrate is required (1\u{2013}40 Mbps, whole number)."];
    }
    if !raw.bytes().all(|b| b.is_ascii_digit()) {
        return vec!["Bitrate must be a whole number of Mbps (e.g. 12)."];
    }
    match raw.parse::<u64>() {
        Ok(0) | Err(_) => vec!["Bitrate must be a positive whole number."],
        Ok(v) if v > 40 => vec!["Bitrate must not exceed 40 Mbps."],
        Ok(_) => Vec::new(),
    }
}

/// Both lists for the draft's modes (empty when the mode isn't Custom).
fn errors(s: &Settings, cx: &gpui_kit::App) -> (Vec<&'static str>, Vec<&'static str>) {
    let d = s.d();
    let raw = |e: &gpui_kit::Entity<gpui_kit::component::input::InputState>| {
        e.read(cx).value().to_string()
    };
    let res = match d.screen_resolution {
        ScreenResolution::Custom { .. } => {
            resolution_errors(&raw(&s.inputs.res_w), &raw(&s.inputs.res_h))
        }
        _ => Vec::new(),
    };
    let rate = match d.video_bitrate.mode {
        VideoBitrateMode::Custom => bitrate_errors(&raw(&s.inputs.mbps)),
        _ => Vec::new(),
    };
    (res, rate)
}

/// `customResolutionBlocked || customBitrateBlocked`: the Video unit waits.
pub fn video_invalid(s: &Settings, cx: &gpui_kit::App) -> bool {
    let (a, b) = errors(s, cx);
    !a.is_empty() || !b.is_empty()
}

/// Capture support (`get_capture_support`): `(pending, unsupported, only original)`.
fn support(s: &Settings) -> (bool, bool, bool) {
    let Some(v) = s.data.capture_support.as_ref() else {
        return (true, false, false);
    };
    let native = v["nativeCaptureSupported"].as_bool() == Some(true);
    let sck = v["supportedSources"]["systemAudio"].as_bool() == Some(true);
    (false, !native, native && !sck)
}

/// The coercion effect: on a Mac that only captures at Original, a screen
/// draft on another resolution goes back to Original.
pub fn coerce_resolution(s: &mut Settings, cx: &mut Context<Settings>) {
    let d = s.d();
    let (_, _, only_original) = support(s);
    if d.capture_screen && only_original && res_ix(&d.screen_resolution) != 0 {
        s.edit(|d| set_res(d, 0), cx);
    }
}

pub fn render(
    s: &Settings,
    p: &Palette,
    _: &mut Window,
    cx: &mut Context<Settings>,
) -> Vec<AnyElement> {
    let d = s.d();
    let current = ladder_interval(d.screen_frame_rate);
    let mut stops: Vec<f64> = STOPS.to_vec();
    if !stops.contains(&current) {
        stops.push(current);
        stops.sort_by(f64::total_cmp);
    }
    let stops_c = stops.clone();
    let rate = row("Screen Capture Rate")
        .desc("How often a snapshot of your screen is captured. More frequent snapshots produce larger files.")
        .control(
            Segmented::new(
                "capture-rate",
                "Screen capture rate",
                stops.iter().map(|i| seg_label(*i)),
                stops.iter().position(|i| *i == current).unwrap_or(0),
            )
            .md()
            .mono()
            .on_change(on(cx, move |s, i: usize, cx| {
                let fps = 1. / stops_c[i];
                s.edit(|d| d.screen_frame_rate = fps, cx)
            })),
        );

    let (pending, unsupported, only_original) = support(s);
    // `.resolution-*-notice`: a full-width tinted box, small text.
    let notice = |bg, bd, fg, ic: &'static str, msg: &'static str| {
        div()
            .flex()
            .items_start()
            .gap(px(8.))
            .w_full()
            .px(px(12.))
            .py(px(9.))
            .rounded(radius::XS)
            .border_1()
            .border_color(bd)
            .bg(bg)
            .text_color(fg)
            .child(div().flex_none().mt(px(1.)).child(icon(ic, px(11.))))
            .child(
                div()
                    .flex_1()
                    // Wraps inside the box instead of running past it.
                    .min_w_0()
                    .text_size(text::XS)
                    .tracking(0.02)
                    .line_height(gpui_kit::relative(1.55))
                    .child(msg),
            )
    };
    let support_line = if unsupported {
        notice(
            p.neutral_bg,
            p.neutral_border,
            p.neutral_text,
            icons::INFO,
            "Native screen capture is unsupported on this system. Resolution settings are saved, but only apply when native screen capture is available.",
        )
    } else if only_original {
        notice(
            p.neutral_bg,
            p.neutral_border,
            p.neutral_text,
            icons::INFO,
            "Scaled and custom resolutions require macOS 15 or later (ScreenCaptureKit). Only Original resolution is available on this system.",
        )
    } else if pending {
        notice(
            p.neutral_bg,
            p.neutral_border,
            p.neutral_text,
            icons::LOADER,
            "Checking capture support\u{2026} Scaled and custom resolutions are disabled until support is confirmed.",
        )
    } else {
        notice(
            p.accent_bg,
            p.accent_border,
            p.accent_strong,
            icons::CHECK,
            "Native capture supports scaled and custom output resolutions.",
        )
    };
    // `nonOriginalResolutionDisabled`: only while the screen is captured.
    let non_original_off = d.capture_screen && (pending || unsupported || only_original);
    let (res_errors, rate_errors) = errors(s, cx);
    let r_ix = res_ix(&d.screen_resolution);
    let mode_hint = match r_ix {
        0 => "Output files will match the captured display's native pixel dimensions.",
        4 => "Output will be scaled to the exact dimensions you specify.",
        _ => "Output will be scaled to the selected preset. Aspect ratio is preserved.",
    };
    let custom = (r_ix == 4).then(|| {
        div()
            .flex()
            .flex_wrap()
            .items_end()
            .gap(space::S2)
            .w_full()
            .child(field(
                "Width (px)",
                TextInput::new("res-width", "Width (px)", &s.inputs.res_w)
                    .num()
                    .invalid(!res_errors.is_empty()),
                p,
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(34.))
                    .text_size(text::BASE)
                    .font_weight(theme::W_SEMIBOLD)
                    .text_color(p.text_faint)
                    .child("\u{d7}"),
            )
            .child(field(
                "Height (px)",
                TextInput::new("res-height", "Height (px)", &s.inputs.res_h)
                    .num()
                    .invalid(!res_errors.is_empty()),
                p,
            ))
    });
    let res_warnings = res_errors.iter().map(|e| field_warning(*e, p));
    let resolution = row("Screen Resolution").full().control(
        div()
            .flex()
            .flex_col()
            .gap(space::S3)
            .w_full()
            .child(support_line)
            .child(
                div().flex().child(
                    Segmented::new("resolution", "Screen resolution", RES, r_ix)
                        .md()
                        .disabled_options(if non_original_off {
                            vec![1, 2, 3, 4]
                        } else {
                            Vec::new()
                        })
                        .on_change(on(cx, |s, i: usize, cx| {
                            s.edit(|d| set_res(d, i), cx);
                            s.video_invalid = video_invalid(s, cx);
                            s.tick(cx);
                        })),
                ),
            )
            .children(custom)
            .children(res_warnings)
            .child(hint(mode_hint, p))
            .when(r_ix == 2, |c| {
                c.child(hint("Code and small text may be hard to read at 720p.", p))
            }),
    );

    let b = &d.video_bitrate;
    let b_ix = match (&b.mode, &b.preset) {
        (VideoBitrateMode::Custom, _) => 3,
        (_, Some(VideoBitratePreset::Low)) => 0,
        (_, Some(VideoBitratePreset::High)) => 2,
        _ => 1,
    };
    let preset_hint = match b_ix {
        0 => "Low \u{2014} ~3 Mbps. Good for long sessions, minimal storage. Best for low-motion content or when disk space is limited.",
        2 => "High \u{2014} ~20 Mbps. Crisp detail and smooth motion at the cost of larger files. Ideal for high-motion content or final delivery.",
        _ => "Medium \u{2014} ~8 Mbps. Recommended default. Balanced quality and file size for most screen recordings.",
    };
    let bitrate_hint = if b_ix == 3 {
        b.custom_mbps.filter(|_| rate_errors.is_empty()).map(|m| {
            let q = match m {
                0..=2 => {
                    "Low quality \u{2014} may show compression artefacts on fast-moving content."
                }
                3..=12 => "Moderate quality \u{2014} good for most recordings.",
                13..=25 => "High quality \u{2014} suitable for detail-sensitive content.",
                _ => "Very high bitrate \u{2014} expect large output files.",
            };
            format!("Custom bitrate: {m} Mbps. {q}{}", at_rate(&d))
        })
    } else {
        Some(format!("{preset_hint}{}", at_rate(&d)))
    };
    let bitrate = row("Bitrate").full().control(
        div()
            .flex()
            .flex_col()
            .gap(space::S3)
            .w_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::S2)
                    .child(
                        div().w(px(220.)).child(
                            Select::new(
                                "bitrate",
                                "Bitrate",
                                [
                                    "Low \u{b7} ~3 Mbps",
                                    "Medium \u{b7} ~8 Mbps",
                                    "High \u{b7} ~20 Mbps",
                                    "Custom\u{2026}",
                                ],
                                Some(b_ix),
                            )
                            .on_change(on(cx, |s, i: usize, cx| {
                                s.edit(
                                    |d| {
                                        let b = &mut d.video_bitrate;
                                        if i == 3 {
                                            b.mode = VideoBitrateMode::Custom;
                                        } else {
                                            b.mode = VideoBitrateMode::Preset;
                                            b.preset = Some(
                                                [
                                                    VideoBitratePreset::Low,
                                                    VideoBitratePreset::Medium,
                                                    VideoBitratePreset::High,
                                                ][i]
                                                    .clone(),
                                            );
                                        }
                                    },
                                    cx,
                                );
                                s.video_invalid = video_invalid(s, cx);
                                s.tick(cx);
                            })),
                        ),
                    )
                    .when(b_ix == 3, |r| {
                        r.child(
                            Stepper::new("bitrate-mbps", "Bitrate (Mbps)", &s.inputs.mbps)
                                .range(Some(1), Some(40))
                                .unit("Mbps")
                                .invalid(!rate_errors.is_empty()),
                        )
                    }),
            )
            .children(rate_errors.iter().map(|e| field_warning(*e, p)))
            .children(bitrate_hint.map(|h| hint(h, p))),
    );
    let _ = text::BASE;
    vec![
        section("Video", None, Vec::new(), false, vec![rate.build(p).into_any_element(), resolution.build(p).into_any_element()], p)
            .into_any_element(),
        section(
            "Video Bitrate",
            Some("Bitrate controls how much data is encoded per second of video. Higher bitrate = sharper image but larger files and higher CPU/GPU load. Applied on macOS 15+ via ScreenCaptureKit; older systems fall back to the system-default bitrate."),
            Vec::new(),
            false,
            vec![bitrate.build(p).into_any_element()],
            p,
        )
        .into_any_element(),
    ]
}

/// `.custom-res-field`: grows from 120 px; a small uppercase label.
fn field(label: &'static str, input: TextInput, p: &Palette) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .flex_grow(1.)
        .flex_shrink(1.)
        .flex_basis(px(120.))
        .min_w_0()
        .child(
            div()
                .text_size(text::XS)
                .font_weight(theme::W_SEMIBOLD)
                .tracking(0.12)
                .text_color(p.text_muted)
                .child(label.to_uppercase()),
        )
        .child(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_snaps_to_the_ladder() {
        assert_eq!(ladder_interval(1.), 1.);
        assert_eq!(ladder_interval(0.5), 2.);
        assert_eq!(ladder_interval(0.45), 2.);
        assert_eq!(seg_label(0.1), "10/s");
        assert_eq!(seg_label(60.), "1/min");
        assert_eq!(short_label(1.), "1 snapshot/sec");
    }
}
