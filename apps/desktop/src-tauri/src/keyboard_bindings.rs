//! Global-shortcut registration and what a pressed shortcut does. The bindings
//! model (shape, defaults, validation, `keyboard-bindings.json`) lives in
//! `app_core::keyboard_bindings`.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use app_core::keyboard_bindings::{
    load_settings_from_disk, persist_settings, KeyboardBindingsSettings, EDITABLE_ACTIONS,
};
use app_core::{AppCore, CoreEvent};
use tauri::{Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

const REGISTRATION_WARNING_ID: &str = "global-shortcuts-registration-failed";

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
    ToggleQuickRecall,
}

fn current_settings(app: &tauri::AppHandle) -> KeyboardBindingsSettings {
    let state = app.state::<KeyboardBindingsState>();
    let mut runtime = state.lock().expect("keyboard bindings state poisoned");
    if let Some(settings) = runtime.settings.clone() {
        return settings;
    }

    let settings = load_settings_from_disk(&app.state::<Arc<AppCore>>());
    runtime.settings = Some(settings.clone());
    settings
}

#[derive(Debug, Clone)]
struct RegisteredShortcut {
    shortcut: Shortcut,
    /// Stable action id from `EDITABLE_ACTIONS`, used to look up the display label.
    action_id: &'static str,
    /// The accelerator string the user assigned (e.g. "CommandOrControl+Shift+Space").
    accelerator: String,
}

impl RegisteredShortcut {
    /// Human-readable description naming the action and its accelerator, e.g.
    /// "Summon Quick Recall (CommandOrControl+Shift+Space)".
    fn display_label(&self) -> String {
        let label = EDITABLE_ACTIONS
            .iter()
            .find(|action| action.id == self.action_id)
            .map(|action| action.label)
            .unwrap_or(self.action_id);
        format!("{label} ({})", self.accelerator)
    }
}

fn parse_registered_shortcuts(settings: &KeyboardBindingsSettings) -> Vec<RegisteredShortcut> {
    if !settings.global_shortcuts.enabled {
        return Vec::new();
    }

    [
        (
            "toggleRecording",
            settings.global_shortcuts.bindings.toggle_recording.as_str(),
        ),
        (
            "pauseResumeRecording",
            settings
                .global_shortcuts
                .bindings
                .pause_resume_recording
                .as_str(),
        ),
        (
            "toggleMainWindow",
            settings
                .global_shortcuts
                .bindings
                .toggle_main_window
                .as_str(),
        ),
        (
            "toggleQuickRecall",
            settings
                .global_shortcuts
                .bindings
                .toggle_quick_recall
                .as_str(),
        ),
    ]
    .into_iter()
    .filter(|(_, binding)| !binding.trim().is_empty())
    .filter_map(|(action_id, binding)| {
        Shortcut::try_from(binding)
            .ok()
            .map(|shortcut| RegisteredShortcut {
                shortcut,
                action_id,
                accelerator: binding.trim().to_string(),
            })
    })
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
    let toggle_quick_recall = parse_shortcut_for_match(&bindings.toggle_quick_recall);

    if Some(shortcut) == toggle_recording.as_ref() {
        Some(GlobalShortcutAction::ToggleRecording)
    } else if Some(shortcut) == pause_resume_recording.as_ref() {
        Some(GlobalShortcutAction::PauseResumeRecording)
    } else if Some(shortcut) == toggle_main_window.as_ref() {
        Some(GlobalShortcutAction::ToggleMainWindow)
    } else if Some(shortcut) == toggle_quick_recall.as_ref() {
        Some(GlobalShortcutAction::ToggleQuickRecall)
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
    if let Err(failures) = refresh_global_shortcuts(app, &settings) {
        warn_registration_failure(app, &failures);
    }
}

fn refresh_global_shortcuts(
    app: &tauri::AppHandle,
    settings: &KeyboardBindingsSettings,
) -> Result<(), Vec<String>> {
    let previous = {
        let state = app.state::<KeyboardBindingsState>();
        let mut runtime = state.lock().expect("keyboard bindings state poisoned");
        std::mem::take(&mut runtime.registered_shortcuts)
    };

    for shortcut in previous {
        if let Err(error) = app.global_shortcut().unregister(shortcut.as_str()) {
            app_core::debug_log::log_warn(format!(
                "failed to unregister global shortcut '{shortcut}': {error}"
            ));
        }
    }

    let shortcuts = parse_registered_shortcuts(settings);
    let mut registered = Vec::new();
    let mut unique = HashSet::new();
    let mut failures = Vec::new();
    for entry in shortcuts {
        let shortcut_string = entry.shortcut.to_string();
        if !unique.insert(shortcut_string.clone()) {
            continue;
        }
        // Best-effort: a failure registering one shortcut is logged/collected but
        // does not abort registration of the remaining shortcuts.
        if let Err(error) = app.global_shortcut().register(entry.shortcut) {
            app_core::debug_log::log_warn(format!(
                "failed to register global shortcut '{shortcut_string}': {error}"
            ));
            failures.push(entry.display_label());
            continue;
        }
        registered.push(shortcut_string);
    }

    let state = app.state::<KeyboardBindingsState>();
    let mut runtime = state.lock().expect("keyboard bindings state poisoned");
    runtime.registered_shortcuts = registered;
    drop(runtime);

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
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
        Some(GlobalShortcutAction::ToggleQuickRecall) => {
            if let Err(error) = crate::windows::toggle_quick_recall_window(app) {
                app_core::debug_log::log_warn(format!(
                    "global shortcut failed to toggle quick recall: {error}"
                ));
            }
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
            app_core::debug_log::log_error(format!(
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
            app_core::debug_log::log_error(format!(
                "global shortcut failed to pause/resume recording: [{}] {}",
                error.code, error.message
            ));
        }
    });
}

fn warn_registration_failure(app: &tauri::AppHandle, failures: &[String]) {
    if failures.is_empty() {
        return;
    }

    let names = failures.join(", ");
    app_core::debug_log::log_warn(format!("global shortcut registration failed: {names}"));

    let plural = failures.len() > 1;
    let body = format!(
        "Mnema could not register {}: {names}. Another app may already be using the same shortcut.",
        if plural {
            "these global shortcuts"
        } else {
            "this global shortcut"
        }
    );
    app_core::native_capture::app_notifications::push_warning_app_notification(
        &app.state::<std::sync::Arc<app_core::AppCore>>(),
        REGISTRATION_WARNING_ID,
        "Global shortcuts unavailable",
        &body,
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
    let core = app.state::<Arc<AppCore>>();
    let settings = request.validated_for_update()?;
    persist_settings(&core, &settings)?;

    {
        let state = app.state::<KeyboardBindingsState>();
        let mut runtime = state.lock().expect("keyboard bindings state poisoned");
        runtime.settings = Some(settings.clone());
    }

    if let Err(failures) = refresh_global_shortcuts(&app, &settings) {
        warn_registration_failure(&app, &failures);
    }

    if let Ok(payload) = serde_json::to_value(&settings) {
        core.host()
            .emit(CoreEvent::KeyboardBindingsSettingsChanged(payload));
    }

    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_core::keyboard_bindings::{
        PAUSE_RESUME_RECORDING_DEFAULT, TOGGLE_QUICK_RECALL_DEFAULT,
    };

    #[test]
    fn defaults_include_quick_recall_global_shortcut() {
        let settings = KeyboardBindingsSettings::defaults();
        assert_eq!(
            settings.global_shortcuts.bindings.toggle_quick_recall,
            TOGGLE_QUICK_RECALL_DEFAULT
        );
        let shortcut = Shortcut::try_from(TOGGLE_QUICK_RECALL_DEFAULT).unwrap();
        assert_eq!(
            global_shortcut_action(&settings, &shortcut, true),
            Some(GlobalShortcutAction::ToggleQuickRecall)
        );
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
