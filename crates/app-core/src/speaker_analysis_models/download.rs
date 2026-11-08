use super::*;

pub(super) fn build_download_plan(
    core: &AppCore,
    request: &SpeakerAnalysisModelDownloadRequestDto,
) -> Result<DownloadPlan, ModelDownloadError> {
    let manifest = builtin_model_manifest();
    let descriptor = find_model_descriptor(&manifest, &request.provider, Some(&request.model_id))
        .cloned()
        .ok_or_else(|| ModelDownloadError::ModelNotFound {
            provider: request.provider.clone(),
            model_id: request.model_id.clone(),
        })?;
    let ModelManagement::AppManaged { artifact, .. } = &descriptor.management;
    let artifact = artifact
        .clone()
        .ok_or_else(|| ModelDownloadError::MissingArtifact {
            provider: request.provider.clone(),
            model_id: request.model_id.clone(),
        })?;
    let ModelArtifactShape::MultiFile { files } = artifact.shape;
    let models_dir = speaker_analysis_models_dir(&core.paths().config);
    let install_dir = model_install_dir(&models_dir, &descriptor)?;
    let total_bytes = files.iter().map(|file| file.byte_size).sum();
    Ok(DownloadPlan {
        provider: request.provider.clone(),
        model_id: request.model_id.clone(),
        models_dir,
        install_dir,
        descriptor,
        files,
        total_bytes,
    })
}

pub(super) fn emit_download_progress(
    core: &AppCore,
    progress: &SpeakerAnalysisModelDownloadProgressDto,
) {
    core.host()
        .emit(CoreEvent::SpeakerAnalysisModelDownloadProgress(
            progress.clone(),
        ));
}

pub(super) async fn run_model_download_task(
    core: Arc<AppCore>,
    plan: DownloadPlan,
    cancel_requested: Arc<AtomicBool>,
) {
    let download_lock = crate::model_download_lock::acquire_model_download_lock(
        &core,
        app_infra::SPEAKER_ANALYSIS_PROCESSOR,
        model_key(&plan.provider, &plan.model_id),
    )
    .await;
    let result = download_and_install_model(&core, &plan, &cancel_requested).await;
    crate::model_download_lock::release_model_download_lock(&core, download_lock).await;
    clear_active_download(&core, &plan.provider, &plan.model_id);
    match result {
        Ok(()) => emit_download_progress(
            &core,
            &SpeakerAnalysisModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                SpeakerAnalysisModelDownloadStatusDto::Completed,
                plan.total_bytes,
                Some(plan.total_bytes),
                None,
            ),
        ),
        Err(ModelDownloadTaskError::Cancelled) => {
            let _ = remove_model_file_if_exists(
                plan.install_dir
                    .join(speaker_analysis::DOWNLOADING_MARKER_FILE_NAME),
            );
            emit_download_progress(
                &core,
                &SpeakerAnalysisModelDownloadProgressDto::new(
                    &plan.provider,
                    &plan.model_id,
                    SpeakerAnalysisModelDownloadStatusDto::Cancelled,
                    0,
                    Some(plan.total_bytes),
                    Some("download cancelled".to_string()),
                ),
            );
        }
        Err(ModelDownloadTaskError::Failed(error)) => {
            let _ = remove_model_file_if_exists(
                plan.install_dir
                    .join(speaker_analysis::DOWNLOADING_MARKER_FILE_NAME),
            );
            let _ = write_failed_marker(
                &plan.models_dir,
                &plan.provider,
                &plan.model_id,
                error.to_string(),
            );
            emit_download_progress(
                &core,
                &SpeakerAnalysisModelDownloadProgressDto::new(
                    &plan.provider,
                    &plan.model_id,
                    SpeakerAnalysisModelDownloadStatusDto::Failed,
                    0,
                    Some(plan.total_bytes),
                    Some(error.to_string()),
                ),
            );
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ModelDownloadTaskError {
    #[error("download cancelled")]
    Cancelled,
    #[error(transparent)]
    Failed(#[from] ModelDownloadError),
}

pub(super) async fn download_and_install_model(
    core: &AppCore,
    plan: &DownloadPlan,
    cancel_requested: &AtomicBool,
) -> Result<(), ModelDownloadTaskError> {
    remove_model_dir_if_exists(&plan.install_dir).map_err(ModelDownloadError::Install)?;
    std::fs::create_dir_all(&plan.install_dir).map_err(|source| ModelDownloadError::CreateDir {
        path: plan.install_dir.clone(),
        source,
    })?;
    write_downloading_marker(&plan.models_dir, &plan.provider, &plan.model_id)
        .map_err(ModelDownloadError::Status)?;

    let mut downloaded_total = 0_u64;
    for file in &plan.files {
        if cancel_requested.load(Ordering::SeqCst) {
            return Err(ModelDownloadTaskError::Cancelled);
        }
        let bytes =
            download_model_file(core, plan, file, downloaded_total, cancel_requested).await?;
        downloaded_total = downloaded_total.saturating_add(bytes.len() as u64);
        let destination = safe_relative_model_file_path(&file.relative_path)?;
        install_downloaded_model_file(&plan.install_dir, file, &destination, &bytes)?;
    }

    emit_download_progress(
        core,
        &SpeakerAnalysisModelDownloadProgressDto::new(
            &plan.provider,
            &plan.model_id,
            SpeakerAnalysisModelDownloadStatusDto::Installing,
            downloaded_total,
            Some(plan.total_bytes),
            Some("validating speaker analysis model layout".to_string()),
        ),
    );
    validate_installed_layout(plan)?;
    remove_model_file_if_exists(
        plan.install_dir
            .join(speaker_analysis::DOWNLOADING_MARKER_FILE_NAME),
    )
    .map_err(ModelDownloadError::Install)?;
    write_installed_marker(&plan.models_dir, &plan.provider, &plan.model_id)
        .map_err(ModelDownloadError::Status)?;
    Ok(())
}

pub(super) async fn download_model_file(
    core: &AppCore,
    plan: &DownloadPlan,
    file: &ModelArtifactFile,
    already_downloaded_bytes: u64,
    cancel_requested: &AtomicBool,
) -> Result<Vec<u8>, ModelDownloadTaskError> {
    // Each artifact file is content-addressed by its manifest `byte_size`, so a
    // correct body is exactly that many bytes. Cap the accepted body at
    // `byte_size + slack` (a small fixed margin) so a misbehaving or compromised
    // host serving an oversized body fails fast with a typed error instead of
    // buffering an unbounded stream into memory and OOMing before the SHA256
    // check ever runs.
    const DOWNLOAD_SIZE_SLACK_BYTES: u64 = 64 * 1024;
    let size_limit = file.byte_size.saturating_add(DOWNLOAD_SIZE_SLACK_BYTES);

    let response = reqwest::get(&file.url)
        .await
        .map_err(ModelDownloadError::Http)?
        .error_for_status()
        .map_err(ModelDownloadError::Http)?;
    // Reject up front when the server advertises a body larger than we will ever
    // accept, so a hostile `Content-Length` never gets the chance to stream.
    if let Some(content_length) = response.content_length() {
        if content_length > size_limit {
            return Err(ModelDownloadError::OversizedDownload {
                relative_path: file.relative_path.clone(),
                byte_size: file.byte_size,
                limit: size_limit,
            }
            .into());
        }
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        if cancel_requested.load(Ordering::SeqCst) {
            return Err(ModelDownloadTaskError::Cancelled);
        }
        let chunk = chunk.map_err(ModelDownloadError::Http)?;
        // Enforce the running-total cap before retaining the chunk so we never
        // grow `bytes` past the limit even if `Content-Length` was absent or lied.
        if bytes.len() as u64 + chunk.len() as u64 > size_limit {
            return Err(ModelDownloadError::OversizedDownload {
                relative_path: file.relative_path.clone(),
                byte_size: file.byte_size,
                limit: size_limit,
            }
            .into());
        }
        bytes.extend_from_slice(&chunk);
        emit_download_progress(
            core,
            &SpeakerAnalysisModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                SpeakerAnalysisModelDownloadStatusDto::Downloading,
                already_downloaded_bytes.saturating_add(bytes.len() as u64),
                if plan.total_bytes == 0 {
                    None
                } else {
                    Some(plan.total_bytes)
                },
                Some(file.relative_path.clone()),
            ),
        );
    }
    Ok(bytes)
}

pub(super) fn install_downloaded_model_file(
    install_dir: &Path,
    file: &ModelArtifactFile,
    destination_relative_path: &Path,
    bytes: &[u8],
) -> Result<(), ModelDownloadError> {
    let temp_path = install_dir.join(".download.tmp");
    install_model_file(&temp_path, bytes).map_err(ModelDownloadError::Install)?;
    // `validate_artifact_sha256` treats a missing/blank checksum as "nothing to
    // verify" and returns `Ok(())`. For an app-managed download that would mean
    // installing unverified bytes, so refuse here before relying on it: every
    // downloaded artifact MUST carry an integrity checksum.
    if file
        .sha256
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        // Best-effort: drop the staged temp file before aborting so a refused
        // install does not leave `.download.tmp` behind for the next attempt to
        // clear. A failure to clean up must never mask the refusal itself.
        let _ = remove_model_file_if_exists(&temp_path);
        return Err(ModelDownloadError::MissingChecksum {
            relative_path: file.relative_path.clone(),
        });
    }
    if let Err(error) = validate_artifact_sha256(&temp_path, file.sha256.as_deref()) {
        // Best-effort cleanup of the staged temp file on a checksum mismatch,
        // leaving the install dir as it was found. A failure to clean up must
        // never mask the checksum-mismatch error we are surfacing.
        let _ = remove_model_file_if_exists(&temp_path);
        return Err(ModelDownloadError::Install(error));
    }
    let install_result = if file.url.ends_with(".tar.bz2") {
        install_from_tar_bz2(install_dir, destination_relative_path, bytes)
    } else {
        install_model_file(install_dir.join(destination_relative_path), bytes)
            .map_err(ModelDownloadError::Install)
    };
    if install_result.is_err() {
        // A post-stage install failure leaves the temp file behind; remove it
        // best-effort before propagating the error. A failure to clean up must
        // never mask the original install failure.
        let _ = remove_model_file_if_exists(&temp_path);
    }
    install_result?;
    remove_model_file_if_exists(temp_path).map_err(ModelDownloadError::Install)?;
    Ok(())
}

pub(super) fn install_from_tar_bz2(
    install_dir: &Path,
    destination_relative_path: &Path,
    bytes: &[u8],
) -> Result<(), ModelDownloadError> {
    let decoder = bzip2::read::BzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries().map_err(ModelDownloadError::Archive)?;
    for entry in entries {
        let mut entry = entry.map_err(ModelDownloadError::Archive)?;
        let entry_path = entry.path().map_err(ModelDownloadError::Archive)?;
        if !entry_path.ends_with("model.onnx") {
            continue;
        }
        let destination = install_dir.join(destination_relative_path);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ModelDownloadError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        entry
            .unpack(&destination)
            .map_err(ModelDownloadError::Archive)?;
        return Ok(());
    }
    Err(ModelDownloadError::InvalidArtifactPath(
        destination_relative_path.display().to_string(),
    ))
}

pub(super) fn validate_installed_layout(plan: &DownloadPlan) -> Result<(), ModelDownloadError> {
    let ModelManagement::AppManaged {
        expected_layout, ..
    } = &plan.descriptor.management;
    let missing = expected_layout
        .required_files
        .iter()
        .filter(|file| !plan.install_dir.join(file).is_file())
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(
            speaker_analysis::ModelInstallError::IncompleteInstalledLayout {
                missing_files: missing,
            }
            .into(),
        );
    }
    Ok(())
}

pub(super) fn safe_relative_model_file_path(
    relative_path: &str,
) -> Result<PathBuf, ModelDownloadError> {
    let path = Path::new(relative_path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(ModelDownloadError::InvalidArtifactPath(
            relative_path.to_string(),
        ));
    }
    Ok(path.to_path_buf())
}
