//! Capture & Storage arithmetic and copy (`lib/onboarding/{capture-sentence,
//! gates,disk-estimate}.ts`, `lib/components/capture-rate.ts`). The verdict the
//! panel prints and `block_reason` (the two hard gates) read the SAME
//! `need_bytes`, so the printed consequence and the held Continue can't disagree.

use capture_types::{RetentionPolicy, ScreenResolution, ScreenResolutionPreset};

use crate::settings::model::format_bytes;

/// The snapshot ladder, seconds between frames (backend bounds 1/60..=10 fps).
pub const LADDER: [f64; 11] = [0.1, 0.5, 1., 2., 3., 5., 10., 15., 30., 45., 60.];
/// 2 s = the 0.5 fps default.
pub const DEFAULT_INTERVAL_S: f64 = 2.;
/// `RESERVE_FLOOR_BYTES` in app-core `disk_space.rs` (1 GiB).
pub const RESERVE_FLOOR_BYTES: f64 = 1024. * 1024. * 1024.;
/// The measured anchor: 270 MB/day at one snapshot every 3 s, 720p.
const ANCHOR_INTERVAL_S: f64 = 3.;
const ANCHOR_MB_PER_DAY: f64 = 270.;
const ANCHOR_VIDEO_MB: f64 = 168.;
pub const ANCHOR_VIDEO_PIXELS: f64 = 1280. * 720.;
/// `~/.mnema`: the folder the backend resolves when the draft is blank.
pub const DEFAULT_FOLDER_LABEL: &str = "~/.mnema";

pub fn fps_to_interval(fps: f64) -> f64 {
    if fps > 0. {
        1. / fps
    } else {
        DEFAULT_INTERVAL_S
    }
}

/// Nearest ladder stop by log distance (a legacy 7.5 fps still lands somewhere).
pub fn nearest_ladder_index(fps: f64) -> usize {
    let target = fps_to_interval(fps).ln();
    LADDER
        .iter()
        .enumerate()
        .min_by(|a, b| {
            (a.1.ln() - target)
                .abs()
                .total_cmp(&(b.1.ln() - target).abs())
        })
        .map_or(0, |(i, _)| i)
}

/// "every 2 seconds", "once per minute", "10 times a second".
pub fn interval_phrase(s: f64) -> String {
    if s < 1. {
        format!("{} times a second", (1. / s).round())
    } else if s == 1. {
        "every second".into()
    } else if s == 60. {
        "once per minute".into()
    } else {
        format!("every {} seconds", trim(s))
    }
}

/// The rate segment's label: `10/s`, `2s`.
pub fn short_rate(s: f64) -> String {
    if s < 1. {
        format!("{}/s", (1. / s).round())
    } else {
        format!("{}s", trim(s))
    }
}

fn trim(v: f64) -> String {
    if v.fract() == 0. {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Pixels per frame under the draft resolution (`draftVideoPixels`).
pub fn video_pixels(res: &ScreenResolution, native: Option<f64>) -> f64 {
    let fallback = native.unwrap_or(1920. * 1080.);
    match res {
        ScreenResolution::Preset { preset } => match preset {
            ScreenResolutionPreset::P1080 => 1920. * 1080.,
            ScreenResolutionPreset::P720 => ANCHOR_VIDEO_PIXELS,
            ScreenResolutionPreset::P540 => 960. * 540.,
            ScreenResolutionPreset::Original => fallback,
        },
        ScreenResolution::Custom { width, height } if *width > 0 && *height > 0 => {
            *width as f64 * *height as f64
        }
        ScreenResolution::Custom { .. } => fallback,
    }
}

/// `estimateDailyStorageMb`: linear in rate; only the video share scales with pixels.
pub fn daily_mb(interval_s: f64, pixels: f64) -> f64 {
    let interval = if interval_s > 0. {
        interval_s
    } else {
        DEFAULT_INTERVAL_S
    };
    let video = ANCHOR_VIDEO_MB * (pixels / ANCHOR_VIDEO_PIXELS);
    let rest = ANCHOR_MB_PER_DAY - ANCHOR_VIDEO_MB;
    (((video + rest) * ANCHOR_INTERVAL_S / interval) * 10.).round() / 10.
}

pub fn daily_bytes(interval_s: f64, pixels: f64) -> f64 {
    daily_mb(interval_s, pixels) * 1e6
}

/// Everything the volume must hold: reserve + downloads + one day of capture.
pub fn need_bytes(downloads: f64, interval_s: f64, pixels: f64) -> f64 {
    RESERVE_FLOOR_BYTES + downloads + daily_bytes(interval_s, pixels)
}

/// A MEASURED figure: a measured zero is a full volume, not "unknown size".
pub fn measured(bytes: f64) -> String {
    if bytes > 0. {
        format_bytes(bytes)
    } else {
        "0 B".into()
    }
}

/// `never` first and default: the only setting whose wrong guess destroys data.
pub const RETENTION: [(RetentionPolicy, Option<u32>); 4] = [
    (RetentionPolicy::Never, None),
    (RetentionPolicy::Days30, Some(30)),
    (RetentionPolicy::Days14, Some(14)),
    (RetentionPolicy::Days7, Some(7)),
];

pub fn retention_index(p: &RetentionPolicy) -> usize {
    RETENTION.iter().position(|(r, _)| r == p).unwrap_or(0)
}

/// The probe of the chosen folder; `free: None` = unreadable (never blocks).
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    pub path: String,
    pub exists: bool,
    pub writable: bool,
    pub free: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeState {
    Checking,
    Done,
    Failed,
}

/// The two hard gates (`captureStorageBlockReason`). An inability to MEASURE
/// never blocks (ADR 0040): only a missing / read-only folder or a measured
/// shortfall does.
pub fn block_reason(
    probe: Option<&Probe>,
    downloads: f64,
    interval_s: f64,
    px: f64,
) -> Option<String> {
    let probe = probe?;
    if !probe.exists {
        return Some("That folder doesn't exist. Choose another folder.".into());
    }
    if !probe.writable {
        return Some("That folder is not writable. Choose another folder.".into());
    }
    let need = need_bytes(downloads, interval_s, px);
    let free = probe.free?;
    if free >= need {
        return None;
    }
    let figures = format!(
        "{} free \u{b7} {} needed.",
        measured(free),
        format_bytes(need)
    );
    Some(
        if downloads > 0. && free < RESERVE_FLOOR_BYTES + downloads {
            format!("Not enough room for the downloads. {figures}")
        } else {
            format!("Not enough room to record a day of capture. {figures}")
        },
    )
}

pub fn volume_label(path: &str) -> String {
    path.strip_prefix("/Volumes/")
        .and_then(|rest| rest.split('/').next())
        .filter(|v| !v.is_empty())
        .map_or_else(|| "your startup disk".into(), str::to_string)
}

/// Home-relative, middle dropped so the chosen folder stays readable.
pub fn sentence_path(path: &str, max: usize) -> String {
    let shown = match path.strip_prefix("/Users/") {
        Some(rest) => match rest.find('/') {
            Some(i) => format!("~{}", &rest[i..]),
            None => "~".into(),
        },
        None => path.to_string(),
    };
    if shown.chars().count() <= max {
        return shown;
    }
    let parts: Vec<&str> = shown.split('/').collect();
    if parts.len() <= 2 {
        let tail: String = shown
            .chars()
            .rev()
            .take(max - 1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return format!("\u{2026}{tail}");
    }
    let head = parts[0];
    let long = format!("{head}/\u{2026}/{}", parts[parts.len() - 2..].join("/"));
    if long.chars().count() > max {
        format!("{head}/\u{2026}/{}", parts[parts.len() - 1])
    } else {
        long
    }
}

fn plural(n: f64, unit: &str) -> String {
    format!("{} {unit}{}", trim(n), if n == 1. { "" } else { "s" })
}

/// "3 weeks" / "8 months" / "1.4 years".
pub fn human_duration(days: f64) -> String {
    if !days.is_finite() || days <= 0. {
        "no time at all".into()
    } else if days < 1. {
        "less than a day".into()
    } else if days < 14. {
        plural(days.round(), "day")
    } else if days < 60. {
        plural((days / 7.).round(), "week")
    } else if days < 365. {
        plural((days / 30.).round(), "month")
    } else {
        plural(((days / 365.) * 10.).round() / 10., "year")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seg {
    Text,
    Fig,
    Verdict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repair {
    Default,
    Pick,
    Recheck,
    NoSemantic,
    Slower,
    Tighten,
    Keep30,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    None,
    Ok,
    Warn,
    Bad,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub tone: Tone,
    pub probing: bool,
    pub clause: Option<String>,
    pub plan: Vec<(Seg, String)>,
    pub text: Vec<(Seg, String)>,
    pub repairs: Vec<(String, Repair)>,
    pub blocking: bool,
}

pub struct Input<'a> {
    pub interval_s: f64,
    pub pixels: f64,
    pub retention: &'a RetentionPolicy,
    pub probe: Option<&'a Probe>,
    pub state: ProbeState,
    pub downloads: f64,
    pub semantic_on: bool,
}

fn t(s: impl Into<String>) -> (Seg, String) {
    (Seg::Text, s.into())
}
fn fig(s: impl Into<String>) -> (Seg, String) {
    (Seg::Fig, s.into())
}
fn verd(s: impl Into<String>) -> (Seg, String) {
    (Seg::Verdict, s.into())
}
fn r(label: impl Into<String>, act: Repair) -> (String, Repair) {
    (label.into(), act)
}

/// Always `rate · horizon`: a first-year projection when nothing is deleted.
pub fn plan(interval_s: f64, retention: &RetentionPolicy, px: f64) -> Vec<(Seg, String)> {
    let daily = daily_bytes(interval_s, px);
    let (_, days) = RETENTION[retention_index(retention)];
    let mut v = vec![fig(format_bytes(daily)), t(" a day")];
    match days {
        None => v.extend([
            t(" \u{b7} about "),
            fig(format_bytes(daily * 365.)),
            t(" in the first year, and it keeps going"),
        ]),
        Some(d) => v.extend([
            t(" \u{b7} "),
            fig(format_bytes(daily * d as f64)),
            t(" held, then it stops growing"),
        ]),
    }
    v
}

/// `sentenceVerdict`: ordered exactly as `block_reason` tests its cases.
pub fn verdict(i: &Input) -> Verdict {
    let daily = daily_bytes(i.interval_s, i.pixels);
    let (_, days) = RETENTION[retention_index(i.retention)];
    let held = days.map(|d| daily * d as f64);
    let base = |tone, text, repairs| Verdict {
        tone,
        probing: false,
        clause: None,
        plan: plan(i.interval_s, i.retention, i.pixels),
        text,
        repairs,
        blocking: false,
    };
    let broken = |clause: String, text, repairs| Verdict {
        clause: Some(clause),
        blocking: true,
        ..base(Tone::Bad, text, repairs)
    };
    if i.state == ProbeState::Checking {
        return Verdict {
            probing: true,
            ..base(Tone::None, vec![], vec![])
        };
    }
    let Some(probe) = i.probe.filter(|_| i.state == ProbeState::Done) else {
        return base(
            Tone::Warn,
            vec![
                verd("Mnema couldn't check that folder."),
                t(" That doesn't block anything \u{2014} the plan above still holds, it just can't be checked against the disk."),
            ],
            vec![r("Re-check", Repair::Recheck)],
        );
    };
    let volume = volume_label(&probe.path);
    let use_default = format!("Use {DEFAULT_FOLDER_LABEL} instead");
    if probe.path.starts_with("/Volumes/") && !probe.exists && probe.free.is_none() {
        return broken(
            "that drive isn't connected right now.".into(),
            vec![t("Mnema won't record to a volume it can't see, and if it disappears mid-session recording stops until it's back.")],
            vec![r(use_default, Repair::Default), r("Re-check", Repair::Recheck)],
        );
    }
    if !probe.exists {
        let free = probe.free.map_or("Free space unknown".to_string(), |f| {
            format!("{} free", measured(f))
        });
        return broken(
            "that folder isn't there yet.".into(),
            vec![
                t(free),
                t(format!(" on {volume} \u{2014} only the folder is missing.")),
            ],
            vec![
                r("Choose another folder\u{2026}", Repair::Pick),
                r("Re-check", Repair::Recheck),
            ],
        );
    }
    if !probe.writable {
        return broken(
            format!("{volume} is read-only."),
            vec![t(
                "The write probe failed there \u{2014} proven, not guessed.",
            )],
            vec![
                r("Choose another folder\u{2026}", Repair::Pick),
                r(use_default, Repair::Default),
            ],
        );
    }
    let Some(free) = probe.free else {
        return base(
            Tone::Warn,
            vec![
                verd(format!("Free space on {volume} couldn't be read.")),
                t(" That doesn't block anything \u{2014} the plan above still holds, it just can't be checked against the disk."),
            ],
            vec![r("Re-check", Repair::Recheck)],
        );
    };
    let dl = i.downloads;
    let usable = free - dl - RESERVE_FLOOR_BYTES;
    let about = if i.semantic_on { "about " } else { "" };
    if free < need_bytes(dl, i.interval_s, i.pixels) {
        if dl > 0. && free < RESERVE_FLOOR_BYTES + dl {
            let mut repairs = vec![];
            if i.semantic_on {
                repairs.push(r("Turn Semantic Search off", Repair::NoSemantic));
            }
            repairs.push(r("Choose another folder\u{2026}", Repair::Pick));
            return broken(
                "there isn't room for the models yet.".into(),
                vec![
                    verd(format!(
                        "{} short before recording even starts.",
                        format_bytes(dl + RESERVE_FLOOR_BYTES - free)
                    )),
                    t(format!(" The downloads are {about}")),
                    fig(format_bytes(dl)),
                    t(format!("; {volume} has ")),
                    fig(measured(free)),
                    t(" free."),
                ],
                repairs,
            );
        }
        let mut text = vec![
            verd("The downloads fit; the capture does not."),
            t(format!(" After {about}")),
            fig(format_bytes(dl + RESERVE_FLOOR_BYTES)),
            t(" of models and safety reserve, "),
        ];
        if usable <= 0. {
            text.push(t("nothing is left"));
        } else {
            text.extend([t("only "), fig(format_bytes(usable)), t(" is left")]);
        }
        text.extend([
            t(" \u{2014} and one day costs "),
            fig(format_bytes(daily)),
            t("."),
        ]);
        return broken(
            "there isn't room for a day of recording.".into(),
            text,
            vec![
                r("Choose another folder\u{2026}", Repair::Pick),
                r("Record less often", Repair::Slower),
            ],
        );
    }
    match held {
        Some(held) if held > usable => base(
            Tone::Warn,
            vec![
                t(format!("Holding {} days takes about ", days.unwrap_or(0))),
                fig(format_bytes(held)),
                t(", and "),
                fig(format_bytes(usable)),
                t(format!(" is free on {volume}. ")),
                verd(format!(
                    "It fills up after about {}.",
                    human_duration(usable / daily)
                )),
            ],
            vec![
                r("Record less often", Repair::Slower),
                r("Keep less", Repair::Tighten),
            ],
        ),
        None => {
            let runout = usable / daily;
            let ok = runout >= 180.;
            base(
                if ok { Tone::Ok } else { Tone::Warn },
                vec![
                    t(format!("{} free on {volume} \u{2014} ", format_bytes(free))),
                    verd(format!(
                        "room for about {} at this rate",
                        human_duration(runout)
                    )),
                    t(", and nothing is deleted to make more."),
                ],
                if ok {
                    vec![]
                } else {
                    vec![
                        r("Keep the last 30 days", Repair::Keep30),
                        r("Record less often", Repair::Slower),
                    ]
                },
            )
        }
        Some(held) => base(
            Tone::Ok,
            vec![
                t(format!("{} free on {volume} \u{2014} ", format_bytes(free))),
                verd(format!(
                    "fits, with {} to spare",
                    format_bytes(usable - held)
                )),
                t(format!(
                    " (and it stops growing after {} days).",
                    days.unwrap_or(0)
                )),
            ],
            vec![],
        ),
    }
}
