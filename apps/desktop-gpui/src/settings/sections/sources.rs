//! Capture › Sources + Inactivity (`Capture.svelte`, `PermissionRows.svelte`).
//! The three families switch independently and lock while a recording runs.

use capture_types::AudioSpeechDetector;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};

use super::ix;
use crate::settings::row::{line, row, section, state_word, Src};
use crate::settings::{icons, on, reload, Settings};
use crate::theme::{space, Palette};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::choice::{Choice, ChoiceOption};
use crate::ui::segmented::Segmented;
use crate::ui::select::Select;
use crate::ui::slider::Slider;
use crate::ui::switch::Switch;
use crate::ui::{notice, Button, Tone};

const SYS_AUDIO_DESC: &str =
    "Capture Mac system audio (macOS 15+) \u{2014} meetings, videos, and app sound.";
const SEGMENT_STOPS: [u64; 3] = [60, 120, 300];
const IDLE_STOPS: [u64; 5] = [10, 30, 60, 120, 300];
const VAD: [AudioSpeechDetector; 3] = [
    AudioSpeechDetector::Silero,
    AudioSpeechDetector::Webrtc,
    AudioSpeechDetector::Off,
];

/// `sourcesHint`.
fn sources_hint(s: &Settings) -> &'static str {
    if s.status.loading_start {
        "Starting\u{2026}"
    } else if s.status.loading_stop {
        "Stopping\u{2026}"
    } else if s.status.running {
        "Stop recording to change sources."
    } else {
        "Three independent families \u{2014} any can run alone."
    }
}

/// A persisted off-ladder value stays listed as its own stop.
fn stops(base: &[u64], current: u64) -> Vec<u64> {
    let mut v: Vec<u64> = base.to_vec();
    if !v.contains(&current) {
        v.push(current);
    }
    v.sort_unstable();
    v
}

fn idle_label(s: u64) -> String {
    if s % 60 == 0 {
        format!("{} minute{}", s / 60, if s == 60 { "" } else { "s" })
    } else {
        format!("{s} seconds")
    }
}

/// The sensitivity bands (`LEVELS`).
fn level(mic: bool, v: u8) -> &'static str {
    const MIC: [&str; 5] = [
        "Very low \u{2014} only very loud audio triggers activity.",
        "Low \u{2014} only louder audio keeps capture active.",
        "Medium \u{2014} normal speech triggers activity. Recommended.",
        "High \u{2014} quiet speech counts as activity.",
        "Very high \u{2014} whispers and background noise keep capture active.",
    ];
    const SYS: [&str; 5] = [
        "Very low \u{2014} only very loud system audio triggers activity.",
        "Low \u{2014} only louder system audio keeps capture active.",
        "Medium \u{2014} typical media playback triggers activity. Recommended.",
        "High \u{2014} moderate system audio counts as activity.",
        "Very high \u{2014} quiet system sounds keep capture active.",
    ];
    let i = (v / 20).min(4) as usize;
    if mic {
        MIC[i]
    } else {
        SYS[i]
    }
}

fn permission_rows(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> Vec<AnyElement> {
    let Some(perms) = s.data.permissions.as_ref() else {
        return Vec::new();
    };
    let mut rows: Vec<AnyElement> = Vec::new();
    rows.extend([("screen", "Screen Recording", "record the screen"), ("microphone", "Microphone", "record the microphone")]
        .into_iter()
        .filter_map(|(key, label, what)| {
            let st = perms[key].as_str().unwrap_or("not_determined");
            let r = row(format!("{label} permission")).desc(format!("macOS access Mnema needs to {what}."));
            let busy = s.ui.perm_busy.is_some();
            let error = s.ui.perm_error.as_ref().filter(|(k, _)| *k == key).map(|(_, e)| line(Tone::Danger, icons::ALERT, e.clone(), p));
            let el = match st {
                "unsupported" => return None,
                "granted" => r.control(div().flex().items_center().gap(px(8.)).children(error).child(state_word("allowed", Some(Tone::Ok), p))),
                "denied" | "restricted" => r.full().control(div().flex().flex_col().gap(px(8.)).w_full().child(notice::notice(
                    Tone::Warn,
                    icons::LOCK,
                    format!("{label} \u{b7} not allowed"),
                    Some(format!("Mnema can\u{2019}t {what} until you allow it under Privacy & Security \u{2192} {label}.").into()),
                    // The pane plus the floating helper, as onboarding's button does.
                    vec![Button::new(format!("perm-open-{key}"), "Open System Settings")
                        .sm()
                        .icon(icons::EXTERNAL)
                        .icon_after()
                        .disabled(busy)
                        .on_click(crate::settings::click(cx, move |_, cx| {
                            crate::small::helper::open(key, cx)
                        }))
                        .into_any_element()],
                    p,
                )).children(error)),
                _ => r.control(
                    div().flex().items_center().gap(px(8.)).children(error).child(
                    Button::new(format!("perm-grant-{key}"), "Grant access")
                        .disabled(busy)
                        .busy(s.ui.perm_busy == Some(key))
                        .on_click(crate::settings::click(cx, move |s, cx| s.request_permission(key, cx)))),
                ),
            };
            Some(el.build(p).into_any_element())
        }));
    rows
}

/// System Audio (ADR 0052): "may be blocked" is inferred, never known, so its
/// hint sits in the row only while the switch is on, and can be dismissed.
fn system_audio_row(
    s: &Settings,
    locked: bool,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let d = s.d();
    let switch = Switch::new("src-sys", "System Audio", d.capture_system_audio)
        .disabled(locked)
        .on_color(Src::SysAudio.color(p))
        .on_toggle(on(cx, |s, v: bool, cx| {
            s.edit(|d| d.capture_system_audio = v, cx)
        }));
    let r = row("System Audio").desc(SYS_AUDIO_DESC).src(Src::SysAudio);
    let shown = s.data.system_audio_blocked && !s.ui.sysaudio_dismissed && d.capture_system_audio;
    if !shown {
        return r.control(switch).build(p).into_any_element();
    }
    let hint = notice::notice(
        Tone::Info,
        icons::LOCK,
        "No system sound yet",
        Some("System audio is on, but no recording has captured a sound yet. If you denied the macOS prompt, allow Mnema under Privacy & Security \u{2192} Screen & System Audio Recording. If your Mac has simply been quiet, ignore this.".into()),
        vec![
            Button::new("perm-open-systemAudio", "Open System Settings")
                .sm()
                .icon(icons::EXTERNAL)
                .icon_after()
                .on_click(crate::settings::click(cx, |_, cx| crate::small::helper::open("systemAudio", cx)))
                .into_any_element(),
            Button::new("sysaudio-dismiss", "Dismiss")
                .ghost()
                .sm()
                .on_click(crate::settings::click(cx, |s, cx| s.dismiss_system_audio_hint(cx)))
                .into_any_element(),
        ],
        p,
    );
    r.full()
        .extra(chip("may be blocked", ChipKind::Info, p))
        .aside(switch)
        .control(
            div()
                .flex()
                .flex_col()
                .gap(space::S2)
                .w_full()
                .child(hint)
                .children(
                    s.ui.sysaudio_error
                        .clone()
                        .map(|e| line(Tone::Warn, icons::INFO, e, p)),
                ),
        )
        .build(p)
        .into_any_element()
}

fn sensitivity(s: &Settings, mic: bool, p: &Palette) -> AnyElement {
    let d = s.d();
    let (on_, v, label) = if mic {
        (
            d.capture_microphone,
            d.microphone_activity_sensitivity,
            "Microphone activity sensitivity",
        )
    } else {
        (
            d.capture_system_audio,
            d.system_audio_activity_sensitivity,
            "System audio activity sensitivity",
        )
    };
    let state = if mic { &s.sliders.mic } else { &s.sliders.sys };
    let slider = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .w(px(220.))
        .child(
            div()
                .flex_1()
                .child(Slider::new(state, label).disabled(!on_)),
        )
        .child(
            div()
                .w(px(34.))
                .flex()
                .justify_end()
                .text_size(crate::theme::text::SM)
                .text_color(p.text_muted)
                .child(format!("{v}%")),
        );
    let r = row(label)
        .desc(level(mic, v))
        .src(if mic { Src::Mic } else { Src::SysAudio });
    let r = if on_ {
        r.control(slider)
    } else {
        let what = if mic { "Microphone" } else { "System audio" };
        r.full().aside(slider).control(line(
            Tone::Warn,
            icons::INFO,
            format!("{what} capture is off \u{2014} this has no effect until you turn it on."),
            p,
        ))
    };
    r.build(p).into_any_element()
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> Vec<AnyElement> {
    let d = s.d();
    let locked = s.status.running || s.status.loading_start || s.status.loading_stop;
    let src = |id: &'static str,
               label: &'static str,
               desc: &'static str,
               on_: bool,
               k: Src,
               f: fn(&mut capture_types::RecordingSettings, bool)| {
        row(label)
            .desc(desc)
            .src(k)
            .control(
                Switch::new(id, label, on_)
                    .disabled(locked)
                    .on_color(k.color(p))
                    .on_toggle(on(cx, move |s, v: bool, cx| s.edit(|d| f(d, v), cx))),
            )
            .build(p)
            .into_any_element()
    };
    let mut rows = vec![
        src(
            "src-screen",
            "Screen",
            "Capture the display.",
            d.capture_screen,
            Src::Screen,
            |d, v| d.capture_screen = v,
        ),
        src(
            "src-mic",
            "Microphone",
            "Capture audio from microphone.",
            d.capture_microphone,
            Src::Mic,
            |d, v| d.capture_microphone = v,
        ),
    ];
    rows.push(system_audio_row(s, locked, p, cx));
    rows.extend(permission_rows(s, p, cx));
    // `recValidationErrors`: the save-block reasons, said where they apply.
    if !d.capture_screen && !d.capture_microphone && !d.capture_system_audio {
        rows.insert(
            0,
            div()
                .my(space::S3)
                .child(notice::notice(
                    Tone::Danger,
                    icons::ALERT,
                    "Unsaved changes blocked",
                    Some("At least one capture source (Screen, Microphone, or System Audio) must be enabled.".into()),
                    Vec::new(),
                    p,
                ))
                .into_any_element(),
        );
    }
    let seg = stops(&SEGMENT_STOPS, d.segment_duration_seconds);
    let seg_labels: Vec<String> = seg
        .iter()
        .map(|s| {
            if s % 60 == 0 {
                format!("{}m", s / 60)
            } else {
                format!("{s}s")
            }
        })
        .collect();
    let seg_c = seg.clone();
    rows.push(
        row("Segment Duration")
            .desc("How long each recording segment is before a new one starts.")
            .control(
                Segmented::new(
                    "segment-duration",
                    "Segment duration",
                    seg_labels,
                    ix(&seg, &d.segment_duration_seconds),
                )
                .md()
                .mono()
                .on_change(on(cx, move |s, i: usize, cx| {
                    let v = seg_c[i];
                    s.edit(|d| d.segment_duration_seconds = v, cx)
                })),
            )
            .build(p)
            .into_any_element(),
    );
    let sources = section(
        "Sources",
        Some(sources_hint(s)),
        vec![reload::button(
            s,
            "reload-capture",
            "Reload capture settings",
            false,
            |s, cx| s.discard_drafts(cx),
            cx,
        )],
        false,
        rows,
        p,
    );

    let idle = stops(&IDLE_STOPS, d.idle_timeout_seconds);
    let idle_c = idle.clone();
    let mut inactivity = vec![row("Pause capture when idle")
        .desc("Pause after the whole Mac has been idle (no mouse, keyboard or other input), and resume when activity is detected.")
        .when(d.pause_capture_on_inactivity, |r| {
            r.control(
                Select::new("idle-timeout", "Idle timeout", idle.iter().map(|s| idle_label(*s)), Some(ix(&idle, &d.idle_timeout_seconds)))
                    .on_change(on(cx, move |s, i: usize, cx| {
                        let v = idle_c[i];
                        s.edit(|d| d.idle_timeout_seconds = v, cx)
                    })),
            )
        })
        .control(
            Switch::new("pause-idle", "Pause capture when idle", d.pause_capture_on_inactivity)
                .on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.pause_capture_on_inactivity = v, cx))),
        )
        .build(p)
        .into_any_element()];
    if d.pause_capture_on_inactivity {
        inactivity.push(
            div()
                .py(px(14.))
                .border_b_1()
                .border_color(p.hairline)
                .child(notice::notice(
                    Tone::Neutral,
                    icons::WAVE,
                    "Activity sources",
                    Some("Keyboard and mouse input, on-screen changes, and source-specific audio. The microphone counts speech when voice detection is on; system audio uses its level threshold below.".into()),
                    Vec::new(),
                    p,
                ))
                .into_any_element(),
        );
        let opts = vec![
            ChoiceOption::new("Silero")
                .tag("default")
                .icon(icons::WAVE)
                .desc("Neural speech detector. Falls back to WebRTC when unavailable.")
                .disabled(!d.capture_microphone),
            ChoiceOption::new("WebRTC")
                .icon(icons::FEATHER)
                .desc("Lightweight local speech detector.")
                .disabled(!d.capture_microphone),
            ChoiceOption::new("Off")
                .icon(icons::OFF)
                .desc("Use the microphone peak level only.")
                .disabled(!d.capture_microphone),
        ];
        let vad = d.audio_speech_detection.detector.clone();
        inactivity.push(
            row("Microphone Voice Detection")
                .desc("Decides when the microphone counts as active.")
                .full()
                .control(
                    div().w_full().flex().flex_col().gap(px(8.)).child(
                        Choice::new("vad", "Microphone voice detection", opts, ix(&VAD, &vad))
                            .on_change(on(cx, |s, i: usize, cx| s.edit(|d| d.audio_speech_detection.detector = VAD[i].clone(), cx))),
                    ).when(!d.capture_microphone, |c| c.child(line(Tone::Warn, icons::INFO, "Microphone capture is disabled \u{2014} voice detection has no effect until enabled.", p))),
                )
                .build(p)
                .into_any_element(),
        );
        if vad == AudioSpeechDetector::Off {
            inactivity.push(sensitivity(s, true, p));
        }
        inactivity.push(sensitivity(s, false, p));
        let both = match (d.capture_microphone, d.capture_system_audio) {
            (false, false) => "Neither microphone nor system audio capture is enabled \u{2014} audio activity detection will not function. Enable at least one source in Capture Sources above.",
            (false, true) => "Microphone capture is disabled \u{2014} only system audio is monitored for activity.",
            (true, false) => "System audio capture is disabled \u{2014} only microphone audio is monitored for activity.",
            (true, true) => "Both microphone and system audio are monitored independently for activity.",
        };
        inactivity.push(
            div()
                .pt(px(14.))
                .child(line(Tone::Neutral, icons::INFO, both, p))
                .into_any_element(),
        );
    }
    vec![
        sources.into_any_element(),
        section(
            "Inactivity",
            Some("Pause and resume rules when you step away."),
            Vec::new(),
            false,
            inactivity,
            p,
        )
        .into_any_element(),
    ]
}
