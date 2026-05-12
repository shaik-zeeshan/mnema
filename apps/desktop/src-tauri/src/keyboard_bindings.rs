use std::{
    collections::HashSet,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::{Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

const KEYBOARD_BINDINGS_FILE_NAME: &str = "keyboard-bindings.json";
const TOGGLE_RECORDING_DEFAULT: &str = "CommandOrControl+Alt+R";
const TOGGLE_MAIN_WINDOW_DEFAULT: &str = "CommandOrControl+Alt+M";
const REGISTRATION_WARNING_ID: &str = "global-shortcuts-registration-failed";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeyboardBindingsSettings {
    schema_version: u32,
    global_shortcuts: GlobalShortcutSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalShortcutSettings {
    enabled: bool,
    bindings: GlobalShortcutBindings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalShortcutBindings {
    toggle_recording: String,
    toggle_main_window: String,
}

#[derive(Debug, Default)]
pub struct KeyboardBindingsRuntime {
    settings: Option<KeyboardBindingsSettings>,
    registered_shortcuts: Vec<String>,
}

pub type KeyboardBindingsState = Mutex<KeyboardBindingsRuntime>;

impl KeyboardBindingsSettings {
    fn defaults() -> Self {
        Self {
            schema_version: 1,
            global_shortcuts: GlobalShortcutSettings {
                enabled: true,
                bindings: GlobalShortcutBindings {
                    toggle_recording: TOGGLE_RECORDING_DEFAULT.to_string(),
                    toggle_main_window: TOGGLE_MAIN_WINDOW_DEFAULT.to_string(),
                },
            },
        }
    }

    fn sanitized(self) -> Self {
        let defaults = Self::defaults();
        let mut seen = HashSet::new();

        let toggle_recording = sanitized_binding(
            self.global_shortcuts.bindings.toggle_recording,
            &defaults.global_shortcuts.bindings.toggle_recording,
            &mut seen,
        );
        let toggle_main_window = sanitized_binding(
            self.global_shortcuts.bindings.toggle_main_window,
            &defaults.global_shortcuts.bindings.toggle_main_window,
            &mut seen,
        );

        Self {
            schema_version: 1,
            global_shortcuts: GlobalShortcutSettings {
                enabled: self.global_shortcuts.enabled,
                bindings: GlobalShortcutBindings {
                    toggle_recording,
                    toggle_main_window,
                },
            },
        }
    }
}

fn sanitized_binding(value: String, fallback: &str, seen: &mut HashSet<String>) -> String {
    let trimmed = value.trim();
    let shortcut = Shortcut::try_from(trimmed)
        .ok()
        .and_then(|shortcut| seen.insert(shortcut.to_string()).then_some(shortcut));

    if let Some(shortcut) = shortcut {
        return shortcut.to_string();
    }

    if let Ok(fallback_shortcut) = Shortcut::try_from(fallback) {
        seen.insert(fallback_shortcut.to_string());
    }
    fallback.to_string()
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

fn load_settings_from_disk(app: &tauri::AppHandle) -> KeyboardBindingsSettings {
    let path = keyboard_bindings_file_path(app);
    let settings = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<KeyboardBindingsSettings>(&raw).ok())
        .map(KeyboardBindingsSettings::sanitized)
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
            .toggle_main_window
            .as_str(),
    ]
    .into_iter()
    .filter_map(|binding| Shortcut::try_from(binding).ok())
    .collect()
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
    for shortcut in shortcuts {
        let shortcut_string = shortcut.to_string();
        app.global_shortcut()
            .register(shortcut)
            .map_err(|error| format!("failed to register '{shortcut_string}': {error}"))?;
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
    if !settings.global_shortcuts.enabled {
        return;
    }

    let Ok(toggle_recording) =
        Shortcut::try_from(settings.global_shortcuts.bindings.toggle_recording.as_str())
    else {
        return;
    };
    let Ok(toggle_main_window) = Shortcut::try_from(
        settings
            .global_shortcuts
            .bindings
            .toggle_main_window
            .as_str(),
    ) else {
        return;
    };

    if shortcut == &toggle_recording {
        handle_toggle_recording(app);
    } else if shortcut == &toggle_main_window {
        crate::windows::toggle_main_window_visibility(app);
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

fn warn_registration_failure(app: &tauri::AppHandle, message: &str) {
    crate::native_capture::debug_log::log_warn(format!(
        "global shortcut registration failed: {message}"
    ));
    crate::native_capture::push_warning_app_notification(
        app,
        REGISTRATION_WARNING_ID,
        "Global shortcuts unavailable",
        "Mnema could not register one or more global shortcuts. Another app may already be using the same shortcut.",
        Some("capture"),
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
    let settings = request.sanitized();
    persist_settings(&app, &settings)?;

    {
        let state = app.state::<KeyboardBindingsState>();
        let mut runtime = state.lock().expect("keyboard bindings state poisoned");
        runtime.settings = Some(settings.clone());
    }

    if let Err(error) = refresh_global_shortcuts(&app, &settings) {
        warn_registration_failure(&app, &error);
    }

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
            settings.global_shortcuts.bindings.toggle_main_window,
            TOGGLE_MAIN_WINDOW_DEFAULT
        );
    }

    #[test]
    fn invalid_bindings_fall_back_to_defaults() {
        let settings = KeyboardBindingsSettings {
            schema_version: 99,
            global_shortcuts: GlobalShortcutSettings {
                enabled: false,
                bindings: GlobalShortcutBindings {
                    toggle_recording: "not-a-shortcut".to_string(),
                    toggle_main_window: "also bad".to_string(),
                },
            },
        }
        .sanitized();

        assert!(!settings.global_shortcuts.enabled);
        assert_eq!(
            settings.global_shortcuts.bindings.toggle_recording,
            TOGGLE_RECORDING_DEFAULT
        );
        assert_eq!(
            settings.global_shortcuts.bindings.toggle_main_window,
            TOGGLE_MAIN_WINDOW_DEFAULT
        );
    }

    #[test]
    fn duplicate_bindings_fall_back_to_distinct_defaults() {
        let settings = KeyboardBindingsSettings {
            schema_version: 1,
            global_shortcuts: GlobalShortcutSettings {
                enabled: true,
                bindings: GlobalShortcutBindings {
                    toggle_recording: TOGGLE_RECORDING_DEFAULT.to_string(),
                    toggle_main_window: TOGGLE_RECORDING_DEFAULT.to_string(),
                },
            },
        }
        .sanitized();

        assert_eq!(
            Shortcut::try_from(settings.global_shortcuts.bindings.toggle_recording.as_str()).ok(),
            Shortcut::try_from(TOGGLE_RECORDING_DEFAULT).ok(),
        );
        assert_eq!(
            settings.global_shortcuts.bindings.toggle_main_window,
            TOGGLE_MAIN_WINDOW_DEFAULT
        );
    }
}
