use super::*;
use crate::recording_settings::default_recording_settings;

#[test]
fn privacy_refresh_uses_metadata_collection_when_metadata_is_enabled() {
    let mut settings = default_recording_settings();
    settings.metadata.enabled = true;

    assert_eq!(
        privacy_refresh_mode(&settings, PrivacyRefreshReason::FallbackPoll),
        PrivacyRefreshMode::MetadataAndStaticApps
    );
    assert_eq!(
        privacy_refresh_mode(&settings, PrivacyRefreshReason::WorkspaceFocusChanged),
        PrivacyRefreshMode::MetadataAndStaticApps
    );
    assert!(settings.privacy.excluded_apps.is_empty());
}

#[test]
fn privacy_refresh_keeps_static_fast_path_for_static_rule_mutations() {
    let mut settings = default_recording_settings();
    settings.metadata.enabled = true;

    assert_eq!(
        privacy_refresh_mode(&settings, PrivacyRefreshReason::StaticAppRuleMutation),
        PrivacyRefreshMode::StaticExcludedAppsOnly
    );
}

#[test]
fn privacy_refresh_uses_metadata_collection_for_metadata_settings_mutations() {
    let mut settings = default_recording_settings();
    settings.metadata.enabled = false;

    assert_eq!(
        privacy_refresh_mode(&settings, PrivacyRefreshReason::MetadataSettingsMutation),
        PrivacyRefreshMode::MetadataAndStaticApps
    );
}

#[test]
fn privacy_refresh_keeps_static_fast_path_when_metadata_is_disabled() {
    let mut settings = default_recording_settings();
    settings.metadata.enabled = false;

    assert_eq!(
        privacy_refresh_mode(&settings, PrivacyRefreshReason::FallbackPoll),
        PrivacyRefreshMode::StaticExcludedAppsOnly
    );
    assert_eq!(
        privacy_refresh_mode(&settings, PrivacyRefreshReason::WorkspaceFocusChanged),
        PrivacyRefreshMode::StaticExcludedAppsOnly
    );
}

// A collector spawned for one capture session can still be running its slow
// Gecko AX live read (~1.4s) when the user stops and restarts capture. The
// restart resets the refresh runtime; if the stale collector's write-back
// is then applied, it poisons `last_completed_generation` with the old
// session's (larger) generation, and the new session can no longer start a
// collection until its `requested_generation` climbs back past it — many
// ticks of stale privacy filter / metadata. The completing collector must
// no-op once it is no longer the in-flight generation.
#[test]
fn stale_collector_completion_does_not_suppress_new_session_collections() {
    let mut runtime = PrivacyFilterRefreshRuntime::default();

    // Session 1 runs for a while and starts a collection at generation 50.
    runtime.requested_generation = 50;
    let (gen1, token1, _reason) = runtime
        .begin_collection()
        .expect("session 1 should start a collection");
    assert_eq!(gen1, 50);

    // Stop + restart: the new session resets the refresh runtime while the
    // session-1 collector is still in flight (its slow AX read hasn't
    // returned yet).
    runtime = PrivacyFilterRefreshRuntime::default();

    // The new session requests a refresh (generation climbs from 0 to 1).
    runtime.requested_generation = runtime.requested_generation.saturating_add(1);

    // Now the stale session-1 collector finally finishes and writes back.
    let applied = runtime.complete_collection(
        token1,
        gen1,
        CollectedPrivacyFilterUpdate {
            generation: gen1,
            reason: PrivacyRefreshReason::FallbackPoll,
            mode: PrivacyRefreshMode::MetadataAndStaticApps,
            update: PrivacyFilterUpdate {
                decision: capture_metadata::PrivacyFilterDecision::default(),
                filter: None,
                filter_system_audio: true,
            },
        },
    );
    assert!(
        !applied,
        "a collector from a previous session must not apply after a reset"
    );

    // The new session must still be able to start its collection.
    assert!(
        runtime.begin_collection().is_some(),
        "new session collection suppressed by a stale collector's write-back"
    );
}

// REGRESSION (deep-review finding): the EQUAL-generation collision the
// generation-only guard could not catch. After a reset the runtime is
// replaced with `default()` (generation counter back to 0), so the new
// session's FIRST collection re-claims the SAME generation a still-in-flight
// stale collector holds. Matching the completion on `generation` alone would
// then apply the previous session's data over the live session's in-flight
// collection. The process-global token must reject the stale write while the
// live session's own collector still applies.
#[test]
fn equal_generation_stale_collector_does_not_clobber_fresh_collector_after_reset() {
    let mut runtime = PrivacyFilterRefreshRuntime::default();

    // Session 1's first collection claims generation 1.
    runtime.requested_generation = runtime.requested_generation.saturating_add(1);
    let (stale_gen, stale_token, _reason) = runtime
        .begin_collection()
        .expect("session 1 should start a collection");
    assert_eq!(stale_gen, 1);

    // Stop + restart resets the runtime while the session-1 collector is
    // still running its slow AX read.
    runtime = PrivacyFilterRefreshRuntime::default();

    // Session 2's first collection ALSO claims generation 1 (the runtime
    // started over at 0). Its fresh collector is now in flight too — and it
    // holds a DIFFERENT token from the never-reset global source.
    runtime.requested_generation = runtime.requested_generation.saturating_add(1);
    let (fresh_gen, fresh_token, _reason) = runtime
        .begin_collection()
        .expect("session 2 should start its collection");
    assert_eq!(fresh_gen, 1);
    assert_ne!(
        stale_token, fresh_token,
        "tokens must be unique across a reset even at the same generation"
    );

    // The STALE session-1 collector lands first with generation 1.
    let stale_applied = runtime.complete_collection(
        stale_token,
        stale_gen,
        CollectedPrivacyFilterUpdate {
            generation: stale_gen,
            reason: PrivacyRefreshReason::FallbackPoll,
            mode: PrivacyRefreshMode::MetadataAndStaticApps,
            update: PrivacyFilterUpdate {
                decision: capture_metadata::PrivacyFilterDecision::default(),
                filter: None,
                filter_system_audio: true,
            },
        },
    );
    assert!(
        !stale_applied,
        "a stale collector must not apply after a reset, even when its \
         generation number collides with the new session's"
    );

    // The fresh session-2 collector must still be able to apply its result.
    let fresh_applied = runtime.complete_collection(
        fresh_token,
        fresh_gen,
        CollectedPrivacyFilterUpdate {
            generation: fresh_gen,
            reason: PrivacyRefreshReason::FallbackPoll,
            mode: PrivacyRefreshMode::MetadataAndStaticApps,
            update: PrivacyFilterUpdate {
                decision: capture_metadata::PrivacyFilterDecision::default(),
                filter: None,
                filter_system_audio: true,
            },
        },
    );
    assert!(
        fresh_applied,
        "the live session's own collector must still apply its result"
    );
}

// A privacy rule for an app that is not installed yet carries a display name
// and no bundle id. It must reach NEITHER consumer until it resolves — a
// half-resolved rule that silently records an app the user believes is
// excluded is the failure mode. Both lists derive from
// `decision.excluded_bundle_ids`, so proving `evaluate_privacy` drops the
// pending rule proves it for the screen filter and the tap together.
#[test]
fn a_pending_privacy_rule_reaches_neither_the_screen_filter_nor_the_tap() {
    let settings = capture_types::PrivacySettings {
        excluded_apps: vec![
            capture_metadata::ExcludedAppEntry {
                id: "resolved".to_string(),
                enabled: true,
                bundle_id: "com.example.installed".to_string(),
                display_name: "Installed".to_string(),
            },
            capture_metadata::ExcludedAppEntry {
                id: "pending".to_string(),
                enabled: true,
                bundle_id: String::new(),
                display_name: "Not Installed Yet".to_string(),
            },
        ],
        filter_system_audio: true,
    };

    let decision = capture_metadata::evaluate_privacy(
        &settings,
        &capture_metadata::MetadataContext::default(),
    );
    let only_resolved = vec!["com.example.installed".to_string()];
    assert_eq!(decision.excluded_bundle_ids, only_resolved);
    assert_eq!(decision.matched_rule_ids, vec!["resolved".to_string()]);

    let screen_filter =
        privacy_filter_from_decision(decision.clone()).expect("the resolved rule applies");
    assert_eq!(screen_filter.excluded_bundle_ids, only_resolved);

    let update = PrivacyFilterUpdate {
        decision: decision.clone(),
        filter: None,
        filter_system_audio: true,
    };
    assert_eq!(update.excluded_bundle_ids(), only_resolved.as_slice());
    let initial = InitialPrivacyFilter {
        decision,
        filter: None,
        filter_system_audio: true,
    };
    assert_eq!(initial.excluded_bundle_ids(), only_resolved);
}

#[test]
fn initial_privacy_filter_collects_metadata_snapshot_when_metadata_is_enabled() {
    let mut settings = default_recording_settings();
    settings.metadata.enabled = true;
    let metadata_state = crate::native_capture::metadata::CaptureMetadataState::default();

    let _decision = collect_initial_privacy_filter_decision(&metadata_state, &settings);

    let runtime = metadata_state
        .lock()
        .expect("capture metadata state should lock");
    assert!(runtime.latest_snapshot().is_some());
}

// The one gate for the system-audio exclude list (Settings → Privacy →
// "Filter system audio"): toggle on = privacy ids reach the tap (own-process
// exclusion is added inside capture-system-audio on top); toggle off = no
// privacy ids, so the tap excludes only Mnema's own process.
#[test]
fn system_audio_exclude_list_gates_privacy_ids_on_the_filter_toggle() {
    let privacy_ids = vec!["com.example.secret".to_string()];

    assert_eq!(
        system_audio_privacy_excluded_bundle_ids(true, &privacy_ids),
        privacy_ids.as_slice()
    );
    assert!(system_audio_privacy_excluded_bundle_ids(false, &privacy_ids).is_empty());

    // Both carriers route through the same gate.
    let decision = capture_metadata::PrivacyFilterDecision {
        excluded_bundle_ids: privacy_ids.clone(),
        ..capture_metadata::PrivacyFilterDecision::default()
    };
    let filtered = PrivacyFilterUpdate {
        decision: decision.clone(),
        filter: None,
        filter_system_audio: true,
    };
    assert_eq!(filtered.excluded_bundle_ids(), privacy_ids.as_slice());
    let unfiltered = PrivacyFilterUpdate {
        decision: decision.clone(),
        filter: None,
        filter_system_audio: false,
    };
    assert!(unfiltered.excluded_bundle_ids().is_empty());

    let initial_filtered = InitialPrivacyFilter {
        decision: decision.clone(),
        filter: None,
        filter_system_audio: true,
    };
    assert_eq!(initial_filtered.excluded_bundle_ids(), privacy_ids);
    let initial_unfiltered = InitialPrivacyFilter {
        decision,
        filter: None,
        filter_system_audio: false,
    };
    assert!(initial_unfiltered.excluded_bundle_ids().is_empty());
}

// The refresh state is AppCore's now: a completed collection is taken through
// the core once, a reset clears it, and a core without the state takes nothing.
#[test]
fn completed_collection_is_taken_through_the_core_managed_refresh_state() {
    let dir = tempfile::tempdir().expect("temp dir");
    let core = crate::test_support::core_in(Default::default(), dir.path());
    assert!(take_completed_privacy_filter_update(&core).is_none());

    core.manage(PrivacyFilterRefreshState::default());
    let complete_one = || {
        let state = core.state::<PrivacyFilterRefreshState>();
        let mut state = state.lock().expect("refresh state should lock");
        state.requested_generation += 1;
        let (generation, token, reason) = state.begin_collection().expect("a collection");
        assert!(state.complete_collection(
            token,
            generation,
            CollectedPrivacyFilterUpdate {
                generation,
                reason,
                mode: PrivacyRefreshMode::StaticExcludedAppsOnly,
                update: PrivacyFilterUpdate {
                    decision: capture_metadata::PrivacyFilterDecision::default(),
                    filter: None,
                    filter_system_audio: false,
                },
            },
        ));
    };

    complete_one();
    assert!(take_completed_privacy_filter_update(&core).is_some());
    assert!(take_completed_privacy_filter_update(&core).is_none());

    complete_one();
    reset_privacy_filter_refresh_state(&core);
    assert!(take_completed_privacy_filter_update(&core).is_none());
}
