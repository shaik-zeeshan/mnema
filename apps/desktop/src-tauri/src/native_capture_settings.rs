//! The recording-settings saves that must ask the running capture first (sources
//! are locked while recording). The rest of recording settings lives in
//! `app_core::recording_settings`; this moves with capture in S3.

use app_core::recording_settings::{
    apply_domain_patch_to_settings, current_recording_settings, persist_recording_settings,
    recording_settings_request_from_settings, validate_privacy_settings,
    validate_recording_settings, AppliedRecordingSettingsUpdate, RecordingSettingsDomainPatch,
    RecordingSettingsState,
};
use app_core::AppCore;
use capture_types::{
    CaptureErrorResponse, RecordingSettings, SettingsOwnershipDomain,
    UpdateRecordingSettingsRequest,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::Manager;

/// Starts that have read their sources (`recording_settings_for_start`), and
/// starts that have since returned. Begun only moves under the settings lock.
static STARTS_BEGUN: AtomicU64 = AtomicU64::new(0);
static STARTS_DONE: AtomicU64 = AtomicU64::new(0);

/// Held for the whole start; dropping it marks the start returned.
pub(crate) struct StartSourcesRead;

impl Drop for StartSourcesRead {
    fn drop(&mut self) {
        STARTS_DONE.fetch_add(1, Ordering::SeqCst);
    }
}

/// The start path's settings read. Taken under the settings lock so a sources
/// save either lands before it (and the start records the new sources) or sees
/// the start and is refused: see [`CaptureUse`].
pub(crate) fn recording_settings_for_start(
    state: &RecordingSettingsState,
) -> (RecordingSettings, StartSourcesRead) {
    let runtime = state.lock().expect("recording settings state poisoned");
    STARTS_BEGUN.fetch_add(1, Ordering::SeqCst);
    (runtime.settings.clone(), StartSourcesRead)
}

/// Whether capture may be using the sources a save is about to change, read in
/// two halves around the settings lock. Capture in use = a start in flight or a
/// session running before the settings lock, or a start begun since (it read
/// the old sources). Read in this order, any start overlapping the save is
/// caught by one of the three.
struct CaptureUse {
    starts_begun: u64,
    start_in_flight: bool,
    session_running: bool,
}

impl CaptureUse {
    /// The first half, before the settings lock: `session_running` takes the
    /// capture lock, which is never taken inside it.
    fn before_settings_lock(session_running: impl FnOnce() -> bool) -> Self {
        let starts_begun = STARTS_BEGUN.load(Ordering::SeqCst);
        let start_in_flight = starts_begun != STARTS_DONE.load(Ordering::SeqCst);
        Self {
            starts_begun,
            start_in_flight,
            session_running: !start_in_flight && session_running(),
        }
    }

    /// The second half, under the settings lock.
    fn in_use(&self) -> bool {
        self.start_in_flight
            || self.session_running
            || STARTS_BEGUN.load(Ordering::SeqCst) != self.starts_begun
    }
}

fn capture_session_running(app_handle: &tauri::AppHandle) -> bool {
    app_handle
        .try_state::<super::NativeCaptureState>()
        .is_some_and(|capture| {
            capture
                .lock()
                .is_ok_and(|lifecycle| lifecycle.runtime().is_running)
        })
}

fn capture_sources_differ(previous: &RecordingSettings, next: &RecordingSettings) -> bool {
    previous.capture_screen != next.capture_screen
        || previous.capture_microphone != next.capture_microphone
        || previous.capture_system_audio != next.capture_system_audio
}

/// Sources are fixed for a session: the running capture never picks up a
/// change, so a saved one would read as off while it keeps recording. The
/// Settings switches lock while recording, but their save is debounced and can
/// land after Start; this is the backstop. Unchanged sources always pass.
fn ensure_capture_sources_unlocked(
    previous: &RecordingSettings,
    next: &RecordingSettings,
    capture_in_use: bool,
) -> Result<(), CaptureErrorResponse> {
    if capture_sources_differ(previous, next) && capture_in_use {
        return Err(CaptureErrorResponse {
            code: CAPTURE_SOURCES_LOCKED.to_string(),
            message: "Stop recording to change sources.".to_string(),
        });
    }
    Ok(())
}

/// Error code for a sources change refused mid-session. The frontend reverts
/// the toggle quietly on it (`isCaptureSourcesLocked`).
pub(crate) const CAPTURE_SOURCES_LOCKED: &str = "capture_sources_locked";

pub(crate) fn apply_recording_settings_update(
    app_handle: &tauri::AppHandle,
    state: &RecordingSettingsState,
    request: UpdateRecordingSettingsRequest,
) -> Result<AppliedRecordingSettingsUpdate, CaptureErrorResponse> {
    let requested_privacy = validate_privacy_settings(request.privacy.clone())?;
    let settings = validate_recording_settings(request)?;
    // The full save carries the sources too, so it is held to the same lock as
    // the per-field one. Only a change asks the capture lock.
    let capture_use = CaptureUse::before_settings_lock(|| {
        capture_sources_differ(&current_recording_settings(state), &settings)
            && capture_session_running(app_handle)
    });

    let mut runtime = state.lock().expect("recording settings state poisoned");
    check_full_recording_settings_update(
        &runtime.settings,
        &settings,
        &requested_privacy,
        &capture_use,
    )?;

    let previous_settings = runtime.settings.clone();
    let previous_save_directory = previous_settings.save_directory.clone();
    let save_directory_changed = previous_save_directory != settings.save_directory;

    persist_recording_settings(&app_handle.state::<Arc<AppCore>>(), &settings)?;
    runtime.settings = settings.clone();

    Ok(AppliedRecordingSettingsUpdate {
        settings,
        previous_settings,
        previous_save_directory,
        save_directory_changed,
    })
}

fn check_full_recording_settings_update(
    previous: &RecordingSettings,
    next: &RecordingSettings,
    requested_privacy: &capture_types::PrivacySettings,
    capture_use: &CaptureUse,
) -> Result<(), CaptureErrorResponse> {
    if *requested_privacy != previous.privacy {
        return Err(CaptureErrorResponse {
            code: "invalid_privacy_rule".to_string(),
            message: "Privacy rules must be changed with dedicated privacy commands".to_string(),
        });
    }
    ensure_capture_sources_unlocked(previous, next, capture_use.in_use())
}

/// Whether a patch's save must ask the capture lock: only one that changes
/// the sources. The lock can be held for seconds while a stop flushes, and
/// these saves run on the main thread.
fn domain_patch_changes_sources(
    current: &RecordingSettings,
    patch: &RecordingSettingsDomainPatch,
) -> bool {
    let RecordingSettingsDomainPatch::CaptureSources(request) = patch else {
        return false;
    };
    let changes = |requested: Option<bool>, current: bool| requested.is_some_and(|v| v != current);
    changes(request.capture_screen, current.capture_screen)
        || changes(request.capture_microphone, current.capture_microphone)
        || changes(request.capture_system_audio, current.capture_system_audio)
}

pub(crate) fn apply_recording_settings_domain_patch(
    app_handle: &tauri::AppHandle,
    state: &RecordingSettingsState,
    patch: RecordingSettingsDomainPatch,
) -> Result<(SettingsOwnershipDomain, AppliedRecordingSettingsUpdate), CaptureErrorResponse> {
    let domain = patch.domain();
    let sources_may_change =
        domain_patch_changes_sources(&current_recording_settings(state), &patch);
    apply_domain_mutation(app_handle, state, domain, sources_may_change, |settings| {
        apply_domain_patch_to_settings(settings, patch)?;
        Ok(())
    })
    .map(|update| (domain, update))
}

pub(crate) fn apply_recording_settings_domain_mutation(
    app_handle: &tauri::AppHandle,
    state: &RecordingSettingsState,
    domain: SettingsOwnershipDomain,
    mutate: impl FnOnce(&mut RecordingSettings) -> Result<(), CaptureErrorResponse>,
) -> Result<AppliedRecordingSettingsUpdate, CaptureErrorResponse> {
    let sources_domain = domain == SettingsOwnershipDomain::CaptureSources;
    apply_domain_mutation(app_handle, state, domain, sources_domain, mutate)
}

/// `sources_may_change` is read before the settings lock, like the full save's
/// check: a concurrent sources save landing in between is the same narrow
/// window the full save accepts.
fn apply_domain_mutation(
    app_handle: &tauri::AppHandle,
    state: &RecordingSettingsState,
    domain: SettingsOwnershipDomain,
    sources_may_change: bool,
    mutate: impl FnOnce(&mut RecordingSettings) -> Result<(), CaptureErrorResponse>,
) -> Result<AppliedRecordingSettingsUpdate, CaptureErrorResponse> {
    let sources_domain = domain == SettingsOwnershipDomain::CaptureSources;
    let capture_use = CaptureUse::before_settings_lock(|| {
        sources_may_change && capture_session_running(app_handle)
    });

    let mut runtime = state.lock().expect("recording settings state poisoned");
    let mut next_settings = runtime.settings.clone();

    mutate(&mut next_settings)?;

    let settings =
        validate_recording_settings(recording_settings_request_from_settings(next_settings))?;
    let previous_settings = runtime.settings.clone();
    let previous_save_directory = previous_settings.save_directory.clone();
    let save_directory_changed = previous_save_directory != settings.save_directory;

    if domain != SettingsOwnershipDomain::AppPrivacyExclusion
        && previous_settings.privacy != settings.privacy
    {
        return Err(CaptureErrorResponse {
            code: "invalid_privacy_rule".to_string(),
            message: "Privacy rules must be changed with dedicated privacy commands".to_string(),
        });
    }

    if sources_domain {
        ensure_capture_sources_unlocked(&previous_settings, &settings, capture_use.in_use())?;
    }

    persist_recording_settings(&app_handle.state::<Arc<AppCore>>(), &settings)?;
    runtime.settings = settings.clone();

    Ok(AppliedRecordingSettingsUpdate {
        settings,
        previous_settings,
        previous_save_directory,
        save_directory_changed,
    })
}

#[cfg(test)]
#[path = "native_capture_settings_tests.rs"]
mod tests;
