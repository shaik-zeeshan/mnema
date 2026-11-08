use super::*;

/// B1 regression: startup reconciliation resolves the expected `(model, dimension)`
/// from `model_id` IGNORING `enabled`, so a disabled-but-previously-selected
/// non-default model keeps its table instead of being wiped back to the default.
#[test]
fn reconcile_model_ignores_enabled_and_keys_off_model_id() {
    // A user on the Multilingual tier (e5-small, 384-dim) who toggled the feature
    // OFF: `model_id` is still persisted (disabling never clears it), so the
    // expected model must stay e5 — NOT fall back to the default that would
    // DROP+recreate their vector index.
    let disabled_e5 = capture_types::SemanticSearchSettings {
        enabled: false,
        provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
        model_id: Some("multilingual-e5-small".to_string()),
    };
    assert_eq!(
        reconcile_expected_model(&disabled_e5),
        Some(("multilingual-e5-small".to_string(), 384)),
        "a disabled non-default model must keep its table (B1: never wipe)"
    );

    // Only a genuinely never-selected profile (model_id == None) falls back to
    // the English default tier, which is what a fresh 768-wide DB holds.
    let never_selected = capture_types::SemanticSearchSettings {
        enabled: false,
        provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
        model_id: None,
    };
    assert_eq!(
        reconcile_expected_model(&never_selected),
        Some((
            "nomic-embed-text-v1.5".to_string(),
            DEFAULT_SEMANTIC_SEARCH_DIMENSION
        )),
        "no model ever selected => the English default tier at the migration width"
    );
    assert_eq!(DEFAULT_SEMANTIC_SEARCH_DIMENSION, 768);

    // The `model_id == None` arm resolves against the DEFAULT provider on
    // purpose, never `settings.provider`: a never-selected profile only exists
    // on a hand-edited or partially-deserialized config, where the provider
    // field may be junk. Resolving through `settings.provider` would return
    // `None` here and silently skip reconciliation forever.
    let never_selected_with_junk_provider = capture_types::SemanticSearchSettings {
        enabled: false,
        provider: "not-a-provider".to_string(),
        model_id: None,
    };
    assert_eq!(
        reconcile_expected_model(&never_selected_with_junk_provider),
        Some((
            "nomic-embed-text-v1.5".to_string(),
            DEFAULT_SEMANTIC_SEARCH_DIMENSION
        )),
        "a junk provider with no model ever selected still resolves the default tier"
    );

    // The pairing itself, not the two literals: whatever the English default
    // becomes, its id and its dimension must come from the SAME descriptor.
    // Issue #190 is precisely "change the English default", and pairing the
    // default id with a hardcoded 768 meant the first non-768 default would build
    // a 768 table stamped with a narrower model — the write gate then passes the
    // stamp check and fails the width check on every anchor forever, with no
    // error and no self-heal. This assertion is what makes that unrepresentable.
    let (default_id, default_dimension) =
        reconcile_expected_model(&never_selected).expect("the default tier resolves");
    assert_eq!(
        resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, &default_id)
            .expect("the default id is in the catalog")
            .dimension,
        default_dimension,
        "the default tier's dimension must come from its own descriptor, never a \
         hardcoded constant that can drift away from the id it is paired with"
    );

    // An enabled non-default model resolves the same way (enabled is irrelevant).
    let enabled_bge = capture_types::SemanticSearchSettings {
        enabled: true,
        provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
        model_id: Some("bge-m3".to_string()),
    };
    assert_eq!(
        reconcile_expected_model(&enabled_bge),
        Some(("bge-m3".to_string(), 1024))
    );

    // A same-dimension Custom model resolves to ITS id at the shared width — the
    // pair reconciliation compares, and the reason it can no longer be a bare
    // dimension.
    let gte = capture_types::SemanticSearchSettings {
        enabled: true,
        provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
        model_id: Some("gte-modernbert-base".to_string()),
    };
    assert_eq!(
        reconcile_expected_model(&gte),
        Some((
            "gte-modernbert-base".to_string(),
            DEFAULT_SEMANTIC_SEARCH_DIMENSION
        )),
        "gte-modernbert shares nomic's 768 width, so only the id distinguishes them"
    );

    // FIX 4: a persisted model_id that no longer RESOLVES (catalog/config
    // drift) must return None so the caller SKIPS reconciliation and leaves the
    // populated table untouched — never the default fallback that would DROP it.
    let unresolvable = capture_types::SemanticSearchSettings {
        enabled: true,
        provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
        model_id: Some("a-model-that-no-longer-exists".to_string()),
    };
    assert_eq!(
        reconcile_expected_model(&unresolvable),
        None,
        "an unresolvable model_id must skip reconciliation, not wipe to the default"
    );
}

fn run_async_test(test: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build")
        .block_on(test);
}

fn selection_for(model_id: &str) -> capture_types::SemanticSearchSettings {
    capture_types::SemanticSearchSettings {
        enabled: true,
        provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
        model_id: Some(model_id.to_string()),
    }
}

/// Startup reconciliation must decide from the selection as it stands when it
/// OWNS `SELECT_MODEL_LOCK`, not from a snapshot read before it queued for the
/// lock.
///
/// The pass runs on the deferred-startup seam — after the window opens — so a
/// Settings model switch can be mid-flight (holding the lock across its
/// rebuild+persist) at the exact moment the deferred-startup thread reads the
/// settings and calls in. Deciding from that pre-lock snapshot re-DROPs the table
/// the switch just rebuilt and stamps the OLD model onto it, while the persisted
/// selection names the new one: the model-stamp write gate then skips every batch
/// (a skip, so the sweep idles instead of erroring), and the only other reconciler
/// is the next launch — search silently stays keyword-only for the whole session.
///
/// Same-width models make it invisible to every width check on the way
/// (gte-modernbert is 768, like nomic), so the stamp is the only observable.
#[test]
fn startup_reconciliation_reads_the_selection_after_taking_the_switch_lock() {
    run_async_test(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let infra: crate::AppInfraState = std::sync::Arc::new(
            ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("infra should initialize"),
        );

        // The live persisted selection, as both the Settings switch and the
        // deferred-startup pass observe it.
        let selection = std::sync::Arc::new(std::sync::Mutex::new(selection_for(
            "nomic-embed-text-v1.5",
        )));

        // A model switch is already in flight when deferred startup reaches the
        // reconciler: `select_semantic_search_model` holds this lock across its
        // rebuild+persist pair.
        let switch = select_model_lock().lock().await;

        let infra_for_task = std::sync::Arc::clone(&infra);
        let selection_for_task = std::sync::Arc::clone(&selection);
        let pending = tokio::spawn(async move {
            reconcile_semantic_search_index_on_startup(&infra_for_task, move || {
                selection_for_task.lock().expect("selection").clone()
            })
            .await;
        });
        // Let the pass reach the lock and park behind the switch.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // The switch rebuilds the table for gte-modernbert (the SAME 768 width, so
        // only the stamp distinguishes it), persists the new selection, and
        // releases the lock.
        infra
            .semantic_search()
            .recreate_vectors_table("gte-modernbert-base", 768)
            .await
            .expect("the switch rebuilds the table");
        *selection.lock().expect("selection") = selection_for("gte-modernbert-base");
        drop(switch);

        pending.await.expect("the reconcile task joins");

        assert_eq!(
            infra
                .semantic_search()
                .live_vector_model()
                .await
                .expect("stamp"),
            Some(::app_infra::vectors_index_epoch("gte-modernbert-base")),
            "reconciliation clobbered the just-switched table with a pre-lock \
             settings snapshot: the stamp now names a model the persisted \
             selection does not, so the write gate skips every batch until the \
             next restart"
        );
    });
}

/// B2: the guided-tier fail-closed gate refuses to finalize a guided tier whose
/// weights have no pinned digest, while leaving the Custom tier on the unpinned
/// path.
#[test]
fn guided_tier_gate_requires_a_pinned_weights_digest() {
    fn plan_for(
        hf_repo: &str,
        tier: SemanticSearchModelTier,
        weights_relative_path: &str,
    ) -> DownloadPlan {
        DownloadPlan {
            provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
            model_id: "m".to_string(),
            hf_repo: hf_repo.to_string(),
            hf_revision: "rev".to_string(),
            tier,
            weights_relative_path: weights_relative_path.to_string(),
            aux_weights_relative_path: None,
            install_dir: PathBuf::from("/tmp/x"),
            files: Vec::new(),
            total_bytes: 0,
        }
    }

    // Guided tiers with real pinned weights pass the gate.
    guard_guided_tier_weights_pinned(&plan_for(
        "nomic-ai/nomic-embed-text-v1.5",
        SemanticSearchModelTier::English,
        "model.safetensors",
    ))
    .expect("nomic weights are pinned => gate passes");
    guard_guided_tier_weights_pinned(&plan_for(
        "intfloat/multilingual-e5-small",
        SemanticSearchModelTier::Multilingual,
        "model.safetensors",
    ))
    .expect("e5 weights are pinned => gate passes");

    // A guided tier whose weights are NOT pinned is refused (fail-closed).
    let error = guard_guided_tier_weights_pinned(&plan_for(
        "BAAI/bge-m3",
        SemanticSearchModelTier::English,
        "model.safetensors",
    ))
    .expect_err("an unpinned guided tier must be refused");
    assert!(matches!(
        error,
        ModelDownloadError::GuidedTierWeightsUnpinned { .. }
    ));

    // The Custom tier is exempt: it installs on the unpinned path even though its
    // weights carry no pinned digest.
    guard_guided_tier_weights_pinned(&plan_for(
        "BAAI/bge-m3",
        SemanticSearchModelTier::Custom,
        "model.safetensors",
    ))
    .expect("custom tier is exempt from the fail-closed gate");
}

/// F14: the guided-tier gate requires a pinned digest for EVERY required weights
/// file — both the base backbone AND the auxiliary head when present. A guided
/// tier with an aux head whose head is unpinned must be refused even when the
/// base weights are pinned, so the "guided tiers can never install unverified"
/// contract covers the whole model.
#[test]
fn guided_tier_gate_requires_a_pinned_aux_head_digest() {
    // A synthetic guided tier whose base weights ARE pinned but whose aux head is
    // NOT must be refused (the head would otherwise install unverified).
    let plan_unpinned_head = DownloadPlan {
        provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
        model_id: "m".to_string(),
        // nomic's base `model.safetensors` is pinned, so the base passes; the aux
        // head path below is NOT pinned for this repo, so the gate must fail on it.
        hf_repo: "nomic-ai/nomic-embed-text-v1.5".to_string(),
        hf_revision: "rev".to_string(),
        tier: SemanticSearchModelTier::English,
        weights_relative_path: "model.safetensors".to_string(),
        aux_weights_relative_path: Some("2_Dense_2048/model.safetensors".to_string()),
        install_dir: PathBuf::from("/tmp/x"),
        files: Vec::new(),
        total_bytes: 0,
    };
    let error = guard_guided_tier_weights_pinned(&plan_unpinned_head)
        .expect_err("a guided tier with an unpinned aux head must be refused");
    match error {
        ModelDownloadError::GuidedTierWeightsUnpinned {
            weights_relative_path,
            ..
        } => assert_eq!(weights_relative_path, "2_Dense_2048/model.safetensors"),
        other => panic!("expected GuidedTierWeightsUnpinned for the aux head, got {other:?}"),
    }
}

/// F7: Stella's base + dense head and Arctic's backbone now carry pinned LFS
/// digests, so their multi-GB safetensors install fail-closed instead of on the
/// "integrity UNVERIFIED — trusting TLS only" branch.
#[test]
fn stella_and_arctic_weights_are_pinned() {
    assert_eq!(
        pinned_file_sha256("NovaSearch/stella_en_400M_v5", "model.safetensors")
            .expect("Stella base weights are pinned (F7)"),
        "17e549d16172a548a3115739b55575968eb6523653daad76c46b0758e9425032"
    );
    assert_eq!(
        pinned_file_sha256(
            "NovaSearch/stella_en_400M_v5",
            "2_Dense_2048/model.safetensors"
        )
        .expect("Stella dense head is pinned (F7)"),
        "a831055e5110e81c03ed6559f4ebf5842630f227ded6b6c18826700d548b990f"
    );
    assert_eq!(
        pinned_file_sha256(
            "Snowflake/snowflake-arctic-embed-l-v2.0",
            "model.safetensors"
        )
        .expect("Arctic backbone is pinned (F7)"),
        "21bf1a120b1c6562aeec379dfa9039b0d360591c784cb1c6786e87256b738ee1"
    );

    // Every Custom-tier descriptor that loads safetensors weights must now carry a
    // pinned digest for each weights file (base + aux head), so no Custom model
    // installs its multi-GB weights on the unverified branch.
    let manifest = builtin_model_manifest();
    for descriptor in &manifest.models {
        if descriptor.tier != SemanticSearchModelTier::Custom {
            continue;
        }
        // bge-m3's weights are `pytorch_model.bin` (already pinned); the
        // safetensors-weighted Custom models (Stella, Arctic) must pin every
        // weights file.
        if descriptor
            .expected_layout
            .weights_relative_path
            .ends_with(".safetensors")
        {
            assert!(
                pinned_file_sha256(
                    &descriptor.hf_repo,
                    &descriptor.expected_layout.weights_relative_path
                )
                .is_some(),
                "custom-tier {} base weights must be pinned (F7)",
                descriptor.model_id
            );
            if let Some(aux) = &descriptor.expected_layout.aux_weights_relative_path {
                assert!(
                    pinned_file_sha256(&descriptor.hf_repo, aux).is_some(),
                    "custom-tier {} aux head must be pinned (F7)",
                    descriptor.model_id
                );
            }
        }
    }
}

/// F7: the redirect allowlist accepts HuggingFace hosts and its LFS CDN, and
/// refuses any off-host (or lookalike) redirect target.
#[test]
fn redirect_allowlist_accepts_hf_hosts_and_refuses_others() {
    assert!(is_allowlisted_hf_host("huggingface.co"));
    assert!(is_allowlisted_hf_host("hf.co"));
    assert!(is_allowlisted_hf_host("cdn-lfs.hf.co"));
    assert!(is_allowlisted_hf_host("cdn-lfs-us-1.hf.co"));
    assert!(is_allowlisted_hf_host("CDN-LFS.HF.CO")); // case-insensitive
    assert!(is_allowlisted_hf_host("cas-bridge.xethub.hf.co"));

    // Lookalikes / arbitrary CDNs are refused.
    assert!(!is_allowlisted_hf_host("evilhf.co"));
    assert!(!is_allowlisted_hf_host("huggingface.co.attacker.test"));
    assert!(!is_allowlisted_hf_host("hf.co.evil.test"));
    assert!(!is_allowlisted_hf_host("example.com"));
    assert!(!is_allowlisted_hf_host(""));
}

/// F19: the disk preflight treats the catalog estimate as a lower bound with 10%
/// headroom, so a modest underestimate fails fast rather than mid-write.
#[test]
fn disk_preflight_requires_ten_percent_headroom() {
    assert_eq!(required_free_with_headroom(0), 0);
    assert_eq!(required_free_with_headroom(1_000), 1_100);
    // ~2.27GB bge-m3 estimate gains ~227MB of headroom.
    assert_eq!(
        required_free_with_headroom(2_270_000_000),
        2_270_000_000 + 227_000_000
    );
    // Saturating: an implausibly huge estimate never overflows.
    assert_eq!(required_free_with_headroom(u64::MAX), u64::MAX);
}
