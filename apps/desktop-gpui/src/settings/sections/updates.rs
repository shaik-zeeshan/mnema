//! About › Updates (`About.svelte`): the identity hero + Copy details, the
//! update status panel + Check / Install, and the third-party notices.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};
use serde_json::Value;

use crate::settings::model::{short_datetime, Json};
use crate::settings::row::{hint, row, section};
use crate::settings::{click, Settings};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO, FONT_UI};
use crate::ui::baseline::{self, Line};
use crate::ui::chip::{chip, chip_line, ChipKind};
use crate::ui::link::Link;
use crate::ui::Tracking as _;
use crate::ui::{nid, Button, CopyButton};

/// `appUpdateStateLabel`.
fn state_label(u: Option<&Value>) -> &'static str {
    match u.and_then(|u| u.str_at(&["state"])) {
        None => "Loading",
        Some("idle") => "Not checked",
        Some("checking") => "Checking",
        Some("upToDate") => "Up to date",
        Some("available") => "Update available",
        Some("availableOutOfWindow") => "Outside update window",
        Some("downloading") => "Downloading",
        Some("installing") => "Installing",
        Some("restartRequired") => "Restart required",
        Some("incompatible") => "Incompatible",
        Some("failed") => "Failed",
        Some(_) => "Unknown",
    }
}

/// `appUpdateStatusMessage` (the common states).
fn message(u: Option<&Value>) -> String {
    let Some(u) = u else {
        return "Loading update status.".into();
    };
    if let Some(m) = u.str_at(&["error", "message"]) {
        return m.into();
    }
    match u.str_at(&["state"]) {
        Some("idle") => "Mnema has not checked for updates in this app session.".into(),
        Some("checking") => "Checking the selected update channel.".into(),
        Some("upToDate") => "Mnema is current on the selected channel.".into(),
        Some("available") => format!(
            "Version {} is ready to download and install. Recording stops automatically for the install.",
            u.str_at(&["update", "version"]).unwrap_or("newer")
        ),
        Some("availableOutOfWindow") => match u.str_at(&["update", "version"]) {
            Some(v) => format!("Version {v} is past your update window. Renew to receive new builds \u{2014} your current version keeps working forever."),
            None => "You're on a build newer than your update window. Get the newest build your license covers, or renew to receive new builds. Your recordings are untouched.".into(),
        },
        Some("downloading") => "Downloading the update package.".into(),
        Some("installing") => "Installing the update. Keep Mnema open until this finishes.".into(),
        Some("restartRequired") => "Restart Mnema when you are ready to finish updating.".into(),
        Some("incompatible") => "No compatible update is available for this Mac.".into(),
        Some("failed") => "The last update operation failed. You can retry.".into(),
        _ => "Update status is unavailable.".into(),
    }
}

/// `About.svelte`'s copy buttons: the label reads "Copied" for 2 s.
fn copy_button(
    id: &'static str,
    label: &'static str,
    aria: &'static str,
    text_: String,
) -> CopyButton {
    CopyButton::label(id, label, text_)
        .aria(aria)
        .revert(std::time::Duration::from_millis(2000))
}

const RESTART: &str = "Update restart";

impl Settings {
    /// "Restart to Update" (`restart_after_app_update`); a failure stays here.
    fn restart_after_update(&mut self, cx: &mut Context<Self>) {
        let Some(core) = crate::chat::core(cx) else {
            return;
        };
        if !self.ui.busy.insert(RESTART) {
            return;
        }
        self.ui.update_error = None;
        let job = cx.background_spawn(async move {
            app_core::updates::restart_after_app_update(&core).map_err(|e| e.message)
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |s, cx| {
                s.ui.busy.remove(RESTART);
                if let Err(e) = result {
                    s.ui.update_error = Some(e);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> Vec<AnyElement> {
    let u = s.data.update.as_ref();
    let app = u.map(|u| u.at(&["app"]).clone()).unwrap_or_default();
    let version = app.str_at(&["version"]).map(str::to_string);
    let channel = if u.and_then(|u| u.str_at(&["channel"])) == Some("preview") {
        "Preview"
    } else {
        "Stable"
    };
    let platform = match (app.str_at(&["platform"]), app.str_at(&["arch"])) {
        (Some(os), Some(arch)) => format!(
            "{} \u{b7} {}",
            if os == "macos" { "macOS" } else { os },
            if arch == "aarch64" {
                "Apple Silicon"
            } else {
                arch
            }
        ),
        _ => "Unknown".into(),
    };
    let details = format!(
        "{} {} ({}/{}) {}",
        app.str_at(&["productName"]).unwrap_or("mnema"),
        version.as_deref().unwrap_or("unknown"),
        app.str_at(&["platform"]).unwrap_or("unknown"),
        app.str_at(&["arch"]).unwrap_or("unknown"),
        app.str_at(&["identifier"]).unwrap_or("unknown"),
    );
    // `.about-meta__row`: the label and the value share one baseline.
    let meta = |k: &'static str, v: String| {
        baseline::row_top(vec![
            (
                Line::new(FONT_MONO, text::XS, baseline::PHI),
                div()
                    .w(px(84.))
                    .tracking(0.1)
                    .text_color(p.text_subtle)
                    .child(k.to_uppercase()),
            ),
            (
                Line::new(FONT_UI, text::BASE, baseline::PHI),
                div().text_color(p.text).child(v),
            ),
        ])
        .gap(space::S3)
    };
    let hero = div()
        .flex()
        .flex_col()
        .gap(space::S3)
        .py(space::S4)
        .border_b_1()
        .border_color(p.hairline)
        .child(
            // `.about-hero__head`: the version chip's text on the name's baseline.
            baseline::row(vec![
                (
                    Line::new(FONT_UI, text::XL, baseline::PHI),
                    div()
                        .font_weight(theme::W_SEMIBOLD)
                        .tracking(0.01)
                        .text_color(p.text_strong)
                        .child("Mnema")
                        .into_any_element(),
                ),
                (
                    chip_line(),
                    div()
                        .flex()
                        .items_center()
                        .gap(space::S3)
                        .child(match version.as_ref() {
                            // `.about-id__version`.
                            Some(v) => chip(format!("v{v}"), ChipKind::Accent, p)
                                .rounded(radius::PILL)
                                .tracking(0.01),
                            None => chip("checking\u{2026}", ChipKind::Neutral, p)
                                .rounded(radius::PILL)
                                .tracking(0.01),
                        })
                        .child(
                            // `.badge`.
                            chip(format!("{channel} channel").to_uppercase(), ChipKind::Neutral, p)
                                .rounded(radius::PILL)
                                .tracking(0.08),
                        )
                        .into_any_element(),
                ),
            ])
            .gap(space::S3),
        )
        .child(hint(
            "Your memory, on rewind. Mnema records your screen so you can scrub back to anything you've seen: searchable, local, and yours.",
            p,
        ))
        .child(meta("Platform", platform))
        .child(meta("Identifier", app.str_at(&["identifier"]).unwrap_or("Unknown").to_string()));
    let about = section(
        "About",
        Some("Version, build details, and the projects Mnema is built on."),
        Vec::new(),
        false,
        vec![
            hero.into_any_element(),
            row("Copy details")
                .desc("Copy version and build details to the clipboard.")
                .control(copy_button(
                    "about-copy",
                    "Copy details",
                    "Copy version and build details to the clipboard",
                    details,
                ))
                .build(p)
                .into_any_element(),
        ],
        p,
    );

    let now = crate::ui::clock::now(cx);
    let checked = u
        .and_then(|u| u.num_at(&["lastCheckedAtUnixMs"]))
        .map_or("Not checked yet".to_string(), |ms| {
            short_datetime(ms as i64, now)
        });
    let state = u.and_then(|u| u.str_at(&["state"]));
    // The updater publishes `checking` / `downloading` / `installing` itself.
    let checking = state == Some("checking");
    let installing = matches!(state, Some("downloading" | "installing"));
    let check_off = checking || installing || state == Some("restartRequired");
    let failed = matches!(state, Some("failed" | "incompatible"));
    let release = u
        .and_then(|u| u.str_at(&["update", "date"]))
        .map(crate::small::update::model::release_date);
    let notes = u
        .and_then(|u| u.str_at(&["update", "notes"]))
        .filter(|n| !n.trim().is_empty())
        .map(str::to_string);
    let progress = u.filter(|u| !u.at(&["progress"]).is_null()).map(|_| {
        let pct = crate::small::update::model::progress_percent(u).unwrap_or(8.);
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .h(px(4.))
                    .w_full()
                    .rounded(radius::PILL)
                    .bg(p.wash_strong)
                    .child(
                        div()
                            .h_full()
                            .w(gpui_kit::relative(pct / 100.))
                            .rounded(radius::PILL)
                            .bg(p.accent),
                    ),
            )
            .child(hint(crate::small::update::model::progress_text(u), p))
    });
    let action = if state == Some("restartRequired") {
        let restarting = s.ui.busy.contains(RESTART);
        Button::new(
            "update-restart",
            if restarting {
                "Restarting"
            } else {
                "Restart to Update"
            },
        )
        .busy(restarting)
        .disabled(restarting)
        .on_click(click(cx, |s, cx| s.restart_after_update(cx)))
    } else {
        Button::new(
            "update-install",
            if installing {
                "Installing"
            } else {
                "Install Update"
            },
        )
        .busy(installing)
        .disabled(!crate::small::update::model::can_install(u, checking))
        .on_click(click(cx, |_, cx| crate::small::update::install(false, cx)))
    };
    let headline = u
        .and_then(|u| u.str_at(&["update", "version"]))
        .map(|v| format!("Version {v}"))
        .or(version.clone())
        .unwrap_or_else(|| "Mnema".into());
    let panel = div()
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
                    chip(state_label(u), ChipKind::Neutral, p)
                        .rounded(radius::PILL)
                        .tracking(0.08),
                )
                .child(
                    Button::new(
                        "update-check",
                        if checking {
                            "Checking"
                        } else {
                            "Check for Updates"
                        },
                    )
                    .sm()
                    .busy(checking)
                    .disabled(check_off)
                    .on_click(click(cx, |_, cx| crate::small::update::check(cx))),
                ),
        )
        .child(
            div()
                .flex()
                .items_start()
                .gap(space::S3)
                .p(space::S3)
                .rounded(radius::MD)
                .border_1()
                .border_color(if failed { p.danger_border } else { p.border })
                .bg(if failed {
                    p.danger_bg_soft
                } else {
                    p.surface_subtle
                })
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .text_size(text::MD)
                                .font_weight(theme::W_SEMIBOLD)
                                .text_color(p.text_strong)
                                .child(headline),
                        )
                        .child(
                            div()
                                .text_size(text::BASE)
                                .text_color(p.text_muted)
                                .child(message(u)),
                        )
                        .child(
                            div()
                                .text_size(text::SM)
                                .text_color(p.text_subtle)
                                .child(format!("Last checked: {checked}")),
                        ),
                )
                .children(release.map(|r| {
                    div()
                        .flex_none()
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .child(r)
                })),
        )
        .children(progress)
        .children(notes.map(|n| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(text::SM)
                        .font_weight(theme::W_SEMIBOLD)
                        .text_color(p.text_subtle)
                        .child("Release notes"),
                )
                .child(div().text_size(text::BASE).text_color(p.text).child(n))
        }))
        .child(div().child(action))
        .children(s.ui.update_error.clone().map(|e| {
            div()
                .flex()
                .items_center()
                .gap(space::S2)
                .child(crate::settings::row::line(
                    crate::ui::Tone::Danger,
                    crate::settings::icons::ALERT,
                    e,
                    p,
                ))
                .child(
                    Button::icon_only(
                        "update-error-dismiss",
                        crate::settings::icons::X,
                        "Dismiss error",
                    )
                    .ghost()
                    .sm()
                    .on_click(click(cx, |s, cx| {
                        s.ui.update_error = None;
                        cx.notify();
                    })),
                )
        }));
    let updates = section(
        "Updates",
        Some("Mnema checks for updates at startup after onboarding."),
        Vec::new(),
        false,
        vec![row("Status")
            .desc("Check for a new build and install it from here.")
            .full()
            .control(panel)
            .build(p)
            .into_any_element()],
        p,
    );

    let notices = s.data.notices.clone().unwrap_or_default();
    let plain = notices
        .str_at(&["plainText"])
        .unwrap_or_default()
        .to_string();
    let entries = notices
        .at(&["entries"])
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut kinds: Vec<String> = Vec::new();
    for e in &entries {
        let k = e.str_at(&["kind"]).unwrap_or_default().to_string();
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    let groups = kinds.iter().map(|k| {
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .text_size(text::SM)
                    .font_weight(theme::W_SEMIBOLD)
                    .text_color(p.text_subtle)
                    .child(k.clone()),
            )
            .children(
                entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.str_at(&["kind"]) == Some(k.as_str()))
                    .map(|(i, e)| {
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .gap(space::S2)
                                    .text_size(text::BASE)
                                    .child(div().text_color(p.text_strong).child(
                                        e.str_at(&["displayName"]).unwrap_or_default().to_string(),
                                    ))
                                    .child(div().text_color(p.text_subtle).tracking(0.02).child(
                                        e.str_at(&["license"]).unwrap_or_default().to_string(),
                                    )),
                            )
                            .children(e.str_at(&["sourceUrl"]).map(|u| {
                                Link::new(
                                    nid("notice-source", i as u64),
                                    u.to_string(),
                                    u.to_string(),
                                )
                                .quiet()
                            }))
                    }),
            )
    });
    let acks = section(
        "Acknowledgements",
        Some("Mnema's on-device models and fonts come from these open projects. Each is credited under its license."),
        Vec::new(),
        false,
        vec![row("Third-party notices")
            .desc("The full attribution list for the bundled open-source components.")
            .full()
            .control(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::S4)
                    .w_full()
                    .child(div().child(copy_button(
                        "notices-copy",
                        "Copy notices",
                        "Copy the full third-party notices to the clipboard",
                        plain,
                    )))
                    .when(entries.is_empty(), |d| {
                        d.child(hint(
                            if s.data.notices.is_none() {
                                "Loading notices\u{2026}"
                            } else {
                                "No third-party notices to show."
                            },
                            p,
                        ))
                    })
                    .children(groups),
            )
            .build(p)
            .into_any_element()],
        p,
    );
    vec![
        about.into_any_element(),
        updates.into_any_element(),
        acks.into_any_element(),
    ]
}
