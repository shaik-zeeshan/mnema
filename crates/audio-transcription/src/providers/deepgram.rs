//! Deepgram pre-recorded cloud transcription provider.
//!
//! This crate cannot depend on app-infra or the keychain, so the desktop wiring site injects the
//! API-key loader and the shared auth-status cell. Error classification follows ADR 0048:
//! connectivity- and auth-shaped failures are transient liveness (requeue without burning a
//! retry), only a segment-specific rejection is a genuine per-segment failure.

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{
    TranscriptionError, TranscriptionMetadata, TranscriptionOutput, TranscriptionProvider,
    TranscriptionRequest, TranscriptionResult, TranscriptionSegment, DEEPGRAM_PROVIDER_ID,
};

/// Loads the Deepgram API key at call time (from the OS keychain, at the desktop wiring site).
/// Returns None when no key is configured.
pub type DeepgramKeyLoader = Arc<dyn Fn() -> Option<String> + Send + Sync>;

/// Shared cell holding the last API-key rejection message (or None). The provider SETS it on a
/// 401/403 rejection and CLEARS it on a successful transcription; the desktop layer reads it for
/// the Settings status line and clears it on key change. ADR 0048: a rejected key is transient
/// liveness AND must surface, since liveness-requeued jobs are otherwise silent.
pub type DeepgramAuthStatus = Arc<Mutex<Option<String>>>;

const DEEPGRAM_ENDPOINT: &str = "https://api.deepgram.com/v1/listen";
/// Deepgram's documented key-validation endpoint: GET with `Authorization: Token <key>` returns 200
/// + key details for a valid key, or an "invalid credentials" error otherwise. No audio, no billing.
const DEEPGRAM_AUTH_ENDPOINT: &str = "https://api.deepgram.com/v1/auth/token";
const AUTH_REJECTED_MESSAGE: &str = "Deepgram rejected your API key";

pub struct DeepgramProvider {
    key_loader: DeepgramKeyLoader,
    auth_status: DeepgramAuthStatus,
    client: reqwest::Client,
    endpoint: String,
}

impl DeepgramProvider {
    pub fn new(key_loader: DeepgramKeyLoader, auth_status: DeepgramAuthStatus) -> Self {
        Self {
            key_loader,
            auth_status,
            // 120s timeout so a hung upload can't wedge the transcription worker.
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("reqwest client with rustls TLS should build"),
            endpoint: DEEPGRAM_ENDPOINT.to_string(),
        }
    }

    #[cfg(test)]
    pub fn with_endpoint(
        key_loader: DeepgramKeyLoader,
        auth_status: DeepgramAuthStatus,
        endpoint: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            ..Self::new(key_loader, auth_status)
        }
    }
}

#[async_trait]
impl TranscriptionProvider for DeepgramProvider {
    fn provider(&self) -> &'static str {
        DEEPGRAM_PROVIDER_ID
    }

    async fn transcribe(
        &self,
        request: TranscriptionRequest,
    ) -> TranscriptionResult<TranscriptionOutput> {
        let key = (self.key_loader)().ok_or_else(|| {
            TranscriptionError::ProviderUnavailable("no Deepgram API key is configured".into())
        })?;

        let bytes = std::fs::read(&request.audio_path).map_err(|error| {
            TranscriptionError::InvalidRequest(format!(
                "failed to read audio file {}: {error}",
                request.audio_path.display()
            ))
        })?;

        let params = deepgram_query_params(request.model_id.as_deref(), &request.language);

        // A reqwest error here has no HTTP status (offline/timeout/DNS) => transient liveness.
        let response = self
            .client
            .post(&self.endpoint)
            .query(&params)
            .header("Authorization", format!("Token {key}"))
            .header("Content-Type", "audio/mp4")
            .body(bytes)
            .send()
            .await
            .map_err(|error| {
                TranscriptionError::TransientLiveness(format!("Deepgram request failed: {error}"))
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(error_for_failure_status(
                status.as_u16(),
                &body,
                &self.auth_status,
            ));
        }

        if let Ok(mut guard) = self.auth_status.lock() {
            *guard = None;
        }
        let body = response.text().await.map_err(|error| {
            TranscriptionError::TransientLiveness(format!(
                "failed to read Deepgram response body: {error}"
            ))
        })?;
        parse_deepgram_response(&body, &request)
    }
}

impl DeepgramProvider {
    /// Validate the configured API key against Deepgram's key-check endpoint (`GET /v1/auth/token`)
    /// — no audio, no billing. `Ok(())` means the key was accepted; a 401/403 sets the same
    /// "rejected your API key" status the Settings line reads, and a success clears it. nova-3/nova-2
    /// are always available to any account, so a valid key is the whole availability gate.
    pub async fn check_health(&self) -> TranscriptionResult<()> {
        let key = (self.key_loader)().ok_or_else(|| {
            TranscriptionError::ProviderUnavailable("no Deepgram API key is configured".into())
        })?;

        let response = self
            .client
            .get(DEEPGRAM_AUTH_ENDPOINT)
            .header("Authorization", format!("Token {key}"))
            .send()
            .await
            .map_err(|error| {
                TranscriptionError::TransientLiveness(format!("Deepgram request failed: {error}"))
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(error_for_failure_status(
                status.as_u16(),
                &body,
                &self.auth_status,
            ));
        }

        if let Ok(mut guard) = self.auth_status.lock() {
            *guard = None;
        }
        Ok(())
    }
}

/// Deepgram query params: always smart_format=true & utterances=true, plus language handling:
///  - "auto" + nova-3 → language=multi
///  - "auto" + nova-2 → detect_language=true
///  - explicit code    → language=<code>
fn deepgram_query_params(model_id: Option<&str>, language: &str) -> Vec<(String, String)> {
    let mut params = vec![
        ("smart_format".to_string(), "true".to_string()),
        ("utterances".to_string(), "true".to_string()),
    ];
    if let Some(model) = model_id {
        params.push(("model".to_string(), model.to_string()));
    }
    match language {
        // nova-3 exposes multilingual detection through `language=multi`; nova-2 exposes a broader
        // per-language list gated behind `detect_language`.
        "auto" if model_id == Some("nova-2") => {
            params.push(("detect_language".to_string(), "true".to_string()));
        }
        "auto" => params.push(("language".to_string(), "multi".to_string())),
        code => params.push(("language".to_string(), code.to_string())),
    }
    params
}

#[derive(Deserialize, Default)]
struct DeepgramBody {
    #[serde(default)]
    results: DeepgramResults,
}

#[derive(Deserialize, Default)]
struct DeepgramResults {
    #[serde(default)]
    channels: Vec<DeepgramChannel>,
    #[serde(default)]
    utterances: Vec<DeepgramUtterance>,
}

#[derive(Deserialize, Default)]
struct DeepgramChannel {
    #[serde(default)]
    alternatives: Vec<DeepgramAlternative>,
}

#[derive(Deserialize, Default)]
struct DeepgramAlternative {
    #[serde(default)]
    transcript: String,
}

#[derive(Deserialize)]
struct DeepgramUtterance {
    start: f64,
    end: f64,
    #[serde(default)]
    transcript: String,
    confidence: Option<f32>,
}

/// Parse a Deepgram pre-recorded JSON body into a TranscriptionOutput. Utterances (when present)
/// become the transcript segments; text is the space-joined utterance transcripts, falling back to
/// results.channels[0].alternatives[0].transcript. An empty transcript yields
/// TranscriptionOutput::no_speech(metadata) (a successful no-speech transcription, per CONTEXT.md).
fn parse_deepgram_response(
    body: &str,
    request: &TranscriptionRequest,
) -> TranscriptionResult<TranscriptionOutput> {
    let parsed: DeepgramBody = serde_json::from_str(body).map_err(|error| {
        TranscriptionError::Transcription(format!("failed to parse Deepgram response: {error}"))
    })?;

    let segments: Vec<TranscriptionSegment> = parsed
        .results
        .utterances
        .iter()
        .map(|utterance| TranscriptionSegment {
            start_ms: (utterance.start * 1000.0).round() as u64,
            end_ms: (utterance.end * 1000.0).round() as u64,
            text: utterance.transcript.clone(),
            confidence: utterance.confidence,
        })
        .collect();

    let text = if segments.is_empty() {
        parsed
            .results
            .channels
            .first()
            .and_then(|channel| channel.alternatives.first())
            .map(|alternative| alternative.transcript.clone())
            .unwrap_or_default()
    } else {
        segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };

    let mut metadata = TranscriptionMetadata::from_request(request);
    metadata.segments = segments;
    // Segment-preferred derivation is enough; leave words empty.

    let version = request
        .model_id
        .clone()
        .unwrap_or_else(|| DEEPGRAM_PROVIDER_ID.to_string());

    let output = if text.is_empty() {
        TranscriptionOutput::no_speech(metadata)
    } else {
        TranscriptionOutput::new(text, metadata)
    };
    Ok(output.with_provider_version(version))
}

/// Classify an HTTP failure status into the queue's error model (ADR 0048), setting the auth cell
/// on a rejected key. 401/403 → set auth_status + TransientLiveness; 429 and 5xx → TransientLiveness;
/// any other 4xx → genuine per-segment rejection = TranscriptionError::Transcription(..).
fn error_for_failure_status(
    status: u16,
    body: &str,
    auth_status: &DeepgramAuthStatus,
) -> TranscriptionError {
    match status {
        401 | 403 => {
            if let Ok(mut guard) = auth_status.lock() {
                *guard = Some(AUTH_REJECTED_MESSAGE.to_string());
            }
            TranscriptionError::TransientLiveness(format!(
                "Deepgram rejected the API key (HTTP {status})"
            ))
        }
        429 | 500..=599 => TranscriptionError::TransientLiveness(format!(
            "Deepgram is temporarily unavailable (HTTP {status})"
        )),
        _ => TranscriptionError::Transcription(format!(
            "Deepgram rejected the segment (HTTP {status}): {body}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> TranscriptionRequest {
        TranscriptionRequest::new(
            "/tmp/audio.m4a",
            DEEPGRAM_PROVIDER_ID,
            Some("nova-3".to_string()),
            "auto",
        )
    }

    #[test]
    fn query_params_language_handling() {
        let nova3 = deepgram_query_params(Some("nova-3"), "auto");
        assert!(nova3.contains(&("smart_format".to_string(), "true".to_string())));
        assert!(nova3.contains(&("utterances".to_string(), "true".to_string())));
        assert!(nova3.contains(&("language".to_string(), "multi".to_string())));

        let nova2 = deepgram_query_params(Some("nova-2"), "auto");
        assert!(nova2.contains(&("detect_language".to_string(), "true".to_string())));
        assert!(!nova2.contains(&("language".to_string(), "multi".to_string())));

        let explicit = deepgram_query_params(Some("nova-3"), "en");
        assert!(explicit.contains(&("language".to_string(), "en".to_string())));
    }

    #[test]
    fn parses_utterances_into_segments() {
        let body = r#"{
            "metadata": { "request_id": "abc", "models": ["nova-3"] },
            "results": {
                "channels": [ { "alternatives": [ { "transcript": "hello world", "confidence": 0.99 } ] } ],
                "utterances": [ { "start": 0.12, "end": 0.98, "confidence": 0.97, "transcript": "hello world" } ]
            }
        }"#;

        let output = parse_deepgram_response(body, &request()).expect("parse should succeed");
        assert_eq!(output.text, "hello world");
        assert_eq!(output.metadata.segments.len(), 1);
        let segment = &output.metadata.segments[0];
        assert_eq!(segment.start_ms, 120);
        assert_eq!(segment.end_ms, 980);
        assert!((segment.confidence.expect("confidence") - 0.97).abs() < 1e-6);
        assert_eq!(output.metadata.provider, DEEPGRAM_PROVIDER_ID);
    }

    #[test]
    fn empty_transcript_is_no_speech_success() {
        let body = r#"{
            "results": {
                "channels": [ { "alternatives": [ { "transcript": "", "confidence": 0.0 } ] } ],
                "utterances": []
            }
        }"#;

        let output = parse_deepgram_response(body, &request()).expect("parse should succeed");
        assert!(output.text.is_empty());
    }

    #[test]
    fn failure_status_classification() {
        let auth = DeepgramAuthStatus::default();
        let rejected = error_for_failure_status(401, "unauthorized", &auth);
        assert!(matches!(rejected, TranscriptionError::TransientLiveness(_)));
        assert_eq!(
            auth.lock().expect("lock").as_deref(),
            Some("Deepgram rejected your API key")
        );

        let auth = DeepgramAuthStatus::default();
        assert!(matches!(
            error_for_failure_status(429, "", &auth),
            TranscriptionError::TransientLiveness(_)
        ));
        assert!(matches!(
            error_for_failure_status(503, "", &auth),
            TranscriptionError::TransientLiveness(_)
        ));
        assert!(auth.lock().expect("lock").is_none());

        let auth = DeepgramAuthStatus::default();
        let genuine = error_for_failure_status(400, "bad audio", &auth);
        assert!(matches!(genuine, TranscriptionError::Transcription(_)));
        assert!(auth.lock().expect("lock").is_none());
    }
}
