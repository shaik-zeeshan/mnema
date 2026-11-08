//! Settings › CLI Access: the standing per-tool permissions (ADR 0059), their
//! history, and the bundled CLI's install state.

mod install;

pub use install::{get_cli_status_inner, install_cli_inner, MnemaCliStatus};

use crate::AppCore;

fn access(core: &AppCore) -> ::app_infra::brokered_access::BrokeredCaptureAccess {
    ::app_infra::brokered_access::BrokeredCaptureAccess::from_config_dir(
        core.paths().config.clone(),
    )
}

pub async fn list_cli_access_grants(
    core: &AppCore,
) -> Result<::app_infra::brokered_access::BrokerGrantFile, String> {
    access(core)
        .list_grants()
        .map_err(|error| format!("failed to load CLI Access grants: {error}"))
}

/// Block a tool: a standing rejection. The row stays visible in Settings, is
/// denied without ever opening the approval window, and never idle-expires
/// (ADR 0059). Returns whether a row actually changed.
pub async fn block_cli_access_client(core: &AppCore, client_name: String) -> Result<bool, String> {
    access(core)
        .block_client(&client_name)
        .map_err(|error| format!("failed to block CLI Access for this tool: {error}"))
}

pub async fn unblock_cli_access_client(
    core: &AppCore,
    client_name: String,
) -> Result<bool, String> {
    access(core)
        .unblock_client(&client_name)
        .map_err(|error| format!("failed to re-enable CLI Access for this tool: {error}"))
}

pub async fn list_cli_access_history(
    core: &AppCore,
) -> Result<::app_infra::brokered_access::BrokerAuditFile, String> {
    access(core)
        .list_history()
        .map_err(|error| format!("failed to load CLI Access history: {error}"))
}
