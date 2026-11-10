//! Data › Storage (`Storage.svelte`, `StorageMeter.svelte`): the usage meter,
//! the save directory and retention.

use capture_types::RetentionPolicy;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Hsla};

use super::ix;
use crate::settings::model::{format_bytes, format_measured, Json};
use crate::settings::row::{row, section};
use crate::settings::{icons, on, Settings};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::segmented::Segmented;
use crate::ui::Button;

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
            let _ = this.update(cx, |s, cx| {
                s.data.storage_location = Some(path.clone());
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
        return div().h(px(80.)).into_any_element();
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

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let d = s.d();
    let home = std::env::var("HOME").unwrap_or_default();
    let location = s.data.storage_location.clone().unwrap_or_default();
    let shown = if !home.is_empty() && location.starts_with(&home) {
        format!("~{}", &location[home.len()..])
    } else {
        location.clone()
    };
    let rows = vec![
        meter(s, p),
        row("Save directory")
            .desc("Where captures and the database live on disk. Downloaded models stay in Mnema's app data.")
            .control(div().font_family(FONT_MONO).text_size(text::SM).text_color(p.text_muted).child(shown))
            .control(
                Button::new("storage-browse", "Browse\u{2026}")
                    .icon(icons::FOLDER)
                    .on_click(browse(cx)),
            )
            .build(p)
            .into_any_element(),
        row("Retention")
            .desc("Automatically delete captured data after the chosen window. Shortening it shows what will go first. Your User Context is kept.")
            .control(
                Segmented::new("retention", "Retention duration", ["7 days", "14 days", "30 days", "Forever"], ix(&RETENTION, &d.retention_policy))
                    .md()
                    .on_change(on(cx, |s, i: usize, cx| s.set_retention(RETENTION[i].clone(), cx))),
            )
            .control(
                Button::new("retention-run", "Run cleanup now")
                    .ghost()
                    .disabled(s.ui.busy.contains("Retention cleanup"))
                    .on_click(crate::settings::click(cx, |s, cx| {
                        s.run_command("Retention cleanup", cx, crate::settings::live::retention_run)
                    })),
            )
            .build(p)
            .into_any_element(),
    ];
    section("Storage", None, Vec::new(), false, rows, p).into_any_element()
}
