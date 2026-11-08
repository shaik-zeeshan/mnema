//! Deep links (`mnema://…`, `mnema-dev://…`) and the handoff queues they and
//! other windows fill for the main window: broker open-capture results, Quick
//! Recall → Chat conversations, and license activate/claim/renewed links. The
//! event fires for a live listener; the queue covers a window that mounts later.

use std::{collections::VecDeque, path::Path, sync::Mutex};

use tauri::{Emitter, Manager};

use app_core::deep_links::{self, DeepLink};

use crate::windows;

pub(crate) const BROKER_OPEN_CAPTURE_RESULT_EVENT: &str = "broker_open_capture_result";
/// Event the main window listens for to switch the Insights surface to the Chat
/// tab and select a given conversation. Emitted by `open_conversation_in_chat`
/// when Quick Recall promotes a thread into the full Chat workspace (issue #111,
/// ADR 0031). The conversation is already persisted under the same id, so Chat's
/// `get_conversation` + resurrect-from-transcript path continues it seamlessly.
pub(crate) const INSIGHTS_OPEN_CONVERSATION_EVENT: &str = "insights_open_conversation";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrokerOpenCaptureResultPayload {
    opaque_id: String,
    kind: String,
    frame_id: Option<i64>,
    audio_segment_id: Option<i64>,
    /// Audio Search Result Anchor: the match span start within the segment (ms)
    /// and the aligned frame id, so an audio handoff lands on the selected
    /// transcript match instead of the segment start. Absent for the broker-URL
    /// path, which only resolves a capture reference (kind/frame/segment id).
    #[serde(default)]
    span_start_ms: Option<i64>,
    #[serde(default)]
    aligned_frame_id: Option<i64>,
    /// The Quick Recall query that found this hit, so the Timeline can show a
    /// dismissible `from Recall · "query"` landing chip. Absent for other senders.
    #[serde(default)]
    query: Option<String>,
    /// The Recall hit was meaning-only (`foundByMeaning`), so the Timeline's
    /// landed-frame match outlines draw dashed.
    #[serde(default)]
    by_meaning: bool,
}

#[derive(Default)]
pub(crate) struct BrokerOpenCaptureResultState {
    pending: Mutex<VecDeque<BrokerOpenCaptureResultPayload>>,
}

#[tauri::command]
pub(crate) fn drain_pending_broker_open_capture_results(
    state: tauri::State<'_, BrokerOpenCaptureResultState>,
) -> Vec<BrokerOpenCaptureResultPayload> {
    let Ok(mut pending) = state.pending.lock() else {
        return Vec::new();
    };
    pending.drain(..).collect()
}

#[tauri::command]
pub(crate) fn open_capture_result_in_main_window(
    kind: String,
    frame_id: Option<i64>,
    audio_segment_id: Option<i64>,
    span_start_ms: Option<i64>,
    aligned_frame_id: Option<i64>,
    query: Option<String>,
    by_meaning: Option<bool>,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, BrokerOpenCaptureResultState>,
) {
    let payload = BrokerOpenCaptureResultPayload {
        opaque_id: String::from("quick-recall"),
        kind,
        frame_id,
        audio_segment_id,
        span_start_ms,
        aligned_frame_id,
        query,
        by_meaning: by_meaning.unwrap_or(false),
    };
    if let Ok(mut pending) = state.pending.lock() {
        pending.push_back(payload.clone());
    }
    let _ = windows::open_main_window(&app_handle);
    let _ = app_handle.emit(BROKER_OPEN_CAPTURE_RESULT_EVENT, payload);
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InsightsOpenConversationPayload {
    conversation_id: String,
}

/// Pending Insights→Chat conversation handoff(s) for a cold main window. Mirrors
/// `BrokerOpenCaptureResultState`: the live event drives a warm window, while a
/// freshly-opened (cold) main window drains this queue on mount so the handoff
/// still lands when Insights mounts after the event fired.
#[derive(Default)]
pub(crate) struct InsightsOpenConversationState {
    pending: Mutex<VecDeque<InsightsOpenConversationPayload>>,
}

#[tauri::command]
pub(crate) fn drain_pending_insights_open_conversations(
    state: tauri::State<'_, InsightsOpenConversationState>,
) -> Vec<InsightsOpenConversationPayload> {
    let Ok(mut pending) = state.pending.lock() else {
        return Vec::new();
    };
    pending.drain(..).collect()
}

/// Non-draining peek used by the cold-window route shim: a freshly-opened main
/// window boots on Timeline (`/`), so the layout asks whether a Quick Recall →
/// Chat handoff is queued and, if so, routes to `/insights` — where the Insights
/// surface mounts and drains the queue. Returns `false` if the lock is poisoned.
#[tauri::command]
pub(crate) fn has_pending_insights_open_conversations(
    state: tauri::State<'_, InsightsOpenConversationState>,
) -> bool {
    state
        .pending
        .lock()
        .map(|pending| !pending.is_empty())
        .unwrap_or(false)
}

/// Show + focus the main window, navigate it to the Insights → Chat tab, and
/// select `conversation_id`. Mirrors `open_capture_result_in_main_window`: the
/// payload is emitted as a live event so a warm window reacts immediately, and
/// is queued for a cold-window drain ONLY when the main window isn't already
/// open. Queuing on a warm window would leave the entry stranded — the page
/// doesn't remount, so `drain_pending_insights_open_conversations` never runs,
/// and the next genuine mount would replay every stale handoff and hijack the
/// view onto an old thread. Quick Recall calls this for "Open in Chat".
#[tauri::command]
pub(crate) fn open_conversation_in_chat(
    conversation_id: String,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, InsightsOpenConversationState>,
) {
    let payload = InsightsOpenConversationPayload { conversation_id };
    // A cold main window drains the queue on mount; a warm one is served by the
    // live event alone. Only queue when the window is not yet open so a warm
    // window never accumulates stale entries.
    let main_window_open = app_handle.get_webview_window("main").is_some();
    if !main_window_open {
        if let Ok(mut pending) = state.pending.lock() {
            pending.push_back(payload.clone());
        }
    }
    let _ = windows::open_main_window(&app_handle);
    let _ = app_handle.emit(INSIGHTS_OPEN_CONVERSATION_EVENT, payload);
}

/// Fired the instant a license deep link is routed — before any activation /
/// claim / renewal work runs — so the frontend can raise the deep-link receipt
/// modal in its "working" face right away. Every later transition (activated /
/// pending / over-cap / renewed) rides the existing `license_status` event.
pub(crate) const LICENSE_DEEP_LINK_EVENT: &str = "license_deep_link";

/// Hand-mirrored in `$lib/license-deeplink-receipt.ts` (`DeepLinkFlow`).
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct LicenseDeepLinkPayload {
    /// "activate" | "claim" | "renewed" — the `mnema://license/*` route.
    flow: &'static str,
}

/// Pending license deep-link announcement for a cold main window. Mirrors
/// `InsightsOpenConversationState`: the live event drives a warm window, while
/// a freshly-opened (cold) window — the checkout-bounce path launches the app
/// via the deep link, so the webview isn't listening yet — takes this slot on
/// mount. One slot, latest wins: only the newest deep link deserves a modal.
#[derive(Default)]
pub(crate) struct LicenseDeepLinkState {
    pending: Mutex<Option<LicenseDeepLinkPayload>>,
}

#[tauri::command]
pub(crate) fn take_pending_license_deep_link(
    state: tauri::State<'_, LicenseDeepLinkState>,
) -> Option<LicenseDeepLinkPayload> {
    state
        .pending
        .lock()
        .ok()
        .and_then(|mut pending| pending.take())
}

/// Queue-when-cold + emit, same shape as `open_conversation_in_chat` (queuing
/// on a warm window would strand a stale entry that replays on the next boot).
fn announce_license_deep_link(app_handle: &tauri::AppHandle, flow: &'static str) {
    let payload = LicenseDeepLinkPayload { flow };
    if app_handle.get_webview_window("main").is_none() {
        if let Ok(mut pending) = app_handle.state::<LicenseDeepLinkState>().pending.lock() {
            *pending = Some(payload.clone());
        }
    }
    let _ = app_handle.emit(LICENSE_DEEP_LINK_EVENT, payload);
}

/// Route one deep-link URL to its handler (`app_core::deep_links::route` says
/// which of the five payloads it carries).
pub(crate) fn dispatch_deep_link(app_handle: &tauri::AppHandle, url: &url::Url) {
    let Some(link) = deep_links::route(url) else {
        return;
    };
    if let DeepLink::LicenseActivate { key } = link {
        announce_license_deep_link(app_handle, "activate");
        let app_handle = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            app_core::licensing::activate_from_deep_link(
                app_handle
                    .state::<std::sync::Arc<app_core::AppCore>>()
                    .inner()
                    .clone(),
                key,
            )
            .await;
            let _ = windows::open_main_window(&app_handle);
        });
    } else if let DeepLink::LicenseClaim { checkout_id } = link {
        announce_license_deep_link(app_handle, "claim");
        let app_handle = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            // Surface the app before polling — the buyer was just bounced back
            // from the browser and the claim poll can take ~30s.
            let _ = windows::open_main_window(&app_handle);
            app_core::licensing::claim_from_deep_link(
                app_handle
                    .state::<std::sync::Arc<app_core::AppCore>>()
                    .inner()
                    .clone(),
                checkout_id,
            )
            .await;
        });
    } else if link == DeepLink::LicenseRenewed {
        announce_license_deep_link(app_handle, "renewed");
        let app_handle = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            // Surface the app before polling — the buyer was just bounced back
            // from the browser and the refresh poll can take up to ~60s.
            let _ = windows::open_main_window(&app_handle);
            app_core::licensing::renewed_from_deep_link(
                app_handle
                    .state::<std::sync::Arc<app_core::AppCore>>()
                    .inner()
                    .clone(),
            )
            .await;
        });
    } else if link == DeepLink::OAuthCallback {
        let core = app_handle.state::<std::sync::Arc<app_core::AppCore>>();
        core.state::<app_core::ask_ai::mcp::McpManager>()
            .complete_oauth_callback(&core, url);
    } else {
        enqueue_broker_open_result(app_handle, url);
    }
}

async fn broker_payload_from_url(
    config_dir: &Path,
    url: &url::Url,
) -> Option<BrokerOpenCaptureResultPayload> {
    let opaque_id = deep_links::broker_opaque_id_from_url(url)?;
    let capture_ref = ::app_infra::brokered_access::authorize_active_opaque_capture_reference(
        config_dir, &opaque_id,
    )
    .await
    .ok()
    .flatten()?;
    Some(BrokerOpenCaptureResultPayload {
        opaque_id: capture_ref.opaque_id,
        frame_id: capture_ref.frame_id,
        audio_segment_id: capture_ref.audio_segment_id,
        kind: capture_ref.kind,
        // The broker-URL handoff resolves only a capture reference, so it carries
        // no search-result anchor; the audio receiver falls back to the segment start.
        span_start_ms: None,
        aligned_frame_id: None,
        query: None,
        by_meaning: false,
    })
}

fn enqueue_broker_open_result(app_handle: &tauri::AppHandle, url: &url::Url) {
    let Ok(config_dir) = app_handle.path().app_config_dir() else {
        return;
    };
    if deep_links::broker_opaque_id_from_url(url).is_none() {
        return;
    }
    let url = url.clone();
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let Some(payload) = broker_payload_from_url(&config_dir, &url).await else {
            return;
        };
        if let Ok(mut pending) = app_handle
            .state::<BrokerOpenCaptureResultState>()
            .pending
            .lock()
        {
            pending.push_back(payload.clone());
        }
        let _ = windows::open_main_window(&app_handle);
        let _ = app_handle.emit(BROKER_OPEN_CAPTURE_RESULT_EVENT, payload);
    });
}

#[cfg(test)]
mod tests {
    use super::broker_payload_from_url;

    #[test]
    fn broker_deep_link_rejects_unsigned_opaque_id() {
        let dir = tempfile::tempdir().expect("config dir should be created");
        let url = url::Url::parse("mnema://open/f1").expect("url should parse");

        let payload = tauri::async_runtime::block_on(broker_payload_from_url(dir.path(), &url));

        assert!(payload.is_none());
    }
}
