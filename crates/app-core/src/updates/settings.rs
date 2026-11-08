//! The update channel, its feed URLs, and the persisted update settings.

use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use super::status::{AppUpdateError, AppUpdateErrorKind};
use crate::AppCore;

const APP_UPDATE_SETTINGS_FILE_NAME: &str = "app-update-settings.json";
// Update feeds live on Cloudflare R2 behind release.mnema.day (one latest.json
// per channel, written by the promote workflow), so they stay reachable even
// if the source repository goes private. See docs/release-process.md.
pub const STABLE_UPDATE_ENDPOINT: &str = "https://release.mnema.day/stable/latest.json";
pub const PREVIEW_UPDATE_ENDPOINT: &str = "https://release.mnema.day/preview/latest.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AppUpdateChannel {
    Stable,
    Preview,
}

impl Default for AppUpdateChannel {
    fn default() -> Self {
        Self::Stable
    }
}

impl AppUpdateChannel {
    pub fn endpoint(self) -> &'static str {
        match self {
            Self::Stable => STABLE_UPDATE_ENDPOINT,
            Self::Preview => PREVIEW_UPDATE_ENDPOINT,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateSettings {
    pub channel: AppUpdateChannel,
    /// Remote version the out-of-window notice last opened the update window
    /// for. Persisted — unlike the in-window `notified_version` — because only a
    /// renewal ends that state, so an in-memory gate would reopen it every launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notified_out_of_window_version: Option<String>,
}

impl Default for AppUpdateSettings {
    fn default() -> Self {
        Self {
            channel: AppUpdateChannel::Stable,
            notified_out_of_window_version: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct AppUpdateSettingsRuntime {
    settings: Option<AppUpdateSettings>,
}

/// Registered on the `AppCore` by the host.
pub type AppUpdateSettingsState = Mutex<AppUpdateSettingsRuntime>;

fn settings_file_path(core: &AppCore) -> PathBuf {
    core.paths().config.join(APP_UPDATE_SETTINGS_FILE_NAME)
}

pub fn load_app_update_settings_from_path(path: &Path) -> AppUpdateSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<AppUpdateSettings>(&raw).ok())
        .unwrap_or_default()
}

pub fn current_settings(core: &AppCore) -> AppUpdateSettings {
    let state = core.state::<AppUpdateSettingsState>();
    let mut runtime = state.lock().expect("app update settings state poisoned");
    if let Some(settings) = runtime.settings.clone() {
        return settings;
    }

    let settings = load_app_update_settings_from_path(&settings_file_path(core));
    runtime.settings = Some(settings.clone());
    settings
}

/// Logs the update channel loaded at startup.
pub fn initialize(core: &AppCore) {
    let settings = current_settings(core);
    crate::debug_log::log_info(format!(
        "loaded app update settings (channel={:?}, endpoint={})",
        settings.channel,
        settings.channel.endpoint()
    ));
}

pub fn persist_settings(
    core: &AppCore,
    settings: AppUpdateSettings,
) -> Result<AppUpdateSettings, AppUpdateError> {
    let path = settings_file_path(core);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppUpdateError::new(
                AppUpdateErrorKind::Unknown,
                format!("Failed to create update settings directory: {error}"),
            )
        })?;
    }

    let serialized = serde_json::to_string_pretty(&settings).map_err(|error| {
        AppUpdateError::new(
            AppUpdateErrorKind::Unknown,
            format!("Failed to serialize update settings: {error}"),
        )
    })?;
    std::fs::write(path, serialized).map_err(|error| {
        AppUpdateError::new(
            AppUpdateErrorKind::Unknown,
            format!("Failed to persist update settings: {error}"),
        )
    })?;

    let state = core.state::<AppUpdateSettingsState>();
    let mut runtime = state.lock().expect("app update settings state poisoned");
    runtime.settings = Some(settings.clone());
    Ok(settings)
}

#[cfg(test)]
pub fn channel_endpoint(channel: AppUpdateChannel) -> &'static str {
    channel.endpoint()
}

#[cfg(test)]
pub fn channel_requires_additional_per_install_state(_channel: AppUpdateChannel) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_loads_stable_when_no_config_exists() {
        let dir = tempfile::tempdir().expect("temp dir should be created");
        let settings = load_app_update_settings_from_path(&dir.path().join("missing.json"));

        assert_eq!(settings.channel, AppUpdateChannel::Stable);
    }

    #[test]
    fn preview_channel_persists_and_reloads() {
        let dir = tempfile::tempdir().expect("temp dir should be created");
        let path = dir.path().join("app-update-settings.json");
        std::fs::write(
            &path,
            serde_json::to_string(&AppUpdateSettings {
                channel: AppUpdateChannel::Preview,
                ..AppUpdateSettings::default()
            })
            .expect("settings should serialize"),
        )
        .expect("settings should write");

        let settings = load_app_update_settings_from_path(&path);

        assert_eq!(settings.channel, AppUpdateChannel::Preview);
    }

    #[test]
    fn invalid_settings_file_falls_back_to_stable() {
        let dir = tempfile::tempdir().expect("temp dir should be created");
        let path = dir.path().join("app-update-settings.json");
        std::fs::write(&path, "{not-json").expect("invalid settings should write");

        let settings = load_app_update_settings_from_path(&path);

        assert_eq!(settings.channel, AppUpdateChannel::Stable);
    }

    #[test]
    fn channel_endpoint_selection_returns_stable_and_preview_urls() {
        assert_eq!(
            channel_endpoint(AppUpdateChannel::Stable),
            STABLE_UPDATE_ENDPOINT
        );
        assert_eq!(
            channel_endpoint(AppUpdateChannel::Preview),
            PREVIEW_UPDATE_ENDPOINT
        );
    }

    #[test]
    fn preview_channel_requires_no_extra_per_install_state_after_opt_in() {
        assert!(!channel_requires_additional_per_install_state(
            AppUpdateChannel::Preview
        ));
    }
}
