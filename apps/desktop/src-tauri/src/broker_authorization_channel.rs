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
    BrokerClientIdentity, BrokerClientIdentitySource, BrokerGrantScope, BrokeredCaptureAccess,
};
use serde::{Deserialize, Serialize};
use tauri::Manager;
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};
use tokio::sync::oneshot;
#[cfg(unix)]
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener as TokioUnixListener, UnixStream},
    time::timeout,
};

use crate::windows;

const QUICK_APPROVAL_SCOPE: &str = "lastDay";
const QUICK_APPROVAL_DURATION_SECONDS: u64 = 24 * 60 * 60;
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
        let socket_path = socket_path_for_identifier(app.config().identifier.as_str());
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

enum AuthorizationDecision {
    Approved,
    MoreOptions,
    Cancelled,
}

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

fn quick_approval_grant_policy() -> GrantPolicy {
    grant_policy(QUICK_APPROVAL_SCOPE, QUICK_APPROVAL_DURATION_SECONDS)
}

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
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join(identifier).join("cli-access.sock")
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
