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
use crate::ui::{notice, Button, Tone};

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
fn hint(
    st: &LicenseStatus,
    devices: Option<(u64, u64)>,
    offset: time::UtcOffset,
) -> Option<String> {
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
                    if let Some((used, cap)) = devices {
                        parts.push(format!("{used} of {cap} devices activated"));
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

const REFRESH: &str = "License check";
const ACTIVATE: &str = "License activation";
const RESET: &str = "Device reset";

/// `safeExternalUrl`: server-given links open only when they are https.
fn open_https(url: &str, cx: &mut gpui_kit::App) {
    if url.starts_with("https://") {
        crate::ui::link::open(url, cx);
    } else {
        log::warn!("settings: refused to open a non-https license link");
    }
}

impl Settings {
    /// Activate (or Enter in the key field): the key stays on a failure.
    pub fn activate_license(&mut self, cx: &mut Context<Self>) {
        let key = self.inputs.license_key.read(cx).value().trim().to_string();
        if key.is_empty() || self.ui.busy.contains(ACTIVATE) {
            return;
        }
        self.ui.lic_activate = None;
        self.run_slow_command_with(
            ACTIVATE,
            cx,
            move |core| crate::settings::live::license_activate(core, key),
            |s, outcome, _| {
                s.ui.lic_activate = Some(outcome.as_ref().map(|_| ()).map_err(Clone::clone));
                s.ui.clear_license_key = outcome.is_ok();
                true
            },
        );
    }

    /// Refresh: "Checked just now" or "Failed to reach the server." for 4 s.
    fn refresh_license(&mut self, cx: &mut Context<Self>) {
        self.ui.lic_refresh = None;
        self.run_slow_command_with(
            REFRESH,
            cx,
            crate::settings::live::license_refresh,
            |s, outcome, cx| {
                s.ui.lic_refresh = Some(outcome.is_ok());
                s.ui.lic_refresh_gen += 1;
                // Keep the line for 4 s; a newer check's line outlives this timer.
                let gen = s.ui.lic_refresh_gen;
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(4000))
                        .await;
                    let _ = this.update(cx, |s, cx| {
                        if s.ui.lic_refresh_gen == gen {
                            s.ui.lic_refresh = None;
                            cx.notify();
                        }
                    });
                })
                .detach();
                true
            },
        );
    }

    /// "Free up my devices…": asks, then resets the license's device list.
    fn free_up_devices(&mut self, cx: &mut Context<Self>) {
        crate::settings::confirm(
            cx,
            "Free up my devices",
            "Free up your devices? This clears the license's device list so this Mac can activate. You can do this once every 30 days.",
            "OK",
            |s, cx| {
                let Some(core) = crate::chat::core(cx) else {
                    return;
                };
                if !s.ui.busy.insert(RESET) {
                    return;
                }
                s.ui.lic_reset = None;
                let job = cx.background_spawn(async move {
                    crate::settings::live::license_reset_devices(&core)
                });
                cx.spawn(async move |this, cx| {
                    let result = job.await;
                    let _ = this.update(cx, |s, cx| {
                        s.ui.busy.remove(RESET);
                        let offset = offset(cx);
                        s.ui.lic_reset = Some(match result {
                            Ok(capture_types::ResetDevicesOutcome::Reset) => (
                                Tone::Neutral,
                                "Devices freed \u{2014} finishing activation on this Mac\u{2026}".into(),
                            ),
                            Ok(capture_types::ResetDevicesOutcome::RateLimited { retry_at_ms }) => (
                                Tone::Neutral,
                                match retry_at_ms.and_then(|ms| date(ms, offset)) {
                                    Some(d) => format!("Your devices were already reset recently \u{2014} you can reset again on {d}."),
                                    None => "Your devices were already reset recently \u{2014} you can reset again later.".into(),
                                },
                            ),
                            Err(e) => (Tone::Danger, e),
                        });
                        s.refresh(cx);
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
            },
        );
    }
}

/// The over-cap notice: free the slots here, buy another, or read the help.
fn over_cap(
    s: &Settings,
    reset_url: String,
    buy_url: String,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let resetting = s.ui.busy.contains(RESET);
    div()
        .flex()
        .flex_col()
        .gap(space::S2)
        .w_full()
        .child(notice::notice(
            Tone::Warn,
            icons::LOCK,
            "Already active on its 3 devices",
            Some(
                "Free them up to move it here (once every 30 days), or buy another license.".into(),
            ),
            vec![
                Button::new(
                    "lic-free-up",
                    if resetting {
                        "Freeing up\u{2026}"
                    } else {
                        "Free up my devices\u{2026}"
                    },
                )
                .sm()
                .busy(resetting)
                .disabled(resetting)
                .on_click(crate::settings::click(cx, |s, cx| s.free_up_devices(cx)))
                .into_any_element(),
                Button::new("lic-buy-another", "Buy another")
                    .ghost()
                    .sm()
                    .icon(icons::EXTERNAL)
                    .icon_after()
                    .on_click(move |_, _, cx| open_https(&buy_url, cx))
                    .into_any_element(),
                Button::new("lic-help", "Help")
                    .ghost()
                    .sm()
                    .icon(icons::EXTERNAL)
                    .icon_after()
                    .on_click(move |_, _, cx| open_https(&reset_url, cx))
                    .into_any_element(),
            ],
            p,
        ))
        .children(s.ui.lic_reset.clone().map(|(tone, words)| {
            let ic = if tone == Tone::Danger {
                icons::ALERT
            } else {
                icons::INFO
            };
            line(tone, ic, words, p)
        }))
        .into_any_element()
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let st = s.data.license.as_ref();
    let mut status =
        row("Status").desc_opt(st.and_then(|st| hint(st, s.data.license_devices, offset(cx))));
    if let Some((label, kind)) = st.and_then(badge) {
        status = status.extra(chip(label, kind, p));
    }
    let over = match st {
        Some(LicenseStatus::Licensed {
            activation: Activation::RefusedOverCap { reset_url, buy_url },
            ..
        }) => Some((reset_url.clone(), buy_url.clone())),
        _ => None,
    };
    status = match (st, over) {
        (_, Some((reset_url, buy_url))) => status
            .full()
            .control(over_cap(s, reset_url, buy_url, p, cx)),
        (None, _) if s.data.complete => status.below(line(
            Tone::Warn,
            icons::ALERT,
            "Couldn\u{2019}t read the license status.",
            p,
        )),
        (None, _) => status.below(line(
            Tone::Neutral,
            icons::LOADER,
            "Checking license status\u{2026}",
            p,
        )),
        (Some(st), None) => match inline(st, cx) {
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
    let refreshing = s.ui.busy.contains(REFRESH);
    let refresh_line = match s.ui.lic_refresh {
        Some(true) => Some(line(Tone::Ok, icons::CHECK, "Checked just now", p)),
        Some(false) => Some(line(
            Tone::Danger,
            icons::CLOUD_OFF,
            "Failed to reach the server.",
            p,
        )),
        None => None,
    };
    let mut rows = vec![
        status.build(p).into_any_element(),
        row("Refresh license status")
            .desc("Re-check with the license server \u{2014} a renewal or freed-up device lands right away.")
            .control(
                Button::new("lic-refresh", if refreshing { "Checking\u{2026}" } else { "Refresh" })
                    .busy(refreshing)
                    .disabled(refreshing)
                    .when(s.ui.lic_refresh == Some(false) && !refreshing, |b| b.icon(icons::RELOAD))
                    .on_click(crate::settings::click(cx, |s, cx| s.refresh_license(cx))),
            )
            .below_opt(refresh_line)
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
    let activating = s.ui.busy.contains(ACTIVATE);
    let activate_line = match &s.ui.lic_activate {
        Some(Err(e)) => Some(line(Tone::Danger, icons::ALERT, e.clone(), p)),
        Some(Ok(())) => Some(line(
            Tone::Ok,
            icons::CHECK,
            "License activated \u{2014} recording unlocked on this device.",
            p,
        )),
        None => None,
    };
    rows.push(
        row("Activate license")
            .desc("Paste the license key from your purchase email to unlock recording on this device.")
            .full()
            .control(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::S2)
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .gap(space::S2)
                            .w_full()
                            .child(div().flex_1().child(
                                TextInput::new("lic-key", "License key", &s.inputs.license_key)
                                    .invalid(matches!(s.ui.lic_activate, Some(Err(_))))
                                    .disabled(activating),
                            ))
                            .child(
                                Button::new("lic-activate", if activating { "Activating\u{2026}" } else { "Activate" })
                                    .busy(activating)
                                    // Nothing to activate until a key is pasted.
                                    .disabled(
                                        activating
                                            || s.inputs.license_key.read(cx).value().trim().is_empty(),
                                    )
                                    .on_click(crate::settings::click(cx, |s, cx| s.activate_license(cx))),
                            ),
                    )
                    .children(activate_line),
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

#[cfg(test)]
mod tests {
    use super::*;

    const UTC: time::UtcOffset = time::UtcOffset::UTC;

    fn hint_of(st: &LicenseStatus, offset: time::UtcOffset) -> Option<String> {
        hint(st, None, offset)
    }

    #[test]
    fn the_device_count_joins_an_activated_hint() {
        assert_eq!(
            hint(&licensed("Dev", "", i64::MAX), Some((2, 3)), UTC).as_deref(),
            Some("Licensed to Dev \u{b7} updates included \u{b7} 2 of 3 devices activated.")
        );
    }

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
            hint_of(&licensed("Dev", "", i64::MAX), UTC).as_deref(),
            Some("Licensed to Dev \u{b7} updates included.")
        );
        assert_eq!(
            hint_of(&licensed("Dev", "", 1_794_560_520_000), UTC).as_deref(),
            Some("Licensed to Dev \u{b7} updates included through Nov 13, 2026.")
        );
        // No name: the email (Svelte's `name || email`); neither: plain "Licensed".
        assert_eq!(
            hint_of(&licensed("", "a@b.c", i64::MAX), UTC).as_deref(),
            Some("Licensed to a@b.c \u{b7} updates included.")
        );
        assert_eq!(
            hint_of(&licensed(" ", "", i64::MAX), UTC).as_deref(),
            Some("Licensed \u{b7} updates included.")
        );
        let trial = LicenseStatus::Trial {
            days_left: 1,
            trial_end_ms: i64::MAX,
        };
        assert!(hint_of(&trial, UTC)
            .unwrap()
            .starts_with("1 day left. After that"));
        let trial = LicenseStatus::Trial {
            days_left: 9,
            trial_end_ms: 1_794_560_520_000,
        };
        assert!(hint_of(&trial, UTC)
            .unwrap()
            .starts_with("9 days left \u{b7} ends Nov 13, 2026."));
        for st in [licensed("", "", i64::MAX), trial] {
            let h = hint_of(&st, UTC).unwrap();
            assert!(
                !h.contains(" .") && !h.contains("to .") && !h.contains("through ."),
                "{h}"
            );
        }
    }
}
