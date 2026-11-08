use super::*;
use capture_metadata::BrowserUrlMode;

#[test]
fn initial_privacy_decision_includes_static_apps_when_metadata_is_disabled() {
    let metadata = MetadataSettings {
        enabled: false,
        browser_url_mode: BrowserUrlMode::Sanitized,
    };
    let privacy = PrivacySettings {
        excluded_apps: vec![capture_metadata::ExcludedAppEntry {
            id: "app".to_string(),
            enabled: true,
            bundle_id: "com.secret".to_string(),
            display_name: "Secret".to_string(),
        }],
        ..PrivacySettings::default()
    };

    let state = CaptureMetadataState::default();
    let decision = refresh_metadata_state(&state, &metadata, &privacy, BrowserUrlReadMode::Live);
    let runtime = state.lock().expect("capture metadata state should lock");

    assert_eq!(decision.excluded_bundle_ids, vec!["com.secret"]);
    assert!(decision.metadata_redaction_reason.is_none());
    assert_eq!(runtime.latest_decision, decision);
    assert!(runtime.latest_snapshot().is_none());
}

#[test]
fn refresh_with_metadata_disabled_keeps_static_privacy_without_snapshot() {
    let state = CaptureMetadataState::default();
    let metadata = MetadataSettings {
        enabled: false,
        browser_url_mode: BrowserUrlMode::Sanitized,
    };
    let privacy = PrivacySettings {
        excluded_apps: vec![capture_metadata::ExcludedAppEntry {
            id: "app-rule".to_string(),
            enabled: true,
            bundle_id: "com.example.Secret".to_string(),
            display_name: "Secret".to_string(),
        }],
        ..PrivacySettings::default()
    };

    let decision = refresh_metadata_state(&state, &metadata, &privacy, BrowserUrlReadMode::Live);
    let runtime = state.lock().expect("capture metadata state should lock");

    assert_eq!(decision.excluded_bundle_ids, vec!["com.example.Secret"]);
    assert!(decision.metadata_redaction_reason.is_none());
    assert!(runtime.latest_snapshot().is_none());
    assert_eq!(runtime.latest_decision, decision);
}

fn snapshot_named(app: &str) -> FrameMetadataSnapshot {
    FrameMetadataSnapshot {
        app_name: Some(app.to_string()),
        ..FrameMetadataSnapshot::default()
    }
}

#[test]
fn snapshot_in_effect_at_stamps_the_app_frontmost_at_capture_time() {
    let mut runtime = CaptureMetadataRuntime::default();
    runtime.publish_snapshot(1000, Some(snapshot_named("Hitch")));
    runtime.publish_snapshot(2000, Some(snapshot_named("mnema")));

    // Frame captured before the switch (t=1900) must keep Hitch, even though
    // "mnema" is now the latest snapshot.
    assert_eq!(
        runtime
            .snapshot_in_effect_at(1900)
            .unwrap()
            .app_name
            .unwrap(),
        "Hitch"
    );
    // Frame captured after the switch gets mnema.
    assert_eq!(
        runtime
            .snapshot_in_effect_at(2100)
            .unwrap()
            .app_name
            .unwrap(),
        "mnema"
    );
    // Frame captured before any snapshot was published gets None (session start).
    assert!(runtime.snapshot_in_effect_at(500).is_none());
}

#[test]
fn snapshot_history_is_bounded_and_reset_clears_it() {
    let state = CaptureMetadataState::default();
    {
        let mut runtime = state.lock().expect("lock");
        for t in 0..(SNAPSHOT_HISTORY_CAP as u64 + 5) {
            runtime.publish_snapshot(t, Some(snapshot_named("app")));
        }
        assert_eq!(runtime.snapshot_history.len(), SNAPSHOT_HISTORY_CAP);
    }

    reset_recording_session_privacy_state(&state);

    let runtime = state.lock().expect("lock");
    assert!(runtime.snapshot_history.is_empty());
    assert!(runtime.snapshot_in_effect_at(u64::MAX).is_none());
}

#[test]
fn reset_recording_session_privacy_state_clears_verified_windows_and_website_holds() {
    let state = CaptureMetadataState::default();
    {
        let mut runtime = state.lock().expect("capture metadata state should lock");
        runtime.publish_snapshot(
            1000,
            Some(FrameMetadataSnapshot {
                app_bundle_id: Some("net.imput.helium".to_string()),
                app_name: Some("Helium".to_string()),
                window_title: Some("Private window".to_string()),
                browser_url: Some("https://secret.example".to_string()),
                ..FrameMetadataSnapshot::default()
            }),
        );
        runtime.latest_decision = PrivacyFilterDecision {
            excluded_bundle_ids: vec!["net.imput.helium".to_string()],
            privacy_filter_applied: true,
            ..PrivacyFilterDecision::default()
        };
        runtime.latest_applied_decision = PrivacyFilterDecision {
            excluded_bundle_ids: vec!["net.imput.helium".to_string()],
            privacy_filter_applied: true,
            ..PrivacyFilterDecision::default()
        };
    }

    reset_recording_session_privacy_state(&state);

    let runtime = state.lock().expect("capture metadata state should lock");
    assert!(runtime.latest_snapshot().is_none());
    assert_eq!(runtime.latest_decision, PrivacyFilterDecision::default());
    assert_eq!(
        runtime.latest_applied_decision,
        PrivacyFilterDecision::default()
    );
}

#[test]
fn reset_recording_session_privacy_state_clears_browser_url_probe_cache() {
    let state = CaptureMetadataState::default();
    {
        let mut runtime = state.lock().expect("capture metadata state should lock");
        runtime.browser_url_probe_cache = BrowserUrlProbeCache::from_probe(
            Some("com.google.Chrome".to_string()),
            Some("Old Tab — Example".to_string()),
            Some("https://example.com/old-tab".to_string()),
            Instant::now(),
        );
    }

    reset_recording_session_privacy_state(&state);

    let runtime = state.lock().expect("capture metadata state should lock");
    assert_eq!(
        runtime.browser_url_probe_cache.cached_url_for(
            "com.google.Chrome",
            Some("Old Tab — Example"),
            Instant::now()
        ),
        None
    );
}

// The strategy dispatch in `active_browser_url` routes a Chromium bundle to
// the AppleScript path and a Gecko bundle to the Accessibility path. We do
// not run osascript or a live Accessibility read here (those need a running
// browser + permission — that is a manual verify step); we assert the
// routing inputs the dispatch keys off. `browser_url_applescript` backs the
// AppleScript branch, `browser_url_strategy` selects the branch.
#[cfg(target_os = "macos")]
#[test]
fn active_browser_url_routes_chromium_to_applescript_and_gecko_away() {
    // Chromium routes to AppleScript, which has a script to run.
    assert!(matches!(
        browser_url_strategy("com.google.Chrome"),
        Some(BrowserUrlStrategy::AppleScript(_))
    ));
    assert!(browser_url_applescript("com.google.Chrome")
        .is_some_and(|script| script.contains("active tab of front window")));
    // Gecko routes to Accessibility and has no AppleScript surface at all, so
    // dispatch never reaches the osascript path for it.
    assert_eq!(
        browser_url_strategy("org.mozilla.firefox"),
        Some(BrowserUrlStrategy::Accessibility),
    );
    assert_eq!(browser_url_applescript("org.mozilla.firefox"), None);
}

// The provider reads the core-managed state: a frame is stamped with the
// snapshot in effect at its capture instant, and none before the first publish.
#[test]
fn frame_snapshot_provider_reads_the_core_managed_metadata_state() {
    let dir = tempfile::tempdir().expect("temp dir");
    let core = crate::test_support::core_in(Default::default(), dir.path());
    core.manage(CaptureMetadataState::default());
    let snapshot = FrameMetadataSnapshot {
        app_bundle_id: Some("com.example.App".to_string()),
        ..FrameMetadataSnapshot::default()
    };
    core.state::<CaptureMetadataState>()
        .lock()
        .expect("capture metadata state should lock")
        .publish_snapshot(100, Some(snapshot.clone()));

    let provider = frame_metadata_snapshot_provider(&core);

    assert_eq!(provider(50), None);
    assert_eq!(provider(150), Some(snapshot));
}
