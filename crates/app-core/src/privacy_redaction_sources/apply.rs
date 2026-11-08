//! The app-exclusion edits, applied through the privacy-domain settings save so
//! a mid-recording edit reaches the live filter through the same refresh path.

use std::sync::Arc;

use capture_types::{
    CaptureErrorResponse, RecordingSettings, RecordingSettingsDomainUpdateResponse,
    SettingsOwnershipDomain,
};

use crate::privacy_redaction_sources as sources;
use crate::AppCore;

fn with_app_exclusion_mutation(
    core: &Arc<AppCore>,
    mutate: impl FnOnce(&mut RecordingSettings) -> Result<(), CaptureErrorResponse>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    crate::native_capture::settings_update::apply_recording_settings_domain_mutation_from_core(
        core,
        SettingsOwnershipDomain::AppPrivacyExclusion,
        mutate,
    )
}

pub fn add_privacy_excluded_app(
    bundle_id: String,
    display_name: String,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(core, |settings| {
        sources::upsert_privacy_excluded_app(settings, &bundle_id, &display_name, false)
    })
}

pub fn add_or_enable_privacy_excluded_app_from_core(
    core: &Arc<AppCore>,
    bundle_id: String,
    display_name: String,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(core, |settings| {
        sources::upsert_privacy_excluded_app(settings, &bundle_id, &display_name, true)
    })
}

pub fn set_privacy_excluded_app_enabled(
    source_id: String,
    enabled: bool,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(core, |settings| {
        sources::set_privacy_excluded_app_enabled(settings, &source_id, enabled)
    })
}

/// Whether the privacy excluded-apps list also filters the system-audio tap
/// (Settings → Privacy → "Filter system audio"). Mnema's own-process exclusion
/// is never toggleable. Routed through the same privacy-domain mutation seam as
/// the excluded-apps edits, so a mid-recording flip reaches the live tap through
/// the same refresh path a privacy-list edit takes.
pub fn set_privacy_filter_system_audio(
    enabled: bool,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(core, |settings| {
        settings.privacy.filter_system_audio = enabled;
        Ok(())
    })
}

pub fn remove_privacy_excluded_app(
    source_id: String,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(core, |settings| {
        sources::remove_privacy_excluded_app(settings, &source_id)
    })
}
