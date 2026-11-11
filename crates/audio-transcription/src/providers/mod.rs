pub mod apple_speech;
pub mod deepgram;
pub mod local_whisper;
pub mod parakeet;

pub use apple_speech::{
    AppleSpeechOnDeviceAvailability, AppleSpeechOnDeviceAvailabilityStatus,
    AppleSpeechOnDeviceProvider,
};
pub use deepgram::{DeepgramAuthStatus, DeepgramKeyLoader, DeepgramProvider};
pub use local_whisper::{
    ConfiguredLocalWhisperProvider, LocalWhisperModelSelection, LocalWhisperProvider,
};
pub use parakeet::{
    ConfiguredParakeetProvider, ParakeetAvailability, ParakeetAvailabilityStatus,
    ParakeetOnnxBundleLayout, ParakeetProvider, PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID,
    PARAKEET_TDT_0_6B_V3_ONNX_MODEL_ID,
};

/// Whether this build carries `provider`'s engine: Whisper and Parakeet sit behind cargo features,
/// and a selection whose engine is missing fails every job as a retryable outage.
pub fn provider_runtime_available(provider: &str) -> bool {
    match provider {
        crate::LOCAL_WHISPER_PROVIDER_ID => cfg!(feature = "local-whisper"),
        crate::PARAKEET_PROVIDER_ID => cfg!(feature = "parakeet-onnx"),
        crate::APPLE_SPEECH_ON_DEVICE_PROVIDER_ID => cfg!(target_os = "macos"),
        crate::DEEPGRAM_PROVIDER_ID => true,
        _ => false,
    }
}
