//! The GPUI binary compiles the same on-device engines as the Tauri app.
//!
//! Without `local-whisper` every job on the default transcription provider fails
//! with "not enabled in this build", which the queue treats as an outage and
//! requeues forever: no transcripts, silently. Tesseract and PaddleOCR fail the
//! same way. These features come from this crate's manifest (Cargo unifies them
//! into app-core's copy of the engine crates), so this test pins them here.

use audio_transcription::{
    providers::LocalWhisperProvider, TranscriptionProvider, TranscriptionRequest,
    LOCAL_WHISPER_PROVIDER_ID,
};

#[test]
fn ocr_engines_are_compiled_in() {
    for provider in [ocr::TESSERACT_PROVIDER_ID, ocr::PADDLE_OCR_PROVIDER_ID] {
        assert!(
            ocr::provider_runtime_available(provider),
            "{provider} OCR runtime is not compiled into mnema-gpui"
        );
    }
}

#[test]
fn local_whisper_runtime_is_compiled_in() {
    let dir = tempfile::tempdir().unwrap();
    let model = LocalWhisperProvider::model_path_in_store(dir.path(), "base").unwrap();
    std::fs::create_dir_all(model.parent().unwrap()).unwrap();
    std::fs::write(&model, b"not a ggml model").unwrap();
    let audio = dir.path().join("clip.m4a");
    std::fs::write(&audio, b"not audio").unwrap();

    // Both files exist, so the provider gets past its install checks: a build with
    // the runtime fails on decoding the bogus audio; one without it answers
    // "not enabled in this build" before touching either file.
    let request = TranscriptionRequest::new(
        &audio,
        LOCAL_WHISPER_PROVIDER_ID,
        Some("base".to_string()),
        "en",
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result =
        runtime.block_on(LocalWhisperProvider::with_models_dir(dir.path()).transcribe(request));
    let error = result
        .expect_err("bogus audio cannot transcribe")
        .to_string();
    assert!(
        !error.contains("not enabled in this build"),
        "local Whisper runtime is not compiled into mnema-gpui: {error}"
    );
}
