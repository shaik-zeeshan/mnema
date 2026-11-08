//! Every app-wide event shared behaviour raises (PLAN appendix A). Window-targeted
//! events (`open_settings_tab`, `quick_recall_dismissed`, `quick_recall_query`)
//! stay host-side and are not variants.
//!
//! shortcut: payloads are the event's JSON exactly as the Tauri app emits it today,
//! because the payload types still live in src-tauri. Each one becomes a typed
//! field when S2/S3 move its type into a crate.

use serde_json::Value;

use crate::app_infra::TimelineDataChangedPayload;
use crate::audio_transcription_models::AudioTranscriptionModelDownloadProgressDto;
use crate::ocr_models::OcrModelDownloadProgressDto;
use crate::speaker_analysis_models::SpeakerAnalysisModelDownloadProgressDto;
use crate::semantic_search_models::SemanticSearchModelDownloadProgressDto;
use crate::updates::AppUpdateStatus;

#[derive(Debug, Clone, PartialEq)]
pub enum CoreEvent {
    // Streams.
    AskAiUpdate(Value),
    AudioTranscriptionModelDownloadProgress(AudioTranscriptionModelDownloadProgressDto),
    SpeakerAnalysisModelDownloadProgress(SpeakerAnalysisModelDownloadProgressDto),
    OcrModelDownloadProgress(OcrModelDownloadProgressDto),
    SemanticSearchModelDownloadProgress(SemanticSearchModelDownloadProgressDto),
    NativeCaptureSessionChanged(Value),
    AppUpdateStatusChanged(AppUpdateStatus),
    // Invalidations.
    ConversationChanged,
    UserContextChanged,
    AudioSegmentsChanged,
    TimelineDataChanged(TimelineDataChangedPayload),
    ScrubPreviewCacheChanged(Value),
    RecordingSettingsChanged(Value),
    RecordingSettingsDomainChanged(Value),
    AppNotificationsChanged(Value),
    MicrophoneControllerChanged(Value),
    MicrophoneAutoDisconnectTransitionFailed(Value),
    OpenAppNotifications,
    LicenseStatus(Value),
    LicenseDeepLink(Value),
    LicenseDeepLinkDone(Value),
    KeyboardBindingsSettingsChanged(Value),
    McpAuthorizationChanged(Value),
    ChatgptLoginUpdate(Value),
    SystemDidWake,
    ScreensDidSleep,
    ScreensDidWake,
    BrokerOpenCaptureResult(Value),
    InsightsOpenConversation(Value),
}
