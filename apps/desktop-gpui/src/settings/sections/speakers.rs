//! Intelligence › Speakers (`Speakers.svelte`): separation, recognition, the
//! helper timeout, the preset (every provider's models; switching away from
//! the saved one asks first when saved people would need a re-tag) and the
//! speaker model row with Download / Cancel / Delete.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};

use super::model_row::{no_status_row, status_row};
use crate::settings::data::Family;
use crate::settings::model::{format_bytes, provider_models, ModelView};
use crate::settings::row::{row, section};
use crate::settings::{on, reload, Settings};
use crate::ui::Button;

/// The busy name of a speaker model delete.
const DELETING: &str = "Speaker model";

/// `speakerModelOptions`: every provider's models as `((provider, id), label)`,
/// the label carrying the download size when there is one.
fn presets(status: Option<&serde_json::Value>) -> Vec<((String, Option<String>), String)> {
    provider_models(status)
        .into_iter()
        .flat_map(|(_, _, models)| models)
        .map(|m| {
            let label = match m.download_bytes {
                Some(b) => format!("{} \u{b7} {}", m.display_name, format_bytes(b as f64)),
                None => m.display_name.clone(),
            };
            ((m.provider, m.model_id), label)
        })
        .collect()
}

/// `selectedSpeakerModel`: provider + id, else the id alone.
fn selected(
    status: Option<&serde_json::Value>,
    provider: &str,
    id: Option<&str>,
) -> Option<ModelView> {
    let all: Vec<ModelView> = provider_models(status)
        .into_iter()
        .flat_map(|(_, _, models)| models)
        .collect();
    all.iter()
        .find(|m| m.provider == provider && m.model_id.as_deref() == id)
        .or_else(|| all.iter().find(|m| m.model_id.as_deref() == id))
        .cloned()
}

/// `chooseSpeakerModel`: leaving the saved preset while saved people are
/// recognised asks first (their voices need a one-time re-tag).
fn choose(s: &mut Settings, provider: String, id: Option<String>, cx: &mut Context<Settings>) {
    let a = s.d().speaker_analysis;
    if a.provider == provider && a.model_id == id {
        return;
    }
    let saved = s.saved.as_ref().map(|r| &r.speaker_analysis);
    let away = saved.is_none_or(|v| v.provider != provider || v.model_id != id);
    let apply = move |s: &mut Settings, cx: &mut Context<Settings>| {
        s.edit(
            move |d| {
                d.speaker_analysis.provider = provider;
                d.speaker_analysis.model_id = id;
            },
            cx,
        )
    };
    if away && a.recognize_saved_people && s.data.person_count > 0 {
        crate::settings::commands::confirm_as(
            cx,
            "Switch speaker model?",
            "Switching the speaker model is safe and reversible \u{2014} your saved people are not deleted. But saved voices won't be recognized under the new model until you re-tag each person once. Switching back to the previous model restores them. Switch anyway?",
            "Switch",
            "Keep current",
            apply,
        );
    } else {
        apply(s, cx);
    }
}

/// `deleteSpeakerModel`: asks, deletes, then says so on the row.
fn delete(m: &ModelView, cx: &mut Context<Settings>) {
    let (Some(id), provider, name) = (
        m.model_id.clone(),
        m.provider.clone(),
        m.display_name.clone(),
    ) else {
        return;
    };
    crate::settings::confirm(
        cx,
        "Delete speaker model",
        &format!("Delete {name}?"),
        "Delete",
        move |s, cx| {
            s.ui.model_notes.remove(&Family::Speaker);
            s.run_command_with(
                DELETING,
                cx,
                move |core| crate::settings::live::delete_speaker_model(core, provider, id),
                move |s, outcome| {
                    let note = match outcome {
                        Ok(_) => (true, format!("Deleted {name}.")),
                        Err(e) => (false, format!("Speaker model action failed: {e}")),
                    };
                    s.ui.model_notes.insert(Family::Speaker, note);
                    true
                },
            );
        },
    );
}
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
    let presets = presets(status);
    let m = selected(status, &a.provider, a.model_id.as_deref());
    let keys: Vec<(String, Option<String>)> = presets.iter().map(|(k, _)| k.clone()).collect();
    let sel = keys.iter().position(|(pv, id)| match &m {
        Some(m) => *pv == m.provider && *id == m.model_id,
        None => *pv == a.provider && *id == a.model_id,
    });
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
                        if let Some((provider, id)) = keys.get(i).cloned() {
                            choose(s, provider, id, cx)
                        }
                    })),
                ),
            )
            .build(p)
            .into_any_element(),
    ];
    let model = match &m {
        None => no_status_row(
            s,
            Family::Speaker,
            "Speaker model",
            "Couldn\u{2019}t load speaker model status: ",
            "Checking installed speaker models\u{2026}",
            "No speaker model status is available.",
        ),
        Some(m) => {
            let failed = m.status == "failed";
            let mut r = status_row("Speaker model", m, failed, "installed");
            // `speakerStatusLabel`, lower-cased.
            let word = match m.status.as_str() {
                "installed" | "downloading" | "failed" | "incomplete" => m.status.as_str(),
                _ => "missing",
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
            r.name = Some("speaker model".into());
            if m.available {
                let deleting = s.ui.busy.contains(DELETING);
                let target = m.clone();
                r.controls.push(
                    Button::new("spk-delete", if deleting { "Deleting" } else { "Delete" })
                        .ghost()
                        .spinner(deleting)
                        .disabled(deleting)
                        .on_click(crate::settings::click(cx, move |_, cx| delete(&target, cx)))
                        .into_any_element(),
                );
            }
            r
        }
    };
    rows.push(model.build("spk-model", s, p, cx));
    section(
        "Speakers",
        Some("Anonymous separation first; naming saved people only when you opt in."),
        vec![reload::button(
            s,
            "reload-spk",
            "Refresh speaker model status",
            s.ui.models_loading,
            |s, cx| s.reload_all(cx),
            cx,
        )],
        false,
        rows,
        p,
    )
    .into_any_element()
}
