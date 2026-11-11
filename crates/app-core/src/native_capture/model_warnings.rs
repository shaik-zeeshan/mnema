//! The "selected model is missing" warnings: the in-app notifications a
//! recording start or app startup raises when OCR, transcription, the speech
//! detector or speaker analysis can't run with what is installed.

use capture_types::{
    AudioTranscriptionProvider, AudioTranscriptionSettings, OcrProvider, OcrSettings,
    RecordingSettings,
};

use crate::debug_log;
use crate::native_capture::app_notifications::{
    push_app_notification, AppNotification, AppNotificationAction, AppNotificationsState,
};
use crate::native_capture::runtime;
use crate::recording_settings::{current_recording_settings, RecordingSettingsState};
use crate::AppCore;

const AUDIO_TRANSCRIPTION_UNAVAILABLE_NOTIFICATION_ID: &str = "audio-transcription-unavailable";
const OCR_UNAVAILABLE_NOTIFICATION_ID: &str = "ocr-unavailable";
const SPEECH_DETECTOR_UNAVAILABLE_NOTIFICATION_ID: &str = "speech-detector-unavailable";
const SPEAKER_ANALYSIS_UNAVAILABLE_NOTIFICATION_ID: &str = "speaker-analysis-unavailable";
const PROCESSING_SETTINGS_TAB_ID: &str = "processing";
const TRANSCRIPTION_SETTINGS_TAB_ID: &str = "transcription";
const SPEAKER_SETTINGS_TAB_ID: &str = "speakers";

pub(crate) fn should_warn_audio_transcription_unavailable_at_start(
    settings: &RecordingSettings,
) -> bool {
    settings.transcription.enabled
        && ((settings.capture_microphone && settings.transcription.microphone_enabled)
            || (settings.capture_system_audio && settings.transcription.system_audio_enabled))
}

pub(crate) fn should_warn_audio_transcription_unavailable_at_startup(
    settings: &RecordingSettings,
) -> bool {
    should_warn_audio_transcription_unavailable_at_start(settings)
}

pub(crate) fn audio_transcription_provider_label(
    provider: AudioTranscriptionProvider,
) -> &'static str {
    match provider {
        AudioTranscriptionProvider::LocalWhisper => "Local Whisper",
        AudioTranscriptionProvider::AppleSpeechOnDevice => "Apple Speech on-device recognition",
        AudioTranscriptionProvider::Parakeet => "Parakeet",
        AudioTranscriptionProvider::Deepgram => "Deepgram (cloud)",
    }
}

pub(crate) fn audio_transcription_selection_label(settings: &AudioTranscriptionSettings) -> String {
    let provider = audio_transcription_provider_label(settings.provider);
    match settings.model_id.as_deref() {
        Some(model_id) if !model_id.is_empty() => format!("{provider} `{model_id}`"),
        _ => provider.to_string(),
    }
}

/// False when this build lacks the selected provider's engine, so the fix is another engine, not
/// a download.
pub(crate) fn audio_transcription_engine_in_build(settings: &AudioTranscriptionSettings) -> bool {
    audio_transcription::providers::provider_runtime_available(
        crate::audio_transcription_models::provider_id_for_settings(settings.provider),
    )
}

pub(crate) fn audio_transcription_unavailable_notification(
    settings: &RecordingSettings,
    created_at_unix_ms: u64,
) -> AppNotification {
    let selection = audio_transcription_selection_label(&settings.transcription);
    let message = if audio_transcription_engine_in_build(&settings.transcription) {
        format!("{selection} is not available. Requested audio will not be transcribed until you install or choose an available model.")
    } else {
        format!("{selection} is not included in this build of Mnema. Requested audio will not be transcribed until you choose another transcription engine.")
    };
    AppNotification {
        id: AUDIO_TRANSCRIPTION_UNAVAILABLE_NOTIFICATION_ID.to_string(),
        severity: "warning".to_string(),
        title: "Transcription model unavailable".to_string(),
        message,
        created_at_unix_ms,
        action: Some(AppNotificationAction::OpenSettingsTab {
            tab: TRANSCRIPTION_SETTINGS_TAB_ID.to_string(),
        }),
    }
}

pub(crate) fn speech_detector_unavailable_notification(created_at_unix_ms: u64) -> AppNotification {
    AppNotification {
        id: SPEECH_DETECTOR_UNAVAILABLE_NOTIFICATION_ID.to_string(),
        severity: "warning".to_string(),
        title: "Speech detector unavailable".to_string(),
        message: "The selected speech detector is unavailable. Choose an available detector before starting this recording.".to_string(),
        created_at_unix_ms,
        action: Some(AppNotificationAction::OpenSettingsTab {
            tab: TRANSCRIPTION_SETTINGS_TAB_ID.to_string(),
        }),
    }
}

pub(crate) fn speaker_analysis_unavailable_notification(
    created_at_unix_ms: u64,
) -> AppNotification {
    AppNotification {
        id: SPEAKER_ANALYSIS_UNAVAILABLE_NOTIFICATION_ID.to_string(),
        severity: "warning".to_string(),
        title: "Speaker analysis model unavailable".to_string(),
        message: "The selected speaker analysis model is unavailable. Install or choose an available model before starting this recording.".to_string(),
        created_at_unix_ms,
        action: Some(AppNotificationAction::OpenSettingsTab {
            tab: SPEAKER_SETTINGS_TAB_ID.to_string(),
        }),
    }
}

pub(crate) fn maybe_push_audio_transcription_unavailable_warning(
    core: &AppCore,
    app_notifications_state: &AppNotificationsState,
    settings: &RecordingSettings,
    context: &str,
) {
    let app_data_dir = core.paths().config.clone();

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
                core,
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
                core,
                app_notifications_state,
                audio_transcription_unavailable_notification(settings, runtime::now_unix_ms()),
            );
        }
    }
}

pub(crate) fn maybe_push_audio_transcription_unavailable_start_warning(
    core: &AppCore,
    app_notifications_state: &AppNotificationsState,
    settings: &RecordingSettings,
) {
    if !should_warn_audio_transcription_unavailable_at_start(settings) {
        return;
    }

    maybe_push_audio_transcription_unavailable_warning(
        core,
        app_notifications_state,
        settings,
        "recording start",
    );
}

pub(crate) fn recording_requires_speech_detector(settings: &RecordingSettings) -> bool {
    settings.audio_speech_detection.detector != capture_types::AudioSpeechDetector::Off
        && settings.capture_system_audio
        && settings.transcription.enabled
        && settings.transcription.system_audio_enabled
}

pub(crate) fn selected_speech_detector_available(
    settings: &RecordingSettings,
) -> Result<bool, String> {
    if settings.audio_speech_detection.detector == capture_types::AudioSpeechDetector::Off {
        return Ok(false);
    }
    capture_vad::AudioSpeechDetectorRuntime::new(settings.audio_speech_detection.detector)
        .map(|_| true)
        .map_err(|error| error.to_string())
}

pub(crate) fn recording_requires_transcription_model(settings: &RecordingSettings) -> bool {
    settings.transcription.enabled
        && ((settings.capture_microphone && settings.transcription.microphone_enabled)
            || (settings.capture_system_audio && settings.transcription.system_audio_enabled))
}

pub(crate) fn recording_requires_speaker_analysis_model(settings: &RecordingSettings) -> bool {
    settings.speaker_analysis.separate_speakers && recording_requires_transcription_model(settings)
}

pub(crate) fn selected_speaker_analysis_model_available(
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

pub(crate) fn should_warn_ocr_unavailable_at_start(settings: &RecordingSettings) -> bool {
    settings.capture_screen && settings.ocr.enabled
}

pub(crate) fn should_warn_ocr_unavailable_at_startup(settings: &RecordingSettings) -> bool {
    should_warn_ocr_unavailable_at_start(settings)
}

pub(crate) fn ocr_provider_label(provider: OcrProvider) -> &'static str {
    match provider {
        OcrProvider::AppleVision => "Apple Vision",
        OcrProvider::Tesseract => "Tesseract",
        OcrProvider::PaddleOcr => "PaddleOCR",
    }
}

pub(crate) fn ocr_selection_label(settings: &OcrSettings) -> String {
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

pub(crate) fn ocr_unavailable_notification(
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

pub(crate) fn maybe_push_ocr_unavailable_warning(
    core: &AppCore,
    app_notifications_state: &AppNotificationsState,
    settings: &RecordingSettings,
    context: &str,
) {
    let app_data_dir = core.paths().config.clone();

    match crate::ocr_models::selected_ocr_model_available(&app_data_dir, &settings.ocr) {
        Ok(true) => {}
        Ok(false) => {
            let selection = ocr_selection_label(&settings.ocr);
            debug_log::log_warn(format!(
                "ocr unavailable at {context} (selection={selection})"
            ));
            push_app_notification(
                core,
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
                core,
                app_notifications_state,
                ocr_unavailable_notification(settings, runtime::now_unix_ms()),
            );
        }
    }
}

pub(crate) fn maybe_push_ocr_unavailable_start_warning(
    core: &AppCore,
    app_notifications_state: &AppNotificationsState,
    settings: &RecordingSettings,
) {
    if !should_warn_ocr_unavailable_at_start(settings) {
        return;
    }

    maybe_push_ocr_unavailable_warning(core, app_notifications_state, settings, "recording start");
}

pub fn maybe_push_audio_transcription_unavailable_startup_warning(core: &AppCore) {
    let settings_state = core.state::<RecordingSettingsState>();
    let settings = current_recording_settings(&settings_state);
    if !should_warn_audio_transcription_unavailable_at_startup(&settings) {
        return;
    }

    maybe_push_audio_transcription_unavailable_warning(
        core,
        &core.state::<AppNotificationsState>(),
        &settings,
        "app startup",
    );
}

pub fn maybe_push_ocr_unavailable_startup_warning(core: &AppCore) {
    let settings_state = core.state::<RecordingSettingsState>();
    let settings = current_recording_settings(&settings_state);
    if !should_warn_ocr_unavailable_at_startup(&settings) {
        return;
    }

    maybe_push_ocr_unavailable_warning(
        core,
        &core.state::<AppNotificationsState>(),
        &settings,
        "app startup",
    );
}
