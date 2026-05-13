mod activity;
#[path = "native_capture_debug_log.rs"]
pub(crate) mod debug_log;
#[path = "native_capture_inactivity.rs"]
pub(crate) mod inactivity;
mod lifecycle;
#[path = "native_capture_metadata.rs"]
pub(crate) mod metadata;
mod microphone;
#[path = "native_capture_output.rs"]
pub(crate) mod output;
mod privacy;
mod runtime;
mod segments;
#[path = "native_capture_settings.rs"]
pub(crate) mod settings;
#[path = "native_capture_system_idle.rs"]
pub(crate) mod system_idle;
#[cfg(test)]
mod tests;

use capture_microphone as microphone_capture;
use capture_types::{
    AudioTranscriptionProvider, AudioTranscriptionSettings, CaptureErrorResponse,
    CaptureOutputFiles, CapturePermissionState, CapturePermissions, CapturePermissionsResponse,
    CaptureSources, CaptureSupportResponse, InactivityActivityMode, MicrophoneControllerState,
    NativeCaptureDebugLogStatus, NativeCaptureSessionResponse, OcrProvider, OcrSettings,
    RecordingSettings, ScreenResolution, ScreenResolutionPreset, StartNativeCaptureRequest,
    UpdateMicrophoneControllerRequest, UpdateRecordingSettingsRequest, VideoBitrateMode,
    VideoBitratePreset, VideoBitrateSettings,
};
use capture_vad::configured_adapter_as_str;
use settings::{
    apply_recording_settings_update, current_auto_start,
    current_native_capture_debug_logging_enabled, current_recording_settings,
    initialize_recording_settings_state_from_disk,
};
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
#[cfg(target_os = "macos")]
use std::time::Duration;
use tauri::{Emitter, Manager};

pub use capture_types::IdleDebugInfo;
pub(crate) use debug_log::install_panic_hook;
use lifecycle::{RecordingLifecycle, StartRecordingLifecycleOutcome};
use microphone::{
    resolve_capture_microphone_device_id, should_wait_for_same_microphone_device,
    update_microphone_controller as update_microphone_controller_impl,
};
pub use microphone::{
    start_microphone_device_change_notifier, MicrophoneControllerPreferencesState,
    MicrophoneDeviceChangeNotifierState,
};
use runtime::validate_start_request;
pub type NativeCaptureState = Mutex<RecordingLifecycle>;
pub use settings::RecordingSettingsState;
// Re-exported so adapter-level Tauri commands (e.g. `open_debug_window`) can
// read the persisted recording settings through the same seam used by the
// rest of `native_capture` without bypassing it to touch persistence directly.
pub(crate) use settings::current_recording_settings as read_recording_settings;

#[cfg(target_os = "macos")]
#[derive(Default)]
pub struct SystemWakeNotifierState(std::sync::Mutex<Vec<cidre::ns::NotificationGuard>>);

#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct SystemWakeNotifierState(std::sync::Mutex<Vec<()>>);

#[cfg(target_os = "macos")]
#[derive(Default)]
pub struct MetadataNotifierState(std::sync::Mutex<Vec<cidre::ns::NotificationGuard>>);

#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct MetadataNotifierState(std::sync::Mutex<Vec<()>>);

#[cfg(target_os = "macos")]
impl MetadataNotifierState {
    pub(crate) fn replace(&self, guards: Vec<cidre::ns::NotificationGuard>) {
        *self.0.lock().expect("metadata notifier state poisoned") = guards;
    }
}

#[cfg(not(target_os = "macos"))]
impl MetadataNotifierState {
    pub(crate) fn replace(&self, guards: Vec<()>) {
        *self.0.lock().expect("metadata notifier state poisoned") = guards;
    }
}

pub const SYSTEM_DID_WAKE_EVENT: &str = "system_did_wake";
#[cfg(target_os = "macos")]
const SYSTEM_WAKE_RECOVERY_RETRY_DELAYS_MS: &[u64] = &[500, 1_500, 3_000];
pub const AUDIO_SEGMENTS_CHANGED_EVENT: &str = "audio_segments_changed";
pub const RECORDING_SETTINGS_CHANGED_EVENT: &str = "recording_settings_changed";
pub const NATIVE_CAPTURE_SESSION_CHANGED_EVENT: &str = "native_capture_session_changed";
pub const APP_NOTIFICATIONS_CHANGED_EVENT: &str = "app_notifications_changed";
const AUDIO_TRANSCRIPTION_UNAVAILABLE_NOTIFICATION_ID: &str = "audio-transcription-unavailable";
const OCR_UNAVAILABLE_NOTIFICATION_ID: &str = "ocr-unavailable";
const SPEECH_DETECTOR_UNAVAILABLE_NOTIFICATION_ID: &str = "speech-detector-unavailable";
const SPEAKER_ANALYSIS_UNAVAILABLE_NOTIFICATION_ID: &str = "speaker-analysis-unavailable";
const PRIVACY_RECOVERY_RESTART_REQUIRED_NOTIFICATION_ID: &str = "privacy-recovery-restart-required";
const PROCESSING_SETTINGS_TAB_ID: &str = "processing";

#[derive(Debug, Clone, serde::Serialize)]
#[allow(dead_code)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppNotificationAction {
    OpenSettingsTab { tab: String },
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppNotification {
    pub id: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    pub created_at_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<AppNotificationAction>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyAppCandidate {
    pub bundle_id: String,
    pub display_name: String,
    pub running: bool,
    pub icon_path: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckBrowserUrlSupportRequest {
    pub bundle_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserUrlSupportResponse {
    pub bundle_id: String,
    pub supported: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePrivacyDebugResponse {
    pub metadata_enabled: bool,
    pub browser_url_mode: capture_metadata::BrowserUrlMode,
    pub private_browser_exclusion_enabled: bool,
    pub privacy_debug: metadata::CapturePrivacyDebugInfo,
}

#[derive(Debug, Default)]
pub struct AppNotificationsRuntime {
    notifications: Vec<AppNotification>,
}

impl AppNotificationsRuntime {
    fn list(&self) -> Vec<AppNotification> {
        self.notifications.clone()
    }

    fn push_session_notification(&mut self, notification: AppNotification) -> Vec<AppNotification> {
        self.notifications.retain(|item| item.id != notification.id);
        self.notifications.push(notification);
        self.list()
    }

    fn clear_one(&mut self, id: &str) -> Vec<AppNotification> {
        self.notifications.retain(|item| item.id != id);
        self.list()
    }

    fn clear_all(&mut self) -> Vec<AppNotification> {
        self.notifications.clear();
        self.list()
    }
}

pub type AppNotificationsState = Mutex<AppNotificationsRuntime>;
pub use metadata::{start_metadata_notifier, CaptureMetadataState};

#[tauri::command]
pub async fn list_privacy_app_candidates() -> Result<Vec<PrivacyAppCandidate>, String> {
    let mut candidates = vec![PrivacyAppCandidate {
        bundle_id: "com.shaikzeeshan.mnema".to_string(),
        display_name: "Mnema".to_string(),
        running: true,
        icon_path: None,
    }];

    #[cfg(target_os = "macos")]
    {
        candidates.extend(
            [
                ("com.1password.1password", "1Password"),
                ("org.signal.Signal", "Signal"),
                ("com.apple.Notes", "Notes"),
                ("com.apple.mail", "Mail"),
                ("com.apple.MobileSMS", "Messages"),
                ("net.whatsapp.WhatsApp", "WhatsApp"),
            ]
            .into_iter()
            .map(|(bundle_id, display_name)| PrivacyAppCandidate {
                bundle_id: bundle_id.to_string(),
                display_name: display_name.to_string(),
                running: false,
                icon_path: None,
            }),
        );
    }

    Ok(candidates)
}

#[tauri::command]
pub async fn check_browser_url_support(
    request: CheckBrowserUrlSupportRequest,
) -> Result<BrowserUrlSupportResponse, String> {
    let supported = capture_metadata::browser_url_metadata_supported(&request.bundle_id);
    Ok(BrowserUrlSupportResponse {
        bundle_id: request.bundle_id,
        supported,
        warning: (!supported).then(|| {
            "URL metadata support is unknown for this browser; browsing will still be recorded."
                .to_string()
        }),
    })
}

#[tauri::command]
pub fn get_capture_privacy_debug(
    metadata_state: tauri::State<'_, CaptureMetadataState>,
    settings_state: tauri::State<'_, RecordingSettingsState>,
) -> CapturePrivacyDebugResponse {
    let settings = current_recording_settings(settings_state.inner());
    CapturePrivacyDebugResponse {
        metadata_enabled: settings.metadata.enabled,
        browser_url_mode: settings.metadata.browser_url_mode,
        private_browser_exclusion_enabled: settings.privacy.private_browser_exclusion_enabled,
        privacy_debug: metadata::capture_privacy_debug_info(metadata_state.inner()),
    }
}

fn emit_system_did_wake(app_handle: &tauri::AppHandle) {
    let _ = app_handle.emit(SYSTEM_DID_WAKE_EVENT, ());
}

pub(super) fn emit_audio_segments_changed(app_handle: &tauri::AppHandle) {
    let _ = app_handle.emit(AUDIO_SEGMENTS_CHANGED_EVENT, ());
}

fn emit_recording_settings_changed(app_handle: &tauri::AppHandle, settings: &RecordingSettings) {
    let _ = app_handle.emit(RECORDING_SETTINGS_CHANGED_EVENT, settings);
}

fn emit_native_capture_session_changed(
    app_handle: &tauri::AppHandle,
    session: &capture_types::NativeCaptureSession,
) {
    let _ = app_handle.emit(NATIVE_CAPTURE_SESSION_CHANGED_EVENT, session);
}

fn emit_app_notifications_changed(
    app_handle: &tauri::AppHandle,
    notifications: &[AppNotification],
) {
    let _ = app_handle.emit(APP_NOTIFICATIONS_CHANGED_EVENT, notifications);
}

fn push_app_notification(
    app_handle: &tauri::AppHandle,
    state: &AppNotificationsState,
    notification: AppNotification,
) {
    let notifications = {
        let mut runtime = state.lock().expect("app notifications state poisoned");
        runtime.push_session_notification(notification)
    };
    emit_app_notifications_changed(app_handle, &notifications);
}

pub(super) fn push_privacy_recovery_restart_required_notification(app_handle: &tauri::AppHandle) {
    let Some(state) = app_handle.try_state::<AppNotificationsState>() else {
        debug_log::log_warn(
            "app notifications state unavailable while reporting privacy recovery restart requirement",
        );
        return;
    };
    push_app_notification(
        app_handle,
        state.inner(),
        AppNotification {
            id: PRIVACY_RECOVERY_RESTART_REQUIRED_NOTIFICATION_ID.to_string(),
            severity: "warning".to_string(),
            title: "Screen capture paused for privacy".to_string(),
            message: "Screen and system audio capture were paused after privacy filter recovery failed. Stop and start recording to resume those sources.".to_string(),
            created_at_unix_ms: runtime::now_unix_ms(),
            action: None,
        },
    );
}

pub(crate) fn push_warning_app_notification(
    app_handle: &tauri::AppHandle,
    id: &str,
    title: &str,
    message: &str,
    settings_tab: Option<&str>,
    created_at_unix_ms: u64,
) {
    let action = settings_tab.map(|tab| AppNotificationAction::OpenSettingsTab {
        tab: tab.to_string(),
    });
    push_app_notification(
        app_handle,
        app_handle.state::<AppNotificationsState>().inner(),
        AppNotification {
            id: id.to_string(),
            severity: "warning".to_string(),
            title: title.to_string(),
            message: message.to_string(),
            created_at_unix_ms,
            action,
        },
    );
}

fn should_warn_audio_transcription_unavailable_at_start(settings: &RecordingSettings) -> bool {
    settings.transcription.enabled
        && ((settings.capture_microphone && settings.transcription.microphone_enabled)
            || (settings.capture_system_audio && settings.transcription.system_audio_enabled))
}

fn should_warn_audio_transcription_unavailable_at_startup(settings: &RecordingSettings) -> bool {
    should_warn_audio_transcription_unavailable_at_start(settings)
}

fn audio_transcription_provider_label(provider: AudioTranscriptionProvider) -> &'static str {
    match provider {
        AudioTranscriptionProvider::LocalWhisper => "Local Whisper",
        AudioTranscriptionProvider::AppleSpeechOnDevice => "Apple Speech on-device recognition",
        AudioTranscriptionProvider::Parakeet => "Parakeet",
    }
}

fn audio_transcription_selection_label(settings: &AudioTranscriptionSettings) -> String {
    let provider = audio_transcription_provider_label(settings.provider);
    match settings.model_id.as_deref() {
        Some(model_id) if !model_id.is_empty() => format!("{provider} `{model_id}`"),
        _ => provider.to_string(),
    }
}

fn audio_transcription_unavailable_notification(
    settings: &RecordingSettings,
    created_at_unix_ms: u64,
) -> AppNotification {
    let selection = audio_transcription_selection_label(&settings.transcription);
    AppNotification {
        id: AUDIO_TRANSCRIPTION_UNAVAILABLE_NOTIFICATION_ID.to_string(),
        severity: "warning".to_string(),
        title: "Transcription model unavailable".to_string(),
        message: format!(
            "{selection} is not available. Requested audio will not be transcribed until you install or choose an available model."
        ),
        created_at_unix_ms,
        action: Some(AppNotificationAction::OpenSettingsTab {
            tab: PROCESSING_SETTINGS_TAB_ID.to_string(),
        }),
    }
}

fn speech_detector_unavailable_notification(created_at_unix_ms: u64) -> AppNotification {
    AppNotification {
        id: SPEECH_DETECTOR_UNAVAILABLE_NOTIFICATION_ID.to_string(),
        severity: "warning".to_string(),
        title: "Speech detector unavailable".to_string(),
        message: "The selected speech detector is unavailable. Choose an available detector before starting this recording.".to_string(),
        created_at_unix_ms,
        action: Some(AppNotificationAction::OpenSettingsTab {
            tab: PROCESSING_SETTINGS_TAB_ID.to_string(),
        }),
    }
}

fn speaker_analysis_unavailable_notification(created_at_unix_ms: u64) -> AppNotification {
    AppNotification {
        id: SPEAKER_ANALYSIS_UNAVAILABLE_NOTIFICATION_ID.to_string(),
        severity: "warning".to_string(),
        title: "Speaker analysis model unavailable".to_string(),
        message: "The selected speaker analysis model is unavailable. Install or choose an available model before starting this recording.".to_string(),
        created_at_unix_ms,
        action: Some(AppNotificationAction::OpenSettingsTab {
            tab: PROCESSING_SETTINGS_TAB_ID.to_string(),
        }),
    }
}

fn maybe_push_audio_transcription_unavailable_warning(
    app_handle: &tauri::AppHandle,
    app_notifications_state: &AppNotificationsState,
    settings: &RecordingSettings,
    context: &str,
) {
    let app_data_dir = match app_handle.path().app_data_dir() {
        Ok(dir) => dir,
        Err(error) => {
            debug_log::log_warn(format!(
                "failed to resolve app data directory for {context} audio transcription warning: {error}"
            ));
            return;
        }
    };

    match crate::audio_transcription_models::selected_audio_transcription_model_available(
        &app_data_dir,
        &settings.transcription,
    ) {
        Ok(true) => {}
        Ok(false) => {
            let selection = audio_transcription_selection_label(&settings.transcription);
            debug_log::log_warn(format!(
                "audio transcription unavailable at {context} (selection={selection})"
            ));
            push_app_notification(
                app_handle,
                app_notifications_state,
                audio_transcription_unavailable_notification(settings, runtime::now_unix_ms()),
            );
        }
        Err(error) => {
            let selection = audio_transcription_selection_label(&settings.transcription);
            debug_log::log_warn(format!(
                "failed to inspect selected audio transcription model at {context} (selection={selection}): {error}"
            ));
            push_app_notification(
                app_handle,
                app_notifications_state,
                audio_transcription_unavailable_notification(settings, runtime::now_unix_ms()),
            );
        }
    }
}

fn maybe_push_audio_transcription_unavailable_start_warning(
    app_handle: &tauri::AppHandle,
    app_notifications_state: &AppNotificationsState,
    settings: &RecordingSettings,
) {
    if !should_warn_audio_transcription_unavailable_at_start(settings) {
        return;
    }

    maybe_push_audio_transcription_unavailable_warning(
        app_handle,
        app_notifications_state,
        settings,
        "recording start",
    );
}

fn recording_requires_speech_detector(settings: &RecordingSettings) -> bool {
    settings.audio_speech_detection.detector != capture_types::AudioSpeechDetector::Off
        && settings.capture_system_audio
        && settings.transcription.enabled
        && settings.transcription.system_audio_enabled
}

fn selected_speech_detector_available(settings: &RecordingSettings) -> Result<bool, String> {
    if settings.audio_speech_detection.detector == capture_types::AudioSpeechDetector::Off {
        return Ok(false);
    }
    capture_vad::AudioSpeechDetectorRuntime::new(settings.audio_speech_detection.detector)
        .map(|_| true)
        .map_err(|error| error.to_string())
}

fn recording_requires_transcription_model(settings: &RecordingSettings) -> bool {
    settings.transcription.enabled
        && ((settings.capture_microphone && settings.transcription.microphone_enabled)
            || (settings.capture_system_audio && settings.transcription.system_audio_enabled))
}

fn recording_requires_speaker_analysis_model(settings: &RecordingSettings) -> bool {
    settings.speaker_analysis.separate_speakers && recording_requires_transcription_model(settings)
}

fn selected_speaker_analysis_model_available(
    app_data_dir: &std::path::Path,
    settings: &RecordingSettings,
) -> Result<bool, String> {
    let models_dir = speaker_analysis::speaker_analysis_models_dir(app_data_dir);
    let manifest = speaker_analysis::builtin_model_manifest();
    let Some(descriptor) = speaker_analysis::find_model_descriptor(
        &manifest,
        &settings.speaker_analysis.provider,
        settings.speaker_analysis.model_id.as_deref(),
    ) else {
        return Ok(false);
    };
    speaker_analysis::detect_model_status(&models_dir, descriptor)
        .map(|status| status.status == speaker_analysis::ModelStatusKind::Installed)
        .map_err(|error| error.to_string())
}

fn should_warn_ocr_unavailable_at_start(settings: &RecordingSettings) -> bool {
    settings.capture_screen && settings.ocr.enabled
}

fn should_warn_ocr_unavailable_at_startup(settings: &RecordingSettings) -> bool {
    should_warn_ocr_unavailable_at_start(settings)
}

fn ocr_provider_label(provider: OcrProvider) -> &'static str {
    match provider {
        OcrProvider::AppleVision => "Apple Vision",
        OcrProvider::Tesseract => "Tesseract",
        OcrProvider::PaddleOcr => "PaddleOCR",
    }
}

fn ocr_selection_label(settings: &OcrSettings) -> String {
    let provider = ocr_provider_label(settings.provider);
    match settings
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(model_id) => format!("{provider} `{model_id}`"),
        None => provider.to_string(),
    }
}

fn ocr_unavailable_notification(
    settings: &RecordingSettings,
    created_at_unix_ms: u64,
) -> AppNotification {
    let selection = ocr_selection_label(&settings.ocr);
    AppNotification {
        id: OCR_UNAVAILABLE_NOTIFICATION_ID.to_string(),
        severity: "warning".to_string(),
        title: "OCR engine unavailable".to_string(),
        message: format!(
            "{selection} is not available. Screen recording is blocked until you install or choose an available OCR engine."
        ),
        created_at_unix_ms,
        action: Some(AppNotificationAction::OpenSettingsTab {
            tab: PROCESSING_SETTINGS_TAB_ID.to_string(),
        }),
    }
}

fn maybe_push_ocr_unavailable_warning(
    app_handle: &tauri::AppHandle,
    app_notifications_state: &AppNotificationsState,
    settings: &RecordingSettings,
    context: &str,
) {
    let app_data_dir = match app_handle.path().app_data_dir() {
        Ok(dir) => dir,
        Err(error) => {
            debug_log::log_warn(format!(
                "failed to resolve app data directory for {context} OCR warning: {error}"
            ));
            return;
        }
    };

    match crate::ocr_models::selected_ocr_model_available(&app_data_dir, &settings.ocr) {
        Ok(true) => {}
        Ok(false) => {
            let selection = ocr_selection_label(&settings.ocr);
            debug_log::log_warn(format!(
                "ocr unavailable at {context} (selection={selection})"
            ));
            push_app_notification(
                app_handle,
                app_notifications_state,
                ocr_unavailable_notification(settings, runtime::now_unix_ms()),
            );
        }
        Err(error) => {
            let selection = ocr_selection_label(&settings.ocr);
            debug_log::log_warn(format!(
                "failed to inspect selected OCR model at {context} (selection={selection}): {error}"
            ));
            push_app_notification(
                app_handle,
                app_notifications_state,
                ocr_unavailable_notification(settings, runtime::now_unix_ms()),
            );
        }
    }
}

fn maybe_push_ocr_unavailable_start_warning(
    app_handle: &tauri::AppHandle,
    app_notifications_state: &AppNotificationsState,
    settings: &RecordingSettings,
) {
    if !should_warn_ocr_unavailable_at_start(settings) {
        return;
    }

    maybe_push_ocr_unavailable_warning(
        app_handle,
        app_notifications_state,
        settings,
        "recording start",
    );
}

pub fn maybe_push_audio_transcription_unavailable_startup_warning(app_handle: &tauri::AppHandle) {
    let settings_state = app_handle.state::<RecordingSettingsState>();
    let settings = current_recording_settings(settings_state.inner());
    if !should_warn_audio_transcription_unavailable_at_startup(&settings) {
        return;
    }

    maybe_push_audio_transcription_unavailable_warning(
        app_handle,
        app_handle.state::<AppNotificationsState>().inner(),
        &settings,
        "app startup",
    );
}

pub fn maybe_push_ocr_unavailable_startup_warning(app_handle: &tauri::AppHandle) {
    let settings_state = app_handle.state::<RecordingSettingsState>();
    let settings = current_recording_settings(settings_state.inner());
    if !should_warn_ocr_unavailable_at_startup(&settings) {
        return;
    }

    maybe_push_ocr_unavailable_warning(
        app_handle,
        app_handle.state::<AppNotificationsState>().inner(),
        &settings,
        "app startup",
    );
}

#[cfg(target_os = "macos")]
fn handle_system_will_sleep(app_handle: &tauri::AppHandle) {
    let state = app_handle.state::<NativeCaptureState>();
    let mut runtime = match state.lock() {
        Ok(runtime) => runtime,
        Err(_) => return,
    };

    if runtime.handle_system_will_sleep() {
        let runtime_state = runtime.runtime();
        debug_log::log_info(format!(
            "marked screen capture inactive for system sleep (session_id='{}', requested_sources={})",
            runtime_log_session_id(runtime_state),
            format_optional_capture_source_flags(runtime_state.requested_sources.as_ref())
        ));
    }
}

#[cfg(target_os = "macos")]
fn recover_screen_capture_after_system_wake_once(
    app_handle: &tauri::AppHandle,
) -> Result<bool, CaptureErrorResponse> {
    let state = app_handle.state::<NativeCaptureState>();
    let mut runtime = state.lock().map_err(|_| CaptureErrorResponse {
        code: "native_capture_state_poisoned".to_string(),
        message: "Native capture state is unavailable while recovering after system wake"
            .to_string(),
    })?;

    let outcome = runtime.recover_after_wake(Some(app_handle));
    let runtime_state = runtime.runtime();
    match &outcome {
        Ok(true) => {
            debug_log::log_info(format!(
                "recovered screen capture after system wake (session_id='{}', requested_sources={})",
                runtime_log_session_id(runtime_state),
                format_optional_capture_source_flags(runtime_state.requested_sources.as_ref())
            ));
        }
        Ok(false) => {}
        Err(error) => {
            debug_log::log_error(format!(
                "failed to recover screen capture after system wake (session_id='{}', requested_sources={}): [{}] {}",
                runtime_log_session_id(runtime_state),
                format_optional_capture_source_flags(runtime_state.requested_sources.as_ref()),
                error.code,
                error.message
            ));
        }
    }

    outcome
}

#[cfg(target_os = "macos")]
fn is_recover_after_wake_retryable_error(error: &CaptureErrorResponse) -> bool {
    matches!(
        error.code.as_str(),
        "capture_stream_start_failed"
            | "capture_stream_start_timeout"
            | "capture_shareable_content_failed"
            | "capture_shareable_content_timeout"
            | "capture_shareable_content_unavailable"
            | "capture_display_unavailable"
    ) || error
        .message
        .contains("Failed to find any displays or windows")
        || error.message.contains("code: -3815")
}

#[cfg(target_os = "macos")]
fn log_scheduled_system_wake_recovery_retry(error: &CaptureErrorResponse, delay_ms: u64) {
    debug_log::log_warn(format!(
        "screen capture wake recovery hit a transient ScreenCaptureKit error; retrying in {}ms: [{}] {}",
        delay_ms, error.code, error.message
    ));
}

#[cfg(target_os = "macos")]
fn system_wake_recovery_in_progress() -> &'static AtomicBool {
    static IN_PROGRESS: OnceLock<AtomicBool> = OnceLock::new();
    IN_PROGRESS.get_or_init(|| AtomicBool::new(false))
}

#[cfg(target_os = "macos")]
fn begin_system_wake_recovery() -> bool {
    system_wake_recovery_in_progress()
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

#[cfg(target_os = "macos")]
fn finish_system_wake_recovery() {
    system_wake_recovery_in_progress().store(false, Ordering::Release);
}

#[cfg(target_os = "macos")]
fn retry_screen_capture_recovery_after_system_wake(
    app_handle: tauri::AppHandle,
    mut last_error: CaptureErrorResponse,
) {
    std::thread::spawn(move || {
        for delay_ms in SYSTEM_WAKE_RECOVERY_RETRY_DELAYS_MS {
            log_scheduled_system_wake_recovery_retry(&last_error, *delay_ms);
            std::thread::sleep(Duration::from_millis(*delay_ms));

            match recover_screen_capture_after_system_wake_once(&app_handle) {
                Ok(_) => {
                    finish_system_wake_recovery();
                    emit_system_did_wake(&app_handle);
                    return;
                }
                Err(error) if is_recover_after_wake_retryable_error(&error) => {
                    last_error = error;
                }
                Err(_) => {
                    finish_system_wake_recovery();
                    emit_system_did_wake(&app_handle);
                    return;
                }
            }
        }

        finish_system_wake_recovery();
        emit_system_did_wake(&app_handle);
    });
}

#[cfg(target_os = "macos")]
fn recover_screen_capture_after_system_wake(app_handle: tauri::AppHandle) {
    if !begin_system_wake_recovery() {
        return;
    }

    match recover_screen_capture_after_system_wake_once(&app_handle) {
        Ok(_) => {
            finish_system_wake_recovery();
            emit_system_did_wake(&app_handle);
        }
        Err(error) if is_recover_after_wake_retryable_error(&error) => {
            retry_screen_capture_recovery_after_system_wake(app_handle, error);
        }
        Err(_) => {
            finish_system_wake_recovery();
            emit_system_did_wake(&app_handle);
        }
    }
}

#[cfg(target_os = "macos")]
fn recover_screen_capture_after_possible_missed_wake(app_handle: tauri::AppHandle) {
    let state = app_handle.state::<NativeCaptureState>();
    let should_recover = state
        .lock()
        .map(|runtime| runtime.should_attempt_recovery_after_possible_wake())
        .unwrap_or(false);

    if !should_recover {
        return;
    }

    debug_log::log_info(
        "attempting screen capture recovery during session resync after possible missed system wake notification"
            .to_string(),
    );
    recover_screen_capture_after_system_wake(app_handle);
}

#[cfg(target_os = "macos")]
pub fn start_system_wake_notifier(app_handle: tauri::AppHandle) {
    use cidre::ns;

    let mut center = ns::Workspace::shared().notification_center();
    let will_sleep_guard =
        center.add_observer_guard(ns::workspace::notification::will_sleep(), None, None, {
            let app_handle = app_handle.clone();
            move |_notification| {
                handle_system_will_sleep(&app_handle);
            }
        });
    let did_wake_guard =
        center.add_observer_guard(ns::workspace::notification::did_wake(), None, None, {
            let app_handle = app_handle.clone();
            move |_notification| {
                recover_screen_capture_after_system_wake(app_handle.clone());
            }
        });

    let notifier_state = app_handle.state::<SystemWakeNotifierState>();
    let mut notifier_slot = notifier_state
        .0
        .lock()
        .expect("system wake notifier state poisoned");
    notifier_slot.clear();
    notifier_slot.push(will_sleep_guard);
    notifier_slot.push(did_wake_guard);
}

#[cfg(not(target_os = "macos"))]
pub fn start_system_wake_notifier(_app_handle: tauri::AppHandle) {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CaptureSupportSnapshot {
    platform: String,
    native_capture_supported: bool,
    supported_sources: CaptureSources,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturePermissionsSnapshot {
    screen: &'static str,
    microphone: &'static str,
    system_audio: &'static str,
}

fn capture_support_log_snapshot_state() -> &'static std::sync::Mutex<Option<CaptureSupportSnapshot>>
{
    static LAST_CAPTURE_SUPPORT_SNAPSHOT: OnceLock<
        std::sync::Mutex<Option<CaptureSupportSnapshot>>,
    > = OnceLock::new();

    LAST_CAPTURE_SUPPORT_SNAPSHOT.get_or_init(|| std::sync::Mutex::new(None))
}

fn capture_permissions_log_snapshot_state(
) -> &'static std::sync::Mutex<Option<CapturePermissionsSnapshot>> {
    static LAST_CAPTURE_PERMISSIONS_SNAPSHOT: OnceLock<
        std::sync::Mutex<Option<CapturePermissionsSnapshot>>,
    > = OnceLock::new();

    LAST_CAPTURE_PERMISSIONS_SNAPSHOT.get_or_init(|| std::sync::Mutex::new(None))
}

fn reset_capture_log_snapshots() {
    *capture_support_log_snapshot_state()
        .lock()
        .expect("capture support log snapshot poisoned") = None;
    *capture_permissions_log_snapshot_state()
        .lock()
        .expect("capture permissions log snapshot poisoned") = None;
}

fn capture_sources_from_settings(settings: &RecordingSettings) -> CaptureSources {
    CaptureSources {
        screen: settings.capture_screen,
        microphone: settings.capture_microphone,
        system_audio: settings.capture_system_audio,
    }
}

fn update_recording_settings_request_from_settings(
    settings: RecordingSettings,
) -> UpdateRecordingSettingsRequest {
    UpdateRecordingSettingsRequest {
        capture_screen: settings.capture_screen,
        capture_microphone: settings.capture_microphone,
        capture_system_audio: settings.capture_system_audio,
        segment_duration_seconds: settings.segment_duration_seconds,
        screen_frame_rate: settings.screen_frame_rate,
        screen_resolution: settings.screen_resolution,
        video_bitrate: settings.video_bitrate,
        save_directory: settings.save_directory,
        auto_start: settings.auto_start,
        native_capture_debug_logging_enabled: settings.native_capture_debug_logging_enabled,
        developer_options_enabled: settings.developer_options_enabled,
        preview_cache_ttl_seconds: settings.preview_cache_ttl_seconds,
        follow_timeline_live: settings.follow_timeline_live,
        retention_policy: settings.retention_policy,
        appearance: settings.appearance,
        ocr: settings.ocr,
        transcription: settings.transcription,
        speaker_analysis: settings.speaker_analysis,
        audio_speech_detection: settings.audio_speech_detection,
        metadata: settings.metadata,
        privacy: settings.privacy,
        pause_capture_on_inactivity: settings.pause_capture_on_inactivity,
        idle_timeout_seconds: settings.idle_timeout_seconds,
        microphone_activity_sensitivity: settings.microphone_activity_sensitivity,
        system_audio_activity_sensitivity: settings.system_audio_activity_sensitivity,
        microphone_vad_adapter: settings.microphone_vad_adapter,
        inactivity_activity_mode: settings.inactivity_activity_mode,
    }
}

fn capture_sources_from_start_request(request: &StartNativeCaptureRequest) -> CaptureSources {
    CaptureSources {
        screen: request.capture_screen,
        microphone: request.capture_microphone,
        system_audio: request.capture_system_audio,
    }
}

fn format_capture_source_flags(sources: &CaptureSources) -> String {
    format!(
        "screen={}, microphone={}, system_audio={}",
        sources.screen, sources.microphone, sources.system_audio
    )
}

fn format_optional_capture_source_flags(sources: Option<&CaptureSources>) -> String {
    sources
        .map(format_capture_source_flags)
        .unwrap_or_else(|| "screen=unknown, microphone=unknown, system_audio=unknown".to_string())
}

fn runtime_log_session_id(runtime: &runtime::NativeCaptureRuntime) -> &str {
    runtime
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.screen.as_ref())
        .map(|session| session.session_id.as_str())
        .unwrap_or("unknown")
}

fn session_log_session_id(session: &capture_types::NativeCaptureSession) -> &str {
    session
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.screen.as_ref())
        .map(|source| source.session_id.as_str())
        .unwrap_or("unknown")
}

fn permission_state_label(state: &CapturePermissionState) -> &'static str {
    match state {
        CapturePermissionState::Granted => "granted",
        CapturePermissionState::Denied => "denied",
        CapturePermissionState::NotDetermined => "not_determined",
        CapturePermissionState::Unsupported => "unsupported",
        CapturePermissionState::Unknown => "unknown",
    }
}

fn format_screen_resolution(resolution: &ScreenResolution) -> String {
    match resolution {
        ScreenResolution::Preset { preset } => match preset {
            ScreenResolutionPreset::Original => "original".to_string(),
            ScreenResolutionPreset::P1080 => "1080p".to_string(),
            ScreenResolutionPreset::P720 => "720p".to_string(),
            ScreenResolutionPreset::P540 => "540p".to_string(),
        },
        ScreenResolution::Custom { width, height } => format!("{width}x{height}"),
    }
}

fn format_video_bitrate(settings: &VideoBitrateSettings) -> String {
    match settings.mode {
        VideoBitrateMode::Preset => {
            let preset = settings
                .preset
                .clone()
                .unwrap_or(VideoBitratePreset::Medium);
            let label = match preset {
                VideoBitratePreset::Low => "low",
                VideoBitratePreset::Medium => "medium",
                VideoBitratePreset::High => "high",
            };

            format!("preset:{label}")
        }
        VideoBitrateMode::Custom => format!("custom:{}mbps", settings.custom_mbps.unwrap_or(0)),
    }
}

fn inactivity_activity_mode_label(mode: &InactivityActivityMode) -> &'static str {
    match mode {
        InactivityActivityMode::SystemInputOnly => "system_input_only",
        InactivityActivityMode::SystemInputOrScreen => "system_input_or_screen",
        InactivityActivityMode::SystemInputOrScreenOrAudio => "system_input_or_screen_or_audio",
    }
}

fn recording_settings_overview(settings: &RecordingSettings) -> String {
    format!(
        "sources={}, auto_start={}, save_directory='{}', debug_logging={}, preview_cache_ttl_seconds={}, follow_timeline_live={}, segment_duration_seconds={}, screen_frame_rate={}, screen_resolution={}, video_bitrate={}, pause_on_inactivity={}, idle_timeout_seconds={}, microphone_activity_sensitivity={}, system_audio_activity_sensitivity={}, activity_mode={}",
        format_capture_source_flags(&capture_sources_from_settings(settings)),
        settings.auto_start,
        settings.save_directory,
        settings.native_capture_debug_logging_enabled,
        settings.preview_cache_ttl_seconds,
        settings.follow_timeline_live,
        settings.segment_duration_seconds,
        settings.screen_frame_rate,
        format_screen_resolution(&settings.screen_resolution),
        format_video_bitrate(&settings.video_bitrate),
        settings.pause_capture_on_inactivity,
        settings.idle_timeout_seconds,
        settings.microphone_activity_sensitivity,
        settings.system_audio_activity_sensitivity,
        inactivity_activity_mode_label(&settings.inactivity_activity_mode)
    )
}

fn describe_recording_settings_changes(
    previous: &RecordingSettings,
    next: &RecordingSettings,
) -> Vec<String> {
    let mut changes = Vec::new();
    let previous_sources = capture_sources_from_settings(previous);
    let next_sources = capture_sources_from_settings(next);

    if previous_sources != next_sources {
        changes.push(format!(
            "sources {} -> {}",
            format_capture_source_flags(&previous_sources),
            format_capture_source_flags(&next_sources)
        ));
    }

    if previous.auto_start != next.auto_start {
        changes.push(format!(
            "auto_start {} -> {}",
            previous.auto_start, next.auto_start
        ));
    }

    if previous.native_capture_debug_logging_enabled != next.native_capture_debug_logging_enabled {
        changes.push(format!(
            "debug_logging {} -> {}",
            previous.native_capture_debug_logging_enabled,
            next.native_capture_debug_logging_enabled
        ));
    }

    if previous.preview_cache_ttl_seconds != next.preview_cache_ttl_seconds {
        changes.push(format!(
            "preview_cache_ttl_seconds {} -> {}",
            previous.preview_cache_ttl_seconds, next.preview_cache_ttl_seconds
        ));
    }

    if previous.follow_timeline_live != next.follow_timeline_live {
        changes.push(format!(
            "follow_timeline_live {} -> {}",
            previous.follow_timeline_live, next.follow_timeline_live
        ));
    }

    if previous.segment_duration_seconds != next.segment_duration_seconds {
        changes.push(format!(
            "segment_duration_seconds {} -> {}",
            previous.segment_duration_seconds, next.segment_duration_seconds
        ));
    }

    if previous.screen_frame_rate != next.screen_frame_rate {
        changes.push(format!(
            "screen_frame_rate {} -> {}",
            previous.screen_frame_rate, next.screen_frame_rate
        ));
    }

    if previous.screen_resolution != next.screen_resolution {
        changes.push(format!(
            "screen_resolution {} -> {}",
            format_screen_resolution(&previous.screen_resolution),
            format_screen_resolution(&next.screen_resolution)
        ));
    }

    if previous.video_bitrate != next.video_bitrate {
        changes.push(format!(
            "video_bitrate {} -> {}",
            format_video_bitrate(&previous.video_bitrate),
            format_video_bitrate(&next.video_bitrate)
        ));
    }

    if previous.pause_capture_on_inactivity != next.pause_capture_on_inactivity {
        changes.push(format!(
            "pause_on_inactivity {} -> {}",
            previous.pause_capture_on_inactivity, next.pause_capture_on_inactivity
        ));
    }

    if previous.idle_timeout_seconds != next.idle_timeout_seconds {
        changes.push(format!(
            "idle_timeout_seconds {} -> {}",
            previous.idle_timeout_seconds, next.idle_timeout_seconds
        ));
    }

    if previous.inactivity_activity_mode != next.inactivity_activity_mode {
        changes.push(format!(
            "activity_mode {} -> {}",
            inactivity_activity_mode_label(&previous.inactivity_activity_mode),
            inactivity_activity_mode_label(&next.inactivity_activity_mode)
        ));
    }

    if previous.microphone_activity_sensitivity != next.microphone_activity_sensitivity {
        changes.push(format!(
            "microphone_activity_sensitivity {} -> {}",
            previous.microphone_activity_sensitivity, next.microphone_activity_sensitivity
        ));
    }

    if previous.system_audio_activity_sensitivity != next.system_audio_activity_sensitivity {
        changes.push(format!(
            "system_audio_activity_sensitivity {} -> {}",
            previous.system_audio_activity_sensitivity, next.system_audio_activity_sensitivity
        ));
    }

    changes
}

fn format_output_file_counts(output_files: Option<&CaptureOutputFiles>) -> String {
    output_files
        .map(|output_files| {
            format!(
                "screen_files={}, microphone_files={}, system_audio_files={}",
                output_files.screen_files.len(),
                output_files.microphone_files.len(),
                output_files.system_audio_files.len()
            )
        })
        .unwrap_or_else(|| "screen_files=0, microphone_files=0, system_audio_files=0".to_string())
}

fn log_capture_support_if_changed(response: &CaptureSupportResponse) {
    let snapshot = CaptureSupportSnapshot {
        platform: response.platform.clone(),
        native_capture_supported: response.native_capture_supported,
        supported_sources: response.supported_sources.clone(),
    };
    let mut last_snapshot = capture_support_log_snapshot_state()
        .lock()
        .expect("capture support log snapshot poisoned");

    if last_snapshot.as_ref() == Some(&snapshot) {
        return;
    }

    *last_snapshot = Some(snapshot.clone());

    debug_log::log(format!(
        "observed native capture support (platform='{}', native_supported={}, supported_sources={})",
        snapshot.platform,
        snapshot.native_capture_supported,
        format_capture_source_flags(&snapshot.supported_sources)
    ));
}

fn log_capture_permissions_if_changed(permissions: &CapturePermissions) {
    let snapshot = CapturePermissionsSnapshot {
        screen: permission_state_label(&permissions.screen),
        microphone: permission_state_label(&permissions.microphone),
        system_audio: permission_state_label(&permissions.system_audio),
    };
    let mut last_snapshot = capture_permissions_log_snapshot_state()
        .lock()
        .expect("capture permissions log snapshot poisoned");

    if last_snapshot.as_ref() == Some(&snapshot) {
        return;
    }

    *last_snapshot = Some(snapshot.clone());

    debug_log::log(format!(
        "observed native capture permissions (screen={}, microphone={}, system_audio={})",
        snapshot.screen, snapshot.microphone, snapshot.system_audio
    ));
}

fn log_loaded_recording_settings(source: &str, settings: &RecordingSettings) {
    debug_log::log_info(format!(
        "loaded recording settings from {source} ({})",
        recording_settings_overview(settings)
    ));
}

fn log_recording_settings_changes(previous: &RecordingSettings, next: &RecordingSettings) {
    let changes = describe_recording_settings_changes(previous, next);

    if changes.is_empty() {
        return;
    }

    debug_log::log_info(format!(
        "updated recording settings ({})",
        changes.join(", ")
    ));
}

fn start_native_capture_inner(
    origin: &str,
    request: StartNativeCaptureRequest,
    state: tauri::State<'_, NativeCaptureState>,
    microphone_controller_preferences_state: tauri::State<'_, MicrophoneControllerPreferencesState>,
    recording_settings_state: tauri::State<'_, RecordingSettingsState>,
    app_notifications_state: tauri::State<'_, AppNotificationsState>,
    app_handle: tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let incoming_sources = capture_sources_from_start_request(&request);
    let settings = recording_settings_state.inner();
    let settings = current_recording_settings(settings);

    let resolved_request = StartNativeCaptureRequest {
        capture_screen: settings.capture_screen,
        capture_microphone: settings.capture_microphone,
        capture_system_audio: settings.capture_system_audio,
    };
    let resolved_sources = capture_sources_from_start_request(&resolved_request);

    debug_log::log_info(format!(
        "attempting native capture {origin} start (incoming_sources={}, resolved_sources={}, save_directory='{}')",
        format_capture_source_flags(&incoming_sources),
        format_capture_source_flags(&resolved_sources),
        settings.save_directory
    ));

    let support = get_capture_support();
    let sources = match validate_start_request(&resolved_request, &support) {
        Ok(sources) => sources,
        Err(error) => {
            debug_log::log_warn(format!(
                "rejected native capture {origin} start during source validation (resolved_sources={}, supported_sources={}): [{}] {}",
                format_capture_source_flags(&resolved_sources),
                format_capture_source_flags(&support.supported_sources),
                error.code,
                error.message
            ));
            return Err(error);
        }
    };

    if resolved_request.capture_screen && settings.ocr.enabled {
        let app_data_dir =
            app_handle
                .path()
                .app_data_dir()
                .map_err(|error| CaptureErrorResponse {
                    code: "ocr_model_unavailable".to_string(),
                    message: format!(
                        "failed to resolve app data directory for OCR preflight: {error}"
                    ),
                })?;
        match crate::ocr_models::selected_ocr_model_available(&app_data_dir, &settings.ocr) {
            Ok(true) => {}
            Ok(false) => {
                maybe_push_ocr_unavailable_start_warning(
                    &app_handle,
                    app_notifications_state.inner(),
                    &settings,
                );
                let error = CaptureErrorResponse {
                    code: "ocr_model_unavailable".to_string(),
                    message: format!(
                        "{} is unavailable. Install or choose an available OCR engine before recording screen capture.",
                        ocr_selection_label(&settings.ocr)
                    ),
                };
                debug_log::log_warn(format!(
                    "rejected native capture {origin} start because OCR is unavailable: [{}] {}",
                    error.code, error.message
                ));
                return Err(error);
            }
            Err(status_error) => {
                maybe_push_ocr_unavailable_start_warning(
                    &app_handle,
                    app_notifications_state.inner(),
                    &settings,
                );
                let error = CaptureErrorResponse {
                    code: "ocr_model_unavailable".to_string(),
                    message: format!(
                        "failed to verify OCR availability for {}: {status_error}",
                        ocr_selection_label(&settings.ocr)
                    ),
                };
                debug_log::log_warn(format!(
                    "rejected native capture {origin} start because OCR availability check failed: [{}] {}",
                    error.code, error.message
                ));
                return Err(error);
            }
        }
    }

    if recording_requires_speech_detector(&settings) {
        match selected_speech_detector_available(&settings) {
            Ok(true) => {}
            Ok(false) => {
                push_app_notification(
                    &app_handle,
                    app_notifications_state.inner(),
                    speech_detector_unavailable_notification(runtime::now_unix_ms()),
                );
                return Err(CaptureErrorResponse {
                    code: "speech_detector_unavailable".to_string(),
                    message: "Selected speech detector is unavailable for the requested recording sources.".to_string(),
                });
            }
            Err(error) => {
                push_app_notification(
                    &app_handle,
                    app_notifications_state.inner(),
                    speech_detector_unavailable_notification(runtime::now_unix_ms()),
                );
                return Err(CaptureErrorResponse {
                    code: "speech_detector_unavailable".to_string(),
                    message: format!("Failed to verify selected speech detector: {error}"),
                });
            }
        }
    }

    let app_data_dir_for_processing = if recording_requires_transcription_model(&settings)
        || recording_requires_speaker_analysis_model(&settings)
    {
        Some(
            app_handle
                .path()
                .app_data_dir()
                .map_err(|error| CaptureErrorResponse {
                    code: "processing_model_unavailable".to_string(),
                    message: format!(
                        "failed to resolve app data directory for processing preflight: {error}"
                    ),
                })?,
        )
    } else {
        None
    };

    if recording_requires_transcription_model(&settings) {
        let app_data_dir = app_data_dir_for_processing
            .as_deref()
            .expect("processing dir should exist");
        match crate::audio_transcription_models::selected_audio_transcription_model_available(
            app_data_dir,
            &settings.transcription,
        ) {
            Ok(true) => {}
            Ok(false) => {
                maybe_push_audio_transcription_unavailable_start_warning(
                    &app_handle,
                    app_notifications_state.inner(),
                    &settings,
                );
                return Err(CaptureErrorResponse {
                    code: "audio_transcription_model_unavailable".to_string(),
                    message: format!(
                        "{} is unavailable. Install or choose an available transcription model before recording requested audio.",
                        audio_transcription_selection_label(&settings.transcription)
                    ),
                });
            }
            Err(error) => {
                maybe_push_audio_transcription_unavailable_start_warning(
                    &app_handle,
                    app_notifications_state.inner(),
                    &settings,
                );
                return Err(CaptureErrorResponse {
                    code: "audio_transcription_model_unavailable".to_string(),
                    message: format!("failed to verify transcription model availability: {error}"),
                });
            }
        }
    }

    if recording_requires_speaker_analysis_model(&settings) {
        let app_data_dir = app_data_dir_for_processing
            .as_deref()
            .expect("processing dir should exist");
        match selected_speaker_analysis_model_available(app_data_dir, &settings) {
            Ok(true) => {}
            Ok(false) => {
                push_app_notification(
                    &app_handle,
                    app_notifications_state.inner(),
                    speaker_analysis_unavailable_notification(runtime::now_unix_ms()),
                );
                return Err(CaptureErrorResponse {
                    code: "speaker_analysis_model_unavailable".to_string(),
                    message: "Selected speaker analysis model is unavailable. Install or choose an available model before recording requested audio.".to_string(),
                });
            }
            Err(error) => {
                push_app_notification(
                    &app_handle,
                    app_notifications_state.inner(),
                    speaker_analysis_unavailable_notification(runtime::now_unix_ms()),
                );
                return Err(CaptureErrorResponse {
                    code: "speaker_analysis_model_unavailable".to_string(),
                    message: format!(
                        "failed to verify speaker analysis model availability: {error}"
                    ),
                });
            }
        }
    }

    let microphone_device_id_for_capture = if resolved_request.capture_microphone {
        let preferences_runtime = microphone_controller_preferences_state
            .lock()
            .expect("microphone controller preferences state poisoned");
        let controller_state = match microphone_capture::microphone_controller_state(
            preferences_runtime.preference.clone(),
            preferences_runtime.disconnect_policy.clone(),
        ) {
            Ok(state) => state,
            Err(error) => {
                debug_log::log_error(format!(
                    "failed to resolve microphone controller state for native capture {origin} start: [{}] {}",
                    error.code, error.message
                ));
                return Err(error);
            }
        };

        if should_wait_for_same_microphone_device(&controller_state) {
            let error = CaptureErrorResponse {
                code: "microphone_device_unavailable_waiting_for_selected_device".to_string(),
                message: "The selected microphone is unavailable. Reconnect the same device or change microphone preference."
                    .to_string(),
            };
            debug_log::log_warn(format!(
                "rejected native capture {origin} start because the selected microphone is unavailable and wait-for-same-device is active: [{}] {}",
                error.code, error.message
            ));
            return Err(error);
        }

        resolve_capture_microphone_device_id(&controller_state)
    } else {
        None
    };

    let mut runtime = state.lock().expect("native capture state poisoned");
    if runtime.runtime().is_running {
        let existing_sources =
            format_optional_capture_source_flags(runtime.runtime().requested_sources.as_ref());
        let session_id = runtime_log_session_id(runtime.runtime());

        if runtime.runtime().requested_sources.as_ref() != Some(&sources) {
            let error = CaptureErrorResponse {
                code: "capture_session_already_running".to_string(),
                message: "A native capture session is already running with different sources"
                    .to_string(),
            };
            debug_log::log_warn(format!(
                "rejected native capture {origin} start because another session is already running (session_id='{}', existing_sources={}, requested_sources={}): [{}] {}",
                session_id,
                existing_sources,
                format_capture_source_flags(&sources),
                error.code,
                error.message
            ));
            return Err(error);
        }

        debug_log::log_info(format!(
            "native capture {origin} start requested while session is already running; returning existing session (session_id='{}', requested_sources={})",
            session_id, existing_sources
        ));

        return Ok(NativeCaptureSessionResponse {
            session: runtime.session(),
        });
    }

    let requested_sources_for_log = sources.clone();
    let started_session = match runtime.start(
        app_handle.clone(),
        &settings,
        sources,
        microphone_device_id_for_capture,
    ) {
        Ok(StartRecordingLifecycleOutcome::Started(session)) => session,
        Ok(StartRecordingLifecycleOutcome::AlreadyRunning(session)) => {
            debug_log::log_info(format!(
                "native capture {origin} start requested while session is already running; returning existing session (session_id='{}', requested_sources={})",
                session_log_session_id(&session),
                format_optional_capture_source_flags(session.requested_sources.as_ref())
            ));

            return Ok(NativeCaptureSessionResponse { session });
        }
        Err(error) => {
            debug_log::log_error(format!(
                "failed to start native capture ({origin}, requested_sources={}): [{}] {}",
                format_capture_source_flags(&requested_sources_for_log),
                error.code,
                error.message
            ));
            return Err(error);
        }
    };

    debug_log::log_info(format!(
        "started native capture successfully ({origin}, session_id='{}', requested_sources={}, segment_index={}, save_directory='{}')",
        runtime_log_session_id(runtime.runtime()),
        format_optional_capture_source_flags(runtime.runtime().requested_sources.as_ref()),
        runtime.runtime().current_segment_index,
        settings.save_directory
    ));

    maybe_push_audio_transcription_unavailable_start_warning(
        &app_handle,
        app_notifications_state.inner(),
        &settings,
    );

    if let Some(notice) = runtime.take_microphone_vad_fallback_notification() {
        let message = format!(
            "Configured microphone VAD '{}' could not run. Using '{}' for this recording session.",
            configured_adapter_as_str(notice.configured_adapter),
            notice.effective_adapter.as_str(),
        );
        debug_log::log_warn(format!(
            "microphone VAD fallback active: configured_adapter={}, effective_adapter={}, reason={}",
            configured_adapter_as_str(notice.configured_adapter),
            notice.effective_adapter.as_str(),
            notice.reason
        ));
        push_app_notification(
            &app_handle,
            app_notifications_state.inner(),
            AppNotification {
                id: format!(
                    "microphone-vad-fallback-{}",
                    configured_adapter_as_str(notice.configured_adapter)
                ),
                severity: "warning".to_string(),
                title: "Microphone VAD fallback".to_string(),
                message,
                created_at_unix_ms: runtime::now_unix_ms(),
                action: None,
            },
        );
    }

    Ok(NativeCaptureSessionResponse {
        session: started_session,
    })
}

#[tauri::command]
pub fn get_capture_support() -> CaptureSupportResponse {
    let screen_support = capture_screen::support_for_current_platform();
    let microphone_supported = !matches!(
        microphone_capture::microphone_permission_state(),
        CapturePermissionState::Unsupported
    );

    let response = CaptureSupportResponse {
        platform: screen_support.platform,
        native_capture_supported: screen_support.native_capture_supported,
        supported_sources: CaptureSources {
            screen: screen_support.screen,
            microphone: microphone_supported,
            system_audio: screen_support.system_audio,
        },
    };

    log_capture_support_if_changed(&response);
    response
}

#[tauri::command]
pub fn get_capture_permissions(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, NativeCaptureState>,
) -> CapturePermissionsResponse {
    #[cfg(target_os = "macos")]
    recover_screen_capture_after_possible_missed_wake(app_handle);

    let runtime = state.lock().expect("native capture state poisoned");
    let permissions = CapturePermissions {
        screen: capture_screen::screen_permission_state(),
        microphone: microphone_capture::microphone_permission_state(),
        system_audio: capture_screen::system_audio_permission_state(),
    };

    log_capture_permissions_if_changed(&permissions);

    CapturePermissionsResponse {
        permissions,
        session: runtime.session(),
    }
}

#[tauri::command]
pub fn get_idle_debug(state: tauri::State<'_, NativeCaptureState>) -> IdleDebugInfo {
    activity::get_idle_debug(state)
}

#[tauri::command]
pub fn get_app_notifications(
    state: tauri::State<'_, AppNotificationsState>,
) -> Vec<AppNotification> {
    state
        .lock()
        .expect("app notifications state poisoned")
        .list()
}

#[tauri::command]
pub fn clear_app_notification(
    id: String,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppNotificationsState>,
) -> Vec<AppNotification> {
    let notifications = {
        let mut runtime = state.lock().expect("app notifications state poisoned");
        runtime.clear_one(&id)
    };
    emit_app_notifications_changed(&app_handle, &notifications);
    notifications
}

#[tauri::command]
pub fn clear_app_notifications(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppNotificationsState>,
) -> Vec<AppNotification> {
    let notifications = {
        let mut runtime = state.lock().expect("app notifications state poisoned");
        runtime.clear_all()
    };
    emit_app_notifications_changed(&app_handle, &notifications);
    notifications
}

#[tauri::command]
pub fn get_microphone_controller_state(
    state: tauri::State<'_, MicrophoneControllerPreferencesState>,
) -> Result<MicrophoneControllerState, CaptureErrorResponse> {
    let runtime = state
        .lock()
        .expect("microphone controller preferences state poisoned");
    microphone_capture::microphone_controller_state(
        runtime.preference.clone(),
        runtime.disconnect_policy.clone(),
    )
}

#[tauri::command]
pub fn update_microphone_controller(
    request: UpdateMicrophoneControllerRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, MicrophoneControllerPreferencesState>,
) -> Result<MicrophoneControllerState, CaptureErrorResponse> {
    update_microphone_controller_impl(request, &app_handle, state)
}

pub fn initialize_recording_settings_from_disk(app_handle: &tauri::AppHandle) {
    reset_capture_log_snapshots();
    let settings_state = app_handle.state::<RecordingSettingsState>();
    let loaded = initialize_recording_settings_state_from_disk(app_handle, settings_state.inner());

    debug_log::configure(
        app_handle,
        loaded.settings.native_capture_debug_logging_enabled,
    );
    log_loaded_recording_settings(loaded.source, &loaded.settings);
}

pub fn maybe_auto_start_native_capture(app_handle: &tauri::AppHandle) {
    let settings_state = app_handle.state::<RecordingSettingsState>();
    let auto_start_enabled = current_auto_start(settings_state.inner());

    if !auto_start_enabled {
        return;
    }

    let _ = start_native_capture_from_app_handle("auto-start", app_handle);
}

pub(crate) fn current_native_capture_session(
    app_handle: &tauri::AppHandle,
) -> capture_types::NativeCaptureSession {
    let state = app_handle.state::<NativeCaptureState>();
    let runtime = state.lock().expect("native capture state poisoned");
    runtime.session()
}

pub(crate) fn current_recording_settings_from_app_handle(
    app_handle: &tauri::AppHandle,
) -> RecordingSettings {
    let state = app_handle.state::<RecordingSettingsState>();
    current_recording_settings(state.inner())
}

pub(crate) fn apply_recording_settings_update_from_app_handle(
    app_handle: &tauri::AppHandle,
    request: UpdateRecordingSettingsRequest,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    let state = app_handle.state::<RecordingSettingsState>();
    let update = apply_recording_settings_update(app_handle, state.inner(), request)?;
    let settings = update.settings;
    let previous_settings = update.previous_settings;
    let previous_save_directory = update.previous_save_directory;
    let save_directory_changed = update.save_directory_changed;
    let debug_logging_enabled_changed = update.debug_logging_enabled_changed;

    if previous_settings.native_capture_debug_logging_enabled
        && !settings.native_capture_debug_logging_enabled
    {
        log_recording_settings_changes(&previous_settings, &settings);

        if save_directory_changed {
            debug_log::log_info(format!(
                "recording save directory changed from '{}' to '{}'; app infrastructure database location will update on next app start",
                previous_save_directory, settings.save_directory
            ));
        }
    }

    debug_log::configure(app_handle, settings.native_capture_debug_logging_enabled);

    if !previous_settings.native_capture_debug_logging_enabled
        && settings.native_capture_debug_logging_enabled
    {
        reset_capture_log_snapshots();
    }

    if settings.native_capture_debug_logging_enabled {
        if debug_logging_enabled_changed {
            debug_log::log_info(format!(
                "native capture debug logging {}",
                if previous_settings.native_capture_debug_logging_enabled {
                    "re-enabled"
                } else {
                    "enabled"
                }
            ));
        }

        log_recording_settings_changes(&previous_settings, &settings);
    }

    if save_directory_changed && settings.native_capture_debug_logging_enabled {
        debug_log::log_info(format!(
            "recording save directory changed from '{}' to '{}'; app infrastructure database location will update on next app start",
            previous_save_directory, settings.save_directory
        ));
    }

    if previous_settings.ocr.enabled && !settings.ocr.enabled {
        if let Some(infra) = app_handle.try_state::<crate::app_infra::AppInfraState>() {
            match tauri::async_runtime::block_on(infra.fail_queued_ocr_jobs_because_disabled()) {
                Ok(failed_count) => debug_log::log_info(format!(
                    "marked queued OCR jobs failed because OCR was disabled (count={failed_count})"
                )),
                Err(error) => debug_log::log_error(format!(
                    "failed to mark queued OCR jobs failed after disabling OCR: {error}"
                )),
            }
        } else {
            debug_log::log_warn(
                "app infrastructure state unavailable while disabling OCR; queued OCR jobs were not updated",
            );
        }
    }

    if previous_settings.retention_policy != settings.retention_policy {
        if let Some(background_workers) =
            app_handle.try_state::<crate::app_infra::BackgroundWorkersState>()
        {
            background_workers.notify_retention_schedule_changed();
        } else {
            debug_log::log_warn(
                "background workers state unavailable while updating retention policy; retention cleanup schedule was not woken",
            );
        }
    }

    emit_recording_settings_changed(app_handle, &settings);
    crate::status_bar::refresh(app_handle);

    Ok(settings)
}

pub(crate) fn update_recording_sources_from_app_handle(
    app_handle: &tauri::AppHandle,
    sources: CaptureSources,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    let mut request = update_recording_settings_request_from_settings(
        current_recording_settings_from_app_handle(app_handle),
    );
    request.capture_screen = sources.screen;
    request.capture_microphone = sources.microphone;
    request.capture_system_audio = sources.system_audio;
    apply_recording_settings_update_from_app_handle(app_handle, request)
}

pub(crate) fn start_native_capture_from_app_handle(
    origin: &str,
    app_handle: &tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let response = start_native_capture_inner(
        origin,
        StartNativeCaptureRequest {
            capture_screen: false,
            capture_microphone: false,
            capture_system_audio: false,
        },
        app_handle.state::<NativeCaptureState>(),
        app_handle.state::<MicrophoneControllerPreferencesState>(),
        app_handle.state::<RecordingSettingsState>(),
        app_handle.state::<AppNotificationsState>(),
        app_handle.clone(),
    )?;
    emit_native_capture_session_changed(app_handle, &response.session);
    crate::status_bar::refresh(app_handle);
    Ok(response)
}

pub(crate) fn stop_native_capture_from_app_handle(
    app_handle: &tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let response =
        stop_native_capture_with_state(app_handle.state::<NativeCaptureState>(), app_handle)?;
    emit_native_capture_session_changed(app_handle, &response.session);
    crate::status_bar::refresh(app_handle);
    Ok(response)
}

#[tauri::command]
pub fn get_recording_settings(
    state: tauri::State<'_, RecordingSettingsState>,
) -> RecordingSettings {
    current_recording_settings(state.inner())
}

#[tauri::command]
pub fn get_native_capture_debug_log_status(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> NativeCaptureDebugLogStatus {
    let enabled = current_native_capture_debug_logging_enabled(state.inner());

    debug_log::status(&app_handle, enabled)
}

#[tauri::command]
pub fn delete_native_capture_debug_log(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<NativeCaptureDebugLogStatus, CaptureErrorResponse> {
    let enabled = current_native_capture_debug_logging_enabled(state.inner());

    debug_log::delete(&app_handle, enabled)
}

#[tauri::command]
pub fn update_recording_settings(
    request: UpdateRecordingSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    let _ = state;
    apply_recording_settings_update_from_app_handle(&app_handle, request)
}

#[tauri::command]
pub fn start_native_capture(
    request: StartNativeCaptureRequest,
    state: tauri::State<'_, NativeCaptureState>,
    microphone_controller_preferences_state: tauri::State<'_, MicrophoneControllerPreferencesState>,
    recording_settings_state: tauri::State<'_, RecordingSettingsState>,
    app_notifications_state: tauri::State<'_, AppNotificationsState>,
    app_handle: tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let response = start_native_capture_inner(
        "command",
        request,
        state,
        microphone_controller_preferences_state,
        recording_settings_state,
        app_notifications_state,
        app_handle.clone(),
    )?;
    emit_native_capture_session_changed(&app_handle, &response.session);
    crate::status_bar::refresh(&app_handle);
    Ok(response)
}

fn stop_native_capture_with_state(
    state: tauri::State<'_, NativeCaptureState>,
    app_handle: &tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let mut runtime = state.lock().expect("native capture state poisoned");
    let session_id = runtime_log_session_id(runtime.runtime()).to_string();
    let requested_sources = runtime.runtime().requested_sources.clone();
    let output_files_before_stop = runtime.runtime().output_files.clone();
    let source_session_ids_before_stop = runtime
        .runtime()
        .source_sessions
        .clone()
        .map(|source_sessions| {
            [
                source_sessions.screen,
                source_sessions.microphone,
                source_sessions.system_audio,
            ]
            .into_iter()
            .flatten()
            .map(|source_session| source_session.session_id)
            .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    debug_log::log_info(format!(
        "received native capture stop request (is_running={}, session_id='{}', requested_sources={}, output_files_before_stop={})",
        runtime.runtime().is_running,
        session_id,
        format_optional_capture_source_flags(requested_sources.as_ref()),
        format_output_file_counts(output_files_before_stop.as_ref())
    ));

    let session = match runtime.stop(app_handle) {
        Ok(session) => session,
        Err(error) => {
            if capture_screen::should_preserve_runtime_on_stop_error(&error) {
                debug_log::log_error(format!(
                    "failed to stop native capture but preserved runtime for recovery (session_id='{}'): [{}] {}",
                    session_id,
                    error.code,
                    error.message
                ));
            } else {
                debug_log::log_error(format!(
                    "failed to stop native capture; runtime marked stopped (session_id='{}'): [{}] {}",
                    session_id, error.code, error.message
                ));
            }

            return Err(error);
        }
    };
    if let Some(metadata_state) = app_handle.try_state::<CaptureMetadataState>() {
        metadata::reset_recording_session_privacy_state(metadata_state.inner());
    }

    debug_log::log_info(format!(
        "stopped native capture successfully (session_id='{}', requested_sources={}, finalized_outputs={})",
        session_log_session_id(&session),
        format_optional_capture_source_flags(session.requested_sources.as_ref()),
        format_output_file_counts(session.output_files.as_ref())
    ));

    if !source_session_ids_before_stop.is_empty() {
        if let Some(infra) = app_handle.try_state::<crate::app_infra::AppInfraState>() {
            let stopped_at = time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());
            let source_session_ids = source_session_ids_before_stop;
            let infra = std::sync::Arc::clone(&*infra);
            if let Err(error) = tauri::async_runtime::block_on(async move {
                infra
                    .capture_retention()
                    .complete_capture_sessions_for_source_session_ids(
                        &source_session_ids,
                        &stopped_at,
                        "completed",
                    )
                    .await
            }) {
                debug_log::log_error(format!(
                    "failed to mark capture session completed after stop: {error}"
                ));
            }
        }
    }

    Ok(NativeCaptureSessionResponse { session })
}

#[tauri::command]
pub fn stop_native_capture(
    state: tauri::State<'_, NativeCaptureState>,
    app_handle: tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let response = stop_native_capture_with_state(state, &app_handle)?;
    emit_native_capture_session_changed(&app_handle, &response.session);
    crate::status_bar::refresh(&app_handle);
    Ok(response)
}
