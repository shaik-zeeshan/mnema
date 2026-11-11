//! Data › Storage (`Storage.svelte`, `StorageMeter.svelte`): the usage meter,
//! the save directory and retention.

use capture_types::RetentionPolicy;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Hsla};

use super::ix;
use crate::settings::model::{format_bytes, format_measured, Json};
use crate::settings::row::{line, row, section};
use crate::settings::{click, icons, on, Settings};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::segmented::Segmented;
use crate::ui::{notice, skeleton, Button, Tone};

const RETENTION: [RetentionPolicy; 4] = [
    RetentionPolicy::Days7,
    RetentionPolicy::Days14,
    RetentionPolicy::Days30,
    RetentionPolicy::Never,
];

/// Browse… (`browseSaveDirectory`): the native folder picker; a pick becomes the
/// draft save directory (it takes effect after a restart).
fn browse(
    cx: &Context<Settings>,
) -> impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static {
    let this = cx.entity().downgrade();
    move |_, _, cx| {
        let this = this.clone();
        crate::native::dialogs::pick_folder("Choose", cx, move |path, cx| {
            let path = path.to_string_lossy().into_owned();
            if path.trim().is_empty() {
                return;
            }
            let _ = this.update(cx, |s, cx| {
                s.ui.picked_location = Some(path.clone());
                s.data.storage_location_error = None;
                s.edit(|d| d.save_directory = path, cx);
            });
        });
    }
}

/// The meter's five parts (`meterParts`), in mockup order.
fn parts(u: &serde_json::Value, p: &Palette) -> Vec<(&'static str, f64, Hsla)> {
    let n = |k: &str| u.num_at(&[k]).unwrap_or(0.);
    vec![
        ("screen", n("screenBytes"), p.source_screen),
        ("mic", n("microphoneBytes"), p.source_mic),
        ("system", n("systemAudioBytes"), p.source_sysaudio),
        ("index", n("indexBytes"), p.chart_grey_4),
        ("models", n("modelsBytes"), p.chart_grey_2),
    ]
}

fn meter(s: &Settings, p: &Palette) -> AnyElement {
    let Some(u) = s.data.storage_usage.as_ref() else {
        // Measuring: the shape of the meter, not a spinner.
        return div()
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
                    .child(skeleton::skel(p).w(px(96.)).h(px(28.)))
                    .child(skeleton::skel(p).w(px(220.)).h(px(10.))),
            )
            .child(div().h(px(8.)).w_full().rounded(radius::PILL).bg(p.wash))
            .into_any_element();
    };
    let parts = parts(u, p);
    let total: f64 = parts.iter().map(|(_, b, _)| b).sum();
    let formatted = format_measured(total);
    let (value, unit) = formatted.split_once(' ').unwrap_or((&formatted, ""));
    let days = u.num_at(&["captureDays"]).unwrap_or(0.);
    let per_day = (days > 0.).then(|| (parts[0].1 + parts[1].1 + parts[2].1) / days);
    let free = u.num_at(&["freeBytes"]);
    let mut meta = vec!["used".to_string()];
    if let Some(f) = free {
        meta.push(format!("{} free", format_measured(f)));
    }
    if let Some(d) = per_day {
        meta.push(format!("~{} a day", format_bytes(d)));
    }
    div()
        .flex()
        .flex_col()
        .gap(space::S3)
        .py(space::S4)
        .border_b_1()
        .border_color(p.hairline)
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(space::S3)
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(3.))
                        .font_family(FONT_MONO)
                        .text_color(p.text_strong)
                        .child(
                            div()
                                .text_size(text::XL)
                                .font_weight(theme::W_SEMIBOLD)
                                .child(value.to_string()),
                        )
                        .child(
                            div()
                                .text_size(text::BASE)
                                .text_color(p.text_muted)
                                .child(unit.to_string()),
                        ),
                )
                .child(
                    div()
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .child(meta.join(" \u{b7} ")),
                ),
        )
        .child(
            div()
                .flex()
                .h(px(8.))
                .w_full()
                .rounded(radius::PILL)
                .overflow_hidden()
                .bg(p.wash)
                .children(parts.iter().filter(|(_, b, _)| *b > 0.).map(|(_, b, c)| {
                    div()
                        .h_full()
                        .w(gpui_kit::relative((*b / total.max(1.)) as f32))
                        .bg(*c)
                })),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(space::S4)
                .children(parts.iter().map(|(label, b, c)| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(text::SM)
                        .text_color(p.text_muted)
                        .child(div().size(px(8.)).rounded(px(2.)).bg(*c))
                        .child(*label)
                        .child(
                            div()
                                .font_weight(theme::W_SEMIBOLD)
                                .text_color(p.text)
                                .child(format_measured(*b)),
                        )
                })),
        )
        .into_any_element()
}

/// `count`: "1,204 frames".
pub fn count(n: i64, one: &str) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let sign = if n < 0 { "-" } else { "" };
    format!("{sign}{grouped} {one}{}", if n == 1 { "" } else { "s" })
}

pub const CLEANUP: &str = "Retention cleanup";

/// The resolved folder as a read-only mono chip (`.storage-path`); home as `~`.
fn path_chip(s: &Settings, p: &Palette) -> AnyElement {
    let home = std::env::var("HOME").unwrap_or_default();
    let home = home.trim_end_matches('/');
    let location =
        s.ui.picked_location
            .clone()
            .or(s.data.storage_location.clone());
    let chip = div()
        .flex()
        .items_center()
        .flex_shrink(1.)
        .min_w_0()
        .max_w(px(320.))
        .h(theme::size::H_MD)
        .px(space::S3)
        .rounded(radius::MD)
        .border_1()
        .border_color(p.border)
        .bg(p.surface_subtle)
        .font_family(FONT_MONO)
        .text_size(text::BASE);
    match location {
        Some(l) => {
            let shown = if !home.is_empty() && (l == home || l.starts_with(&format!("{home}/"))) {
                format!("~{}", &l[home.len()..])
            } else {
                l.clone()
            };
            crate::ui::with_tip(
                chip.id("storage-path").text_color(p.text).child(
                    div()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(shown),
                ),
                l,
            )
            .into_any_element()
        }
        None if s.data.storage_location_error.is_some() => chip
            .text_color(p.text_subtle)
            .child("\u{2014}")
            .into_any_element(),
        None => chip
            .id("storage-path")
            .role(gpui_kit::Role::Status)
            .aria_label("Resolving storage location\u{2026}")
            .child(skeleton::line(1., p).w(px(90.)))
            .into_any_element(),
    }
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let d = s.d();
    let running = s.ui.busy.contains(CLEANUP);
    let applied = s.data.storage_location.clone();
    let pending_restart = matches!(
        (&s.ui.picked_location, &applied),
        (Some(picked), Some(applied)) if picked != applied
    );
    let mut rows: Vec<AnyElement> = Vec::new();
    if pending_restart {
        let restarting = s.ui.restarting;
        rows.push(
            div()
                .mb(space::S3)
                .child(notice::notice(
                    Tone::Warn,
                    icons::FOLDER,
                    "Saved \u{2014} takes effect after a restart",
                    Some("Captures already on disk stay where they are. Any recording in progress is saved first.".into()),
                    vec![Button::new("storage-restart", if restarting { "Restarting\u{2026}" } else { "Restart Mnema" })
                        .sm()
                        .busy(restarting)
                        .disabled(restarting)
                        .on_click(click(cx, |s, cx| s.restart_for_storage(cx)))
                        .into_any_element()],
                    p,
                ))
                .into_any_element(),
        );
    }
    rows.push(meter(s, p));
    let location_error = s.data.storage_location_error.clone().map(|e| {
        let resolved = s.ui.picked_location.is_some() || s.data.storage_location.is_some();
        let words = if resolved {
            e
        } else {
            "Couldn\u{2019}t resolve storage location.".into()
        };
        div()
            .flex()
            .items_center()
            .gap(space::S2)
            .child(line(Tone::Danger, icons::ALERT, words, p))
            .when(!resolved, |d| {
                d.child(
                    Button::new("storage-location-retry", "Retry")
                        .ghost()
                        .sm()
                        .on_click(click(cx, |s, cx| s.refresh(cx))),
                )
            })
    });
    rows.push(
        row("Save directory")
            .desc("Where captures and the database live on disk. Downloaded models stay in Mnema's app data.")
            .control(path_chip(s, p))
            .control(
                Button::new("storage-browse", "Browse\u{2026}")
                    .icon(icons::FOLDER)
                    .on_click(browse(cx)),
            )
            .below_opt(location_error)
            .build(p)
            .into_any_element(),
    );
    let cleanup_line = match &s.ui.cleanup {
        Some(Err(e)) => Some(
            div()
                .flex()
                .items_center()
                .gap(space::S2)
                .child(line(
                    Tone::Danger,
                    icons::ALERT,
                    format!("Cleanup failed: {e}"),
                    p,
                ))
                .child(
                    Button::new("retention-retry", "Retry")
                        .ghost()
                        .sm()
                        .disabled(running)
                        .on_click(click(cx, |s, cx| s.ask_retention_cleanup(cx))),
                )
                .into_any_element(),
        ),
        // `<b>Latest cleanup</b> 12 segments, …`
        Some(Ok((segments, frames, audio))) => Some(
            notice::inline(
                Tone::Ok,
                Some(
                    div()
                        .flex_none()
                        .mt(px(3.))
                        .child(crate::ui::icon(icons::CHECK, theme::size::ICON_SM))
                        .into_any_element(),
                ),
                vec![
                    div()
                        .flex_none()
                        .font_weight(theme::W_SEMIBOLD)
                        .child("Latest cleanup")
                        .into_any_element(),
                    div()
                        .min_w_0()
                        .child(format!(
                            "{}, {}, {}.",
                            count(*segments, "segment"),
                            count(*frames, "frame"),
                            count(*audio, "audio segment")
                        ))
                        .into_any_element(),
                ],
                p,
            )
            .flex_nowrap()
            .items_start()
            .max_w_full()
            .into_any_element(),
        ),
        None => None,
    };
    rows.push(
        row("Retention")
            .desc("Automatically delete captured data after the chosen window. Shortening it shows what will go first. Your User Context is kept.")
            .control(
                Segmented::new("retention", "Retention duration", ["7 days", "14 days", "30 days", "Forever"], ix(&RETENTION, &d.retention_policy))
                    .md()
                    .on_change(on(cx, |s, i: usize, cx| s.set_retention(RETENTION[i].clone(), cx))),
            )
            .control(
                Button::new("retention-run", if running { "Running\u{2026}" } else { "Run cleanup now" })
                    .ghost()
                    .busy(running)
                    .disabled(running)
                    .on_click(click(cx, |s, cx| s.ask_retention_cleanup(cx))),
            )
            .below_opt(cleanup_line)
            .build(p)
            .into_any_element(),
    );
    section("Storage", None, Vec::new(), false, rows, p).into_any_element()
}

impl Settings {
    /// "Run cleanup now" asks first (it deletes what the policy marks old).
    pub fn ask_retention_cleanup(&mut self, cx: &mut Context<Self>) {
        crate::settings::confirm(
            cx,
            "Run cleanup now",
            "Run cleanup now? This can delete captured data that matches the current retention policy.",
            "Run cleanup",
            |s, cx| s.run_retention_cleanup(cx),
        );
    }

    /// `run_retention_cleanup_now`; its counts (or failure) show under Retention.
    pub fn run_retention_cleanup(&mut self, cx: &mut Context<Self>) {
        let Some(core) = crate::chat::core(cx) else {
            return;
        };
        if !self.ui.busy.insert(CLEANUP) {
            return;
        }
        let job = cx.background_spawn(async move { crate::settings::live::retention_run(&core) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |s, cx| {
                s.ui.busy.remove(CLEANUP);
                if let Err(e) = &result {
                    log::warn!("settings: retention cleanup failed: {e}");
                }
                s.ui.cleanup = Some(result);
                s.refresh(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// "Restart Mnema" for a new save folder: asks, then the graceful restart
    /// (an in-progress recording is finalized first).
    fn restart_for_storage(&mut self, cx: &mut Context<Self>) {
        if self.ui.restarting {
            return;
        }
        crate::settings::confirm(
            cx,
            "Restart Mnema",
            "Restart now? Any in-progress recording will be saved first.",
            "OK",
            |s, cx| {
                let Some(core) = crate::chat::core(cx) else {
                    return;
                };
                s.ui.restarting = true;
                cx.notify();
                cx.background_spawn(async move { core.shutdown(app_core::ExitKind::Restart) })
                    .detach();
            },
        );
    }
}
