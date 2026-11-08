use std::sync::Arc;

use app_core::one_time_prompts::{self as core_prompts, OneTimePromptState};
use app_core::AppCore;
use tauri::State;

#[tauri::command]
pub fn get_one_time_prompt_state(core: State<'_, Arc<AppCore>>) -> OneTimePromptState {
    core_prompts::get_one_time_prompt_state(&core)
}

#[tauri::command]
pub fn mark_one_time_prompt_shown(
    prompt_id: String,
    core: State<'_, Arc<AppCore>>,
) -> Result<OneTimePromptState, String> {
    core_prompts::mark_one_time_prompt_shown(prompt_id, &core)
}

#[tauri::command]
pub fn dismiss_one_time_prompt(
    prompt_id: String,
    core: State<'_, Arc<AppCore>>,
) -> Result<OneTimePromptState, String> {
    core_prompts::dismiss_one_time_prompt(prompt_id, &core)
}

#[tauri::command]
pub fn complete_one_time_prompt(
    prompt_id: String,
    core: State<'_, Arc<AppCore>>,
) -> Result<OneTimePromptState, String> {
    core_prompts::complete_one_time_prompt(prompt_id, &core)
}
