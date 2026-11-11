//! Settings › Intelligence pickers through the fixture backend: the two
//! language comboboxes, the semantic search catalog + rebuild confirm, and the
//! Settings voice take (timing on both sides of each boundary, and the
//! microphone released on every exit path, against the fake recorder).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use app_core::{
    AppCore, ConfirmDialog, CoreEvent, ExitKind, Host, MessageDialog, Paths, WindowRequest,
    WindowSignal,
};
use capture_types::{AudioTranscriptionProvider, OcrProvider};
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle};
use serde_json::json;

use super::{languages, semantic_search};
use crate::settings::autosave::{Unit, RECORDING_DEBOUNCE};
use crate::settings::fixture::{settings_data, Call, FixtureBackend};
use crate::settings::groups::SectionId;
use crate::settings::tests::{frame, open_at};
use crate::settings::{domain_value, Settings};
use crate::ui::nid;
use crate::voice::{self, Enrollment};

pub(super) const MS: Duration = Duration::from_millis(1);
/// Tall enough that a whole group's column is on screen (clicks need hit tests).
pub(super) const TALL: (f32, f32) = (1280., 4000.);

pub(super) fn advance(cx: &mut TestAppContext, d: Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

pub(super) fn on_window<R>(
    cx: &mut TestAppContext,
    w: WindowHandle<Root>,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    let r = cx
        .update_window(w.into(), |_, window, cx| {
            window.render_frame(cx);
            f(window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    r
}

pub(super) fn click(
    cx: &mut TestAppContext,
    w: WindowHandle<Root>,
    id: impl Into<gpui_kit::ElementId>,
) {
    let id = id.into();
    on_window(cx, w, |window, cx| window.click(id, cx));
}

pub(super) fn present(cx: &mut TestAppContext, w: WindowHandle<Root>, id: &'static str) -> bool {
    on_window(cx, w, |window, _| window.try_find(id).is_some())
}

pub(super) fn label(cx: &mut TestAppContext, w: WindowHandle<Root>, id: &'static str) -> String {
    on_window(cx, w, |window, _| {
        window.find(id).label().unwrap_or_default().to_string()
    })
}

pub(super) fn open(
    cx: &mut TestAppContext,
    tab: &str,
) -> (WindowHandle<Root>, Entity<Settings>, Arc<FixtureBackend>) {
    let (w, v, b) = open_at(cx, false, TALL);
    v.update(cx, |s, cx| s.open_tab(tab, cx));
    frame(cx, w);
    (w, v, b)
}

/// Open combobox `id`, filter it to `query`, and click the one option left
/// (the option rows aren't observable; the last row sits at the list's foot).
pub(super) fn choose(
    cx: &mut TestAppContext,
    w: WindowHandle<Root>,
    id: &'static str,
    query: &str,
) {
    click(cx, w, id);
    on_window(cx, w, |window, cx| window.input(query, cx));
    let list = nid(&format!("{id}-listbox"), 0);
    let h = on_window(cx, w, |window, _| {
        window.find(list.clone()).bounds().size.height
    });
    on_window(cx, w, |window, cx| {
        window.click_at(
            list,
            gpui_kit::point(gpui_kit::px(24.), h - gpui_kit::px(14.)),
            cx,
        )
    });
}

fn expanded(cx: &mut TestAppContext, w: WindowHandle<Root>, id: &'static str) -> bool {
    on_window(cx, w, |window, _| window.find(id).expanded() == Some(true))
}

pub(super) fn count(b: &FixtureBackend, c: &Call) -> usize {
    b.calls().iter().filter(|x| *x == c).count()
}

// ── Languages ──────────────────────────────────────────────────────────────

#[test]
fn the_language_lists_match_svelte() {
    assert_eq!(languages::TRANSCRIPTION.len(), 65);
    assert_eq!(languages::TRANSCRIPTION[0], ("auto", "Auto-detect"));
    assert_eq!(languages::OCR.len(), 9);
    assert_eq!(languages::OCR[0], ("eng", "English (eng)"));
    assert_eq!(
        languages::pick(&languages::OCR, "deu"),
        (Some(2), String::new())
    );
    assert_eq!(
        languages::pick(&languages::OCR, "eng+deu"),
        (None, "eng+deu".to_string()),
        "a code outside the list selects nothing and shows as stored"
    );
}

#[gpui_kit::test]
fn the_transcription_language_saves_the_picked_code_for_every_provider(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "transcription");
    choose(cx, w, "tx-language", "French");
    assert_eq!(v.read_with(cx, |s, _| s.d().transcription.language), "fr");
    advance(cx, RECORDING_DEBOUNCE - MS);
    assert_eq!(
        count(&b, &Call::Domain(Unit::Processing)),
        0,
        "449 ms: not yet"
    );
    advance(cx, MS);
    assert_eq!(count(&b, &Call::Domain(Unit::Processing)), 1);
    let saved = v.read_with(cx, |s, _| {
        domain_value(Unit::Processing, s.saved.as_ref().unwrap())
    });
    assert_eq!(
        saved[1]["language"],
        json!("fr"),
        "the payload carries the code"
    );
    // One list for every provider; a provider switch keeps the stored language.
    for provider in [
        AudioTranscriptionProvider::LocalWhisper,
        AudioTranscriptionProvider::AppleSpeechOnDevice,
        AudioTranscriptionProvider::Deepgram,
        AudioTranscriptionProvider::Parakeet,
    ] {
        v.update(cx, |s, cx| {
            s.edit(|d| d.transcription.provider = provider, cx)
        });
        assert!(present(cx, w, "tx-language"), "{provider:?} keeps the pick");
        assert_eq!(v.read_with(cx, |s, _| s.d().transcription.language), "fr");
    }
}

#[gpui_kit::test]
fn the_ocr_language_is_tesseract_only_and_saves_the_code(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "ocr");
    assert!(
        !present(cx, w, "ocr-language"),
        "Apple Vision has no language pick"
    );
    v.update(cx, |s, cx| {
        s.edit(|d| d.ocr.provider = OcrProvider::PaddleOcr, cx)
    });
    assert!(
        !present(cx, w, "ocr-language"),
        "nor does PaddleOCR (Svelte: tesseract only)"
    );
    v.update(cx, |s, cx| {
        s.edit(|d| d.ocr.provider = OcrProvider::Tesseract, cx)
    });
    assert!(present(cx, w, "ocr-language"));
    choose(cx, w, "ocr-language", "chi_sim");
    assert_eq!(
        v.read_with(cx, |s, _| s.d().ocr.language),
        Some("chi_sim".into())
    );
    advance(cx, RECORDING_DEBOUNCE);
    let saved = v.read_with(cx, |s, _| {
        domain_value(Unit::Processing, s.saved.as_ref().unwrap())
    });
    assert_eq!(saved[0]["language"], json!("chi_sim"));
    assert!(count(&b, &Call::Domain(Unit::Processing)) >= 1);
    // OCR off disables the pick (Svelte `disabled={!rec.draftOcrEnabled}`).
    v.update(cx, |s, cx| s.edit(|d| d.ocr.enabled = false, cx));
    let disabled = on_window(cx, w, |window, _| window.find("ocr-language").disabled());
    assert_ne!(disabled, Some(false));
    click(cx, w, "ocr-language");
    assert!(
        !expanded(cx, w, "ocr-language"),
        "a disabled pick doesn't open"
    );
}

// ── Semantic search ────────────────────────────────────────────────────────

#[test]
fn the_catalog_follows_the_recommended_tiers_and_a_catalog_pick_stands_alone() {
    let d = settings_data().unwrap();
    let status = d.semantic_models.as_ref();
    let map = crate::shell::fixture::load("settings", "default").unwrap();
    let catalog = map["list_semantic_search_supported_models"]
        .as_array()
        .unwrap()
        .clone();
    let opts = semantic_search::options(status, &catalog);
    assert_eq!(opts.len(), 8, "2 recommended + 6 from the catalog");
    assert!(opts[0].1.ends_with("recommended") && opts[1].1.contains("multilingual"));
    assert_eq!(opts[2].0, "bge-m3");
    assert_eq!(
        opts[2].1,
        "BGE-M3 (Multilingual, Custom) \u{2014} 1024d \u{b7} multilingual"
    );
    assert_eq!(
        semantic_search::options(status, &[]).len(),
        2,
        "no catalog: the tiers only"
    );
    // A catalog model the status doesn't list: Custom, not installed, the status's provider.
    let mut only_tiers = status.unwrap().clone();
    only_tiers["models"].as_array_mut().unwrap().truncate(2);
    let p = semantic_search::picked(Some(&only_tiers), &catalog, "bge-m3").unwrap();
    assert_eq!(
        p.meta,
        "Custom \u{b7} 1024-dim \u{b7} runs on-device \u{b7} multilingual"
    );
    assert!(!p.available);
    assert_eq!(p.provider.as_deref(), Some("local"));
}

#[gpui_kit::test]
fn the_picker_lists_the_catalog_from_the_backend(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "semantic-search");
    // Granite Small is custom-tier: only the backend's catalog lists it.
    choose(cx, w, "ss-model", "Granite Embedding Small");
    let pick = v.read_with(cx, |s, _| s.ui.semantic_pick.clone());
    assert_eq!(pick.as_deref(), Some("granite-embedding-small-english-r2"));
}

/// Answers every confirm with `answer` and records the titles asked.
pub(super) struct ConfirmHost {
    answer: bool,
    pub(super) asked: Mutex<Vec<String>>,
    /// Each dialog's cancel label, in order.
    pub(super) cancels: Mutex<Vec<String>>,
}

impl Host for ConfirmHost {
    fn emit(&self, _: CoreEvent) {}
    fn message(&self, _: MessageDialog) {}
    fn confirm(&self, d: ConfirmDialog) -> tokio::sync::oneshot::Receiver<bool> {
        self.asked.lock().unwrap().push(d.title.unwrap_or_default());
        self.cancels.lock().unwrap().push(d.cancel_label);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = tx.send(self.answer);
        rx
    }
    fn open_url(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn refresh_tray(&self) {}
    fn open_window(&self, _: WindowRequest) -> Result<(), String> {
        Ok(())
    }
    fn window_signal(&self, _: WindowSignal) {}
    fn exit(&self, _: ExitKind) {}
}

/// A bare core whose only job is `Host::confirm` (no states, no data on disk).
pub(super) fn confirm_core(
    cx: &mut TestAppContext,
    answer: bool,
) -> (Arc<ConfirmHost>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(ConfirmHost {
        answer,
        asked: Mutex::default(),
        cancels: Mutex::default(),
    });
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        cache: dir.path().join("cache"),
        log: dir.path().join("log"),
    };
    let core = Arc::new(AppCore::new(host.clone(), paths).unwrap());
    cx.update(|cx| cx.set_global(crate::bridge::events::Core(core)));
    (host, dir)
}

/// bge-m3 installed and picked, nomic the active model (the fixture's).
fn pick_installed(v: &Entity<Settings>, cx: &mut TestAppContext) {
    v.update(cx, |s, cx| {
        let models = s.data.semantic_models.as_mut().unwrap()["models"]
            .as_array_mut()
            .unwrap();
        let bge = models
            .iter_mut()
            .find(|m| m["modelId"] == "bge-m3")
            .unwrap();
        bge["available"] = json!(true);
        bge["status"] = json!("installed");
        s.ui.semantic_pick = Some("bge-m3".into());
        cx.notify();
    });
}

fn use_model(cx: &mut TestAppContext, w: WindowHandle<Root>) {
    click(cx, w, "ss-use");
}

#[gpui_kit::test]
fn switching_models_asks_first_and_a_no_changes_nothing(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "semantic-search");
    let (host, _dir) = confirm_core(cx, false);
    pick_installed(&v, cx);
    use_model(cx, w);
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Re-index for new search model?".to_string()]
    );
    assert_eq!(count(&b, &Call::SelectSemantic("bge-m3".into())), 0);
    let active = v.read_with(cx, |s, _| s.d().semantic_search.model_id);
    assert_eq!(active.as_deref(), Some("nomic-embed-text-v1.5"));
}

#[gpui_kit::test]
fn a_confirmed_switch_rebuilds_and_says_so(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "semantic-search");
    let (host, _dir) = confirm_core(cx, true);
    pick_installed(&v, cx);
    use_model(cx, w);
    assert_eq!(host.asked.lock().unwrap().len(), 1);
    assert_eq!(count(&b, &Call::SelectSemantic("bge-m3".into())), 1);
    v.read_with(cx, |s, _| {
        assert_eq!(s.d().semantic_search.model_id.as_deref(), Some("bge-m3"));
        assert_eq!(
            s.saved
                .as_ref()
                .unwrap()
                .semantic_search
                .model_id
                .as_deref(),
            Some("bge-m3")
        );
        assert_eq!(
            s.ui.reindex.as_deref(),
            Some("Cleared 42 vectors; re-indexing in the background."),
            "the row keeps the re-index line (not a page note)"
        );
    });
    assert!(
        !present(cx, w, "ss-use"),
        "the active model has nothing to switch to"
    );
    assert!(
        present(cx, w, "ss-reindexing"),
        "the re-indexing progress shows"
    );
    assert_eq!(
        *host.cancels.lock().unwrap(),
        vec!["Keep Current Model".to_string()]
    );
}

#[gpui_kit::test]
fn the_first_pick_switches_without_asking(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "semantic-search");
    let (host, _dir) = confirm_core(cx, false);
    v.update(cx, |s, _| {
        for d in [s.draft.as_mut(), s.saved.as_mut()].into_iter().flatten() {
            d.semantic_search.model_id = None;
        }
    });
    pick_installed(&v, cx);
    use_model(cx, w);
    assert!(
        host.asked.lock().unwrap().is_empty(),
        "nothing to rebuild yet"
    );
    assert_eq!(count(&b, &Call::SelectSemantic("bge-m3".into())), 1);
    assert_eq!(v.read_with(cx, |s, _| s.ui.note.clone()), None);
}

// ── Voice take ─────────────────────────────────────────────────────────────

fn record(cx: &mut TestAppContext, w: WindowHandle<Root>, b: &FixtureBackend) {
    assert_eq!(
        count(b, &Call::RecordClip(15_000)),
        0,
        "the mic opens only for a take"
    );
    click(cx, w, "voice-record");
    assert_eq!(count(b, &Call::RecordClip(15_000)), 1);
}

#[gpui_kit::test]
fn the_take_counts_down_for_fifteen_seconds_then_waits_for_review(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "speakers");
    record(cx, w, &b);
    assert_eq!(label(cx, w, "voice-meter"), "Recording, 15s left");
    advance(cx, Duration::from_millis(1_050));
    assert_eq!(label(cx, w, "voice-meter"), "Recording, 14s left");
    advance(cx, voice::CLIP - Duration::from_millis(1_050) - MS);
    assert!(present(cx, w, "voice-meter"), "14 999 ms: still recording");
    assert!(!present(cx, w, "voice-use"));
    advance(cx, MS);
    assert!(
        !present(cx, w, "voice-meter"),
        "15 000 ms: the take is done"
    );
    assert!(present(cx, w, "voice-use") && present(cx, w, "voice-discard"));
    // Listen back: the fake player plays, then Stop.
    click(cx, w, "voice-play");
    assert!(b.player.state().1, "playing");
    click(cx, w, "voice-play");
    assert!(!b.player.state().1);
    // Use this take: enrolled, and the draft mirrors recognize_saved_people.
    v.update(cx, |s, cx| {
        s.edit(|d| d.speaker_analysis.recognize_saved_people = false, cx)
    });
    advance(cx, RECORDING_DEBOUNCE);
    click(cx, w, "voice-use");
    assert_eq!(count(&b, &Call::Enroll), 1);
    assert!(v.read_with(cx, |s, _| s.d().speaker_analysis.recognize_saved_people));
    let before = count(&b, &Call::Domain(Unit::Processing));
    advance(cx, RECORDING_DEBOUNCE);
    assert_eq!(count(&b, &Call::Domain(Unit::Processing)), before + 1);
    let saved = v.read_with(cx, |s, _| {
        domain_value(Unit::Processing, s.saved.as_ref().unwrap())
    });
    assert_eq!(saved[2]["recognizeSavedPeople"], json!(true));
    assert_eq!(
        count(&b, &Call::CancelClip),
        0,
        "the recorder released the mic itself"
    );
    // Leaving after a finished take has nothing to release.
    v.update(cx, |s, cx| s.navigate(SectionId::Appearance, cx));
    frame(cx, w);
    assert_eq!(count(&b, &Call::CancelClip), 0);
}

#[gpui_kit::test]
fn a_rejected_take_says_why_and_discard_drops_the_review(cx: &mut TestAppContext) {
    let (w, _, b) = open(cx, "speakers");
    *b.enrollment.lock().unwrap() = Some(Enrollment::NoSpeech);
    record(cx, w, &b);
    advance(cx, voice::CLIP);
    click(cx, w, "voice-discard");
    assert!(present(cx, w, "voice-record") && !present(cx, w, "voice-use"));
    assert_eq!(
        count(&b, &Call::Enroll),
        0,
        "a discarded take is never enrolled"
    );
    click(cx, w, "voice-record");
    advance(cx, voice::CLIP);
    click(cx, w, "voice-use");
    assert_eq!(count(&b, &Call::Enroll), 1);
    assert!(present(cx, w, "voice-record"), "a rejection can be retaken");
    assert_eq!(count(&b, &Call::CancelClip), 0);
}

#[gpui_kit::test]
fn cancel_releases_the_mic_and_ends_the_take(cx: &mut TestAppContext) {
    let (w, _, b) = open(cx, "speakers");
    record(cx, w, &b);
    advance(cx, Duration::from_secs(5));
    click(cx, w, "voice-cancel");
    assert_eq!(count(&b, &Call::CancelClip), 1);
    assert!(!present(cx, w, "voice-meter") && !present(cx, w, "voice-cancel"));
    advance(cx, voice::CLIP);
    assert!(
        !present(cx, w, "voice-use"),
        "a cancelled take never reaches review"
    );
    assert_eq!(count(&b, &Call::RecordClip(15_000)), 1);
    assert_eq!(count(&b, &Call::CancelClip), 1);
}

#[gpui_kit::test]
fn leaving_the_section_releases_the_mic(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "speakers");
    record(cx, w, &b);
    advance(cx, Duration::from_secs(1));
    v.update(cx, |s, cx| s.navigate(SectionId::Appearance, cx));
    frame(cx, w);
    frame(cx, w);
    assert_eq!(count(&b, &Call::CancelClip), 1);
    advance(cx, voice::CLIP);
    assert_eq!(count(&b, &Call::CancelClip), 1);
    // Back again: a fresh take, the mic still closed until Record.
    v.update(cx, |s, cx| s.navigate(SectionId::Speakers, cx));
    frame(cx, w);
    assert!(present(cx, w, "voice-record") && !present(cx, w, "voice-meter"));
    assert_eq!(count(&b, &Call::RecordClip(15_000)), 1);
}

#[gpui_kit::test]
fn closing_settings_releases_the_mic(cx: &mut TestAppContext) {
    let (w, _, b) = open(cx, "speakers");
    record(cx, w, &b);
    advance(cx, Duration::from_secs(1));
    cx.update_window(w.into(), |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert_eq!(count(&b, &Call::CancelClip), 1);
}

#[gpui_kit::test]
fn a_take_cancelled_elsewhere_goes_back_to_idle(cx: &mut TestAppContext) {
    // A quit cancels in app-core; the recorder answers "cancelled", not an error.
    let (w, _, b) = open(cx, "speakers");
    *b.clip_cancelled.lock().unwrap() = true;
    record(cx, w, &b);
    advance(cx, voice::CLIP);
    assert!(present(cx, w, "voice-record"), "idle again");
    assert!(!present(cx, w, "voice-use") && !present(cx, w, "voice-meter"));
    assert_eq!(count(&b, &Call::Enroll), 0);
    assert_eq!(count(&b, &Call::CancelClip), 0, "nothing left to release");
}

#[gpui_kit::test]
fn apple_speech_shows_its_permission_notice_with_the_right_action(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "transcription");
    let with_status = |cx: &mut TestAppContext, status: &'static str| {
        v.update(cx, |s, cx| {
            s.edit(
                |d| d.transcription.provider = AudioTranscriptionProvider::AppleSpeechOnDevice,
                cx,
            );
            fn set(v: &mut serde_json::Value, status: &str) {
                match v {
                    serde_json::Value::Object(m) => {
                        if m.contains_key("availabilityStatus") {
                            m.insert("availabilityStatus".into(), json!(status));
                            m.insert("available".into(), json!(status == "available"));
                        }
                        m.values_mut().for_each(|x| set(x, status));
                    }
                    serde_json::Value::Array(a) => a.iter_mut().for_each(|x| set(x, status)),
                    _ => {}
                }
            }
            if let Some(m) = s.data.transcription_models.as_mut() {
                set(m, status);
            }
            cx.notify();
        });
        cx.run_until_parked();
        (
            present(cx, w, "speech-allow"),
            present(cx, w, "speech-settings"),
        )
    };
    assert_eq!(
        with_status(cx, "permission_not_determined"),
        (true, false),
        "Get permission"
    );
    assert_eq!(
        with_status(cx, "permission_denied"),
        (false, true),
        "Open System Settings"
    );
    assert_eq!(
        with_status(cx, "framework_unavailable"),
        (false, false),
        "nothing to ask"
    );
    assert_eq!(
        with_status(cx, "available"),
        (false, false),
        "granted: no prompt at all"
    );
}
