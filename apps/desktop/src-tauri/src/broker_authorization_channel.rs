use std::sync::Arc;

use app_core::broker_authorization_channel::{
    self as channel, ApproveCliAccessRequest, PendingCliAccessRequestDto,
};
use app_core::AppCore;
use tauri::State;

#[tauri::command]
pub fn get_pending_cli_access_request(
    core: State<'_, Arc<AppCore>>,
) -> Option<PendingCliAccessRequestDto> {
    channel::get_pending_cli_access_request(&core)
}

#[tauri::command]
pub fn approve_pending_cli_access_request(
    core: State<'_, Arc<AppCore>>,
    approval: ApproveCliAccessRequest,
) -> Result<(), String> {
    channel::approve_pending_cli_access_request(&core, approval)
}

#[tauri::command]
pub fn block_pending_cli_access_request(
    core: State<'_, Arc<AppCore>>,
    request_id: String,
) -> Result<(), String> {
    channel::block_pending_cli_access_request(&core, request_id)
}

#[tauri::command]
pub fn cancel_pending_cli_access_request(core: State<'_, Arc<AppCore>>) -> Result<(), String> {
    channel::cancel_pending_cli_access_request(&core)
}
