use super::*;
use crate::recording_settings::default_recording_settings;
use capture_types::UpdateCaptureSourceSettingsRequest;

// A sources save that changes nothing never asks the capture lock, which a
// stop can hold for seconds while these saves run on the main thread.
#[test]
fn only_a_sources_patch_that_changes_them_consults_the_capture() {
    let current = default_recording_settings();
    let patch = |microphone: bool| {
        RecordingSettingsDomainPatch::CaptureSources(UpdateCaptureSourceSettingsRequest {
            capture_screen: Some(current.capture_screen),
            capture_microphone: Some(microphone),
            capture_system_audio: None,
        })
    };

    assert!(!domain_patch_changes_sources(
        &current,
        &patch(current.capture_microphone)
    ));
    assert!(domain_patch_changes_sources(
        &current,
        &patch(!current.capture_microphone)
    ));
    assert!(!domain_patch_changes_sources(
        &current,
        &RecordingSettingsDomainPatch::Developer(Default::default())
    ));
}

#[test]
fn capture_sources_change_is_refused_only_while_capture_is_in_use() {
    let previous = default_recording_settings();
    let mut next = previous.clone();
    next.capture_microphone = !next.capture_microphone;

    let refused = ensure_capture_sources_unlocked(&previous, &next, true)
        .expect_err("a source change mid-session never reaches the capture");
    assert_eq!(refused.code, CAPTURE_SOURCES_LOCKED);
    assert!(ensure_capture_sources_unlocked(&previous, &next, false).is_ok());
    // Re-saving the same sources mid-session is harmless.
    assert!(ensure_capture_sources_unlocked(&previous, &previous, true).is_ok());
}

// The start/save counter protocol end to end, through the full save's
// check. One test, because the counters are process-wide.
#[test]
fn a_full_save_refuses_a_sources_change_while_a_start_overlaps_it() {
    let state = RecordingSettingsState::default();
    let previous = default_recording_settings();
    let mut next = previous.clone();
    next.capture_microphone = !next.capture_microphone;
    let check = |next: &RecordingSettings, capture_use: &CaptureUse| {
        check_full_recording_settings_update(&previous, next, &previous.privacy, capture_use)
    };

    // A start in flight when the save begins.
    let start = recording_settings_for_start(&state);
    let capture_use =
        CaptureUse::before_settings_lock(|| panic!("a start in flight answers first"));
    let refused = check(&next, &capture_use).expect_err("the start may have read the old sources");
    assert_eq!(refused.code, CAPTURE_SOURCES_LOCKED);
    assert!(
        check(&previous, &capture_use).is_ok(),
        "unchanged sources always pass"
    );
    drop(start);

    // A start begun between the two halves read the old sources.
    let capture_use = CaptureUse::before_settings_lock(|| false);
    let start = recording_settings_for_start(&state);
    assert!(check(&next, &capture_use).is_err());
    drop(start);

    // A running session.
    assert!(check(&next, &CaptureUse::before_settings_lock(|| true)).is_err());

    // Nothing overlaps: the change saves.
    assert!(check(&next, &CaptureUse::before_settings_lock(|| false)).is_ok());
}
