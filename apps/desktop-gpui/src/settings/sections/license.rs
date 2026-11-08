//! About › License & Trial (`License.svelte`, `licensing-panel.ts`): status
//! chip + hint, Refresh, Buy / Renew, and Activate.

use capture_types::{Activation, LicenseStatus};
use gpui_kit::prelude::*;
use gpui_kit::{div, AnyElement, Context};

use crate::settings::row::{line, row, section};
use crate::settings::{icons, Settings};
use crate::theme::{space, Palette};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::input::TextInput;
use crate::ui::{Button, Tone};

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `fmtLicenseDate`: "Nov 13, 2026".
fn date(ms: i64, cx: &gpui_kit::App) -> String {
    let now = crate::ui::clock::now(cx);
    time::OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .map(|t| {
            let t = t.to_offset(now.offset());
            format!(
                "{} {}, {}",
                MONTHS[t.month() as usize - 1],
                t.day(),
                t.year()
            )
        })
        .unwrap_or_default()
}

fn days(n: u32) -> String {
    format!("{n} {}", if n == 1 { "day" } else { "days" })
}

/// `badgeFor`.
fn badge(st: &LicenseStatus) -> Option<(&'static str, ChipKind)> {
    Some(match st {
        LicenseStatus::Licensed { activation, .. } => match activation {
            Activation::Pending { .. } => ("activating\u{2026}", ChipKind::Neutral),
            Activation::RefusedOverCap { .. } => ("device limit", ChipKind::Warn),
            Activation::Lapsed => ("not activated", ChipKind::Warn),
            _ => ("licensed", ChipKind::Accent),
        },
        LicenseStatus::Trial { .. } => ("trial", ChipKind::Info),
        LicenseStatus::TrialNotStarted { .. } => ("trial ready", ChipKind::Neutral),
        LicenseStatus::ReadOnly => ("read-only", ChipKind::Warn),
        LicenseStatus::Revoked => ("revoked", ChipKind::Warn),
    })
}

/// `statusHintFor` (without the device count, which needs `get_license_devices`).
fn hint(st: &LicenseStatus, cx: &gpui_kit::App) -> Option<String> {
    match st {
        LicenseStatus::TrialNotStarted { trial_days } => Some(format!("Free trial \u{2014} starts when you first record ({}).", days(*trial_days))),
        LicenseStatus::Trial { days_left, trial_end_ms } => Some(format!(
            "{} left \u{b7} ends {}. After that Mnema switches to Read-Only Mode \u{2014} your history stays searchable, only new recording pauses until you buy.",
            days(*days_left),
            date(*trial_end_ms, cx)
        )),
        LicenseStatus::Licensed { name, email, activation, in_window, update_through_ms } => {
            let who = if name.is_empty() { email.clone() } else { name.clone() };
            match activation {
                Activation::Pending { provisional_days_left } => Some(format!(
                    "Licensed to {who}. Finishing activation\u{2026} ({} to connect).",
                    days(*provisional_days_left)
                )),
                Activation::Activated => {
                    let mut parts = vec![format!("Licensed to {who}")];
                    if *in_window {
                        parts.push(format!("updates included through {}", date(*update_through_ms, cx)));
                    }
                    Some(format!("{}.", parts.join(" \u{b7} ")))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// `statusInlineFor`.
fn inline(st: &LicenseStatus, cx: &gpui_kit::App) -> Option<(Tone, &'static str, String)> {
    match st {
        LicenseStatus::ReadOnly => Some((Tone::Warn, icons::LOCK, "Trial ended \u{2014} Read-Only Mode. History stays searchable; buy to resume recording.".into())),
        LicenseStatus::Revoked => Some((
            Tone::Danger,
            icons::ALERT,
            "This license has been revoked \u{2014} Read-Only Mode. History stays searchable; new recording is paused.".into(),
        )),
        LicenseStatus::Licensed { activation: Activation::Lapsed, .. } => Some((
            Tone::Warn,
            icons::CLOUD_OFF,
            "We couldn\u{2019}t confirm your license \u{2014} connect to the internet once to finish activation. New recording is paused until then.".into(),
        )),
        LicenseStatus::Licensed { in_window: false, update_through_ms, .. } => Some((
            Tone::Warn,
            icons::INFO,
            format!("Update window lapsed {} \u{2014} you keep this version forever; renew for new builds.", date(*update_through_ms, cx)),
        )),
        _ => None,
    }
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let st = s.data.license.as_ref();
    let mut status = row("Status").desc_opt(st.and_then(|st| hint(st, cx)));
    if let Some((label, kind)) = st.and_then(badge) {
        status = status.extra(chip(label, kind, p));
    }
    status = match st {
        None => status.below(line(
            Tone::Neutral,
            icons::LOADER,
            "Checking license status\u{2026}",
            p,
        )),
        Some(st) => match inline(st, cx) {
            Some((tone, ic, text_)) => status.below(line(tone, ic, text_, p)),
            None => status,
        },
    };
    let out_of_window = matches!(
        st,
        Some(LicenseStatus::Licensed {
            in_window: false,
            ..
        })
    );
    let show_buy = !matches!(
        st,
        Some(LicenseStatus::Licensed {
            in_window: true,
            ..
        })
    );
    let mut rows = vec![
        status.build(p).into_any_element(),
        row("Refresh license status")
            .desc("Re-check with the license server \u{2014} a renewal or freed-up device lands right away.")
            // shortcut: refresh_license_now is app-core; its "Checked just now" line lands with
            // the license_status_changed event wiring (S13 follow-up).
            .control(Button::new("lic-refresh", "Refresh"))
            .build(p)
            .into_any_element(),
    ];
    if show_buy {
        rows.push(
            row(if out_of_window { "Renew" } else { "Buy Mnema" })
                .desc(if out_of_window {
                    "Renew ($29) to extend your update window for another year of new builds."
                } else {
                    "$69 one-time purchase \u{2014} includes a 1-year update window. Pay once, own it forever."
                })
                .control(Button::new("lic-buy", if out_of_window { "Renew ($29)" } else { "Buy ($69)" }).icon(icons::EXTERNAL).icon_after())
                .build(p)
                .into_any_element(),
        );
    }
    rows.push(
        row("Activate license")
            .desc("Paste the license key from your purchase email to unlock recording on this device.")
            .full()
            .control(
                div()
                    .flex()
                    .gap(space::S2)
                    .w_full()
                    .child(div().flex_1().child(TextInput::new("lic-key", "License key", &s.inputs.license_key).num()))
                    // shortcut: activate_license is app-core; wiring its result lines is an S13 follow-up.
                    .child(Button::new("lic-activate", "Activate").disabled(true)),
            )
            .build(p)
            .into_any_element(),
    );
    section(
        "License & Trial",
        Some("Buy once, keep it forever. Verified offline."),
        Vec::new(),
        false,
        rows,
        p,
    )
    .into_any_element()
}
