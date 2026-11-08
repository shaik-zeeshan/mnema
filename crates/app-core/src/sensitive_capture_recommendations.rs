//! The Sensitive Capture Protection recommendations: which password-manager and
//! browser apps to suggest excluding, given the installed/running app list.

use capture_types::RecordingSettings;
use serde::Serialize;

use crate::privacy_redaction_sources::PrivacyAppCandidate;
use crate::recording_settings::{current_recording_settings, RecordingSettingsState};
use crate::AppCore;

pub const RECOMMENDED_EXCLUSIONS_PROMPT_ID: &str =
    "sensitive_capture_protection_v1_recommended_exclusions";

pub struct SensitiveAppCatalogEntry {
    pub bundle_id: &'static str,
    pub display_name: &'static str,
    pub category: SensitiveAppCategory,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum SensitiveAppCategory {
    PasswordManager,
    Authenticator,
    ApplePasswords,
    Banking,
}

impl SensitiveAppCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::PasswordManager => "password_manager",
            Self::Authenticator => "authenticator",
            Self::ApplePasswords => "apple_passwords",
            Self::Banking => "banking",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::PasswordManager => "Password manager",
            Self::Authenticator => "Authenticator",
            Self::ApplePasswords => "Apple passwords",
            Self::Banking => "Banking",
        }
    }
}

const SENSITIVE_APP_CATALOG: &[SensitiveAppCatalogEntry] = &[
    SensitiveAppCatalogEntry {
        bundle_id: "com.apple.Passwords",
        display_name: "Apple Passwords",
        category: SensitiveAppCategory::ApplePasswords,
    },
    SensitiveAppCatalogEntry {
        bundle_id: "com.apple.keychainaccess",
        display_name: "Keychain Access",
        category: SensitiveAppCategory::ApplePasswords,
    },
    SensitiveAppCatalogEntry {
        bundle_id: "com.1password.1password",
        display_name: "1Password",
        category: SensitiveAppCategory::PasswordManager,
    },
    SensitiveAppCatalogEntry {
        bundle_id: "com.bitwarden.desktop",
        display_name: "Bitwarden",
        category: SensitiveAppCategory::PasswordManager,
    },
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecommendedExclusionState {
    Missing,
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedAppExclusionDto {
    pub bundle_id: String,
    pub display_name: String,
    pub category: String,
    pub category_label: String,
    pub running: bool,
    pub icon_path: Option<String>,
    pub exclusion_state: RecommendedExclusionState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDisclosureAppDto {
    pub bundle_id: String,
    pub display_name: String,
    pub running: bool,
    pub icon_path: Option<String>,
    pub exclusion_state: RecommendedExclusionState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SensitiveCaptureRecommendationsResponse {
    pub prompt_id: String,
    pub recommended_apps: Vec<RecommendedAppExclusionDto>,
    pub actionable_recommendation_count: usize,
    pub should_show_existing_user_prompt: bool,
    pub browser_disclosures: Vec<BrowserDisclosureAppDto>,
}

fn same_bundle_id(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn matching_candidate<'a>(
    candidates: &'a [PrivacyAppCandidate],
    bundle_id: &str,
) -> Option<&'a PrivacyAppCandidate> {
    candidates
        .iter()
        .find(|candidate| same_bundle_id(&candidate.bundle_id, bundle_id))
}

fn exclusion_state(settings: &RecordingSettings, bundle_id: &str) -> RecommendedExclusionState {
    match settings
        .privacy
        .excluded_apps
        .iter()
        .find(|app| same_bundle_id(&app.bundle_id, bundle_id))
    {
        Some(entry) if entry.enabled => RecommendedExclusionState::Enabled,
        Some(_) => RecommendedExclusionState::Disabled,
        None => RecommendedExclusionState::Missing,
    }
}

fn prompt_not_closed(core: &AppCore) -> bool {
    crate::one_time_prompts::current_state(core)
        .prompts
        .get(RECOMMENDED_EXCLUSIONS_PROMPT_ID)
        .map(|record| record.dismissed_at.is_none() && record.completed_at.is_none())
        .unwrap_or(true)
}

/// `candidates` is the installed/running app list (`list_privacy_app_candidates`).
pub fn get_sensitive_capture_recommendations(
    core: &AppCore,
    candidates: &[PrivacyAppCandidate],
) -> SensitiveCaptureRecommendationsResponse {
    let settings = current_recording_settings(&core.state::<RecordingSettingsState>());

    let recommended_apps = SENSITIVE_APP_CATALOG
        .iter()
        .map(|entry| {
            let candidate = matching_candidate(candidates, entry.bundle_id);
            RecommendedAppExclusionDto {
                bundle_id: entry.bundle_id.to_string(),
                display_name: candidate
                    .map(|candidate| candidate.display_name.clone())
                    .unwrap_or_else(|| entry.display_name.to_string()),
                category: entry.category.as_str().to_string(),
                category_label: entry.category.label().to_string(),
                running: candidate
                    .map(|candidate| candidate.running)
                    .unwrap_or(false),
                icon_path: candidate.and_then(|candidate| candidate.icon_path.clone()),
                exclusion_state: exclusion_state(&settings, entry.bundle_id),
            }
        })
        .collect::<Vec<_>>();

    let browser_disclosures = capture_metadata::KNOWN_BROWSER_APPS
        .iter()
        .map(|browser| {
            let candidate = matching_candidate(candidates, browser.bundle_id);
            BrowserDisclosureAppDto {
                bundle_id: browser.bundle_id.to_string(),
                display_name: candidate
                    .map(|candidate| candidate.display_name.clone())
                    .unwrap_or_else(|| browser.display_name.to_string()),
                running: candidate
                    .map(|candidate| candidate.running)
                    .unwrap_or(false),
                icon_path: candidate.and_then(|candidate| candidate.icon_path.clone()),
                exclusion_state: exclusion_state(&settings, browser.bundle_id),
            }
        })
        .collect::<Vec<_>>();

    let actionable_recommendation_count = recommended_apps
        .iter()
        .filter(|app| app.exclusion_state != RecommendedExclusionState::Enabled)
        .count();
    let should_show_existing_user_prompt =
        actionable_recommendation_count > 0 && prompt_not_closed(core);

    SensitiveCaptureRecommendationsResponse {
        prompt_id: RECOMMENDED_EXCLUSIONS_PROMPT_ID.to_string(),
        recommended_apps,
        actionable_recommendation_count,
        should_show_existing_user_prompt,
        browser_disclosures,
    }
}

/// The command's whole body: today's app list, then the recommendations.
pub async fn sensitive_capture_recommendations_now(
    core: &AppCore,
) -> Result<SensitiveCaptureRecommendationsResponse, String> {
    let candidates = crate::native_capture::app_candidates::list_privacy_app_candidates().await?;
    Ok(get_sensitive_capture_recommendations(core, &candidates))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn privacy_app_candidate(bundle_id: &str) -> PrivacyAppCandidate {
        PrivacyAppCandidate {
            bundle_id: bundle_id.to_string(),
            display_name: "Candidate".to_string(),
            running: true,
            icon_path: Some("/tmp/icon.png".to_string()),
            bundle_path: None,
        }
    }

    #[test]
    fn recommendation_candidate_lookup_matches_bundle_ids_case_insensitively() {
        let candidates = vec![privacy_app_candidate("COM.APPLE.PASSWORDS")];

        let candidate = matching_candidate(&candidates, "com.apple.Passwords")
            .expect("candidate should match ignoring bundle ID case");

        assert_eq!(candidate.display_name, "Candidate");
        assert!(candidate.running);
        assert_eq!(candidate.icon_path.as_deref(), Some("/tmp/icon.png"));
    }

    #[test]
    fn recommendation_exclusion_state_matches_bundle_ids_case_insensitively() {
        let mut settings = crate::recording_settings::default_recording_settings();
        settings.privacy.excluded_apps = vec![capture_metadata::ExcludedAppEntry {
            id: "app-a".to_string(),
            enabled: false,
            bundle_id: "COM.APPLE.PASSWORDS".to_string(),
            display_name: "Apple Passwords".to_string(),
        }];

        assert_eq!(
            exclusion_state(&settings, "com.apple.Passwords"),
            RecommendedExclusionState::Disabled
        );
    }

    #[test]
    fn recommendations_read_core_settings_and_hide_the_prompt_once_dismissed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let core = crate::test_support::core_in(Default::default(), dir.path());
        core.manage(RecordingSettingsState::default());
        core.manage(crate::one_time_prompts::OneTimePromptStateStore::default());
        {
            let state = core.state::<RecordingSettingsState>();
            let mut runtime = state.lock().unwrap();
            runtime.settings.privacy.excluded_apps = vec![capture_metadata::ExcludedAppEntry {
                id: "app-a".to_string(),
                enabled: true,
                bundle_id: "com.1password.1password".to_string(),
                display_name: "1Password".to_string(),
            }];
        }
        let candidates = vec![privacy_app_candidate("com.bitwarden.desktop")];

        let response = get_sensitive_capture_recommendations(&core, &candidates);
        let bitwarden = response
            .recommended_apps
            .iter()
            .find(|app| app.bundle_id == "com.bitwarden.desktop")
            .expect("catalog app");
        assert_eq!(bitwarden.display_name, "Candidate");
        assert!(bitwarden.running);
        assert_eq!(response.actionable_recommendation_count, 3);
        assert!(response.should_show_existing_user_prompt);

        crate::one_time_prompts::dismiss_one_time_prompt(
            RECOMMENDED_EXCLUSIONS_PROMPT_ID.to_string(),
            &core,
        )
        .expect("dismiss");
        let response = get_sensitive_capture_recommendations(&core, &candidates);
        assert_eq!(response.actionable_recommendation_count, 3);
        assert!(!response.should_show_existing_user_prompt);
    }
}
