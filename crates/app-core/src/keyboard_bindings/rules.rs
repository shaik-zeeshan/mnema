//! What makes a binding valid: canonical parsing, the native-shortcut check,
//! per-scope conflicts and the reserved keys.

use std::collections::{HashMap, HashSet};

use global_hotkey::hotkey::HotKey as Shortcut;

use super::KeyboardBindingsSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BindingPolicy {
    NativeBackground,
    Foreground,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ShortcutScope {
    NativeGlobal,
    Foreground,
    Dashboard,
    AudioDrawer,
}

#[derive(Debug, Clone, Copy)]
pub struct EditableAction {
    pub id: &'static str,
    pub label: &'static str,
    pub(super) scope: ShortcutScope,
    pub(super) policy: BindingPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReservedShortcutScope {
    Foreground,
    Dashboard,
}

#[derive(Debug, Clone, Copy)]
struct ReservedShortcut {
    scope: ReservedShortcutScope,
    binding: &'static str,
    label: &'static str,
}

#[derive(Debug, Clone)]
struct ParsedShortcut {
    primary: bool,
    alt: bool,
    shift: bool,
    key: String,
}

const RESERVED_SHORTCUTS: &[ReservedShortcut] = &[
    ReservedShortcut {
        scope: ReservedShortcutScope::Foreground,
        binding: "Escape",
        label: "close the active surface",
    },
    ReservedShortcut {
        scope: ReservedShortcutScope::Foreground,
        binding: "Tab",
        label: "move keyboard focus",
    },
    ReservedShortcut {
        scope: ReservedShortcutScope::Foreground,
        binding: "Shift+Tab",
        label: "move keyboard focus backward",
    },
    ReservedShortcut {
        scope: ReservedShortcutScope::Dashboard,
        binding: "ArrowLeft",
        label: "move to an older frame",
    },
    ReservedShortcut {
        scope: ReservedShortcutScope::Dashboard,
        binding: "ArrowRight",
        label: "move to a newer frame",
    },
    ReservedShortcut {
        scope: ReservedShortcutScope::Dashboard,
        binding: "Shift+ArrowLeft",
        label: "move 10 frames older",
    },
    ReservedShortcut {
        scope: ReservedShortcutScope::Dashboard,
        binding: "Shift+ArrowRight",
        label: "move 10 frames newer",
    },
];

pub const EDITABLE_ACTIONS: &[EditableAction] = &[
    EditableAction {
        id: "toggleRecording",
        label: "Start or stop recording",
        scope: ShortcutScope::NativeGlobal,
        policy: BindingPolicy::NativeBackground,
    },
    EditableAction {
        id: "pauseResumeRecording",
        label: "Pause or resume recording",
        scope: ShortcutScope::NativeGlobal,
        policy: BindingPolicy::NativeBackground,
    },
    EditableAction {
        id: "toggleMainWindow",
        label: "Show or hide Mnema",
        scope: ShortcutScope::NativeGlobal,
        policy: BindingPolicy::NativeBackground,
    },
    EditableAction {
        id: "toggleQuickRecall",
        label: "Summon Quick Recall",
        scope: ShortcutScope::NativeGlobal,
        policy: BindingPolicy::NativeBackground,
    },
    EditableAction {
        id: "openSettings",
        label: "Open settings",
        scope: ShortcutScope::Foreground,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "openDebug",
        label: "Open debug",
        scope: ShortcutScope::Foreground,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "toggleSourceScreen",
        label: "Toggle screen for the next recording",
        scope: ShortcutScope::Foreground,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "toggleSourceMicrophone",
        label: "Toggle microphone for the next recording",
        scope: ShortcutScope::Foreground,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "toggleSourceSystemAudio",
        label: "Toggle system audio for the next recording",
        scope: ShortcutScope::Foreground,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "toggleShortcutsHelp",
        label: "Show keyboard shortcuts",
        scope: ShortcutScope::Foreground,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "dashboard.openJumpPicker",
        label: "Open jump picker",
        scope: ShortcutScope::Dashboard,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "dashboard.jumpLatest",
        label: "Jump to latest",
        scope: ShortcutScope::Dashboard,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "dashboard.toggleOcr",
        label: "Toggle OCR panel",
        scope: ShortcutScope::Dashboard,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "dashboard.refreshTimeline",
        label: "Refresh timeline",
        scope: ShortcutScope::Dashboard,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "dashboard.copyFrame",
        label: "Copy active frame image",
        scope: ShortcutScope::Dashboard,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "dashboard.downloadFrame",
        label: "Download active frame image",
        scope: ShortcutScope::Dashboard,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "audioDrawer.playPause",
        label: "Play or pause",
        scope: ShortcutScope::AudioDrawer,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "audioDrawer.seekBack",
        label: "Seek back 5 seconds",
        scope: ShortcutScope::AudioDrawer,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "audioDrawer.seekForward",
        label: "Seek forward 5 seconds",
        scope: ShortcutScope::AudioDrawer,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "audioDrawer.seekBackFast",
        label: "Seek back 30 seconds",
        scope: ShortcutScope::AudioDrawer,
        policy: BindingPolicy::Foreground,
    },
    EditableAction {
        id: "audioDrawer.seekForwardFast",
        label: "Seek forward 30 seconds",
        scope: ShortcutScope::AudioDrawer,
        policy: BindingPolicy::Foreground,
    },
];

pub(super) fn get_binding<'a>(settings: &'a KeyboardBindingsSettings, id: &str) -> &'a str {
    match id {
        "toggleRecording" => &settings.global_shortcuts.bindings.toggle_recording,
        "pauseResumeRecording" => &settings.global_shortcuts.bindings.pause_resume_recording,
        "toggleMainWindow" => &settings.global_shortcuts.bindings.toggle_main_window,
        "toggleQuickRecall" => &settings.global_shortcuts.bindings.toggle_quick_recall,
        "openSettings" => &settings.app_shortcuts.open_settings,
        "openDebug" => &settings.app_shortcuts.open_debug,
        "toggleSourceScreen" => &settings.app_shortcuts.toggle_source_screen,
        "toggleSourceMicrophone" => &settings.app_shortcuts.toggle_source_microphone,
        "toggleSourceSystemAudio" => &settings.app_shortcuts.toggle_source_system_audio,
        "toggleShortcutsHelp" => &settings.app_shortcuts.toggle_shortcuts_help,
        "dashboard.openJumpPicker" => &settings.dashboard_shortcuts.open_jump_picker,
        "dashboard.jumpLatest" => &settings.dashboard_shortcuts.jump_latest,
        "dashboard.toggleOcr" => &settings.dashboard_shortcuts.toggle_ocr,
        "dashboard.refreshTimeline" => &settings.dashboard_shortcuts.refresh_timeline,
        "dashboard.copyFrame" => &settings.dashboard_shortcuts.copy_frame,
        "dashboard.downloadFrame" => &settings.dashboard_shortcuts.download_frame,
        "audioDrawer.playPause" => &settings.audio_drawer_shortcuts.play_pause,
        "audioDrawer.seekBack" => &settings.audio_drawer_shortcuts.seek_back,
        "audioDrawer.seekForward" => &settings.audio_drawer_shortcuts.seek_forward,
        "audioDrawer.seekBackFast" => &settings.audio_drawer_shortcuts.seek_back_fast,
        "audioDrawer.seekForwardFast" => &settings.audio_drawer_shortcuts.seek_forward_fast,
        _ => "",
    }
}

pub(super) fn set_binding(settings: &mut KeyboardBindingsSettings, id: &str, value: String) {
    match id {
        "toggleRecording" => settings.global_shortcuts.bindings.toggle_recording = value,
        "pauseResumeRecording" => settings.global_shortcuts.bindings.pause_resume_recording = value,
        "toggleMainWindow" => settings.global_shortcuts.bindings.toggle_main_window = value,
        "toggleQuickRecall" => settings.global_shortcuts.bindings.toggle_quick_recall = value,
        "openSettings" => settings.app_shortcuts.open_settings = value,
        "openDebug" => settings.app_shortcuts.open_debug = value,
        "toggleSourceScreen" => settings.app_shortcuts.toggle_source_screen = value,
        "toggleSourceMicrophone" => settings.app_shortcuts.toggle_source_microphone = value,
        "toggleSourceSystemAudio" => settings.app_shortcuts.toggle_source_system_audio = value,
        "toggleShortcutsHelp" => settings.app_shortcuts.toggle_shortcuts_help = value,
        "dashboard.openJumpPicker" => settings.dashboard_shortcuts.open_jump_picker = value,
        "dashboard.jumpLatest" => settings.dashboard_shortcuts.jump_latest = value,
        "dashboard.toggleOcr" => settings.dashboard_shortcuts.toggle_ocr = value,
        "dashboard.refreshTimeline" => settings.dashboard_shortcuts.refresh_timeline = value,
        "dashboard.copyFrame" => settings.dashboard_shortcuts.copy_frame = value,
        "dashboard.downloadFrame" => settings.dashboard_shortcuts.download_frame = value,
        "audioDrawer.playPause" => settings.audio_drawer_shortcuts.play_pause = value,
        "audioDrawer.seekBack" => settings.audio_drawer_shortcuts.seek_back = value,
        "audioDrawer.seekForward" => settings.audio_drawer_shortcuts.seek_forward = value,
        "audioDrawer.seekBackFast" => settings.audio_drawer_shortcuts.seek_back_fast = value,
        "audioDrawer.seekForwardFast" => settings.audio_drawer_shortcuts.seek_forward_fast = value,
        _ => {}
    }
}

pub(super) fn normalize_binding(
    value: &str,
    policy: BindingPolicy,
) -> Result<Option<String>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let parsed = parse_shortcut(trimmed)?;
    if policy == BindingPolicy::NativeBackground && !parsed.has_non_shift_modifier() {
        return Err("Background shortcuts must include Command/Control or Alt".to_string());
    }
    let canonical = parsed.canonical();
    if policy == BindingPolicy::NativeBackground && Shortcut::try_from(canonical.as_str()).is_err()
    {
        return Err(format!("'{canonical}' is not a valid native shortcut"));
    }
    Ok(Some(canonical))
}

fn parse_shortcut(value: &str) -> Result<ParsedShortcut, String> {
    let mut primary = false;
    let mut alt = false;
    let mut shift = false;
    let mut key: Option<String> = None;

    for raw_part in value.split('+') {
        let part = raw_part.trim();
        if part.is_empty() {
            return Err("Shortcut contains an empty key segment".to_string());
        }
        let lower = part.to_ascii_lowercase().replace('-', "");
        let is_modifier = match lower.as_str() {
            "commandorcontrol" | "cmdorctrl" | "primary" | "command" | "cmd" | "meta"
            | "control" | "ctrl" => {
                if primary {
                    return Err("Shortcut repeats the primary modifier".to_string());
                }
                primary = true;
                true
            }
            "alt" | "option" | "opt" => {
                if alt {
                    return Err("Shortcut repeats the Alt modifier".to_string());
                }
                alt = true;
                true
            }
            "shift" => {
                if shift {
                    return Err("Shortcut repeats the Shift modifier".to_string());
                }
                shift = true;
                true
            }
            _ => false,
        };
        if is_modifier {
            if key.is_some() {
                return Err("Shortcut modifiers must come before the key".to_string());
            }
            continue;
        }
        if key.is_some() {
            return Err("Shortcut can only contain one non-modifier key".to_string());
        }
        key = Some(normalize_key(part)?);
    }

    let key = key.ok_or_else(|| "Shortcut must include a key".to_string())?;
    Ok(ParsedShortcut {
        primary,
        alt,
        shift,
        key,
    })
}

fn normalize_key(key: &str) -> Result<String, String> {
    let lower = key.to_ascii_lowercase();
    let normalized = match lower.as_str() {
        "esc" | "escape" => "Escape".to_string(),
        "space" | "spacebar" => "Space".to_string(),
        "left" | "arrowleft" => "ArrowLeft".to_string(),
        "right" | "arrowright" => "ArrowRight".to_string(),
        "up" | "arrowup" => "ArrowUp".to_string(),
        "down" | "arrowdown" => "ArrowDown".to_string(),
        "tab" => "Tab".to_string(),
        "enter" | "return" => "Enter".to_string(),
        "backspace" => "Backspace".to_string(),
        "delete" => "Delete".to_string(),
        _ if key.chars().count() == 1 => key.to_ascii_uppercase(),
        _ if lower.starts_with('f') && lower[1..].parse::<u8>().is_ok() => key.to_ascii_uppercase(),
        _ => return Err(format!("Unsupported shortcut key '{key}'")),
    };
    Ok(normalized)
}

impl ParsedShortcut {
    fn has_non_shift_modifier(&self) -> bool {
        self.primary || self.alt
    }

    fn canonical(&self) -> String {
        let mut parts = Vec::new();
        if self.primary {
            parts.push("CommandOrControl".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        parts.push(self.key.clone());
        parts.join("+")
    }
}

fn scopes_conflict(left: ShortcutScope, right: ShortcutScope) -> bool {
    left == ShortcutScope::NativeGlobal
        || right == ShortcutScope::NativeGlobal
        || left == ShortcutScope::Foreground
        || right == ShortcutScope::Foreground
        || left == right
}

fn action_matches_reserved_scope(
    action: &EditableAction,
    reserved_scope: ReservedShortcutScope,
) -> bool {
    match reserved_scope {
        ReservedShortcutScope::Foreground => action.scope != ShortcutScope::NativeGlobal,
        ReservedShortcutScope::Dashboard => action.id.starts_with("dashboard."),
    }
}

fn validate_reserved_conflicts(settings: &KeyboardBindingsSettings) -> Result<(), String> {
    for action in EDITABLE_ACTIONS {
        let binding = get_binding(settings, action.id).trim();
        if binding.is_empty() {
            continue;
        }
        let key = binding.to_ascii_lowercase();
        for reserved in RESERVED_SHORTCUTS {
            if key == reserved.binding.to_ascii_lowercase()
                && action_matches_reserved_scope(action, reserved.scope)
            {
                return Err(format!(
                    "Shortcut '{}' for '{}' is reserved to {}",
                    binding, action.label, reserved.label
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_conflicts(settings: &KeyboardBindingsSettings) -> Result<(), String> {
    let mut seen: HashMap<String, (&EditableAction, String)> = HashMap::new();
    for action in EDITABLE_ACTIONS {
        let binding = get_binding(settings, action.id).trim();
        if binding.is_empty() {
            continue;
        }
        let key = binding.to_ascii_lowercase();
        if let Some((previous, previous_binding)) = seen.get(&key) {
            if scopes_conflict(previous.scope, action.scope) {
                return Err(format!(
                    "Shortcut '{previous_binding}' is assigned to both '{}' and '{}'",
                    previous.label, action.label
                ));
            }
        }
        seen.insert(key, (action, binding.to_string()));
    }
    validate_reserved_conflicts(settings)
}

pub(super) fn repair_load_conflicts(
    settings: &mut KeyboardBindingsSettings,
    defaults: &KeyboardBindingsSettings,
    defaulted_bindings: &HashSet<&'static str>,
) {
    let mut accepted: HashMap<String, (&EditableAction, String)> = HashMap::new();
    for repair_defaulted in [false, true] {
        for action in EDITABLE_ACTIONS {
            if defaulted_bindings.contains(action.id) != repair_defaulted {
                continue;
            }
            let binding = get_binding(settings, action.id).trim().to_string();
            if binding.is_empty() {
                continue;
            }

            if binding_conflicts_with_accepted(action, &binding, &accepted)
                || binding_conflicts_with_reserved(action, &binding)
            {
                let fallback = get_binding(defaults, action.id).trim().to_string();
                let replacement = if !fallback.is_empty()
                    && !binding_conflicts_with_accepted(action, &fallback, &accepted)
                    && !binding_conflicts_with_reserved(action, &fallback)
                {
                    fallback
                } else {
                    String::new()
                };
                set_binding(settings, action.id, replacement.clone());
                if replacement.is_empty() {
                    continue;
                }
                accepted.insert(replacement.to_ascii_lowercase(), (action, replacement));
                continue;
            }

            accepted.insert(binding.to_ascii_lowercase(), (action, binding));
        }
    }
}

fn binding_conflicts_with_accepted(
    action: &EditableAction,
    binding: &str,
    accepted: &HashMap<String, (&EditableAction, String)>,
) -> bool {
    accepted
        .get(&binding.to_ascii_lowercase())
        .is_some_and(|(previous, _)| scopes_conflict(previous.scope, action.scope))
}

fn binding_conflicts_with_reserved(action: &EditableAction, binding: &str) -> bool {
    let key = binding.to_ascii_lowercase();
    RESERVED_SHORTCUTS.iter().any(|reserved| {
        key == reserved.binding.to_ascii_lowercase()
            && action_matches_reserved_scope(action, reserved.scope)
    })
}
