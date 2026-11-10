//! About › Updates (`About.svelte`): the identity hero + Copy details, the
//! update status panel + Check / Install, and the third-party notices.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, ClipboardItem, Context};
use serde_json::Value;

use crate::settings::model::{short_datetime, Json};
use crate::settings::row::{hint, row, section};
use crate::settings::{click, Settings};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::Button;

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
        Some("downloading") => "Downloading the update package.".into(),
        _ => String::new(),
    }
}

fn copy_button(
    id: &'static str,
    label: &'static str,
    text_: String,
    cx: &mut Context<Settings>,
) -> Button {
    Button::new(id, label)
        .sm()
        .on_click(click(cx, move |_, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(text_.clone()))
        }))
}

/// `set_app_update_channel`: preview builds ask first (they are less tested).
fn channel_button(u: Option<&serde_json::Value>, cx: &mut Context<Settings>) -> Button {
    let preview = u.and_then(|u| u.str_at(&["channel"])) == Some("preview");
    let label = if preview {
        "Use stable builds"
    } else {
        "Use preview builds"
    };
    Button::new("update-channel", label)
        .ghost()
        .sm()
        .on_click(click(cx, move |s, cx| {
            let set = move |s: &mut Settings, cx: &mut Context<Settings>| {
                let to = if preview { "stable" } else { "preview" };
                s.run_command("Update channel", cx, move |core| {
                    crate::settings::live::set_update_channel(core, to)
                })
            };
            if preview {
                set(s, cx);
            } else {
                crate::settings::confirm(
                    cx,
                    "Use preview builds?",
                    "Preview builds arrive earlier and are less tested. You can switch back to stable at any time.",
                    "Use preview",
                    set,
                );
            }
        }))
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
    let meta = |k: &'static str, v: String| {
        div()
            .flex()
            .gap(space::S3)
            .text_size(text::BASE)
            .child(
                div()
                    .w(px(84.))
                    .font_family(FONT_MONO)
                    .text_size(text::XS)
                    .text_color(p.text_subtle)
                    .child(k.to_uppercase()),
            )
            .child(div().text_color(p.text).child(v))
    };
    let hero = div()
        .flex()
        .flex_col()
        .gap(space::S3)
        .py(space::S4)
        .border_b_1()
        .border_color(p.hairline)
        .child(
            div()
                .flex()
                .items_center()
                .gap(space::S3)
                .child(div().text_size(text::XL).font_weight(theme::W_SEMIBOLD).text_color(p.text_strong).child("Mnema"))
                .child(match version.as_ref() {
                    Some(v) => chip(format!("v{v}"), ChipKind::Accent, p),
                    None => chip("checking\u{2026}", ChipKind::Neutral, p),
                })
                .child(chip(format!("{channel} channel").to_uppercase(), ChipKind::Neutral, p)),
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
                .no_divider()
                .control(copy_button("about-copy", "Copy details", details, cx))
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
                .child(chip(state_label(u), ChipKind::Neutral, p))
                .child(
                    Button::new("update-check", "Check for Updates")
                        .sm()
                        .on_click(click(cx, |_, cx| crate::small::update::check(cx))),
                )
                .child(channel_button(u, cx)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .p(space::S3)
                .rounded(radius::MD)
                .border_1()
                .border_color(p.border)
                .bg(p.surface_subtle)
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
        .child(
            div().child(
                Button::new("update-install", "Install Update")
                    .disabled(u.and_then(|u| u.str_at(&["state"])) != Some("available"))
                    .on_click(click(cx, |_, cx| crate::small::update::install(false, cx))),
            ),
        );
    let updates = section(
        "Updates",
        Some("Mnema checks for updates at startup after onboarding."),
        Vec::new(),
        false,
        vec![row("Status")
            .desc("Check for a new build and install it from here.")
            .full()
            .no_divider()
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
                    .filter(|e| e.str_at(&["kind"]) == Some(k.as_str()))
                    .map(|e| {
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
                                    .child(div().text_color(p.text_subtle).child(
                                        e.str_at(&["license"]).unwrap_or_default().to_string(),
                                    )),
                            )
                            .children(e.str_at(&["sourceUrl"]).map(|u| {
                                div()
                                    .font_family(FONT_MONO)
                                    .text_size(text::SM)
                                    .text_color(p.text_muted)
                                    .child(u.to_string())
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
            .no_divider()
            .control(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::S4)
                    .w_full()
                    .child(div().child(copy_button("notices-copy", "Copy notices", plain, cx)))
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
