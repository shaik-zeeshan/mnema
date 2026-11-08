//! Reading and writing the standing permission rows behind an approval.

use app_infra::brokered_access::{
    BrokerClientIdentity, BrokerClientIdentitySource, BrokerGrantFile, BrokerGrantScope,
    BrokeredCaptureAccess,
};

use super::{
    blocked_response, AuthorizationChannelGrant, AuthorizationChannelRequest,
    AuthorizationChannelResponse,
};
use crate::AppCore;

/// The scope this client's standing permission already carries.
///
/// An approval SETS the row's scope, and the window pre-selects the request's
/// `minimum` — so on any door whose minimum lands BELOW what the row already
/// holds (`mnema access request --scope last-day` against an `allRetained` row, or
/// a `show-text` on a deleted result id, whose ask derives to `lastDay`), clicking
/// Allow takes access away. Making the window say what the tool already has is
/// what keeps that from being silent; the CLI's widen door additionally refuses to
/// ask for less than the call needs.
///
/// A blocked row is not access (the upsert leaves it blocked and refuses the
/// approval anyway), and an unreadable permission file just leaves the line
/// unsaid — the worst case is the window telling the user less than it could.
pub(super) fn current_scope_for_client(core: &AppCore, label: &str) -> Option<String> {
    let config_dir = core.paths().config.clone();
    let grants = BrokeredCaptureAccess::from_config_dir(config_dir)
        .list_grants()
        .ok()?;
    current_scope_in(&grants, label)
}

/// The floor the approval window enforces and pre-selects for this request.
///
/// The wire `minimum` is the requester's own claim, and the window DISABLES every
/// option below it — so a client declaring `allRetained` makes the entire
/// retained history the only answer the consent sheet can give, under this app's
/// own copy asserting the tool requires it. ADR 0059 pins that per door: on a
/// WIDEN the floor is the scope the call needs (the row already stands narrower,
/// and an approval SETS the scope, so a narrower answer both fails the call and
/// takes away access the tool already had), but on a FIRST GRANT it stays at the
/// narrowest band so a never-seen tool cannot make "All retained" the only thing
/// the user can grant on its first prompt.
///
/// `authorize_wider_access` applies that rule in the CLI — and the CLI is not in
/// this path. Anything that can open the socket writes this field itself, so the
/// rule is re-derived here, where it is actually enforced. `preferred` is left
/// alone: the window still names what the tool asked for, offered rather than
/// forced.
pub(super) fn enforced_minimum_scope(
    requested_minimum: &str,
    current_scope: Option<&str>,
) -> String {
    match current_scope {
        // A widen: this client already holds a row, so the floor it states is the
        // scope its call needs and every narrower answer is a dead end that also
        // narrows the row.
        Some(_) => requested_minimum.to_string(),
        // A first grant: the narrowest band, always.
        None => BrokerGrantScope::LAST_DAY.wire_name().to_string(),
    }
}

pub(super) fn current_scope_in(grants: &BrokerGrantFile, label: &str) -> Option<String> {
    let normalized = app_infra::brokered_access::normalize_client_label(label)?;
    grants
        .grants
        .iter()
        .find(|grant| !grant.blocked && grant.normalized_label.eq_ignore_ascii_case(&normalized))
        .map(|grant| grant.scope.wire_name().to_string())
}

/// Is there a standing block for this client? Read before anything else in
/// `handle_connection`, so a blocked tool never opens a window and never sees a
/// retryable reason code.
///
/// Failures (no config dir, an unreadable permission file) resolve to "not
/// blocked": the cost is that the user is asked, and the window still lets them
/// refuse.
#[cfg(unix)]
pub(super) fn client_is_blocked(core: &AppCore, label: &str) -> bool {
    let config_dir = core.paths().config.clone();
    let Ok(grants) = BrokeredCaptureAccess::from_config_dir(config_dir).list_grants() else {
        return false;
    };
    grant_file_blocks(&grants, label)
}

pub(super) fn grant_file_blocks(grants: &BrokerGrantFile, label: &str) -> bool {
    let Some(normalized) = app_infra::brokered_access::normalize_client_label(label) else {
        return false;
    };
    grants
        .grants
        .iter()
        .any(|grant| grant.blocked && grant.normalized_label.eq_ignore_ascii_case(&normalized))
}

pub(super) fn identity_for_request(
    request: &AuthorizationChannelRequest,
) -> Result<BrokerClientIdentity, String> {
    BrokerClientIdentity::new(
        request.client.label.clone(),
        match request.client.source.as_str() {
            "explicit" => BrokerClientIdentitySource::Explicit,
            "env" => BrokerClientIdentitySource::Env,
            "inferred" => BrokerClientIdentitySource::Inferred,
            _ => BrokerClientIdentitySource::Defaulted,
        },
    )
    .map_err(|error| error.to_string())
}

pub(super) fn create_grant_response(
    core: &AppCore,
    request: &AuthorizationChannelRequest,
    scope: BrokerGrantScope,
) -> Result<AuthorizationChannelResponse, String> {
    create_grant_response_in(&core.paths().config, request, scope)
}

/// Upsert this identity's standing permission and describe what it now grants.
///
/// The upsert SETS the row's scope to the one passed in — it is not a widen, so
/// the row can come out of an approval narrower than it went in. The response
/// reports the scope the ROW carries afterwards, which is the scope the user
/// picked, not the scope the client asked for: the picker can select a scope
/// NARROWER than the preferred one (never below the request's minimum, which
/// `validate_cli_access_approval` enforces), so the caller MUST check the
/// returned grant covers its request instead of treating `approved` as a yes.
/// The row keeps its id across the upsert, so opaque ids already handed to a
/// running agent keep resolving.
pub(super) fn create_grant_response_in(
    config_dir: &std::path::Path,
    request: &AuthorizationChannelRequest,
    scope: BrokerGrantScope,
) -> Result<AuthorizationChannelResponse, String> {
    let identity = identity_for_request(request)?;
    let upsert = BrokeredCaptureAccess::from_config_dir(config_dir)
        .upsert_grant_for_identity(identity, scope)
        .map_err(|error| error.to_string())?;
    if upsert.grant.blocked {
        // Blocked in Settings while this window sat open. The block check that
        // let the connection through ran back when the socket connected, so the
        // rejection is the NEWER decision — the upsert refused to clear it, and
        // the client is told so instead of getting an approval receipt for a
        // permission the file does not carry.
        return Ok(blocked_response(request.request_id.clone()));
    }
    Ok(AuthorizationChannelResponse {
        schema_version: 1,
        request_id: request.request_id.clone(),
        decision: "approved".to_string(),
        reason: None,
        grant: Some(AuthorizationChannelGrant {
            id: upsert.grant.id,
            client_label: upsert.grant.label,
            scope: upsert.grant.scope.wire_name().to_string(),
            created: upsert.created,
        }),
    })
}
