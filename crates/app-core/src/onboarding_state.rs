//! Whether onboarding has been completed: `onboarding-state.json` in the config
//! dir, cached in the managed [`OnboardingStateStore`] after the first read.

use std::{
    path::PathBuf,
    sync::{Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::AppCore;

const ONBOARDING_STATE_FILE_NAME: &str = "onboarding-state.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingState {
    schema_version: u32,
    completed_at_unix_ms: Option<u64>,
}

impl OnboardingState {
    fn incomplete() -> Self {
        Self {
            schema_version: 1,
            completed_at_unix_ms: None,
        }
    }

    pub fn completed_now() -> Self {
        Self {
            schema_version: 1,
            completed_at_unix_ms: Some(now_unix_ms()),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.completed_at_unix_ms.is_some()
    }
}

/// Command-only return shape for `get_onboarding_state`. Mirrors the persisted
/// `OnboardingState` fields and adds `recording_settings_ever_saved` — a
/// LIVE-COMPUTED, NON-PERSISTED signal (the existence of `recording-settings.json`
/// on disk). It lives on a SEPARATE type so the persisted file shape
/// (`OnboardingState`) stays unchanged: the computed flag is never written into
/// `onboarding-state.json` and never required when deserializing existing files.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingStateView {
    schema_version: u32,
    completed_at_unix_ms: Option<u64>,
    recording_settings_ever_saved: bool,
}

impl OnboardingStateView {
    fn from_state_and_disk(state: OnboardingState, recording_settings_ever_saved: bool) -> Self {
        Self {
            schema_version: state.schema_version,
            completed_at_unix_ms: state.completed_at_unix_ms,
            recording_settings_ever_saved,
        }
    }
}

#[derive(Debug, Default)]
pub struct OnboardingStateRuntime {
    state: Option<OnboardingState>,
}

pub type OnboardingStateStore = Mutex<OnboardingStateRuntime>;

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn onboarding_state_file_path(core: &AppCore) -> PathBuf {
    core.paths().config.join(ONBOARDING_STATE_FILE_NAME)
}

fn load_onboarding_state_from_path(path: PathBuf) -> OnboardingState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<OnboardingState>(&raw).ok())
        .filter(OnboardingState::is_complete)
        .unwrap_or_else(OnboardingState::incomplete)
}

fn lock_onboarding_state(store: &OnboardingStateStore) -> MutexGuard<'_, OnboardingStateRuntime> {
    store.lock().expect("onboarding state store poisoned")
}

pub fn current_onboarding_state(core: &AppCore, store: &OnboardingStateStore) -> OnboardingState {
    let mut runtime = lock_onboarding_state(store);
    if let Some(state) = runtime.state.clone() {
        return state;
    }

    let state = load_onboarding_state_from_path(onboarding_state_file_path(core));
    runtime.state = Some(state.clone());
    state
}

pub fn persist_onboarding_state(
    core: &AppCore,
    store: &OnboardingStateStore,
    state: OnboardingState,
) -> Result<OnboardingState, String> {
    let file_path = onboarding_state_file_path(core);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create onboarding state directory: {error}"))?;
    }

    let serialized = serde_json::to_string_pretty(&state)
        .map_err(|error| format!("Failed to serialize onboarding state: {error}"))?;
    std::fs::write(file_path, serialized)
        .map_err(|error| format!("Failed to persist onboarding state: {error}"))?;

    let mut runtime = lock_onboarding_state(store);
    runtime.state = Some(state.clone());

    Ok(state)
}

pub fn get_onboarding_state(core: &AppCore, store: &OnboardingStateStore) -> OnboardingStateView {
    // `recording-settings.json` is written ONLY by explicit user saves (never at
    // install/startup), so its existence == "the user has saved settings at least
    // once" == returning user. It must be read LIVE per call (NOT cached in
    // OnboardingStateStore) so a save performed between two onboarding entries in
    // one process lifetime is visible. Use the same path resolver saves land in.
    let ever_saved = crate::recording_settings::recording_settings_file_path(core).exists();
    OnboardingStateView::from_state_and_disk(current_onboarding_state(core, store), ever_saved)
}

#[cfg(test)]
mod tests {
    use super::{load_onboarding_state_from_path, OnboardingState, OnboardingStateView};

    #[test]
    fn missing_onboarding_state_is_incomplete() {
        let path = std::env::temp_dir().join(format!(
            "mnema-missing-onboarding-state-{}.json",
            super::now_unix_ms()
        ));

        assert!(!load_onboarding_state_from_path(path).is_complete());
    }

    #[test]
    fn onboarding_state_view_serializes_recording_settings_ever_saved_in_camel_case() {
        let view = OnboardingStateView::from_state_and_disk(
            OnboardingState {
                schema_version: 1,
                completed_at_unix_ms: Some(42),
            },
            true,
        );
        let json = serde_json::to_value(&view).expect("view serializes");
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["completedAtUnixMs"], 42);
        assert_eq!(json["recordingSettingsEverSaved"], true);
    }

    #[test]
    fn onboarding_state_view_carries_ever_saved_signal_independently() {
        // The signal is independent of completion: a not-yet-completed onboarding
        // can still report a returning user (settings saved), and vice versa.
        let returning =
            OnboardingStateView::from_state_and_disk(OnboardingState::incomplete(), true);
        assert!(returning.recording_settings_ever_saved);
        assert_eq!(returning.completed_at_unix_ms, None);

        let first_run =
            OnboardingStateView::from_state_and_disk(OnboardingState::incomplete(), false);
        assert!(!first_run.recording_settings_ever_saved);
    }

    #[test]
    fn invalid_onboarding_state_is_incomplete() {
        let path = std::env::temp_dir().join(format!(
            "mnema-invalid-onboarding-state-{}.json",
            super::now_unix_ms()
        ));
        std::fs::write(&path, "{not-json").expect("invalid test state should write");

        assert!(!load_onboarding_state_from_path(path.clone()).is_complete());

        let _ = std::fs::remove_file(path);
    }
}
