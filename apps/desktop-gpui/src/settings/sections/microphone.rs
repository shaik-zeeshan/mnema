//! Capture › Microphone (`Audio.svelte`): active device, preference, device
//! and the disconnect policy. Saves through the mic unit (250 ms).

use capture_types::{MicrophoneDisconnectPolicy, MicrophonePreferenceMode};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};

use crate::settings::row::{line, row, section};
use crate::settings::{icons, on, reload, Settings};
use crate::theme::{space, Palette};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::segmented::Segmented;
use crate::ui::select::Select;
use crate::ui::{Button, Tone};

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let Some(m) = s.mic_draft.clone() else {
        let failed = empty(
            EmptyLayout::Compact,
            Tone::Danger,
            icons::ALERT,
            "Couldn\u{2019}t load microphone state",
            "Devices and your preference will show here once it loads.",
            vec![Button::new("mic-retry", "Retry")
                .sm()
                .icon(icons::RETRY)
                .on_click(crate::settings::click(cx, |s, cx| s.reload_all(cx)))
                .into_any_element()],
            p,
        );
        return section(
            "Microphone",
            None,
            Vec::new(),
            false,
            vec![div().py(space::S3).child(failed).into_any_element()],
            p,
        )
        .into_any_element();
    };
    let mut rows = Vec::new();
    if m.devices.is_empty() {
        rows.push(
            div()
                .py(space::S3)
                .child(empty(
                    EmptyLayout::Compact,
                    Tone::Neutral,
                    icons::MIC,
                    "No microphone devices found",
                    "Plug one in or connect a headset \u{2014} it shows up here on its own.",
                    Vec::new(),
                    p,
                ))
                .into_any_element(),
        );
    } else {
        let active = match &m.effective_device {
            Some(dev) => chip(dev.name.clone(), ChipKind::Mic, p),
            None => chip("No active device", ChipKind::Neutral, p),
        };
        rows.push(
            row("Active device")
                .desc("The microphone currently in use.")
                .control(active)
                .control(reload::button(
                    s,
                    "reload-mic",
                    "Reload microphone state",
                    false,
                    |s, cx| s.reload_all(cx),
                    cx,
                ))
                .build(p)
                .into_any_element(),
        );
    }
    let specific = m.preference.mode == MicrophonePreferenceMode::SpecificDevice;
    rows.push(
        row("Preference")
            .desc("Which microphone capture should use.")
            .control(
                Segmented::new(
                    "mic-pref",
                    "Microphone preference",
                    ["System Default", "Specific Device"],
                    specific as usize,
                )
                .md()
                .on_change(on(cx, |s, i: usize, cx| {
                    s.edit_mic(
                        |m| {
                            m.preference.mode = if i == 1 {
                                MicrophonePreferenceMode::SpecificDevice
                            } else {
                                MicrophonePreferenceMode::Default
                            }
                        },
                        cx,
                    )
                })),
            )
            .build(p)
            .into_any_element(),
    );
    if specific {
        let needs = line(
            Tone::Warn,
            icons::ALERT,
            "Select a device before saving Specific Device mode.",
            p,
        );
        if !m.devices.is_empty() {
            let ids: Vec<String> = m.devices.iter().map(|d| d.id.clone()).collect();
            let sel = m
                .preference
                .device_id
                .as_ref()
                .and_then(|id| ids.iter().position(|i| i == id));
            rows.push(
                row("Device")
                    .desc("Pick the microphone to lock to.")
                    .control(
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap(px(6.))
                            .child(
                                div().w(px(240.)).child(
                                    Select::new(
                                        "mic-device",
                                        "Device",
                                        m.devices.iter().map(|d| d.name.clone()),
                                        sel,
                                    )
                                    .placeholder("Choose a microphone")
                                    .on_change(on(
                                        cx,
                                        move |s, i: usize, cx| {
                                            let id = ids[i].clone();
                                            s.edit_mic(|m| m.preference.device_id = Some(id), cx)
                                        },
                                    )),
                                ),
                            )
                            .when(sel.is_none(), |d| d.child(needs)),
                    )
                    .build(p)
                    .into_any_element(),
            );
        } else if m.preference.device_id.is_none() {
            rows.push(div().py(space::S3).child(needs).into_any_element());
        }
        let wait = m.disconnect_policy == MicrophoneDisconnectPolicy::WaitForSameDevice;
        rows.push(
            row("On Disconnect")
                .desc("What to do when the chosen microphone disconnects.")
                .control(
                    Segmented::new(
                        "mic-disconnect",
                        "On disconnect policy",
                        ["Fallback to Default", "Wait for Same Device"],
                        wait as usize,
                    )
                    .md()
                    .on_change(on(cx, |s, i: usize, cx| {
                        s.edit_mic(
                            |m| {
                                m.disconnect_policy = if i == 1 {
                                    MicrophoneDisconnectPolicy::WaitForSameDevice
                                } else {
                                    MicrophoneDisconnectPolicy::FallbackToDefault
                                }
                            },
                            cx,
                        )
                    })),
                )
                .build(p)
                .into_any_element(),
        );
    }
    section("Microphone", None, Vec::new(), false, rows, p).into_any_element()
}
