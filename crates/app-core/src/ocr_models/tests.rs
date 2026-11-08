use super::*;

fn find_model<'a>(
    response: &'a OcrModelStatusResponseDto,
    provider: &str,
    model_id: Option<&str>,
) -> Option<&'a OcrModelStatusDto> {
    response
        .providers
        .iter()
        .flat_map(|provider_status| provider_status.models.iter())
        .find(|model| model.provider == provider && model.model_id.as_deref() == model_id)
}

#[test]
fn selected_ocr_model_availability_tracks_provider_and_runtime() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = OcrSettings {
        provider: OcrProvider::Tesseract,
        model_id: Some(ocr::DEFAULT_TESSERACT_MODEL_ID.to_string()),
        language: Some(ocr::DEFAULT_TESSERACT_LANGUAGE.to_string()),
        ..capture_types::default_ocr_settings()
    };
    let available = selected_ocr_model_available(temp.path(), &settings)
        .expect("availability should inspect status");
    assert!(!available);
}

#[test]
fn staged_download_temp_paths_are_unique_for_same_file_names() {
    let temp = tempfile::tempdir().expect("tempdir");
    let det = safe_relative_model_file_path("det/model.mnn").expect("det path");
    let rec = safe_relative_model_file_path("rec/model.mnn").expect("rec path");

    let det_temp = staged_download_temp_path(temp.path(), &det, 0);
    let rec_temp = staged_download_temp_path(temp.path(), &rec, 1);

    assert_ne!(det_temp, rec_temp);
}

#[test]
fn status_response_includes_only_desktop_selectable_ocr_providers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let response = build_ocr_model_status_response(temp.path()).expect("status response");
    assert!(find_model(&response, ocr::APPLE_VISION_PROVIDER_ID, None).is_some());
    assert!(find_model(
        &response,
        ocr::TESSERACT_PROVIDER_ID,
        Some(ocr::DEFAULT_TESSERACT_MODEL_ID),
    )
    .is_some());
    assert!(find_model(
        &response,
        ocr::PADDLE_OCR_PROVIDER_ID,
        Some(ocr::DEFAULT_PADDLE_OCR_MODEL_ID),
    )
    .is_none());
}

#[test]
fn delete_unused_ocr_models_preserves_models_referenced_by_processing_jobs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let models_dir = ocr_models_dir(temp.path());
    let install_dir = model_install_dir(
        &models_dir,
        ocr::TESSERACT_PROVIDER_ID,
        ocr::DEFAULT_TESSERACT_MODEL_ID,
    )
    .expect("install dir");
    std::fs::create_dir_all(&install_dir).expect("install dir should be created");
    std::fs::write(install_dir.join("model.bin"), b"model").expect("model file");
    let protected_models = BTreeSet::from([model_key(
        ocr::TESSERACT_PROVIDER_ID,
        ocr::DEFAULT_TESSERACT_MODEL_ID,
    )]);

    let response = delete_unused_ocr_models_inner(
        temp.path(),
        ocr::APPLE_VISION_PROVIDER_ID,
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
        ocr::TESSERACT_PROVIDER_ID
    );
    assert_eq!(
        response.skipped_processing_jobs[0].model_id,
        ocr::DEFAULT_TESSERACT_MODEL_ID
    );
}

#[test]
fn retargetable_ocr_deletion_model_keys_exclude_running_job_models() {
    let deletion_candidates = BTreeSet::from([
        model_key(ocr::TESSERACT_PROVIDER_ID, ocr::DEFAULT_TESSERACT_MODEL_ID),
        model_key(
            ocr::PADDLE_OCR_PROVIDER_ID,
            ocr::DEFAULT_PADDLE_OCR_MODEL_ID,
        ),
    ]);
    let running_job_models = BTreeSet::from([model_key(
        ocr::TESSERACT_PROVIDER_ID,
        ocr::DEFAULT_TESSERACT_MODEL_ID,
    )]);

    let retargetable = retargetable_deletion_model_keys(&deletion_candidates, &running_job_models);

    assert_eq!(
        retargetable,
        BTreeSet::from([model_key(
            ocr::PADDLE_OCR_PROVIDER_ID,
            ocr::DEFAULT_PADDLE_OCR_MODEL_ID,
        )])
    );
}

/// The absent lane's whole value is that the key it locks is the SAME key the claim
/// predicate derives from a job's payload. Every store-level test proves "a locked key
/// parks a job"; this is the other half — that the app computes a key a job can match.
/// The claim SQL builds `TRIM($.provider) || '/' || TRIM($.modelId)` from the payload, so
/// a producer emitting any other shape locks a key nothing carries and the fix is inert
/// with every test still green.
#[test]
fn an_uninstalled_ocr_model_locks_the_key_the_claim_predicate_derives() {
    let dir = std::env::temp_dir().join(format!(
        "mnema-ocr-absent-key-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("test dir should be created");

    let mut settings = crate::recording_settings::default_recording_settings().ocr;
    settings.provider = OcrProvider::Tesseract;
    settings.model_id = Some(ocr::DEFAULT_TESSERACT_MODEL_ID.to_string());

    assert_eq!(
        absent_selected_ocr_model_key(&dir, &settings),
        Some(format!(
            "{}/{}",
            ocr::TESSERACT_PROVIDER_ID,
            ocr::DEFAULT_TESSERACT_MODEL_ID
        )),
        "the absent lock key must be provider/modelId — the exact shape the claim SQL builds"
    );

    // Apple Vision has no model files to be absent, so it must never take a lock:
    // locking it would park every OCR job forever against a model that cannot download.
    settings.provider = OcrProvider::AppleVision;
    settings.model_id = None;
    assert_eq!(absent_selected_ocr_model_key(&dir, &settings), None);

    // A blank model id resolves to the provider's default rather than to "no selection",
    // so it still locks — the job it must park carries that same resolved default.
    settings.provider = OcrProvider::Tesseract;
    settings.model_id = Some("   ".to_string());
    assert_eq!(
        absent_selected_ocr_model_key(&dir, &settings),
        Some(format!(
            "{}/{}",
            ocr::TESSERACT_PROVIDER_ID,
            ocr::DEFAULT_TESSERACT_MODEL_ID
        )),
        "a blank selection resolves to the provider default on both sides, not to None"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn download_progress_reaches_the_host_as_a_typed_event() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host = Arc::new(crate::test_support::FakeHost::default());
    let core = crate::test_support::core_in(host.clone(), dir.path());
    let progress = OcrModelDownloadProgressDto::new(
        ocr::TESSERACT_PROVIDER_ID,
        ocr::DEFAULT_TESSERACT_MODEL_ID,
        OcrModelDownloadStatusDto::Downloading,
        5,
        Some(10),
        None,
    );

    emit_download_progress(&core, &progress);

    assert_eq!(
        *host.events.lock().unwrap(),
        vec![CoreEvent::OcrModelDownloadProgress(progress)]
    );
}

#[test]
fn an_unknown_model_is_refused_before_any_progress_is_emitted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host = Arc::new(crate::test_support::FakeHost::default());
    let core = Arc::new(crate::test_support::core_in(host.clone(), dir.path()));
    core.manage(OcrModelDownloadState::default());

    let result = start_ocr_model_download(
        &core,
        OcrModelDownloadRequestDto {
            provider: ocr::TESSERACT_PROVIDER_ID.to_string(),
            model_id: "no-such-model".to_string(),
        },
    );

    assert!(result.is_err());
    assert!(host.events.lock().unwrap().is_empty());
    assert_eq!(
        cancel_ocr_model_download(&core),
        Err(ModelDownloadError::NoActiveDownload.to_string())
    );
}
