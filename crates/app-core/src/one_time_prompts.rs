use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::AppCore;

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

fn prompt_state_path(core: &AppCore) -> PathBuf {
    core.paths().config.join(ONE_TIME_PROMPTS_FILE_NAME)
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

fn load_from_disk(core: &AppCore) -> OneTimePromptState {
    let path = prompt_state_path(core);
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => OneTimePromptState::default(),
    }
}

fn persist_to_disk(core: &AppCore, state: &OneTimePromptState) -> Result<(), String> {
    let path = prompt_state_path(core);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create prompt state directory: {error}"))?;
    }
    let json = serde_json::to_string_pretty(state)
        .map_err(|error| format!("Failed to serialize prompt state: {error}"))?;
    std::fs::write(path, json).map_err(|error| format!("Failed to persist prompt state: {error}"))
}

pub fn initialize(core: &AppCore) {
    let loaded = load_from_disk(core);
    if let Some(state) = core.try_state::<OneTimePromptStateStore>() {
        *state.lock().expect("one-time prompt state poisoned") = loaded;
    }
}

pub fn current_state(core: &AppCore) -> OneTimePromptState {
    core.try_state::<OneTimePromptStateStore>()
        .map(|state| {
            state
                .lock()
                .expect("one-time prompt state poisoned")
                .clone()
        })
        .unwrap_or_else(|| load_from_disk(core))
}

fn mutate_prompt(
    core: &AppCore,
    prompt_id: String,
    mutate: impl FnOnce(&mut OneTimePromptRecord, String),
) -> Result<OneTimePromptState, String> {
    let prompt_id = valid_prompt_id(&prompt_id)?;
    let state = core.state::<OneTimePromptStateStore>();
    let mut guard = state.lock().expect("one-time prompt state poisoned");
    mutate_prompt_state(&mut guard, prompt_id, now_rfc3339(), mutate, |next| {
        persist_to_disk(core, next)
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

pub fn get_one_time_prompt_state(core: &AppCore) -> OneTimePromptState {
    current_state(core)
}

pub fn mark_one_time_prompt_shown(
    prompt_id: String,
    core: &AppCore,
) -> Result<OneTimePromptState, String> {
    mutate_prompt(core, prompt_id, |record, now| {
        if record.shown_at.is_none() {
            record.shown_at = Some(now);
        }
    })
}

pub fn dismiss_one_time_prompt(
    prompt_id: String,
    core: &AppCore,
) -> Result<OneTimePromptState, String> {
    mutate_prompt(core, prompt_id, |record, now| {
        record.dismissed_at = Some(now)
    })
}

pub fn complete_one_time_prompt(
    prompt_id: String,
    core: &AppCore,
) -> Result<OneTimePromptState, String> {
    mutate_prompt(core, prompt_id, |record, now| {
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

    #[test]
    fn marks_persist_under_the_config_dir_and_reload_on_initialize() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::test_support::core_in(Default::default(), dir.path());
        core.manage(OneTimePromptStateStore::default());

        let state = mark_one_time_prompt_shown("privacy/v1".to_string(), &core).unwrap();
        let shown = state.prompts["privacy/v1"].shown_at.clone();
        assert!(shown.is_some());
        let again = mark_one_time_prompt_shown("privacy/v1".to_string(), &core).unwrap();
        assert_eq!(
            again.prompts["privacy/v1"].shown_at, shown,
            "first shown wins"
        );
        assert!(dir
            .path()
            .join("config")
            .join(ONE_TIME_PROMPTS_FILE_NAME)
            .is_file());
        assert_eq!(
            dismiss_one_time_prompt(" ".to_string(), &core),
            Err("Prompt id is required".to_string())
        );

        let fresh = crate::test_support::core_in(Default::default(), dir.path());
        assert_eq!(
            current_state(&fresh),
            again,
            "unmanaged reads fall back to disk"
        );
        fresh.manage(OneTimePromptStateStore::default());
        initialize(&fresh);
        assert_eq!(get_one_time_prompt_state(&fresh), again);
    }
}
