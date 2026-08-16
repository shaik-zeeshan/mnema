#[cfg(unix)]
use std::os::unix::net::UnixListener as StdUnixListener;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use app_infra::brokered_access::{
    BrokerClientIdentity, BrokerClientIdentitySource, BrokerGrantFile, BrokerGrantScope,
    BrokeredCaptureAccess,
};
use serde::{Deserialize, Serialize};
use tauri::Manager;
use tokio::sync::oneshot;
#[cfg(unix)]
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener as TokioUnixListener, UnixStream},
    time::timeout,
};

use crate::windows;

/// Cap on the client-supplied name shown in the approval window, so it can't push
/// the rest of the consent copy off the sheet.
const CLIENT_LABEL_MAX_CHARS: usize = 64;
#[cfg(unix)]
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const REQUEST_MAX_BYTES: usize = 64 * 1024;

#[derive(Clone, Default)]
pub struct BrokerAuthorizationChannelState {
    active: Arc<AtomicBool>,
    pending: Arc<Mutex<Option<PendingAuthorizationRequest>>>,
}

struct PendingAuthorizationRequest {
    request: AuthorizationChannelRequest,
    respond: oneshot::Sender<AuthorizationChannelResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationChannelRequest {
    pub schema_version: u32,
    pub request_id: String,
    pub client: AuthorizationChannelClient,
    pub command: String,
    pub scope: AuthorizationChannelScope,
    pub interactive: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationChannelClient {
    pub label: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationChannelScope {
    pub minimum: String,
    pub preferred: String,
}

/// One response shape for every outcome. `decision` is the branch the caller
/// switches on; `reason` names *which* of that branch's cases fired.
///
/// - `approved` — `grant` is present and states the scope actually granted. The
///   caller MUST check it covers what it asked for: an existing permission is
///   widened, never narrowed, but a user can approve exactly the minimum.
/// - `denied` + `userCancelled` — the user denied, or the window was closed.
/// - `denied` + `closed` — the approval waiter was dropped without a verdict.
/// - `blocked` + `blocked` — a standing user rejection for this client. No
///   window opened and none will; only Settings can lift it. Do not retry.
/// - `unavailable` + `busy` — another approval is already in flight. The app IS
///   running; do not relaunch it.
/// - `unavailable` + `onboardingRequired` — onboarding is not finished.
/// - `unavailable` + `invalidRequest` / `unsupportedVersion` — the request did
///   not parse, was oversized, or carried a `schemaVersion` other than 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationChannelResponse {
    pub schema_version: u32,
    pub request_id: String,
    pub decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant: Option<AuthorizationChannelGrant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationChannelGrant {
    pub id: String,
    pub client_label: String,
    /// The scope the standing permission now carries — `lastDay` | `last7Days` |
    /// `allRetained`. There is no expiry: the row lives until it idles out or is
    /// blocked in Settings.
    pub scope: String,
    /// `true` when this approval created the permission, `false` when it widened
    /// an existing one (which keeps its id, so opaque ids stay valid).
    pub created: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingCliAccessRequestDto {
    pub request_id: String,
    pub client: AuthorizationChannelClient,
    pub command: String,
    pub minimum_scope: String,
    pub preferred_scope: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveCliAccessRequest {
    pub scope: String,
}

struct ActiveRequestGuard {
    active: Arc<AtomicBool>,
}

impl ActiveRequestGuard {
    fn acquire(active: Arc<AtomicBool>) -> Option<Self> {
        (!active.swap(true, Ordering::SeqCst)).then_some(Self { active })
    }
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
    }
}

pub fn start(app: &tauri::AppHandle) -> Result<(), String> {
    #[cfg(not(unix))]
    {
        let _ = app;
        return Ok(());
    }

    #[cfg(unix)]
    {
        let socket_path = app
            .path()
            .app_config_dir()
            .map(|dir| socket_path_for_config_dir(&dir))
            .unwrap_or_else(|_| socket_path_for_identifier(app.config().identifier.as_str()));
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create CLI access socket dir: {error}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }
        if socket_path.exists() && stale_socket(&socket_path) {
            let _ = std::fs::remove_file(&socket_path);
        }
        let listener = StdUnixListener::bind(&socket_path)
            .map_err(|error| format!("failed to bind CLI access socket: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("failed to configure CLI access socket: {error}"))?;
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let listener = match TokioUnixListener::from_std(listener) {
                Ok(listener) => listener,
                Err(error) => {
                    tauri_plugin_log::log::error!(
                        "failed to initialize CLI access socket listener: {error}"
                    );
                    return;
                }
            };
            loop {
                let Ok((stream, _addr)) = listener.accept().await else {
                    continue;
                };
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    handle_connection(app, stream).await;
                });
            }
        });
        Ok(())
    }
}

#[cfg(unix)]
async fn handle_connection(app: tauri::AppHandle, mut stream: UnixStream) {
    let raw = match timeout(REQUEST_READ_TIMEOUT, read_request_line(&mut stream)).await {
        Ok(Ok(Some(raw))) => raw,
        Ok(Ok(None)) | Err(_) => return,
        Ok(Err(_)) => {
            let _ = write_unavailable(stream, String::new(), "invalidRequest").await;
            return;
        }
    };
    let request = match serde_json::from_str::<AuthorizationChannelRequest>(&raw) {
        Ok(request) if request.schema_version == 1 => request,
        Ok(request) => {
            let _ = write_unavailable(stream, request.request_id, "unsupportedVersion").await;
            return;
        }
        Err(_) => {
            let _ = write_unavailable(stream, String::new(), "invalidRequest").await;
            return;
        }
    };

    // Blocked is a STANDING rejection, so it is answered before anything else —
    // ahead of the single-flight guard, so a blocked client can never be told
    // `busy` (a code that invites a retry) and can never queue behind an
    // unrelated approval. No window opens; only Settings lifts it.
    if client_is_blocked(&app, &request.client.label) {
        let _ = write_blocked(stream, request.request_id).await;
        return;
    }

    let state = app.state::<BrokerAuthorizationChannelState>();
    let Some(_guard) = ActiveRequestGuard::acquire(state.active.clone()) else {
        let _ = write_unavailable(stream, request.request_id, "busy").await;
        return;
    };

    let onboarding_state = app.state::<windows::OnboardingStateStore>();
    if !windows::current_onboarding_state_for_app(&app, onboarding_state.inner()).is_complete() {
        let _ = write_unavailable(stream, request.request_id, "onboardingRequired").await;
        return;
    }

    // The window is the only consent surface: approval now fires a few times in
    // a tool's life, so a native fast path bought nothing and cost the identity
    // provenance chip and the anti-reflex affordances (focus on Deny, Enter
    // unbound, Esc denies).
    let (send, receive) = oneshot::channel();
    if !store_pending_request(&app, request.clone(), send) {
        let _ = write_unavailable(stream, request.request_id, "busy").await;
        return;
    }
    let _ = windows::open_cli_access_request_window(&app);
    match receive.await {
        Ok(response) => {
            let _ = write_response(stream, response).await;
        }
        Err(_) => {
            let _ = write_denied(stream, request.request_id, "closed").await;
        }
    }
}

#[cfg(unix)]
async fn read_request_line(stream: &mut UnixStream) -> std::io::Result<Option<String>> {
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        if raw.len() >= REQUEST_MAX_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "CLI Access request is too large",
            ));
        }
        let remaining = REQUEST_MAX_BYTES - raw.len();
        let read_len = remaining.min(buffer.len());
        let bytes = stream.read(&mut buffer[..read_len]).await?;
        if bytes == 0 {
            if raw.is_empty() {
                return Ok(None);
            }
            break;
        }
        if let Some(position) = buffer[..bytes].iter().position(|byte| *byte == b'\n') {
            raw.extend_from_slice(&buffer[..=position]);
            break;
        }
        raw.extend_from_slice(&buffer[..bytes]);
    }
    String::from_utf8(raw)
        .map(Some)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

fn store_pending_request(
    app: &tauri::AppHandle,
    request: AuthorizationChannelRequest,
    respond: oneshot::Sender<AuthorizationChannelResponse>,
) -> bool {
    let state = app.state::<BrokerAuthorizationChannelState>();
    let Ok(mut pending) = state.pending.lock() else {
        return false;
    };
    if pending.is_some() {
        return false;
    }
    *pending = Some(PendingAuthorizationRequest { request, respond });
    true
}

fn take_pending_request(app: &tauri::AppHandle) -> Option<PendingAuthorizationRequest> {
    app.state::<BrokerAuthorizationChannelState>()
        .pending
        .lock()
        .ok()
        .and_then(|mut pending| pending.take())
}

#[tauri::command]
pub fn get_pending_cli_access_request(app: tauri::AppHandle) -> Option<PendingCliAccessRequestDto> {
    app.state::<BrokerAuthorizationChannelState>()
        .pending
        .lock()
        .ok()
        .and_then(|pending| pending.as_ref().map(|pending| pending.request.clone()))
        .map(|request| PendingCliAccessRequestDto {
            request_id: request.request_id,
            client: AuthorizationChannelClient {
                // The window is now the ONLY consent surface, and it renders this
                // name straight into the requester chip. Sanitize on the way out.
                label: client_label_display(&request.client.label),
                source: request.client.source,
            },
            command: request.command,
            minimum_scope: request.scope.minimum,
            preferred_scope: request.scope.preferred,
            created_at: request.created_at,
        })
}

#[tauri::command]
pub fn approve_pending_cli_access_request(
    app: tauri::AppHandle,
    approval: ApproveCliAccessRequest,
) -> Result<(), String> {
    let pending = take_pending_request_for_approval(&app, &approval)?;
    let mut request = pending.request;
    request.scope.preferred = approval.scope;
    let request_id = request.request_id.clone();
    let response = BrokerGrantScope::from_wire_name(&request.scope.preferred)
        .ok_or_else(|| "unknown scope".to_string())
        .and_then(|scope| create_grant_response(&app, &request, scope))
        .unwrap_or_else(|_| AuthorizationChannelResponse {
            schema_version: 1,
            request_id,
            decision: "unavailable".to_string(),
            reason: Some("invalidRequest".to_string()),
            grant: None,
        });
    let _ = pending.respond.send(response);
    let _ = close_cli_access_request_window(&app);
    Ok(())
}

fn take_pending_request_for_approval(
    app: &tauri::AppHandle,
    approval: &ApproveCliAccessRequest,
) -> Result<PendingAuthorizationRequest, String> {
    let state = app.state::<BrokerAuthorizationChannelState>();
    let Ok(mut pending) = state.pending.lock() else {
        return Err("no pending CLI Access request".to_string());
    };
    take_validated_pending_request(&mut pending, approval)
}

fn take_validated_pending_request(
    pending: &mut Option<PendingAuthorizationRequest>,
    approval: &ApproveCliAccessRequest,
) -> Result<PendingAuthorizationRequest, String> {
    let Some(current) = pending.as_ref() else {
        return Err("no pending CLI Access request".to_string());
    };
    validate_cli_access_approval(&current.request, approval)?;
    pending
        .take()
        .ok_or_else(|| "no pending CLI Access request".to_string())
}

fn validate_cli_access_approval(
    request: &AuthorizationChannelRequest,
    approval: &ApproveCliAccessRequest,
) -> Result<(), String> {
    if !scope_satisfies_minimum(&approval.scope, &request.scope.minimum) {
        return Err("selected scope does not satisfy the pending command".to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn cancel_pending_cli_access_request(app: tauri::AppHandle) -> Result<(), String> {
    cancel_pending_request(&app, "userCancelled");
    let _ = close_cli_access_request_window(&app);
    Ok(())
}

pub fn cancel_pending_request(app: &tauri::AppHandle, reason: &str) {
    let Some(pending) = take_pending_request(app) else {
        return;
    };
    let _ = pending.respond.send(AuthorizationChannelResponse {
        schema_version: 1,
        request_id: pending.request.request_id,
        decision: "denied".to_string(),
        reason: Some(reason.to_string()),
        grant: None,
    });
}

fn close_cli_access_request_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("cli-access-request") {
        window.close().map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Does the selected scope cover the request's minimum? Both sides are wire
/// spellings, so both route through [`BrokerGrantScope`] rather than being
/// compared as strings — an unrecognized spelling is a rejection, never a
/// coincidental match.
fn scope_satisfies_minimum(selected: &str, minimum: &str) -> bool {
    match (
        BrokerGrantScope::from_wire_name(selected),
        BrokerGrantScope::from_wire_name(minimum),
    ) {
        (Some(selected), Some(minimum)) => selected.covers(&minimum),
        _ => false,
    }
}

/// The client name is wire input from whoever connected to the socket, and it
/// lands in the approval window's requester chip. Control characters and
/// unbounded length would let that name restructure the sheet — fabricating the
/// app's own copy, or pushing the consent controls out of view. Same
/// normalization app-infra applies to a stored grant label
/// (`display_client_label`), plus a cap.
/// Characters that render as nothing, or that reorder the text around them.
///
/// `char::is_control()` covers only the C0/C1 blocks (Unicode `Cc`), so bidi
/// overrides/isolates (U+202E, U+2066..=U+2069) and zero-width characters (U+200B,
/// U+FEFF, tag characters) sail straight past it — they are `Cf`, and none of them
/// is `White_Space`. In a consent prompt that is not cosmetic: a label made
/// entirely of zero-width characters is non-empty by `is_empty()`, so it defeats
/// the "An unnamed local tool" fallback below and the window names no requester at
/// all; and an override reverses the display order of the copy around it.
///
/// The set is Unicode's `Default_Ignorable_Code_Point` — the closed, stable list
/// of "this is meant to render as nothing", which is exactly the property that
/// matters here and covers the bidi controls, the zero-widths, the fillers
/// (U+3164 HANGUL FILLER is the classic invisible-name trick) and the variation
/// selectors in one go — plus the two blank-glyph outliers it leaves out:
/// U+2800 BRAILLE PATTERN BLANK and the U+FFF9..=U+FFFB annotation controls.
/// Enumerating "invisible" any other way is a blocklist that leaks; this one is
/// a Unicode property with a fixed definition.
fn is_invisible_or_reordering(ch: char) -> bool {
    matches!(ch,
        '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{2800}' | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}'
        | '\u{115F}'..='\u{1160}'
        | '\u{17B4}'..='\u{17B5}'
        | '\u{180B}'..='\u{180F}'
        | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}'
        | '\u{FE00}'..='\u{FE0F}'
        | '\u{FFF0}'..='\u{FFFB}'
        | '\u{1BCA0}'..='\u{1BCA3}'
        | '\u{1D173}'..='\u{1D17A}'
        | '\u{E0000}'..='\u{E0FFF}'
    )
}

fn client_label_display(label: &str) -> String {
    let cleaned = label
        .chars()
        .filter(|ch| !is_invisible_or_reordering(*ch))
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() {
        return "An unnamed local tool".to_string();
    }
    if cleaned.chars().count() <= CLIENT_LABEL_MAX_CHARS {
        return cleaned;
    }
    cleaned
        .chars()
        .take(CLIENT_LABEL_MAX_CHARS)
        .chain(std::iter::once('…'))
        .collect()
}

/// Is there a standing block for this client? Read before anything else in
/// `handle_connection`, so a blocked tool never opens a window and never sees a
/// retryable reason code.
///
/// Failures (no config dir, an unreadable permission file) resolve to "not
/// blocked": the cost is that the user is asked, and the window still lets them
/// refuse.
#[cfg(unix)]
fn client_is_blocked(app: &tauri::AppHandle, label: &str) -> bool {
    let Ok(config_dir) = app.path().app_config_dir() else {
        return false;
    };
    let Ok(grants) = BrokeredCaptureAccess::from_config_dir(config_dir).list_grants() else {
        return false;
    };
    grant_file_blocks(&grants, label)
}

fn grant_file_blocks(grants: &BrokerGrantFile, label: &str) -> bool {
    let Some(normalized) = app_infra::brokered_access::normalize_client_label(label) else {
        return false;
    };
    grants
        .grants
        .iter()
        .any(|grant| grant.blocked && grant.normalized_label.eq_ignore_ascii_case(&normalized))
}

fn identity_for_request(
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

fn create_grant_response(
    app: &tauri::AppHandle,
    request: &AuthorizationChannelRequest,
    scope: BrokerGrantScope,
) -> Result<AuthorizationChannelResponse, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("failed to resolve app config dir: {error}"))?;
    create_grant_response_in(&config_dir, request, scope)
}

/// Upsert this identity's standing permission and describe what it now grants.
///
/// The response reports the scope the ROW carries after the approval, which is
/// the scope the user picked — not the scope the client asked for. The picker
/// can set a scope NARROWER than the preferred one, so the caller MUST check the
/// returned grant covers its request instead of treating `approved` as a yes.
/// The row keeps its id across the upsert, so opaque ids already handed to a
/// running agent keep resolving.
fn create_grant_response_in(
    config_dir: &std::path::Path,
    request: &AuthorizationChannelRequest,
    scope: BrokerGrantScope,
) -> Result<AuthorizationChannelResponse, String> {
    let identity = identity_for_request(request)?;
    let upsert = BrokeredCaptureAccess::from_config_dir(config_dir)
        .upsert_grant_for_identity(identity, scope)
        .map_err(|error| error.to_string())?;
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

#[cfg(unix)]
async fn write_blocked(stream: UnixStream, request_id: String) -> std::io::Result<()> {
    write_response(
        stream,
        AuthorizationChannelResponse {
            schema_version: 1,
            request_id,
            decision: "blocked".to_string(),
            reason: Some("blocked".to_string()),
            grant: None,
        },
    )
    .await
}

#[cfg(unix)]
async fn write_denied(stream: UnixStream, request_id: String, reason: &str) -> std::io::Result<()> {
    write_response(
        stream,
        AuthorizationChannelResponse {
            schema_version: 1,
            request_id,
            decision: "denied".to_string(),
            reason: Some(reason.to_string()),
            grant: None,
        },
    )
    .await
}

#[cfg(unix)]
async fn write_unavailable(
    stream: UnixStream,
    request_id: String,
    reason: &str,
) -> std::io::Result<()> {
    write_response(
        stream,
        AuthorizationChannelResponse {
            schema_version: 1,
            request_id,
            decision: "unavailable".to_string(),
            reason: Some(reason.to_string()),
            grant: None,
        },
    )
    .await
}

#[cfg(unix)]
async fn write_response(
    mut stream: UnixStream,
    response: AuthorizationChannelResponse,
) -> std::io::Result<()> {
    let raw = serde_json::to_string(&response)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    stream.write_all(format!("{raw}\n").as_bytes()).await
}

#[cfg(unix)]
fn stale_socket(path: &PathBuf) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_err()
}

pub fn socket_path_for_identifier(identifier: &str) -> PathBuf {
    default_app_config_dir_for_identifier(identifier)
        .unwrap_or_else(|| std::env::temp_dir().join(identifier))
        .join("cli-access.sock")
}

fn socket_path_for_config_dir(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("cli-access.sock")
}

fn default_app_config_dir_for_identifier(identifier: &str) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("MNEMA_APP_CONFIG_DIR") {
        return Some(PathBuf::from(path));
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(PathBuf::from).map(|home| {
            home.join("Library")
                .join("Application Support")
                .join(identifier)
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|dir| dir.join(identifier))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_infra::brokered_access::BrokerGrant;
    use tokio::sync::oneshot::error::TryRecvError;

    fn test_authorization_request(minimum_scope: &str) -> AuthorizationChannelRequest {
        AuthorizationChannelRequest {
            schema_version: 1,
            request_id: "request-1".to_string(),
            client: AuthorizationChannelClient {
                label: "Test Client".to_string(),
                source: "explicit".to_string(),
            },
            command: "search".to_string(),
            scope: AuthorizationChannelScope {
                minimum: minimum_scope.to_string(),
                preferred: minimum_scope.to_string(),
            },
            interactive: true,
            created_at: "2026-05-23T00:00:00Z".to_string(),
        }
    }

    fn request_preferring(preferred_scope: &str) -> AuthorizationChannelRequest {
        let mut request = test_authorization_request("lastDay");
        request.scope.preferred = preferred_scope.to_string();
        request
    }

    #[test]
    fn socket_path_uses_configured_identifier() {
        let path = socket_path_for_identifier("com.example.mnema-test");
        assert!(path.ends_with("com.example.mnema-test/cli-access.sock"));
    }

    /// Every scope pair the wire can send, routed through `BrokerGrantScope` so
    /// the window, the CLI and the permission row cannot disagree about which
    /// spelling outranks which. `last7Days` is the band a string compare missed.
    #[test]
    fn scope_ranking_matches_the_permission_scopes() {
        assert!(scope_satisfies_minimum("allRetained", "lastDay"));
        assert!(scope_satisfies_minimum("allRetained", "last7Days"));
        assert!(scope_satisfies_minimum("last7Days", "lastDay"));
        assert!(scope_satisfies_minimum("lastDay", "lastDay"));
        assert!(!scope_satisfies_minimum("lastDay", "allRetained"));
        assert!(!scope_satisfies_minimum("lastDay", "last7Days"));
        assert!(!scope_satisfies_minimum("last7Days", "allRetained"));
        // An unrecognized spelling is never a coincidental match.
        assert!(!scope_satisfies_minimum("forever", "forever"));
    }

    fn blocked_grant(label: &str) -> BrokerGrant {
        BrokerGrant {
            id: "grant-1".to_string(),
            label: label.to_string(),
            normalized_label: app_infra::brokered_access::normalize_client_label(label)
                .expect("test label normalizes"),
            identity_source: BrokerClientIdentitySource::Explicit,
            created_at_unix_ms: 1,
            last_used_at_unix_ms: 1,
            scope: BrokerGrantScope::LAST_DAY,
            blocked: true,
            blocked_at_unix_ms: Some(2),
        }
    }

    /// A blocked client is a STANDING rejection: the connection is answered from
    /// the permission file alone, so no approval window ever opens and the user
    /// is never asked again. This is the check `handle_connection` runs before it
    /// even takes the single-flight guard.
    #[test]
    fn a_blocked_client_is_refused_from_the_permission_file_alone() {
        let grants = BrokerGrantFile {
            schema_version: 1,
            grants: vec![blocked_grant("Claude Code")],
        };

        // Matched by the same normalization the permission row is keyed on, so
        // case and separator variants of the name stay blocked.
        assert!(grant_file_blocks(&grants, "Claude Code"));
        assert!(grant_file_blocks(&grants, "claude-code"));
        assert!(!grant_file_blocks(&grants, "Some Other Tool"));

        let mut unblocked = grants.clone();
        unblocked.grants[0].blocked = false;
        assert!(!grant_file_blocks(&unblocked, "Claude Code"));
    }

    #[test]
    fn a_blocked_response_names_the_block_on_both_fields() {
        tauri::async_runtime::block_on(async {
            let (client, server) = UnixStream::pair().expect("socket pair should open");
            write_blocked(server, "request-1".to_string())
                .await
                .expect("blocked response should write");

            let response = read_response(client).await;
            assert_eq!(response.decision, "blocked");
            assert_eq!(response.reason.as_deref(), Some("blocked"));
            assert!(
                response.grant.is_none(),
                "a blocked client is granted nothing"
            );
        });
    }

    async fn read_response(mut stream: UnixStream) -> AuthorizationChannelResponse {
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .await
            .expect("response should read");
        serde_json::from_slice(&raw).expect("response should parse")
    }

    /// Widening a permission MUST keep the row's id: opaque result ids are
    /// HMAC-signed against the issuing grant id, so a new id would invalidate
    /// every id already handed to a running agent, mid-task. The response also
    /// has to report the scope the ROW now carries.
    #[test]
    fn approving_an_existing_client_reuses_the_row_and_keeps_its_id() {
        let config_dir = tempfile::tempdir().expect("temp config dir");
        let request = request_preferring("lastDay");

        let first =
            create_grant_response_in(config_dir.path(), &request, BrokerGrantScope::LAST_DAY)
                .expect("first approval should mint a permission");
        let first_grant = first.grant.expect("an approval carries its grant");
        assert!(first_grant.created, "the first approval creates the row");
        assert_eq!(first_grant.scope, "lastDay");

        let second = create_grant_response_in(
            config_dir.path(),
            &request,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("widening should succeed");
        let second_grant = second.grant.expect("an approval carries its grant");

        assert_eq!(
            second_grant.id, first_grant.id,
            "a widen must reuse the row, so ids already issued keep resolving"
        );
        assert!(
            !second_grant.created,
            "the second approval widened an existing row"
        );
        assert_eq!(second_grant.scope, "allRetained");
        assert_eq!(second.decision, "approved");

        let stored = BrokeredCaptureAccess::from_config_dir(config_dir.path())
            .list_grants()
            .expect("permissions should load");
        assert_eq!(stored.grants.len(), 1, "one row per client, never a second");
    }

    /// An approval SETS the row's scope, so it can hand back less than the client
    /// asked for — the user picks in the window, and the picker only enforces the
    /// request's minimum. That is exactly why the response has to state the scope
    /// and why the caller has to verify it covers the request rather than reading
    /// `approved` as a yes.
    #[test]
    fn an_approval_reports_the_scope_the_row_actually_carries() {
        let config_dir = tempfile::tempdir().expect("temp config dir");
        let request = request_preferring("allRetained");

        create_grant_response_in(
            config_dir.path(),
            &request,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("first approval should mint a permission");
        let narrowed =
            create_grant_response_in(config_dir.path(), &request, BrokerGrantScope::LAST_DAY)
                .expect("a narrower approval still succeeds")
                .grant
                .expect("an approval carries its grant");

        assert_eq!(
            narrowed.scope, "lastDay",
            "the response states what the row carries, not what was asked for"
        );
        let stored = BrokeredCaptureAccess::from_config_dir(config_dir.path())
            .list_grants()
            .expect("permissions should load");
        assert_eq!(stored.grants.len(), 1);
        assert_eq!(stored.grants[0].scope, BrokerGrantScope::LAST_DAY);
        assert_eq!(stored.grants[0].id, narrowed.id, "still the same row");
    }

    /// A name made only of invisible characters is not a name. `is_control()` does
    /// not cover them, so an all-zero-width label survives the empty check and the
    /// approval window renders with NO visible requester at all — defeating the
    /// fallback that exists to guarantee one.
    #[test]
    fn a_client_name_that_renders_as_nothing_falls_back_to_the_unnamed_wording() {
        for label in [
            "\u{200B}\u{200C}\u{200D}\u{FEFF}\u{2060}",
            "\u{3164}",                 // HANGUL FILLER
            "\u{2800}",                 // BRAILLE PATTERN BLANK
            "\u{FFF9}\u{FFFA}\u{FFFB}", // interlinear annotation controls
            "\u{1D173}\u{1D17A}",       // musical symbol format controls
            "\u{E0100}",                // variation selector supplement
        ] {
            assert_eq!(
                client_label_display(label),
                "An unnamed local tool",
                "a name that renders as nothing must fall back: {label:?}"
            );
        }
    }

    /// Bidi overrides and isolates reorder every character after them when the
    /// window renders, so a client name must not be able to smuggle one into the
    /// requester chip and restructure the consent copy around it.
    #[test]
    fn bidi_reordering_never_reaches_the_requester_chip() {
        let display = client_label_display("Mnema CLI\u{202E}\u{2066}\u{2067}");

        assert!(
            !display.contains(['\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}']),
            "no bidi embedding/override may reach the window: {display:?}"
        );
        assert!(
            !display.contains(['\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}']),
            "no bidi isolate may reach the window: {display:?}"
        );
        assert_eq!(display, "Mnema CLI");
    }

    /// The truncation boundary itself: a label of exactly
    /// [`CLIENT_LABEL_MAX_CHARS`] chars survives verbatim, and one char more is
    /// cut to exactly that many chars plus a single '…'. A label long enough to
    /// push the consent controls off the sheet is not a name.
    #[test]
    fn a_client_label_is_capped_before_it_reaches_the_window() {
        let at_cap = "A".repeat(CLIENT_LABEL_MAX_CHARS);
        assert_eq!(client_label_display(&at_cap), at_cap);

        let over_cap = "A".repeat(CLIENT_LABEL_MAX_CHARS + 1);
        let display = client_label_display(&over_cap);
        assert_eq!(
            display,
            format!("{}…", "A".repeat(CLIENT_LABEL_MAX_CHARS)),
            "one char over the cap truncates to the cap plus a single ellipsis"
        );
        assert_eq!(display.chars().count(), CLIENT_LABEL_MAX_CHARS + 1);

        // Newlines would let a name fabricate the window's own copy.
        assert_eq!(
            client_label_display("Mnema Helper\n\nAllow grants everything"),
            "Mnema Helper Allow grants everything"
        );
    }

    #[test]
    fn request_line_reader_rejects_oversized_requests() {
        tauri::async_runtime::block_on(async {
            let (mut client, mut server) = UnixStream::pair().expect("socket pair should open");
            let request = vec![b'a'; REQUEST_MAX_BYTES + 1];
            let writer = tokio::spawn(async move { client.write_all(&request).await });

            let result = read_request_line(&mut server).await;

            assert!(result.is_err());
            writer
                .await
                .expect("writer task should finish")
                .expect("oversized request should write");
        });
    }

    #[test]
    fn invalid_approval_scope_preserves_pending_request_and_waiter() {
        let (respond, mut receive) = oneshot::channel();
        let mut pending = Some(PendingAuthorizationRequest {
            request: test_authorization_request("allRetained"),
            respond,
        });

        let result = take_validated_pending_request(
            &mut pending,
            &ApproveCliAccessRequest {
                scope: "lastDay".to_string(),
            },
        );

        assert!(result.is_err());
        assert!(pending.is_some());
        assert!(matches!(receive.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn active_request_guard_allows_only_one_authorization_flow() {
        let active = Arc::new(AtomicBool::new(false));
        let first = ActiveRequestGuard::acquire(active.clone());

        assert!(first.is_some());
        assert!(ActiveRequestGuard::acquire(active.clone()).is_none());

        drop(first);
        assert!(ActiveRequestGuard::acquire(active).is_some());
    }
}
