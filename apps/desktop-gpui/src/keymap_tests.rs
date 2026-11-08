use serde_json::json;

use super::*;

#[test]
fn settings_strings_become_gpui_keystrokes_and_labels() {
    assert_eq!(to_gpui("CommandOrControl+Alt+R"), "cmd-alt-r");
    assert_eq!(to_gpui("CommandOrControl+,"), "cmd-,");
    assert_eq!(to_gpui("Shift+ArrowLeft"), "shift-left");
    assert_eq!(to_gpui("CommandOrControl++"), "cmd-+");
    assert_eq!(display("CommandOrControl+Alt+Space"), "⌘⌥Space");
    assert_eq!(display("CommandOrControl+,"), "⌘,");
    assert_eq!(display("/"), "/");
    assert_eq!(to_gpui("1"), "1");
}

#[test]
fn reserved_and_missing_entries_keep_defaults() {
    let settings = json!({
        "globalShortcuts": {"bindings": {"toggleRecording": "CommandOrControl+Alt+X"}},
        "appShortcuts": {"toggleSourceScreen": "Escape", "toggleShortcutsHelp": "Shift+Tab"},
    });
    let b = Bindings::from_settings(&settings);
    assert_eq!(b.toggle_recording, "CommandOrControl+Alt+X");
    assert_eq!(b.sources[0], "1", "Escape is reserved");
    assert_eq!(b.toggle_shortcuts_help, "/", "Shift+Tab is reserved");
    assert!(is_reserved("shift+ArrowRight") && !is_reserved("ArrowUp"));
}

#[test]
fn help_sheet_is_built_from_state() {
    let b = Bindings::default();
    let stopped = HelpState {
        can_toggle_recording: true,
        can_toggle_sources: true,
        ..Default::default()
    };
    let groups = help_groups(&b, stopped, vec![]);
    let global: Vec<_> = groups[0].rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(global[0], "Start recording");
    assert!(global.contains(&"Toggle screen for the next recording"));
    assert!(!global.contains(&"Pause recording") && !global.contains(&"Open debug"));
    assert_eq!(groups[1].title, "Here");

    let recording = HelpState {
        capturing: true,
        user_paused: true,
        can_toggle_recording: true,
        developer_options: true,
        ..Default::default()
    };
    let extra = HelpGroup {
        title: "Timeline",
        rows: vec![],
    };
    let groups = help_groups(&b, recording, vec![extra]);
    let global: Vec<_> = groups[0].rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(&global[..2], ["Stop recording", "Resume recording"]);
    assert!(global.contains(&"Open debug"));
    assert_eq!(groups.len(), 2, "empty surface groups are dropped");
}
