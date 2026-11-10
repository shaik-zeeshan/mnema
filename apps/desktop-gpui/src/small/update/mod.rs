//! The update window (`routes/update/+page.svelte`, 480×520 / min 420×240):
//! three bands, identity · notes · controls, and only the middle one flexes,
//! so the buttons never move as the state advances. Status is app-core's
//! `AppUpdateStatus`, streamed by `CoreEvent::AppUpdateStatusChanged`.

pub mod model;

use std::sync::Arc;

use capture_types::LicenseStatus;
use gpui_kit::base::{TextView, TextViewStyle};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, img, px, relative, AnyElement, App, AppContext, Context, Global, Image, ImageFormat,
    Render, Window,
};
use serde_json::Value;

use super::parts::{b, frame, h2, inset_panel, note, para, spec_row};
use crate::settings::model::{format_bytes, Json};
use crate::theme::{self, space, text, Palette, FONT_MONO};
use crate::ui::window_strip::WindowStrip;
use crate::ui::{icon, kicker, progress, Button, Tone};
use model::{Key, LineIcon, LineTone};

pub const SIZE: (f32, f32) = (480., 520.);
pub const MIN: (f32, f32) = (420., 240.);

/// `lib/licensing.ts` `RENEWAL_CHECKOUT_URL` (default).
const RENEWAL_CHECKOUT_URL: &str = "https://sandbox-api.polar.sh/v1/checkout-links/polar_cl_Tihx0GjXAe53ljFFHWh4wxUwb3u0KSVLBvJ5b00hFyi/redirect";

/// The real bundle icon (`static/app-icon.png`, a copy of the 128@2x icon).
pub fn app_icon() -> Arc<Image> {
    static BYTES: &[u8] = include_bytes!("../../../../desktop/static/app-icon.png");
    Arc::new(Image::from_bytes(ImageFormat::Png, BYTES.to_vec()))
}

/// The newest status app-core published, so a window opened later starts there.
#[derive(Default)]
struct LastStatus(Option<Value>);

impl Global for LastStatus {}

/// The same status, typed (the tray's update row reads it).
struct LastTyped(app_core::updates::AppUpdateStatus);

impl Global for LastTyped {}

pub fn last_status(cx: &App) -> Option<&app_core::updates::AppUpdateStatus> {
    cx.try_global::<LastTyped>().map(|s| &s.0)
}

pub struct UpdateWindow {
    pub status: Option<Value>,
    pub license: Option<LicenseStatus>,
    pub action_error: Option<String>,
    pub acting: bool,
    pub notes_open: bool,
    icon: Arc<Image>,
}

impl UpdateWindow {
    pub fn new(status: Option<Value>, license: Option<LicenseStatus>) -> Self {
        Self {
            status,
            license,
            action_error: None,
            acting: false,
            notes_open: false,
            icon: app_icon(),
        }
    }

    pub fn set_status(&mut self, status: Value, cx: &mut Context<Self>) {
        // An install in flight ends in a relaunch; anything else re-arms the buttons.
        let state = status.str_at(&["state"]);
        if !matches!(
            state,
            Some("downloading" | "installing" | "restartRequired")
        ) {
            self.acting = false;
        }
        self.status = Some(status);
        cx.notify();
    }

    fn primary(&mut self, cx: &mut Context<Self>) {
        self.action_error = None;
        match model::key(self.status.as_ref()) {
            Key::Expired => cx.open_url(RENEWAL_CHECKOUT_URL),
            Key::Ready => {
                let Some(core) = crate::chat::core(cx) else {
                    return;
                };
                self.acting = true;
                if let Err(e) = app_core::updates::restart_after_app_update(&core) {
                    self.action_error = Some(e.message);
                    self.acting = false;
                }
            }
            // The failure shows here, as the window's own Failed state.
            _ => {
                self.acting = true;
                install(false, cx);
            }
        }
        cx.notify();
    }

    fn identity(&self, k: Key, cx: &App, p: &Palette) -> AnyElement {
        let s = self.status.as_ref();
        let version = s.and_then(|s| s.str_at(&["update", "version"]));
        let current = s.and_then(|s| s.str_at(&["app", "version"]));
        let date = s
            .and_then(|s| s.str_at(&["update", "date"]))
            .map(model::release_date);
        let archive = s
            .and_then(|s| s.num_at(&["update", "sizeBytes"]))
            .filter(|n| *n > 0.)
            .map(format_bytes);
        let mut k_line = model::kicker(k).to_string();
        if let Some(c) = model::channel_line(s) {
            k_line = format!("{k_line} \u{2002}\u{b7}\u{2002} {c}");
        }
        let faint = |t: &'static str| div().text_color(p.text_faint).child(t);
        let ver = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(7.))
            .font_family(FONT_MONO)
            .text_size(text::BASE)
            .line_height(relative(1.))
            .text_color(p.text_subtle);
        let ver = match (version, current) {
            (Some(v), Some(c)) if k != Key::Expired => ver
                .child(c.to_string())
                .child(faint("\u{2192}"))
                .child(
                    div()
                        .font_weight(theme::W_MEDIUM)
                        .text_color(p.text_strong)
                        .child(v.to_string()),
                )
                .when_some(date, |d, date| d.child(faint("\u{b7}")).child(date))
                .when_some(archive, |d, a| d.child(faint("\u{b7}")).child(a)),
            (_, Some(c)) => {
                let tail = match k {
                    Key::Expired => self
                        .license_terms(cx)
                        .map(|(_, end)| format!(" \u{b7} updates covered through {end}")),
                    Key::UpToDate => {
                        model::checked_clock(last_checked(s), crate::ui::clock::now(cx))
                            .map(|t| format!(" \u{b7} checked {t}"))
                    }
                    Key::CheckFailed => {
                        model::checked_clock(last_checked(s), crate::ui::clock::now(cx))
                            .map(|t| format!(" \u{b7} last checked {t}"))
                    }
                    _ => None,
                };
                // `.up-ver__words`: sans words around the mono version (`<b>`).
                ver.font_family(theme::FONT_UI)
                    .line_height(relative(1.3))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_baseline()
                            .gap(px(4.))
                            .child("You have")
                            .child(
                                div()
                                    .font_family(FONT_MONO)
                                    .font_weight(theme::W_MEDIUM)
                                    .text_color(p.text_strong)
                                    .child(c.to_string()),
                            )
                            .children(tail.map(|t| t.trim_start().to_string())),
                    )
            }
            _ => ver,
        };
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .px(space::S5)
            .pt(space::S5)
            .pb(space::S4)
            .child(img(self.icon.clone()).flex_none().size(px(44.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .min_w_0()
                    .child(kicker(&k_line, p).line_height(relative(1.)))
                    .child(h2(model::heading(s), p))
                    .child(ver),
            )
            .into_any_element()
    }

    /// `(licensee, window end)` for the out-of-window state.
    fn license_terms(&self, cx: &App) -> Option<(String, String)> {
        let LicenseStatus::Licensed {
            update_through_ms,
            email,
            name,
            ..
        } = self.license.as_ref()?
        else {
            return None;
        };
        let who = if name.trim().is_empty() {
            email.clone()
        } else {
            name.trim().to_string()
        };
        let now = crate::ui::clock::now(cx);
        let day = crate::settings::model::short_day(*update_through_ms, now);
        let year =
            time::OffsetDateTime::from_unix_timestamp_nanos(*update_through_ms as i128 * 1_000_000)
                .map(|t| t.year())
                .unwrap_or(now.year());
        Some((who, format!("{day}, {year}")))
    }

    fn band(&self, k: Key, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let notes = self
            .status
            .as_ref()
            .and_then(|s| s.str_at(&["update", "notes"]))
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string);
        let show_notes = notes.is_some()
            && !matches!(
                k,
                Key::Checking | Key::UpToDate | Key::CheckFailed | Key::Incompatible
            )
            && (k != Key::Expired || self.notes_open);
        let mut band = div()
            .flex()
            .flex_col()
            .gap(space::S3)
            .min_h_0()
            .max_h_full();
        if k == Key::Expired {
            let current = self
                .status
                .as_ref()
                .and_then(|s| s.str_at(&["app", "version"]))
                .unwrap_or("This version")
                .to_string();
            band = band.child(note(
                Tone::Ok,
                "icons/shield-check.svg",
                para(
                    &[
                        ("Nothing stops working. ", b(p)),
                        (&format!("Mnema {current} is yours to keep \u{2014} capture, search, Ask AI and every recording stay as they are. Renewing only brings newer builds."), None),
                    ],
                    text::BASE,
                    p.text_muted,
                ),
                p,
            ));
            if let Some((who, end)) = self.license_terms(cx) {
                let rows = [
                    ("Licensed to", who),
                    ("Update window", format!("ended {end}")),
                    ("Renewal", "another year of updates, same key".into()),
                ];
                band =
                    band.child(inset_panel(p).px(space::S4).py(px(2.)).children(
                        rows.into_iter().enumerate().map(|(i, (l, v))| {
                            spec_row(i == 0, l, div().child(v), None, px(110.), p)
                        }),
                    ));
            }
            if let (Some(_), Some(v)) = (
                &notes,
                self.status
                    .as_ref()
                    .and_then(|s| s.str_at(&["update", "version"])),
            ) {
                let label = format!("What\u{2019}s in {v}");
                band = band.child(
                    div().flex().child(
                        Button::new("up-seenotes", label)
                            .ghost()
                            .sm()
                            .pressed(self.notes_open)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.notes_open = !this.notes_open;
                                cx.notify();
                            })),
                    ),
                );
            }
        } else if let Some(m) = model::message(k) {
            band = band.child(
                div()
                    .text_size(text::PROSE)
                    .line_height(relative(1.6))
                    .text_color(p.text_muted)
                    .child(m),
            );
        }
        if let (true, Some(n)) = (show_notes, notes) {
            band = band.child(
                inset_panel(p)
                    .id("up-notes")
                    .aria_label("Release notes")
                    .flex_shrink_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(space::S4)
                    .pt(space::S3)
                    .pb(space::S4)
                    .text_size(text::BASE)
                    .line_height(relative(1.5))
                    .text_color(p.text_muted)
                    .child(match model::note_blocks(&n) {
                        Some(blocks) => notes_view(blocks, cx, p).into_any_element(),
                        None => TextView::markdown("up-notes-md", n)
                            .style(notes_style(p, theme::mode(cx) == theme::Mode::Dark))
                            .into_any_element(),
                    }),
            );
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .px(space::S5)
            .child(band)
            .into_any_element()
    }

    fn controls(&self, k: Key, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let s = self.status.as_ref();
        let meter = |label: &'static str, meta: String, value: Option<f32>| {
            div()
                .flex()
                .flex_col()
                .child(progress::progress_meta(label, meta, p))
                .child(progress::progress(value.map(|v| v / 100.), p))
        };
        let bar = match k {
            Key::Downloading => {
                let pct = model::progress_percent(s);
                let mut meta = model::progress_text(s);
                if let Some(v) = pct {
                    meta = format!("{meta}\u{a0}\u{b7} {}%", v.round());
                }
                Some(meter("Downloading", meta, pct))
            }
            Key::Installing => Some(meter("Installing", "keep Mnema open".into(), None)),
            _ => None,
        };
        let line_el = |tone: LineTone, glyph: LineIcon, text_: String| {
            let tone = match tone {
                LineTone::Plain => Tone::Neutral,
                LineTone::Ok => Tone::Ok,
                LineTone::Warn => Tone::Warn,
                LineTone::Danger => Tone::Danger,
            };
            let color = if tone == Tone::Neutral {
                p.text_subtle
            } else {
                tone.colors(p).0
            };
            let lead: Option<AnyElement> = match glyph {
                LineIcon::None => None,
                LineIcon::Spin => Some(progress::spinner(theme::size::SPIN_SM).into_any_element()),
                LineIcon::Power => Some(icon("icons/power.svg", px(14.)).into_any_element()),
                LineIcon::Check => Some(icon("icons/check.svg", px(14.)).into_any_element()),
                LineIcon::CloudOff => Some(icon("icons/cloud-off.svg", px(14.)).into_any_element()),
                LineIcon::Alert => {
                    Some(icon("icons/triangle-alert.svg", px(14.)).into_any_element())
                }
            };
            div()
                .flex()
                .items_start()
                .gap(px(6.))
                .min_h(px(18.))
                .text_size(text::BASE)
                .line_height(relative(1.45))
                .text_color(color)
                .children(lead.map(|l| div().flex_none().mt(px(2.)).child(l)))
                .child(div().flex_1().min_w_0().child(text_))
        };
        let lines = model::lines(s)
            .into_iter()
            .map(|l| line_el(l.tone, l.icon, l.text).into_any_element());
        let err = self
            .action_error
            .clone()
            .map(|e| line_el(LineTone::Danger, LineIcon::Alert, e));
        let secondary = match k {
            Key::Downloading => "Continue in background",
            Key::Expired => "Not now",
            _ if model::message(k).is_some() => "Close",
            _ => "Later",
        };
        let busy = match k {
            Key::Installing => Some("Installing\u{2026}"),
            Key::Downloading => Some("Downloading\u{2026}"),
            _ if self.acting => Some("Downloading\u{2026}"),
            _ => None,
        };
        let primary = match k {
            Key::Ready if self.acting => Button::new("up-primary", "Restarting\u{2026}")
                .spinner(true)
                .busy(true),
            Key::Ready => Button::new("up-primary", "Restart now")
                .disabled(!model::can_restart(s, self.acting)),
            Key::Expired => Button::new("up-primary", "Renew for a year \u{b7} $29")
                .icon("icons/external-link.svg")
                .icon_after(),
            _ => match busy {
                Some(l) => Button::new("up-primary", l)
                    .spinner(true)
                    .busy(true)
                    .disabled(true),
                None => Button::new(
                    "up-primary",
                    if matches!(k, Key::DownloadFailed | Key::InstallFailed) {
                        "Try again"
                    } else {
                        "Install and restart"
                    },
                )
                .disabled(!model::can_install(s, self.acting)),
            },
        }
        .primary()
        .on_click(cx.listener(|this, _, _, cx| this.primary(cx)));
        let actions = div()
            .flex()
            .items_center()
            .gap(space::S2)
            .when(k == Key::Expired, |d| {
                d.child(
                    Button::new("up-refresh", "Refresh license")
                        .ghost()
                        .sm()
                        .icon("icons/rotate-cw.svg")
                        // `refresh_license_now`: the new status arrives as LicenseStatus.
                        .on_click(|_, _, cx| {
                            if let Some(core) = crate::chat::core(cx) {
                                let task_core = core.clone();
                                core.spawn(async move {
                                    if let Err(e) =
                                        app_core::licensing::refresh_license_now(task_core).await
                                    {
                                        log::warn!("update: license refresh failed: {e}");
                                    }
                                });
                            }
                        }),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("up-secondary", secondary)
                    .ghost()
                    .disabled(k == Key::Installing)
                    .on_click(|_, window, _| window.remove_window()),
            )
            .child(primary);
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(space::S3)
            .mt(space::S4)
            .px(space::S5)
            .pt(space::S4)
            .pb(space::S5)
            .border_t_1()
            .border_color(p.hairline)
            .children(bar)
            .children(lines)
            .children(err)
            .child(actions)
            .into_any_element()
    }
}

/// `.up-notes`: muted body, strong 13 px headings, links in body ink.
/// `.up-notes` for GitHub's notes: a 13 px heading, 12/1.5 bullets 5 px apart
/// (faint markers), and the closing meta line at 11 px subtle with its link.
fn notes_view(
    blocks: Vec<model::NoteBlock>,
    cx: &mut Context<UpdateWindow>,
    p: &Palette,
) -> gpui_kit::Div {
    use model::NoteBlock;
    let mut first = true;
    div()
        .flex()
        .flex_col()
        .children(blocks.into_iter().map(|b| {
            let top = !std::mem::take(&mut first);
            match b {
                NoteBlock::Heading(h) => div()
                    .when(top, |d| d.mt(px(4.)))
                    .mb(px(6.))
                    .font_weight(theme::W_SEMIBOLD)
                    .text_size(text::MD)
                    .line_height(relative(1.3))
                    .text_color(p.text_strong)
                    .child(h)
                    .into_any_element(),
                NoteBlock::List(items) => {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .mb(space::S2)
                        .children(items.into_iter().map(|t| {
                            div()
                                .flex()
                                .text_size(text::BASE)
                                .line_height(relative(1.5))
                                .text_color(p.text_muted)
                                .child(
                                    // `li::marker`: a faint round bullet.
                                    div()
                                        .w(px(18.))
                                        .flex_none()
                                        .pl(px(6.))
                                        .pt(px(7.))
                                        .child(div().size(px(4.)).rounded_full().bg(p.text_faint)),
                                )
                                .child(div().flex_1().min_w_0().child(t))
                        }))
                        .into_any_element()
                }
                NoteBlock::Para(words, link) => {
                    // One text run so the link wraps inline after the words, as in CSS.
                    let url = link.unwrap_or_default();
                    let open = url.clone();
                    let mut highlights = Vec::new();
                    if !url.is_empty() {
                        highlights.push((
                            words.len()..words.len() + url.len(),
                            gpui_kit::HighlightStyle {
                                color: Some(p.text),
                                underline: Some(gpui_kit::UnderlineStyle {
                                    thickness: px(1.),
                                    color: Some(p.border_hover),
                                    wavy: false,
                                }),
                                ..Default::default()
                            },
                        ));
                    }
                    div()
                        .id("up-notes-link")
                        .when(top, |d| d.mt(space::S2))
                        .text_size(text::SM)
                        .line_height(relative(1.4))
                        .text_color(p.text_subtle)
                        .child(
                            gpui_kit::StyledText::new(format!("{words}{url}"))
                                .with_highlights(highlights),
                        )
                        .when(!open.is_empty(), |d| {
                            d.role(gpui_kit::Role::Link)
                                .cursor_pointer()
                                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&open)))
                        })
                        .into_any_element()
                }
            }
        }))
}

fn notes_style(p: &Palette, dark: bool) -> TextViewStyle {
    let strong = p.text_strong;
    TextViewStyle::default()
        .with_dark(dark)
        .with_foreground(p.text_muted)
        .with_muted_foreground(p.text_subtle)
        .with_link(p.text)
        .with_border(p.border)
        .with_code_background(p.wash)
        .with_heading(move |_| {
            gpui_kit::StyleRefinement::default()
                .text_size(text::MD)
                .font_weight(theme::W_SEMIBOLD)
                .text_color(strong)
        })
}

fn last_checked(s: Option<&Value>) -> Option<i64> {
    s.and_then(|s| s.num_at(&["lastCheckedAtUnixMs"]))
        .map(|n| n as i64)
}

impl Render for UpdateWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::request_fade_frames(window, cx);
        let p = theme::tokens(cx);
        let k = model::key(self.status.as_ref());
        frame(&p)
            .id("update-window")
            .child(WindowStrip::new("Software Update"))
            .child(self.identity(k, cx, &p))
            .child(self.band(k, cx, &p))
            .child(self.controls(k, cx, &p))
    }
}

/// `CoreEvent::AppUpdateStatusChanged`: remember it, and refresh an open window.
pub fn status_changed(status: app_core::updates::AppUpdateStatus, cx: &mut App) {
    let Ok(v) = serde_json::to_value(&status) else {
        return;
    };
    cx.set_global(LastTyped(status));
    cx.set_global(LastStatus(Some(v.clone())));
    let open = cx
        .try_global::<super::Open>()
        .and_then(|o| o.0.get(&super::Kind::Update).cloned());
    if let Some((handle, weak)) = open {
        if let Some(view) = weak
            .upgrade()
            .and_then(|e| e.downcast::<UpdateWindow>().ok())
        {
            let _ = handle.update(cx, |_, _, cx| view.update(cx, |w, cx| w.set_status(v, cx)));
        }
    }
}

/// "Check for Updates" (Settings).
pub fn check(cx: &App) {
    if let Some(core) = crate::chat::core(cx) {
        let task_core = core.clone();
        core.spawn(async move {
            app_core::updates::check_for_app_update(&task_core).await;
        });
    }
}

/// Install, then relaunch (the window, Settings and the tray row). The tray has
/// no window to show a failure in, so it asks for an alert.
pub fn install(alert_on_failure: bool, cx: &App) {
    if let Some(core) = crate::chat::core(cx) {
        app_core::updates::install_and_restart(&core, alert_on_failure);
    }
}

pub fn open(cx: &mut App) {
    let status = cx.try_global::<LastStatus>().and_then(|s| s.0.clone());
    let core = crate::chat::core(cx);
    let license = core
        .as_ref()
        .and_then(|core| app_core::licensing::cached_status(core));
    let status = status.or_else(|| {
        core.as_ref()
            .and_then(|core| app_core::updates::current_status(core))
            .and_then(|s| serde_json::to_value(s).ok())
    });
    super::open_or_focus::<UpdateWindow>(
        super::Kind::Update,
        cx,
        |_, cx| cx.new(|_| UpdateWindow::new(status, license)),
        |_, _, _| {},
    );
}
