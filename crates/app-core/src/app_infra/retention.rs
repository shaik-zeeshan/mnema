//! Retention Cleanup's policy mapping and status, plus the payload every
//! timeline invalidation (retention, Delete Recent Capture) carries.

use super::*;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRetentionCleanupRequest {
    pub policy: Option<SettingsRetentionPolicy>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineDataChangedPayload {
    pub reason: String,
    pub deleted_before: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub deleted_frame_ids: Vec<i64>,
    pub deleted_audio_segment_ids: Vec<i64>,
}

pub fn app_retention_policy(policy: SettingsRetentionPolicy) -> ::app_infra::RetentionPolicy {
    match policy {
        SettingsRetentionPolicy::Never => ::app_infra::RetentionPolicy::Never,
        SettingsRetentionPolicy::Days7 => ::app_infra::RetentionPolicy::Days7,
        SettingsRetentionPolicy::Days14 => ::app_infra::RetentionPolicy::Days14,
        SettingsRetentionPolicy::Days30 => ::app_infra::RetentionPolicy::Days30,
    }
}

pub fn local_now_for_retention() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}

pub fn retention_policy_from_settings(
    settings: &crate::recording_settings::RecordingSettingsState,
    override_policy: Option<SettingsRetentionPolicy>,
) -> SettingsRetentionPolicy {
    override_policy.unwrap_or_else(|| {
        settings
            .lock()
            .map(|guard| guard.settings.retention_policy)
            .unwrap_or(SettingsRetentionPolicy::Never)
    })
}

pub async fn get_retention_cleanup_status(
    infra: &AppInfraState,
    settings: &crate::recording_settings::RecordingSettingsState,
) -> Result<::app_infra::RetentionCleanupSummary, String> {
    let policy = retention_policy_from_settings(settings, None);
    Arc::clone(&*infra)
        .capture_retention()
        .latest_status(app_retention_policy(policy))
        .await
        .map_err(|error| format!("failed to read retention cleanup status: {error}"))
}
