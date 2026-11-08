//! The keyboard bindings model: the settings shape, its defaults, the
//! parse/validate/conflict rules and `keyboard-bindings.json` load/save. The
//! host owns global-shortcut registration and what a pressed shortcut does.

use std::{collections::HashSet, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::AppCore;

mod rules;
use rules::*;
pub use rules::{EditableAction, EDITABLE_ACTIONS};

const KEYBOARD_BINDINGS_FILE_NAME: &str = "keyboard-bindings.json";
const TOGGLE_RECORDING_DEFAULT: &str = "CommandOrControl+Alt+R";
pub const PAUSE_RESUME_RECORDING_DEFAULT: &str = "CommandOrControl+Alt+P";
const TOGGLE_MAIN_WINDOW_DEFAULT: &str = "CommandOrControl+Alt+M";
pub const TOGGLE_QUICK_RECALL_DEFAULT: &str = "CommandOrControl+Alt+Space";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeyboardBindingsSettings {
    #[serde(default = "schema_version_default")]
    schema_version: u32,
    #[serde(default = "GlobalShortcutSettings::defaults")]
    pub global_shortcuts: GlobalShortcutSettings,
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
    pub enabled: bool,
    #[serde(default = "GlobalShortcutBindings::defaults")]
    pub bindings: GlobalShortcutBindings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalShortcutBindings {
    #[serde(default = "default_toggle_recording")]
    pub toggle_recording: String,
    #[serde(default = "default_pause_resume_recording")]
    pub pause_resume_recording: String,
    #[serde(default = "default_toggle_main_window")]
    pub toggle_main_window: String,
    #[serde(rename = "quickRecall", default = "default_toggle_quick_recall")]
    pub toggle_quick_recall: String,
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
fn default_toggle_quick_recall() -> String {
    TOGGLE_QUICK_RECALL_DEFAULT.to_string()
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
            toggle_quick_recall: default_toggle_quick_recall(),
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
    pub fn defaults() -> Self {
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

    pub fn validated_for_update(mut self) -> Result<Self, String> {
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

fn keyboard_bindings_file_path(core: &AppCore) -> PathBuf {
    core.paths().config.join(KEYBOARD_BINDINGS_FILE_NAME)
}

pub fn persist_settings(core: &AppCore, settings: &KeyboardBindingsSettings) -> Result<(), String> {
    let file_path = keyboard_bindings_file_path(core);
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
        "toggleQuickRecall" => Some("/globalShortcuts/bindings/quickRecall"),
        "openSettings" => Some("/appShortcuts/openSettings"),
        "openDebug" => Some("/appShortcuts/openDebug"),
        "toggleSourceScreen" => Some("/appShortcuts/toggleSourceScreen"),
        "toggleSourceMicrophone" => Some("/appShortcuts/toggleSourceMicrophone"),
        "toggleSourceSystemAudio" => Some("/appShortcuts/toggleSourceSystemAudio"),
        "toggleShortcutsHelp" => Some("/appShortcuts/toggleShortcutsHelp"),
        "dashboard.openJumpPicker" => Some("/dashboardShortcuts/openJumpPicker"),
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

pub fn load_settings_from_disk(core: &AppCore) -> KeyboardBindingsSettings {
    let path = keyboard_bindings_file_path(core);
    let settings = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| parse_settings_from_raw(&raw))
        .unwrap_or_else(KeyboardBindingsSettings::defaults);

    if let Err(error) = persist_settings(core, &settings) {
        crate::debug_log::log_warn(format!(
            "failed to persist keyboard bindings defaults: {error}"
        ));
    }

    settings
}

#[cfg(test)]
mod tests;
