//! Intelligence › Semantic Search (`SemanticSearch.svelte`): the switch and the
//! picked-model row (recommended tiers first, then the custom catalog).

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};
use serde_json::Value;

use super::model_row::ModelRow;
use crate::settings::model::{format_bytes, Json};
use crate::settings::row::{row, section};
use crate::settings::{on, reload, Settings};
use crate::theme::Palette;
use crate::ui::chip::ChipKind;
use crate::ui::combobox::Combobox;
use crate::ui::switch::Switch;

fn tier_label(tier: &str) -> &'static str {
    match tier {
        "english" => "English",
        "multilingual" => "Multilingual",
        _ => "Custom",
    }
}

/// `semanticSearchModelOptions`: `(model id, label)`.
fn options(status: Option<&Value>, supported: &[Value]) -> Vec<(String, String)> {
    let guided: Vec<&Value> = status
        .and_then(|s| s["models"].as_array())
        .map(|m| {
            m.iter()
                .filter(|m| m.str_at(&["tier"]) != Some("custom"))
                .collect()
        })
        .unwrap_or_default();
    let mut out: Vec<(String, String)> = guided
        .iter()
        .map(|m| {
            let multi = if m.str_at(&["tier"]) == Some("multilingual") {
                " \u{b7} multilingual"
            } else {
                ""
            };
            (
                m.str_at(&["modelId"]).unwrap_or_default().to_string(),
                format!(
                    "{} \u{b7} {}d{multi} \u{b7} recommended",
                    m.str_at(&["displayName"]).unwrap_or_default(),
                    m.num_at(&["dimension"]).unwrap_or(0.)
                ),
            )
        })
        .collect();
    for c in supported {
        let id = c.str_at(&["modelId"]).unwrap_or_default();
        if guided.iter().any(|g| g.str_at(&["modelId"]) == Some(id)) {
            continue;
        }
        let multi = if c.bool_at(&["multilingual"]) {
            " \u{b7} multilingual"
        } else {
            ""
        };
        out.push((
            id.to_string(),
            format!(
                "{} \u{2014} {}d{multi}",
                c.str_at(&["displayName"]).unwrap_or_default(),
                c.num_at(&["dimension"]).unwrap_or(0.)
            ),
        ));
    }
    out
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let d = s.d();
    let ss = &d.semantic_search;
    let status = s.data.semantic_models.as_ref();
    // shortcut: list_semantic_search_supported_models (the custom catalog) isn't loaded
    // into SettingsData yet; the picker lists the status's models.
    let opts = options(status, &[]);
    let picked_id = s.ui.semantic_pick.clone().or_else(|| ss.model_id.clone());
    let picked = picked_id.as_ref().and_then(|id| {
        status
            .and_then(|s| s["models"].as_array())
            .and_then(|ms| {
                ms.iter()
                    .find(|m| m.str_at(&["modelId"]) == Some(id.as_str()))
            })
            .cloned()
    });
    let ids: Vec<String> = opts.iter().map(|(i, _)| i.clone()).collect();
    let sel = picked_id
        .as_ref()
        .and_then(|id| ids.iter().position(|i| i == id));
    let picker = div()
        .w(px(300.))
        .child(
            Combobox::new(
                "ss-model",
                "Semantic search model",
                opts.iter().map(|(_, l)| l.clone()),
                sel,
            )
            .placeholder("Select a model\u{2026}")
            .search_placeholder("Search models\u{2026}")
            .on_change(on(cx, move |s, i: usize, cx| {
                s.ui.semantic_pick = ids.get(i).cloned();
                cx.notify();
            })),
        )
        .into_any_element();
    let model = match &picked {
        None => ModelRow {
            label: "Model",
            desc: Some(if status.is_some() {
                "Recommended tiers first. Nothing downloads until you choose one.".into()
            } else {
                "No search model status is available.".into()
            }),
            controls: vec![picker],
            ..Default::default()
        },
        Some(m) => {
            let available = m.bool_at(&["available"]);
            let failed = m.str_at(&["status"]) == Some("failed");
            let selected = ss.model_id.as_deref() == m.str_at(&["modelId"]) && available;
            let bytes = m.num_at(&["approxDownloadBytes"]).unwrap_or(0.);
            let licence = m
                .str_at(&["licenseLabel"])
                .map(|l| format!(" \u{b7} {l}"))
                .unwrap_or_default();
            let meta = format!(
                "{} \u{b7} {} on disk \u{b7} {}-dim \u{b7} runs on-device{licence}",
                tier_label(m.str_at(&["tier"]).unwrap_or_default()),
                format_bytes(bytes),
                m.num_at(&["dimension"]).unwrap_or(0.)
            );
            let step = if !available && !failed {
                " Step 1: download \u{b7} Step 2: use this model."
            } else {
                ""
            };
            ModelRow {
                label: "Model",
                chip: Some(match (available, failed, selected) {
                    (false, true, _) => ("failed".into(), ChipKind::Danger),
                    (false, false, _) => ("not installed".into(), ChipKind::Neutral),
                    (true, _, true) => ("active".into(), ChipKind::Accent),
                    _ => ("installed".into(), ChipKind::Neutral),
                }),
                desc: Some(format!("{meta}.{step}")),
                controls: vec![picker],
                // shortcut: start_semantic_search_model_download + select_semantic_search_model
                // (the atomic re-index) are src-tauri until S3; Download is shown, not wired.
                download: (!available).then_some((Some(bytes as u64), failed)),
                folder: status
                    .and_then(|v| v["modelsDirectory"].as_str())
                    .map(str::to_string),
                ..Default::default()
            }
        }
    };
    let rows = vec![
        row("Enable semantic search")
            .desc("Fuse meaning-based results with keyword search. Inert until a model below is installed; indexes in the background, which costs CPU and battery while it catches up.")
            .control(
                Switch::new("ss-enabled", "Enable semantic search", ss.enabled)
                    // shortcut: update_semantic_search_settings is its own src-tauri command until S3.
                    .on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.semantic_search.enabled = v, cx))),
            )
            .build(p)
            .into_any_element(),
        model.build("ss-model-row", s, p, cx),
    ];
    section(
        "Semantic Search",
        Some("Meaning-based search, fully on-device."),
        vec![reload::button(
            s,
            "reload-ss",
            "Refresh semantic search model status",
            false,
            |s, cx| s.reload_all(cx),
            cx,
        )],
        false,
        rows,
        p,
    )
    .into_any_element()
}
