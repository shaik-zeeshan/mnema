//! Capture & Storage's "Never recorded" row and Change settings › Engines,
//! driven on the real view: a typed app that isn't installed becomes a rule
//! that waits for it, and the engine choice walks back to the recommended one.

use std::sync::Arc;

use capture_types::{AppearanceSetting, AudioTranscriptionProvider};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, TestAppContext, VisualTestContext, WindowAppearance};

use super::data::Backend;
use super::fixture::FixtureBackend;
use super::flow::Step;
use super::screens::capture_apps::excluded_note;
use super::Onboarding;
use crate::theme;

fn open(cx: &mut TestAppContext) -> (Entity<Onboarding>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
    });
    let b: Arc<dyn Backend> = Arc::new(FixtureBackend::load("granted").unwrap());
    let (view, vcx) = cx.add_window_view(move |_, cx| Onboarding::new(b.clone(), |_, _| {}, cx));
    vcx.run_until_parked();
    (view, vcx)
}

#[gpui_kit::test]
fn a_typed_app_that_isnt_installed_waits_for_it(cx: &mut TestAppContext) {
    let (view, vcx) = open(cx);
    view.update(vcx, |o, cx| o.go(Step::Capture, cx));
    vcx.run_until_parked();
    view.update_in(vcx, |o, window, cx| o.open_app_pick(window, cx));
    vcx.run_until_parked();
    vcx.simulate_input("Zzz Notes");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    view.read_with(vcx, |o, _| {
        let rule = o
            .draft
            .privacy
            .excluded_apps
            .iter()
            .find(|a| a.display_name == "Zzz Notes")
            .expect("the typed name is kept as a rule");
        assert!(rule.enabled && rule.bundle_id.is_empty());
        let (text, hint) = excluded_note(&o.draft.privacy.excluded_apps);
        assert!(
            text.contains("isn\u{2019}t installed yet \u{2014} the rule waits."),
            "{text}"
        );
        assert_eq!(hint, "Click a name to record it after all.");
    });
    // The note is drawn under the apps.
    vcx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("ob-apps-note").is_some());
    });
}

#[test]
fn the_note_says_nothing_is_hidden_until_a_rule_exists() {
    let (text, hint) = excluded_note(&[]);
    assert_eq!(
        text,
        "Nothing is hidden. Everything on your screen is recorded."
    );
    assert_eq!(hint, "Add an app it should never see.");
}

#[gpui_kit::test]
fn use_recommended_walks_the_engine_back_to_whisper(cx: &mut TestAppContext) {
    let (view, vcx) = open(cx);
    view.update(vcx, |o, cx| {
        o.go(Step::Change, cx);
        o.tab = 1;
        cx.notify();
    });
    vcx.run_until_parked();
    let parakeet = view.read_with(vcx, |o, _| {
        o.engines.iter().position(|e| e.provider == "parakeet")
    });
    let Some(parakeet) = parakeet else {
        panic!("the fixture lists Parakeet");
    };
    view.update(vcx, |o, cx| o.set_engine(parakeet, cx));
    vcx.run_until_parked();
    // Parakeet starts on its int8 build, as Svelte's default, not the
    // full-precision one the status happens to list first.
    view.read_with(vcx, |o, _| {
        assert_eq!(
            o.draft.transcription.model_id.as_deref(),
            Some("parakeet-tdt-0.6b-v3-onnx-int8")
        );
    });
    vcx.update(|window, cx| {
        window.render_frame(cx);
        window.click("ob-engine-recommended", cx);
    });
    vcx.run_until_parked();
    view.read_with(vcx, |o, _| {
        assert_eq!(
            o.draft.transcription.provider,
            AudioTranscriptionProvider::LocalWhisper
        );
    });
    vcx.update(|window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("ob-engine-recommended").is_none(),
            "no walk-back on the recommended engine"
        );
        window.click("ob-engine-cloud", cx);
    });
    assert!(
        view.read_with(vcx, |o, _| o.cloud_open),
        "the cloud note opens"
    );
}

#[gpui_kit::test]
fn never_recorded_asks_once_for_the_listed_apps_icons(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
    });
    let mut map: serde_json::Value = serde_json::from_str(include_str!(
        "../../../desktop/parity/fixtures/onboarding/granted.json"
    ))
    .unwrap();
    map["resolve_app_icons"] = serde_json::json!([
        { "bundleId": "com.1password.1password", "iconPath": "/icons/1password.png" },
        { "bundleId": "com.apple.keychainaccess", "iconPath": null }
    ]);
    let backend = Arc::new(FixtureBackend::new(map));
    let b: Arc<dyn Backend> = backend.clone();
    let (view, vcx) = cx.add_window_view(move |_, cx| Onboarding::new(b.clone(), |_, _| {}, cx));
    vcx.run_until_parked();
    view.update(vcx, |o, cx| o.go(Step::Capture, cx));
    vcx.run_until_parked();
    // Two more frames: the icons are asked for once, not per render.
    for _ in 0..2 {
        vcx.update(|window, cx| window.render_frame(cx));
        vcx.run_until_parked();
    }
    view.read_with(vcx, |o, _| {
        let icons = &o.app_pick.icons;
        assert_eq!(
            icons
                .get("com.1password.1password")
                .map(|p| p.to_string_lossy().into_owned()),
            Some("/icons/1password.png".into()),
            "an installed app gets its icon"
        );
        assert!(
            !icons.contains_key("com.apple.keychainaccess"),
            "no icon: the letter tile stays"
        );
    });
    let asks = backend.calls().iter().filter(|c| *c == "app_icons").count();
    assert_eq!(asks, 1);
}
