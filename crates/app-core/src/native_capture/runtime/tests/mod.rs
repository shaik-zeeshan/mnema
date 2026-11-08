mod session;
mod sources;

use super::source_session_suffix;
#[cfg(target_os = "macos")]
use super::{
    CaptureSuspension, CaptureSuspensionKind, CaptureSuspensionStatus,
    MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS,
};
#[cfg(target_os = "macos")]
use capture_types::CaptureErrorResponse;

#[test]
fn source_session_suffix_removes_native_session_prefix() {
    assert_eq!(
        source_session_suffix("native-session-ceb00964-9039-4e1c-a770-c2c1a1251e83"),
        "ceb00964_9039_4e1c_a770_c2c1a1251e83"
    );
}

#[test]
fn source_session_suffix_keeps_legacy_session_prefix_compatibility() {
    assert_eq!(
        source_session_suffix("session-ceb00964-9039-4e1c-a770-c2c1a1251e83"),
        "ceb00964_9039_4e1c_a770_c2c1a1251e83"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn capture_suspension_requires_restart_after_bounded_failures() {
    let error = CaptureErrorResponse {
        code: "privacy_filter_apply_failed".to_string(),
        message: "filter failed".to_string(),
    };
    let mut suspension = CaptureSuspension::with_kind(CaptureSuspensionKind::PrivacyFilter, &error);

    for _ in 0..MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS {
        assert!(suspension.can_retry());
        suspension.record_recovery_failure(&error);
    }

    assert!(!suspension.can_retry());
    assert_eq!(suspension.status, CaptureSuspensionStatus::RestartRequired);
    assert_eq!(suspension.reason, "privacy_recovery_restart_required");
}

#[cfg(target_os = "macos")]
#[test]
fn display_unavailable_suspension_never_stops_retrying() {
    let error = CaptureErrorResponse {
        code: "privacy_filter_display_unavailable".to_string(),
        message: "no display".to_string(),
    };
    let mut suspension =
        CaptureSuspension::with_kind(CaptureSuspensionKind::DisplayUnavailable, &error);

    // A display returning is expected, not a failure to give up on: even far
    // past the privacy-filter cap, recovery keeps retrying and never escalates
    // to the manual-restart state.
    for _ in 0..(MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS as u16 + 5) {
        assert!(suspension.can_retry());
        suspension.record_recovery_failure(&error);
    }

    assert!(suspension.can_retry());
    assert_eq!(suspension.status, CaptureSuspensionStatus::Retryable);
}

#[cfg(target_os = "macos")]
#[test]
fn low_disk_suspension_never_stops_retrying() {
    let error = CaptureErrorResponse {
        code: "capture_low_disk".to_string(),
        message: "insufficient disk space".to_string(),
    };
    let mut suspension = CaptureSuspension::with_kind(CaptureSuspensionKind::LowDisk, &error);

    // Free space recovering is expected, not a failure to give up on: even far
    // past the privacy-filter cap, recovery keeps retrying and never escalates
    // to the manual-restart state.
    for _ in 0..(MAX_PRIVACY_CAPTURE_RECOVERY_ATTEMPTS as u16 + 5) {
        assert!(suspension.can_retry());
        suspension.record_recovery_failure(&error);
    }

    assert!(suspension.can_retry());
    assert_eq!(suspension.status, CaptureSuspensionStatus::Retryable);
}
