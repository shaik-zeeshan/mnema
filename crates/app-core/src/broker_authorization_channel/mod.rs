//! The CLI Access approval channel: the Unix socket `mnema-cli` asks on, the
//! single pending request the approval window renders, and the window's
//! approve / block / cancel answers (ADR 0059).

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

use app_infra::brokered_access::{BrokerGrantScope, BrokeredCaptureAccess};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
#[cfg(unix)]
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener as TokioUnixListener, UnixStream},
    time::timeout,
};

use crate::{AppCore, WindowRequest, WindowSignal};

mod grants;
#[cfg(test)]
mod tests;
mod wire_text;

use grants::*;
use wire_text::*;

/// Cap on the client-supplied name shown in the approval window, so it can't push
/// the rest of the consent copy off the sheet.
const CLIENT_LABEL_MAX_CHARS: usize = 64;
/// What the window says when the permission could not be written. The retry it
/// offers is only real because [`restore_pending`] leaves the request in the
/// slot, so the copy and that restore are one decision.
const GRANT_WRITE_FAILED: &str =
    "Couldn't save this permission — nothing was granted. Try again, or deny the request.";
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
    /// Has the approval window actually been handed this request? Set by
    /// [`take_request_for_display`], read by [`cancel_pending`] — it is what makes
    /// a Deny click an ANSWER rather than a guess. See both for why.
    rendered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationChannelRequest {
    pub schema_version: u32,
    pub request_id: String,
    pub client: AuthorizationChannelClient,
    pub command: String,
    pub scope: AuthorizationChannelScope,
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
///   caller MUST check it covers what it asked for: the approval SETS the row's
///   scope to whatever the picker selected, which can be narrower than the
///   `preferred` scope (never below `minimum` — the window refuses that).
/// - `denied` + `userCancelled` — the user answered no, via Deny or Esc. A
///   denial is an answer: the caller must not retry it.
/// - `denied` + `closed` — no verdict was ever given: the approval window was
///   closed, it failed to open, or the waiter was dropped. This one IS
///   retryable, which is why it must never be spelled `userCancelled`.
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
    /// `true` when this approval created the permission, `false` when it updated
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
    /// The scope this client's standing permission ALREADY carries, or `None` when
    /// it holds none. The window states it whenever the selection sits below it —
    /// see [`current_scope_for_client`].
    pub current_scope: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveCliAccessRequest {
    /// The request the window RENDERED. The pending slot is single-flight but not
    /// stable over time: a client that hangs up frees it, and the next request
    /// refills it while the reused window is still showing the previous one (the
    /// focus re-read never fires on an already-frontmost window and is skipped
    /// mid-approve). An approval that names no request would then grant whatever
    /// has since taken the slot — a different tool than the one the user read.
    pub request_id: String,
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

pub fn start(core: &Arc<AppCore>) -> Result<(), String> {
    #[cfg(not(unix))]
    {
        let _ = core;
        return Ok(());
    }

    #[cfg(unix)]
    {
        let socket_path = socket_path_for_config_dir(&core.paths().config);
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
        let task_core = Arc::clone(core);
        core.spawn(async move {
            let listener = match TokioUnixListener::from_std(listener) {
                Ok(listener) => listener,
                Err(error) => {
                    log::error!("failed to initialize CLI access socket listener: {error}");
                    return;
                }
            };
            loop {
                let Ok((stream, _addr)) = listener.accept().await else {
                    continue;
                };
                let core = Arc::clone(&task_core);
                task_core.spawn(async move {
                    handle_connection(core, stream).await;
                });
            }
        });
        Ok(())
    }
}

#[cfg(unix)]
async fn handle_connection(core: Arc<AppCore>, mut stream: UnixStream) {
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
    if client_is_blocked(&core, &request.client.label) {
        let _ = write_blocked(stream, request.request_id).await;
        return;
    }

    // A request no click could ever grant is refused at the door — still ahead of
    // the single-flight guard. An all-invisible client name cannot key a
    // permission row and an unrecognized scope spelling can never satisfy the
    // window's picker, so either would open a consent sheet whose Allow button
    // only ever errors, holding the guard (every other tool told `busy`) until
    // the client's own timeout.
    if !request_is_well_formed(&request) {
        let _ = write_unavailable(stream, request.request_id, "invalidRequest").await;
        return;
    }

    let state = core.state::<BrokerAuthorizationChannelState>();
    let Some(_guard) = ActiveRequestGuard::acquire(state.active.clone()) else {
        let _ = write_unavailable(stream, request.request_id, "busy").await;
        return;
    };

    let onboarding_state = core.state::<crate::onboarding_state::OnboardingStateStore>();
    if !crate::onboarding_state::current_onboarding_state(&core, &onboarding_state).is_complete() {
        let _ = write_unavailable(stream, request.request_id, "onboardingRequired").await;
        return;
    }

    // The window is the only consent surface: approval now fires a few times in
    // a tool's life, so a native fast path bought nothing and cost the identity
    // provenance chip and the anti-reflex affordances (focus on Deny, Enter
    // unbound, Esc denies).
    // Race the verdict against the client hanging up. A pending approval must not
    // outlive the process that asked for it: the socket task parks on the oneshot
    // and never polls the stream, so without this it cannot notice the CLI's own
    // 120 s timeout, a killed client, or a window that was silently reused and
    // torn down by a previous request's teardown. Any of those would leave the
    // single-flight guard held and the pending slot occupied for the life of the
    // app, answering every later request `busy` — a window that is gone can
    // never resolve the waiter, and nothing else will.
    // Bound outside the future so the borrow outlives it. `None` here is the safe
    // default — it makes the floor `lastDay` — so this lookup can only ever widen
    // what the window offers, never narrow the consent guarantee.
    let current_scope = current_scope_for_client(&core, &request.client.label);
    let verdict = {
        let wait = await_user_verdict(&state, &request, current_scope.as_deref(), || {
            core.host().open_window(WindowRequest::CliAccessRequest)
        });
        tokio::pin!(wait);
        tokio::select! {
            response = &mut wait => Some(response),
            () = peer_hung_up(&mut stream) => None,
        }
    };
    match verdict {
        Some(response) => {
            let _ = write_response(stream, response).await;
        }
        // Nobody is left to read an answer. Release the slot so the NEXT client
        // gets a window instead of `busy`; the guard drops with this task.
        None => {
            cancel_pending(&state, "closed");
            // ...and tear down the window this dead request left on screen. The
            // approval window is REUSED by label and `open_or_focus_window` only
            // FOCUSES an existing one, so an already-frontmost window fires no
            // `focus` event and the page keeps rendering this abandoned request
            // while the next client waits, invisibly, for a verdict its window
            // was never shown for. `CloseCliAccessRequest` (a `destroy()`, see the
            // Tauri host), not `close()`: `CloseRequested` is what lib.rs reads as
            // "the user dismissed the approval", and the pending slot is already
            // empty here.
            core.host()
                .window_signal(WindowSignal::CloseCliAccessRequest);
        }
    }
}

/// Resolves when the peer closes its end (or the connection breaks). The CLI
/// sends exactly one request line and then only reads, so any successful read
/// here is unexpected trailing input and is ignored rather than treated as a
/// hangup.
#[cfg(unix)]
async fn peer_hung_up(stream: &mut UnixStream) {
    let mut scratch = [0_u8; 256];
    loop {
        match stream.read(&mut scratch).await {
            Ok(0) | Err(_) => return,
            Ok(_) => continue,
        }
    }
}

/// Everything between "this connection owns the single-flight guard" and "there
/// is a verdict to write back". The consent surface is injected because it is
/// the ONLY thing that can resolve the waiter: if it never appears, nothing
/// else does, and the socket task awaits forever holding the guard while the
/// pending slot stays occupied — which answers every later CLI request `busy`
/// for the life of the app.
async fn await_user_verdict(
    state: &BrokerAuthorizationChannelState,
    request: &AuthorizationChannelRequest,
    current_scope: Option<&str>,
    open_consent_surface: impl FnOnce() -> Result<(), String>,
) -> AuthorizationChannelResponse {
    // The floor the window ENFORCES and PRE-SELECTS is wire input, and the window
    // disables every option below it — so the clamp belongs on the ONE path that
    // reaches the window, not at a call site upstream that can be deleted without
    // a test noticing. `current_scope` defaults SAFE: a caller that passes `None`
    // gets the narrowest floor, so losing the lookup costs a widen its
    // pre-selection, never the consent guarantee.
    let mut request = request.clone();
    request.scope.minimum = enforced_minimum_scope(&request.scope.minimum, current_scope);
    let request = &request;

    let (send, receive) = oneshot::channel();
    if !store_pending(state, request.clone(), send) {
        return unavailable_response(request.request_id.clone(), "busy");
    }
    if open_consent_surface().is_err() {
        // Nothing else ever resolves this waiter, so releasing the slot here is
        // the only thing standing between one failed window and a channel that
        // answers `busy` until the app restarts.
        cancel_pending(state, "closed");
    }
    receive
        .await
        .unwrap_or_else(|_| denied_response(request.request_id.clone(), "closed"))
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

fn store_pending(
    state: &BrokerAuthorizationChannelState,
    request: AuthorizationChannelRequest,
    respond: oneshot::Sender<AuthorizationChannelResponse>,
) -> bool {
    let Ok(mut pending) = state.pending.lock() else {
        return false;
    };
    if pending.is_some() {
        return false;
    }
    *pending = Some(PendingAuthorizationRequest {
        request,
        respond,
        rendered: false,
    });
    true
}

/// Hand the pending request to the approval window, and record that the window
/// has now been shown it.
///
/// Every caller of `get_pending_cli_access_request` renders what it gets, so this
/// flag is exactly "the sheet the user is looking at is about this request" — and
/// that is what makes a Deny click an ANSWER. The slot is single-flight but not
/// stable over time: an approval that fails leaves the window up with its inline
/// error and the slot EMPTY, the socket task then drops the single-flight guard,
/// and the next client refills the slot while the window — already frontmost, so
/// `set_focus` fires no `focus` event — keeps showing the previous request.
/// Without the flag, that user's Deny would answer the unseen successor
/// `userCancelled`: exit 10, `retryable: false`, "the user refused you" for a
/// decision nobody made.
fn take_request_for_display(
    state: &BrokerAuthorizationChannelState,
) -> Option<AuthorizationChannelRequest> {
    let mut slot = state.pending.lock().ok()?;
    let pending = slot.as_mut()?;
    pending.rendered = true;
    Some(pending.request.clone())
}

pub fn get_pending_cli_access_request(core: &AppCore) -> Option<PendingCliAccessRequestDto> {
    take_request_for_display(&core.state::<BrokerAuthorizationChannelState>()).map(|request| {
        PendingCliAccessRequestDto {
            request_id: request.request_id,
            client: AuthorizationChannelClient {
                // The window is now the ONLY consent surface, and it renders this
                // name straight into the requester chip. Sanitize on the way out.
                label: client_label_display(&request.client.label),
                source: request.client.source,
            },
            // The command string is wire input too, and it renders inline in the
            // window's "Requested via" line — same cleanup as the label.
            command: command_display(&request.command),
            minimum_scope: request.scope.minimum,
            preferred_scope: request.scope.preferred,
            current_scope: current_scope_for_client(core, &request.client.label),
            created_at: request.created_at,
        }
    })
}

pub fn approve_pending_cli_access_request(
    core: &AppCore,
    approval: ApproveCliAccessRequest,
) -> Result<(), String> {
    let outcome = approve_pending_request_in(
        &core.state::<BrokerAuthorizationChannelState>(),
        &approval,
        |request, scope| create_grant_response(core, request, scope),
    );
    if outcome.is_ok() {
        core.host()
            .window_signal(WindowSignal::CloseCliAccessRequest);
    }
    // The page renders its granted receipt on Ok and ONLY on Ok, so this mapping
    // is what stops a permission that was never written — or a blocked row —
    // from being shown to the user as "Access granted". A non-Ok return leaves
    // the window open so the error names why.
    outcome
}

/// Everything the approve command does to the pending slot, with the permission
/// write injected — it is the only part that touches disk, and every failure mode
/// this has to answer for (a full disk, a read-only config dir, an unparseable
/// `broker-grants.json`) lives in it.
fn approve_pending_request_in(
    state: &BrokerAuthorizationChannelState,
    approval: &ApproveCliAccessRequest,
    write_grant: impl FnOnce(
        &AuthorizationChannelRequest,
        BrokerGrantScope,
    ) -> Result<AuthorizationChannelResponse, String>,
) -> Result<(), String> {
    let mut pending = take_pending_request_for_approval(state, approval)?;
    pending.request.scope.preferred = approval.scope.clone();
    let written = BrokerGrantScope::from_wire_name(&pending.request.scope.preferred)
        .ok_or_else(|| "unknown scope".to_string())
        .and_then(|scope| write_grant(&pending.request, scope));
    let response = match written {
        Ok(response) => response,
        // The permission could not be WRITTEN — a full disk, a read-only config
        // dir, an unparseable `broker-grants.json`. Nothing was granted and the
        // client has not been answered, so this is no verdict on its request: put
        // the request back instead of consuming it. Answering here spent the CLI's
        // one question on `unavailable`/`invalidRequest`, which it reports as
        // "Mnema rejected the access request as malformed" (exit 18,
        // non-retryable) over a failed disk write — and left the window offering a
        // "Try again" that could only come back "no pending CLI Access request",
        // on a slot the next tool was free to take while the window still rendered
        // this one. The client keeps waiting on its own timeout, and every door
        // that ends this window still answers it.
        Err(_) => return Err(restore_pending(state, pending)),
    };
    let outcome = approval_command_outcome(&response);
    let _ = pending.respond.send(response);
    outcome
}

/// Put a request nobody answered back in the slot, and name the failure for the
/// window.
///
/// A request that took the slot while the write was in flight keeps it: dropping
/// this one then resolves its own waiter `denied`/`closed` — retryable — rather
/// than evicting a live request no window has shown yet.
fn restore_pending(
    state: &BrokerAuthorizationChannelState,
    pending: PendingAuthorizationRequest,
) -> String {
    if let Ok(mut slot) = state.pending.lock() {
        if slot.is_none() {
            *slot = Some(pending);
        }
    }
    GRANT_WRITE_FAILED.to_string()
}

/// Ok exactly when the client was handed an approval, i.e. when the permission
/// file really carries the grant. Every other outcome is an error the window
/// must surface instead of a receipt.
fn approval_command_outcome(response: &AuthorizationChannelResponse) -> Result<(), String> {
    match response.decision.as_str() {
        "approved" => Ok(()),
        "blocked" => Err("This tool is blocked in Settings. Unblock it there first.".to_string()),
        _ => Err(GRANT_WRITE_FAILED.to_string()),
    }
}

fn take_pending_request_for_approval(
    state: &BrokerAuthorizationChannelState,
    approval: &ApproveCliAccessRequest,
) -> Result<PendingAuthorizationRequest, String> {
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
    // The click must approve the request the window RENDERED, not whichever
    // request has since taken the single-flight slot. Spelled with the same
    // "no pending CLI Access request" substring the empty-slot case uses, so the
    // page maps it to its "this request is no longer valid" copy.
    if approval.request_id != request.request_id {
        return Err("no pending CLI Access request matches this approval".to_string());
    }
    if !scope_satisfies_minimum(&approval.scope, &request.scope.minimum) {
        return Err("selected scope does not satisfy the pending command".to_string());
    }
    Ok(())
}

/// Block from the window: a standing rejection, persisted even for a tool that
/// was never granted (`block_identity`), answered `blocked` so the CLI stops.
pub fn block_pending_cli_access_request(core: &AppCore, request_id: String) -> Result<(), String> {
    let config_dir = core.paths().config.clone();
    block_pending_request_in(
        &core.state::<BrokerAuthorizationChannelState>(),
        &request_id,
        |request| {
            BrokeredCaptureAccess::from_config_dir(&config_dir)
                .block_identity(identity_for_request(request)?)
                .map(drop)
                .map_err(|error| error.to_string())
        },
    )?;
    core.host()
        .window_signal(WindowSignal::CloseCliAccessRequest);
    Ok(())
}

/// Same slot rules as [`approve_pending_request_in`]: only the rendered request
/// (by id) is answered, and a failed write puts it back unanswered.
fn block_pending_request_in(
    state: &BrokerAuthorizationChannelState,
    request_id: &str,
    write_block: impl FnOnce(&AuthorizationChannelRequest) -> Result<(), String>,
) -> Result<(), String> {
    let pending = state
        .pending
        .lock()
        .ok()
        .and_then(|mut slot| match slot.as_ref() {
            Some(p) if p.request.request_id == request_id => slot.take(),
            _ => None,
        })
        .ok_or_else(|| "no pending CLI Access request matches this block".to_string())?;
    if write_block(&pending.request).is_err() {
        return Err(restore_pending(state, pending));
    }
    let _ = pending
        .respond
        .send(blocked_response(pending.request.request_id.clone()));
    Ok(())
}

pub fn cancel_pending_cli_access_request(core: &AppCore) -> Result<(), String> {
    cancel_pending_request(core, "userCancelled");
    core.host()
        .window_signal(WindowSignal::CloseCliAccessRequest);
    Ok(())
}

pub fn cancel_pending_request(core: &AppCore, reason: &str) {
    cancel_pending(&core.state::<BrokerAuthorizationChannelState>(), reason);
}

fn cancel_pending(state: &BrokerAuthorizationChannelState, reason: &str) {
    let Some(pending) = state.pending.lock().ok().and_then(|mut slot| slot.take()) else {
        return;
    };
    // A dismissal can only DENY what the window actually showed. The slot can hold
    // a request this sheet was never re-read for (see [`take_request_for_display`]),
    // and answering that one `userCancelled` spends the user's click on a tool they
    // never saw — exit 10, `retryable: false`, never retried. It still has to be
    // answered or it waits on a window that is going away: `closed` is the honest
    // verdict-free answer, and the one the CLI retries.
    let reason = if pending.rendered { reason } else { "closed" };
    let _ = pending
        .respond
        .send(denied_response(pending.request.request_id, reason));
}

fn denied_response(request_id: String, reason: &str) -> AuthorizationChannelResponse {
    AuthorizationChannelResponse {
        schema_version: 1,
        request_id,
        decision: "denied".to_string(),
        reason: Some(reason.to_string()),
        grant: None,
    }
}

fn unavailable_response(request_id: String, reason: &str) -> AuthorizationChannelResponse {
    AuthorizationChannelResponse {
        schema_version: 1,
        request_id,
        decision: "unavailable".to_string(),
        reason: Some(reason.to_string()),
        grant: None,
    }
}

fn blocked_response(request_id: String) -> AuthorizationChannelResponse {
    AuthorizationChannelResponse {
        schema_version: 1,
        request_id,
        decision: "blocked".to_string(),
        reason: Some("blocked".to_string()),
        grant: None,
    }
}

#[cfg(unix)]
async fn write_blocked(stream: UnixStream, request_id: String) -> std::io::Result<()> {
    write_response(stream, blocked_response(request_id)).await
}

#[cfg(unix)]
async fn write_unavailable(
    stream: UnixStream,
    request_id: String,
    reason: &str,
) -> std::io::Result<()> {
    write_response(stream, unavailable_response(request_id, reason)).await
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
