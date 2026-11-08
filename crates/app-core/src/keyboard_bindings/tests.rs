use super::rules::validate_conflicts;
use super::*;

#[test]
fn defaults_include_expected_global_shortcuts() {
    let settings = KeyboardBindingsSettings::defaults();
    assert!(settings.global_shortcuts.enabled);
    assert_eq!(
        settings.global_shortcuts.bindings.toggle_recording,
        TOGGLE_RECORDING_DEFAULT
    );
    assert_eq!(
        settings.global_shortcuts.bindings.pause_resume_recording,
        PAUSE_RESUME_RECORDING_DEFAULT
    );
    assert_eq!(
        settings.global_shortcuts.bindings.toggle_main_window,
        TOGGLE_MAIN_WINDOW_DEFAULT
    );
}

#[test]
fn load_sanitization_preserves_clear_bindings_and_falls_back_invalid_values() {
    let settings = KeyboardBindingsSettings {
        schema_version: 99,
        global_shortcuts: GlobalShortcutSettings {
            enabled: false,
            bindings: GlobalShortcutBindings {
                toggle_recording: "".to_string(),
                pause_resume_recording: "not-a-shortcut".to_string(),
                toggle_main_window: TOGGLE_MAIN_WINDOW_DEFAULT.to_string(),
                toggle_quick_recall: TOGGLE_QUICK_RECALL_DEFAULT.to_string(),
            },
        },
        app_shortcuts: AppShortcutBindings::defaults(),
        dashboard_shortcuts: DashboardShortcutBindings::defaults(),
        audio_drawer_shortcuts: AudioDrawerShortcutBindings::defaults(),
    }
    .sanitized_for_load();

    assert!(!settings.global_shortcuts.enabled);
    assert_eq!(settings.global_shortcuts.bindings.toggle_recording, "");
    assert_eq!(
        settings.global_shortcuts.bindings.pause_resume_recording,
        PAUSE_RESUME_RECORDING_DEFAULT
    );
}

#[test]
fn load_sanitization_preserves_existing_bindings_when_new_default_collides() {
    let settings = parse_settings_from_raw(
        r#"{
        "schemaVersion": 1,
        "globalShortcuts": {
            "enabled": true,
            "bindings": {
                "toggleRecording": "CommandOrControl+Alt+P",
                "toggleMainWindow": "CommandOrControl+Alt+M"
            }
        },
        "appShortcuts": {
            "openSettings": "CommandOrControl+Shift+,"
        }
    }"#,
    )
    .expect("legacy settings should deserialize");

    assert_eq!(
        settings.global_shortcuts.bindings.toggle_recording,
        PAUSE_RESUME_RECORDING_DEFAULT
    );
    assert_eq!(
        settings.global_shortcuts.bindings.pause_resume_recording,
        ""
    );
    assert_eq!(
        settings.app_shortcuts.open_settings,
        "CommandOrControl+Shift+,"
    );
    validate_conflicts(&settings).expect("sanitized settings should be conflict-free");

    let settings = parse_settings_from_raw(
        r#"{
        "schemaVersion": 1,
        "globalShortcuts": {
            "enabled": true,
            "bindings": {
                "toggleRecording": "CommandOrControl+Alt+R",
                "toggleMainWindow": "CommandOrControl+Alt+P"
            }
        }
    }"#,
    )
    .expect("legacy settings should deserialize");

    assert_eq!(
        settings.global_shortcuts.bindings.pause_resume_recording,
        ""
    );
    assert_eq!(
        settings.global_shortcuts.bindings.toggle_main_window,
        PAUSE_RESUME_RECORDING_DEFAULT
    );
    validate_conflicts(&settings).expect("sanitized settings should be conflict-free");
}

#[test]
fn update_validation_allows_cleared_binding() {
    let mut settings = KeyboardBindingsSettings::defaults();
    settings.dashboard_shortcuts.copy_frame = "".to_string();
    let settings = settings
        .validated_for_update()
        .expect("cleared binding should be valid");
    assert_eq!(settings.dashboard_shortcuts.copy_frame, "");
}

#[test]
fn native_shortcuts_require_non_shift_modifier() {
    let mut settings = KeyboardBindingsSettings::defaults();
    settings.global_shortcuts.bindings.pause_resume_recording = "P".to_string();
    assert!(settings.clone().validated_for_update().is_err());

    settings.global_shortcuts.bindings.pause_resume_recording = "Shift+P".to_string();
    assert!(settings.clone().validated_for_update().is_err());

    settings.global_shortcuts.bindings.pause_resume_recording = "Alt+P".to_string();
    assert!(settings.validated_for_update().is_ok());
}

#[test]
fn scoped_conflicts_are_rejected() {
    let mut settings = KeyboardBindingsSettings::defaults();
    settings.dashboard_shortcuts.copy_frame = "J".to_string();
    assert!(settings.validated_for_update().is_err());
}

#[test]
fn dashboard_and_audio_drawer_shortcuts_can_overlap() {
    let mut settings = KeyboardBindingsSettings::defaults();
    settings.audio_drawer_shortcuts.play_pause = "J".to_string();
    assert!(settings.validated_for_update().is_ok());
}

#[test]
fn app_and_audio_drawer_shortcuts_still_conflict() {
    let mut settings = KeyboardBindingsSettings::defaults();
    settings.audio_drawer_shortcuts.play_pause = "/".to_string();
    assert!(settings.validated_for_update().is_err());
}

#[test]
fn shortcuts_are_canonicalized() {
    let mut settings = KeyboardBindingsSettings::defaults();
    settings.dashboard_shortcuts.open_jump_picker = "shift+j".to_string();
    let settings = settings
        .validated_for_update()
        .expect("shortcut should canonicalize");
    assert_eq!(settings.dashboard_shortcuts.open_jump_picker, "Shift+J");
}

#[test]
fn unsupported_keys_are_rejected() {
    let mut settings = KeyboardBindingsSettings::defaults();
    settings.dashboard_shortcuts.open_jump_picker = "Home".to_string();
    assert!(settings.validated_for_update().is_err());
}

#[test]
fn fixed_foreground_behavior_shortcuts_are_reserved() {
    let mut settings = KeyboardBindingsSettings::defaults();
    settings.audio_drawer_shortcuts.play_pause = "Escape".to_string();
    assert!(settings.validated_for_update().is_err());
}

#[test]
fn fixed_dashboard_behavior_shortcuts_are_reserved() {
    let mut settings = KeyboardBindingsSettings::defaults();
    settings.audio_drawer_shortcuts.seek_back = "".to_string();
    settings.dashboard_shortcuts.open_jump_picker = "ArrowLeft".to_string();
    assert!(settings.validated_for_update().is_err());
}
