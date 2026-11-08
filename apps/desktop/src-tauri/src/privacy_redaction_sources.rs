//! Tauri commands for the app-exclusion rules; the mutations live in
//! `app_core::privacy_redaction_sources`. They stay here because they apply
//! through `native_capture`'s settings-domain update (S3).

use app_core::privacy_redaction_sources as sources;
use capture_types::{
    CaptureErrorResponse, RecordingSettings, RecordingSettingsDomainUpdateResponse,
    SettingsOwnershipDomain,
};

fn with_app_exclusion_mutation(
    app_handle: tauri::AppHandle,
    mutate: impl FnOnce(&mut RecordingSettings) -> Result<(), CaptureErrorResponse>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    crate::native_capture::apply_recording_settings_domain_mutation_from_app_handle(
        &app_handle,
        SettingsOwnershipDomain::AppPrivacyExclusion,
        mutate,
    )
}

#[tauri::command]
pub fn add_privacy_excluded_app(
    bundle_id: String,
    display_name: String,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(app_handle, |settings| {
        sources::upsert_privacy_excluded_app(settings, &bundle_id, &display_name, false)
    })
}

pub(crate) fn add_or_enable_privacy_excluded_app_from_app_handle(
    app_handle: tauri::AppHandle,
    bundle_id: String,
    display_name: String,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(app_handle, |settings| {
        sources::upsert_privacy_excluded_app(settings, &bundle_id, &display_name, true)
    })
}

#[tauri::command]
pub fn set_privacy_excluded_app_enabled(
    source_id: String,
    enabled: bool,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(app_handle, |settings| {
        sources::set_privacy_excluded_app_enabled(settings, &source_id, enabled)
    })
}

/// Whether the privacy excluded-apps list also filters the system-audio tap
/// (Settings → Privacy → "Filter system audio"). Mnema's own-process exclusion
/// is never toggleable. Routed through the same privacy-domain mutation seam as
/// the excluded-apps edits, so a mid-recording flip reaches the live tap through
/// the same refresh path a privacy-list edit takes.
#[tauri::command]
pub fn set_privacy_filter_system_audio(
    enabled: bool,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(app_handle, |settings| {
        settings.privacy.filter_system_audio = enabled;
        Ok(())
    })
}

#[tauri::command]
pub fn remove_privacy_excluded_app(
    source_id: String,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(app_handle, |settings| {
        sources::remove_privacy_excluded_app(settings, &source_id)
    })
}
