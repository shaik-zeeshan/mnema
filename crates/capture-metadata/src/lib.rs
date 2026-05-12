use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use url::Url;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebsiteRule {
    pub id: String,
    pub enabled: bool,
    pub pattern: String,
    pub host: Option<String>,
    pub include_subdomains: bool,
    pub path_prefix: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserTitleRuleMatchType {
    Substring,
    Regex,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTitleRule {
    pub id: String,
    pub enabled: bool,
    pub match_type: BrowserTitleRuleMatchType,
    pub pattern: String,
}

pub fn default_private_browser_exclusion_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrivacySettings {
    #[serde(default)]
    pub excluded_apps: Vec<ExcludedAppEntry>,
    #[serde(default)]
    pub excluded_website_rules: Vec<WebsiteRule>,
    #[serde(default)]
    pub browser_title_rules: Vec<BrowserTitleRule>,
    #[serde(default = "default_private_browser_exclusion_enabled")]
    pub private_browser_exclusion_enabled: bool,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            excluded_apps: Vec::new(),
            excluded_website_rules: Vec::new(),
            browser_title_rules: Vec::new(),
            private_browser_exclusion_enabled: default_private_browser_exclusion_enabled(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FrameMetadataSnapshot {
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub browser_url: Option<String>,
    pub display_id: Option<u32>,
    pub metadata_redaction_reason: Option<String>,
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
    pub active_url: Option<String>,
    pub visible_browser_windows: Vec<BrowserWindowContext>,
    pub private_browser_window_id: Option<u32>,
    pub private_browser_ambiguous_bundle_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserWindowContext {
    pub window_id: u32,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyFilterDecision {
    pub excluded_bundle_ids: Vec<String>,
    pub excluded_window_ids: Vec<u32>,
    pub matched_rule_ids: Vec<String>,
    pub metadata_redaction_reason: Option<String>,
    pub privacy_filter_applied: bool,
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

pub fn parse_website_rule(id: impl Into<String>, enabled: bool, pattern: &str) -> WebsiteRule {
    let trimmed = pattern.trim();
    let parse_target = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let parsed = Url::parse(&parse_target).ok();
    let raw_host = parsed
        .as_ref()
        .and_then(Url::host_str)
        .map(str::to_ascii_lowercase);
    let include_subdomains = raw_host.as_deref().is_some_and(|host| host.starts_with("*."));
    let host = raw_host.map(|host| host.trim_start_matches("*.").to_string());
    let path_prefix = parsed
        .as_ref()
        .map(Url::path)
        .filter(|path| !path.is_empty() && *path != "/")
        .map(str::to_string);
    WebsiteRule {
        id: id.into(),
        enabled,
        pattern: trimmed.to_string(),
        host,
        include_subdomains,
        path_prefix,
        port: parsed.as_ref().and_then(Url::port),
    }
}

pub fn website_rule_matches(rule: &WebsiteRule, raw_url: &str) -> bool {
    if !rule.enabled {
        return false;
    }
    if rule.host.is_none() {
        let parsed = parse_website_rule(rule.id.clone(), rule.enabled, &rule.pattern);
        return website_rule_matches(&parsed, raw_url);
    }
    let Some(rule_host) = rule.host.as_deref().map(str::to_ascii_lowercase) else {
        return false;
    };
    let Ok(url) = Url::parse(raw_url) else {
        return false;
    };
    let Some(url_host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    let host_matches = url_host == rule_host
        || (rule.include_subdomains && url_host.ends_with(&format!(".{rule_host}")));
    if !host_matches {
        return false;
    }
    if let Some(port) = rule.port {
        if url.port_or_known_default() != Some(port) && url.port() != Some(port) {
            return false;
        }
    }
    if let Some(prefix) = rule.path_prefix.as_deref() {
        return url.path().starts_with(prefix);
    }
    true
}

pub fn title_rule_is_valid(rule: &BrowserTitleRule) -> bool {
    match rule.match_type {
        BrowserTitleRuleMatchType::Substring => true,
        BrowserTitleRuleMatchType::Regex => RegexBuilder::new(&rule.pattern)
            .case_insensitive(true)
            .build()
            .is_ok(),
    }
}

pub fn title_rule_matches(rule: &BrowserTitleRule, title: &str) -> bool {
    if !rule.enabled {
        return false;
    }
    match rule.match_type {
        BrowserTitleRuleMatchType::Substring => title
            .to_ascii_lowercase()
            .contains(&rule.pattern.to_ascii_lowercase()),
        BrowserTitleRuleMatchType::Regex => RegexBuilder::new(&rule.pattern)
            .case_insensitive(true)
            .build()
            .map(|regex| regex.is_match(title))
            .unwrap_or(false),
    }
}

pub fn evaluate_privacy(settings: &PrivacySettings, context: &MetadataContext) -> PrivacyFilterDecision {
    let mut bundle_ids = BTreeSet::new();
    let mut window_ids = BTreeSet::new();
    let mut rule_ids = BTreeSet::new();
    let mut redaction_reason = None;

    for app in &settings.excluded_apps {
        if app.enabled && !app.bundle_id.trim().is_empty() {
            bundle_ids.insert(app.bundle_id.trim().to_string());
            rule_ids.insert(app.id.clone());
            redaction_reason.get_or_insert_with(|| "excluded_app".to_string());
        }
    }

    if let (Some(active_bundle), Some(active_url)) = (&context.active_bundle_id, &context.active_url) {
        for rule in &settings.excluded_website_rules {
            if website_rule_matches(rule, active_url) {
                bundle_ids.insert(active_bundle.clone());
                rule_ids.insert(rule.id.clone());
                redaction_reason.get_or_insert_with(|| "website_rule".to_string());
            }
        }
    }

    for rule in &settings.browser_title_rules {
        for window in &context.visible_browser_windows {
            if title_rule_matches(rule, &window.title) {
                window_ids.insert(window.window_id);
                rule_ids.insert(rule.id.clone());
                redaction_reason.get_or_insert_with(|| "title_rule".to_string());
            }
        }
    }

    if settings.private_browser_exclusion_enabled {
        if let Some(window_id) = context.private_browser_window_id {
            window_ids.insert(window_id);
            redaction_reason.get_or_insert_with(|| "private_browser".to_string());
        }
    }

    PrivacyFilterDecision {
        privacy_filter_applied: !bundle_ids.is_empty() || !window_ids.is_empty(),
        excluded_bundle_ids: bundle_ids.into_iter().collect(),
        excluded_window_ids: window_ids.into_iter().collect(),
        matched_rule_ids: rule_ids.into_iter().collect(),
        metadata_redaction_reason: redaction_reason,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivacyFilterKey {
    pub display_id: u32,
    pub bundle_ids: Vec<String>,
    pub window_ids: Vec<u32>,
}

impl PrivacyFilterKey {
    pub fn new(display_id: u32, mut bundle_ids: Vec<String>, mut window_ids: Vec<u32>) -> Self {
        bundle_ids.sort();
        bundle_ids.dedup();
        window_ids.sort_unstable();
        window_ids.dedup();
        Self {
            display_id,
            bundle_ids,
            window_ids,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_urls() {
        assert_eq!(
            sanitize_url("https://Example.com:8443/a/b?q=1#frag", BrowserUrlMode::Sanitized),
            Some("https://example.com:8443/a/b".to_string())
        );
        assert_eq!(
            sanitize_url("file:///Users/me/private.txt", BrowserUrlMode::Sanitized),
            Some("file://[local-file]".to_string())
        );
        assert_eq!(
            sanitize_url("http://localhost:3000/a?token=1", BrowserUrlMode::Sanitized),
            Some("http://localhost:3000/a".to_string())
        );
    }

    #[test]
    fn matches_website_rules() {
        let wildcard = parse_website_rule("w", true, "*.example.com/private");
        assert!(website_rule_matches(&wildcard, "https://a.example.com/private/x"));
        assert!(website_rule_matches(&wildcard, "https://example.com/private/x"));
        let port = parse_website_rule("p", true, "localhost:5173/app");
        assert!(website_rule_matches(&port, "http://localhost:5173/app/%7Bsecret%7D"));
        assert!(!website_rule_matches(&port, "http://localhost:5174/app"));
        let unnormalized = WebsiteRule {
            id: "raw".into(),
            enabled: true,
            pattern: "example.com/private".into(),
            host: None,
            include_subdomains: false,
            path_prefix: None,
            port: None,
        };
        assert!(website_rule_matches(
            &unnormalized,
            "https://example.com/private/page"
        ));
    }

    #[test]
    fn matches_titles_and_rejects_enabled_invalid_regex() {
        let substring = BrowserTitleRule {
            id: "s".into(),
            enabled: true,
            match_type: BrowserTitleRuleMatchType::Substring,
            pattern: "Bank".into(),
        };
        assert!(title_rule_matches(&substring, "personal bank"));
        let invalid = BrowserTitleRule {
            id: "r".into(),
            enabled: true,
            match_type: BrowserTitleRuleMatchType::Regex,
            pattern: "(".into(),
        };
        assert!(!title_rule_is_valid(&invalid));
        assert!(!title_rule_matches(&invalid, "anything"));
    }

    #[test]
    fn defaults_private_browser_exclusion_on() {
        assert!(PrivacySettings::default().private_browser_exclusion_enabled);

        let parsed: PrivacySettings =
            serde_json::from_str(r#"{"excludedApps":[]}"#).expect("privacy settings should load");
        assert!(parsed.private_browser_exclusion_enabled);
    }

    #[test]
    fn computes_effective_privacy_union() {
        let settings = PrivacySettings {
            excluded_apps: vec![ExcludedAppEntry {
                id: "app".into(),
                enabled: true,
                bundle_id: "com.secret".into(),
                display_name: "Secret".into(),
            }],
            excluded_website_rules: vec![parse_website_rule("site", true, "example.com/private")],
            browser_title_rules: vec![BrowserTitleRule {
                id: "title".into(),
                enabled: true,
                match_type: BrowserTitleRuleMatchType::Substring,
                pattern: "vault".into(),
            }],
            private_browser_exclusion_enabled: true,
        };
        let decision = evaluate_privacy(
            &settings,
            &MetadataContext {
                active_bundle_id: Some("com.browser".into()),
                active_url: Some("https://example.com/private".into()),
                visible_browser_windows: vec![BrowserWindowContext {
                    window_id: 7,
                    title: "Vault".into(),
                }],
                private_browser_window_id: Some(9),
                private_browser_ambiguous_bundle_id: Some("com.browser".into()),
            },
        );
        assert_eq!(decision.excluded_bundle_ids, vec!["com.browser", "com.secret"]);
        assert_eq!(decision.excluded_window_ids, vec![7, 9]);
        assert_eq!(decision.metadata_redaction_reason.as_deref(), Some("excluded_app"));
    }

    #[test]
    fn private_browser_excludes_matched_window_not_ambiguous_bundle() {
        let settings = PrivacySettings {
            private_browser_exclusion_enabled: true,
            ..PrivacySettings::default()
        };
        let decision = evaluate_privacy(
            &settings,
            &MetadataContext {
                private_browser_window_id: Some(9),
                private_browser_ambiguous_bundle_id: Some("com.browser".into()),
                ..MetadataContext::default()
            },
        );

        assert!(decision.excluded_bundle_ids.is_empty());
        assert_eq!(decision.excluded_window_ids, vec![9]);
        assert_eq!(
            decision.metadata_redaction_reason.as_deref(),
            Some("private_browser")
        );
    }

    #[test]
    fn filter_key_sorts_and_dedupes() {
        let key = PrivacyFilterKey::new(1, vec!["b".into(), "a".into(), "a".into()], vec![3, 2, 3]);
        assert_eq!(key.bundle_ids, vec!["a", "b"]);
        assert_eq!(key.window_ids, vec![2, 3]);
    }
}
