//! Global shortcuts (BUILD-PROMPT §3.5): ⌘⌥R record, ⌘⌥P pause, ⌘⌥M show/hide
//! main, ⌘⌥Space Quick Recall by default. On press only, and only once
//! onboarding is complete and shortcuts are on. A change unregisters every
//! shortcut and registers the new set; a failure raises the in-app warning
//! `global-shortcuts-registration-failed`.
//!
//! app-core validates bindings with global-hotkey 0.7 (the Tauri plugin's); the
//! stored strings ("CommandOrControl+Alt+R") parse the same in 0.9, which only
//! renamed `SUPER` to `META`, so the conversion is a re-parse here.

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

use app_core::keyboard_bindings::{KeyboardBindingsSettings, EDITABLE_ACTIONS};
use app_core::AppCore;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui_kit::{App, Global};

pub const REGISTRATION_WARNING_ID: &str = "global-shortcuts-registration-failed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    ToggleRecording,
    PauseResumeRecording,
    ToggleMainWindow,
    ToggleQuickRecall,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub action: Action,
    pub hotkey: HotKey,
    /// "Summon Quick Recall (CommandOrControl+Alt+Space)": the warning names it.
    pub label: String,
}

/// An app-core (0.7-validated) binding string as a global-hotkey 0.9 shortcut.
pub fn convert(binding: &str) -> Option<HotKey> {
    let binding = binding.trim();
    (!binding.is_empty())
        .then(|| HotKey::from_str(binding).ok())
        .flatten()
}

/// What to register for `settings`: nothing when shortcuts are off; blank or
/// unparseable bindings skipped; a duplicate shortcut registered once.
pub fn plan(settings: &KeyboardBindingsSettings) -> Vec<Entry> {
    if !settings.global_shortcuts.enabled {
        return Vec::new();
    }
    let b = &settings.global_shortcuts.bindings;
    let mut seen = HashSet::new();
    [
        (
            "toggleRecording",
            Action::ToggleRecording,
            &b.toggle_recording,
        ),
        (
            "pauseResumeRecording",
            Action::PauseResumeRecording,
            &b.pause_resume_recording,
        ),
        (
            "toggleMainWindow",
            Action::ToggleMainWindow,
            &b.toggle_main_window,
        ),
        (
            "toggleQuickRecall",
            Action::ToggleQuickRecall,
            &b.toggle_quick_recall,
        ),
    ]
    .into_iter()
    .filter_map(|(id, action, binding)| {
        let hotkey = convert(binding)?;
        seen.insert(hotkey.id()).then(|| {
            let name = EDITABLE_ACTIONS
                .iter()
                .find(|a| a.id == id)
                .map_or(id, |a| a.label);
            Entry {
                action,
                hotkey,
                label: format!("{name} ({})", binding.trim()),
            }
        })
    })
    .collect()
}

/// The OS side, so the re-register logic is testable without Carbon.
pub trait Registrar {
    fn register(&self, hotkey: HotKey) -> Result<(), String>;
    fn unregister(&self, hotkey: HotKey) -> Result<(), String>;
}

impl Registrar for GlobalHotKeyManager {
    fn register(&self, hotkey: HotKey) -> Result<(), String> {
        GlobalHotKeyManager::register(self, hotkey).map_err(|e| e.to_string())
    }
    fn unregister(&self, hotkey: HotKey) -> Result<(), String> {
        GlobalHotKeyManager::unregister(self, hotkey).map_err(|e| e.to_string())
    }
}

/// Unregister everything we hold, then register `settings`' set. Returns what
/// is now registered and the labels that failed (best effort: one failure
/// doesn't stop the rest).
pub fn reregister(
    os: &impl Registrar,
    previous: Vec<Entry>,
    settings: &KeyboardBindingsSettings,
) -> (Vec<Entry>, Vec<String>) {
    for entry in previous {
        if let Err(e) = os.unregister(entry.hotkey) {
            log::warn!("failed to unregister a global shortcut: {e}");
        }
    }
    let (mut ok, mut failed) = (Vec::new(), Vec::new());
    for entry in plan(settings) {
        match os.register(entry.hotkey) {
            Ok(()) => ok.push(entry),
            Err(e) => {
                log::warn!("failed to register global shortcut {}: {e}", entry.label);
                failed.push(entry.label);
            }
        }
    }
    (ok, failed)
}

/// Which action a press fires, if any.
pub fn action_for(registered: &[Entry], id: u32, onboarding_complete: bool) -> Option<Action> {
    if !onboarding_complete {
        return None;
    }
    registered
        .iter()
        .find(|e| e.hotkey.id() == id)
        .map(|e| e.action)
}

pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    registered: Vec<Entry>,
}

impl Global for Hotkeys {}

/// Creates the manager (main thread) and registers the saved bindings.
/// Called from `AppCore::start`'s host setup; `events` forwards presses.
pub fn init(core: &Arc<AppCore>, events: async_channel::Sender<u32>) -> Option<Hotkeys> {
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(e) => {
            log::warn!("global shortcuts unavailable: {e}");
            return None;
        }
    };
    let settings = app_core::keyboard_bindings::load_settings_from_disk(core);
    let (registered, failed) = reregister(&manager, Vec::new(), &settings);
    warn_failures(core, &failed);
    let receiver = GlobalHotKeyEvent::receiver().clone();
    let spawned = std::thread::Builder::new()
        .name("mnema-hotkeys".into())
        .spawn(move || {
            while let Ok(event) = receiver.recv() {
                if event.state() == HotKeyState::Pressed
                    && events.send_blocking(event.id()).is_err()
                {
                    break;
                }
            }
        });
    if let Err(e) = spawned {
        log::warn!("global shortcut listener failed to start: {e}");
    }
    Some(Hotkeys {
        manager,
        registered,
    })
}

pub fn install(hotkeys: Option<Hotkeys>, cx: &mut App) {
    if let Some(h) = hotkeys {
        cx.set_global(h);
    }
}

/// Keyboard settings changed: unregister all, register the new set.
pub fn settings_changed(core: &Arc<AppCore>, settings: &KeyboardBindingsSettings, cx: &mut App) {
    let Some(h) = cx.try_global::<Hotkeys>() else {
        return;
    };
    let (registered, failed) = reregister(&h.manager, h.registered.clone(), settings);
    cx.global_mut::<Hotkeys>().registered = registered;
    warn_failures(core, &failed);
}

/// A press arrived from the listener thread.
pub fn pressed(core: &Arc<AppCore>, id: u32, cx: &mut App) {
    let Some(h) = cx.try_global::<Hotkeys>() else {
        return;
    };
    let complete = crate::native::onboarding_complete(core);
    match action_for(&h.registered, id, complete) {
        Some(Action::ToggleRecording) => off_main(core, |core| {
            crate::native::toggle_recording(core, "global-shortcut")
        }),
        Some(Action::PauseResumeRecording) => off_main(core, crate::native::toggle_pause),
        Some(Action::ToggleMainWindow) => crate::window::toggle_main(cx),
        Some(Action::ToggleQuickRecall) => {
            crate::feel::summon_started();
            crate::quick_recall::panel::toggle(cx)
        }
        None => {}
    }
}

/// Capture start/stop opens and flushes streams: never on the main thread.
fn off_main(
    core: &Arc<AppCore>,
    f: impl FnOnce(&Arc<AppCore>) -> crate::native::CaptureResult + Send + 'static,
) {
    let core = core.clone();
    std::thread::spawn(move || {
        if let Err((title, e)) = f(&core) {
            crate::native::report_capture_error(&core, title, &e, false);
        }
    });
}

fn warn_failures(core: &AppCore, failures: &[String]) {
    if failures.is_empty() {
        return;
    }
    let names = failures.join(", ");
    let body = format!(
        "Mnema could not register {}: {names}. Another app may already be using the same shortcut.",
        if failures.len() > 1 {
            "these global shortcuts"
        } else {
            "this global shortcut"
        }
    );
    app_core::native_capture::app_notifications::push_warning_app_notification(
        core,
        REGISTRATION_WARNING_ID,
        "Global shortcuts unavailable",
        &body,
        Some("shortcuts"),
        crate::native::now_unix_ms(),
    );
}

#[cfg(test)]
#[path = "hotkeys_tests.rs"]
mod tests;
