//! Floating layers in Settings: the Add provider menu opens and closes like
//! `Providers.svelte` (trigger toggles, outside click and a pick close it), and
//! the shared Select's trigger closes an open list instead of reopening it.

use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ElementId, Entity, TestAppContext, WindowHandle};

use crate::settings::tests::{frame, open_at};
use crate::settings::Settings;
use crate::ui::nid;

/// Tall enough that a whole group's column is on screen (clicks need hit tests).
const TALL: (f32, f32) = (1280., 4000.);

fn open(cx: &mut TestAppContext, tab: &str) -> (WindowHandle<Root>, Entity<Settings>) {
    let (w, v, _) = open_at(cx, false, TALL);
    v.update(cx, |s, cx| s.open_tab(tab, cx));
    frame(cx, w);
    (w, v)
}

fn click(cx: &mut TestAppContext, w: WindowHandle<Root>, id: &'static str) {
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn present(cx: &mut TestAppContext, w: WindowHandle<Root>, id: impl Into<ElementId>) -> bool {
    let id = id.into();
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn add_provider_menu_toggles_closes_outside_and_on_a_pick(cx: &mut TestAppContext) {
    let (w, v) = open(cx, "intelligence");
    click(cx, w, "pv-add");
    assert!(present(cx, w, "pv-add-menu"), "the trigger opens it");
    click(cx, w, "pv-add");
    assert!(
        !present(cx, w, "pv-add-menu"),
        "the trigger closes it again"
    );

    click(cx, w, "pv-add");
    click(cx, w, "ai-enabled");
    assert!(!present(cx, w, "pv-add-menu"), "a click outside closes it");

    click(cx, w, "pv-add");
    let before = v.read_with(cx, |s, _| s.d().ai_runtime.providers.len());
    click(cx, w, "pv-kind-Llamafile");
    assert!(!present(cx, w, "pv-add-menu"), "a pick closes it");
    let after = v.read_with(cx, |s, _| s.d().ai_runtime.providers.len());
    assert_eq!(after, before + 1, "the pick adds the provider");
}

#[gpui_kit::test]
fn select_trigger_closes_its_open_list(cx: &mut TestAppContext) {
    let (w, _) = open(cx, "capture");
    click(cx, w, "idle-timeout");
    assert!(
        present(cx, w, nid("idle-timeout-listbox", 0)),
        "the trigger opens it"
    );
    click(cx, w, "idle-timeout");
    assert!(
        !present(cx, w, nid("idle-timeout-listbox", 0)),
        "the trigger closes it"
    );
}

#[gpui_kit::test]
fn combobox_and_model_picker_triggers_close_their_open_lists(cx: &mut TestAppContext) {
    let (w, v) = open(cx, "ocr");
    // Apple Vision has no language list; Tesseract shows the combobox.
    v.update(cx, |s, cx| {
        s.edit(
            |d| d.ocr.provider = capture_types::OcrProvider::Tesseract,
            cx,
        )
    });
    frame(cx, w);
    click(cx, w, "ocr-language");
    assert!(present(cx, w, nid("ocr-language-listbox", 0)), "opens");
    click(cx, w, "ocr-language");
    assert!(!present(cx, w, nid("ocr-language-listbox", 0)), "closes");

    let (w, _) = open(cx, "intelligence");
    click(cx, w, "default-model");
    assert!(present(cx, w, "default-model-pop"), "opens");
    click(cx, w, "default-model");
    assert!(!present(cx, w, "default-model-pop"), "closes");
}
