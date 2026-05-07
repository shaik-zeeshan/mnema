pub mod apple_speech;
pub mod local_whisper;
pub mod parakeet;

pub use apple_speech::{
    AppleSpeechOnDeviceAvailability, AppleSpeechOnDeviceAvailabilityStatus,
    AppleSpeechOnDeviceProvider,
};
pub use local_whisper::{
    ConfiguredLocalWhisperProvider, LocalWhisperModelSelection, LocalWhisperProvider,
};
pub use parakeet::{
    ConfiguredParakeetProvider, ParakeetAvailability, ParakeetAvailabilityStatus,
    ParakeetOnnxBundleLayout, ParakeetProvider, PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID,
    PARAKEET_TDT_0_6B_V3_ONNX_MODEL_ID,
};
