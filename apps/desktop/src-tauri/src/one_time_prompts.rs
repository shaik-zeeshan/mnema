use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const ONE_TIME_PROMPTS_FILE_NAME: &str = "one-time-prompts.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OneTimePromptState {
    #[serde(default)]
    pub prompts: BTreeMap<String, OneTimePromptRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OneTimePromptRecord {
    pub shown_at: Option<String>,
    pub dismissed_at: Option<String>,
    pub completed_at: Option<String>,
}

pub type OneTimePromptStateStore = Mutex<OneTimePromptState>;

fn prompt_state_path(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(ONE_TIME_PROMPTS_FILE_NAME)
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn valid_prompt_id(prompt_id: &str) -> Result<String, String> {
    let prompt_id = prompt_id.trim();
    if prompt_id.is_empty() {
        return Err("Prompt id is required".to_string());
    }
    Ok(prompt_id.to_string())
}

fn load_from_disk(app: &tauri::AppHandle) -> OneTimePromptState {
    let path = prompt_state_path(app);
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => OneTimePromptState::default(),
    }
}

fn persist_to_disk(app: &tauri::AppHandle, state: &OneTimePromptState) -> Result<(), String> {
    let path = prompt_state_path(app);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create prompt state directory: {error}"))?;
    }
    let json = serde_json::to_string_pretty(state)
        .map_err(|error| format!("Failed to serialize prompt state: {error}"))?;
    std::fs::write(path, json).map_err(|error| format!("Failed to persist prompt state: {error}"))
}

pub(crate) fn initialize(app: &tauri::AppHandle) {
    let loaded = load_from_disk(app);
    if let Some(state) = app.try_state::<OneTimePromptStateStore>() {
        *state.lock().expect("one-time prompt state poisoned") = loaded;
    }
}

pub(crate) fn current_state(app: &tauri::AppHandle) -> OneTimePromptState {
    app.try_state::<OneTimePromptStateStore>()
        .map(|state| {
            state
                .lock()
                .expect("one-time prompt state poisoned")
                .clone()
        })
        .unwrap_or_else(|| load_from_disk(app))
}

fn mutate_prompt(
    app: tauri::AppHandle,
    prompt_id: String,
    mutate: impl FnOnce(&mut OneTimePromptRecord, String),
) -> Result<OneTimePromptState, String> {
    let prompt_id = valid_prompt_id(&prompt_id)?;
    let state = app.state::<OneTimePromptStateStore>();
    let mut guard = state.lock().expect("one-time prompt state poisoned");
    mutate_prompt_state(&mut guard, prompt_id, now_rfc3339(), mutate, |next| {
        persist_to_disk(&app, next)
    })
}

fn mutate_prompt_state(
    state: &mut OneTimePromptState,
    prompt_id: String,
    now: String,
    mutate: impl FnOnce(&mut OneTimePromptRecord, String),
    persist: impl FnOnce(&OneTimePromptState) -> Result<(), String>,
) -> Result<OneTimePromptState, String> {
    let mut next = state.clone();
    let record = next.prompts.entry(prompt_id).or_default();
    mutate(record, now);
    persist(&next)?;
    *state = next.clone();
    Ok(next)
}

#[tauri::command]
pub fn get_one_time_prompt_state(app: tauri::AppHandle) -> OneTimePromptState {
    current_state(&app)
}

#[tauri::command]
pub fn mark_one_time_prompt_shown(
    prompt_id: String,
    app: tauri::AppHandle,
) -> Result<OneTimePromptState, String> {
    mutate_prompt(app, prompt_id, |record, now| {
        if record.shown_at.is_none() {
            record.shown_at = Some(now);
        }
    })
}

#[tauri::command]
pub fn dismiss_one_time_prompt(
    prompt_id: String,
    app: tauri::AppHandle,
) -> Result<OneTimePromptState, String> {
    mutate_prompt(app, prompt_id, |record, now| {
        record.dismissed_at = Some(now)
    })
}

#[tauri::command]
pub fn complete_one_time_prompt(
    prompt_id: String,
    app: tauri::AppHandle,
) -> Result<OneTimePromptState, String> {
    mutate_prompt(app, prompt_id, |record, now| {
        record.completed_at = Some(now)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutate_prompt_state_commits_after_successful_persist() {
        let mut state = OneTimePromptState::default();

        let result = mutate_prompt_state(
            &mut state,
            "privacy/v1".to_string(),
            "2026-05-19T10:00:00Z".to_string(),
            |record, now| record.dismissed_at = Some(now),
            |_| Ok(()),
        )
        .expect("mutation should succeed");

        assert_eq!(
            result
                .prompts
                .get("privacy/v1")
                .and_then(|record| record.dismissed_at.as_deref()),
            Some("2026-05-19T10:00:00Z")
        );
        assert_eq!(state, result);
    }

    #[test]
    fn mutate_prompt_state_does_not_commit_after_failed_persist() {
        let mut state = OneTimePromptState::default();

        let result = mutate_prompt_state(
            &mut state,
            "privacy/v1".to_string(),
            "2026-05-19T10:00:00Z".to_string(),
            |record, now| record.completed_at = Some(now),
            |_| Err("disk is unavailable".to_string()),
        );

        assert_eq!(result, Err("disk is unavailable".to_string()));
        assert!(state.prompts.is_empty());
    }
}
