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
use crate::ui::Button;

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
    // shortcut: the picker lists the status's models (the recommended tiers), not the
    // custom catalog (`list_semantic_search_supported_models`); add it with a search field.
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
                controls: {
                    let mut c = vec![picker];
                    let mid = m.str_at(&["modelId"]).unwrap_or_default().to_string();
                    if available && !selected {
                        // The atomic switch re-indexes everything: ask first.
                        let use_id = mid.clone();
                        c.push(
                            Button::new("ss-use", "Use this model")
                                .disabled(s.ui.busy.contains("Search model"))
                                .on_click(crate::settings::click(cx, move |_, cx| {
                                    let id = use_id.clone();
                                    crate::settings::confirm(
                                        cx,
                                        "Switch search model?",
                                        "Switching rebuilds the meaning index from scratch with the new model. Keyword search keeps working while it catches up.",
                                        "Switch",
                                        move |s, cx| {
                                            s.ui.semantic_pick = None;
                                            s.run_command("Search model", cx, move |core| {
                                                crate::settings::live::semantic_select(core, id)
                                            })
                                        },
                                    );
                                }))
                                .into_any_element(),
                        );
                    }
                    if available && !selected {
                        let del = mid.clone();
                        c.push(
                            Button::new("ss-delete", "Delete files")
                                .ghost()
                                .on_click(crate::settings::click(cx, move |_, cx| {
                                    let id = del.clone();
                                    crate::settings::confirm(
                                        cx,
                                        "Delete search model files?",
                                        "The downloaded model files are removed from this Mac. You can download them again.",
                                        "Delete",
                                        move |s, cx| {
                                            s.run_command("Search model", cx, move |core| {
                                                crate::settings::live::semantic_delete(core, id)
                                            })
                                        },
                                    );
                                }))
                                .into_any_element(),
                        );
                    }
                    c
                },
                download: (!available).then_some((Some(bytes as u64), failed)),
                start: m.str_at(&["modelId"]).map(|id| {
                    (
                        crate::settings::data::Family::Semantic,
                        "local".to_string(),
                        id.to_string(),
                    )
                }),
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
                    .on_toggle(on(cx, |s, v: bool, cx| s.set_semantic_enabled(v, cx))),
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
