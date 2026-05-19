use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use url::Url;

pub const BROWSER_URL_METADATA_POLL_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserUrlMode {
    Off,
    Sanitized,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MetadataSettings {
    pub enabled: bool,
    pub browser_url_mode: BrowserUrlMode,
}

impl Default for MetadataSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            browser_url_mode: BrowserUrlMode::Sanitized,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExcludedAppEntry {
    pub id: String,
    pub enabled: bool,
    pub bundle_id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PrivacySettings {
    #[serde(default)]
    pub excluded_apps: Vec<ExcludedAppEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FrameMetadataSnapshot {
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    #[serde(default, skip_serializing)]
    pub window_id: Option<u32>,
    pub browser_url: Option<String>,
    pub display_id: Option<u32>,
    pub metadata_redaction_reason: Option<String>,
    #[serde(default)]
    pub metadata_redaction_source_id: Option<String>,
}

impl FrameMetadataSnapshot {
    pub fn normalized_json(&self) -> String {
        serde_json::to_string(self).expect("metadata snapshot should serialize")
    }

    pub fn normalized_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.normalized_json());
        format!("{:x}", hasher.finalize())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MetadataContext {
    pub active_bundle_id: Option<String>,
    pub active_window_id: Option<u32>,
    pub active_window_title: Option<String>,
    pub active_privacy_window_id: Option<u32>,
    pub active_url: Option<String>,
    pub visible_windows: Vec<WindowContext>,
    pub private_browser_window_ids: Vec<u32>,
    pub private_browser_ambiguous_bundle_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowContext {
    pub window_id: u32,
    pub bundle_id: Option<String>,
    pub owner_pid: Option<i32>,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyFilterDecision {
    pub excluded_bundle_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub excluded_bundle_reasons: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub excluded_bundle_source_ids: BTreeMap<String, String>,
    pub matched_rule_ids: Vec<String>,
    pub metadata_redaction_reason: Option<String>,
    pub privacy_filter_applied: bool,
}

#[derive(Debug, Clone, Default)]
pub struct BrowserUrlProbeCache {
    bundle_id: Option<String>,
    raw_url: Option<String>,
    probed_at: Option<Instant>,
}

impl BrowserUrlProbeCache {
    pub fn cached_url_for(&self, bundle_id: &str, now: Instant) -> Option<Option<String>> {
        if self.bundle_id.as_deref() != Some(bundle_id) {
            return None;
        }
        let probed_at = self.probed_at?;
        if now.saturating_duration_since(probed_at) >= BROWSER_URL_METADATA_POLL_INTERVAL {
            return None;
        }
        Some(self.raw_url.clone())
    }

    pub fn from_probe(bundle_id: Option<String>, raw_url: Option<String>, now: Instant) -> Self {
        Self {
            bundle_id,
            raw_url,
            probed_at: Some(now),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeActiveWindowSnapshot {
    pub bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub pid: Option<i32>,
    pub window_id: Option<u32>,
    pub window_title: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawWindowInfo {
    pub owner_pid: i32,
    pub window_id: u32,
    pub layer: i64,
    pub width: f64,
    pub height: f64,
    pub title: Option<String>,
}

pub fn select_frontmost_pid_window<'a>(
    windows: &'a [RawWindowInfo],
    frontmost_pid: i32,
) -> Option<&'a RawWindowInfo> {
    windows.iter().find(|window| {
        window.owner_pid == frontmost_pid
            && window.layer == 0
            && window.width > 0.0
            && window.height > 0.0
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MetadataCollectionPlan {
    pub collect_active_window: bool,
    pub collect_browser_url_for_metadata: bool,
    pub collect_browser_url_for_privacy: bool,
    pub collect_visible_windows: bool,
}

pub fn metadata_collection_plan(metadata: &MetadataSettings) -> MetadataCollectionPlan {
    MetadataCollectionPlan {
        collect_active_window: metadata.enabled,
        collect_browser_url_for_metadata: metadata.enabled
            && metadata.browser_url_mode != BrowserUrlMode::Off,
        collect_browser_url_for_privacy: false,
        collect_visible_windows: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserAppDescriptor {
    pub bundle_id: &'static str,
    pub display_name: &'static str,
    pub url_script_app_name: Option<&'static str>,
}

pub const KNOWN_BROWSER_APPS: &[BrowserAppDescriptor] = &[
    BrowserAppDescriptor {
        bundle_id: "com.apple.Safari",
        display_name: "Safari",
        url_script_app_name: Some("Safari"),
    },
    BrowserAppDescriptor {
        bundle_id: "com.google.Chrome",
        display_name: "Google Chrome",
        url_script_app_name: Some("Google Chrome"),
    },
    BrowserAppDescriptor {
        bundle_id: "com.google.Chrome.canary",
        display_name: "Google Chrome Canary",
        url_script_app_name: Some("Google Chrome Canary"),
    },
    BrowserAppDescriptor {
        bundle_id: "com.microsoft.edgemac",
        display_name: "Microsoft Edge",
        url_script_app_name: Some("Microsoft Edge"),
    },
    BrowserAppDescriptor {
        bundle_id: "org.mozilla.firefox",
        display_name: "Firefox",
        url_script_app_name: None,
    },
    BrowserAppDescriptor {
        bundle_id: "com.brave.Browser",
        display_name: "Brave Browser",
        url_script_app_name: Some("Brave Browser"),
    },
    BrowserAppDescriptor {
        bundle_id: "company.thebrowser.Browser",
        display_name: "Arc",
        url_script_app_name: Some("Arc"),
    },
    BrowserAppDescriptor {
        bundle_id: "net.imput.helium",
        display_name: "Helium",
        url_script_app_name: Some("Helium"),
    },
];

pub fn known_browser_app(bundle_id: &str) -> Option<&'static BrowserAppDescriptor> {
    KNOWN_BROWSER_APPS
        .iter()
        .find(|browser| browser.bundle_id == bundle_id)
}

pub fn is_known_browser_bundle(bundle_id: &str) -> bool {
    known_browser_app(bundle_id).is_some()
}

pub fn browser_url_script_app_name(bundle_id: &str) -> Option<&'static str> {
    known_browser_app(bundle_id).and_then(|browser| browser.url_script_app_name)
}

pub fn browser_url_metadata_supported(bundle_id: &str) -> bool {
    browser_url_script_app_name(bundle_id).is_some()
}

pub const REDACTION_REASON_EXCLUDED_APP: &str = "excluded_app";

pub fn evaluate_privacy(
    settings: &PrivacySettings,
    _context: &MetadataContext,
) -> PrivacyFilterDecision {
    let mut bundle_source_ids = BTreeMap::new();
    let mut bundle_ids = Vec::new();
    let mut matched_rule_ids = Vec::new();

    for app in &settings.excluded_apps {
        let bundle_id = app.bundle_id.trim();
        if !app.enabled
            || bundle_id.is_empty()
            || bundle_ids.iter().any(|existing| existing == bundle_id)
        {
            continue;
        }
        let bundle_id = bundle_id.to_string();
        bundle_source_ids.insert(bundle_id.clone(), app.id.clone());
        bundle_ids.push(bundle_id);
        matched_rule_ids.push(app.id.clone());
    }

    let excluded_bundle_reasons = bundle_ids
        .iter()
        .map(|bundle_id| (bundle_id.clone(), REDACTION_REASON_EXCLUDED_APP.to_string()))
        .collect();

    PrivacyFilterDecision {
        privacy_filter_applied: !bundle_ids.is_empty(),
        excluded_bundle_ids: bundle_ids,
        excluded_bundle_reasons,
        excluded_bundle_source_ids: bundle_source_ids,
        matched_rule_ids,
        metadata_redaction_reason: None,
    }
}

pub fn sanitize_url(raw_url: &str, mode: BrowserUrlMode) -> Option<String> {
    if mode == BrowserUrlMode::Off {
        return None;
    }
    let parsed = Url::parse(raw_url).ok()?;
    if parsed.scheme() == "file" && mode != BrowserUrlMode::Full {
        return Some("file://[local-file]".to_string());
    }
    if mode == BrowserUrlMode::Full {
        return Some(parsed.to_string());
    }
    let mut sanitized = parsed;
    sanitized.set_query(None);
    sanitized.set_fragment(None);
    Some(sanitized.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivacyFilterKey {
    pub display_id: u32,
    pub bundle_ids: Vec<String>,
}

impl PrivacyFilterKey {
    pub fn new(display_id: u32, mut bundle_ids: Vec<String>) -> Self {
        bundle_ids.sort();
        bundle_ids.dedup();
        Self {
            display_id,
            bundle_ids,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_urls() {
        assert_eq!(
            sanitize_url(
                "https://Example.com:8443/a/b?q=1#frag",
                BrowserUrlMode::Sanitized
            ),
            Some("https://example.com:8443/a/b".to_string())
        );
        assert_eq!(
            sanitize_url("file:///Users/me/private.txt", BrowserUrlMode::Sanitized),
            Some("file://[local-file]".to_string())
        );
    }

    #[test]
    fn default_privacy_settings_are_app_only() {
        assert!(PrivacySettings::default().excluded_apps.is_empty());
        let parsed: PrivacySettings = serde_json::from_str(
            r#"{"excludedApps":[],"excludedWebsiteRules":[{"id":"site","enabled":true,"pattern":"example.com"}],"browserTitleRules":[{"id":"title","enabled":true,"matchType":"substring","pattern":"secret"}],"privateBrowserExclusionEnabled":true}"#,
        )
        .expect("legacy privacy settings should be ignored");
        assert!(parsed.excluded_apps.is_empty());
    }

    #[test]
    fn metadata_collection_plan_is_metadata_only() {
        let metadata = MetadataSettings {
            enabled: false,
            browser_url_mode: BrowserUrlMode::Full,
        };
        assert_eq!(
            metadata_collection_plan(&metadata),
            MetadataCollectionPlan::default()
        );

        let metadata = MetadataSettings {
            enabled: true,
            browser_url_mode: BrowserUrlMode::Full,
        };
        let plan = metadata_collection_plan(&metadata);
        assert!(plan.collect_active_window);
        assert!(plan.collect_browser_url_for_metadata);
        assert!(!plan.collect_browser_url_for_privacy);
        assert!(!plan.collect_visible_windows);
    }

    #[test]
    fn evaluate_privacy_is_app_only_and_dedupes_bundle_ids() {
        let settings = PrivacySettings {
            excluded_apps: vec![
                ExcludedAppEntry {
                    id: "a".to_string(),
                    enabled: true,
                    bundle_id: " com.example.Secret ".to_string(),
                    display_name: "Secret".to_string(),
                },
                ExcludedAppEntry {
                    id: "b".to_string(),
                    enabled: true,
                    bundle_id: "com.example.Secret".to_string(),
                    display_name: "Secret Again".to_string(),
                },
            ],
        };
        let context = MetadataContext {
            active_bundle_id: Some("com.apple.Safari".to_string()),
            active_window_id: Some(42),
            active_window_title: Some("Private Browsing".to_string()),
            active_privacy_window_id: Some(42),
            active_url: Some("https://example.com/private".to_string()),
            visible_windows: vec![WindowContext {
                window_id: 42,
                bundle_id: Some("com.apple.Safari".to_string()),
                owner_pid: Some(1),
                title: "Private Browsing".to_string(),
            }],
            private_browser_window_ids: vec![42],
            private_browser_ambiguous_bundle_id: Some("com.apple.Safari".to_string()),
        };

        let decision = evaluate_privacy(&settings, &context);

        assert_eq!(decision.excluded_bundle_ids, vec!["com.example.Secret"]);
        assert_eq!(decision.matched_rule_ids, vec!["a"]);
        assert!(decision.privacy_filter_applied);
        assert!(decision.metadata_redaction_reason.is_none());
    }
}
