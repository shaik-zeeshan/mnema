use super::*;
use audio_transcription::{
    model_install_dir, write_failed_marker, write_installed_marker, DOWNLOADING_MARKER_FILE_NAME,
    LOCAL_WHISPER_PROVIDER_ID,
};
use std::fs;

fn find_model<'a>(
    response: &'a AudioTranscriptionModelStatusResponseDto,
    provider: &str,
    model_id: Option<&str>,
) -> &'a AudioTranscriptionModelStatusDto {
    response
        .providers
        .iter()
        .flat_map(|provider_status| provider_status.models.iter())
        .find(|model| model.provider == provider && model.model_id.as_deref() == model_id)
        .expect("model status")
}

#[test]
fn status_response_uses_app_data_models_directory_not_save_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app_data_dir = temp.path().join("app-data");
    let save_directory = temp.path().join("capture-save-directory");

    let response =
        build_audio_transcription_model_status_response(&app_data_dir).expect("status response");

    assert_eq!(
        response.models_directory,
        path_to_string(&app_data_dir.join(audio_transcription::MODEL_STORE_DIR_NAME))
    );
    assert!(!Path::new(&response.models_directory).starts_with(&save_directory));
}

#[test]
fn status_response_includes_provider_and_model_options() {
    let temp = tempfile::tempdir().expect("tempdir");
    let response =
        build_audio_transcription_model_status_response(temp.path()).expect("status response");

    let local_whisper = response
        .providers
        .iter()
        .find(|provider| provider.provider == audio_transcription::LOCAL_WHISPER_PROVIDER_ID)
        .expect("local whisper provider");
    let model_ids: Vec<_> = local_whisper
        .models
        .iter()
        .map(|model| model.model_id.as_deref())
        .collect();
    assert_eq!(
        model_ids,
        vec![Some("tiny"), Some("base"), Some("small"), Some("medium")]
    );

    let base = find_model(
        &response,
        audio_transcription::LOCAL_WHISPER_PROVIDER_ID,
        Some("base"),
    );
    let download = base.download.as_ref().expect("base download artifact");
    assert_eq!(download.byte_size, 147_951_465);
    assert_eq!(
        download.sha256,
        "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"
    );
    assert_eq!(
        download.url,
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"
    );

    let parakeet = find_model(
        &response,
        audio_transcription::PARAKEET_PROVIDER_ID,
        Some("parakeet-tdt-0.6b-v3-onnx"),
    );
    let parakeet_download = parakeet
        .download
        .as_ref()
        .expect("parakeet download artifact");
    assert_eq!(parakeet_download.byte_size, 2_549_945_719);
    assert_eq!(
        parakeet_download.url,
        "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx"
    );

    let apple = find_model(
        &response,
        audio_transcription::APPLE_SPEECH_ON_DEVICE_PROVIDER_ID,
        None,
    );
    assert_eq!(
        apple.management,
        AudioTranscriptionModelManagementDto::OsManaged
    );
    assert_eq!(apple.status, ModelStatusKind::OsManaged);
    assert_eq!(apple.install_path, None);
    assert_eq!(
        apple.availability_status,
        Some(audio_transcription::providers::AppleSpeechOnDeviceProvider::availability().status)
    );
}

#[test]
fn selected_apple_speech_model_tracks_provider_availability() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = capture_types::AudioTranscriptionSettings {
        enabled: true,
        microphone_enabled: capture_types::default_audio_transcription_microphone_enabled(),
        system_audio_enabled: capture_types::default_audio_transcription_system_audio_enabled(),
        provider: capture_types::AudioTranscriptionProvider::AppleSpeechOnDevice,
        model_id: None,
        language: "auto".to_string(),
        memory_mode: capture_types::default_audio_transcription_memory_mode(),
        idle_unload_seconds: capture_types::default_audio_transcription_idle_unload_seconds(),
        chunk_seconds: capture_types::default_audio_transcription_chunk_seconds(),
    };

    assert_eq!(
        selected_audio_transcription_model_available(temp.path(), &settings)
            .expect("apple speech availability should inspect provider status"),
        audio_transcription::providers::AppleSpeechOnDeviceProvider::availability_for_language(
            &settings.language,
        )
        .available
    );
}

/// An installed model whose engine this build lacks is unavailable, so capture start refuses
/// instead of queueing jobs that fail forever as an outage (the GPUI build once shipped without
/// `local-whisper`). app-core's own build is exactly such a build.
#[test]
fn an_installed_model_without_its_engine_in_the_build_is_unavailable() {
    assert!(
        !audio_transcription::providers::provider_runtime_available(LOCAL_WHISPER_PROVIDER_ID),
        "app-core tests build audio-transcription without `local-whisper`; this test relies on it"
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let models_dir = audio_transcription::audio_transcription_models_dir(temp.path());
    let install_dir = model_install_dir(&models_dir, LOCAL_WHISPER_PROVIDER_ID, "base").unwrap();
    fs::create_dir_all(&install_dir).unwrap();
    fs::write(install_dir.join("ggml-base.bin"), b"model").unwrap();
    write_installed_marker(&models_dir, LOCAL_WHISPER_PROVIDER_ID, "base").unwrap();
    let mut settings = crate::recording_settings::default_recording_settings().transcription;
    settings.enabled = true;
    settings.provider = capture_types::AudioTranscriptionProvider::LocalWhisper;
    settings.model_id = Some("base".to_string());

    let manifest = builtin_model_manifest();
    let descriptor =
        find_model_descriptor(&manifest, LOCAL_WHISPER_PROVIDER_ID, Some("base")).unwrap();
    assert!(
        detect_model_status(&models_dir, descriptor)
            .unwrap()
            .is_available(),
        "the model files themselves are installed"
    );
    assert!(!selected_audio_transcription_model_available(temp.path(), &settings).unwrap());
}

// Denied ≠ missing (ADR 0048 amendment). These test the extracted pure
// helpers rather than the full functions: the Deepgram branch reads keys
// via the process-global vault slot, which is shared with other parallel
// tests (transcription_deepgram, mcp), so installing a denying handle in
// that slot would flake them.
#[test]
fn deepgram_denied_vault_read_keeps_enqueue_available() {
    assert!(deepgram_available_for_enqueue(Err(
        app_infra::AppInfraError::SecretVaultDenied("user denied keychain access".to_string())
    )));
    assert!(deepgram_available_for_enqueue(Ok(true)));
    assert!(!deepgram_available_for_enqueue(Ok(false)));
    assert!(!deepgram_available_for_enqueue(Err(
        app_infra::AppInfraError::SecretVault("vault file unreadable".to_string())
    )));
}

#[test]
fn deepgram_status_reports_denial_message_not_missing_key_copy() {
    let (available, message) = deepgram_status_fields(Err(
        app_infra::AppInfraError::SecretVaultDenied("user denied keychain access".to_string()),
    ));
    assert!(!available);
    let message = message.expect("denied read must carry a failure message");
    assert!(
        message.contains("user denied keychain access"),
        "expected the vault denial reason, got: {message}"
    );
    assert!(
        !message.contains("Add a Deepgram API key"),
        "denied must not render the missing-key copy, got: {message}"
    );

    let (available, message) = deepgram_status_fields(Ok(false));
    assert!(!available);
    assert_eq!(
        message.as_deref(),
        Some("Add a Deepgram API key in Settings to enable cloud transcription.")
    );

    let (available, message) = deepgram_status_fields(Ok(true));
    assert!(available);
    assert_eq!(message, None);
}

#[test]
fn only_one_model_download_can_be_claimed_at_a_time() {
    let state = AudioTranscriptionModelDownloadState::default();
    let first_cancel = Arc::new(AtomicBool::new(false));
    claim_model_download(
        &state,
        LOCAL_WHISPER_PROVIDER_ID,
        "base",
        Arc::clone(&first_cancel),
    )
    .expect("first claim");

    let second = claim_model_download(
        &state,
        LOCAL_WHISPER_PROVIDER_ID,
        "small",
        Arc::new(AtomicBool::new(false)),
    )
    .expect_err("second claim should fail");

    assert!(matches!(second, ModelDownloadError::AlreadyRunning { .. }));
    let active = state.lock().expect("state");
    assert_eq!(active.as_ref().expect("active").model_id, "base");
}

#[test]
fn staged_download_temp_paths_are_unique_for_same_file_names() {
    let temp = tempfile::tempdir().expect("tempdir");
    let encoder = safe_relative_model_file_path("encoder/model.onnx").expect("encoder path");
    let decoder = safe_relative_model_file_path("decoder/model.onnx").expect("decoder path");

    let encoder_temp = staged_download_temp_path(temp.path(), &encoder, 0);
    let decoder_temp = staged_download_temp_path(temp.path(), &decoder, 1);

    assert_ne!(encoder_temp, decoder_temp);
}

#[test]
fn status_response_marks_installed_parakeet_int8_model_available() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app_data_dir = temp.path();
    let models_dir = audio_transcription_models_dir(app_data_dir);
    let install_dir = model_install_dir(
        &models_dir,
        audio_transcription::PARAKEET_PROVIDER_ID,
        "parakeet-tdt-0.6b-v3-onnx-int8",
    )
    .expect("parakeet int8 dir");
    fs::create_dir_all(&install_dir).expect("install dir");
    for file_name in [
        "config.json",
        "nemo128.onnx",
        "encoder-model.int8.onnx",
        "decoder_joint-model.int8.onnx",
        "vocab.txt",
    ] {
        fs::write(install_dir.join(file_name), b"model").expect("model file");
    }
    write_installed_marker(
        &models_dir,
        audio_transcription::PARAKEET_PROVIDER_ID,
        "parakeet-tdt-0.6b-v3-onnx-int8",
    )
    .expect("installed marker");

    let response =
        build_audio_transcription_model_status_response(app_data_dir).expect("status response");
    let model = find_model(
        &response,
        audio_transcription::PARAKEET_PROVIDER_ID,
        Some("parakeet-tdt-0.6b-v3-onnx-int8"),
    );

    assert_eq!(model.status, ModelStatusKind::Installed);
    assert!(model.available);
    assert_eq!(model.failure_message, None);
}

#[test]
fn status_response_reports_app_managed_statuses() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app_data_dir = temp.path();
    let models_dir = audio_transcription_models_dir(app_data_dir);

    write_failed_marker(
        &models_dir,
        LOCAL_WHISPER_PROVIDER_ID,
        "tiny",
        "checksum mismatch",
    )
    .expect("failed marker");

    let base_dir =
        model_install_dir(&models_dir, LOCAL_WHISPER_PROVIDER_ID, "base").expect("base dir");
    fs::create_dir_all(&base_dir).expect("base dir");
    fs::write(base_dir.join(DOWNLOADING_MARKER_FILE_NAME), b"").expect("downloading marker");

    let small_dir =
        model_install_dir(&models_dir, LOCAL_WHISPER_PROVIDER_ID, "small").expect("small dir");
    fs::create_dir_all(&small_dir).expect("small dir");
    fs::write(small_dir.join("ggml-small.bin"), b"model").expect("model file");
    write_installed_marker(&models_dir, LOCAL_WHISPER_PROVIDER_ID, "small")
        .expect("installed marker");

    let response =
        build_audio_transcription_model_status_response(app_data_dir).expect("status response");

    assert_eq!(
        find_model(&response, LOCAL_WHISPER_PROVIDER_ID, Some("tiny")).status,
        ModelStatusKind::Failed
    );
    assert_eq!(
        find_model(&response, LOCAL_WHISPER_PROVIDER_ID, Some("tiny"))
            .failure_message
            .as_deref(),
        Some("checksum mismatch")
    );
    assert_eq!(
        find_model(&response, LOCAL_WHISPER_PROVIDER_ID, Some("base")).status,
        ModelStatusKind::Downloading
    );
    assert_eq!(
        find_model(&response, LOCAL_WHISPER_PROVIDER_ID, Some("small")).status,
        ModelStatusKind::Installed
    );
    assert_eq!(
        find_model(&response, LOCAL_WHISPER_PROVIDER_ID, Some("medium")).status,
        ModelStatusKind::Missing
    );
}

#[test]
fn delete_unused_audio_transcription_models_preserves_models_referenced_by_processing_jobs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app_data_dir = temp.path();
    let models_dir = audio_transcription_models_dir(app_data_dir);
    let install_dir =
        model_install_dir(&models_dir, LOCAL_WHISPER_PROVIDER_ID, "base").expect("base dir");
    std::fs::create_dir_all(&install_dir).expect("install dir should be created");
    std::fs::write(install_dir.join("ggml-base.bin"), b"model").expect("model file");
    let protected_models = BTreeSet::from([model_key(LOCAL_WHISPER_PROVIDER_ID, "base")]);

    let response = delete_unused_audio_transcription_models_inner(
        app_data_dir,
        audio_transcription::APPLE_SPEECH_ON_DEVICE_PROVIDER_ID,
        None,
        None,
        &protected_models,
        0,
    )
    .expect("unused model deletion should succeed");

    assert!(install_dir.exists());
    assert!(response.deleted.is_empty());
    assert_eq!(response.skipped_processing_jobs.len(), 1);
    assert_eq!(
        response.skipped_processing_jobs[0].provider,
        LOCAL_WHISPER_PROVIDER_ID
    );
    assert_eq!(response.skipped_processing_jobs[0].model_id, "base");
}

#[test]
fn retargetable_audio_transcription_deletion_model_keys_exclude_running_job_models() {
    let deletion_candidates = BTreeSet::from([
        model_key(LOCAL_WHISPER_PROVIDER_ID, "base"),
        model_key(LOCAL_WHISPER_PROVIDER_ID, "small"),
    ]);
    let running_job_models = BTreeSet::from([model_key(LOCAL_WHISPER_PROVIDER_ID, "base")]);

    let retargetable = retargetable_deletion_model_keys(&deletion_candidates, &running_job_models);

    assert_eq!(
        retargetable,
        BTreeSet::from([model_key(LOCAL_WHISPER_PROVIDER_ID, "small")])
    );
}

/// See the sibling test in `ocr_models.rs`. The claim predicate derives an
/// audio_transcription job's key as `TRIM($.provider) || '/' || TRIM($.modelId)`; the
/// absent lane must produce exactly that, or a selected-but-missing transcription model
/// takes a lock no job matches and its jobs die the six-minute death this PR exists to
/// stop — with every store-level test still green.
#[test]
fn an_uninstalled_transcription_model_locks_the_key_the_claim_predicate_derives() {
    let dir = std::env::temp_dir().join(format!(
        "mnema-transcription-absent-key-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(&dir).expect("test dir should be created");

    let mut settings = crate::recording_settings::default_recording_settings().transcription;
    settings.provider = capture_types::AudioTranscriptionProvider::LocalWhisper;
    settings.model_id = Some("base".to_string());

    assert_eq!(
        absent_selected_audio_transcription_model_key(&dir, &settings),
        Some(format!("{LOCAL_WHISPER_PROVIDER_ID}/base")),
        "the absent lock key must be provider/modelId — the shape the claim SQL builds"
    );

    // A model id the manifest does not know still takes a lock, and that is the safe
    // direction: such a job can never succeed, so parking it costs nothing, while NOT
    // locking it would send it down the terminal-failure path and destroy the transcript
    // — the exact loss claim-time gating exists to prevent. The key keeps the same shape,
    // so the claim predicate matches it.
    settings.model_id = Some("no-such-whisper-model".to_string());
    assert_eq!(
        absent_selected_audio_transcription_model_key(&dir, &settings),
        Some(format!("{LOCAL_WHISPER_PROVIDER_ID}/no-such-whisper-model")),
        "an unknown model parks its jobs rather than letting them burn their attempts"
    );

    // A blank model id is not a selection.
    settings.model_id = Some("  ".to_string());
    assert_eq!(
        absent_selected_audio_transcription_model_key(&dir, &settings),
        None
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn privacy_settings_open_through_the_host_and_status_reads_the_config_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host = Arc::new(crate::test_support::FakeHost::default());
    let core = crate::test_support::core_in(host.clone(), dir.path());

    open_apple_speech_recognition_privacy_settings(&core).expect("host opens the url");
    let status = get_audio_transcription_model_status(&core).expect("status");

    assert_eq!(
        *host.opened_urls.lock().unwrap(),
        vec![
            "x-apple.systempreferences:com.apple.preference.security?Privacy_SpeechRecognition"
                .to_string()
        ]
    );
    assert_eq!(
        status.models_directory,
        path_to_string(
            &core
                .paths()
                .config
                .join(audio_transcription::MODEL_STORE_DIR_NAME)
        )
    );
}
