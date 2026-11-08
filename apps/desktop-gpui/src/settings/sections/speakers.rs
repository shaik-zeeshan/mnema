//! Intelligence › Speakers (`Speakers.svelte`): separation, recognition, the
//! helper timeout, the preset and the speaker model row.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};

use super::model_row::{status_row, ModelRow};
use crate::settings::data::Family;
use crate::settings::model::{format_bytes, provider_models, selected_model};
use crate::settings::row::{row, section};
use crate::settings::{on, reload, Settings};
use crate::theme::Palette;
use crate::ui::combobox::Combobox;
use crate::ui::slider::Slider;
use crate::ui::switch::Switch;

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let d = s.d();
    let a = &d.speaker_analysis;
    let off = !a.separate_speakers;
    let minutes = (a.timeout_seconds / 60).max(1);
    let status = s.data.speaker_models.as_ref();
    let presets: Vec<(Option<String>, String)> = provider_models(status)
        .into_iter()
        .find(|(pv, _, _)| *pv == a.provider)
        .map(|(_, _, ms)| {
            ms.into_iter()
                .map(|m| (m.model_id, m.display_name))
                .collect()
        })
        .unwrap_or_default();
    let ids: Vec<Option<String>> = presets.iter().map(|(i, _)| i.clone()).collect();
    let sel = ids
        .iter()
        .position(|i| i.as_deref() == a.model_id.as_deref());
    let mut rows = vec![
        row("Separate speakers in transcripts")
            .desc("Queue local diarization after successful microphone transcription.")
            .control(
                Switch::new("spk-separate", "Separate speakers in transcripts", a.separate_speakers)
                    .on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.speaker_analysis.separate_speakers = v, cx))),
            )
            .build(p)
            .into_any_element(),
        row("Recognize saved people")
            .desc("Opt in to matching against confirmed local Person voice profiles.")
            .disabled(off)
            .control(
                Switch::new("spk-recognize", "Recognize saved people", a.recognize_saved_people)
                    .disabled(off)
                    .on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.speaker_analysis.recognize_saved_people = v, cx))),
            )
            .build(p)
            .into_any_element(),
        row("Helper timeout")
            .desc("Stops speaker analysis if it runs too long. Queued jobs keep the timeout they were created with.")
            .disabled(off)
            .control(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .w(px(220.))
                    .child(div().flex_1().child(Slider::new(&s.sliders.speaker_timeout, "Helper timeout").disabled(off)))
                    .child(div().w(px(34.)).text_color(p.text_muted).child(format!("{minutes}m"))),
            )
            .build(p)
            .into_any_element(),
        row("Preset")
            .desc("Pick by intent. Switching is reversible, but saved voices need a one-time re-tag under a new preset.")
            .disabled(off)
            .control(
                div().w(px(240.)).child(
                    Combobox::new(
                        "spk-preset",
                        "Preset",
                        if presets.is_empty() { vec!["Loading preset options".to_string()] } else { presets.iter().map(|(_, n)| n.clone()).collect() },
                        sel.or(Some(0)),
                    )
                    .search_placeholder("Search presets\u{2026}")
                    .disabled(off)
                    .on_change(on(cx, move |s, i: usize, cx| {
                        if let Some(id) = ids.get(i).cloned() {
                            s.edit(|d| d.speaker_analysis.model_id = id, cx)
                        }
                    })),
                ),
            )
            .build(p)
            .into_any_element(),
    ];
    let m = selected_model(status, &a.provider, a.model_id.as_deref());
    let model = match &m {
        None => ModelRow {
            label: "Speaker model",
            warning: Some("No speaker model status is available.".into()),
            ..Default::default()
        },
        Some(m) => {
            let failed = m.status == "failed";
            let mut r = status_row("Speaker model", m, failed, "installed");
            let word = if m.available {
                "installed".to_string()
            } else {
                if m.status == "not_installed" {
                    "missing".to_string()
                } else {
                    m.status.replace('_', " ")
                }
            };
            let size = m
                .download_bytes
                .map(|b| format!(" \u{b7} {}", format_bytes(b as f64)))
                .unwrap_or_default();
            r.desc = Some(format!("{} \u{b7} {word}{size}", m.display_name));
            r.folder = status
                .and_then(|v| v["modelsDirectory"].as_str())
                .map(str::to_string);
            r.start = m
                .model_id
                .clone()
                .map(|id| (Family::Speaker, m.provider.clone(), id));
            r
        }
    };
    rows.push(model.build("spk-model", p, cx));
    section(
        "Speakers",
        Some("Anonymous separation first; naming saved people only when you opt in."),
        vec![reload::button(
            s,
            "reload-spk",
            "Refresh speaker model status",
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
