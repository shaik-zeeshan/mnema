use super::*;

/// A shape-version bump (the `vN:` fingerprint tag) must regenerate on the
/// next view even inside the freshness floor: the floor rate-limits capture
/// churn, it must not keep serving prose written in the old shape.
#[tokio::test]
async fn a_shape_version_bump_is_not_held_by_the_freshness_floor() {
    use capture_types::{
        AiEngineRef, AiProviderConfig, AiProviderKind, AiRuntimeSettings, UserContextDigestResult,
    };

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("digest-shape-bump-{unique}"));
    std::fs::create_dir_all(&dir).expect("test dir");
    let infra = app_infra::AppInfra::initialize(&dir)
        .await
        .expect("app infra should initialize");
    let store = infra.user_context();

    // Connected (step 1 passes) but unresolvable (generation fails, no network).
    let provider_id = format!("chatgpt-digest-shape-bump-{unique}");
    crate::secret_vault_test_support::install_shared_test_secret_vault();
    crate::chatgpt_auth::store_token_set(
        &provider_id,
        &crate::chatgpt_auth::ChatgptTokenSet {
            access_token: "expired-access-token".to_string(),
            refresh_token: None,
            expires_at: Some(0),
            rejected: false,
        },
    )
    .expect("seed the expired token set");
    let settings = AiRuntimeSettings {
        enabled: true,
        providers: vec![AiProviderConfig {
            id: provider_id.clone(),
            kind: AiProviderKind::Chatgpt,
            label: String::new(),
            base_url: String::new(),
        }],
        default_model: Some(AiEngineRef {
            provider: provider_id,
            model: "gpt-5.6-terra".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    };

    let range_start_ms = 1_780_876_800_000_i64 + 21 * DAY_MS;
    let range_end_ms = range_start_ms + DAY_MS;
    for (offset, title) in [(0_i64, "Billing migration"), (2 * 3_600_000, "Code review")] {
        store
            .insert_activity_with_evidence(app_infra::NewActivity {
                title: title.to_string(),
                summary: format!("{title} work"),
                category: None,
                focus: None,
                started_at_ms: range_start_ms + offset,
                ended_at_ms: range_start_ms + offset + 1_800_000,
                derivation_run_id: None,
                evidence: Vec::new(),
            })
            .await
            .expect("seed an activity");
    }
    // An old-shape digest generated just now: well inside the day's 1h floor.
    store
        .upsert_digest(
            "day",
            range_start_ms,
            range_end_ms,
            "An old-shape read.",
            None,
            "v3:2:2:0:00000000000000aa",
            now_ms(),
        )
        .await
        .expect("seed an old-shape digest");

    let outcome = get_or_generate_digest(
        &settings,
        true,
        store,
        "day",
        range_start_ms,
        range_end_ms,
        false,
    )
    .await;
    assert!(
        !matches!(
            &outcome,
            Ok(UserContextDigestResult {
                digest: Some(_),
                ..
            })
        ),
        "an old-shape digest must not be served from the floor: {outcome:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
