use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

const KEYBOARD_BINDINGS_FILE_NAME: &str = "keyboard-bindings.json";
const KEYBOARD_BINDINGS_CHANGED_EVENT: &str = "keyboard_bindings_settings_changed";
const TOGGLE_RECORDING_DEFAULT: &str = "CommandOrControl+Alt+R";
const PAUSE_RESUME_RECORDING_DEFAULT: &str = "CommandOrControl+Alt+P";
const TOGGLE_MAIN_WINDOW_DEFAULT: &str = "CommandOrControl+Alt+M";
const REGISTRATION_WARNING_ID: &str = "global-shortcuts-registration-failed";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeyboardBindingsSettings {
    #[serde(default = "schema_version_default")]
    schema_version: u32,
    #[serde(default = "GlobalShortcutSettings::defaults")]
    global_shortcuts: GlobalShortcutSettings,
    #[serde(default = "AppShortcutBindings::defaults")]
    app_shortcuts: AppShortcutBindings,
    #[serde(default = "DashboardShortcutBindings::defaults")]
    dashboard_shortcuts: DashboardShortcutBindings,
    #[serde(default = "AudioDrawerShortcutBindings::defaults")]
    audio_drawer_shortcuts: AudioDrawerShortcutBindings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalShortcutSettings {
    #[serde(default = "bool_true")]
    enabled: bool,
    #[serde(default = "GlobalShortcutBindings::defaults")]
    bindings: GlobalShortcutBindings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalShortcutBindings {
    #[serde(default = "default_toggle_recording")]
    toggle_recording: String,
    #[serde(default = "default_pause_resume_recording")]
    pause_resume_recording: String,
    #[serde(default = "default_toggle_main_window")]
    toggle_main_window: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppShortcutBindings {
    #[serde(default = "default_open_settings")]
    open_settings: String,
    #[serde(default = "default_open_debug")]
    open_debug: String,
    #[serde(default = "default_toggle_source_screen")]
    toggle_source_screen: String,
    #[serde(default = "default_toggle_source_microphone")]
    toggle_source_microphone: String,
    #[serde(default = "default_toggle_source_system_audio")]
    toggle_source_system_audio: String,
    #[serde(default = "default_toggle_shortcuts_help")]
    toggle_shortcuts_help: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DashboardShortcutBindings {
    #[serde(default = "default_dashboard_open_jump_picker")]
    open_jump_picker: String,
    #[serde(default = "default_dashboard_search")]
    search: String,
    #[serde(default = "default_dashboard_jump_latest")]
    jump_latest: String,
    #[serde(default = "default_dashboard_toggle_ocr")]
    toggle_ocr: String,
    #[serde(default = "default_dashboard_refresh_timeline")]
    refresh_timeline: String,
    #[serde(default = "default_dashboard_copy_frame")]
    copy_frame: String,
    #[serde(default = "default_dashboard_download_frame")]
    download_frame: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioDrawerShortcutBindings {
    #[serde(default = "default_audio_play_pause")]
    play_pause: String,
    #[serde(default = "default_audio_seek_back")]
    seek_back: String,
    #[serde(default = "default_audio_seek_forward")]
    seek_forward: String,
    #[serde(default = "default_audio_seek_back_fast")]
    seek_back_fast: String,
    #[serde(default = "default_audio_seek_forward_fast")]
    seek_forward_fast: String,
}

#[derive(Debug, Default)]
pub struct KeyboardBindingsRuntime {
    settings: Option<KeyboardBindingsSettings>,
    registered_shortcuts: Vec<String>,
}

pub type KeyboardBindingsState = Mutex<KeyboardBindingsRuntime>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlobalShortcutAction {
    ToggleRecording,
    PauseResumeRecording,
    ToggleMainWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingPolicy {
    NativeBackground,
    Foreground,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ShortcutScope {
    NativeGlobal,
    Foreground,
    Dashboard,
    AudioDrawer,
}

#[derive(Debug, Clone, Copy)]
struct EditableAction {
    id: &'static str,
    label: &'static str,
    scope: ShortcutScope,
    policy: BindingPolicy,
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

const EDITABLE_ACTIONS: &[EditableAction] = &[
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
        id: "dashboard.search",
        label: "Search captured content",
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

fn schema_version_default() -> u32 {
    1
}
fn bool_true() -> bool {
    true
}
fn default_toggle_recording() -> String {
    TOGGLE_RECORDING_DEFAULT.to_string()
}
fn default_pause_resume_recording() -> String {
    PAUSE_RESUME_RECORDING_DEFAULT.to_string()
}
fn default_toggle_main_window() -> String {
    TOGGLE_MAIN_WINDOW_DEFAULT.to_string()
}
fn default_open_settings() -> String {
    "CommandOrControl+,".to_string()
}
fn default_open_debug() -> String {
    "CommandOrControl+D".to_string()
}
fn default_toggle_source_screen() -> String {
    "1".to_string()
}
fn default_toggle_source_microphone() -> String {
    "2".to_string()
}
fn default_toggle_source_system_audio() -> String {
    "3".to_string()
}
fn default_toggle_shortcuts_help() -> String {
    "/".to_string()
}
fn default_dashboard_open_jump_picker() -> String {
    "J".to_string()
}
fn default_dashboard_search() -> String {
    "CommandOrControl+K".to_string()
}
fn default_dashboard_jump_latest() -> String {
    "L".to_string()
}
fn default_dashboard_toggle_ocr() -> String {
    "O".to_string()
}
fn default_dashboard_refresh_timeline() -> String {
    "R".to_string()
}
fn default_dashboard_copy_frame() -> String {
    "C".to_string()
}
fn default_dashboard_download_frame() -> String {
    "D".to_string()
}
fn default_audio_play_pause() -> String {
    "Space".to_string()
}
fn default_audio_seek_back() -> String {
    "ArrowLeft".to_string()
}
fn default_audio_seek_forward() -> String {
    "ArrowRight".to_string()
}
fn default_audio_seek_back_fast() -> String {
    "Shift+ArrowLeft".to_string()
}
fn default_audio_seek_forward_fast() -> String {
    "Shift+ArrowRight".to_string()
}

impl GlobalShortcutSettings {
    fn defaults() -> Self {
        Self {
            enabled: true,
            bindings: GlobalShortcutBindings::defaults(),
        }
    }
}

impl GlobalShortcutBindings {
    fn defaults() -> Self {
        Self {
            toggle_recording: default_toggle_recording(),
            pause_resume_recording: default_pause_resume_recording(),
            toggle_main_window: default_toggle_main_window(),
        }
    }
}

impl AppShortcutBindings {
    fn defaults() -> Self {
        Self {
            open_settings: default_open_settings(),
            open_debug: default_open_debug(),
            toggle_source_screen: default_toggle_source_screen(),
            toggle_source_microphone: default_toggle_source_microphone(),
            toggle_source_system_audio: default_toggle_source_system_audio(),
            toggle_shortcuts_help: default_toggle_shortcuts_help(),
        }
    }
}

impl DashboardShortcutBindings {
    fn defaults() -> Self {
        Self {
            open_jump_picker: default_dashboard_open_jump_picker(),
            search: default_dashboard_search(),
            jump_latest: default_dashboard_jump_latest(),
            toggle_ocr: default_dashboard_toggle_ocr(),
            refresh_timeline: default_dashboard_refresh_timeline(),
            copy_frame: default_dashboard_copy_frame(),
            download_frame: default_dashboard_download_frame(),
        }
    }
}

impl AudioDrawerShortcutBindings {
    fn defaults() -> Self {
        Self {
            play_pause: default_audio_play_pause(),
            seek_back: default_audio_seek_back(),
            seek_forward: default_audio_seek_forward(),
            seek_back_fast: default_audio_seek_back_fast(),
            seek_forward_fast: default_audio_seek_forward_fast(),
        }
    }
}

impl KeyboardBindingsSettings {
    fn defaults() -> Self {
        Self {
            schema_version: 1,
            global_shortcuts: GlobalShortcutSettings::defaults(),
            app_shortcuts: AppShortcutBindings::defaults(),
            dashboard_shortcuts: DashboardShortcutBindings::defaults(),
            audio_drawer_shortcuts: AudioDrawerShortcutBindings::defaults(),
        }
    }

    #[cfg(test)]
    fn sanitized_for_load(self) -> Self {
        self.sanitized_for_load_with_defaulted_bindings(HashSet::new())
    }

    fn sanitized_for_load_with_defaulted_bindings(
        self,
        mut defaulted_bindings: HashSet<&'static str>,
    ) -> Self {
        let mut settings = self;
        settings.schema_version = 1;
        settings.normalize_or_fallback(&mut defaulted_bindings);
        settings
    }

    fn validated_for_update(mut self) -> Result<Self, String> {
        self.schema_version = 1;
        self.normalize_or_error()?;
        validate_conflicts(&self)?;
        Ok(self)
    }

    fn normalize_or_fallback(&mut self, defaulted_bindings: &mut HashSet<&'static str>) {
        let defaults = Self::defaults();
        for action in EDITABLE_ACTIONS {
            let value = get_binding(self, action.id).to_string();
            let normalized = match normalize_binding(&value, action.policy) {
                Ok(Some(binding)) => binding,
                Ok(None) => String::new(),
                Err(_) => {
                    defaulted_bindings.insert(action.id);
                    get_binding(&defaults, action.id).to_string()
                }
            };
            set_binding(self, action.id, normalized);
        }
        repair_load_conflicts(self, &defaults, defaulted_bindings);
    }

    fn normalize_or_error(&mut self) -> Result<(), String> {
        for action in EDITABLE_ACTIONS {
            let value = get_binding(self, action.id).to_string();
            let normalized = normalize_binding(&value, action.policy)?;
            set_binding(self, action.id, normalized.unwrap_or_default());
        }
        Ok(())
    }
}

fn get_binding<'a>(settings: &'a KeyboardBindingsSettings, id: &str) -> &'a str {
    match id {
        "toggleRecording" => &settings.global_shortcuts.bindings.toggle_recording,
        "pauseResumeRecording" => &settings.global_shortcuts.bindings.pause_resume_recording,
        "toggleMainWindow" => &settings.global_shortcuts.bindings.toggle_main_window,
        "openSettings" => &settings.app_shortcuts.open_settings,
        "openDebug" => &settings.app_shortcuts.open_debug,
        "toggleSourceScreen" => &settings.app_shortcuts.toggle_source_screen,
        "toggleSourceMicrophone" => &settings.app_shortcuts.toggle_source_microphone,
        "toggleSourceSystemAudio" => &settings.app_shortcuts.toggle_source_system_audio,
        "toggleShortcutsHelp" => &settings.app_shortcuts.toggle_shortcuts_help,
        "dashboard.openJumpPicker" => &settings.dashboard_shortcuts.open_jump_picker,
        "dashboard.search" => &settings.dashboard_shortcuts.search,
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

fn set_binding(settings: &mut KeyboardBindingsSettings, id: &str, value: String) {
    match id {
        "toggleRecording" => settings.global_shortcuts.bindings.toggle_recording = value,
        "pauseResumeRecording" => settings.global_shortcuts.bindings.pause_resume_recording = value,
        "toggleMainWindow" => settings.global_shortcuts.bindings.toggle_main_window = value,
        "openSettings" => settings.app_shortcuts.open_settings = value,
        "openDebug" => settings.app_shortcuts.open_debug = value,
        "toggleSourceScreen" => settings.app_shortcuts.toggle_source_screen = value,
        "toggleSourceMicrophone" => settings.app_shortcuts.toggle_source_microphone = value,
        "toggleSourceSystemAudio" => settings.app_shortcuts.toggle_source_system_audio = value,
        "toggleShortcutsHelp" => settings.app_shortcuts.toggle_shortcuts_help = value,
        "dashboard.openJumpPicker" => settings.dashboard_shortcuts.open_jump_picker = value,
        "dashboard.search" => settings.dashboard_shortcuts.search = value,
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

fn normalize_binding(value: &str, policy: BindingPolicy) -> Result<Option<String>, String> {
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

fn validate_conflicts(settings: &KeyboardBindingsSettings) -> Result<(), String> {
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

fn repair_load_conflicts(
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

fn keyboard_bindings_file_path(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(config_dir) = app.path().app_config_dir() {
        return config_dir.join(KEYBOARD_BINDINGS_FILE_NAME);
    }

    PathBuf::from(".mnema").join(KEYBOARD_BINDINGS_FILE_NAME)
}

fn persist_settings(
    app: &tauri::AppHandle,
    settings: &KeyboardBindingsSettings,
) -> Result<(), String> {
    let file_path = keyboard_bindings_file_path(app);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create keyboard bindings directory: {error}"))?;
    }

    let serialized = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("Failed to serialize keyboard bindings: {error}"))?;
    std::fs::write(file_path, serialized)
        .map_err(|error| format!("Failed to persist keyboard bindings: {error}"))
}

fn parse_settings_from_raw(raw: &str) -> Option<KeyboardBindingsSettings> {
    let defaulted_bindings = defaulted_binding_ids(raw);
    serde_json::from_str::<KeyboardBindingsSettings>(raw)
        .ok()
        .map(|settings| settings.sanitized_for_load_with_defaulted_bindings(defaulted_bindings))
}

fn defaulted_binding_ids(raw: &str) -> HashSet<&'static str> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return HashSet::new();
    };
    EDITABLE_ACTIONS
        .iter()
        .filter_map(|action| {
            binding_json_pointer(action.id)
                .filter(|pointer| value.pointer(pointer).is_none())
                .map(|_| action.id)
        })
        .collect()
}

fn binding_json_pointer(id: &str) -> Option<&'static str> {
    match id {
        "toggleRecording" => Some("/globalShortcuts/bindings/toggleRecording"),
        "pauseResumeRecording" => Some("/globalShortcuts/bindings/pauseResumeRecording"),
        "toggleMainWindow" => Some("/globalShortcuts/bindings/toggleMainWindow"),
        "openSettings" => Some("/appShortcuts/openSettings"),
        "openDebug" => Some("/appShortcuts/openDebug"),
        "toggleSourceScreen" => Some("/appShortcuts/toggleSourceScreen"),
        "toggleSourceMicrophone" => Some("/appShortcuts/toggleSourceMicrophone"),
        "toggleSourceSystemAudio" => Some("/appShortcuts/toggleSourceSystemAudio"),
        "toggleShortcutsHelp" => Some("/appShortcuts/toggleShortcutsHelp"),
        "dashboard.openJumpPicker" => Some("/dashboardShortcuts/openJumpPicker"),
        "dashboard.search" => Some("/dashboardShortcuts/search"),
        "dashboard.jumpLatest" => Some("/dashboardShortcuts/jumpLatest"),
        "dashboard.toggleOcr" => Some("/dashboardShortcuts/toggleOcr"),
        "dashboard.refreshTimeline" => Some("/dashboardShortcuts/refreshTimeline"),
        "dashboard.copyFrame" => Some("/dashboardShortcuts/copyFrame"),
        "dashboard.downloadFrame" => Some("/dashboardShortcuts/downloadFrame"),
        "audioDrawer.playPause" => Some("/audioDrawerShortcuts/playPause"),
        "audioDrawer.seekBack" => Some("/audioDrawerShortcuts/seekBack"),
        "audioDrawer.seekForward" => Some("/audioDrawerShortcuts/seekForward"),
        "audioDrawer.seekBackFast" => Some("/audioDrawerShortcuts/seekBackFast"),
        "audioDrawer.seekForwardFast" => Some("/audioDrawerShortcuts/seekForwardFast"),
        _ => None,
    }
}

fn load_settings_from_disk(app: &tauri::AppHandle) -> KeyboardBindingsSettings {
    let path = keyboard_bindings_file_path(app);
    let settings = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| parse_settings_from_raw(&raw))
        .unwrap_or_else(KeyboardBindingsSettings::defaults);

    if let Err(error) = persist_settings(app, &settings) {
        crate::native_capture::debug_log::log_warn(format!(
            "failed to persist keyboard bindings defaults: {error}"
        ));
    }

    settings
}

fn current_settings(app: &tauri::AppHandle) -> KeyboardBindingsSettings {
    let state = app.state::<KeyboardBindingsState>();
    let mut runtime = state.lock().expect("keyboard bindings state poisoned");
    if let Some(settings) = runtime.settings.clone() {
        return settings;
    }

    let settings = load_settings_from_disk(app);
    runtime.settings = Some(settings.clone());
    settings
}

fn parse_registered_shortcuts(settings: &KeyboardBindingsSettings) -> Vec<Shortcut> {
    if !settings.global_shortcuts.enabled {
        return Vec::new();
    }

    [
        settings.global_shortcuts.bindings.toggle_recording.as_str(),
        settings
            .global_shortcuts
            .bindings
            .pause_resume_recording
            .as_str(),
        settings
            .global_shortcuts
            .bindings
            .toggle_main_window
            .as_str(),
    ]
    .into_iter()
    .filter(|binding| !binding.trim().is_empty())
    .filter_map(|binding| Shortcut::try_from(binding).ok())
    .collect()
}

fn global_shortcut_action(
    settings: &KeyboardBindingsSettings,
    shortcut: &Shortcut,
    onboarding_complete: bool,
) -> Option<GlobalShortcutAction> {
    if !onboarding_complete || !settings.global_shortcuts.enabled {
        return None;
    }

    let bindings = &settings.global_shortcuts.bindings;
    let toggle_recording = parse_shortcut_for_match(&bindings.toggle_recording);
    let pause_resume_recording = parse_shortcut_for_match(&bindings.pause_resume_recording);
    let toggle_main_window = parse_shortcut_for_match(&bindings.toggle_main_window);

    if Some(shortcut) == toggle_recording.as_ref() {
        Some(GlobalShortcutAction::ToggleRecording)
    } else if Some(shortcut) == pause_resume_recording.as_ref() {
        Some(GlobalShortcutAction::PauseResumeRecording)
    } else if Some(shortcut) == toggle_main_window.as_ref() {
        Some(GlobalShortcutAction::ToggleMainWindow)
    } else {
        None
    }
}

fn parse_shortcut_for_match(binding: &str) -> Option<Shortcut> {
    if binding.trim().is_empty() {
        return None;
    }
    Shortcut::try_from(binding).ok()
}

pub(crate) fn initialize(app: &tauri::AppHandle) {
    let settings = current_settings(app);
    if let Err(error) = refresh_global_shortcuts(app, &settings) {
        warn_registration_failure(app, &error);
    }
}

fn refresh_global_shortcuts(
    app: &tauri::AppHandle,
    settings: &KeyboardBindingsSettings,
) -> Result<(), String> {
    let previous = {
        let state = app.state::<KeyboardBindingsState>();
        let mut runtime = state.lock().expect("keyboard bindings state poisoned");
        std::mem::take(&mut runtime.registered_shortcuts)
    };

    for shortcut in previous {
        if let Err(error) = app.global_shortcut().unregister(shortcut.as_str()) {
            crate::native_capture::debug_log::log_warn(format!(
                "failed to unregister global shortcut '{shortcut}': {error}"
            ));
        }
    }

    let shortcuts = parse_registered_shortcuts(settings);
    let mut registered = Vec::new();
    let mut unique = HashSet::new();
    for shortcut in shortcuts {
        let shortcut_string = shortcut.to_string();
        if !unique.insert(shortcut_string.clone()) {
            continue;
        }
        if let Err(error) = app.global_shortcut().register(shortcut) {
            let state = app.state::<KeyboardBindingsState>();
            let mut runtime = state.lock().expect("keyboard bindings state poisoned");
            runtime.registered_shortcuts = registered;
            return Err(format!("failed to register '{shortcut_string}': {error}"));
        }
        registered.push(shortcut_string);
    }

    let state = app.state::<KeyboardBindingsState>();
    let mut runtime = state.lock().expect("keyboard bindings state poisoned");
    runtime.registered_shortcuts = registered;

    Ok(())
}

pub(crate) fn handle_global_shortcut(
    app: &tauri::AppHandle<Wry>,
    shortcut: &Shortcut,
    event: ShortcutEvent,
) {
    if event.state() != ShortcutState::Pressed {
        return;
    }

    let settings = current_settings(app);
    match global_shortcut_action(
        &settings,
        shortcut,
        crate::windows::is_onboarding_complete(app),
    ) {
        Some(GlobalShortcutAction::ToggleRecording) => handle_toggle_recording(app),
        Some(GlobalShortcutAction::PauseResumeRecording) => handle_pause_resume_recording(app),
        Some(GlobalShortcutAction::ToggleMainWindow) => {
            crate::windows::toggle_main_window_visibility(app);
        }
        None => {}
    }
}

fn handle_toggle_recording(app: &tauri::AppHandle) {
    let app_handle = app.clone();
    std::thread::spawn(move || {
        let recording =
            crate::native_capture::current_native_capture_session(&app_handle).is_running;
        let result = if recording {
            crate::native_capture::stop_native_capture_from_app_handle(&app_handle).map(|_| ())
        } else {
            crate::native_capture::start_native_capture_from_app_handle(
                "global-shortcut",
                &app_handle,
            )
            .map(|_| ())
        };

        if let Err(error) = result {
            crate::native_capture::debug_log::log_warn(format!(
                "global shortcut failed to toggle recording: [{}] {}",
                error.code, error.message
            ));
        }
    });
}

fn handle_pause_resume_recording(app: &tauri::AppHandle) {
    let app_handle = app.clone();
    std::thread::spawn(move || {
        let session = crate::native_capture::current_native_capture_session(&app_handle);
        if !session.is_running {
            return;
        }
        let result = if session.is_user_paused {
            crate::native_capture::resume_native_capture_from_app_handle(&app_handle).map(|_| ())
        } else {
            crate::native_capture::pause_native_capture_from_app_handle(&app_handle).map(|_| ())
        };

        if let Err(error) = result {
            crate::native_capture::debug_log::log_warn(format!(
                "global shortcut failed to pause/resume recording: [{}] {}",
                error.code, error.message
            ));
        }
    });
}

fn warn_registration_failure(app: &tauri::AppHandle, message: &str) {
    crate::native_capture::debug_log::log_warn(format!(
        "global shortcut registration failed: {message}"
    ));
    crate::native_capture::push_warning_app_notification(
        app,
        REGISTRATION_WARNING_ID,
        "Global shortcuts unavailable",
        "Mnema could not register one or more global shortcuts. Another app may already be using the same shortcut.",
        Some("shortcuts"),
        now_unix_ms(),
    );
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[tauri::command]
pub fn get_keyboard_bindings_settings(app: tauri::AppHandle) -> KeyboardBindingsSettings {
    current_settings(&app)
}

#[tauri::command]
pub fn update_keyboard_bindings_settings(
    app: tauri::AppHandle,
    request: KeyboardBindingsSettings,
) -> Result<KeyboardBindingsSettings, String> {
    let settings = request.validated_for_update()?;
    persist_settings(&app, &settings)?;

    {
        let state = app.state::<KeyboardBindingsState>();
        let mut runtime = state.lock().expect("keyboard bindings state poisoned");
        runtime.settings = Some(settings.clone());
    }

    if let Err(error) = refresh_global_shortcuts(&app, &settings) {
        warn_registration_failure(&app, &error);
    }

    let _ = app.emit(KEYBOARD_BINDINGS_CHANGED_EVENT, &settings);

    Ok(settings)
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn global_shortcut_action_matches_pause_resume() {
        let settings = KeyboardBindingsSettings::defaults();
        let shortcut = Shortcut::try_from(PAUSE_RESUME_RECORDING_DEFAULT).unwrap();
        assert_eq!(
            global_shortcut_action(&settings, &shortcut, true),
            Some(GlobalShortcutAction::PauseResumeRecording)
        );
    }
}
