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

/// `fmtLicenseDate`: "Nov 13, 2026"; `None` past the representable range
/// (a key with no update expiry carries `i64::MAX`).
fn date(ms: i64, offset: time::UtcOffset) -> Option<String> {
    let t = time::OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000).ok()?;
    let t = t.checked_to_offset(offset)?;
    Some(format!(
        "{} {}, {}",
        MONTHS[t.month() as usize - 1],
        t.day(),
        t.year()
    ))
}

fn offset(cx: &gpui_kit::App) -> time::UtcOffset {
    crate::ui::clock::now(cx).offset()
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
/// An empty value never leaves a dangling "through ." / "to .".
fn hint(st: &LicenseStatus, offset: time::UtcOffset) -> Option<String> {
    match st {
        LicenseStatus::TrialNotStarted { trial_days } => Some(format!("Free trial \u{2014} starts when you first record ({}).", days(*trial_days))),
        LicenseStatus::Trial { days_left, trial_end_ms } => Some(format!(
            "{} left{}. After that Mnema switches to Read-Only Mode \u{2014} your history stays searchable, only new recording pauses until you buy.",
            days(*days_left),
            date(*trial_end_ms, offset).map_or_else(String::new, |d| format!(" \u{b7} ends {d}"))
        )),
        LicenseStatus::Licensed { name, email, activation, in_window, update_through_ms } => {
            let who = [name, email].into_iter().map(|s| s.trim()).find(|s| !s.is_empty());
            let licensed = who.map_or_else(|| "Licensed".to_string(), |w| format!("Licensed to {w}"));
            match activation {
                Activation::Pending { provisional_days_left } => Some(format!(
                    "{licensed}. Finishing activation\u{2026} ({} to connect).",
                    days(*provisional_days_left)
                )),
                Activation::Activated => {
                    let mut parts = vec![licensed];
                    if *in_window {
                        // Svelte: "updates included" when the date is out of range.
                        parts.push(date(*update_through_ms, offset).map_or_else(
                            || "updates included".into(),
                            |d| format!("updates included through {d}"),
                        ));
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
            format!(
                "Update window lapsed{} \u{2014} you keep this version forever; renew for new builds.",
                date(*update_through_ms, offset(cx)).map_or_else(String::new, |d| format!(" {d}"))
            ),
        )),
        _ => None,
    }
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let st = s.data.license.as_ref();
    let mut status = row("Status").desc_opt(st.and_then(|st| hint(st, offset(cx))));
    if let Some((label, kind)) = st.and_then(badge) {
        status = status.extra(chip(label, kind, p));
    }
    status = match st {
        None if s.data.complete => status.below(line(
            Tone::Warn,
            icons::ALERT,
            "Couldn\u{2019}t read the license status.",
            p,
        )),
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
            .control(
                Button::new("lic-refresh", "Refresh")
                    .disabled(s.ui.busy.contains("License check"))
                    .on_click(crate::settings::click(cx, |s, cx| {
                        s.run_command("License check", cx, crate::settings::live::license_refresh)
                    })),
            )
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
                .control(
                    Button::new("lic-buy", if out_of_window { "Renew ($29)" } else { "Buy ($69)" })
                        .icon(icons::EXTERNAL)
                        .icon_after()
                        // `checkoutUrlFor`: lapsed owners renew, everyone else buys.
                        .on_click(move |_, _, cx| {
                            crate::ui::link::open(
                                if out_of_window {
                                    crate::small::update::RENEWAL_CHECKOUT_URL
                                } else {
                                    crate::licence::LICENSE_CHECKOUT_URL
                                },
                                cx,
                            );
                        }),
                )
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
                    .child(
                        Button::new("lic-activate", "Activate")
                            // Nothing to activate until a key is pasted.
                            .disabled(
                                s.ui.busy.contains("License activation")
                                    || s.inputs.license_key.read(cx).value().trim().is_empty(),
                            )
                            .on_click(crate::settings::commands::click_w(cx, |s, window, cx| {
                                let input = s.inputs.license_key.clone();
                                let key = crate::settings::commands::take_input(&input, window, cx);
                                if !key.is_empty() {
                                    s.run_command("License activation", cx, move |core| {
                                        crate::settings::live::license_activate(core, key)
                                    });
                                }
                            })),
                    ),
            )
            .build(p)
            .into_any_element(),
    );
    if matches!(st, Some(LicenseStatus::Licensed { .. })) {
        rows.push(
            row("Reset devices")
                .desc("Frees every device slot on this license, then activates this Mac again. Use it when you replaced a computer.")
                .control(
                    Button::new("lic-reset", "Reset devices\u{2026}")
                        .ghost()
                        .on_click(crate::settings::click(cx, |_, cx| {
                            crate::settings::confirm(
                                cx,
                                "Reset devices?",
                                "Every other device on this license is signed out and has to activate again.",
                                "Reset",
                                |s, cx| {
                                    s.run_command(
                                        "Device reset",
                                        cx,
                                        crate::settings::live::license_reset_devices,
                                    )
                                },
                            );
                        })),
                )
                .build(p)
                .into_any_element(),
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const UTC: time::UtcOffset = time::UtcOffset::UTC;

    fn licensed(name: &str, email: &str, through: i64) -> LicenseStatus {
        LicenseStatus::Licensed {
            update_through_ms: through,
            in_window: true,
            email: email.into(),
            name: name.into(),
            activation: Activation::Activated,
        }
    }

    #[test]
    fn hint_never_leaves_an_empty_value_dangling() {
        // Real app: a dev key with no update expiry (i64::MAX) read
        // "Licensed to Dev · updates included through ." (capture 13).
        assert_eq!(
            hint(&licensed("Dev", "", i64::MAX), UTC).as_deref(),
            Some("Licensed to Dev \u{b7} updates included.")
        );
        assert_eq!(
            hint(&licensed("Dev", "", 1_794_560_520_000), UTC).as_deref(),
            Some("Licensed to Dev \u{b7} updates included through Nov 13, 2026.")
        );
        // No name: the email (Svelte's `name || email`); neither: plain "Licensed".
        assert_eq!(
            hint(&licensed("", "a@b.c", i64::MAX), UTC).as_deref(),
            Some("Licensed to a@b.c \u{b7} updates included.")
        );
        assert_eq!(
            hint(&licensed(" ", "", i64::MAX), UTC).as_deref(),
            Some("Licensed \u{b7} updates included.")
        );
        let trial = LicenseStatus::Trial {
            days_left: 1,
            trial_end_ms: i64::MAX,
        };
        assert!(hint(&trial, UTC)
            .unwrap()
            .starts_with("1 day left. After that"));
        let trial = LicenseStatus::Trial {
            days_left: 9,
            trial_end_ms: 1_794_560_520_000,
        };
        assert!(hint(&trial, UTC)
            .unwrap()
            .starts_with("9 days left \u{b7} ends Nov 13, 2026."));
        for st in [licensed("", "", i64::MAX), trial] {
            let h = hint(&st, UTC).unwrap();
            assert!(
                !h.contains(" .") && !h.contains("to .") && !h.contains("through ."),
                "{h}"
            );
        }
    }
}
