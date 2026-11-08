//! The Tauri app's side of the `app_core::Host` seam: events go out under their
//! old names and payloads, dialogs through the dialog plugin, windows/tray/exit
//! through the existing `windows`/`status_bar` code. Nothing calls it yet; S2/S3
//! route moved modules through it.

use std::sync::{Arc, OnceLock};

use app_core::{
    AppCore, ConfirmDialog, CoreEvent, DialogKind, ExitKind, Host, MessageDialog, Paths,
    WindowRequest, WindowSignal,
};
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::oneshot;

use crate::{status_bar, windows};
use app_core::debug_log::{log, log_warn};

/// Built before the Tauri app exists (the runtime must be set first), so the
/// handle arrives in `setup` via [`TauriHost::attach`].
#[derive(Default)]
pub(crate) struct TauriHost {
    app: OnceLock<AppHandle>,
}

impl TauriHost {
    pub(crate) fn attach(&self, app: AppHandle) {
        let _ = self.app.set(app);
    }

    fn app(&self) -> Option<&AppHandle> {
        let app = self.app.get();
        if app.is_none() {
            log("app-core host call before the Tauri app was attached; dropped");
        }
        app
    }
}

/// Build the app's `AppCore` and make its runtime Tauri's, before the builder
/// exists so every plugin spawns onto it. Call once, from `run()`.
pub(crate) fn start_core(bundle_id: &str) -> (Arc<TauriHost>, Arc<AppCore>) {
    let host = Arc::new(TauriHost::default());
    let core = AppCore::new(host.clone(), Paths::from_env(bundle_id))
        .expect("failed to start the tokio runtime");
    tauri::async_runtime::set(core.runtime_handle());
    (host, Arc::new(core))
}

/// The frontend's event name and payload for a core event (PLAN appendix A).
fn tauri_event(event: CoreEvent) -> (&'static str, Value) {
    use CoreEvent::*;
    match event {
        AskAiUpdate(p) => ("ask_ai_update", p),
        AudioTranscriptionModelDownloadProgress(p) => {
            ("audio_transcription_model_download_progress", p)
        }
        SpeakerAnalysisModelDownloadProgress(p) => ("speaker_analysis_model_download_progress", p),
        OcrModelDownloadProgress(p) => ("ocr_model_download_progress", p),
        SemanticSearchModelDownloadProgress(p) => ("semantic_search_model_download_progress", p),
        NativeCaptureSessionChanged(p) => ("native_capture_session_changed", p),
        AppUpdateStatusChanged(p) => ("app_update_status_changed", p),
        // Unit events were emitted with `()`, which serializes to `null`.
        ConversationChanged => ("conversation_changed", Value::Null),
        UserContextChanged => ("user_context_changed", Value::Null),
        AudioSegmentsChanged => ("audio_segments_changed", Value::Null),
        TimelineDataChanged(p) => ("timeline_data_changed", p),
        ScrubPreviewCacheChanged(p) => ("scrub_preview_cache_changed", p),
        RecordingSettingsChanged(p) => ("recording_settings_changed", p),
        RecordingSettingsDomainChanged(p) => ("recording_settings_domain_changed", p),
        AppNotificationsChanged(p) => ("app_notifications_changed", p),
        MicrophoneControllerChanged(p) => ("microphone_controller_changed", p),
        MicrophoneAutoDisconnectTransitionFailed(p) => {
            ("microphone_auto_disconnect_transition_failed", p)
        }
        OpenAppNotifications => ("open_app_notifications", Value::Null),
        LicenseStatus(p) => ("license_status", p),
        LicenseDeepLink(p) => ("license_deep_link", p),
        LicenseDeepLinkDone(p) => ("license_deep_link_done", p),
        KeyboardBindingsSettingsChanged(p) => ("keyboard_bindings_settings_changed", p),
        McpAuthorizationChanged(p) => ("mcp_authorization_changed", p),
        ChatgptLoginUpdate(p) => ("chatgpt_login_update", p),
        SystemDidWake => ("system_did_wake", Value::Null),
        ScreensDidSleep => ("screens_did_sleep", Value::Null),
        ScreensDidWake => ("screens_did_wake", Value::Null),
        BrokerOpenCaptureResult(p) => ("broker_open_capture_result", p),
        InsightsOpenConversation(p) => ("insights_open_conversation", p),
    }
}

fn dialog_kind(kind: DialogKind) -> MessageDialogKind {
    match kind {
        DialogKind::Info => MessageDialogKind::Info,
        DialogKind::Warning => MessageDialogKind::Warning,
        DialogKind::Error => MessageDialogKind::Error,
    }
}

impl Host for TauriHost {
    fn emit(&self, event: CoreEvent) {
        let Some(app) = self.app() else { return };
        let (name, payload) = tauri_event(event);
        let _ = app.emit(name, payload);
    }

    fn message(&self, req: MessageDialog) {
        let Some(app) = self.app() else { return };
        app.dialog()
            .message(req.message)
            .kind(dialog_kind(req.kind))
            .title(req.title)
            .show(|_| {});
    }

    fn confirm(&self, req: ConfirmDialog) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        // No app yet: `tx` drops here, which the caller reads as cancel.
        let Some(app) = self.app() else { return rx };
        let mut dialog = app
            .dialog()
            .message(req.message)
            .kind(dialog_kind(req.kind))
            .buttons(MessageDialogButtons::OkCancelCustom(
                req.ok_label,
                req.cancel_label,
            ));
        if let Some(title) = req.title {
            dialog = dialog.title(title);
        }
        dialog.show(move |ok| {
            let _ = tx.send(ok);
        });
        rx
    }

    fn open_url(&self, url: &str) {
        let Some(app) = self.app() else { return };
        if let Err(error) = app.opener().open_url(url, None::<String>) {
            log_warn(format!("failed to open a URL: {error}"));
        }
    }

    fn refresh_tray(&self) {
        let Some(app) = self.app() else { return };
        status_bar::refresh(app);
    }

    fn open_window(&self, req: WindowRequest) {
        let Some(app) = self.app() else { return };
        let result = match req {
            WindowRequest::Main => windows::open_main_window(app),
            WindowRequest::Settings { tab, focus } => {
                windows::focus_main_and_open_settings(app.clone(), tab, focus)
            }
            WindowRequest::CliAccessRequest => windows::open_cli_access_request_window(app),
            WindowRequest::Update => windows::open_update_window(app),
            WindowRequest::PermissionHelper { kind } => {
                windows::permission_helper::show_permission_helper(app.clone(), kind)
            }
        };
        if let Err(error) = result {
            log_warn(format!("failed to open a window: {error}"));
        }
    }

    fn window_signal(&self, signal: WindowSignal) {
        let Some(app) = self.app() else { return };
        match signal {
            WindowSignal::CaptureExclusionChanged => windows::refresh_capture_exclusion(app),
            WindowSignal::HideAllViews(hidden) => {
                windows::set_all_webview_views_hidden(app, hidden)
            }
        }
    }

    fn exit(&self, kind: ExitKind) {
        let Some(app) = self.app() else { return };
        match kind {
            ExitKind::Quit => windows::request_graceful_exit(app),
            ExitKind::Restart => windows::request_app_relaunch(app.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// One of each variant. `tauri_event`'s exhaustive match breaks the build on a
    /// new variant; add it here and to `APPENDIX_A` too.
    fn every_event() -> Vec<CoreEvent> {
        use CoreEvent::*;
        let p = || json!({ "k": 1 });
        vec![
            AskAiUpdate(p()),
            AudioTranscriptionModelDownloadProgress(p()),
            SpeakerAnalysisModelDownloadProgress(p()),
            OcrModelDownloadProgress(p()),
            SemanticSearchModelDownloadProgress(p()),
            NativeCaptureSessionChanged(p()),
            AppUpdateStatusChanged(p()),
            ConversationChanged,
            UserContextChanged,
            AudioSegmentsChanged,
            TimelineDataChanged(p()),
            ScrubPreviewCacheChanged(p()),
            RecordingSettingsChanged(p()),
            RecordingSettingsDomainChanged(p()),
            AppNotificationsChanged(p()),
            MicrophoneControllerChanged(p()),
            MicrophoneAutoDisconnectTransitionFailed(p()),
            OpenAppNotifications,
            LicenseStatus(p()),
            LicenseDeepLink(p()),
            LicenseDeepLinkDone(p()),
            KeyboardBindingsSettingsChanged(p()),
            McpAuthorizationChanged(p()),
            ChatgptLoginUpdate(p()),
            SystemDidWake,
            ScreensDidSleep,
            ScreensDidWake,
            BrokerOpenCaptureResult(p()),
            InsightsOpenConversation(p()),
        ]
    }

    /// The 29 non-window-targeted event names the Svelte frontend listens for.
    const APPENDIX_A: [&str; 29] = [
        "ask_ai_update",
        "audio_transcription_model_download_progress",
        "speaker_analysis_model_download_progress",
        "ocr_model_download_progress",
        "semantic_search_model_download_progress",
        "native_capture_session_changed",
        "app_update_status_changed",
        "conversation_changed",
        "user_context_changed",
        "audio_segments_changed",
        "timeline_data_changed",
        "scrub_preview_cache_changed",
        "recording_settings_changed",
        "recording_settings_domain_changed",
        "app_notifications_changed",
        "microphone_controller_changed",
        "microphone_auto_disconnect_transition_failed",
        "open_app_notifications",
        "license_status",
        "license_deep_link",
        "license_deep_link_done",
        "keyboard_bindings_settings_changed",
        "mcp_authorization_changed",
        "chatgpt_login_update",
        "system_did_wake",
        "screens_did_sleep",
        "screens_did_wake",
        "broker_open_capture_result",
        "insights_open_conversation",
    ];

    #[test]
    fn every_core_event_maps_to_exactly_its_old_event_name() {
        let names: Vec<&str> = every_event()
            .into_iter()
            .map(|event| tauri_event(event).0)
            .collect();
        assert_eq!(names, APPENDIX_A);
    }

    #[test]
    fn event_names_match_the_constants_the_old_emit_sites_use() {
        use crate::*;
        let constants = [
            app_updates::APP_UPDATE_STATUS_CHANGED_EVENT,
            app_infra::TIMELINE_DATA_CHANGED_EVENT,
            app_infra::frame_preview::SCRUB_PREVIEW_CACHE_CHANGED_EVENT,
            audio_transcription_models::AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT,
            conversation::commands::CONVERSATION_CHANGED_EVENT,
            licensing::LICENSE_DEEP_LINK_DONE_EVENT,
            licensing::LICENSE_STATUS_EVENT,
            native_capture::APP_NOTIFICATIONS_CHANGED_EVENT,
            native_capture::AUDIO_SEGMENTS_CHANGED_EVENT,
            native_capture::NATIVE_CAPTURE_SESSION_CHANGED_EVENT,
            native_capture::RECORDING_SETTINGS_CHANGED_EVENT,
            native_capture::RECORDING_SETTINGS_DOMAIN_CHANGED_EVENT,
            native_capture::SCREENS_DID_SLEEP_EVENT,
            native_capture::SCREENS_DID_WAKE_EVENT,
            native_capture::SYSTEM_DID_WAKE_EVENT,
            ocr_models::OCR_MODEL_DOWNLOAD_PROGRESS_EVENT,
            semantic_search_models::SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT,
            speaker_analysis_models::SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT,
            user_context::worker::USER_CONTEXT_CHANGED_EVENT,
            ask_ai::mcp::oauth_flow::MCP_AUTHORIZATION_CHANGED_EVENT,
            BROKER_OPEN_CAPTURE_RESULT_EVENT,
            INSIGHTS_OPEN_CONVERSATION_EVENT,
            LICENSE_DEEP_LINK_EVENT,
        ];
        for constant in constants {
            assert!(
                APPENDIX_A.contains(&constant),
                "{constant} has no CoreEvent"
            );
        }
    }

    #[test]
    fn payloads_pass_through_and_unit_events_emit_null() {
        let payload = json!({ "conversationId": "c1", "version": 3 });
        assert_eq!(
            tauri_event(CoreEvent::AskAiUpdate(payload.clone())),
            ("ask_ai_update", payload)
        );
        assert_eq!(
            serde_json::to_value(()).unwrap(),
            tauri_event(CoreEvent::ConversationChanged).1
        );
    }
}
