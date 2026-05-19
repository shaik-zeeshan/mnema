use capture_types::RecordingSettings;
use serde::Serialize;

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

fn exclusion_state(settings: &RecordingSettings, bundle_id: &str) -> RecommendedExclusionState {
    let canonical = crate::native_capture::settings::canonicalize_app_bundle_id(bundle_id);
    match settings.privacy.excluded_apps.iter().find(|app| {
        crate::native_capture::settings::canonicalize_app_bundle_id(&app.bundle_id) == canonical
    }) {
        Some(entry) if entry.enabled => RecommendedExclusionState::Enabled,
        Some(_) => RecommendedExclusionState::Disabled,
        None => RecommendedExclusionState::Missing,
    }
}

fn prompt_not_closed(app: &tauri::AppHandle) -> bool {
    crate::one_time_prompts::current_state(app)
        .prompts
        .get(RECOMMENDED_EXCLUSIONS_PROMPT_ID)
        .map(|record| record.dismissed_at.is_none() && record.completed_at.is_none())
        .unwrap_or(true)
}

#[tauri::command]
pub async fn get_sensitive_capture_recommendations(
    app: tauri::AppHandle,
) -> Result<SensitiveCaptureRecommendationsResponse, String> {
    let settings = crate::native_capture::current_recording_settings_from_app_handle(&app);
    let candidates = crate::native_capture::list_privacy_app_candidates().await?;

    let recommended_apps = SENSITIVE_APP_CATALOG
        .iter()
        .map(|entry| {
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.bundle_id == entry.bundle_id);
            RecommendedAppExclusionDto {
                bundle_id: entry.bundle_id.to_string(),
                display_name: candidate
                    .map(|candidate| candidate.display_name.clone())
                    .unwrap_or_else(|| entry.display_name.to_string()),
                category: entry.category.as_str().to_string(),
                category_label: entry.category.label().to_string(),
                running: candidate.map(|candidate| candidate.running).unwrap_or(false),
                icon_path: candidate.and_then(|candidate| candidate.icon_path.clone()),
                exclusion_state: exclusion_state(&settings, entry.bundle_id),
            }
        })
        .collect::<Vec<_>>();

    let browser_disclosures = capture_metadata::KNOWN_BROWSER_APPS
        .iter()
        .map(|browser| {
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.bundle_id == browser.bundle_id);
            BrowserDisclosureAppDto {
                bundle_id: browser.bundle_id.to_string(),
                display_name: candidate
                    .map(|candidate| candidate.display_name.clone())
                    .unwrap_or_else(|| browser.display_name.to_string()),
                running: candidate.map(|candidate| candidate.running).unwrap_or(false),
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
        actionable_recommendation_count > 0 && prompt_not_closed(&app);

    Ok(SensitiveCaptureRecommendationsResponse {
        prompt_id: RECOMMENDED_EXCLUSIONS_PROMPT_ID.to_string(),
        recommended_apps,
        actionable_recommendation_count,
        should_show_existing_user_prompt,
        browser_disclosures,
    })
}
