use super::*;
use crate::licensing::LicenseGate;
use crate::test_support::{core_in, FakeHost};
use crate::updates::settings::load_app_update_settings_from_path;
use std::sync::{Arc, Mutex};

#[test]
fn update_size_comes_from_the_selected_platform_entry() {
    let url = "https://release.mnema.day/releases/v0.1.24/Mnema_aarch64.app.tar.gz";
    let feed = |size: serde_json::Value| {
        serde_json::json!({
            "version": "0.1.24",
            "platforms": {
                "darwin-x86_64": { "url": "https://x/other.app.tar.gz", "signature": "s", "size": 1 },
                "darwin-aarch64": { "url": url, "signature": "s", "size": size },
            }
        })
    };
    assert_eq!(
        update_size_bytes(&feed(serde_json::json!(38_400_000)), url),
        Some(38_400_000)
    );
    assert_eq!(
        update_size_bytes(&feed(serde_json::json!("38400000")), url),
        None
    );
    assert_eq!(update_size_bytes(&feed(serde_json::json!(-1)), url), None);
    let mut absent = feed(serde_json::Value::Null);
    absent["platforms"]["darwin-aarch64"]
        .as_object_mut()
        .unwrap()
        .remove("size");
    assert_eq!(update_size_bytes(&absent, url), None);
    assert_eq!(
        update_size_bytes(&serde_json::json!({ "version": "0.1.24" }), url),
        None
    );
}

#[test]
fn restart_command_rejects_when_no_installed_update_is_pending() {
    let error =
        restart_after_update_error(false).expect("missing pending update should reject restart");

    assert_eq!(error.kind, AppUpdateErrorKind::Install);
}

#[test]
fn restart_command_allows_restart_while_capture_is_running() {
    // The graceful restart stops and finalizes capture itself.
    assert_eq!(restart_after_update_error(true), None);
}

#[test]
fn update_window_gate_only_declines_out_of_window_licensed_builds() {
    let licensed = |update_through_ms| capture_types::LicenseStatus::Licensed {
        update_through_ms,
        in_window: true,
        email: "a@b.c".into(),
        name: String::new(),
        activation: capture_types::Activation::Activated,
    };

    // Build released after the window → out of window.
    assert!(build_out_of_window(Some(&licensed(1_000)), Some(2_000)));
    // Build within the window → allowed.
    assert!(!build_out_of_window(Some(&licensed(2_000)), Some(1_000)));
    // Build exactly at the boundary → allowed (`<=` is in window).
    assert!(!build_out_of_window(Some(&licensed(1_000)), Some(1_000)));
    // Missing build date → never decline.
    assert!(!build_out_of_window(Some(&licensed(1_000)), None));
    // Non-Licensed states have no Update Window: never gated.
    assert!(!build_out_of_window(
        Some(&capture_types::LicenseStatus::ReadOnly),
        Some(9_999)
    ));
    assert!(!build_out_of_window(
        Some(&capture_types::LicenseStatus::Trial {
            days_left: 3,
            trial_end_ms: 0
        }),
        Some(9_999)
    ));
    // Gate not yet computed → updates flow.
    assert!(!build_out_of_window(None, Some(9_999)));
}

fn out_of_window_licensed() -> capture_types::LicenseStatus {
    capture_types::LicenseStatus::Licensed {
        update_through_ms: 1_000,
        in_window: false,
        email: "a@b.c".into(),
        name: String::new(),
        activation: capture_types::Activation::Activated,
    }
}

#[test]
fn running_build_window_gate_flips_only_resting_states() {
    let status = out_of_window_licensed();
    let build_after_window = Some(2_000);

    // Idle / UpToDate → AvailableOutOfWindow when the running build is past
    // the owner's window (the fresh-install-after-lapse edge).
    for state in [AppUpdateState::Idle, AppUpdateState::UpToDate] {
        assert_eq!(
            running_build_window_gate(state, Some(&status), build_after_window),
            AppUpdateState::AvailableOutOfWindow,
        );
    }
    // Every other state passes through untouched (remote gating is decided
    // in run_update_check, in-flight states must not be rewritten).
    for state in [
        AppUpdateState::Checking,
        AppUpdateState::Available,
        AppUpdateState::AvailableOutOfWindow,
        AppUpdateState::Downloading,
        AppUpdateState::Installing,
        AppUpdateState::RestartRequired,
        AppUpdateState::Incompatible,
        AppUpdateState::Failed,
    ] {
        assert_eq!(
            running_build_window_gate(state, Some(&status), build_after_window),
            state,
        );
    }
}

#[test]
fn running_build_window_gate_never_flips_in_window_or_unknown() {
    // In-window build, missing build date, or no computed status → resting
    // states stay resting.
    for (status, build_date) in [
        (Some(out_of_window_licensed()), Some(500)), // build within window
        (Some(out_of_window_licensed()), None),      // no build date
        (None, Some(2_000)),                         // gate not yet computed
    ] {
        assert_eq!(
            running_build_window_gate(AppUpdateState::Idle, status.as_ref(), build_date),
            AppUpdateState::Idle,
        );
    }
}

#[test]
fn out_of_window_remote_update_is_never_installable_but_still_notifies() {
    // Out of window: surfaced, pending_update dropped; a notify check still
    // notifies (the window, gated once per version — not the install bell).
    assert_eq!(
        decide_remote_update(true, true),
        RemoteUpdateDecision {
            state: AppUpdateState::AvailableOutOfWindow,
            installable: false,
            notify: true,
        },
    );
    assert!(!decide_remote_update(true, false).notify);
    assert_eq!(
        decide_remote_update(true, false).state,
        AppUpdateState::AvailableOutOfWindow,
    );
    // In window: installable; notification tracks the caller's flag.
    assert_eq!(
        decide_remote_update(false, true),
        RemoteUpdateDecision {
            state: AppUpdateState::Available,
            installable: true,
            notify: true,
        },
    );
    assert_eq!(
        decide_remote_update(false, false),
        RemoteUpdateDecision {
            state: AppUpdateState::Available,
            installable: true,
            notify: false,
        },
    );
}

/// The out-of-window notice opens the window once per remote version, and
/// the gate survives a reload of the settings file (a relaunch).
#[test]
fn out_of_window_notice_opens_once_per_version_across_reloads() {
    let dir = tempfile::tempdir().expect("temp dir should be created");
    let path = dir.path().join("app-update-settings.json");
    let stored = AppUpdateSettings {
        channel: AppUpdateChannel::Preview,
        ..AppUpdateSettings::default()
    };

    let first = out_of_window_notice(&stored, "0.1.24").expect("first sighting opens");
    assert_eq!(first.channel, AppUpdateChannel::Preview, "channel kept");
    std::fs::write(&path, serde_json::to_string(&first).unwrap()).unwrap();

    let reloaded = load_app_update_settings_from_path(&path);
    assert_eq!(out_of_window_notice(&reloaded, "0.1.24"), None, "no reopen");
    let next = out_of_window_notice(&reloaded, "0.1.25").expect("new version opens again");
    let gate = next.notified_out_of_window_version;
    assert_eq!(gate.as_deref(), Some("0.1.25"));

    // A file written before the field existed still loads.
    std::fs::write(&path, r#"{"channel":"preview"}"#).unwrap();
    assert_eq!(
        load_app_update_settings_from_path(&path).notified_out_of_window_version,
        None
    );
}

fn status(state: AppUpdateState) -> AppUpdateStatus {
    AppUpdateStatus {
        app: AppUpdateAppInfo {
            product_name: "Mnema".into(),
            version: "0.1.24".into(),
            identifier: "day.mnema".into(),
            platform: "macos".into(),
            arch: "aarch64".into(),
        },
        channel: AppUpdateChannel::Stable,
        state,
        update: None,
        progress: None,
        error: None,
        last_checked_at_unix_ms: Some(42),
    }
}

#[test]
fn a_status_change_emits_app_update_status_changed_with_the_old_json() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(FakeHost::default());
    let core = core_in(host.clone(), dir.path());

    emit_status(&core, status(AppUpdateState::Checking));

    let events = host.events.lock().unwrap();
    let [CoreEvent::AppUpdateStatusChanged(emitted)] = events.as_slice() else {
        panic!("expected one AppUpdateStatusChanged, got {events:?}");
    };
    assert_eq!(emitted, &status(AppUpdateState::Checking));
    // The Tauri frontend reads this camelCase shape.
    let json = serde_json::to_value(emitted).unwrap();
    assert_eq!(json["state"], "checking");
    assert_eq!(json["channel"], "stable");
    assert_eq!(json["lastCheckedAtUnixMs"], 42);
    assert_eq!(json["app"]["productName"], "Mnema");
}

#[test]
fn an_out_of_window_license_gives_available_out_of_window() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(Arc::new(FakeHost::default()), dir.path());

    // No license computed yet: updates flow.
    assert_eq!(
        apply_running_build_window_gate(&core, AppUpdateState::Idle),
        AppUpdateState::Idle
    );

    // update_through 1_000 ms: every real build (stamped by build.rs) is past it.
    core.manage(LicenseGate(Mutex::new(Some(out_of_window_licensed()))));
    for state in [AppUpdateState::Idle, AppUpdateState::UpToDate] {
        assert_eq!(
            apply_running_build_window_gate(&core, state),
            AppUpdateState::AvailableOutOfWindow
        );
    }
    let remote_out =
        build_out_of_window(crate::licensing::cached_status(&core).as_ref(), Some(2_000));
    let decision = decide_remote_update(remote_out, true);
    assert_eq!(decision.state, AppUpdateState::AvailableOutOfWindow);
    assert!(!decision.installable);
}
