//! Live engine-resolve failures and the failure cooldown.

use super::*;

/// A forced re-read that cannot resolve an engine must SAY so.
///
/// `regenerate_user_context_digest` is the Overview's explicit re-read
/// button, and its contract is that a `too_few` absence means the range genuinely
/// has nothing to read — the Overview renders exactly one sentence for it:
/// "Not enough activity in this range to write a read."
///
/// Since the engine resolve went LIVE (it now refreshes a ChatGPT OAuth
/// token), this call can fail for a credential/connectivity reason on a
/// range that is full of Activities. Collapsing that into `too_few` tells
/// the user their data is thin when the truth is the engine never
/// resolved — and records nothing in the derivation-run ledger either, so
/// the failure is invisible on every surface.
#[tokio::test]
async fn a_forced_reread_surfaces_a_live_engine_failure_instead_of_reading_as_no_activity() {
    live_resolve_failure_surfaces(true).await;
}

/// OV-06: the lazy path must not read an engine failure as "not enough
/// activity" either. A sign-in failure is NOT cooled down: the second visit
/// re-resolves (no network), so signing back in takes effect at once.
#[tokio::test]
async fn a_lazy_read_surfaces_a_sign_in_failure_without_a_cooldown() {
    live_resolve_failure_surfaces(false).await;
}

async fn live_resolve_failure_surfaces(force: bool) {
    use capture_types::{AiEngineRef, AiProviderConfig, AiProviderKind};

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("digest-live-resolve-{unique}"));
    std::fs::create_dir_all(&dir).expect("test dir");
    let infra = app_infra::AppInfra::initialize(&dir)
        .await
        .expect("app infra should initialize");
    let store = infra.user_context();

    // A ChatGPT provider that IS connected (so the no-network prerequisite
    // at step 1 passes) but whose access token is long expired with no
    // refresh token to spend — the live resolver's terminal failure, with
    // no network involved.
    let provider_id = format!("chatgpt-digest-live-resolve-{unique}");
    crate::test_support::install_shared_test_secret_vault();
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
            provider: provider_id.clone(),
            model: "gpt-5.6-terra".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    };

    // A range with plenty to read: two ordinary, non-sensitive Activities.
    // A UTC midnight; the two callers use different days so the shared
    // in-memory cooldown can't couple them.
    let range_start_ms = 1_780_876_800_000_i64 + if force { 0 } else { 7 * DAY_MS };
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

    let outcome = get_or_generate_digest(
        &settings,
        true,
        store,
        "day",
        range_start_ms,
        range_end_ms,
        force,
    )
    .await;

    match &outcome {
        Err(message) => assert!(
            !message.trim().is_empty(),
            "the failure must carry a reason the user can act on"
        ),
        other => panic!(
            "a forced re-read whose engine never resolved must not read as \
             'not enough activity in this range': {other:?}"
        ),
    }

    // …and it is on the record: the run ledger is where a failed engine
    // pass is accounted for (the extraction-failure arm already writes one).
    let runs = store
        .list_derivation_runs(10)
        .await
        .expect("read the run ledger");
    assert!(
        runs.iter()
            .any(|run| run.kind == "digest" && run.status == "failed"),
        "a failed forced re-read must leave a failed digest run: {runs:?}"
    );

    if !force {
        let again = get_or_generate_digest(
            &settings,
            true,
            store,
            "day",
            range_start_ms,
            range_end_ms,
            false,
        )
        .await;
        assert_eq!(again, outcome, "the lazy retry reports the same failure");
        let failed_runs = store
            .list_derivation_runs(10)
            .await
            .expect("read the run ledger")
            .into_iter()
            .filter(|run| run.kind == "digest" && run.status == "failed")
            .count();
        assert_eq!(
            failed_runs, 2,
            "a sign-in failure re-resolves instead of replaying"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failure_cooldown_hits_within_the_window_and_misses_after() {
    let start = 9_000_001;
    let _: Result<(), _> = cooldown_fail("day", start, "fp", 1_000, "boom".to_string());
    assert_eq!(
        cooldown_replay("day", start, "fp", false, 1_000 + FAILURE_COOLDOWN_MS - 1),
        Some("boom".to_string())
    );
    assert_eq!(
        cooldown_replay("day", start, "fp", false, 1_000 + FAILURE_COOLDOWN_MS),
        None
    );
    // An expired entry is gone for good.
    assert_eq!(cooldown_replay("day", start, "fp", false, 1_001), None);
}

#[test]
fn failure_cooldown_is_bypassed_by_reread_and_cleared_by_a_new_fingerprint() {
    let start = 9_000_002;
    let _: Result<(), _> = cooldown_fail("week", start, "fp", 1_000, "boom".to_string());
    assert_eq!(
        cooldown_replay("week", start, "fp", true, 1_001),
        None,
        "re-read bypasses"
    );
    assert!(
        cooldown_replay("week", start, "fp", false, 1_001).is_some(),
        "bypass keeps it"
    );
    assert_eq!(cooldown_replay("week", start, "fp2", false, 1_001), None);
    assert_eq!(
        cooldown_replay("week", start, "fp", false, 1_001),
        None,
        "a new fingerprint clears the entry"
    );
}

#[test]
fn the_sensitive_hold_is_a_distinct_outcome_the_cooldown_replays() {
    let start = 9_000_003;
    let held: Result<Option<UserContextDigest>, String> = cooldown_fail(
        "month",
        start,
        "fp",
        1_000,
        DIGEST_SENSITIVE_HOLD.to_string(),
    );
    assert_eq!(held, Err(DIGEST_SENSITIVE_HOLD.to_string()));
    assert_eq!(
        cooldown_replay("month", start, "fp", false, 1_001).as_deref(),
        Some(DIGEST_SENSITIVE_HOLD)
    );
    cooldown_clear("month", start);
    assert_eq!(cooldown_replay("month", start, "fp", false, 1_001), None);
}
