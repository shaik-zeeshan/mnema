#![allow(unexpected_cfgs)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    TranscriptionError, TranscriptionMetadata, TranscriptionOutput, TranscriptionProvider,
    TranscriptionRequest, TranscriptionResult, TranscriptionSegment, TranscriptionWord,
    APPLE_SPEECH_ON_DEVICE_PROVIDER_ID,
};

#[cfg(target_os = "macos")]
use std::{sync::mpsc, time::Duration};

#[cfg(target_os = "macos")]
use block2::RcBlock;
#[cfg(target_os = "macos")]
use objc2::{rc::Retained, AnyThread};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSError, NSLocale, NSString, NSURL};
#[cfg(target_os = "macos")]
use objc2_speech::{
    SFSpeechRecognitionResult, SFSpeechRecognitionTask, SFSpeechRecognizer,
    SFSpeechRecognizerAuthorizationStatus, SFSpeechURLRecognitionRequest,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct AppleSpeechOnDeviceProvider;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppleSpeechOnDeviceAvailability {
    pub available: bool,
    pub status: AppleSpeechOnDeviceAvailabilityStatus,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppleSpeechOnDeviceAvailabilityStatus {
    Available,
    UnsupportedPlatform,
    FrameworkUnavailable,
    PermissionNotDetermined,
    PermissionDenied,
    PermissionRestricted,
    RecognizerUnavailable,
    OnDeviceRecognitionUnavailable,
}

#[derive(Debug)]
#[cfg(target_os = "macos")]
enum AppleSpeechRecognitionEvent {
    Completed(TranscriptionOutput),
    Failed(TranscriptionError),
}

#[derive(Debug, Clone)]
#[cfg(target_os = "macos")]
struct AppleSpeechRecognizerHandle {
    recognizer: Retained<SFSpeechRecognizer>,
    locale_identifier: String,
}

impl AppleSpeechOnDeviceProvider {
    pub fn availability() -> AppleSpeechOnDeviceAvailability {
        Self::availability_for_language("auto")
    }

    pub fn availability_for_language(language: &str) -> AppleSpeechOnDeviceAvailability {
        apple_speech_on_device_availability(language)
    }

    pub fn request_permission() -> AppleSpeechOnDeviceAvailability {
        request_apple_speech_recognition_permission()
    }
}

#[async_trait]
impl TranscriptionProvider for AppleSpeechOnDeviceProvider {
    fn provider(&self) -> &'static str {
        APPLE_SPEECH_ON_DEVICE_PROVIDER_ID
    }

    async fn transcribe(
        &self,
        request: TranscriptionRequest,
    ) -> TranscriptionResult<TranscriptionOutput> {
        if request.provider != APPLE_SPEECH_ON_DEVICE_PROVIDER_ID {
            return Err(TranscriptionError::InvalidRequest(format!(
                "Apple Speech on-device provider received request for {}",
                request.provider
            )));
        }

        let availability = Self::availability_for_language(&request.language);
        if !availability.available {
            return Err(TranscriptionError::ProviderUnavailable(
                availability.message,
            ));
        }

        #[cfg(target_os = "macos")]
        {
            transcribe_with_apple_speech(request)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = request;
            Err(TranscriptionError::ProviderUnavailable(
                "Apple Speech on-device recognition is only available on macOS".to_string(),
            ))
        }
    }
}

#[cfg(target_os = "macos")]
#[link(name = "Speech", kind = "framework")]
unsafe extern "C" {}

#[cfg(target_os = "macos")]
fn transcribe_with_apple_speech(
    request: TranscriptionRequest,
) -> TranscriptionResult<TranscriptionOutput> {
    if !request.audio_path.is_file() {
        return Err(TranscriptionError::InvalidRequest(format!(
            "audio file does not exist: {}",
            request.audio_path.display()
        )));
    }

    let recognizer = recognizer_for_language(&request.language)?;
    let path_str = request.audio_path.to_str().ok_or_else(|| {
        TranscriptionError::InvalidRequest(format!(
            "audio path is not valid UTF-8 for Apple Speech: {}",
            request.audio_path.display()
        ))
    })?;

    let ns_path = NSString::from_str(path_str);
    let url = NSURL::fileURLWithPath(&ns_path);
    let request_object = unsafe {
        SFSpeechURLRecognitionRequest::initWithURL(SFSpeechURLRecognitionRequest::alloc(), &url)
    };

    unsafe {
        request_object.setRequiresOnDeviceRecognition(true);
        request_object.setShouldReportPartialResults(false);
    }

    let (sender, receiver) = mpsc::channel();
    let request_for_handler = request.clone();
    let locale_identifier = recognizer.locale_identifier.clone();
    let result_handler = RcBlock::new(
        move |result_ptr: *mut SFSpeechRecognitionResult, error_ptr: *mut NSError| {
            if let Some(result) = unsafe { result_ptr.as_ref() } {
                if !unsafe { result.isFinal() } {
                    return;
                }

                let event = match transcription_output_from_result(
                    &request_for_handler,
                    &locale_identifier,
                    result,
                ) {
                    Ok(output) => AppleSpeechRecognitionEvent::Completed(output),
                    Err(error) => AppleSpeechRecognitionEvent::Failed(error),
                };
                let _ = sender.send(event);
                return;
            }

            if let Some(error) = unsafe { error_ptr.as_ref() } {
                if is_no_speech_error(error) {
                    let _ = sender.send(AppleSpeechRecognitionEvent::Completed(no_speech_output(
                        &request_for_handler,
                        &locale_identifier,
                    )));
                    return;
                }

                let _ = sender.send(AppleSpeechRecognitionEvent::Failed(
                    TranscriptionError::Transcription(format_apple_speech_error(error)),
                ));
            }
        },
    );

    let task: Retained<SFSpeechRecognitionTask> = unsafe {
        recognizer
            .recognizer
            .recognitionTaskWithRequest_resultHandler(&request_object, &result_handler)
    };

    match receiver.recv_timeout(Duration::from_secs(300)) {
        Ok(AppleSpeechRecognitionEvent::Completed(output)) => Ok(output),
        Ok(AppleSpeechRecognitionEvent::Failed(error)) => Err(error),
        Err(_) => {
            unsafe {
                task.cancel();
            }
            Err(TranscriptionError::Transcription(
                "timed out waiting for Apple Speech recognition result".to_string(),
            ))
        }
    }
}

#[cfg(target_os = "macos")]
fn transcription_output_from_result(
    request: &TranscriptionRequest,
    locale_identifier: &str,
    result: &SFSpeechRecognitionResult,
) -> TranscriptionResult<TranscriptionOutput> {
    let transcription = unsafe { result.bestTranscription() };
    let text = unsafe { transcription.formattedString() }.to_string();
    let text = text.trim().to_string();
    let mut metadata = apple_speech_metadata(request, locale_identifier);

    for segment in unsafe { transcription.segments() }.iter() {
        let segment_text = unsafe { segment.substring() }
            .to_string()
            .trim()
            .to_string();
        let start_ms = seconds_to_ms(unsafe { segment.timestamp() });
        let end_ms = start_ms.saturating_add(seconds_to_ms(unsafe { segment.duration() }));
        let confidence = Some(unsafe { segment.confidence() }.clamp(0.0, 1.0));

        metadata.words.push(TranscriptionWord {
            start_ms,
            end_ms,
            text: segment_text.clone(),
            confidence,
        });
        metadata.segments.push(TranscriptionSegment {
            start_ms,
            end_ms,
            text: segment_text,
            confidence,
        });
    }

    let provider_version = "speech.framework/on-device";
    if text.is_empty() {
        return Ok(TranscriptionOutput::no_speech(metadata).with_provider_version(provider_version));
    }

    if metadata.segments.is_empty() {
        metadata.segments.push(TranscriptionSegment {
            start_ms: 0,
            end_ms: 0,
            text: text.clone(),
            confidence: None,
        });
    }

    Ok(TranscriptionOutput::new(text, metadata).with_provider_version(provider_version))
}

#[cfg(target_os = "macos")]
fn no_speech_output(
    request: &TranscriptionRequest,
    locale_identifier: &str,
) -> TranscriptionOutput {
    TranscriptionOutput::no_speech(apple_speech_metadata(request, locale_identifier))
        .with_provider_version("speech.framework/on-device")
}

#[cfg(target_os = "macos")]
fn apple_speech_metadata(
    request: &TranscriptionRequest,
    locale_identifier: &str,
) -> TranscriptionMetadata {
    let mut metadata = TranscriptionMetadata::from_request(request);
    metadata.provenance.insert(
        "recognizedLocaleIdentifier".to_string(),
        serde_json::Value::String(locale_identifier.to_string()),
    );
    metadata.provenance.insert(
        "requiresOnDeviceRecognition".to_string(),
        serde_json::Value::Bool(true),
    );
    metadata.provenance.insert(
        "requestKind".to_string(),
        serde_json::Value::String("url".to_string()),
    );
    metadata
}

#[cfg(target_os = "macos")]
fn recognizer_for_language(language: &str) -> TranscriptionResult<AppleSpeechRecognizerHandle> {
    let language = normalized_language_hint(language);
    let recognizer = match language {
        Some(language) => {
            let locale = NSLocale::localeWithLocaleIdentifier(&NSString::from_str(language));
            unsafe { SFSpeechRecognizer::initWithLocale(SFSpeechRecognizer::alloc(), &locale) }
                .ok_or_else(|| {
                    TranscriptionError::ProviderUnavailable(format!(
                        "Apple Speech recognizer could not be created for locale `{language}`"
                    ))
                })?
        }
        None => {
            unsafe { SFSpeechRecognizer::init(SFSpeechRecognizer::alloc()) }.ok_or_else(|| {
                TranscriptionError::ProviderUnavailable(
                    "Apple Speech recognizer could not be created for the current locale"
                        .to_string(),
                )
            })?
        }
    };

    let locale_identifier = unsafe { recognizer.locale() }
        .localeIdentifier()
        .to_string();
    Ok(AppleSpeechRecognizerHandle {
        recognizer,
        locale_identifier,
    })
}

#[cfg(target_os = "macos")]
fn apple_speech_on_device_availability(language: &str) -> AppleSpeechOnDeviceAvailability {
    if !main_bundle_has_speech_usage_description() {
        return unavailable(
            AppleSpeechOnDeviceAvailabilityStatus::FrameworkUnavailable,
            "Current macOS app bundle is missing NSSpeechRecognitionUsageDescription. Quit Mnema and launch a rebuilt .app bundle before requesting Apple Speech permission.",
        );
    }

    let authorization_status = unsafe { SFSpeechRecognizer::authorizationStatus() };
    match authorization_status {
        SFSpeechRecognizerAuthorizationStatus::NotDetermined => {
            return unavailable(
                AppleSpeechOnDeviceAvailabilityStatus::PermissionNotDetermined,
                "Apple Speech recognition permission has not been requested yet",
            );
        }
        SFSpeechRecognizerAuthorizationStatus::Denied => {
            return unavailable(
                AppleSpeechOnDeviceAvailabilityStatus::PermissionDenied,
                "Apple Speech recognition permission is denied",
            );
        }
        SFSpeechRecognizerAuthorizationStatus::Restricted => {
            return unavailable(
                AppleSpeechOnDeviceAvailabilityStatus::PermissionRestricted,
                "Apple Speech recognition permission is restricted",
            );
        }
        SFSpeechRecognizerAuthorizationStatus::Authorized => {}
        other => {
            return unavailable(
                AppleSpeechOnDeviceAvailabilityStatus::RecognizerUnavailable,
                format!(
                    "Apple Speech recognition returned unknown authorization status {}",
                    other.0
                ),
            );
        }
    }

    let recognizer = match recognizer_for_language(language) {
        Ok(recognizer) => recognizer,
        Err(error) => {
            return unavailable(
                AppleSpeechOnDeviceAvailabilityStatus::RecognizerUnavailable,
                error.to_string(),
            );
        }
    };

    if !unsafe { recognizer.recognizer.isAvailable() } {
        return unavailable(
            AppleSpeechOnDeviceAvailabilityStatus::RecognizerUnavailable,
            recognizer_unavailable_message(language, &recognizer.locale_identifier),
        );
    }
    if !unsafe { recognizer.recognizer.supportsOnDeviceRecognition() } {
        return unavailable(
            AppleSpeechOnDeviceAvailabilityStatus::OnDeviceRecognitionUnavailable,
            on_device_unavailable_message(language, &recognizer.locale_identifier),
        );
    }

    AppleSpeechOnDeviceAvailability {
        available: true,
        status: AppleSpeechOnDeviceAvailabilityStatus::Available,
        message: format!(
            "Apple Speech on-device recognition is available for locale {}",
            recognizer.locale_identifier
        ),
    }
}

#[cfg(target_os = "macos")]
fn request_apple_speech_recognition_permission() -> AppleSpeechOnDeviceAvailability {
    if !main_bundle_has_speech_usage_description() {
        return unavailable(
            AppleSpeechOnDeviceAvailabilityStatus::FrameworkUnavailable,
            "Current macOS app bundle is missing NSSpeechRecognitionUsageDescription. Quit Mnema and launch a rebuilt .app bundle before requesting Apple Speech permission.",
        );
    }

    let (sender, receiver) = mpsc::channel();
    let completion = RcBlock::new(move |authorization_status| {
        let _ = sender.send(authorization_status);
    });

    unsafe {
        SFSpeechRecognizer::requestAuthorization(&completion);
    }

    match receiver.recv_timeout(Duration::from_secs(300)) {
        Ok(_) => apple_speech_on_device_availability("auto"),
        Err(_) => unavailable(
            AppleSpeechOnDeviceAvailabilityStatus::RecognizerUnavailable,
            "Timed out waiting for Apple Speech recognition permission response",
        ),
    }
}

#[cfg(target_os = "macos")]
fn normalized_language_hint(language: &str) -> Option<&str> {
    let trimmed = language.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(target_os = "macos")]
fn recognizer_unavailable_message(language: &str, locale_identifier: &str) -> String {
    match normalized_language_hint(language) {
        Some(language) => format!(
            "Apple Speech recognizer is not currently available for requested locale `{language}` (resolved locale `{locale_identifier}`)"
        ),
        None => format!(
            "Apple Speech recognizer is not currently available for the current locale `{locale_identifier}`"
        ),
    }
}

#[cfg(target_os = "macos")]
fn on_device_unavailable_message(language: &str, locale_identifier: &str) -> String {
    match normalized_language_hint(language) {
        Some(language) => format!(
            "Apple Speech on-device recognition is unavailable for requested locale `{language}` (resolved locale `{locale_identifier}`)"
        ),
        None => format!(
            "Apple Speech on-device recognition is unavailable for the current locale `{locale_identifier}`"
        ),
    }
}

#[cfg(target_os = "macos")]
fn is_no_speech_error(error: &NSError) -> bool {
    error.code() == 1110
        || error.localizedDescription().to_string() == "Failed to recognize any speech"
}

#[cfg(target_os = "macos")]
fn format_apple_speech_error(error: &NSError) -> String {
    format!(
        "{} (domain={}, code={})",
        error.localizedDescription(),
        error.domain(),
        error.code()
    )
}

#[cfg(target_os = "macos")]
fn seconds_to_ms(value: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    (value * 1000.0).round().clamp(0.0, u64::MAX as f64) as u64
}

#[cfg(target_os = "macos")]
fn main_bundle_has_speech_usage_description() -> bool {
    let Ok(exe_path) = std::env::current_exe() else {
        return false;
    };

    let Some(macos_dir) = exe_path.parent() else {
        return false;
    };
    if macos_dir.file_name().and_then(|name| name.to_str()) != Some("MacOS") {
        return false;
    }

    let Some(contents_dir) = macos_dir.parent() else {
        return false;
    };
    if contents_dir.file_name().and_then(|name| name.to_str()) != Some("Contents") {
        return false;
    }

    let Some(app_dir) = contents_dir.parent() else {
        return false;
    };
    if app_dir.extension().and_then(|extension| extension.to_str()) != Some("app") {
        return false;
    }

    let info_plist_path = contents_dir.join("Info.plist");
    let Ok(info_plist) = std::fs::read_to_string(info_plist_path) else {
        return false;
    };

    info_plist.contains("<key>NSSpeechRecognitionUsageDescription</key>")
        && info_plist.contains("Speech")
}

#[cfg(not(target_os = "macos"))]
fn request_apple_speech_recognition_permission() -> AppleSpeechOnDeviceAvailability {
    apple_speech_on_device_availability("auto")
}

#[cfg(not(target_os = "macos"))]
fn apple_speech_on_device_availability(_language: &str) -> AppleSpeechOnDeviceAvailability {
    unavailable(
        AppleSpeechOnDeviceAvailabilityStatus::UnsupportedPlatform,
        "Apple Speech on-device recognition is only available on macOS",
    )
}

fn unavailable(
    status: AppleSpeechOnDeviceAvailabilityStatus,
    message: impl Into<String>,
) -> AppleSpeechOnDeviceAvailability {
    AppleSpeechOnDeviceAvailability {
        available: false,
        status,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_has_message() {
        let availability = AppleSpeechOnDeviceProvider::availability();
        assert!(!availability.message.is_empty());
    }

    #[test]
    fn provider_rejects_mismatched_request() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let request = TranscriptionRequest::new(
                    "/tmp/audio.m4a",
                    crate::LOCAL_WHISPER_PROVIDER_ID,
                    None,
                    "auto",
                );
                let error = AppleSpeechOnDeviceProvider
                    .transcribe(request)
                    .await
                    .expect_err("mismatched provider should fail");
                assert!(matches!(error, TranscriptionError::InvalidRequest(_)));
            });
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn normalized_language_hint_treats_auto_as_default() {
        assert_eq!(normalized_language_hint("auto"), None);
        assert_eq!(normalized_language_hint(" Auto "), None);
        assert_eq!(normalized_language_hint("en-US"), Some("en-US"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seconds_to_ms_rounds_to_nearest_millisecond() {
        assert_eq!(seconds_to_ms(0.0), 0);
        assert_eq!(seconds_to_ms(0.3334), 333);
        assert_eq!(seconds_to_ms(0.3336), 334);
        assert_eq!(seconds_to_ms(-1.0), 0);
    }
}
