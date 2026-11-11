//! Settings outside Intelligence, driven by clicks and keys on the real view:
//! shortcut capture and conflicts, the save notices and their retry, the
//! system-audio hint, Video's validation and Original-only Macs, privacy, the
//! destructive Data / About actions (each asks first and reports on its own
//! row), and the licence's over-cap and activation flows.

use std::time::Duration;

use capture_types::{Activation, LicenseStatus, ScreenResolution, ScreenResolutionPreset};
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{TestAppContext, WindowHandle};
use serde_json::json;

use super::pickers_tests::{advance, click, confirm_core, on_window, open, present, MS};
use crate::settings::autosave::{SaveStatus, Unit, RECORDING_DEBOUNCE};
use crate::settings::data::Family;
use crate::settings::fixture::Call;
use crate::settings::tests::{frame, open_at};
use crate::ui::nid;

fn press(cx: &mut TestAppContext, w: WindowHandle<Root>, key: &str) {
    on_window(cx, w, |window, cx| window.press(key, cx));
}

fn saves(b: &crate::settings::fixture::FixtureBackend, c: Call) -> usize {
    b.calls().iter().filter(|x| **x == c).count()
}

#[gpui_kit::test]
fn a_conflicting_chord_marks_both_rows_and_holds_the_save(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "shortcuts");
    click(cx, w, "kb-openSettings");
    assert_eq!(
        v.read_with(cx, |s, _| s.ui.kb_listening),
        Some("openSettings")
    );
    press(cx, w, "cmd-d");
    let issues = v.read_with(cx, |s, _| s.shortcut_issues());
    assert_eq!(issues["openSettings"], "Conflicts with Open debug.");
    assert_eq!(issues["openDebug"], "Conflicts with Open settings.");
    assert!(
        present(cx, w, "kb-jump"),
        "Shortcuts not saved + Jump to conflict"
    );
    assert_eq!(v.read_with(cx, |s, _| s.save_status()), SaveStatus::Blocked);
    advance(cx, Duration::from_secs(2));
    assert_eq!(saves(&b, Call::Keyboard), 0, "a conflict never saves");
    // ⌫ while listening clears the chord: the conflict goes, the save runs.
    click(cx, w, "kb-openSettings");
    press(cx, w, "backspace");
    assert!(v.read_with(cx, |s, _| s.shortcut_issues().is_empty()));
    advance(cx, RECORDING_DEBOUNCE + MS);
    assert_eq!(saves(&b, Call::Keyboard), 1);
}

#[gpui_kit::test]
fn a_control_chord_is_refused_and_a_press_elsewhere_stops_listening(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "shortcuts");
    click(cx, w, "kb-openDebug");
    press(cx, w, "ctrl-k");
    let (listening, error) = v.read_with(cx, |s, _| {
        (s.ui.kb_listening, s.ui.kb_capture_error.clone())
    });
    assert_eq!(listening, Some("openDebug"), "still listening");
    assert_eq!(
        error,
        Some((
            "openDebug",
            "Control shortcuts are not supported on macOS. Use Command or Option.".into()
        ))
    );
    click(cx, w, "kb-restore");
    assert_eq!(v.read_with(cx, |s, _| s.ui.kb_listening), None);
    assert_eq!(v.read_with(cx, |s, _| s.ui.kb_capture_error.clone()), None);
    advance(cx, Duration::from_secs(2));
    assert_eq!(saves(&b, Call::Keyboard), 0, "nothing changed");
}

#[gpui_kit::test]
fn a_failed_recording_save_retries_on_its_own(cx: &mut TestAppContext) {
    let (w, v, b) = open_at(cx, true, super::pickers_tests::TALL);
    v.update(cx, |s, cx| {
        s.edit(|d| d.follow_timeline_live = !d.follow_timeline_live, cx)
    });
    advance(cx, RECORDING_DEBOUNCE + MS);
    assert_eq!(saves(&b, Call::Domain(Unit::Display)), 1);
    assert!(
        present(cx, w, "settings-notice-retry"),
        "recording failures offer Retry"
    );
    // It retries by itself, within the first 500 ms backoff…
    let mut waited = 0;
    while saves(&b, Call::Domain(Unit::Display)) == 1 && waited < 600 {
        advance(cx, Duration::from_millis(10));
        waited += 10;
    }
    assert_eq!(
        saves(&b, Call::Domain(Unit::Display)),
        2,
        "retried on its own"
    );
    // …and a second failure waits twice as long (1 s), not the 450 ms debounce.
    advance(cx, Duration::from_millis(900));
    assert_eq!(saves(&b, Call::Domain(Unit::Display)), 2);
    advance(cx, Duration::from_millis(200));
    assert_eq!(saves(&b, Call::Domain(Unit::Display)), 3);
    click(cx, w, "settings-notice-dismiss");
    let (failed, back) = v.read_with(cx, |s, _| {
        (
            s.failed.is_some(),
            s.d().follow_timeline_live == s.data.settings.as_ref().unwrap().follow_timeline_live,
        )
    });
    assert!(!failed && back, "Dismiss puts the saved value back");
}

#[gpui_kit::test]
fn a_failed_shortcut_save_says_shortcuts_and_offers_only_dismiss(cx: &mut TestAppContext) {
    let (w, v, _) = open_at(cx, true, super::pickers_tests::TALL);
    v.update(cx, |s, cx| s.open_tab("shortcuts", cx));
    v.update(cx, |s, cx| {
        s.edit_keyboard(|k| k["globalShortcuts"]["enabled"] = json!(false), cx)
    });
    advance(cx, RECORDING_DEBOUNCE + MS);
    assert_eq!(
        v.read_with(cx, |s, _| s.failed.as_ref().map(|f| f.0)),
        Some(Unit::Keyboard)
    );
    assert!(present(cx, w, "settings-notice-dismiss"));
    assert!(!present(cx, w, "settings-notice-retry"));
}

#[gpui_kit::test]
fn search_enter_jumps_to_the_first_match_and_esc_clears_it(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "appearance");
    click(cx, w, "settings-search");
    on_window(cx, w, |window, cx| window.input("retention", cx));
    press(cx, w, "enter");
    assert_eq!(
        v.read_with(cx, |s, _| s.section),
        crate::settings::groups::SectionId::Storage
    );
    assert!(
        v.update(cx, |s, cx| s.close_top(cx)),
        "Esc clears the search first"
    );
    frame(cx, w);
    assert_eq!(
        v.read_with(cx, |s, cx| s.inputs.search.read(cx).value().to_string()),
        ""
    );
    assert!(
        !v.update(cx, |s, cx| s.close_top(cx)),
        "then Esc means Back to app"
    );
}

#[gpui_kit::test]
fn the_system_audio_hint_lives_in_its_row_and_dismisses(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "capture");
    v.update(cx, |s, cx| {
        s.data.system_audio_blocked = true;
        s.edit(|d| d.capture_system_audio = true, cx);
    });
    assert!(present(cx, w, "sysaudio-dismiss"));
    v.update(cx, |s, cx| s.edit(|d| d.capture_system_audio = false, cx));
    assert!(
        !present(cx, w, "sysaudio-dismiss"),
        "only while the switch is on"
    );
    v.update(cx, |s, cx| s.edit(|d| d.capture_system_audio = true, cx));
    click(cx, w, "sysaudio-dismiss");
    assert!(!present(cx, w, "sysaudio-dismiss"));
    assert!(b.calls().contains(&Call::Prompt(
        crate::settings::privacy_prompt::Mark::Dismissed
    )));
}

#[gpui_kit::test]
fn a_custom_size_off_the_range_blocks_video_and_says_why(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "video");
    click(cx, w, nid("resolution-opt", 4));
    assert!(
        v.read_with(cx, |s, _| s.video_invalid),
        "both fields are required"
    );
    click(cx, w, "res-width");
    on_window(cx, w, |window, cx| window.input("5", cx));
    click(cx, w, "res-height");
    on_window(cx, w, |window, cx| window.input("1080", cx));
    assert!(v.read_with(cx, |s, _| s.video_invalid));
    assert_eq!(
        super::video::resolution_errors("5", "1080"),
        vec!["Width must be between 16 and 8192."]
    );
    advance(cx, Duration::from_secs(2));
    assert_eq!(saves(&b, Call::Domain(Unit::Video)), 0);
    click(cx, w, "res-width");
    on_window(cx, w, |window, cx| window.input("12", cx));
    assert!(
        !v.read_with(cx, |s, _| s.video_invalid),
        "512 × 1080 is fine"
    );
    advance(cx, RECORDING_DEBOUNCE + MS);
    assert_eq!(saves(&b, Call::Domain(Unit::Video)), 1);
}

#[gpui_kit::test]
fn a_mac_without_scaled_capture_goes_back_to_original(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "video");
    v.update(cx, |s, cx| {
        s.edit(
            |d| {
                d.capture_screen = true;
                d.screen_resolution = ScreenResolution::Preset {
                    preset: ScreenResolutionPreset::P1080,
                }
            },
            cx,
        );
        s.data.capture_support = Some(json!({
            "nativeCaptureSupported": true,
            "supportedSources": { "screen": true, "microphone": true, "systemAudio": false }
        }));
    });
    frame(cx, w);
    let res = v.read_with(cx, |s, _| s.d().screen_resolution);
    assert_eq!(
        res,
        ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original
        }
    );
    click(cx, w, nid("resolution-opt", 1));
    let res = v.read_with(cx, |s, _| s.d().screen_resolution);
    assert!(
        matches!(
            res,
            ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original
            }
        ),
        "1080p can't be picked"
    );
}

#[gpui_kit::test]
fn full_urls_ask_with_enable(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "privacy");
    let (host, _dir) = confirm_core(cx, false);
    click(cx, w, nid("url-mode-opt", 2));
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Enable full URL metadata".to_string()]
    );
    assert_eq!(*host.oks.lock().unwrap(), vec!["Enable".to_string()]);
    assert_ne!(
        v.read_with(cx, |s, _| s.d().metadata.browser_url_mode),
        capture_types::BrowserUrlMode::Full,
        "Cancel keeps it"
    );
}

#[gpui_kit::test]
fn run_cleanup_asks_and_reports_on_its_row(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "storage");
    let (host, _dir) = confirm_core(cx, false);
    click(cx, w, "retention-run");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Run cleanup now".to_string()]
    );
    assert_eq!(*host.oks.lock().unwrap(), vec!["Run cleanup".to_string()]);
    assert!(
        v.read_with(cx, |s, _| s.ui.cleanup.is_none()),
        "a No runs nothing"
    );
    let (_host, _dir2) = confirm_core(cx, true);
    click(cx, w, "retention-run");
    let (cleanup, note) = v.read_with(cx, |s, _| (s.ui.cleanup.clone(), s.ui.note.clone()));
    assert_eq!(cleanup, Some(Err("app infrastructure isn't ready".into())));
    assert!(note.is_none(), "on the row, not a page note");
    assert!(present(cx, w, "retention-retry"));
}

#[gpui_kit::test]
fn delete_recent_asks_and_leaves_a_receipt(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "storage");
    let (host, _dir) = confirm_core(cx, true);
    click(cx, w, nid("delete-window-opt", 0));
    click(cx, w, "delete-recent");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Delete Recent Capture".to_string()]
    );
    let receipt = v.read_with(cx, |s, _| s.ui.delete_receipt.clone());
    let (tone, words) = receipt.expect("a receipt");
    assert_eq!(tone, crate::ui::Tone::Danger);
    assert!(
        words.starts_with("Couldn\u{2019}t delete the last 1 minute: "),
        "{words}"
    );
    assert_eq!(
        super::delete_recent::receipt(300, 2).1,
        "Deleted the last 5 minutes, but 2 files couldn\u{2019}t be removed from disk yet \u{2014} Mnema will retry."
    );
}

#[gpui_kit::test]
fn a_new_save_folder_offers_the_restart(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "storage");
    assert!(!present(cx, w, "storage-restart"));
    v.update(cx, |s, cx| {
        s.ui.picked_location = Some("/Volumes/Big/mnema".into());
        cx.notify();
    });
    assert!(present(cx, w, "storage-restart"));
    let (host, _dir) = confirm_core(cx, false);
    click(cx, w, "storage-restart");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Restart Mnema".to_string()]
    );
    assert!(
        !v.read_with(cx, |s, _| s.ui.restarting),
        "Cancel restarts nothing"
    );
}

#[gpui_kit::test]
fn deleting_the_app_log_asks_first(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "developer");
    v.update(cx, |s, cx| {
        s.data.log_status = Some(json!({ "path": "/tmp/rust.log", "exists": true }));
        cx.notify();
    });
    let (host, _dir) = confirm_core(cx, false);
    click(cx, w, "log-delete");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Delete general application log".to_string()]
    );
    assert!(
        v.read_with(cx, |s, _| s.ui.busy.is_empty()),
        "a No deletes nothing"
    );
}

#[gpui_kit::test]
fn a_full_device_list_offers_free_up_not_a_bare_reset(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "license");
    v.update(cx, |s, cx| {
        s.data.license = Some(LicenseStatus::Licensed {
            update_through_ms: i64::MAX,
            in_window: true,
            email: String::new(),
            name: "Ada".into(),
            activation: Activation::RefusedOverCap {
                reset_url: "https://help".into(),
                buy_url: "https://buy".into(),
            },
        });
        cx.notify();
    });
    assert!(present(cx, w, "lic-free-up"));
    assert!(present(cx, w, "lic-buy-another") && present(cx, w, "lic-help"));
    assert!(!present(cx, w, "lic-reset"));
    let (host, _dir) = confirm_core(cx, false);
    click(cx, w, "lic-free-up");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Free up my devices".to_string()]
    );
}

#[gpui_kit::test]
fn a_refused_licence_key_stays_in_the_field(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "license");
    let (_host, _dir) = confirm_core(cx, true);
    click(cx, w, "lic-key");
    on_window(cx, w, |window, cx| window.input("MNEMA-BAD-KEY", cx));
    click(cx, w, "lic-activate");
    let (outcome, note) = v.read_with(cx, |s, _| (s.ui.lic_activate.clone(), s.ui.note.clone()));
    assert_eq!(outcome, Some(Err("app infrastructure isn't ready".into())));
    assert!(note.is_none(), "on the row, not a page note");
    frame(cx, w);
    let key = v.read_with(cx, |s, cx| {
        s.inputs.license_key.read(cx).value().to_string()
    });
    assert_eq!(key, "MNEMA-BAD-KEY");
    click(cx, w, "lic-key");
    on_window(cx, w, |window, cx| window.input("X", cx));
    assert_eq!(
        v.read_with(cx, |s, _| s.ui.lic_activate.clone()),
        None,
        "editing clears it"
    );
}

#[gpui_kit::test]
fn delete_unused_says_what_happened_on_its_row(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "ocr");
    let (host, _dir) = confirm_core(cx, true);
    cx.update(crate::bridge::tokio::init);
    click(cx, w, "ocr-delete-unused");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["Delete unused OCR models".to_string()]
    );
    let note = v.read_with(cx, |s, _| s.ui.unused_notes.get(&Family::Ocr).cloned());
    assert_eq!(
        note,
        Some((
            false,
            "Delete failed: app infrastructure isn't ready".into()
        ))
    );
}

fn apps() -> Vec<(String, String, bool)> {
    vec![
        (
            "com.agilebits.onepassword7".into(),
            "1Password".into(),
            true,
        ),
        ("com.bitwarden.desktop".into(), "Bitwarden".into(), false),
        ("com.apple.Notes".into(), "Notes".into(), false),
    ]
}

#[gpui_kit::test]
fn add_app_opens_a_dropdown_that_searches_and_picks_by_keys(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "privacy");
    // A core so "Add app…" may open (its app listing is replaced below).
    let (_host, _dir) = confirm_core(cx, false);
    cx.update(crate::bridge::tokio::init);
    click(cx, w, "excl-add");
    assert!(present(cx, w, "excl-pop"), "a dropdown, not a modal");
    v.update(cx, |s, cx| {
        s.ui.app_picker = Some(apps());
        cx.notify();
    });
    assert!(present(cx, w, "excl-pick-com.apple.Notes"));
    // The search field has focus: typing filters at once.
    on_window(cx, w, |window, cx| window.input("b", cx));
    assert!(!present(cx, w, "excl-pick-com.apple.Notes"), "filtered out");
    assert!(present(cx, w, "excl-pick-com.bitwarden.desktop"));
    assert!(
        present(cx, w, "excl-pick-com.agilebits.onepassword7"),
        "bundle ids match too"
    );
    press(cx, w, "down");
    assert_eq!(v.read_with(cx, |s, _| s.ui.app_highlight), 1);
    press(cx, w, "down");
    assert_eq!(v.read_with(cx, |s, _| s.ui.app_highlight), 0, "wraps");
    press(cx, w, "up");
    press(cx, w, "enter");
    assert!(
        v.read_with(cx, |s, _| s.ui.app_picker.is_none()),
        "a pick closes it"
    );
    assert!(b.calls().contains(&Call::Privacy), "the exclusion is saved");
}

#[gpui_kit::test]
fn add_app_closes_on_esc_and_on_a_press_outside(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, "privacy");
    let (_host, _dir) = confirm_core(cx, false);
    cx.update(crate::bridge::tokio::init);
    click(cx, w, "excl-add");
    v.update(cx, |s, cx| {
        s.ui.app_picker = Some(apps());
        cx.notify();
    });
    // The app's Esc binding for an open popover (`keymap.rs`).
    cx.update(|cx| {
        cx.bind_keys([gpui_kit::KeyBinding::new(
            "escape",
            crate::ui::popover::ClosePopover,
            Some(crate::ui::popover::OPEN_CONTEXT),
        )])
    });
    press(cx, w, "escape");
    assert!(!present(cx, w, "excl-pop"), "Esc closes it");
    click(cx, w, "excl-add");
    assert!(present(cx, w, "excl-pop"));
    click(cx, w, "filter-sys");
    assert!(!present(cx, w, "excl-pop"), "a press outside closes it");
}

#[gpui_kit::test]
fn cli_access_rereads_every_30s_only_while_data_is_shown(cx: &mut TestAppContext) {
    let (w, v, b) = open(cx, "access");
    let rechecks = |b: &crate::settings::fixture::FixtureBackend| saves(b, Call::Recheck);
    advance(cx, Duration::from_millis(29_999));
    assert_eq!(rechecks(&b), 0);
    advance(cx, Duration::from_millis(2));
    assert_eq!(rechecks(&b), 1, "the first re-read at 30 s");
    advance(cx, Duration::from_secs(30));
    assert_eq!(rechecks(&b), 2);
    // Another group: the poll stops.
    v.update(cx, |s, cx| s.open_tab("appearance", cx));
    frame(cx, w);
    advance(cx, Duration::from_secs(90));
    assert_eq!(rechecks(&b), 2);
    // Back on Data it runs again; leaving Settings stops it.
    v.update(cx, |s, cx| s.open_tab("access", cx));
    frame(cx, w);
    advance(cx, Duration::from_secs(30) + MS);
    assert_eq!(rechecks(&b), 3);
    v.update(cx, |s, _| s.set_shown(false));
    advance(cx, Duration::from_secs(90));
    assert_eq!(rechecks(&b), 3, "no timer outlives the page");
}
