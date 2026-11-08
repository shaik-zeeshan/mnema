//! Deep links (`mnema://…`, `mnema-dev://…`) and the handoff queues they and
//! other windows fill for the main window: broker open-capture results, Quick
//! Recall → Chat conversations, and license activate/claim/renewed links. The
//! event fires for a live listener; the queue covers a window that mounts later.

use std::{collections::VecDeque, path::Path, sync::Mutex};

use tauri::{Emitter, Manager};

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

fn broker_opaque_id_from_url(url: &url::Url) -> Option<String> {
    if url.scheme() != "mnema" {
        return None;
    }
    let mut segments = url.path_segments()?.collect::<Vec<_>>();
    if let Some(host) = url.host_str() {
        segments.insert(0, host);
    }
    match segments.as_slice() {
        ["open", opaque_id] | ["broker", "open", opaque_id] => Some((*opaque_id).to_string()),
        _ => return None,
    }
}

/// Whether `url` is our MCP OAuth deep-link callback (`mnema://oauth/callback` in
/// prod, `mnema-dev://oauth/callback` in dev). A custom-scheme URL parses its first
/// path element as the HOST, so the callback lands as host `oauth` + path
/// `/callback`. Broker deep links use host `open`/`broker`, so the two never
/// collide — this is the dispatch discriminator in `on_open_url`.
fn is_oauth_callback_url(url: &url::Url) -> bool {
    matches!(url.scheme(), "mnema" | "mnema-dev")
        && url.host_str() == Some("oauth")
        && url.path() == "/callback"
}

/// The license-activation deep link (`mnema://license/activate?key=…` in prod,
/// `mnema-dev://…` in dev). Host `license` + path `/activate` is distinct from the
/// oauth (`oauth`) and broker (`open`/`broker`) hosts, so the three never collide.
/// Returns the URL-decoded key (standard base64 `.`-joined, as minted).
fn license_key_from_url(url: &url::Url) -> Option<String> {
    if !matches!(url.scheme(), "mnema" | "mnema-dev") {
        return None;
    }
    if url.host_str() != Some("license") || url.path() != "/activate" {
        return None;
    }
    url.query_pairs()
        .find(|(k, _)| k == "key")
        .map(|(_, v)| v.into_owned())
        .filter(|k| !k.is_empty())
}

/// The purchase-claim deep link (`mnema://license/claim?checkout_id=…` in prod,
/// `mnema-dev://…` in dev) — Polar's success redirect. Host `license` + path
/// `/claim` sits beside the activate route and apart from the oauth/broker hosts.
fn claim_checkout_id_from_url(url: &url::Url) -> Option<String> {
    if !matches!(url.scheme(), "mnema" | "mnema-dev") {
        return None;
    }
    if url.host_str() != Some("license") || url.path() != "/claim" {
        return None;
    }
    url.query_pairs()
        .find(|(k, _)| k == "checkout_id")
        .map(|(_, v)| v.into_owned())
        .filter(|id| !id.is_empty())
}

/// The renewal-return deep link (`mnema://license/renewed` in prod,
/// `mnema-dev://…` in dev) — the Polar RENEWAL product's success redirect. No
/// payload: the machine already holds the key (a renewal extends the existing
/// license, ADR 0055), so the hit only cues a short Receipt Refresh poll.
/// Ops note: the Polar renewal product's success URL must point at
/// `https://mnema.day/license/open?flow=renewal&checkout_id={CHECKOUT_ID}` — the
/// bounce page that fires this deep link (providers can't emit custom schemes).
fn is_license_renewed_url(url: &url::Url) -> bool {
    matches!(url.scheme(), "mnema" | "mnema-dev")
        && url.host_str() == Some("license")
        && url.path() == "/renewed"
}

/// Route one deep-link URL to its handler. The `mnema`/`mnema-dev` scheme carries
/// five unrelated payloads (license activation, purchase claim, renewal return,
/// MCP OAuth callback, capture-broker handoff), discriminated by host+path so
/// none swallows another.
pub(crate) fn dispatch_deep_link(app_handle: &tauri::AppHandle, url: &url::Url) {
    if let Some(key) = license_key_from_url(url) {
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
    } else if let Some(checkout_id) = claim_checkout_id_from_url(url) {
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
    } else if is_license_renewed_url(url) {
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
    } else if is_oauth_callback_url(url) {
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
    let opaque_id = broker_opaque_id_from_url(url)?;
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
    if broker_opaque_id_from_url(url).is_none() {
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
    use super::{
        broker_opaque_id_from_url, broker_payload_from_url, claim_checkout_id_from_url,
        is_license_renewed_url, is_oauth_callback_url, license_key_from_url,
    };

    /// The deep-link dispatch discriminator: an OAuth callback and a capture-broker
    /// handoff share the `mnema`/`mnema-dev` scheme, so they must NOT swallow each
    /// other. `is_oauth_callback_url` claims only the `oauth/callback` host+path,
    /// and `broker_opaque_id_from_url` claims only the broker hosts — in BOTH
    /// directions.
    #[test]
    fn oauth_callback_and_broker_urls_dispatch_apart() {
        let prod = url::Url::parse("mnema://oauth/callback?code=abc&state=xyz").expect("url");
        let dev = url::Url::parse("mnema-dev://oauth/callback?code=abc&state=xyz").expect("url");
        assert!(is_oauth_callback_url(&prod));
        assert!(is_oauth_callback_url(&dev));

        let broker_open = url::Url::parse("mnema://open/f1").expect("url");
        let broker_nested = url::Url::parse("mnema://broker/open/f1").expect("url");
        // A broker URL is never read as an oauth callback…
        assert!(!is_oauth_callback_url(&broker_open));
        assert!(!is_oauth_callback_url(&broker_nested));
        // …and an oauth callback is never read as a broker URL (no cross-swallow).
        assert!(broker_opaque_id_from_url(&prod).is_none());
        assert_eq!(
            broker_opaque_id_from_url(&broker_open).as_deref(),
            Some("f1")
        );
        assert_eq!(
            broker_opaque_id_from_url(&broker_nested).as_deref(),
            Some("f1")
        );
    }

    #[test]
    fn license_deep_link_extracts_url_decoded_key_and_stays_apart() {
        // Standard base64 (`+ / =`) joined by `.`, percent-encoded in the URL.
        let key = "eyJlbWFpbCI6ImFAYi5jbyJ9.sig+with/slash=";
        let url = url::Url::parse(&format!(
            "mnema://license/activate?key={}",
            "eyJlbWFpbCI6ImFAYi5jbyJ9.sig%2Bwith%2Fslash%3D"
        ))
        .expect("url");
        assert_eq!(license_key_from_url(&url).as_deref(), Some(key));

        // Never confused for oauth or broker; never claims their URLs.
        assert!(!is_oauth_callback_url(&url));
        assert!(broker_opaque_id_from_url(&url).is_none());
        let oauth = url::Url::parse("mnema://oauth/callback?code=x").expect("url");
        let broker = url::Url::parse("mnema://open/f1").expect("url");
        assert!(license_key_from_url(&oauth).is_none());
        assert!(license_key_from_url(&broker).is_none());
        // Missing/empty key -> not an activation link.
        assert!(
            license_key_from_url(&url::Url::parse("mnema://license/activate").expect("url"))
                .is_none()
        );
    }

    #[test]
    fn claim_deep_link_extracts_checkout_id_and_stays_apart() {
        let prod = url::Url::parse("mnema://license/claim?checkout_id=co_abc123").expect("url");
        let dev = url::Url::parse("mnema-dev://license/claim?checkout_id=co_abc123").expect("url");
        assert_eq!(
            claim_checkout_id_from_url(&prod).as_deref(),
            Some("co_abc123")
        );
        assert_eq!(
            claim_checkout_id_from_url(&dev).as_deref(),
            Some("co_abc123")
        );

        // Never claims the activate/oauth/broker payloads…
        let activate = url::Url::parse("mnema://license/activate?key=abc.def").expect("url");
        let oauth = url::Url::parse("mnema://oauth/callback?code=x").expect("url");
        let broker = url::Url::parse("mnema://open/f1").expect("url");
        assert!(claim_checkout_id_from_url(&activate).is_none());
        assert!(claim_checkout_id_from_url(&oauth).is_none());
        assert!(claim_checkout_id_from_url(&broker).is_none());
        // …and none of them claims a claim URL (no cross-swallow).
        assert!(license_key_from_url(&prod).is_none());
        assert!(!is_oauth_callback_url(&prod));
        assert!(broker_opaque_id_from_url(&prod).is_none());

        // Missing/empty checkout_id -> not a claim link. Wrong scheme neither.
        assert!(claim_checkout_id_from_url(
            &url::Url::parse("mnema://license/claim").expect("url")
        )
        .is_none());
        assert!(claim_checkout_id_from_url(
            &url::Url::parse("mnema://license/claim?checkout_id=").expect("url")
        )
        .is_none());
        assert!(claim_checkout_id_from_url(
            &url::Url::parse("https://license/claim?checkout_id=co_abc123").expect("url")
        )
        .is_none());
    }

    #[test]
    fn renewed_deep_link_matches_both_schemes_and_stays_apart() {
        let prod = url::Url::parse("mnema://license/renewed").expect("url");
        let dev = url::Url::parse("mnema-dev://license/renewed").expect("url");
        assert!(is_license_renewed_url(&prod));
        assert!(is_license_renewed_url(&dev));
        // A stray query string (checkout providers append them) still matches.
        assert!(is_license_renewed_url(
            &url::Url::parse("mnema://license/renewed?checkout_id=co_x").expect("url")
        ));

        // Never claims the activate/claim/oauth/broker payloads…
        let activate = url::Url::parse("mnema://license/activate?key=abc.def").expect("url");
        let claim = url::Url::parse("mnema://license/claim?checkout_id=co_x").expect("url");
        let oauth = url::Url::parse("mnema://oauth/callback?code=x").expect("url");
        let broker = url::Url::parse("mnema://open/f1").expect("url");
        assert!(!is_license_renewed_url(&activate));
        assert!(!is_license_renewed_url(&claim));
        assert!(!is_license_renewed_url(&oauth));
        assert!(!is_license_renewed_url(&broker));
        // …and none of them claims a renewed URL (no cross-swallow).
        assert!(license_key_from_url(&prod).is_none());
        assert!(claim_checkout_id_from_url(&prod).is_none());
        assert!(!is_oauth_callback_url(&prod));
        assert!(broker_opaque_id_from_url(&prod).is_none());

        // Wrong scheme -> not a renewed link.
        assert!(!is_license_renewed_url(
            &url::Url::parse("https://license/renewed").expect("url")
        ));
    }

    #[test]
    fn broker_deep_link_rejects_unsigned_opaque_id() {
        let dir = tempfile::tempdir().expect("config dir should be created");
        let url = url::Url::parse("mnema://open/f1").expect("url should parse");

        let payload = tauri::async_runtime::block_on(broker_payload_from_url(dir.path(), &url));

        assert!(payload.is_none());
    }
}
