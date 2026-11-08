use super::*;
use capture_types::{NativeCaptureSession, SourceSessionMeta, SourceSessions};

fn stopped_session() -> NativeCaptureSession {
    NativeCaptureSession {
        is_running: false,
        is_inactivity_paused: false,
        is_user_paused: false,
        is_low_disk_suspended: false,
        requested_sources: None,
        output_files: None,
        source_sessions: None,
    }
}

#[test]
fn install_stops_a_running_capture_session_first() {
    let mut session = stopped_session();
    session.is_running = true;

    assert!(capture_session_needs_stop_before_install(&session));
}

#[test]
fn install_stops_a_user_paused_capture_session_first() {
    let mut session = stopped_session();
    session.is_user_paused = true;

    assert!(capture_session_needs_stop_before_install(&session));
}

#[test]
fn stopped_session_with_finalized_source_metadata_needs_no_stop() {
    // A stopped session preserves `source_sessions` as finalized metadata
    // (see `stopped_session_from_runtime`). That is not a liveness signal, so
    // it must not trigger a pointless stop before the install.
    let mut session = stopped_session();
    session.source_sessions = Some(SourceSessions {
        screen: Some(SourceSessionMeta {
            session_id: "s1".into(),
            started_at_unix_ms: 1,
        }),
        microphone: None,
        system_audio: None,
    });

    assert!(!capture_session_needs_stop_before_install(&session));
}

#[test]
fn startup_check_availability_notification_targets_about_settings() {
    let update = AppUpdateInfo {
        version: "0.3.0".into(),
        date: None,
        notes: None,
        channel: AppUpdateChannel::Stable,
        size_bytes: None,
    };

    let notification = startup_update_notification_for_update(&update, 42);

    assert_eq!(notification.id, APP_UPDATE_AVAILABLE_NOTIFICATION_ID);
    assert_eq!(notification.severity, "info");
    match notification.action {
        Some(native_capture::AppNotificationAction::OpenSettingsTab { tab }) => {
            assert_eq!(tab, "about");
        }
        _ => panic!("expected open settings action"),
    }
}

#[test]
fn update_error_mapping_buckets_common_errors() {
    assert_eq!(
        map_update_error_kind(&tauri_plugin_updater::Error::ReleaseNotFound),
        AppUpdateErrorKind::Feed
    );
    assert_eq!(
        map_update_error_kind(&tauri_plugin_updater::Error::UnsupportedOs),
        AppUpdateErrorKind::Incompatible
    );
    assert_eq!(
        map_update_error_kind(&tauri_plugin_updater::Error::Network("offline".into())),
        AppUpdateErrorKind::Network
    );
    assert_eq!(
        map_update_error_kind(&tauri_plugin_updater::Error::PackageInstallFailed),
        AppUpdateErrorKind::Install
    );
}
