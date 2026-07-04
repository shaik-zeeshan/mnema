#[cfg(unix)]
use std::os::unix::net::UnixListener as StdUnixListener;
#[cfg(any(test, unix))]
use std::path::PathBuf;
#[cfg(any(test, unix, windows))]
use std::sync::atomic::Ordering;
use std::sync::{atomic::AtomicBool, Arc, Mutex};
#[cfg(any(unix, windows))]
use std::time::Duration;

use app_infra::brokered_access::{
    BrokerClientIdentity, BrokerClientIdentitySource, BrokerGrantScope, BrokeredCaptureAccess,
};
use serde::{Deserialize, Serialize};
use tauri::Manager;
#[cfg(any(unix, windows))]
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};
use tokio::sync::oneshot;
#[cfg(unix)]
use tokio::net::UnixListener as TokioUnixListener;
#[cfg(any(unix, windows))]
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::timeout,
};

#[cfg(any(unix, windows))]
use crate::windows;

#[cfg(any(test, unix, windows))]
const QUICK_APPROVAL_SCOPE: &str = "lastDay";
#[cfg(any(test, unix, windows))]
const QUICK_APPROVAL_DURATION_SECONDS: u64 = 24 * 60 * 60;
#[cfg(any(unix, windows))]
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(any(unix, windows))]
const REQUEST_MAX_BYTES: usize = 64 * 1024;

#[derive(Clone, Default)]
pub struct BrokerAuthorizationChannelState {
    #[cfg_attr(not(any(unix, windows)), allow(dead_code))]
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
    pub duration: AuthorizationChannelDuration,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationChannelDuration {
    pub minimum_seconds: u64,
    pub preferred_seconds: u64,
}

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
    pub scope: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingCliAccessRequestDto {
    pub request_id: String,
    pub client: AuthorizationChannelClient,
    pub command: String,
    pub minimum_scope: String,
    pub preferred_scope: String,
    pub minimum_duration_seconds: u64,
    pub preferred_duration_seconds: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveCliAccessRequest {
    pub scope: String,
    pub duration_seconds: u64,
}

#[cfg(any(test, unix, windows))]
struct ActiveRequestGuard {
    active: Arc<AtomicBool>,
}

#[cfg(any(test, unix, windows))]
impl ActiveRequestGuard {
    fn acquire(active: Arc<AtomicBool>) -> Option<Self> {
        (!active.swap(true, Ordering::SeqCst)).then_some(Self { active })
    }
}

#[cfg(any(test, unix, windows))]
impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
    }
}

pub fn start(app: &tauri::AppHandle) -> Result<(), String> {
    #[cfg(not(any(unix, windows)))]
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

    #[cfg(windows)]
    {
        use std::ffi::c_void;
        use tokio::net::windows::named_pipe::ServerOptions;
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

        // The DACL is always keyed to the *real* current-user SID, even when the
        // pipe name is overridden for tests, so resolve it up front and log a
        // dedicated error if the token lookup fails (ADR 0045 observability note).
        let sid = match current_user_sid_string() {
            Ok(sid) => sid,
            Err(error) => {
                tauri_plugin_log::log::error!(
                    "failed to resolve current user SID for CLI access pipe: {error}"
                );
                return Ok(());
            }
        };
        // Reuse the SID resolved above rather than looking it up a second time.
        let pipe_name = cli_access_pipe_name(app.config().identifier.as_str(), &sid);

        // Protected DACL granting GENERIC_ALL only to the current user — no
        // Everyone, no anonymous, no inherited ACE can widen it.
        let sddl = format!("D:P(A;;GA;;;{sid})");
        let sddl_wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut security_descriptor: *mut c_void = std::ptr::null_mut();
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl_wide.as_ptr(),
                SDDL_REVISION_1,
                &mut security_descriptor,
                std::ptr::null_mut(),
            )
        };
        if converted == 0 {
            tauri_plugin_log::log::error!(
                "failed to build CLI access pipe security descriptor (SDDL {sddl})"
            );
            return Ok(());
        }

        // The security descriptor and its SECURITY_ATTRIBUTES must outlive every
        // pipe instance created over the process lifetime — each instance carries
        // the same DACL, not just the first. Per ADR 0045 ("Security-descriptor
        // lifetime"), build it once and intentionally leak it: the accept loop
        // runs until the app exits, so there is nothing to free. We carry the raw
        // pointer across `.await` points as a `usize` (raw pointers are not `Send`)
        // and re-materialize it for each `create_*` call, which never crosses an
        // await.
        let security_attributes = Box::new(SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: security_descriptor,
            bInheritHandle: 0,
        });
        let security_attributes_addr = Box::into_raw(security_attributes) as usize;

        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            // First instance owns the endpoint name: `first_pipe_instance(true)`
            // makes us fail loudly rather than adopt a squatted pipe.
            let mut server = match unsafe {
                ServerOptions::new()
                    .first_pipe_instance(true)
                    .reject_remote_clients(true)
                    .create_with_security_attributes_raw(
                        &pipe_name,
                        security_attributes_addr as *mut c_void,
                    )
            } {
                Ok(server) => server,
                Err(error) => {
                    tauri_plugin_log::log::error!(
                        "failed to create CLI access pipe (endpoint may already be owned): {error}"
                    );
                    return;
                }
            };
            loop {
                if let Err(error) = server.connect().await {
                    // A failed connect leaves this instance unusable; replace it
                    // with a fresh one rather than re-polling the same handle in
                    // a tight (potentially 100% CPU) loop, keeping the channel
                    // alive rather than killing it for the app's lifetime.
                    tauri_plugin_log::log::error!("CLI access pipe connect failed: {error}");
                    server =
                        create_next_pipe_instance_with_backoff(&pipe_name, security_attributes_addr)
                            .await;
                    continue;
                }
                let connected = server;
                // Hand off the connected client first, so a failure to create the
                // *next* listening instance can never drop an already-connected
                // client or permanently kill the channel.
                let handler_app = app.clone();
                tauri::async_runtime::spawn(async move {
                    handle_connection(handler_app, connected).await;
                });
                // Bring up the next listening instance (same DACL, no
                // `first_pipe_instance`) so a concurrent second client can
                // connect and get the fast `busy` response.
                server =
                    create_next_pipe_instance_with_backoff(&pipe_name, security_attributes_addr)
                        .await;
            }
        });
        Ok(())
    }
}

#[cfg(any(unix, windows))]
async fn handle_connection<S>(app: tauri::AppHandle, mut stream: S)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
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

    let state = app.state::<BrokerAuthorizationChannelState>();
    let Some(_guard) = ActiveRequestGuard::acquire(state.active.clone()) else {
        let _ = write_unavailable(stream, request.request_id, "busy").await;
        return;
    };

    if !windows::is_onboarding_complete(&app) {
        let _ = write_unavailable(stream, request.request_id, "onboardingRequired").await;
        return;
    }

    match prompt_for_default_access(&app, &request).await {
        AuthorizationDecision::Approved => {
            let response = quick_approval_grant_policy_for_request(&request)
                .and_then(|policy| create_grant_response(&app, &request, policy))
                .unwrap_or_else(|_| AuthorizationChannelResponse {
                    schema_version: 1,
                    request_id: request.request_id.clone(),
                    decision: "unavailable".to_string(),
                    reason: Some("invalidRequest".to_string()),
                    grant: None,
                });
            let _ = write_response(stream, response).await;
        }
        AuthorizationDecision::MoreOptions => {
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
        AuthorizationDecision::Cancelled => {
            let _ = write_denied(stream, request.request_id, "userCancelled").await;
        }
    }
}

#[cfg(any(unix, windows))]
async fn read_request_line<S>(stream: &mut S) -> std::io::Result<Option<String>>
where
    S: AsyncRead + Unpin,
{
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

#[cfg(any(unix, windows))]
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

pub fn has_pending_cli_access_request(app: &tauri::AppHandle) -> bool {
    app.state::<BrokerAuthorizationChannelState>()
        .pending
        .lock()
        .map(|pending| pending.is_some())
        .unwrap_or(false)
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
            client: request.client,
            command: request.command,
            minimum_scope: request.scope.minimum,
            preferred_scope: request.scope.preferred,
            minimum_duration_seconds: request.duration.minimum_seconds,
            preferred_duration_seconds: request.duration.preferred_seconds,
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
    request.duration.preferred_seconds = approval.duration_seconds;
    let response = create_grant_response(
        &app,
        &request,
        grant_policy(&request.scope.preferred, request.duration.preferred_seconds),
    )
    .unwrap_or_else(|_| AuthorizationChannelResponse {
        schema_version: 1,
        request_id: request.request_id,
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
    if approval.duration_seconds < request.duration.minimum_seconds {
        return Err("selected duration does not satisfy the pending command".to_string());
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

fn scope_satisfies_minimum(selected: &str, minimum: &str) -> bool {
    selected == minimum || selected == "allRetained"
}

#[cfg(any(unix, windows))]
enum AuthorizationDecision {
    Approved,
    MoreOptions,
    Cancelled,
}

#[cfg(any(unix, windows))]
async fn prompt_for_default_access(
    app: &tauri::AppHandle,
    request: &AuthorizationChannelRequest,
) -> AuthorizationDecision {
    let app = app.clone();
    let request = request.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let body = format!(
            "{} wants access to searchable Mnema text from the last day for 24 hours.",
            request.client.label
        );
        match app
            .dialog()
            .message(body)
            .kind(MessageDialogKind::Info)
            .title("Allow CLI Access?")
            .buttons(MessageDialogButtons::YesNoCancelCustom(
                "Allow".to_string(),
                "More Options".to_string(),
                "Cancel".to_string(),
            ))
            .blocking_show_with_result()
        {
            MessageDialogResult::Yes => AuthorizationDecision::Approved,
            MessageDialogResult::No => AuthorizationDecision::MoreOptions,
            MessageDialogResult::Custom(label) if label == "Allow" => {
                AuthorizationDecision::Approved
            }
            MessageDialogResult::Custom(label) if label == "More Options" => {
                AuthorizationDecision::MoreOptions
            }
            _ => AuthorizationDecision::Cancelled,
        }
    })
    .await
    .unwrap_or(AuthorizationDecision::Cancelled)
}

fn create_grant_response(
    app: &tauri::AppHandle,
    request: &AuthorizationChannelRequest,
    policy: GrantPolicy,
) -> Result<AuthorizationChannelResponse, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("failed to resolve app config dir: {error}"))?;
    let identity = BrokerClientIdentity::new(
        request.client.label.clone(),
        match request.client.source.as_str() {
            "explicit" => BrokerClientIdentitySource::Explicit,
            "env" => BrokerClientIdentitySource::Env,
            "inferred" => BrokerClientIdentitySource::Inferred,
            _ => BrokerClientIdentitySource::Defaulted,
        },
    )
    .map_err(|error| error.to_string())?;
    let grant = BrokeredCaptureAccess::from_config_dir(config_dir)
        .create_grant_for_identity(identity, policy.hours, policy.scope.clone())
        .map_err(|error| error.to_string())?;
    Ok(AuthorizationChannelResponse {
        schema_version: 1,
        request_id: request.request_id.clone(),
        decision: "approved".to_string(),
        reason: None,
        grant: Some(AuthorizationChannelGrant {
            id: grant.id,
            client_label: grant.label,
            scope: match policy.scope {
                BrokerGrantScope::RecentDays { .. } => "lastDay",
                BrokerGrantScope::AllRetainedHistory => "allRetained",
            }
            .to_string(),
            expires_at: app_infra::brokered_access::format_broker_unix_ms(grant.expires_at_unix_ms),
        }),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GrantPolicy {
    scope: BrokerGrantScope,
    hours: u64,
}

fn grant_policy(scope: &str, duration_seconds: u64) -> GrantPolicy {
    GrantPolicy {
        scope: if scope == "allRetained" {
            BrokerGrantScope::AllRetainedHistory
        } else {
            BrokerGrantScope::RecentDays { days: 1 }
        },
        hours: duration_seconds.div_ceil(60 * 60).clamp(1, 24 * 7),
    }
}

#[cfg(any(test, unix, windows))]
fn quick_approval_grant_policy() -> GrantPolicy {
    grant_policy(QUICK_APPROVAL_SCOPE, QUICK_APPROVAL_DURATION_SECONDS)
}

#[cfg(any(test, unix, windows))]
fn quick_approval_grant_policy_for_request(
    request: &AuthorizationChannelRequest,
) -> Result<GrantPolicy, String> {
    let approval = ApproveCliAccessRequest {
        scope: QUICK_APPROVAL_SCOPE.to_string(),
        duration_seconds: QUICK_APPROVAL_DURATION_SECONDS,
    };
    validate_cli_access_approval(request, &approval)?;
    Ok(quick_approval_grant_policy())
}

#[cfg(any(unix, windows))]
async fn write_denied<S>(stream: S, request_id: String, reason: &str) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
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

#[cfg(any(unix, windows))]
async fn write_unavailable<S>(
    stream: S,
    request_id: String,
    reason: &str,
) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
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

#[cfg(any(unix, windows))]
async fn write_response<S>(
    mut stream: S,
    response: AuthorizationChannelResponse,
) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    let raw = serde_json::to_string(&response)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    stream.write_all(format!("{raw}\n").as_bytes()).await
}

#[cfg(unix)]
fn stale_socket(path: &PathBuf) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_err()
}

#[cfg(any(test, unix))]
pub fn socket_path_for_identifier(identifier: &str) -> PathBuf {
    default_app_config_dir_for_identifier(identifier)
        .unwrap_or_else(|| std::env::temp_dir().join(identifier))
        .join("cli-access.sock")
}

/// Pure Windows named-pipe endpoint name, mirroring `socket_path_for_identifier`.
/// Ungated so it can be unit-tested on every OS. The SID sits in the *name*
/// (not only the DACL) because the name determines which user's server owns the
/// endpoint on a multi-session box (ADR 0045).
pub fn pipe_name_for(identifier: &str, sid: &str) -> String {
    format!(r"\\.\pipe\{identifier}-{sid}-cli-access")
}

/// Resolve the current user's SID as an `S-1-…` string from this process's own
/// token — no handoff and no durable discovery artifact; the CLI derives the
/// same value from its own token when running as the same user (ADR 0045).
#[cfg(windows)]
fn current_user_sid_string() -> Result<String, String> {
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err("OpenProcessToken failed for current process".to_string());
        }
        // Size probe: expected to fail with ERROR_INSUFFICIENT_BUFFER, filling
        // `needed` with the required byte count.
        let mut needed: u32 = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        if needed == 0 {
            CloseHandle(token);
            return Err("GetTokenInformation size probe failed".to_string());
        }
        let mut buffer = vec![0u8; needed as usize];
        if GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr() as *mut core::ffi::c_void,
            needed,
            &mut needed,
        ) == 0
        {
            CloseHandle(token);
            return Err("GetTokenInformation failed to read TokenUser".to_string());
        }
        CloseHandle(token);

        // `buffer` is a `Vec<u8>` (alignment 1); `TOKEN_USER` contains a pointer
        // and needs pointer alignment, so read it out with `read_unaligned`
        // rather than forming a (possibly-misaligned) reference, which is UB.
        let token_user = std::ptr::read_unaligned(buffer.as_ptr() as *const TOKEN_USER);
        let mut sid_pwstr: windows_sys::core::PWSTR = std::ptr::null_mut();
        if ConvertSidToStringSidW(token_user.User.Sid, &mut sid_pwstr) == 0 || sid_pwstr.is_null() {
            return Err("ConvertSidToStringSidW failed".to_string());
        }
        let mut len = 0usize;
        while *sid_pwstr.add(len) != 0 {
            len += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(sid_pwstr, len));
        LocalFree(sid_pwstr as *mut core::ffi::c_void);
        Ok(sid)
    }
}

/// The CLI-access pipe name, honoring a non-empty `MNEMA_CLI_ACCESS_PIPE_NAME`
/// override for tests (returned verbatim); otherwise derived from the app
/// identifier and the already-resolved current-user SID.
#[cfg(windows)]
fn cli_access_pipe_name(identifier: &str, sid: &str) -> String {
    if let Ok(name) = std::env::var("MNEMA_CLI_ACCESS_PIPE_NAME") {
        if !name.is_empty() {
            return name;
        }
    }
    pipe_name_for(identifier, sid)
}

/// Create the next listening pipe instance (same DACL, without
/// `first_pipe_instance`), retrying transient failures with a capped
/// exponential backoff so the channel stays alive rather than dying on a
/// one-off error — while bounding log volume to at most one line per backoff
/// interval (≤ 30 s at the cap).
#[cfg(windows)]
async fn create_next_pipe_instance_with_backoff(
    pipe_name: &str,
    security_attributes_addr: usize,
) -> tokio::net::windows::named_pipe::NamedPipeServer {
    use std::ffi::c_void;
    use tokio::net::windows::named_pipe::ServerOptions;

    let mut delay = Duration::from_millis(200);
    loop {
        let result = unsafe {
            ServerOptions::new()
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(
                    pipe_name,
                    security_attributes_addr as *mut c_void,
                )
        };
        match result {
            Ok(server) => return server,
            Err(error) => {
                tauri_plugin_log::log::error!(
                    "failed to create CLI access pipe instance, retrying in {delay:?}: {error}"
                );
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(30));
            }
        }
    }
}

#[cfg(unix)]
fn socket_path_for_config_dir(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("cli-access.sock")
}

#[cfg(any(test, unix))]
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
    use tokio::sync::oneshot::error::TryRecvError;

    fn test_authorization_request(
        minimum_scope: &str,
        minimum_duration_seconds: u64,
    ) -> AuthorizationChannelRequest {
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
            duration: AuthorizationChannelDuration {
                minimum_seconds: minimum_duration_seconds,
                preferred_seconds: minimum_duration_seconds,
            },
            interactive: true,
            created_at: "2026-05-23T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn socket_path_uses_configured_identifier() {
        let path = socket_path_for_identifier("com.example.mnema-test");
        assert!(path.ends_with("com.example.mnema-test/cli-access.sock"));
    }

    #[test]
    fn all_retained_satisfies_last_day_minimum() {
        assert!(scope_satisfies_minimum("allRetained", "lastDay"));
    }

    #[test]
    fn last_day_does_not_satisfy_all_retained_minimum() {
        assert!(!scope_satisfies_minimum("lastDay", "allRetained"));
    }

    #[test]
    fn quick_approval_policy_uses_fixed_default_grant() {
        assert_eq!(
            quick_approval_grant_policy(),
            GrantPolicy {
                scope: BrokerGrantScope::RecentDays { days: 1 },
                hours: 24,
            }
        );
    }

    #[test]
    fn quick_approval_is_rejected_when_request_requires_broader_scope() {
        let request = test_authorization_request("allRetained", 24 * 60 * 60);

        let result = quick_approval_grant_policy_for_request(&request);

        assert!(result.is_err());
    }

    #[test]
    fn quick_approval_is_rejected_when_request_requires_longer_duration() {
        let request = test_authorization_request("lastDay", 25 * 60 * 60);

        let result = quick_approval_grant_policy_for_request(&request);

        assert!(result.is_err());
    }

    #[test]
    fn explicit_approval_policy_uses_selected_broader_access() {
        assert_eq!(
            grant_policy("allRetained", 7 * 24 * 60 * 60),
            GrantPolicy {
                scope: BrokerGrantScope::AllRetainedHistory,
                hours: 24 * 7,
            }
        );
    }

    #[test]
    fn grant_policy_ceil_rounds_fractional_hours() {
        assert_eq!(
            grant_policy("lastDay", 90 * 60),
            GrantPolicy {
                scope: BrokerGrantScope::RecentDays { days: 1 },
                hours: 2,
            }
        );
    }

    #[test]
    fn pipe_name_uses_configured_identifier_and_sid() {
        assert_eq!(
            pipe_name_for("com.example.mnema-test", "S-1-5-21-1-2-3-1001"),
            r"\\.\pipe\com.example.mnema-test-S-1-5-21-1-2-3-1001-cli-access"
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn request_line_reader_rejects_oversized_requests() {
        tauri::async_runtime::block_on(async {
            let (mut client, mut server) = tokio::io::duplex(REQUEST_MAX_BYTES * 2);
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
            request: test_authorization_request("allRetained", 3600),
            respond,
        });

        let result = take_validated_pending_request(
            &mut pending,
            &ApproveCliAccessRequest {
                scope: "lastDay".to_string(),
                duration_seconds: 3600,
            },
        );

        assert!(result.is_err());
        assert!(pending.is_some());
        assert!(matches!(receive.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn invalid_approval_duration_preserves_pending_request_and_waiter() {
        let (respond, mut receive) = oneshot::channel();
        let mut pending = Some(PendingAuthorizationRequest {
            request: test_authorization_request("lastDay", 3600),
            respond,
        });

        let result = take_validated_pending_request(
            &mut pending,
            &ApproveCliAccessRequest {
                scope: "lastDay".to_string(),
                duration_seconds: 3599,
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
