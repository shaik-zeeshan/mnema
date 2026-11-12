use app_core::recording_settings::default_recording_settings;
use app_core::status_bar_model::*;
use capture_types::{CaptureSources, LicenseStatus};

use super::{entries, Entry};

fn support_all() -> CaptureSources {
    CaptureSources {
        screen: true,
        microphone: true,
        system_audio: true,
    }
}

fn model(onboarded: bool, recording: bool, low_disk: bool) -> StatusBarMenuModel {
    build_menu_model(
        onboarded,
        recording,
        false,
        low_disk,
        &default_recording_settings(),
        &support_all(),
        StatusBarOperation::Idle,
    )
}

/// Flattened ids (`-` for a separator, `[label]` for a submenu head).
fn ids(entries: &[Entry]) -> Vec<String> {
    let mut out = Vec::new();
    for e in entries {
        match e {
            Entry::Item { id, .. } | Entry::Check { id, .. } => out.push(id.to_string()),
            Entry::Submenu { label, items } => {
                out.push(format!("[{label}]"));
                out.extend(ids(items));
            }
            Entry::Separator => out.push("-".into()),
        }
    }
    out
}

#[test]
fn before_onboarding_only_setup_settings_quit() {
    assert_eq!(
        ids(&entries(&model(false, false, false))),
        [
            "tray_complete_setup",
            "tray_open_settings",
            "-",
            "tray_quit"
        ]
    );
}

#[test]
fn after_onboarding_full_menu_with_tauri_ids_in_order() {
    assert_eq!(
        ids(&entries(&model(true, false, false))),
        [
            "tray_recording_toggle",
            "tray_pause_toggle",
            "[Sources]",
            "tray_source_screen",
            "tray_source_microphone",
            "tray_source_system_audio",
            "tray_exclude_current_app",
            "[Delete Recent Capture]",
            "tray_delete_recent_60",
            "tray_delete_recent_300",
            "tray_delete_recent_900",
            "-",
            "tray_open_main",
            "tray_open_settings",
            "-",
            "tray_quit"
        ]
    );
}

#[test]
fn status_header_and_update_row_sit_where_the_tauri_menu_puts_them() {
    let mut m = model(true, true, true);
    m.update_item = Some(("Install Update 0.3.0\u{2026}".into(), true));
    let e = entries(&m);
    assert_eq!(
        e[0],
        Entry::Item {
            id: STATUS_HEADER_ID,
            label: "Paused — low disk".into(),
            enabled: false
        }
    );
    assert_eq!(e[1], Entry::Separator);
    let flat = ids(&e);
    let update = flat.iter().position(|i| i == UPDATE_ID).unwrap();
    assert_eq!(flat[update + 1], "-");
    assert_eq!(flat[update + 2], OPEN_MAIN_ID);
    assert!(e.contains(&Entry::Item {
        id: RECORDING_TOGGLE_ID,
        label: "Stop Recording".into(),
        enabled: true
    }));
}

#[test]
fn read_only_greys_start_and_sources_follow_the_model() {
    let mut m = model(true, false, false);
    apply_read_only_status(&mut m, true, Some(&LicenseStatus::ReadOnly));
    let e = entries(&m);
    assert_eq!(
        e[0],
        Entry::Item {
            id: STATUS_HEADER_ID,
            label: "Read-Only — trial ended".into(),
            enabled: false
        }
    );
    assert!(e.contains(&Entry::Item {
        id: RECORDING_TOGGLE_ID,
        label: "Start Recording".into(),
        enabled: false
    }));
    let Some(Entry::Submenu { items, .. }) = e.iter().find(|x| {
        matches!(
            x,
            Entry::Submenu {
                label: "Sources",
                ..
            }
        )
    }) else {
        panic!("no Sources submenu");
    };
    for (entry, source) in items.iter().zip(&m.source_items) {
        assert_eq!(
            entry,
            &Entry::Check {
                id: source.id,
                label: source.label,
                checked: source.checked,
                enabled: source.enabled
            }
        );
    }
}

fn session(running: bool, paused: bool) -> capture_types::NativeCaptureSession {
    capture_types::NativeCaptureSession {
        is_running: running,
        is_inactivity_paused: false,
        is_user_paused: paused,
        is_low_disk_suspended: false,
        requested_sources: None,
        output_files: None,
        source_sessions: None,
        recording_started_at_unix_ms: None,
    }
}

#[test]
fn a_busy_capture_lock_reads_the_last_session_seen() {
    use super::{capture, Capture};
    let mut last = Capture::default();
    let fresh = capture(Some(session(true, true)), &mut last);
    let held = Capture {
        running: true,
        paused: true,
        low_disk: false,
    };
    assert_eq!((fresh, last), (held, held));
    // Busy: the cache answers and stays as it was.
    assert_eq!(capture(None, &mut last), held);
    assert_eq!(last, held);
    // Free again: the fresh read replaces it.
    assert_eq!(
        capture(Some(session(false, false)), &mut last),
        Capture::default()
    );
    assert_eq!(last, Capture::default());
}
