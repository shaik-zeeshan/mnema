//! Intelligence › Semantic Search (`SemanticSearch.svelte` +
//! `controller-semantic-search.svelte.ts`): the switch and the picked-model row.
//! The picker lists the recommended tiers first, then the rest of the embedding
//! catalog (`list_semantic_search_supported_models`, read once per visit through
//! the backend). Switching away from a model rebuilds the index, so it asks first.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Entity, Window};
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

/// The busy / note name for a model switch or delete.
const BUSY: &str = "Search model";

fn tier_label(tier: &str) -> &'static str {
    match tier {
        "english" => "English",
        "multilingual" => "Multilingual",
        _ => "Custom",
    }
}

/// `semanticSearchModelOptions`: `(model id, label)`, recommended tiers first.
pub(super) fn options(status: Option<&Value>, supported: &[Value]) -> Vec<(String, String)> {
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

/// `SemanticSearchPickedView`: a status model, or a catalog-only pick.
#[derive(Debug, PartialEq)]
pub(super) struct Picked {
    pub id: String,
    pub name: String,
    pub meta: String,
    pub available: bool,
    pub failed: bool,
    pub bytes: Option<f64>,
    pub provider: Option<String>,
}

pub(super) fn picked(status: Option<&Value>, catalog: &[Value], id: &str) -> Option<Picked> {
    let models = status.and_then(|s| s["models"].as_array());
    if let Some(m) = models.and_then(|ms| ms.iter().find(|m| m.str_at(&["modelId"]) == Some(id))) {
        let bytes = m.num_at(&["approxDownloadBytes"]);
        let licence = m
            .str_at(&["licenseLabel"])
            .map(|l| format!(" \u{b7} {l}"))
            .unwrap_or_default();
        return Some(Picked {
            id: id.into(),
            name: m.str_at(&["displayName"]).unwrap_or_default().into(),
            meta: format!(
                "{} \u{b7} {} on disk \u{b7} {}-dim \u{b7} runs on-device{licence}",
                tier_label(m.str_at(&["tier"]).unwrap_or_default()),
                format_bytes(bytes.unwrap_or(0.)),
                m.num_at(&["dimension"]).unwrap_or(0.)
            ),
            available: m.bool_at(&["available"]),
            failed: m.str_at(&["status"]) == Some("failed"),
            bytes,
            provider: m.str_at(&["provider"]).map(Into::into),
        });
    }
    let c = catalog
        .iter()
        .find(|c| c.str_at(&["modelId"]) == Some(id))?;
    let bytes = c.num_at(&["approxDownloadBytes"]);
    let size = bytes
        .map(|b| format!("{} on disk \u{b7} ", format_bytes(b)))
        .unwrap_or_default();
    let multi = if c.bool_at(&["multilingual"]) {
        " \u{b7} multilingual"
    } else {
        ""
    };
    Some(Picked {
        id: id.into(),
        name: c.str_at(&["displayName"]).unwrap_or_default().into(),
        meta: format!(
            "Custom \u{b7} {size}{}-dim \u{b7} runs on-device{multi}",
            c.num_at(&["dimension"]).unwrap_or(0.)
        ),
        available: false,
        failed: false,
        bytes,
        // A catalog-only model downloads through the status's provider.
        provider: models
            .and_then(|ms| ms.first())
            .and_then(|m| m.str_at(&["provider"]))
            .map(Into::into),
    })
}

/// The catalog for this visit: `None` while it loads.
pub(super) struct Catalog(pub Option<Result<Vec<Value>, String>>);

fn catalog(s: &Settings, window: &mut Window, cx: &mut Context<Settings>) -> Entity<Catalog> {
    let backend = s.backend.clone();
    window.use_keyed_state("ss-catalog", cx, move |_, cx| {
        let job = cx.background_spawn(async move { backend.semantic_catalog() });
        cx.spawn(async move |this, cx| {
            let answer = job.await;
            if let Err(e) = &answer {
                log::warn!("settings: couldn't list the search model catalog: {e}");
            }
            let _ = this.update(cx, |c: &mut Catalog, cx| {
                c.0 = Some(answer);
                cx.notify();
            });
        })
        .detach();
        Catalog(None)
    })
}

/// `chooseSemanticSearchModel`: the first pick switches at once; any later
/// switch rebuilds the index, so it asks first (`settings::confirm`).
pub(super) fn choose(s: &mut Settings, id: String, name: String, cx: &mut Context<Settings>) {
    if s.ui.busy.contains(BUSY) {
        return;
    }
    let current = s.d().semantic_search.model_id;
    if current.as_deref() == Some(id.as_str()) {
        return;
    }
    if current.is_none() {
        return switch(s, id, true, cx);
    }
    crate::settings::confirm(
        cx,
        "Re-index for new search model?",
        &format!("Switching to \u{201c}{name}\u{201d} re-indexes every recording: all existing meaning vectors are cleared and re-derived under the new model in the background. Your captures are not changed."),
        "Switch & Re-index",
        move |s, cx| switch(s, id, false, cx),
    );
}

fn switch(s: &mut Settings, id: String, first: bool, cx: &mut Context<Settings>) {
    if !s.ui.busy.insert(BUSY) {
        return;
    }
    let backend = s.backend.clone();
    let model = id.clone();
    let job = cx.background_spawn(async move { backend.select_semantic_model(&model) });
    cx.spawn(async move |this, cx| {
        let answer = job.await;
        let _ = this.update(cx, |s, cx| {
            s.ui.busy.remove(BUSY);
            match answer {
                Ok(cleared) => {
                    // app-core persisted it; the draft and the saved copy follow
                    // (no autosave unit owns this field).
                    for d in [s.draft.as_mut(), s.saved.as_mut()].into_iter().flatten() {
                        d.semantic_search.model_id = Some(id.clone());
                    }
                    if !first {
                        let text = if cleared > 0 {
                            format!(
                                "Cleared {cleared} vector{}; re-indexing in the background.",
                                if cleared == 1 { "" } else { "s" }
                            )
                        } else {
                            "Re-index started in the background.".into()
                        };
                        s.ui.note = Some((true, BUSY, text));
                    }
                }
                Err(e) => {
                    log::warn!("settings: the search model switch failed: {e}");
                    s.ui.note = Some((false, BUSY, e));
                }
            }
            cx.notify();
        });
    })
    .detach();
    cx.notify();
}

pub fn render(
    s: &Settings,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let d = s.d();
    let ss = &d.semantic_search;
    let status = s.data.semantic_models.as_ref();
    let loaded = catalog(s, window, cx).read(cx).0.clone();
    let supported = loaded.as_ref().and_then(|r| r.as_ref().ok());
    let opts = options(status, supported.map_or(&[][..], Vec::as_slice));
    let picked_id = s.ui.semantic_pick.clone().or_else(|| ss.model_id.clone());
    let picked = picked_id
        .as_deref()
        .and_then(|id| picked(status, supported.map_or(&[][..], Vec::as_slice), id));
    let catalog_warning = match &loaded {
        Some(Err(e)) => Some(format!("Custom model list failed: {e}")),
        _ => None,
    };
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
    let busy = s.ui.busy.contains(BUSY);
    let model = match &picked {
        None => ModelRow {
            label: "Model",
            desc: status
                .map(|_| "Recommended tiers first. Nothing downloads until you choose one.".into()),
            warning: if status.is_some() {
                catalog_warning
            } else {
                Some("No search model status is available.".into())
            },
            controls: if status.is_some() {
                vec![picker]
            } else {
                Vec::new()
            },
            ..Default::default()
        },
        Some(m) => {
            let selected = ss.model_id.as_deref() == Some(m.id.as_str());
            let step = if !m.available && !m.failed {
                " Step 1: download \u{b7} Step 2: use this model."
            } else {
                ""
            };
            let mut controls = vec![picker];
            if m.available && !selected {
                // Step 2, never automatic (ADR 0036).
                let (id, name) = (m.id.clone(), m.name.clone());
                controls.push(
                    Button::new(
                        "ss-use",
                        if busy {
                            "Re-indexing"
                        } else {
                            "Use this model"
                        },
                    )
                    .disabled(busy)
                    .on_click(crate::settings::click(cx, move |s, cx| {
                        choose(s, id.clone(), name.clone(), cx)
                    }))
                    .into_any_element(),
                );
                // Installed but not active = unused: its disk can be reclaimed.
                let del = m.id.clone();
                controls.push(
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
                                    s.run_command(BUSY, cx, move |core| {
                                        crate::settings::live::semantic_delete(core, id)
                                    })
                                },
                            );
                        }))
                        .into_any_element(),
                );
            }
            ModelRow {
                label: "Model",
                chip: Some(match (m.available, m.failed, selected) {
                    (false, true, _) => ("failed".into(), ChipKind::Danger),
                    (false, false, _) => ("not installed".into(), ChipKind::Neutral),
                    (true, _, true) => ("active".into(), ChipKind::Accent),
                    _ => ("installed".into(), ChipKind::Neutral),
                }),
                desc: Some(format!("{}.{step}", m.meta)),
                warning: catalog_warning,
                controls,
                download: (!m.available).then_some((m.bytes.map(|b| b as u64), m.failed)),
                start: m.provider.clone().map(|provider| {
                    (
                        crate::settings::data::Family::Semantic,
                        provider,
                        m.id.clone(),
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
