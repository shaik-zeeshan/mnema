use super::*;
use crate::recording_settings::default_recording_settings;
use crate::updates::AppUpdateState;
use capture_types::{CaptureSources, RecordingSettings};

fn support_all() -> CaptureSources {
    CaptureSources {
        screen: true,
        microphone: true,
        system_audio: true,
    }
}

fn settings_with_sources(screen: bool, microphone: bool, system_audio: bool) -> RecordingSettings {
    RecordingSettings {
        capture_screen: screen,
        capture_microphone: microphone,
        capture_system_audio: system_audio,
        ..default_recording_settings()
    }
}

#[test]
fn pre_onboarding_model_shows_setup_items_only() {
    let model = build_menu_model(
        false,
        false,
        false,
        false,
        &settings_with_sources(true, false, false),
        &support_all(),
        StatusBarOperation::Idle,
    );
    assert!(!model.onboarding_complete);
    assert_eq!(model.recording_label, None);
    assert!(model.source_items.is_empty());
}

#[test]
fn post_onboarding_idle_model_shows_start_and_enabled_sources() {
    let model = build_menu_model(
        true,
        false,
        false,
        false,
        &settings_with_sources(true, true, false),
        &support_all(),
        StatusBarOperation::Idle,
    );
    assert_eq!(model.recording_label, Some("Start Recording"));
    assert!(model.recording_enabled);
    assert_eq!(
        model.source_items,
        vec![
            SourceItemModel {
                id: SOURCE_SCREEN_ID,
                label: "Screen",
                checked: true,
                enabled: true
            },
            SourceItemModel {
                id: SOURCE_MICROPHONE_ID,
                label: "Microphone",
                checked: true,
                enabled: true
            },
            SourceItemModel {
                id: SOURCE_SYSTEM_AUDIO_ID,
                label: "System Audio",
                checked: false,
                enabled: true
            },
        ]
    );
}

#[test]
fn running_model_shows_stop_and_disables_sources() {
    let model = build_menu_model(
        true,
        true,
        false,
        false,
        &settings_with_sources(true, true, true),
        &support_all(),
        StatusBarOperation::Idle,
    );
    assert_eq!(model.recording_label, Some("Stop Recording"));
    assert!(model.recording_enabled);
    assert!(model.source_items.iter().all(|item| !item.enabled));
}

#[test]
fn low_disk_suspended_model_reads_paused_not_recording() {
    // The low-disk liveness suspension keeps the session alive, so it still
    // reports recording == true; the tray must surface the paused-low-disk
    // state and never read as plainly "Recording" (ADR 0040).
    let model = build_menu_model(
        true,
        true,
        false,
        true,
        &settings_with_sources(true, true, true),
        &support_all(),
        StatusBarOperation::Idle,
    );
    assert_eq!(model.status_label, Some("Paused — low disk"));
    assert_eq!(model.tooltip, "Mnema — Paused (low disk)");
    assert_ne!(model.tooltip, "Mnema - Recording");
}

#[test]
fn read_only_model_surfaces_trial_ended_and_disables_start() {
    // Read-Only Mode (licensing) is distinct from the low-disk suspension:
    // its own header + a greyed Start Recording, never a "paused" message.
    let mut model = build_menu_model(
        true,
        false,
        false,
        false,
        &settings_with_sources(true, true, true),
        &support_all(),
        StatusBarOperation::Idle,
    );
    apply_read_only_status(
        &mut model,
        true,
        Some(&capture_types::LicenseStatus::ReadOnly),
    );
    assert_eq!(model.status_label, Some("Read-Only — trial ended"));
    assert_eq!(model.tooltip, "Mnema — Read-Only (trial ended)");
    assert!(!model.recording_enabled);
}

#[test]
fn revoked_model_surfaces_revoked_not_trial_ended() {
    // A revoked (refunded/leaked) key blocks capture like a lapsed trial but
    // must read as "revoked" — never the trial-ended copy.
    let mut model = build_menu_model(
        true,
        false,
        false,
        false,
        &settings_with_sources(true, true, true),
        &support_all(),
        StatusBarOperation::Idle,
    );
    apply_read_only_status(
        &mut model,
        true,
        Some(&capture_types::LicenseStatus::Revoked),
    );
    assert_eq!(model.status_label, Some("Read-Only — license revoked"));
    assert_eq!(model.tooltip, "Mnema — Read-Only (license revoked)");
    assert!(!model.recording_enabled);
}

#[test]
fn unactivated_license_surfaces_activation_needed_not_trial_ended() {
    // A paid `Licensed` key whose once-per-machine activation Lapsed blocks
    // capture but is NOT a trial. `capture_refusal_copy` distinguishes it
    // ("finish activation" vs "trial ended"); the tray must too — a paying
    // customer must never be told their trial ended.
    let mut model = build_menu_model(
        true,
        false,
        false,
        false,
        &settings_with_sources(true, true, true),
        &support_all(),
        StatusBarOperation::Idle,
    );
    let status = capture_types::LicenseStatus::Licensed {
        update_through_ms: 0,
        in_window: true,
        email: String::new(),
        name: String::new(),
        activation: capture_types::Activation::Lapsed,
    };
    apply_read_only_status(&mut model, true, Some(&status));
    assert_ne!(
        model.status_label,
        Some("Read-Only — trial ended"),
        "a paid but unactivated license must not read as a lapsed trial"
    );
    assert!(!model.recording_enabled);
}

#[test]
fn read_only_override_leaves_a_live_session_alone() {
    // While recording (Stop Recording shown), a read-only read must not grey
    // the Stop button or relabel the tray — the guard only blocks new starts.
    let mut model = build_menu_model(
        true,
        true,
        false,
        false,
        &settings_with_sources(true, true, true),
        &support_all(),
        StatusBarOperation::Idle,
    );
    apply_read_only_status(
        &mut model,
        true,
        Some(&capture_types::LicenseStatus::ReadOnly),
    );
    assert_eq!(model.recording_label, Some("Stop Recording"));
    assert!(model.recording_enabled);
    assert_eq!(model.status_label, None);
}

#[test]
fn busy_models_disable_recording_command_and_sources() {
    for operation in [StatusBarOperation::Starting, StatusBarOperation::Stopping] {
        let model = build_menu_model(
            true,
            false,
            false,
            false,
            &settings_with_sources(true, true, true),
            &support_all(),
            operation,
        );
        assert!(!model.recording_enabled);
        assert!(model.source_items.iter().all(|item| !item.enabled));
    }
}

#[test]
fn last_remaining_source_cannot_be_unchecked() {
    let screen = build_menu_model(
        true,
        false,
        false,
        false,
        &settings_with_sources(true, false, false),
        &support_all(),
        StatusBarOperation::Idle,
    );
    assert!(!screen.source_items[0].enabled);

    let microphone = build_menu_model(
        true,
        false,
        false,
        false,
        &settings_with_sources(false, true, false),
        &support_all(),
        StatusBarOperation::Idle,
    );
    assert!(!microphone.source_items[1].enabled);
}

// System audio is its own capture family now (ADR 0052), so the tray must
// let the screen go while audio keeps recording — the tray was the last
// surface enforcing the coupling the ADR severed.
#[test]
fn screen_can_be_unchecked_while_system_audio_records_alone() {
    let model = build_menu_model(
        true,
        false,
        false,
        false,
        &settings_with_sources(true, false, true),
        &support_all(),
        StatusBarOperation::Idle,
    );
    assert!(model.source_items[0].enabled);
}

#[test]
fn unchecking_screen_leaves_system_audio_recording() {
    assert_eq!(
        computed_toggle_sources(
            CaptureSources {
                screen: true,
                microphone: false,
                system_audio: true,
            },
            SOURCE_SCREEN_ID,
        ),
        Some(CaptureSources {
            screen: false,
            microphone: false,
            system_audio: true,
        })
    );
}

#[test]
fn system_audio_is_toggleable_with_the_screen_off() {
    let model = build_menu_model(
        true,
        false,
        false,
        false,
        &settings_with_sources(false, true, false),
        &support_all(),
        StatusBarOperation::Idle,
    );
    assert!(model.source_items[2].enabled);
}

// The one rule that survives: a session needs a source. An audio-only
// session is a real session now, so system audio is subject to that rule
// like any other source — unchecking it when it stands alone is refused.
#[test]
fn system_audio_alone_is_a_session_and_cannot_be_unchecked() {
    assert_eq!(
        computed_toggle_sources(
            CaptureSources {
                screen: false,
                microphone: false,
                system_audio: true,
            },
            SOURCE_SYSTEM_AUDIO_ID,
        ),
        None
    );
}

#[test]
fn unsupported_sources_are_disabled_without_mutating_checked_state() {
    let model = build_menu_model(
        true,
        false,
        false,
        false,
        &settings_with_sources(true, true, true),
        &CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        },
        StatusBarOperation::Idle,
    );
    assert!(model.source_items[0].checked);
    assert!(model.source_items[1].checked);
    assert!(!model.source_items[1].enabled);
    assert!(model.source_items[2].checked);
    assert!(!model.source_items[2].enabled);
}

#[test]
fn tray_update_item_only_appears_for_installable_states() {
    assert_eq!(
        tray_update_menu_item(AppUpdateState::Available, Some("0.3.0"), true),
        Some(("Install Update 0.3.0\u{2026}".to_string(), true))
    );
    assert_eq!(
        tray_update_menu_item(AppUpdateState::RestartRequired, None, false),
        Some(("Restart to Finish Update".to_string(), true))
    );
    // Mid-install the row shows progress but must not be clickable again.
    assert_eq!(
        tray_update_menu_item(AppUpdateState::Downloading, Some("0.3.0"), true),
        Some(("Updating\u{2026}".to_string(), false))
    );
    // A failed install is retryable and keeps its pending update, so the
    // tray keeps a row — matching the update window and Settings, which
    // both keep `failed` installable.
    assert_eq!(
        tray_update_menu_item(AppUpdateState::Failed, Some("0.3.0"), true),
        Some(("Retry Update 0.3.0\u{2026}".to_string(), true))
    );
    // A failed CHECK carries no version (run_update_check clears it), so
    // there is nothing to retry and no row.
    assert_eq!(
        tray_update_menu_item(AppUpdateState::Failed, None, false),
        None
    );
    // The dangerous shape: `install_app_update`'s no-pending branch and
    // `restart_after_app_update`'s error path both set Failed while leaving
    // `update` populated. A version-keyed row would draw an enabled "Retry"
    // whose only outcome is the failure alert.
    assert_eq!(
        tray_update_menu_item(AppUpdateState::Failed, Some("0.3.0"), false),
        None
    );
    assert_eq!(
        tray_update_menu_item(AppUpdateState::Available, Some("0.3.0"), false),
        None
    );
    for state in [
        AppUpdateState::Idle,
        AppUpdateState::Checking,
        AppUpdateState::UpToDate,
        AppUpdateState::AvailableOutOfWindow,
        AppUpdateState::Incompatible,
    ] {
        assert_eq!(
            tray_update_menu_item(state, Some("0.3.0"), true),
            None,
            "{state:?}"
        );
    }
    // Available with no version in the feed is a broken row, not a row.
    assert_eq!(
        tray_update_menu_item(AppUpdateState::Available, None, true),
        None
    );
}
