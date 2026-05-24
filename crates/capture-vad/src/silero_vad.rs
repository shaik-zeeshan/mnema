use std::fmt;
use std::path::{Path, PathBuf};

#[cfg(feature = "silero")]
pub(super) const SILERO_VAD_MODEL_ENV: &str = "MNEMA_SILERO_VAD_MODEL";
#[cfg(feature = "silero")]
pub(super) const DEFAULT_SILERO_VAD_MODEL_RELATIVE_PATH: &str = "resources/vad/silero_vad.onnx";
#[cfg(feature = "silero")]
const SPEECH_PROBABILITY_THRESHOLD: f32 = 0.5;

pub(super) struct SileroVadAdapter {
    #[cfg(feature = "silero")]
    session: silero::Session,
    #[cfg(feature = "silero")]
    stream: silero::StreamState,
    model_path: Option<PathBuf>,
    #[cfg(feature = "silero")]
    sample_scratch: Vec<f32>,
}

// The adapter is owned by NativeCaptureRuntime and only accessed behind the
// runtime mutex. Moving that owner between threads is safe; concurrent access
// still requires the mutex and mutable methods.
unsafe impl Send for SileroVadAdapter {}

#[cfg(feature = "silero")]
impl fmt::Debug for SileroVadAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SileroVadAdapter")
            .field("model_path", &self.model_path)
            .field("sample_rate_hz", &self.stream.sample_rate().hz())
            .finish()
    }
}

#[cfg(not(feature = "silero"))]
impl fmt::Debug for SileroVadAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SileroVadAdapter")
            .field("model_path", &self.model_path)
            .field("enabled", &false)
            .finish()
    }
}

impl SileroVadAdapter {
    #[cfg(feature = "silero")]
    pub(super) fn load_default() -> Result<Self, SileroVadLoadError> {
        match resolve_model_path(default_model_candidates()) {
            Ok(model_path) => Self::load_from_model_path(model_path),
            Err(SileroVadLoadError::MissingModel { .. }) => Self::load_bundled(),
            Err(error) => Err(error),
        }
    }

    #[cfg(not(feature = "silero"))]
    pub(super) fn load_default() -> Result<Self, SileroVadLoadError> {
        Err(SileroVadLoadError::RuntimeUnavailable {
            model_path: None,
            reason: "Silero VAD support is not enabled in this build".to_string(),
        })
    }

    #[cfg(feature = "silero")]
    fn load_bundled() -> Result<Self, SileroVadLoadError> {
        let session =
            silero::Session::bundled().map_err(|error| SileroVadLoadError::RuntimeUnavailable {
                model_path: None,
                reason: error.to_string(),
            })?;
        Ok(Self::from_session(session, None))
    }

    #[cfg(feature = "silero")]
    fn load_from_model_path(model_path: PathBuf) -> Result<Self, SileroVadLoadError> {
        let session = silero::Session::from_file(&model_path).map_err(|error| {
            SileroVadLoadError::RuntimeUnavailable {
                model_path: Some(model_path.clone()),
                reason: error.to_string(),
            }
        })?;
        Ok(Self::from_session(session, Some(model_path)))
    }

    #[cfg(feature = "silero")]
    fn from_session(session: silero::Session, model_path: Option<PathBuf>) -> Self {
        Self {
            session,
            stream: silero::StreamState::new(silero::SampleRate::Rate16k),
            model_path,
            sample_scratch: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub(super) fn model_path(&self) -> Option<&Path> {
        self.model_path.as_deref()
    }

    #[cfg(feature = "silero")]
    pub(super) fn process_pcm_frame(
        &mut self,
        samples: &[i16],
        sample_rate_hz: u32,
    ) -> Result<Option<bool>, SileroVadProcessError> {
        let sample_rate = silero::SampleRate::from_hz(sample_rate_hz)
            .map_err(|_| SileroVadProcessError::InvalidSampleRate(sample_rate_hz))?;

        if self.stream.sample_rate() != sample_rate {
            self.stream.set_sample_rate(sample_rate);
        }

        self.sample_scratch.clear();
        self.sample_scratch.reserve(samples.len());
        self.sample_scratch.extend(
            samples
                .iter()
                .map(|sample| f32::from(*sample) / f32::from(i16::MAX)),
        );

        let mut emitted_probability = false;
        let mut speech_detected = false;
        self.session
            .process_stream(&mut self.stream, &self.sample_scratch, |probability| {
                emitted_probability = true;
                speech_detected |= probability >= SPEECH_PROBABILITY_THRESHOLD;
            })
            .map_err(|error| SileroVadProcessError::InferenceFailed(error.to_string()))?;

        Ok(emitted_probability.then_some(speech_detected))
    }

    #[cfg(not(feature = "silero"))]
    pub(super) fn process_pcm_frame(
        &mut self,
        _samples: &[i16],
        _sample_rate_hz: u32,
    ) -> Result<Option<bool>, SileroVadProcessError> {
        Err(SileroVadProcessError::InferenceFailed(
            "Silero VAD support is not enabled in this build".to_string(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SileroVadLoadError {
    #[cfg(any(feature = "silero", test))]
    MissingModel { candidates: Vec<PathBuf> },
    RuntimeUnavailable {
        model_path: Option<PathBuf>,
        reason: String,
    },
}

impl SileroVadLoadError {
    pub(super) fn fallback_reason(&self) -> String {
        match self {
            #[cfg(any(feature = "silero", test))]
            Self::MissingModel { candidates } => {
                let candidates = candidates
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Silero VAD model was not found; checked: {candidates}")
            }
            Self::RuntimeUnavailable { model_path, reason } => {
                if let Some(model_path) = model_path {
                    format!(
                        "Silero VAD runtime unavailable: {reason}; model path: {}",
                        model_path.display()
                    )
                } else {
                    format!("Silero VAD runtime unavailable: {reason}; bundled model")
                }
            }
        }
    }
}

impl fmt::Display for SileroVadLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.fallback_reason())
    }
}

impl std::error::Error for SileroVadLoadError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SileroVadProcessError {
    #[cfg(feature = "silero")]
    InvalidSampleRate(u32),
    InferenceFailed(String),
}

impl fmt::Display for SileroVadProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(feature = "silero")]
            Self::InvalidSampleRate(sample_rate_hz) => {
                write!(
                    formatter,
                    "unsupported Silero VAD sample rate: {sample_rate_hz}"
                )
            }
            Self::InferenceFailed(reason) => {
                write!(formatter, "Silero VAD inference failed: {reason}")
            }
        }
    }
}

impl std::error::Error for SileroVadProcessError {}

#[cfg(feature = "silero")]
fn default_model_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(path) = std::env::var_os(SILERO_VAD_MODEL_ENV).filter(|value| !value.is_empty()) {
        candidates.push(PathBuf::from(path));
    }

    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_SILERO_VAD_MODEL_RELATIVE_PATH),
    );

    candidates
}

#[cfg(any(feature = "silero", test))]
fn resolve_model_path(candidates: Vec<PathBuf>) -> Result<PathBuf, SileroVadLoadError> {
    candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .ok_or(SileroVadLoadError::MissingModel { candidates })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_model_reports_all_checked_paths() {
        let missing = PathBuf::from("/tmp/mnema-silero-vad-test-missing.onnx");
        let error = resolve_model_path(vec![missing.clone()])
            .expect_err("missing model should be unavailable");

        assert_eq!(
            error,
            SileroVadLoadError::MissingModel {
                candidates: vec![missing.clone()]
            }
        );
        assert!(error
            .fallback_reason()
            .contains(&missing.display().to_string()));
    }

    #[cfg(feature = "silero")]
    #[test]
    fn bundled_model_loads_without_app_resource_file() {
        let adapter = SileroVadAdapter::load_default().expect("bundled Silero model should load");

        assert!(adapter.model_path().is_none());
    }

    #[cfg(feature = "silero")]
    #[test]
    fn bundled_model_reports_non_speech_for_silence() {
        let mut adapter =
            SileroVadAdapter::load_default().expect("bundled Silero model should load");
        let silence = vec![0_i16; 512];

        let speech = adapter
            .process_pcm_frame(&silence, 16_000)
            .expect("silence should be a valid Silero frame");

        assert_eq!(speech, Some(false));
    }

    #[cfg(feature = "silero")]
    #[test]
    fn process_rejects_unsupported_sample_rate() {
        let mut adapter =
            SileroVadAdapter::load_default().expect("bundled Silero model should load");

        assert_eq!(
            adapter.process_pcm_frame(&[0_i16; 512], 48_000),
            Err(SileroVadProcessError::InvalidSampleRate(48_000))
        );
    }
}
