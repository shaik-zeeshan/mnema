use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const ONE_TIME_PROMPTS_FILE_NAME: &str = "one-time-prompts.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OneTimePromptState {
    #[serde(default)]
    pub prompts: BTreeMap<String, OneTimePromptRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
        .map(|state| state.lock().expect("one-time prompt state poisoned").clone())
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
    let record = guard.prompts.entry(prompt_id).or_default();
    mutate(record, now_rfc3339());
    persist_to_disk(&app, &guard)?;
    Ok(guard.clone())
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
    mutate_prompt(app, prompt_id, |record, now| record.dismissed_at = Some(now))
}

#[tauri::command]
pub fn complete_one_time_prompt(
    prompt_id: String,
    app: tauri::AppHandle,
) -> Result<OneTimePromptState, String> {
    mutate_prompt(app, prompt_id, |record, now| record.completed_at = Some(now))
}
