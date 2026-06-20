// Provider-agnostic helpers shared by the on-device diarization provider.
// Compiled whenever the speakrs provider is enabled.
#[cfg(feature = "speakrs")]
pub mod shared;

// Pure, default-compiled speakrs result mapping. Declared unconditionally so its
// (the plan's highest-value) unit tests run under `cargo test -p
// speaker-analysis` with NO features and no native speakrs build.
pub mod speakrs_mapping;

#[cfg(feature = "speakrs")]
pub mod speakrs;
