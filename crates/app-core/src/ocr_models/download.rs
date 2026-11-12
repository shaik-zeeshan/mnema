use super::*;
use crate::model_download_stream::{self, next_chunk, NextChunk};

pub(super) fn build_download_plan(
    core: &AppCore,
    request: &OcrModelDownloadRequestDto,
) -> Result<DownloadPlan, ModelDownloadError> {
    let manifest = builtin_model_manifest();
    let descriptor = find_model_descriptor(&manifest, &request.provider, Some(&request.model_id))
        .cloned()
        .ok_or_else(|| ModelDownloadError::ModelNotFound {
            provider: request.provider.clone(),
            model_id: request.model_id.clone(),
        })?;

    let artifact = match &descriptor.management {
        ModelManagement::OsManaged => {
            return Err(ModelDownloadError::OsManaged {
                provider: request.provider.clone(),
                model_id: request.model_id.clone(),
            })
        }
        ModelManagement::AppManaged { artifact: None, .. } => {
            return Err(ModelDownloadError::MissingArtifact {
                provider: request.provider.clone(),
                model_id: request.model_id.clone(),
            })
        }
        ModelManagement::AppManaged {
            artifact: Some(artifact),
            ..
        } => artifact.clone(),
    };

    let models_dir = ocr_models_dir(&core.paths().config);
    let install_dir = model_install_dir(&models_dir, &request.provider, &request.model_id)?;
    let temp_file_path = install_dir.join(".download.tmp");

    Ok(DownloadPlan {
        provider: request.provider.clone(),
        model_id: request.model_id.clone(),
        models_dir,
        install_dir,
        temp_file_path,
        descriptor,
        artifact,
    })
}

pub(super) fn emit_download_progress(core: &AppCore, progress: &OcrModelDownloadProgressDto) {
    core.host()
        .emit(CoreEvent::OcrModelDownloadProgress(progress.clone()));
}

pub(super) async fn run_model_download_task(
    core: Arc<AppCore>,
    plan: DownloadPlan,
    cancel_requested: Arc<AtomicBool>,
) {
    let download_lock = crate::model_download_lock::acquire_model_download_lock(
        &core,
        app_infra::OCR_PROCESSOR,
        model_key(&plan.provider, &plan.model_id),
    )
    .await;
    let result = download_and_install_model(&core, &plan, &cancel_requested).await;
    crate::model_download_lock::release_model_download_lock(&core, download_lock).await;
    clear_active_download(&core, &plan.provider, &plan.model_id);

    match result {
        Ok(()) => {
            let progress = OcrModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                OcrModelDownloadStatusDto::Completed,
                plan.artifact.byte_size,
                Some(plan.artifact.byte_size),
                None,
            );
            emit_download_progress(&core, &progress);
        }
        Err(ModelDownloadTaskError::Cancelled) => {
            let _ = remove_model_file_if_exists(&plan.temp_file_path);
            let _ = remove_model_file_if_exists(
                plan.install_dir.join(ocr::DOWNLOADING_MARKER_FILE_NAME),
            );
            let progress = OcrModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                OcrModelDownloadStatusDto::Cancelled,
                0,
                Some(plan.artifact.byte_size),
                Some("download cancelled".to_string()),
            );
            emit_download_progress(&core, &progress);
        }
        Err(ModelDownloadTaskError::Failed(error)) => {
            let _ = remove_model_file_if_exists(&plan.temp_file_path);
            let _ = remove_model_file_if_exists(
                plan.install_dir.join(ocr::DOWNLOADING_MARKER_FILE_NAME),
            );
            let _ = write_failed_marker(
                &plan.models_dir,
                &plan.provider,
                &plan.model_id,
                error.to_string(),
            );
            let progress = OcrModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                OcrModelDownloadStatusDto::Failed,
                0,
                Some(plan.artifact.byte_size),
                Some(error.to_string()),
            );
            emit_download_progress(&core, &progress);
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

    if let ModelArtifactShape::MultiFile { files } = &plan.artifact.shape {
        download_and_install_multi_file_artifact(core, plan, files, cancel_requested).await?;
        return Ok(());
    }

    download_artifact_to_temp(core, plan, cancel_requested).await?;
    if cancel_requested.load(Ordering::SeqCst) {
        return Err(ModelDownloadTaskError::Cancelled);
    }

    emit_download_progress(
        core,
        &OcrModelDownloadProgressDto::new(
            &plan.provider,
            &plan.model_id,
            OcrModelDownloadStatusDto::Installing,
            plan.artifact.byte_size,
            Some(plan.artifact.byte_size),
            Some("validating checksum".to_string()),
        ),
    );

    // Hashing and unpacking hundreds of MB is blocking work: keep it off the runtime workers.
    let install = plan.clone();
    core.run_blocking(move || {
        validate_artifact_sha256(&install.temp_file_path, &install.artifact.sha256)?;
        install_downloaded_model_artifact(
            &install.models_dir,
            &install.descriptor,
            &install.temp_file_path,
        )?;
        remove_model_file_if_exists(&install.temp_file_path)
    })
    .await
    .map_err(ModelDownloadError::Install)?;

    Ok(())
}

pub(super) fn staged_download_temp_path(
    install_dir: &Path,
    relative_path: &Path,
    index: usize,
) -> PathBuf {
    let label = relative_path.to_string_lossy().replace(['/', '\\'], "__");
    install_dir.join(format!(
        ".download-{}-{}-{}",
        std::process::id(),
        index,
        if label.is_empty() { "file" } else { &label }
    ))
}

pub(super) async fn download_and_install_multi_file_artifact(
    core: &AppCore,
    plan: &DownloadPlan,
    files: &[ModelArtifactFile],
    cancel_requested: &AtomicBool,
) -> Result<(), ModelDownloadTaskError> {
    let total_bytes = plan.artifact.byte_size;
    let mut downloaded_total = 0_u64;
    let mut staged_files = Vec::new();

    for (index, file) in files.iter().enumerate() {
        if cancel_requested.load(Ordering::SeqCst) {
            return Err(ModelDownloadTaskError::Cancelled);
        }
        let relative_path = safe_relative_model_file_path(&file.relative_path)?;
        let temp_path = staged_download_temp_path(&plan.install_dir, &relative_path, index);
        let downloaded = download_model_file_to_temp(
            core,
            plan,
            file,
            &temp_path,
            downloaded_total,
            total_bytes,
            cancel_requested,
        )
        .await?;
        downloaded_total = downloaded_total.saturating_add(downloaded);
        let (hashed_path, sha256) = (temp_path.clone(), file.sha256.clone());
        core.run_blocking(move || validate_artifact_sha256(&hashed_path, &sha256))
            .await
            .map_err(ModelDownloadError::Install)?;
        staged_files.push((temp_path, plan.install_dir.join(relative_path)));
    }

    emit_download_progress(
        core,
        &OcrModelDownloadProgressDto::new(
            &plan.provider,
            &plan.model_id,
            OcrModelDownloadStatusDto::Installing,
            downloaded_total,
            Some(total_bytes),
            Some("installing OCR bundle".to_string()),
        ),
    );

    for (temp_path, destination) in staged_files {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ModelDownloadError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        std::fs::rename(&temp_path, &destination)
            .or_else(|_| {
                std::fs::copy(&temp_path, &destination)?;
                std::fs::remove_file(&temp_path)
            })
            .map_err(|source| ModelDownloadError::WriteFile {
                path: destination,
                source,
            })?;
    }

    let missing = match &plan.descriptor.management {
        ModelManagement::AppManaged {
            expected_layout, ..
        } => expected_layout
            .required_files
            .iter()
            .filter(|file| !plan.install_dir.join(file).is_file())
            .cloned()
            .collect::<Vec<_>>(),
        ModelManagement::OsManaged => Vec::new(),
    };
    if !missing.is_empty() {
        return Err(ModelDownloadError::Install(
            ocr::ModelInstallError::IncompleteInstalledLayout {
                missing_files: missing,
            },
        )
        .into());
    }
    remove_model_file_if_exists(plan.install_dir.join(ocr::DOWNLOADING_MARKER_FILE_NAME))
        .map_err(ModelDownloadError::Install)?;
    ocr::write_installed_marker(&plan.models_dir, &plan.provider, &plan.model_id)
        .map_err(ModelDownloadError::Status)?;
    Ok(())
}

pub(super) async fn download_model_file_to_temp(
    core: &AppCore,
    plan: &DownloadPlan,
    file: &ModelArtifactFile,
    temp_file_path: &Path,
    already_downloaded_bytes: u64,
    total_bytes: u64,
    cancel_requested: &AtomicBool,
) -> Result<u64, ModelDownloadTaskError> {
    let response = model_download_stream::get(&file.url)
        .await
        .map_err(ModelDownloadError::Http)?;
    let mut stream = response.bytes_stream();
    let mut output =
        std::fs::File::create(temp_file_path).map_err(|source| ModelDownloadError::CreateFile {
            path: temp_file_path.to_path_buf(),
            source,
        })?;
    let mut file_downloaded = 0_u64;
    loop {
        let chunk = match next_chunk(&mut stream, cancel_requested).await {
            NextChunk::Chunk(chunk) => chunk.map_err(ModelDownloadError::Http)?,
            NextChunk::End => break,
            NextChunk::Cancelled => return Err(ModelDownloadTaskError::Cancelled),
        };
        std::io::Write::write_all(&mut output, &chunk).map_err(|source| {
            ModelDownloadError::WriteFile {
                path: temp_file_path.to_path_buf(),
                source,
            }
        })?;
        file_downloaded = file_downloaded.saturating_add(chunk.len() as u64);
        emit_download_progress(
            core,
            &OcrModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                OcrModelDownloadStatusDto::Downloading,
                already_downloaded_bytes.saturating_add(file_downloaded),
                Some(total_bytes),
                Some(file.relative_path.clone()),
            ),
        );
    }
    Ok(file_downloaded)
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
        return Err(ModelDownloadError::MissingArtifact {
            provider: "invalid model artifact path".to_string(),
            model_id: relative_path.to_string(),
        });
    }
    Ok(path.to_path_buf())
}

pub(super) async fn download_artifact_to_temp(
    core: &AppCore,
    plan: &DownloadPlan,
    cancel_requested: &AtomicBool,
) -> Result<(), ModelDownloadTaskError> {
    let response = model_download_stream::get(&plan.artifact.url)
        .await
        .map_err(ModelDownloadError::Http)?;
    let total_bytes = response.content_length().or(Some(plan.artifact.byte_size));
    let mut stream = response.bytes_stream();
    let mut file = std::fs::File::create(&plan.temp_file_path).map_err(|source| {
        ModelDownloadError::CreateFile {
            path: plan.temp_file_path.clone(),
            source,
        }
    })?;
    let mut downloaded_bytes = 0_u64;

    loop {
        let chunk = match next_chunk(&mut stream, cancel_requested).await {
            NextChunk::Chunk(chunk) => chunk.map_err(ModelDownloadError::Http)?,
            NextChunk::End => break,
            NextChunk::Cancelled => return Err(ModelDownloadTaskError::Cancelled),
        };
        std::io::Write::write_all(&mut file, &chunk).map_err(|source| {
            ModelDownloadError::WriteFile {
                path: plan.temp_file_path.clone(),
                source,
            }
        })?;
        downloaded_bytes += chunk.len() as u64;
        emit_download_progress(
            core,
            &OcrModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                OcrModelDownloadStatusDto::Downloading,
                downloaded_bytes,
                total_bytes,
                None,
            ),
        );
    }

    Ok(())
}
