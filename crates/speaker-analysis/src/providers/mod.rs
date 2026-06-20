#[cfg(feature = "sherpa-onnx")]
pub mod sherpa_onnx;

// Provider-agnostic helpers shared by every on-device diarization provider.
// Compiled whenever any real provider is enabled so neither has to depend on
// the other's feature.
#[cfg(any(feature = "sherpa-onnx", feature = "speakrs"))]
pub mod shared;

// Pure, default-compiled speakrs result mapping. Declared unconditionally so its
// (the plan's highest-value) unit tests run under `cargo test -p
// speaker-analysis` with NO features and no native speakrs build.
pub mod speakrs_mapping;

#[cfg(feature = "speakrs")]
pub mod speakrs;
