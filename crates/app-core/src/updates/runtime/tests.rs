use super::*;
use crate::updates::status::decide_remote_update;

fn release(version: &str) -> RemoteRelease {
    RemoteRelease {
        version: version.into(),
        notes: None,
        date: None,
        date_ms: None,
        url: "https://x/a.tar.gz".into(),
        signature: "sig".into(),
        size_bytes: Some(10),
    }
}

fn available(rt: &mut AppUpdateRuntime) {
    assert!(rt.begin_check(1));
    let r = release("0.2.0");
    let info = update_info(&r, AppUpdateChannel::Stable);
    rt.found(r, info, &decide_remote_update(false, false));
}

#[test]
fn check_runs_idle_to_available_and_keeps_the_release() {
    let mut rt = AppUpdateRuntime::default();
    assert!(rt.begin_check(5));
    assert_eq!(rt.state, AppUpdateState::Checking);
    assert_eq!(rt.last_checked_at_unix_ms, Some(5));
    let r = release("0.2.0");
    let info = update_info(&r, AppUpdateChannel::Stable);
    rt.found(r.clone(), info, &decide_remote_update(false, true));
    assert_eq!(rt.state, AppUpdateState::Available);
    assert_eq!(rt.pending, Some(r));
    assert_eq!(rt.update.as_ref().unwrap().size_bytes, Some(10));
}

#[test]
fn an_out_of_window_release_is_shown_but_not_installable() {
    let mut rt = AppUpdateRuntime::default();
    assert!(rt.begin_check(1));
    let r = release("0.2.0");
    let info = update_info(&r, AppUpdateChannel::Stable);
    rt.found(r, info, &decide_remote_update(true, true));
    assert_eq!(rt.state, AppUpdateState::AvailableOutOfWindow);
    assert!(rt.pending.is_none());
    assert!(rt.update.is_some());
    assert_eq!(rt.begin_install(), Err(()));
    assert_eq!(rt.state, AppUpdateState::Failed);
}

#[test]
fn checking_up_to_date_and_failures() {
    let mut rt = AppUpdateRuntime::default();
    assert!(rt.begin_check(1));
    rt.up_to_date();
    assert_eq!(rt.state, AppUpdateState::UpToDate);
    assert!(rt.begin_check(2));
    rt.fail(AppUpdateError::new(AppUpdateErrorKind::Network, "x"));
    assert_eq!(rt.state, AppUpdateState::Failed);
    assert!(rt.begin_check(3));
    rt.fail(AppUpdateError::new(AppUpdateErrorKind::Incompatible, "x"));
    assert_eq!(rt.state, AppUpdateState::Incompatible);
}

#[test]
fn a_second_check_is_refused_while_one_runs_or_an_install_is_under_way() {
    let mut rt = AppUpdateRuntime::default();
    assert!(rt.begin_check(1));
    assert!(!rt.begin_check(2));
    assert_eq!(rt.last_checked_at_unix_ms, Some(1));
    let r = release("0.2.0");
    let info = update_info(&r, AppUpdateChannel::Stable);
    rt.found(r, info, &decide_remote_update(false, false));
    rt.begin_install().unwrap().unwrap();
    assert!(!rt.begin_check(3));
    rt.installing();
    assert!(!rt.begin_check(3));
    rt.installed();
    assert!(!rt.begin_check(3), "a staged restart never re-checks");
}

#[test]
fn install_runs_available_downloading_installing_restart_required() {
    let mut rt = AppUpdateRuntime::default();
    available(&mut rt);
    let r = rt.begin_install().unwrap().expect("a release to fetch");
    assert_eq!(r.version, "0.2.0");
    assert_eq!(rt.state, AppUpdateState::Downloading);
    assert_eq!(rt.progress.as_ref().unwrap().content_length_bytes, Some(10));
    rt.downloaded(4, Some(10));
    assert_eq!(rt.progress.as_ref().unwrap().downloaded_bytes, 4);
    assert_eq!(rt.begin_install(), Ok(None), "a second click is a no-op");
    rt.installing();
    assert_eq!(rt.state, AppUpdateState::Installing);
    assert!(rt.progress.is_none());
    rt.installed();
    assert_eq!(rt.state, AppUpdateState::RestartRequired);
    assert!(rt.restart_required);
}

#[test]
fn a_refused_install_fails_and_stays_retryable() {
    let mut rt = AppUpdateRuntime::default();
    available(&mut rt);
    rt.begin_install().unwrap().unwrap();
    rt.installing();
    rt.fail(gate_error(GateRefusal::TeamMismatch));
    assert_eq!(rt.state, AppUpdateState::Failed);
    let err = rt.error.clone().unwrap();
    assert_eq!(err.kind, AppUpdateErrorKind::Verification);
    assert_eq!(err.message, NOT_SIGNED_BY_MNEMA);
    assert!(!rt.restart_required);
    // Retry: the same release downloads again.
    assert!(rt.begin_install().unwrap().is_some());
}

#[test]
fn install_without_a_check_fails() {
    let mut rt = AppUpdateRuntime::default();
    assert_eq!(rt.begin_install(), Err(()));
    assert_eq!(rt.state, AppUpdateState::Failed);
    assert_eq!(rt.error.unwrap().kind, AppUpdateErrorKind::Install);
}

#[test]
fn a_stale_channel_check_drops_its_result() {
    let mut rt = AppUpdateRuntime::default();
    available(&mut rt);
    rt.discard_stale_check();
    assert_eq!(rt.state, AppUpdateState::Idle);
    assert!(rt.pending.is_none() && rt.update.is_none());
}

#[test]
fn the_available_notice_is_due_once_per_version() {
    let mut rt = AppUpdateRuntime::default();
    assert!(rt.take_notice("0.2.0"));
    assert!(!rt.take_notice("0.2.0"));
    assert!(rt.take_notice("0.3.0"));
}

#[test]
fn a_running_app_without_a_team_says_so_not_that_the_update_is_bad() {
    let err = gate_error(GateRefusal::NoOwnTeam);
    assert_eq!(err.kind, AppUpdateErrorKind::Install);
    assert_ne!(err.message, NOT_SIGNED_BY_MNEMA);
    for refusal in [
        GateRefusal::InvalidSignature,
        GateRefusal::TeamMismatch,
        GateRefusal::BundleIdMismatch,
        GateRefusal::HelperTeamMismatch,
        GateRefusal::Unreadable,
    ] {
        assert_eq!(gate_error(refusal).message, NOT_SIGNED_BY_MNEMA);
    }
}

#[test]
fn the_feed_override_replaces_the_channel_url() {
    let mut config = UpdaterConfig::for_main_bundle("day.mnema.test", "0.1.0");
    assert_eq!(config.platform_key, GPUI_PLATFORM_KEY);
    config.feed_override = None;
    assert_eq!(
        config.feed_url(AppUpdateChannel::Preview),
        crate::updates::PREVIEW_UPDATE_ENDPOINT
    );
    config.feed_override = Some("http://127.0.0.1:9/latest.json".into());
    assert_eq!(
        config.feed_url(AppUpdateChannel::Stable),
        "http://127.0.0.1:9/latest.json"
    );
    // A test binary isn't a bundle: install is off, the fallbacks stand in.
    assert!(config.installed_app.is_none());
    assert_eq!(config.app.version, "0.1.0");
}
