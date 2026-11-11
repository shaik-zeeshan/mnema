//! The local-model sections against Svelte: unreadable / loading model status,
//! the speaker preset switch warning and Delete, Semantic Search's Delete and
//! language line, the voiceprint's Delete, a download that won't start, and
//! the Deepgram key placeholder. Confirms go through `confirm_core`'s host.

use gpui_kit::{Entity, TestAppContext};
use serde_json::json;

use super::model_row::no_status_row;
use super::pickers_tests::{choose, click, confirm_core, open, present};
use crate::settings::data::Family;
use crate::settings::Settings;

/// `(error, desc, warning)` of a family's no-status row.
fn no_status(
    cx: &mut TestAppContext,
    v: &Entity<Settings>,
    family: Family,
) -> (Option<String>, Option<String>, Option<String>) {
    v.read_with(cx, |s, _| {
        let r = no_status_row(s, family, "Model", "Couldn’t load: ", "Checking…", "None.");
        (r.error, r.desc, r.warning)
    })
}

#[gpui_kit::test]
fn an_unreadable_status_says_why_and_a_loading_one_says_checking(cx: &mut TestAppContext) {
    let (_, v, _) = open(cx, "ocr");
    v.update(cx, |s, _| {
        s.data.model_errors.insert(
            Family::Ocr,
            "failed to inspect OCR models: bad manifest".into(),
        );
    });
    assert_eq!(
        no_status(cx, &v, Family::Ocr),
        (
            Some("Couldn’t load: failed to inspect OCR models: bad manifest".into()),
            None,
            None
        )
    );
    v.update(cx, |s, _| s.ui.models_loading = true);
    assert_eq!(
        no_status(cx, &v, Family::Speaker),
        (None, Some("Checking…".into()), None)
    );
    v.update(cx, |s, _| s.ui.models_loading = false);
    assert_eq!(
        no_status(cx, &v, Family::Speaker),
        (None, None, Some("None.".into()))
    );
}

/// A second speaker preset (another provider) and one saved person.
fn two_presets(v: &Entity<Settings>, cx: &mut TestAppContext, recognize: bool) {
    v.update(cx, |s, cx| {
        let providers = s.data.speaker_models.as_mut().unwrap()["providers"]
            .as_array_mut()
            .unwrap();
        let mut other = providers[0].clone();
        other["provider"] = json!("other");
        other["models"][0]["provider"] = json!("other");
        other["models"][0]["modelId"] = json!("fast-preset");
        other["models"][0]["displayName"] = json!("Fast preset");
        providers.push(other);
        s.data.person_count = 1;
        s.edit(
            |d| d.speaker_analysis.recognize_saved_people = recognize,
            cx,
        );
    });
}

fn preset(cx: &mut TestAppContext, v: &Entity<Settings>) -> (String, Option<String>) {
    v.read_with(cx, |s, _| {
        let a = s.d().speaker_analysis;
        (a.provider, a.model_id)
    })
}

#[gpui_kit::test]
fn leaving_the_saved_preset_with_saved_people_asks_first(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "speakers");
    let (host, _dir) = confirm_core(cx, false);
    two_presets(&v, cx, true);
    choose(cx, w, "spk-preset", "Fast");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Switch speaker model?".to_string()]
    );
    assert_eq!(
        *host.cancels.lock().unwrap(),
        vec!["Keep current".to_string()]
    );
    assert_eq!(
        preset(cx, &v).1.as_deref(),
        Some("pyannote-community-1-wespeaker"),
        "kept"
    );
}

#[gpui_kit::test]
fn a_confirmed_or_unwarned_preset_switch_takes_provider_and_model(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "speakers");
    let (host, _dir) = confirm_core(cx, true);
    two_presets(&v, cx, true);
    choose(cx, w, "spk-preset", "Fast");
    assert_eq!(preset(cx, &v), ("other".into(), Some("fast-preset".into())));
    // Recognition off: nothing to re-tag, so no question.
    let (w, v, _) = open(cx, "speakers");
    two_presets(&v, cx, false);
    host.asked.lock().unwrap().clear();
    choose(cx, w, "spk-preset", "Fast");
    assert!(host.asked.lock().unwrap().is_empty());
    assert_eq!(preset(cx, &v), ("other".into(), Some("fast-preset".into())));
}

#[gpui_kit::test]
fn an_installed_speaker_model_deletes_after_asking_and_says_how_it_went(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "speakers");
    assert!(!present(cx, w, "spk-delete"), "nothing to delete yet");
    let (host, _dir) = confirm_core(cx, true);
    v.update(cx, |s, cx| {
        let m = &mut s.data.speaker_models.as_mut().unwrap()["providers"][0]["models"][0];
        m["available"] = json!(true);
        m["status"] = json!("installed");
        cx.notify();
    });
    click(cx, w, "spk-delete");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Delete speaker model".to_string()]
    );
    // The bare test core has no app infrastructure: the failure lands on the row.
    let note = v.read_with(cx, |s, _| s.ui.model_notes.get(&Family::Speaker).cloned());
    assert_eq!(
        note,
        Some((
            false,
            "Speaker model action failed: app infrastructure isn't ready".into()
        ))
    );
}

#[gpui_kit::test]
fn an_unused_search_model_deletes_only_after_asking(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "semantic-search");
    let (host, _dir) = confirm_core(cx, false);
    v.update(cx, |s, cx| {
        let models = s.data.semantic_models.as_mut().unwrap()["models"]
            .as_array_mut()
            .unwrap();
        let bge = models
            .iter_mut()
            .find(|m| m["modelId"] == "bge-m3")
            .unwrap();
        bge["available"] = json!(true);
        s.ui.semantic_pick = Some("bge-m3".into());
        cx.notify();
    });
    assert!(present(cx, w, "ss-delete"));
    click(cx, w, "ss-delete");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Delete semantic search model".to_string()]
    );
    assert!(
        v.read_with(cx, |s, _| s.ui.busy.is_empty()),
        "a No deletes nothing"
    );
}

#[gpui_kit::test]
fn the_voiceprint_delete_offers_keep(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "speakers");
    let (host, _dir) = confirm_core(cx, false);
    v.update(cx, |s, cx| {
        s.data.voice_owner_id = Some(7);
        s.data.voice_owner = Some("Ada".into());
        cx.notify();
    });
    click(cx, w, "voice-delete");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Delete voiceprint".to_string()]
    );
    assert_eq!(*host.cancels.lock().unwrap(), vec!["Keep".to_string()]);
}

#[gpui_kit::test]
fn a_download_that_will_not_start_says_why_on_its_row(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "transcription");
    let (_host, _dir) = confirm_core(cx, true);
    cx.update(|cx| {
        let core = crate::chat::core(cx).unwrap();
        crate::bridge::tokio::init_from_handle(cx, core.runtime_handle().clone());
    });
    click(cx, w, "tx-model-row-download");
    let note = v.read_with(cx, |s, _| {
        s.ui.model_notes.get(&Family::Transcription).cloned()
    });
    assert_eq!(
        note,
        Some((
            false,
            "Download failed: app infrastructure isn't ready".into()
        )),
        "on the row, not a page note"
    );
    assert!(
        v.read_with(cx, |s, _| s.ui.busy.is_empty()),
        "Starting ends"
    );
}

#[gpui_kit::test]
fn the_deepgram_field_says_a_key_is_saved(cx: &mut TestAppContext) {
    // The kit's input keeps its placeholder private; the synced flag is what
    // `sync_key_placeholder` last wrote into it.
    let (w, v, _) = open(cx, "transcription");
    let shown = |cx: &mut TestAppContext| v.read_with(cx, |s, _| s.ui.deepgram_placeholder);
    assert_eq!(shown(cx), Some(false), "\"Paste your Deepgram API key\"");
    v.update(cx, |s, cx| {
        s.data.deepgram_key = true;
        cx.notify();
    });
    crate::settings::tests::frame(cx, w);
    assert_eq!(
        shown(cx),
        Some(true),
        "\"A key is saved — enter a new one to replace it\""
    );
}
