use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mnema-speaker-analysis-models-{label}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("test dir should be created");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn tar_bz2_install_validates_checksum_before_unpacking() {
    let dir = TestDir::new("archive-checksum");
    let file = ModelArtifactFile {
        relative_path: "segmentation/model.onnx".to_string(),
        url: "https://example.invalid/model.tar.bz2".to_string(),
        byte_size: 15,
        sha256: Some(
            "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        ),
    };

    let result = install_downloaded_model_file(
        dir.path(),
        &file,
        Path::new("segmentation/model.onnx"),
        b"not a tar bz2 archive",
    );

    assert!(matches!(
        result,
        Err(ModelDownloadError::Install(
            speaker_analysis::ModelInstallError::ChecksumMismatch { .. }
        ))
    ));
    assert!(!dir.path().join("segmentation/model.onnx").exists());
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// speakrs ships RAW files (no `.tar.bz2`): each artifact file is fetched
/// directly and placed verbatim at its `relative_path`, preserving nested
/// `.mlmodelc/...` subpaths. The install path must checksum the raw bytes
/// and write them where speakrs's `from_dir` reads them.
#[test]
fn raw_multi_file_install_places_nested_file_after_checksum() {
    let dir = TestDir::new("raw-nested");
    let bytes = b"coreml weight payload";
    let file = ModelArtifactFile {
        relative_path: "segmentation-3.0.mlmodelc/weights/weight.bin".to_string(),
        url: "https://huggingface.co/avencera/speakrs-models/resolve/main/segmentation-3.0.mlmodelc/weights/weight.bin".to_string(),
        byte_size: bytes.len() as u64,
        sha256: Some(sha256_hex(bytes)),
    };

    let destination =
        safe_relative_model_file_path(&file.relative_path).expect("relative path is safe");
    install_downloaded_model_file(dir.path(), &file, &destination, bytes)
        .expect("raw nested file installs");

    let placed = dir
        .path()
        .join("segmentation-3.0.mlmodelc/weights/weight.bin");
    assert!(
        placed.is_file(),
        "raw file lands at its nested relative_path"
    );
    assert_eq!(std::fs::read(&placed).unwrap(), bytes);
    // The temp staging file is cleaned up and nothing is extracted.
    assert!(!dir.path().join(".download.tmp").exists());
}

/// A raw (non-archive) file with a wrong checksum must fail before being
/// written to its destination.
#[test]
fn raw_multi_file_install_rejects_checksum_mismatch() {
    let dir = TestDir::new("raw-checksum");
    let file = ModelArtifactFile {
        relative_path: "plda_lda.npy".to_string(),
        url: "https://huggingface.co/avencera/speakrs-models/resolve/main/plda_lda.npy".to_string(),
        byte_size: 4,
        sha256: Some(
            "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        ),
    };

    let result =
        install_downloaded_model_file(dir.path(), &file, Path::new("plda_lda.npy"), b"data");

    assert!(matches!(
        result,
        Err(ModelDownloadError::Install(
            speaker_analysis::ModelInstallError::ChecksumMismatch { .. }
        ))
    ));
    assert!(!dir.path().join("plda_lda.npy").exists());
}

/// The status response surfaces ONE group per actual provider. With sherpa
/// removed, speakrs is the sole provider group, carrying its own descriptor.
/// This is what lets the settings UI show the speakrs preset as a selectable,
/// downloadable option.
#[test]
fn status_response_groups_models_by_actual_provider() {
    let dir = TestDir::new("provider-grouping");

    let response = build_speaker_analysis_model_status_response(dir.path())
        .expect("status response builds for an empty models dir");

    // sherpa is gone: speakrs is the only provider group.
    assert_eq!(response.providers.len(), 1);
    let speakrs = response
        .providers
        .iter()
        .find(|group| group.provider == speaker_analysis::SPEAKRS_PROVIDER_ID)
        .expect("speakrs group present");

    // Every model in the group belongs to the speakrs provider.
    assert!(speakrs
        .models
        .iter()
        .all(|model| model.provider == speaker_analysis::SPEAKRS_PROVIDER_ID));

    // The speakrs preset is reachable, downloadable, and carries its license.
    let speakrs_default = speakrs
        .models
        .iter()
        .find(|model| model.model_id.as_deref() == Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID))
        .expect("speakrs default preset present");
    assert!(speakrs_default.download.is_some());
    assert!(speakrs_default
        .license_label
        .as_deref()
        .is_some_and(|label| label.contains("CC-BY-4.0")));
    assert_eq!(speakrs.display_name, "On-device (CoreML)");
}

/// The speaker_analysis half of the producer/consumer key agreement, and the one most
/// able to drift: the claim predicate does NOT concatenate this processor's payload the
/// way it does for ocr/audio_transcription. It binds a CONSTANT — `model_key(
/// SPEAKRS_PROVIDER_ID, SPEAKRS_DEFAULT_MODEL_ID)` — precisely so a legacy sherpa-keyed
/// job is still skipped when the speakrs model is locked. The producer instead runs the
/// selection through `normalize_model_selection`. Those two routes reach the same string
/// only while speakrs ships exactly one model; this test is what fails if that stops
/// being true, which is the condition the claim-side NOTE asks to be revisited.
#[test]
fn an_uninstalled_speaker_model_locks_the_constant_the_claim_predicate_binds() {
    let dir = TestDir::new("absent-key");
    let expected = format!(
        "{}/{}",
        speaker_analysis::SPEAKRS_PROVIDER_ID,
        speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID
    );

    let mut settings = crate::recording_settings::default_recording_settings().speaker_analysis;
    settings.provider = speaker_analysis::SPEAKRS_PROVIDER_ID.to_string();
    settings.model_id = Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID.to_string());
    assert_eq!(
        absent_selected_speaker_analysis_model_key(dir.path(), &settings),
        Some(expected.clone()),
        "the absent lock key must equal the constant the claim predicate binds"
    );

    // A legacy sherpa selection normalizes onto the speakrs default, so it must lock the
    // SAME key — otherwise a user carrying an old selection gets no parking at all.
    settings.provider = "sherpa_onnx".to_string();
    settings.model_id = Some("nemo-small".to_string());
    assert_eq!(
        absent_selected_speaker_analysis_model_key(dir.path(), &settings),
        Some(expected.clone()),
        "a legacy selection normalizes onto speakrs and locks the speakrs key"
    );

    // An empty selection normalizes onto the speakrs default too, rather than vanishing.
    settings.provider = String::new();
    settings.model_id = None;
    assert_eq!(
        absent_selected_speaker_analysis_model_key(dir.path(), &settings),
        Some(expected),
    );
}
