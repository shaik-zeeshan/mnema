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

fn excluded_list(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let apps = s.d().privacy.excluded_apps;
    if apps.is_empty() {
        return empty(
            EmptyLayout::Compact,
            Tone::Neutral,
            icons::SHIELD,
            "No app exclusions",
            "Every app\u{2019}s visible content is recorded. Add the ones that should never be.",
            vec![super::app_picker::add_app(s, p, cx)],
            p,
        )
        .into_any_element();
    }
    let enabled = apps.iter().filter(|a| a.enabled).count();
    div()
        .flex()
        .flex_col()
        .w_full()
        .rounded(radius::MD)
        .border_1()
        .border_color(p.border)
        .bg(p.surface)
        .children(apps.into_iter().map(|app| {
            let (id, id2) = (app.id.clone(), app.id.clone());
            div()
                .flex()
                .items_center()
                .gap(space::S3)
                .px(space::S3)
                .py(px(8.))
                .border_b_1()
                .border_color(p.hairline)
                .when(!app.enabled, |d| d.opacity(theme::DISABLED_OPACITY))
                .child(super::app_picker::excl_app(
                    s,
                    &app.display_name,
                    &app.bundle_id,
                    p,
                ))
                // Name and bundle id: one line each, cut with an ellipsis
                // (`.excl-meta`); the tooltips have the rest.
                .child(crate::ui::with_tip(
                    div()
                        .id(SharedString::from(format!("excl-name-{}", app.id)))
                        .min_w_0()
                        .truncate()
                        .text_size(text::MD)
                        .font_weight(theme::W_MEDIUM)
                        .text_color(p.text_strong)
                        .child(app.display_name.clone()),
                    app.display_name.clone(),
                ))
                .child(crate::ui::with_tip(
                    div()
                        .id(SharedString::from(format!("excl-bundle-{}", app.id)))
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(FONT_MONO)
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .child(app.bundle_id.clone()),
                    app.bundle_id.clone(),
                ))
                // `Switch` takes no tip: a size-to-fit wrapper carries it (also while disabled).
                .child(crate::ui::with_tip(
                    div()
                        .id(SharedString::from(format!("excl-on-tip-{}", app.id)))
                        .flex()
                        .flex_none()
                        .child(
                            Switch::new(
                                SharedString::from(format!("excl-on-{}", app.id)),
                                format!("Exclude {}", app.display_name),
                                app.enabled,
                            )
                            .disabled(s.privacy_in_flight)
                            .on_toggle(on(
                                cx,
                                move |s, v: bool, cx| {
                                    let id = id.clone();
                                    s.privacy_command(
                                        move |d| rules::set_privacy_excluded_app_enabled(d, &id, v),
                                        cx,
                                    )
                                },
                            )),
                        ),
                    if app.enabled {
                        "Excluded · switch off to pause"
                    } else {
                        "Paused · switch on to exclude"
                    },
                ))
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
                .rounded_b(radius::MD - px(1.))
                .bg(p.surface_subtle)
                .child(super::app_picker::add_app(s, p, cx))
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
                                "Enable",
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
    if let Some(r) = gecko_row(s, &d, p, cx) {
        rows.push(r);
    }
    section("Privacy", None, Vec::new(), false, rows, p).into_any_element()
}

/// `Browser URL access (Firefox / Zen)`: shown while URLs are stored and one of
/// those browsers is installed; Accessibility granted, or a notice to grant it.
fn gecko_row(
    s: &Settings,
    d: &capture_types::RecordingSettings,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> Option<AnyElement> {
    let g = s.data.gecko.as_ref()?;
    let names: Vec<&str> = g["geckoBrowsers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["installed"].as_bool() == Some(true))
        .filter_map(|b| b["displayName"].as_str())
        .collect();
    if names.is_empty() || !d.metadata.enabled || d.metadata.browser_url_mode == BrowserUrlMode::Off
    {
        return None;
    }
    let names = names.join(" / ");
    let body = if g["trusted"].as_bool() == Some(true) {
        line(
            Tone::Ok,
            icons::CHECK,
            format!("{names} \u{b7} Accessibility granted"),
            p,
        )
        .into_any_element()
    } else {
        let requesting = s.ui.busy.contains(GECKO_REQUEST);
        let rechecking = s.ui.busy.contains(GECKO_RECHECK);
        crate::ui::notice::notice(
            Tone::Warn,
            icons::LOCK,
            format!("{names} \u{b7} Accessibility not granted"),
            Some(format!("Allow Mnema under Privacy & Security \u{2192} Accessibility to capture {names}\u{2019}s page address.").into()),
            vec![
                Button::new("gecko-grant", if requesting { "Requesting\u{2026}" } else { "Grant access" })
                    .sm()
                    .busy(requesting)
                    .disabled(requesting)
                    .on_click(click(cx, |s, cx| s.gecko_command(GECKO_REQUEST, cx)))
                    .into_any_element(),
                Button::new("gecko-open", "Open System Settings")
                    .sm()
                    .icon(icons::EXTERNAL)
                    .icon_after()
                    .on_click(click(cx, |s, cx| s.gecko_command(GECKO_OPEN, cx)))
                    .into_any_element(),
                Button::new("gecko-recheck", if rechecking { "Checking\u{2026}" } else { "Recheck" })
                    .ghost()
                    .sm()
                    .busy(rechecking)
                    .disabled(rechecking)
                    .on_click(click(cx, |s, cx| s.gecko_recheck(cx)))
                    .into_any_element(),
            ],
            p,
        )
        .into_any_element()
    };
    Some(
        row("Browser URL access (Firefox / Zen)")
            .desc("Firefox and Zen expose no scriptable URL, so reading their page address needs the macOS Accessibility permission.")
            .full()
            .control(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::S2)
                    .w_full()
                    .child(body)
                    .children(s.ui.gecko_error.clone().map(|e| {
                        line(Tone::Warn, icons::INFO, format!("Browser URL access request failed: {e}"), p)
                    })),
            )
            .build(p)
            .into_any_element(),
    )
}

const GECKO_REQUEST: &str = "Accessibility request";
const GECKO_OPEN: &str = "Accessibility settings";
const GECKO_RECHECK: &str = "Accessibility recheck";

impl Settings {
    /// `request_browser_url_accessibility` / `open_browser_url_accessibility_settings`:
    /// a failure says so under the notice; the re-read picks up a grant.
    fn gecko_command(&mut self, what: &'static str, cx: &mut Context<Self>) {
        self.ui.gecko_error = None;
        self.run_command_with(
            what,
            cx,
            move |core| {
                use app_core::native_capture::permissions as perms;
                if what == GECKO_REQUEST {
                    perms::request_browser_url_accessibility();
                    Ok(None)
                } else {
                    perms::open_browser_url_accessibility_settings(core).map(|_| None)
                }
            },
            |s, outcome| {
                if let Err(e) = outcome {
                    s.ui.gecko_error = Some(e.clone());
                }
                true
            },
        );
    }

    /// Recheck: re-read the Accessibility grant (and the other permissions).
    fn gecko_recheck(&mut self, cx: &mut Context<Self>) {
        if !self.ui.busy.insert(GECKO_RECHECK) {
            return;
        }
        self.ui.gecko_error = None;
        let backend = self.backend.clone();
        let task = cx.background_spawn(async move { backend.recheck_access() });
        cx.spawn(async move |this, cx| {
            let fresh = task.await;
            let _ = this.update(cx, |s, cx| {
                s.ui.busy.remove(GECKO_RECHECK);
                if let Some(d) = fresh {
                    s.data.gecko = d.gecko;
                    s.data.permissions = d.permissions;
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
