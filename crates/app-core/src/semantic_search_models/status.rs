use super::*;

/// Build one status DTO for a descriptor by detecting its on-disk state.
pub(super) fn status_dto_for(
    models_dir: &Path,
    descriptor: &SemanticSearchModelDescriptor,
) -> Result<SemanticSearchModelStatusDto, ModelStatusError> {
    let status = detect_model_status(models_dir, descriptor)?;
    Ok(SemanticSearchModelStatusDto {
        provider: descriptor.provider.clone(),
        model_id: descriptor.model_id.clone(),
        display_name: descriptor.display_name.clone(),
        description: descriptor.description.clone(),
        tier: descriptor.tier,
        dimension: descriptor.dimension,
        max_tokens: descriptor.max_tokens,
        model_code: descriptor.hf_repo.clone(),
        approx_download_bytes: descriptor.approx_download_bytes,
        license_label: descriptor.license_label.clone(),
        status: status.status,
        available: status.is_available(),
        install_path: status.install_path.to_string_lossy().into_owned(),
        missing_files: status.missing_files,
    })
}

/// Report the 3 guided manifest tiers, plus every **Custom** model that is
/// already downloaded on disk, plus the currently-selected Custom model.
///
/// `selected_model_id` is the persisted `RecordingSettings.semantic_search`
/// selection (always the `local` provider namespace in v1).
///
/// A Custom model (a curated catalog model surfaced via the Custom picker, e.g.
/// `bge-m3`) must be able to be **activated after it is downloaded**. The Settings
/// UI's "Use this model"
/// action only appears once the picked model's status row reports `available`, and
/// the picked-model view only carries a real `available` when the model is present
/// in this response (the catalog fallback is structurally unavailable). So a
/// Custom model that was downloaded but not yet selected MUST appear here with its
/// real on-disk status — otherwise it is a dead end (downloadable, never
/// activatable). We enumerate the on-disk install dirs and append any installed
/// Custom model not already listed; the selected model is appended last even if it
/// is not yet installed, so its "Not installed → Download" state stays visible.
/// Unresolvable ids (legacy, no longer enumerated) are simply omitted, never an
/// error.
pub(super) fn build_semantic_search_model_status_response(
    app_data_dir: &Path,
    selected_model_id: Option<&str>,
) -> Result<SemanticSearchModelStatusResponseDto, ModelStatusError> {
    let models_dir = semantic_search_models_dir(app_data_dir);
    let manifest = builtin_model_manifest();
    let mut models = Vec::with_capacity(manifest.models.len() + 1);

    for descriptor in &manifest.models {
        models.push(status_dto_for(&models_dir, descriptor)?);
    }

    // Append every Custom model already downloaded on disk (not just the selected
    // one), so a downloaded-but-unselected Custom pick carries a real `available`
    // and the UI's "Use this model" activation becomes reachable.
    for model_id in installed_custom_model_ids(&models_dir) {
        if models.iter().any(|model| model.model_id == model_id) {
            continue;
        }
        if let Some(descriptor) = resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, &model_id) {
            models.push(status_dto_for(&models_dir, &descriptor)?);
        }
    }

    // Append the selected Custom model even when it is NOT yet installed, so its
    // Download state is visible. (An installed selection was already covered by the
    // disk scan above; this `already_listed` guard prevents a duplicate.)
    if let Some(model_id) = selected_model_id {
        let already_listed = models.iter().any(|model| model.model_id == model_id);
        if !already_listed {
            if let Some(descriptor) = resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, model_id) {
                models.push(status_dto_for(&models_dir, &descriptor)?);
            }
        }
    }

    Ok(SemanticSearchModelStatusResponseDto {
        models_directory: models_dir.to_string_lossy().into_owned(),
        models,
    })
}

/// Enumerate the `model_id`s of every **local-provider** model directory that
/// exists on disk under `semantic_search_models/local/`. This is a cheap
/// directory listing (no per-model status detection yet): each id is a candidate
/// Custom model the caller resolves + status-checks. A model that has a directory
/// but is incomplete (no marker / missing files) still surfaces — its
/// `detect_model_status` will report it Missing, which is correct (it shows as
/// "Not installed" rather than vanishing). Returns an empty list when the install
/// directory does not exist or cannot be read (a fresh profile), never an error.
pub(super) fn installed_custom_model_ids(models_dir: &Path) -> Vec<String> {
    let provider_dir = models_dir.join(SEMANTIC_SEARCH_PROVIDER_ID);
    let Ok(entries) = std::fs::read_dir(&provider_dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
}
