use super::download::*;
use super::*;
use capture_types::default_semantic_search_settings;
use semantic_search::{find_model_descriptor, selected_semantic_search_model_available};

mod reconcile;

#[test]
fn status_response_lists_catalog_under_app_data_models_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let response =
        build_semantic_search_model_status_response(temp.path(), None).expect("status response");
    assert!(response
        .models_directory
        .ends_with("semantic_search_models"));
    assert!(response
        .models
        .iter()
        .any(|model| model.model_id == "nomic-embed-text-v1.5"
            && model.tier == SemanticSearchModelTier::English
            && model.dimension == 768));
    // No model installed yet => everything is Missing / unavailable.
    assert!(response.models.iter().all(|model| !model.available));
    // With no selection, exactly the 3 guided manifest tiers are listed.
    assert_eq!(response.models.len(), builtin_model_manifest().models.len());
}

/// The curated `Custom`-tier catalog model, surfaced via the Custom picker
/// (ADR 0037: the open "any model outside the manifest" picker is gone, so the
/// Custom tier is now an explicit catalog entry — `bge-m3`).
fn custom_tier_descriptor() -> SemanticSearchModelDescriptor {
    builtin_model_manifest()
        .models
        .into_iter()
        .find(|model| model.tier == SemanticSearchModelTier::Custom)
        .expect("a Custom-tier catalog model")
}

#[test]
fn status_response_surfaces_the_selected_custom_tier_model() {
    // The curated Custom-tier model (bge-m3) appears in the status response when
    // it is the persisted selection, so the Settings UI can show its
    // installed/selected state. It is an explicit catalog entry now, so it is
    // already in the manifest list (no extra appended row) — the selected-append
    // guard must not duplicate it.
    let custom = custom_tier_descriptor();

    let temp = tempfile::tempdir().expect("tempdir");
    let response = build_semantic_search_model_status_response(temp.path(), Some(&custom.model_id))
        .expect("status response");

    // The Custom-tier model is one of the 3 guided manifest tiers, so the count
    // is unchanged (no appended row for an in-catalog selection).
    assert_eq!(response.models.len(), builtin_model_manifest().models.len());
    let custom_row = response
        .models
        .iter()
        .find(|model| model.model_id == custom.model_id)
        .expect("selected custom-tier model must be listed");
    assert_eq!(custom_row.tier, SemanticSearchModelTier::Custom);
    assert_eq!(custom_row.model_code, custom.hf_repo);
    // Not installed on disk => Missing / unavailable, but still surfaced.
    assert!(!custom_row.available);
}

/// Install a catalog model on disk: every required file from its resolved
/// safetensors layout plus the `.installed.json` marker, so
/// `detect_model_status` reports it Installed.
fn install_custom_model_on_disk(models_dir: &Path, descriptor: &SemanticSearchModelDescriptor) {
    let install_dir = model_install_dir(models_dir, &descriptor.provider, &descriptor.model_id)
        .expect("install dir");
    std::fs::create_dir_all(&install_dir).expect("install dir");
    for file_name in &descriptor.expected_layout.required_files {
        let path = install_dir.join(file_name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent dir");
        }
        std::fs::write(path, b"x").expect("model file");
    }
    write_installed_marker(models_dir, &descriptor.provider, &descriptor.model_id).expect("marker");
}

#[test]
fn status_response_surfaces_a_downloaded_but_unselected_custom_model_as_available() {
    // H3 regression: a Custom-tier model downloaded on disk but NOT yet the
    // persisted selection must appear in the status response with a real
    // `available` true, so the Settings UI can offer "Use this model". Without
    // this it was a dead end — downloadable, never activatable.
    let descriptor = custom_tier_descriptor();

    let temp = tempfile::tempdir().expect("tempdir");
    let models_dir = semantic_search_models_dir(temp.path());
    install_custom_model_on_disk(&models_dir, &descriptor);

    // No selection at all — the model is merely downloaded.
    let response =
        build_semantic_search_model_status_response(temp.path(), None).expect("status response");
    let custom_row = response
        .models
        .iter()
        .find(|model| model.model_id == descriptor.model_id)
        .expect("downloaded custom-tier model must be listed even when unselected");
    assert_eq!(custom_row.tier, SemanticSearchModelTier::Custom);
    assert!(
        custom_row.available,
        "a fully-downloaded custom-tier model must report available so it can be activated"
    );
    // The other guided tiers are unaffected (still listed, all unavailable on disk).
    assert!(response
        .models
        .iter()
        .any(|model| model.model_id == "nomic-embed-text-v1.5" && !model.available));
}

#[test]
fn status_response_does_not_duplicate_a_downloaded_custom_model() {
    // A downloaded Custom-tier model that is ALSO the selection must appear
    // exactly once (the manifest list covers it; neither the disk scan nor the
    // selected-append may re-add it).
    let descriptor = custom_tier_descriptor();

    let temp = tempfile::tempdir().expect("tempdir");
    let models_dir = semantic_search_models_dir(temp.path());
    install_custom_model_on_disk(&models_dir, &descriptor);

    let response =
        build_semantic_search_model_status_response(temp.path(), Some(&descriptor.model_id))
            .expect("status response");
    assert_eq!(
        response
            .models
            .iter()
            .filter(|model| model.model_id == descriptor.model_id)
            .count(),
        1,
        "a downloaded + selected custom-tier model must appear exactly once"
    );
}

#[test]
fn status_response_does_not_duplicate_a_selected_manifest_model() {
    // A guided-tier selection is already in the manifest list and must not be
    // appended a second time.
    let temp = tempfile::tempdir().expect("tempdir");
    let response =
        build_semantic_search_model_status_response(temp.path(), Some("nomic-embed-text-v1.5"))
            .expect("status response");
    assert_eq!(response.models.len(), builtin_model_manifest().models.len());
    assert_eq!(
        response
            .models
            .iter()
            .filter(|model| model.model_id == "nomic-embed-text-v1.5")
            .count(),
        1
    );
}

#[test]
fn gating_wrapper_flips_once_the_selected_model_is_installed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = default_semantic_search_settings();
    assert!(
        !selected_semantic_search_model_available(temp.path(), &settings).expect("gating check")
    );

    // Install the selected model on disk: every required file + marker.
    let models_dir = semantic_search_models_dir(temp.path());
    let manifest = builtin_model_manifest();
    let descriptor = manifest
        .models
        .iter()
        .find(|d| d.model_id == settings.model_id.clone().unwrap())
        .expect("selected descriptor");
    let install_dir = model_install_dir(
        &models_dir,
        SEMANTIC_SEARCH_PROVIDER_ID,
        &descriptor.model_id,
    )
    .expect("install dir");
    std::fs::create_dir_all(&install_dir).expect("install dir");
    for file_name in &descriptor.expected_layout.required_files {
        let path = install_dir.join(file_name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent dir");
        }
        std::fs::write(path, b"x").expect("model file");
    }
    // Write the marker at the CURRENT manifest version. After the candle
    // cutover (MANIFEST_VERSION 2) an older `manifestVersion: 1` ONNX-shaped
    // marker no longer validates — that staleness is what forces the
    // re-download into the safetensors layout — so a fresh install must stamp
    // the live version to be recognized as Installed.
    write_installed_marker(
        &models_dir,
        SEMANTIC_SEARCH_PROVIDER_ID,
        &descriptor.model_id,
    )
    .expect("marker");

    assert!(
        selected_semantic_search_model_available(temp.path(), &settings).expect("gating check")
    );
}

#[test]
fn file_specs_cover_every_required_layout_file() {
    // The download spec list must cover every file the detector requires for that
    // model — the three safetensors-layout files — so a model never installs
    // missing its weights. Both views read `expected_layout.required_files`.
    let manifest = builtin_model_manifest();
    for descriptor in &manifest.models {
        let specs = model_file_specs(descriptor);
        let spec_paths: Vec<&str> = specs.iter().map(|s| s.relative_path.as_str()).collect();
        for required in &descriptor.expected_layout.required_files {
            assert!(
                spec_paths.contains(&required.as_str()),
                "model {} is missing a download spec for required file {required}",
                descriptor.model_id
            );
        }
    }
}

#[test]
fn download_plan_covers_each_models_weights_layout() {
    // ADR 0037: every catalog model downloads its weights file plus the two
    // repo-root JSON files (`config.json`, `tokenizer.json`). The catalog no
    // longer downloads `model.safetensors` for every model — nomic/e5 do, but
    // bge-m3's repo ships no safetensors, so it downloads `pytorch_model.bin`
    // instead (still no ONNX siblings). Each plan must therefore carry ITS OWN
    // weights file. The old ONNX graph + external-data siblings (e.g. bge-m3's
    // `onnx/model.onnx_data`) are gone, so they must NOT appear in the plan.
    let manifest = builtin_model_manifest();
    for descriptor in &manifest.models {
        let specs = model_file_specs(descriptor);
        let paths: Vec<&str> = specs.iter().map(|s| s.relative_path.as_str()).collect();
        assert!(
            paths.contains(&descriptor.expected_layout.weights_relative_path.as_str()),
            "{}",
            descriptor.model_id
        );
        assert!(paths.contains(&"config.json"), "{}", descriptor.model_id);
        assert!(paths.contains(&"tokenizer.json"), "{}", descriptor.model_id);
        assert!(
            paths.iter().all(|p| !p.contains("onnx")),
            "{} must not carry any ONNX file",
            descriptor.model_id
        );
    }
}

#[test]
fn supported_models_list_is_the_curated_catalog() {
    let models = list_semantic_search_supported_models().expect("supported models");
    assert!(
        !models.is_empty(),
        "the curated catalog must enumerate models"
    );
    // The open "any ONNX model" picker is gone (ADR 0037); the list is exactly
    // the curated catalog, so gated/arbitrary repos cannot appear.
    assert!(
        models
            .iter()
            .all(|m| !m.model_code.to_ascii_lowercase().contains("gemma")),
        "gated EmbeddingGemma must never appear in the curated list"
    );
    // e5-small carries its catalog slug, hf_repo, dimension, and multilingual flag.
    let e5 = models
        .iter()
        .find(|m| m.model_id == "multilingual-e5-small")
        .expect("e5-small must be in the curated catalog");
    assert_eq!(e5.model_code, "intfloat/multilingual-e5-small");
    assert!(e5.multilingual);
    assert_eq!(e5.dimension, 384);
}

#[test]
fn hf_urls_point_at_the_model_repo_pinned_revision() {
    // B2: downloads pin the immutable commit SHA (`hf_revision`), not the mutable
    // `main` branch — so an upstream force-push can never swap the bytes.
    let manifest = builtin_model_manifest();
    let nomic = find_model_descriptor(
        &manifest,
        SEMANTIC_SEARCH_PROVIDER_ID,
        "nomic-embed-text-v1.5",
    )
    .expect("nomic descriptor");
    let weights_url = hf_file_url(&nomic.hf_repo, &nomic.hf_revision, "model.safetensors");
    assert_eq!(
        weights_url,
        "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/e9b6763023c676ca8431644204f50c2b100d9aab/model.safetensors"
    );
    // The URL must carry the pinned revision, never the mutable `main` branch.
    assert!(!weights_url.contains("/resolve/main/"));
    let tokenizer_url = hf_file_url(&nomic.hf_repo, &nomic.hf_revision, "tokenizer.json");
    assert!(tokenizer_url.ends_with(&format!("/resolve/{}/tokenizer.json", nomic.hf_revision)));
}

#[test]
fn only_one_download_can_be_claimed_at_a_time() {
    let state = SemanticSearchModelDownloadState::default();
    claim_model_download(
        &state,
        SEMANTIC_SEARCH_PROVIDER_ID,
        "nomic-embed-text-v1.5",
        Arc::new(AtomicBool::new(false)),
        Arc::new(tokio::sync::Notify::new()),
    )
    .expect("first claim");
    let second = claim_model_download(
        &state,
        SEMANTIC_SEARCH_PROVIDER_ID,
        "multilingual-e5-small",
        Arc::new(AtomicBool::new(false)),
        Arc::new(tokio::sync::Notify::new()),
    )
    .expect_err("second claim should fail while one is active");
    assert!(matches!(second, ModelDownloadError::AlreadyRunning { .. }));
    assert_eq!(
        state
            .lock()
            .expect("state")
            .as_ref()
            .expect("active")
            .model_id,
        "nomic-embed-text-v1.5"
    );
}

#[test]
fn status_response_exposes_model_code_and_disk_cost() {
    let temp = tempfile::tempdir().expect("tempdir");
    let response =
        build_semantic_search_model_status_response(temp.path(), None).expect("status response");
    let nomic = response
        .models
        .iter()
        .find(|m| m.model_id == "nomic-embed-text-v1.5")
        .expect("nomic model");
    assert_eq!(nomic.model_code, "nomic-ai/nomic-embed-text-v1.5");
    assert!(nomic.approx_download_bytes > 0);
}

#[test]
fn semantic_search_checksum_mismatch_fails_before_install_completes() {
    // L5: a file whose bytes do not match a pinned SHA256 must fail verification
    // (the install must NOT complete / the marker must not be written). We hash
    // a file on disk against a deliberately-wrong pinned digest and assert the
    // verifier returns ChecksumMismatch.
    let temp = tempfile::tempdir().expect("tempdir");
    let file_path = temp.path().join("model.safetensors");
    std::fs::write(&file_path, b"the wrong bytes").expect("write file");

    // The matching path: hashing then pinning the real digest verifies cleanly.
    let real_sha256 = sha256_of_file(&file_path).expect("sha256");
    verify_file_checksum(&file_path, "model.safetensors", Some(&real_sha256))
        .expect("a matching pinned hash verifies");

    // The mismatch path: an unrelated pinned hash fails the install.
    let error = verify_file_checksum(&file_path, "model.safetensors", Some(&"0".repeat(64)))
        .expect_err("a mismatched pinned hash must fail verification");
    match error {
        ModelDownloadError::ChecksumMismatch {
            relative_path,
            actual,
            ..
        } => {
            assert_eq!(relative_path, "model.safetensors");
            assert_eq!(actual, real_sha256);
        }
        other => panic!("expected ChecksumMismatch, got {other:?}"),
    }
}

#[test]
fn semantic_search_unpinned_file_installs_but_is_flagged_unverified() {
    // L5: a file with NO pinned hash (every Custom pick, and any guided-tier
    // file not yet sourced) is a no-op success — it installs, integrity
    // unverified. `verify_file_checksum(None)` must return Ok without touching
    // the bytes.
    let temp = tempfile::tempdir().expect("tempdir");
    let file_path = temp.path().join("model.safetensors");
    std::fs::write(&file_path, b"unpinned but trusted").expect("write file");

    verify_file_checksum(&file_path, "model.safetensors", None)
        .expect("an unpinned file is accepted (integrity unverified)");
    // An empty pinned string is treated as unpinned too (mirrors speaker-analysis).
    verify_file_checksum(&file_path, "model.safetensors", Some("  "))
        .expect("a blank pinned hash is treated as unpinned");

    // B2: the two guided tiers (nomic, e5) carry a real pinned weights digest —
    // fail-closed. bge-m3 (Custom) ships only `pytorch_model.bin`, and that
    // weights file is now pinned too (fail-closed) — even though the Custom tier
    // is exempt from the MUST-pin gate, bge-m3 happens to satisfy it. The unpinned
    // no-op path above still exists generically for any not-yet-sourced file.
    let manifest = builtin_model_manifest();
    for descriptor in &manifest.models {
        let weights = &descriptor.expected_layout.weights_relative_path;
        let weights_spec = model_file_specs(descriptor)
            .into_iter()
            .find(|spec| &spec.relative_path == weights)
            .expect("a weights file spec");
        match descriptor.tier {
            SemanticSearchModelTier::English | SemanticSearchModelTier::Multilingual => assert!(
                weights_spec.expected_sha256.is_some(),
                "guided-tier {} must pin its weights digest (fail-closed)",
                descriptor.model_id
            ),
            // Custom is EXEMPT from the MUST-pin gate, but bge-m3's
            // `pytorch_model.bin` is in fact pinned, so its digest is present.
            SemanticSearchModelTier::Custom => assert!(
                weights_spec.expected_sha256.is_some(),
                "custom-tier {} pins its `pytorch_model.bin` weights digest (bge-m3 is exempt but happens to be pinned)",
                descriptor.model_id
            ),
        }
    }
}

#[test]
fn pinned_hash_lookup_resolves_when_a_constant_is_present() {
    // Guards the verification plumbing: `pinned_file_sha256` returns the real
    // pinned LFS digest for a known (hf_repo, path) and None for an unpinned one.
    let nomic = pinned_file_sha256("nomic-ai/nomic-embed-text-v1.5", "model.safetensors")
        .expect("nomic weights are pinned (fail-closed)");
    assert_eq!(
        nomic,
        "9e7d262b1fe5ea350782829496efa831901b77486bbde1cea54a4c822d010d5c"
    );
    let e5 = pinned_file_sha256("intfloat/multilingual-e5-small", "model.safetensors")
        .expect("e5 weights are pinned (fail-closed)");
    assert_eq!(
        e5,
        "1a55775f53449dac10a2bcbc312469fac40b96d53198c407081a831f81c98477"
    );
    // bge-m3 (Custom) ships no `model.safetensors` — its weights live in
    // `pytorch_model.bin`, which IS pinned. Looking up the nonexistent
    // `model.safetensors` is what returns `None`.
    assert!(
        pinned_file_sha256("BAAI/bge-m3", "model.safetensors").is_none(),
        "bge-m3 has no `model.safetensors` (its weights are `pytorch_model.bin`)"
    );
    let bge_m3 = pinned_file_sha256("BAAI/bge-m3", "pytorch_model.bin")
        .expect("bge-m3 weights live in `pytorch_model.bin` and are pinned");
    assert_eq!(
        bge_m3,
        "b5e0ce3470abf5ef3831aa1bd5553b486803e83251590ab7ff35a117cf6aad38"
    );
    assert!(
        pinned_file_sha256("some/unknown-model", "model.safetensors").is_none(),
        "an unknown hf_repo (not in the catalog) is never pinned"
    );
}

#[test]
fn download_progress_reaches_the_host_as_a_typed_event() {
    let host = Arc::new(crate::test_support::FakeHost::default());
    let dir = tempfile::tempdir().expect("tempdir");
    let core = crate::test_support::core_in(host.clone(), dir.path());
    let progress = SemanticSearchModelDownloadProgressDto::new(
        SEMANTIC_SEARCH_PROVIDER_ID,
        "nomic-embed-text-v1.5",
        SemanticSearchModelDownloadStatusDto::Starting,
        0,
        Some(1),
        None,
    );

    emit_download_progress(&core, &progress);

    assert_eq!(
        *host.events.lock().unwrap(),
        vec![CoreEvent::SemanticSearchModelDownloadProgress(progress)]
    );
}

#[test]
fn start_refuses_while_the_core_download_slot_is_held() {
    let host = Arc::new(crate::test_support::FakeHost::default());
    let dir = tempfile::tempdir().expect("tempdir");
    let core = Arc::new(crate::test_support::core_in(host.clone(), dir.path()));
    core.manage(SemanticSearchModelDownloadState::default());
    claim_model_download(
        &core.state::<SemanticSearchModelDownloadState>(),
        SEMANTIC_SEARCH_PROVIDER_ID,
        "multilingual-e5-small",
        Arc::new(AtomicBool::new(false)),
        Arc::new(tokio::sync::Notify::new()),
    )
    .expect("first claim");

    let request = SemanticSearchModelDownloadRequestDto {
        provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
        model_id: "nomic-embed-text-v1.5".to_string(),
    };
    assert!(start_semantic_search_model_download(&core, request).is_err());
    assert!(host.events.lock().unwrap().is_empty(), "no Starting event");
    assert_eq!(
        core.state::<SemanticSearchModelDownloadState>()
            .lock()
            .unwrap()
            .as_ref()
            .expect("active")
            .model_id,
        "multilingual-e5-small"
    );
}

#[test]
fn status_and_delete_use_the_models_dir_under_the_config_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let core = crate::test_support::core_in(Default::default(), dir.path());
    let models_dir = semantic_search_models_dir(&core.paths().config);
    let status = get_semantic_search_model_status(&core).expect("status");
    assert_eq!(status.models_directory, models_dir.to_string_lossy());

    let state = SemanticSearchModelDownloadState::default();
    let request = |model_id: &str| SemanticSearchModelDownloadRequestDto {
        provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
        model_id: model_id.to_string(),
    };
    // Nothing managed, so the selection is the default model: it is in use.
    let selected = default_semantic_search_settings()
        .model_id
        .expect("default model");
    let refused = delete_semantic_search_model(&core, request(&selected), &state).unwrap_err();
    assert!(refused.contains("currently in use"), "{refused}");

    let install = model_install_dir(&models_dir, SEMANTIC_SEARCH_PROVIDER_ID, "bge-m3").unwrap();
    std::fs::create_dir_all(&install).unwrap();
    delete_semantic_search_model(&core, request("bge-m3"), &state).expect("delete");
    assert!(!install.exists());
}
