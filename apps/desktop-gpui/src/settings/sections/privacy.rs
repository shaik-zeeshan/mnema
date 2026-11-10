//! Capture › Privacy (`Privacy.svelte`, `AppPrivacyExclusion.svelte`): the
//! excluded apps (privacy commands, never autosave), frame context and the
//! browser URL mode (Full asks first).

use app_core::privacy_redaction_sources as rules;
use capture_types::BrowserUrlMode;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, SharedString};

use super::ix;
use crate::settings::row::{line, row, section};
use crate::settings::{click, confirm, icons, on, Settings};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::choice::{Choice, ChoiceOption};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::switch::Switch;
use crate::ui::{Button, Tone};

const URL_MODES: [BrowserUrlMode; 3] = [
    BrowserUrlMode::Off,
    BrowserUrlMode::Sanitized,
    BrowserUrlMode::Full,
];

fn add_app_button(s: &Settings, cx: &mut Context<Settings>) -> Button {
    Button::new("excl-add", "Add app\u{2026}")
        .sm()
        .icon(icons::PLUS)
        .disabled(s.privacy_in_flight)
        .on_click(click(cx, |s, cx| s.open_app_picker(cx)))
}

fn excluded_list(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let apps = s.d().privacy.excluded_apps;
    if apps.is_empty() {
        return empty(
            EmptyLayout::Compact,
            Tone::Neutral,
            icons::SHIELD,
            "No app exclusions",
            "Every app\u{2019}s visible content is recorded. Add the ones that should never be.",
            vec![add_app_button(s, cx).into_any_element()],
            p,
        )
        .into_any_element();
    }
    let enabled = apps.iter().filter(|a| a.enabled).count();
    div()
        .flex()
        .flex_col()
        .w_full()
        .rounded(radius::LG)
        .border_1()
        .border_color(p.border)
        .children(apps.into_iter().map(|app| {
            let (id, id2) = (app.id.clone(), app.id.clone());
            let letter: String = app
                .display_name
                .chars()
                .next()
                .map(|c| c.to_uppercase().collect())
                .unwrap_or_default();
            div()
                .flex()
                .items_center()
                .gap(space::S3)
                .px(space::S3)
                .py(px(8.))
                .border_b_1()
                .border_color(p.hairline)
                .when(!app.enabled, |d| d.opacity(theme::DISABLED_OPACITY))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(22.))
                        .rounded(radius::SM)
                        .bg(p.surface_subtle)
                        .border_1()
                        .border_color(p.hairline)
                        .text_size(text::SM)
                        .font_weight(theme::W_SEMIBOLD)
                        .text_color(p.text_muted)
                        .child(letter),
                )
                .child(
                    div()
                        .text_size(text::MD)
                        .font_weight(theme::W_MEDIUM)
                        .text_color(p.text_strong)
                        .child(app.display_name.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(FONT_MONO)
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .child(app.bundle_id.clone()),
                )
                .child(
                    Switch::new(
                        SharedString::from(format!("excl-on-{}", app.id)),
                        format!("Exclude {}", app.display_name),
                        app.enabled,
                    )
                    .disabled(s.privacy_in_flight)
                    .on_toggle(on(cx, move |s, v: bool, cx| {
                        let id = id.clone();
                        s.privacy_command(
                            move |d| rules::set_privacy_excluded_app_enabled(d, &id, v),
                            cx,
                        )
                    })),
                )
                .child(
                    Button::icon_only(
                        SharedString::from(format!("excl-rm-{}", app.id)),
                        icons::X,
                        format!("Remove {}", app.display_name),
                    )
                    .ghost()
                    .sm()
                    .tip("Remove exclusion")
                    .disabled(s.privacy_in_flight)
                    .on_click(click(cx, move |s, cx| {
                        let id = id2.clone();
                        s.privacy_command(move |d| rules::remove_privacy_excluded_app(d, &id), cx)
                    })),
                )
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap(space::S3)
                .px(space::S3)
                .py(px(8.))
                .child(add_app_button(s, cx))
                .child(
                    div()
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .ml_auto()
                        .child(format!("{enabled} excluded")),
                ),
        )
        .into_any_element()
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let d = s.d();
    let mut rows: Vec<AnyElement> = s
        .privacy_prompt()
        .map(|v| crate::settings::privacy_prompt::section_notice(s, &v, p, cx))
        .into_iter()
        .collect();
    rows.push(row("Excluded Apps")
        .desc("Apps whose visible content is never recorded. Browsers, including private windows, are recorded unless the browser itself is excluded. Mnema does not detect password pages or password fields.")
        .full()
        .control(excluded_list(s, p, cx))
        .when(s.privacy_error.is_some(), |r| {
            r.below(line(Tone::Danger, icons::ALERT, s.privacy_error.clone().unwrap_or_default(), p))
        })
        .build(p)
        .into_any_element());
    rows.push(
        row("Filter system audio")
            .desc("Also keep audio from excluded apps out of recordings.")
            .control(
                Switch::new(
                    "filter-sys",
                    "Filter system audio",
                    d.privacy.filter_system_audio,
                )
                .disabled(s.privacy_in_flight)
                .on_toggle(on(cx, |s, v: bool, cx| {
                    s.privacy_command(
                        move |d| {
                            d.privacy.filter_system_audio = v;
                            Ok(())
                        },
                        cx,
                    )
                })),
            )
            .build(p)
            .into_any_element(),
    );
    rows.push(
        row("Capture frame context")
            .desc("Store app, window, and supported browser context with frames.")
            .control(
                Switch::new("frame-context", "Capture frame context", d.metadata.enabled)
                    .on_toggle(on(cx, |s, v: bool, cx| {
                        s.edit(|d| d.metadata.enabled = v, cx)
                    })),
            )
            .build(p)
            .into_any_element(),
    );
    let off = !d.metadata.enabled;
    let opts = vec![
        ChoiceOption::new("Off")
            .icon(icons::OFF)
            .desc("Don't store browser URLs with captured frames.")
            .disabled(off),
        ChoiceOption::new("Sanitized")
            .tag("default")
            .icon(icons::SHIELD)
            .desc("Keep scheme, host, port and path; drop query strings and fragments.")
            .disabled(off),
        ChoiceOption::new("Full")
            .icon(icons::LINK)
            .desc("The complete URL, including query strings and fragments.")
            .disabled(off),
    ];
    rows.push(
        row("Browser URL mode")
            .desc("How much of a captured browser URL is stored with the frame. Full asks first.")
            .disabled(off)
            .full()
            .control(
                div().w_full().child(
                    Choice::new(
                        "url-mode",
                        "Browser URL mode",
                        opts,
                        ix(&URL_MODES, &d.metadata.browser_url_mode),
                    )
                    .on_change(on(cx, |s, i: usize, cx| {
                        let mode = URL_MODES[i].clone();
                        if mode == BrowserUrlMode::Full {
                            confirm(
                                cx,
                                "Enable full URL metadata",
                                "Full URL metadata stores query strings and fragments. Continue?",
                                "Continue",
                                |s, cx| {
                                    s.edit(
                                        |d| d.metadata.browser_url_mode = BrowserUrlMode::Full,
                                        cx,
                                    )
                                },
                            );
                        } else {
                            s.edit(|d| d.metadata.browser_url_mode = mode, cx);
                        }
                    })),
                ),
            )
            .build(p)
            .into_any_element(),
    );
    // Firefox / Zen: their URLs are read through Accessibility, which needs its own grant.
    if let Some(g) = s.data.gecko.as_ref() {
        let names: Vec<&str> = g["geckoBrowsers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["installed"].as_bool() == Some(true))
            .filter_map(|b| b["displayName"].as_str())
            .collect();
        if !names.is_empty() && d.metadata.enabled {
            let trusted = g["trusted"].as_bool() == Some(true);
            let r = row(format!("{} URLs", names.join(" / ")))
                .desc("These browsers share their address only through macOS Accessibility.");
            let r = if trusted {
                r.control(crate::settings::row::state_word(
                    "allowed",
                    Some(Tone::Ok),
                    p,
                ))
            } else {
                r.control(
                    Button::new("gecko-allow", "Allow Accessibility")
                        .icon(icons::EXTERNAL)
                        .icon_after()
                        .on_click(click(cx, |s, cx| {
                            s.run_command(
                                "Accessibility access",
                                cx,
                                crate::settings::live::request_accessibility,
                            )
                        })),
                )
            };
            rows.push(r.build(p).into_any_element());
        }
    }
    section("Privacy", None, Vec::new(), false, rows, p).into_any_element()
}
