use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
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
    pub excluded_window_ids: Vec<u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub excluded_bundle_reasons: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub excluded_bundle_source_ids: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub excluded_window_reasons: BTreeMap<u32, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub excluded_window_source_ids: BTreeMap<u32, String>,
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

pub fn metadata_collection_plan(
    metadata: &MetadataSettings,
    privacy: &PrivacySettings,
) -> MetadataCollectionPlan {
    let collect_browser_url_for_privacy = has_enabled_website_rules(privacy);
    let collect_visible_windows = has_enabled_browser_title_rules(privacy)
        || has_enabled_website_rules(privacy)
        || privacy.private_browser_exclusion_enabled;
    let collect_active_window_for_privacy = collect_browser_url_for_privacy
        || has_enabled_browser_title_rules(privacy)
        || privacy.private_browser_exclusion_enabled;

    MetadataCollectionPlan {
        collect_active_window: metadata.enabled || collect_active_window_for_privacy,
        collect_browser_url_for_metadata: metadata.enabled
            && metadata.browser_url_mode != BrowserUrlMode::Off,
        collect_browser_url_for_privacy,
        collect_visible_windows,
    }
}

pub fn has_enabled_website_rules(privacy: &PrivacySettings) -> bool {
    privacy
        .excluded_website_rules
        .iter()
        .any(|rule| rule.enabled)
}

pub fn has_enabled_browser_title_rules(privacy: &PrivacySettings) -> bool {
    privacy.browser_title_rules.iter().any(|rule| rule.enabled)
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

pub fn is_known_browser_window(window: &WindowContext) -> bool {
    window
        .bundle_id
        .as_deref()
        .is_some_and(is_known_browser_bundle)
}

pub fn resolve_private_browser_window_id(
    privacy: &PrivacySettings,
    visible_windows: &[WindowContext],
) -> Option<u32> {
    resolve_private_browser_window_ids(privacy, visible_windows)
        .into_iter()
        .next()
}

pub fn resolve_private_browser_window_ids(
    privacy: &PrivacySettings,
    visible_windows: &[WindowContext],
) -> Vec<u32> {
    if !privacy.private_browser_exclusion_enabled {
        return Vec::new();
    }

    visible_windows
        .iter()
        .filter(|window| is_known_browser_window(window) && is_private_browser_title(&window.title))
        .map(|window| window.window_id)
        .collect()
}

pub fn browser_url_script_app_name(bundle_id: &str) -> Option<&'static str> {
    known_browser_app(bundle_id).and_then(|browser| browser.url_script_app_name)
}

pub fn browser_url_metadata_supported(bundle_id: &str) -> bool {
    browser_url_script_app_name(bundle_id).is_some()
}

pub const REDACTION_REASON_EXCLUDED_APP: &str = "excluded_app";
pub const REDACTION_REASON_WEBSITE_RULE: &str = "website_rule";
pub const REDACTION_REASON_WEBSITE_RULE_URL_UNAVAILABLE: &str = "website_rule_url_unavailable";
pub const REDACTION_REASON_WEBSITE_RULE_UNVERIFIED_VISIBLE_BROWSER: &str =
    "website_rule_unverified_visible_browser";
pub const REDACTION_REASON_TITLE_RULE: &str = "title_rule";
pub const REDACTION_REASON_PRIVATE_BROWSER: &str = "private_browser";
pub const REDACTION_REASON_PRIVACY_FILTER: &str = "privacy_filter";

#[derive(Debug, Clone, PartialEq, Eq)]
struct RedactionSource {
    reason: &'static str,
    source_id: Option<String>,
    priority: u8,
}

impl RedactionSource {
    fn excluded_app(source_id: impl Into<String>) -> Self {
        Self {
            reason: REDACTION_REASON_EXCLUDED_APP,
            source_id: Some(source_id.into()),
            priority: 1,
        }
    }

    fn website_rule(source_id: impl Into<String>) -> Self {
        Self {
            reason: REDACTION_REASON_WEBSITE_RULE,
            source_id: Some(source_id.into()),
            priority: 2,
        }
    }

    fn title_rule(source_id: impl Into<String>) -> Self {
        Self {
            reason: REDACTION_REASON_TITLE_RULE,
            source_id: Some(source_id.into()),
            priority: 3,
        }
    }

    fn system(reason: &'static str) -> Self {
        Self {
            reason,
            source_id: None,
            priority: 4,
        }
    }
}

fn apply_redaction_source<T: Ord>(
    targets: &mut BTreeMap<T, RedactionSource>,
    target: T,
    source: RedactionSource,
) {
    match targets.get(&target) {
        Some(existing) if existing.priority >= source.priority => {}
        _ => {
            targets.insert(target, source);
        }
    }
}

pub fn is_private_browser_title(title: &str) -> bool {
    const PRIVATE_TITLE_PATTERNS: &[&str] =
        &["incognito", "private browsing", "inprivate", "(private)"];
    let title = title.to_ascii_lowercase();
    PRIVATE_TITLE_PATTERNS
        .iter()
        .any(|pattern| title.contains(pattern))
}

pub fn active_private_browser_detected(
    privacy: &PrivacySettings,
    bundle_id: Option<&str>,
    window_title: Option<&str>,
) -> bool {
    privacy.private_browser_exclusion_enabled
        && bundle_id.is_some_and(is_known_browser_bundle)
        && window_title.is_some_and(is_private_browser_title)
}

pub fn resolve_active_privacy_window_id(
    active_bundle_id: Option<&str>,
    active_window_id: Option<u32>,
    active_window_title: Option<&str>,
    visible_windows: &[WindowContext],
) -> Option<u32> {
    let active_bundle_id = active_bundle_id.filter(|bundle_id| !bundle_id.trim().is_empty())?;

    if let Some(active_window_id) = active_window_id {
        if visible_windows.iter().any(|window| {
            window.window_id == active_window_id
                && window.bundle_id.as_deref() == Some(active_bundle_id)
        }) {
            return Some(active_window_id);
        }
    }

    let active_window_title = active_window_title
        .map(str::trim)
        .filter(|title| !title.is_empty())?;
    let mut matches = visible_windows.iter().filter(|window| {
        window.bundle_id.as_deref() == Some(active_bundle_id)
            && window.title.trim() == active_window_title
    });
    let matched = matches.next()?;
    matches.next().is_none().then_some(matched.window_id)
}

pub fn apply_website_privacy_hold(
    held_bundle_reasons: &mut BTreeMap<String, String>,
    privacy: &PrivacySettings,
    context: &MetadataContext,
    decision: &mut PrivacyFilterDecision,
) {
    if !has_enabled_website_rules(privacy) {
        held_bundle_reasons.clear();
        return;
    }

    if let Some(active_bundle_id) = context
        .active_bundle_id
        .as_deref()
        .filter(|bundle_id| is_known_browser_bundle(bundle_id))
    {
        let active_private_browser = context
            .private_browser_ambiguous_bundle_id
            .as_deref()
            .is_some_and(|private_bundle_id| private_bundle_id == active_bundle_id);
        if let Some(active_url) = context.active_url.as_deref() {
            let matched_website_rule = privacy
                .excluded_website_rules
                .iter()
                .find(|rule| website_rule_matches(rule, active_url));
            if let Some(rule) = matched_website_rule {
                if let Some(window_id) = context.active_privacy_window_id {
                    if !decision.excluded_window_ids.contains(&window_id) {
                        decision.excluded_window_ids.push(window_id);
                    }
                    decision.excluded_window_reasons.insert(
                        window_id,
                        REDACTION_REASON_WEBSITE_RULE.to_string(),
                    );
                    decision
                        .excluded_window_source_ids
                        .insert(window_id, rule.id.clone());
                    held_bundle_reasons.remove(active_bundle_id);
                } else {
                    held_bundle_reasons.insert(
                        active_bundle_id.to_string(),
                        REDACTION_REASON_WEBSITE_RULE.to_string(),
                    );
                    decision
                        .excluded_bundle_source_ids
                        .insert(active_bundle_id.to_string(), rule.id.clone());
                }
            } else if !active_private_browser {
                held_bundle_reasons.remove(active_bundle_id);
            }
        } else {
            if let Some(window_id) = context.active_privacy_window_id {
                if !decision.excluded_window_ids.contains(&window_id) {
                    decision.excluded_window_ids.push(window_id);
                }
                decision.excluded_window_reasons.insert(
                    window_id,
                    REDACTION_REASON_WEBSITE_RULE_URL_UNAVAILABLE.to_string(),
                );
            } else {
                held_bundle_reasons.insert(
                    active_bundle_id.to_string(),
                    REDACTION_REASON_WEBSITE_RULE_URL_UNAVAILABLE.to_string(),
                );
            }
        }
    }

    for bundle_id in held_bundle_reasons.keys() {
        if !decision
            .excluded_bundle_ids
            .iter()
            .any(|excluded| excluded == bundle_id)
        {
            decision.excluded_bundle_ids.push(bundle_id.clone());
        }
        if let Some(reason) = held_bundle_reasons.get(bundle_id) {
            decision
                .excluded_bundle_reasons
                .insert(bundle_id.clone(), reason.clone());
        }
    }

    if !held_bundle_reasons.is_empty() || !decision.excluded_window_ids.is_empty() {
        decision.excluded_bundle_ids.sort();
        decision.excluded_bundle_ids.dedup();
        decision.excluded_window_ids.sort_unstable();
        decision.excluded_window_ids.dedup();
        decision.privacy_filter_applied = true;
        decision.metadata_redaction_reason.get_or_insert_with(|| {
            decision
                .excluded_window_reasons
                .values()
                .next()
                .cloned()
                .or_else(|| held_bundle_reasons.values().next().cloned())
                .unwrap_or_else(|| REDACTION_REASON_WEBSITE_RULE.to_string())
        });
    }
}

pub fn apply_unverified_visible_browser_window_privacy_guard(
    verified_window_ids: &mut BTreeSet<u32>,
    privacy: &PrivacySettings,
    context: &MetadataContext,
    decision: &mut PrivacyFilterDecision,
) {
    if !has_enabled_website_rules(privacy) {
        verified_window_ids.clear();
        return;
    }

    let visible_window_ids: BTreeSet<u32> = context
        .visible_windows
        .iter()
        .map(|window| window.window_id)
        .chain(context.active_privacy_window_id)
        .collect();
    verified_window_ids.retain(|window_id| visible_window_ids.contains(window_id));

    let active_known_browser_bundle = context
        .active_bundle_id
        .as_deref()
        .filter(|bundle_id| is_known_browser_bundle(bundle_id));
    let active_private_browser = context
        .private_browser_ambiguous_bundle_id
        .as_deref()
        .is_some_and(|private_bundle_id| Some(private_bundle_id) == active_known_browser_bundle);

    if let (Some(active_privacy_window_id), Some(active_bundle_id)) = (
        context.active_privacy_window_id,
        active_known_browser_bundle,
    ) {
        if let Some(active_url) = context.active_url.as_deref() {
            let matched_website_rule = privacy
                .excluded_website_rules
                .iter()
                .any(|rule| website_rule_matches(rule, active_url));
            if matched_website_rule || active_private_browser {
                verified_window_ids.remove(&active_privacy_window_id);
            } else {
                let active_visible_browser_window = context.visible_windows.iter().any(|window| {
                    window.window_id == active_privacy_window_id
                        && window.bundle_id.as_deref() == Some(active_bundle_id)
                });
                if active_visible_browser_window {
                    verified_window_ids.insert(active_privacy_window_id);
                }
            }
        } else {
            verified_window_ids.remove(&active_privacy_window_id);
        }
    }

    for window in &context.visible_windows {
        let Some(bundle_id) = window.bundle_id.as_deref() else {
            continue;
        };
        if !is_known_browser_bundle(bundle_id) {
            continue;
        }
        if decision
            .excluded_bundle_ids
            .iter()
            .any(|excluded| excluded == bundle_id)
        {
            continue;
        }
        if decision.excluded_window_ids.contains(&window.window_id) {
            continue;
        }
        if verified_window_ids.contains(&window.window_id) {
            continue;
        }
        if !decision.excluded_window_ids.contains(&window.window_id) {
            decision.excluded_window_ids.push(window.window_id);
        }
        decision.excluded_window_reasons.insert(
            window.window_id,
            REDACTION_REASON_WEBSITE_RULE_UNVERIFIED_VISIBLE_BROWSER.to_string(),
        );
    }

    if context
        .visible_windows
        .iter()
        .any(|window| decision.excluded_window_ids.contains(&window.window_id))
    {
        decision.excluded_window_ids.sort_unstable();
        decision.excluded_window_ids.dedup();
        decision.privacy_filter_applied = true;
        decision.metadata_redaction_reason.get_or_insert_with(|| {
            REDACTION_REASON_WEBSITE_RULE_UNVERIFIED_VISIBLE_BROWSER.to_string()
        });
    }
}

pub fn apply_metadata_redaction(
    snapshot: &mut FrameMetadataSnapshot,
    privacy: &PrivacySettings,
    context: &MetadataContext,
    decision: &PrivacyFilterDecision,
) {
    let snapshot_bundle_id = snapshot.app_bundle_id.as_deref();
    let decision_excludes_snapshot_bundle = snapshot_bundle_id.is_some_and(|bundle_id| {
        decision
            .excluded_bundle_ids
            .iter()
            .any(|excluded| excluded == bundle_id)
    });
    let snapshot_bundle_redaction_reason = snapshot_bundle_id
        .and_then(|bundle_id| decision.excluded_bundle_reasons.get(bundle_id).cloned());
    let snapshot_bundle_redaction_source_id = snapshot_bundle_id
        .and_then(|bundle_id| decision.excluded_bundle_source_ids.get(bundle_id).cloned());
    let decision_excludes_snapshot_window = snapshot
        .window_id
        .is_some_and(|window_id| decision.excluded_window_ids.contains(&window_id));
    let snapshot_window_redaction_reason = snapshot
        .window_id
        .and_then(|window_id| decision.excluded_window_reasons.get(&window_id))
        .cloned();
    let snapshot_window_redaction_source_id = snapshot
        .window_id
        .and_then(|window_id| decision.excluded_window_source_ids.get(&window_id))
        .cloned();
    let active_private_snapshot = privacy.private_browser_exclusion_enabled
        && (snapshot
            .window_title
            .as_deref()
            .is_some_and(is_private_browser_title)
            || context
                .private_browser_ambiguous_bundle_id
                .as_deref()
                .is_some_and(|private_bundle_id| Some(private_bundle_id) == snapshot_bundle_id));
    if !decision_excludes_snapshot_bundle
        && !decision_excludes_snapshot_window
        && !active_private_snapshot
    {
        return;
    }
    let (reason, source_id) = if active_private_snapshot {
        (REDACTION_REASON_PRIVATE_BROWSER.to_string(), None)
    } else if let Some(reason) = snapshot_window_redaction_reason {
        (reason, snapshot_window_redaction_source_id)
    } else if let Some(reason) = snapshot_bundle_redaction_reason {
        (reason, snapshot_bundle_redaction_source_id)
    } else {
        (
            decision
                .metadata_redaction_reason
                .clone()
                .unwrap_or_else(|| REDACTION_REASON_PRIVACY_FILTER.to_string()),
            None,
        )
    };
    snapshot.metadata_redaction_reason = Some(reason);
    snapshot.metadata_redaction_source_id = source_id;
    snapshot.window_title = None;
    snapshot.browser_url = None;
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
    let include_subdomains = raw_host
        .as_deref()
        .is_some_and(|host| host.starts_with("*."));
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
    let parsed_rule;
    let rule = if rule.host.is_none() {
        parsed_rule = parse_website_rule(rule.id.clone(), rule.enabled, &rule.pattern);
        if parsed_rule.host.is_none() {
            return false;
        }
        &parsed_rule
    } else {
        rule
    };
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

pub fn evaluate_privacy(
    settings: &PrivacySettings,
    context: &MetadataContext,
) -> PrivacyFilterDecision {
    let mut bundle_sources = BTreeMap::new();
    let mut window_sources = BTreeMap::new();
    let mut rule_ids = BTreeSet::new();
    let mut redaction_reason = None;

    for app in &settings.excluded_apps {
        if app.enabled && !app.bundle_id.trim().is_empty() {
            let bundle_id = app.bundle_id.trim().to_string();
            apply_redaction_source(
                &mut bundle_sources,
                bundle_id,
                RedactionSource::excluded_app(app.id.clone()),
            );
            rule_ids.insert(app.id.clone());
            redaction_reason.get_or_insert_with(|| REDACTION_REASON_EXCLUDED_APP.to_string());
        }
    }

    if let (Some(active_bundle), Some(active_url)) =
        (&context.active_bundle_id, &context.active_url)
    {
        for rule in &settings.excluded_website_rules {
            if website_rule_matches(rule, active_url) {
                if let Some(window_id) = context.active_privacy_window_id {
                    apply_redaction_source(
                        &mut window_sources,
                        window_id,
                        RedactionSource::website_rule(rule.id.clone()),
                    );
                } else {
                    apply_redaction_source(
                        &mut bundle_sources,
                        active_bundle.clone(),
                        RedactionSource::website_rule(rule.id.clone()),
                    );
                }
                rule_ids.insert(rule.id.clone());
                redaction_reason.get_or_insert_with(|| REDACTION_REASON_WEBSITE_RULE.to_string());
            }
        }
    }

    for rule in &settings.browser_title_rules {
        for window in &context.visible_windows {
            if title_rule_matches(rule, &window.title) {
                apply_redaction_source(
                    &mut window_sources,
                    window.window_id,
                    RedactionSource::title_rule(rule.id.clone()),
                );
                rule_ids.insert(rule.id.clone());
                redaction_reason.get_or_insert_with(|| REDACTION_REASON_TITLE_RULE.to_string());
            }
        }
        if context
            .active_window_title
            .as_deref()
            .is_some_and(|title| title_rule_matches(rule, title))
        {
            if let Some(window_id) = context.active_privacy_window_id {
                apply_redaction_source(
                    &mut window_sources,
                    window_id,
                    RedactionSource::title_rule(rule.id.clone()),
                );
                rule_ids.insert(rule.id.clone());
                redaction_reason.get_or_insert_with(|| REDACTION_REASON_TITLE_RULE.to_string());
            } else if let Some(bundle_id) = context.active_bundle_id.clone() {
                apply_redaction_source(
                    &mut bundle_sources,
                    bundle_id,
                    RedactionSource::title_rule(rule.id.clone()),
                );
                rule_ids.insert(rule.id.clone());
                redaction_reason.get_or_insert_with(|| REDACTION_REASON_TITLE_RULE.to_string());
            }
        }
    }

    if settings.private_browser_exclusion_enabled {
        if let Some(active_private_bundle_id) = context.private_browser_ambiguous_bundle_id.clone()
        {
            if let Some(window_id) = context.active_privacy_window_id {
                apply_redaction_source(
                    &mut window_sources,
                    window_id,
                    RedactionSource::system(REDACTION_REASON_PRIVATE_BROWSER),
                );
            } else {
                apply_redaction_source(
                    &mut bundle_sources,
                    active_private_bundle_id,
                    RedactionSource::system(REDACTION_REASON_PRIVATE_BROWSER),
                );
            }
            redaction_reason.get_or_insert_with(|| REDACTION_REASON_PRIVATE_BROWSER.to_string());
        }
        for window_id in &context.private_browser_window_ids {
            apply_redaction_source(
                &mut window_sources,
                *window_id,
                RedactionSource::system(REDACTION_REASON_PRIVATE_BROWSER),
            );
            redaction_reason.get_or_insert_with(|| REDACTION_REASON_PRIVATE_BROWSER.to_string());
        }
    }

    let excluded_bundle_ids = bundle_sources.keys().cloned().collect::<Vec<_>>();
    let excluded_window_ids = window_sources.keys().cloned().collect::<Vec<_>>();
    let excluded_bundle_reasons = bundle_sources
        .iter()
        .map(|(bundle_id, source)| (bundle_id.clone(), source.reason.to_string()))
        .collect();
    let excluded_window_reasons = window_sources
        .iter()
        .map(|(window_id, source)| (*window_id, source.reason.to_string()))
        .collect();
    let excluded_bundle_source_ids = bundle_sources
        .iter()
        .filter_map(|(bundle_id, source)| {
            source
                .source_id
                .as_ref()
                .map(|source_id| (bundle_id.clone(), source_id.clone()))
        })
        .collect();
    let excluded_window_source_ids = window_sources
        .iter()
        .filter_map(|(window_id, source)| {
            source
                .source_id
                .as_ref()
                .map(|source_id| (*window_id, source_id.clone()))
        })
        .collect();

    PrivacyFilterDecision {
        privacy_filter_applied: !excluded_bundle_ids.is_empty() || !excluded_window_ids.is_empty(),
        excluded_bundle_ids,
        excluded_window_ids,
        excluded_bundle_reasons,
        excluded_bundle_source_ids,
        excluded_window_reasons,
        excluded_window_source_ids,
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
        assert_eq!(
            sanitize_url("http://localhost:3000/a?token=1", BrowserUrlMode::Sanitized),
            Some("http://localhost:3000/a".to_string())
        );
    }

    #[test]
    fn matches_website_rules() {
        let wildcard = parse_website_rule("w", true, "*.example.com/private");
        assert!(website_rule_matches(
            &wildcard,
            "https://a.example.com/private/x"
        ));
        assert!(website_rule_matches(
            &wildcard,
            "https://example.com/private/x"
        ));
        let port = parse_website_rule("p", true, "localhost:5173/app");
        assert!(website_rule_matches(
            &port,
            "http://localhost:5173/app/%7Bsecret%7D"
        ));
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
    fn unparseable_website_rule_without_host_does_not_match() {
        let unparseable = WebsiteRule {
            id: "invalid".into(),
            enabled: true,
            pattern: "http://[::1".into(),
            host: None,
            include_subdomains: false,
            path_prefix: None,
            port: None,
        };

        assert!(!website_rule_matches(
            &unparseable,
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
                active_bundle_id: Some("com.google.Chrome".into()),
                active_window_id: Some(7),
                active_window_title: Some("Vault".into()),
                active_privacy_window_id: Some(7),
                active_url: Some("https://example.com/private".into()),
                visible_windows: vec![WindowContext {
                    window_id: 7,
                    bundle_id: Some("com.google.Chrome".into()),
                    owner_pid: None,
                    title: "Vault".into(),
                }],
                private_browser_window_ids: vec![9],
                private_browser_ambiguous_bundle_id: Some("com.google.Chrome".into()),
            },
        );
        assert_eq!(
            decision.excluded_bundle_ids,
            vec!["com.secret"]
        );
        assert_eq!(decision.excluded_window_ids, vec![7, 9]);
        assert_eq!(
            decision.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_EXCLUDED_APP)
        );
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
                private_browser_window_ids: vec![9],
                ..MetadataContext::default()
            },
        );

        assert!(decision.excluded_bundle_ids.is_empty());
        assert_eq!(decision.excluded_window_ids, vec![9]);
        assert_eq!(
            decision.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_PRIVATE_BROWSER)
        );
    }

    #[test]
    fn private_browser_excludes_multiple_matched_windows() {
        let settings = PrivacySettings {
            private_browser_exclusion_enabled: true,
            ..PrivacySettings::default()
        };
        let decision = evaluate_privacy(
            &settings,
            &MetadataContext {
                private_browser_window_ids: vec![42, 9],
                ..MetadataContext::default()
            },
        );

        assert_eq!(decision.excluded_window_ids, vec![9, 42]);
        assert_eq!(
            decision.excluded_window_reasons.get(&9).map(String::as_str),
            Some(REDACTION_REASON_PRIVATE_BROWSER)
        );
        assert_eq!(
            decision
                .excluded_window_reasons
                .get(&42)
                .map(String::as_str),
            Some(REDACTION_REASON_PRIVATE_BROWSER)
        );
    }

    #[test]
    fn private_browser_exclusion_disabled_ignores_private_window_ids() {
        let settings = PrivacySettings {
            private_browser_exclusion_enabled: false,
            ..PrivacySettings::default()
        };
        let decision = evaluate_privacy(
            &settings,
            &MetadataContext {
                private_browser_window_ids: vec![9, 42],
                private_browser_ambiguous_bundle_id: Some("com.google.Chrome".to_string()),
                ..MetadataContext::default()
            },
        );

        assert!(decision.excluded_bundle_ids.is_empty());
        assert!(decision.excluded_window_ids.is_empty());
        assert!(!decision.privacy_filter_applied);
    }

    #[test]
    fn active_private_browser_excludes_resolved_active_privacy_window() {
        let settings = PrivacySettings {
            private_browser_exclusion_enabled: true,
            ..PrivacySettings::default()
        };
        let decision = evaluate_privacy(
            &settings,
            &MetadataContext {
                active_privacy_window_id: Some(42),
                private_browser_window_ids: vec![9],
                private_browser_ambiguous_bundle_id: Some("com.google.Chrome".into()),
                ..MetadataContext::default()
            },
        );

        assert!(decision.excluded_bundle_ids.is_empty());
        assert_eq!(decision.excluded_window_ids, vec![9, 42]);
        assert_eq!(
            decision
                .excluded_window_reasons
                .get(&42)
                .map(String::as_str),
            Some(REDACTION_REASON_PRIVATE_BROWSER)
        );
        assert_eq!(
            decision.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_PRIVATE_BROWSER)
        );
    }

    #[test]
    fn active_private_browser_excludes_bundle_when_window_cannot_be_resolved() {
        let settings = PrivacySettings {
            private_browser_exclusion_enabled: true,
            ..PrivacySettings::default()
        };
        let decision = evaluate_privacy(
            &settings,
            &MetadataContext {
                private_browser_ambiguous_bundle_id: Some("com.google.Chrome".into()),
                ..MetadataContext::default()
            },
        );

        assert_eq!(decision.excluded_bundle_ids, vec!["com.google.Chrome"]);
        assert!(decision.excluded_window_ids.is_empty());
        assert_eq!(
            decision
                .excluded_bundle_reasons
                .get("com.google.Chrome")
                .map(String::as_str),
            Some(REDACTION_REASON_PRIVATE_BROWSER)
        );
        assert_eq!(
            decision.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_PRIVATE_BROWSER)
        );
    }

    #[test]
    fn filter_key_sorts_and_dedupes() {
        let key = PrivacyFilterKey::new(1, vec!["b".into(), "a".into(), "a".into()], vec![3, 2, 3]);
        assert_eq!(key.bundle_ids, vec!["a", "b"]);
        assert_eq!(key.window_ids, vec![2, 3]);
    }

    fn website_privacy(pattern: &str) -> PrivacySettings {
        PrivacySettings {
            excluded_website_rules: vec![parse_website_rule("website-rule", true, pattern)],
            ..PrivacySettings::default()
        }
    }

    #[test]
    fn website_privacy_hold_keeps_browser_excluded_after_leaving_browser() {
        let privacy = website_privacy("*.infinityapp.in");
        let mut held_bundle_reasons = BTreeMap::new();
        let mut decision = PrivacyFilterDecision::default();
        let browser_context = MetadataContext {
            active_bundle_id: Some("net.imput.helium".to_string()),
            active_url: Some("https://dashboard.infinityapp.in/app/dashboard".to_string()),
            ..MetadataContext::default()
        };

        apply_website_privacy_hold(
            &mut held_bundle_reasons,
            &privacy,
            &browser_context,
            &mut decision,
        );
        assert_eq!(decision.excluded_bundle_ids, vec!["net.imput.helium"]);

        let mut decision = PrivacyFilterDecision::default();
        let non_browser_context = MetadataContext {
            active_bundle_id: Some("com.apple.finder".to_string()),
            ..MetadataContext::default()
        };

        apply_website_privacy_hold(
            &mut held_bundle_reasons,
            &privacy,
            &non_browser_context,
            &mut decision,
        );

        assert_eq!(decision.excluded_bundle_ids, vec!["net.imput.helium"]);
        assert!(decision.privacy_filter_applied);
        assert_eq!(
            decision.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_WEBSITE_RULE)
        );
    }

    #[test]
    fn website_privacy_hold_clears_after_successful_non_matching_browser_probe() {
        let privacy = website_privacy("*.infinityapp.in");
        let mut held_bundle_reasons = BTreeMap::from([(
            "net.imput.helium".to_string(),
            REDACTION_REASON_WEBSITE_RULE.to_string(),
        )]);
        let mut decision = PrivacyFilterDecision::default();
        let browser_context = MetadataContext {
            active_bundle_id: Some("net.imput.helium".to_string()),
            active_url: Some("https://example.com/".to_string()),
            ..MetadataContext::default()
        };

        apply_website_privacy_hold(
            &mut held_bundle_reasons,
            &privacy,
            &browser_context,
            &mut decision,
        );

        assert!(held_bundle_reasons.is_empty());
        assert!(decision.excluded_bundle_ids.is_empty());
        assert!(!decision.privacy_filter_applied);
    }

    #[test]
    fn website_privacy_hold_excludes_known_browser_when_url_probe_is_unknown() {
        let privacy = website_privacy("*.infinityapp.in");
        let mut held_bundle_reasons = BTreeMap::new();
        let mut decision = PrivacyFilterDecision::default();
        let browser_context = MetadataContext {
            active_bundle_id: Some("net.imput.helium".to_string()),
            active_url: None,
            ..MetadataContext::default()
        };

        apply_website_privacy_hold(
            &mut held_bundle_reasons,
            &privacy,
            &browser_context,
            &mut decision,
        );

        assert_eq!(
            held_bundle_reasons
                .get("net.imput.helium")
                .map(String::as_str),
            Some(REDACTION_REASON_WEBSITE_RULE_URL_UNAVAILABLE)
        );
        assert_eq!(decision.excluded_bundle_ids, vec!["net.imput.helium"]);
        assert!(decision.privacy_filter_applied);
        assert_eq!(
            decision.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_WEBSITE_RULE_URL_UNAVAILABLE)
        );
    }

    #[test]
    fn website_privacy_hold_survives_private_browser_activation() {
        let privacy = website_privacy("*.infinityapp.in");
        let mut held_bundle_reasons = BTreeMap::from([(
            "net.imput.helium".to_string(),
            REDACTION_REASON_WEBSITE_RULE.to_string(),
        )]);
        let private_browser_context = MetadataContext {
            active_bundle_id: Some("net.imput.helium".to_string()),
            active_url: Some("https://example.com/".to_string()),
            private_browser_window_ids: vec![9],
            private_browser_ambiguous_bundle_id: Some("net.imput.helium".to_string()),
            ..MetadataContext::default()
        };
        let mut decision = evaluate_privacy(&privacy, &private_browser_context);

        apply_website_privacy_hold(
            &mut held_bundle_reasons,
            &privacy,
            &private_browser_context,
            &mut decision,
        );

        assert_eq!(
            held_bundle_reasons
                .get("net.imput.helium")
                .map(String::as_str),
            Some(REDACTION_REASON_WEBSITE_RULE)
        );
        assert_eq!(decision.excluded_bundle_ids, vec!["net.imput.helium"]);
        assert_eq!(decision.excluded_window_ids, vec![9]);
        assert!(decision.privacy_filter_applied);
    }

    #[test]
    fn resolves_active_privacy_window_id_from_unique_visible_title_match() {
        let visible_windows = vec![
            WindowContext {
                window_id: 42,
                bundle_id: Some("com.tinyspeck.slackmacgap".to_string()),
                owner_pid: None,
                title: "Example".to_string(),
            },
            WindowContext {
                window_id: 43,
                bundle_id: Some("com.apple.finder".to_string()),
                owner_pid: None,
                title: "Example".to_string(),
            },
        ];

        assert_eq!(
            resolve_active_privacy_window_id(
                Some("com.tinyspeck.slackmacgap"),
                Some(7),
                Some("Example"),
                &visible_windows,
            ),
            Some(42)
        );
    }

    #[test]
    fn active_privacy_window_resolution_stays_conservative_for_ambiguous_title_match() {
        let visible_windows = vec![
            WindowContext {
                window_id: 42,
                bundle_id: Some("com.google.Chrome".to_string()),
                owner_pid: None,
                title: "Example".to_string(),
            },
            WindowContext {
                window_id: 43,
                bundle_id: Some("com.google.Chrome".to_string()),
                owner_pid: None,
                title: "Example".to_string(),
            },
        ];

        assert_eq!(
            resolve_active_privacy_window_id(
                Some("com.google.Chrome"),
                Some(7),
                Some("Example"),
                &visible_windows,
            ),
            None
        );
    }

    #[test]
    fn website_privacy_guard_excludes_unverified_visible_background_browser() {
        let privacy = website_privacy("*.infinityapp.in");
        let mut verified_window_ids = BTreeSet::new();
        let mut decision = PrivacyFilterDecision::default();
        let context = MetadataContext {
            active_bundle_id: Some("com.apple.finder".to_string()),
            visible_windows: vec![WindowContext {
                window_id: 42,
                bundle_id: Some("com.google.Chrome".to_string()),
                owner_pid: None,
                title: "Dashboard".to_string(),
            }],
            ..MetadataContext::default()
        };

        apply_unverified_visible_browser_window_privacy_guard(
            &mut verified_window_ids,
            &privacy,
            &context,
            &mut decision,
        );

        assert_eq!(decision.excluded_window_ids, vec![42]);
        assert_eq!(
            decision
                .excluded_window_reasons
                .get(&42)
                .map(String::as_str),
            Some(REDACTION_REASON_WEBSITE_RULE_UNVERIFIED_VISIBLE_BROWSER)
        );
        assert!(decision.privacy_filter_applied);
        assert!(verified_window_ids.is_empty());
    }

    #[test]
    fn website_privacy_guard_keeps_window_clear_after_active_non_matching_probe() {
        let privacy = website_privacy("*.infinityapp.in");
        let mut verified_window_ids = BTreeSet::new();
        let browser_window = WindowContext {
            window_id: 42,
            bundle_id: Some("com.google.Chrome".to_string()),
            owner_pid: None,
            title: "Example".to_string(),
        };
        let active_browser_context = MetadataContext {
            active_bundle_id: Some("com.google.Chrome".to_string()),
            active_window_id: Some(42),
            active_privacy_window_id: Some(42),
            active_url: Some("https://example.com/".to_string()),
            visible_windows: vec![browser_window.clone()],
            ..MetadataContext::default()
        };
        let mut decision = PrivacyFilterDecision::default();

        apply_unverified_visible_browser_window_privacy_guard(
            &mut verified_window_ids,
            &privacy,
            &active_browser_context,
            &mut decision,
        );

        assert_eq!(verified_window_ids, BTreeSet::from([42]));
        assert!(decision.excluded_window_ids.is_empty());
        assert!(!decision.privacy_filter_applied);

        let background_context = MetadataContext {
            active_bundle_id: Some("com.apple.finder".to_string()),
            visible_windows: vec![browser_window],
            ..MetadataContext::default()
        };
        let mut decision = PrivacyFilterDecision::default();

        apply_unverified_visible_browser_window_privacy_guard(
            &mut verified_window_ids,
            &privacy,
            &background_context,
            &mut decision,
        );

        assert_eq!(verified_window_ids, BTreeSet::from([42]));
        assert!(decision.excluded_window_ids.is_empty());
        assert!(!decision.privacy_filter_applied);
    }

    #[test]
    fn website_privacy_guard_uses_resolved_privacy_window_id_after_mismatched_active_window_probe()
    {
        let privacy = website_privacy("*.infinityapp.in");
        let mut verified_window_ids = BTreeSet::new();
        let browser_window = WindowContext {
            window_id: 42,
            bundle_id: Some("com.google.Chrome".to_string()),
            owner_pid: None,
            title: "Example".to_string(),
        };
        let active_browser_context = MetadataContext {
            active_bundle_id: Some("com.google.Chrome".to_string()),
            active_window_id: Some(7),
            active_privacy_window_id: Some(42),
            active_url: Some("https://example.com/".to_string()),
            visible_windows: vec![browser_window.clone()],
            ..MetadataContext::default()
        };
        let mut decision = PrivacyFilterDecision::default();

        apply_unverified_visible_browser_window_privacy_guard(
            &mut verified_window_ids,
            &privacy,
            &active_browser_context,
            &mut decision,
        );

        assert_eq!(verified_window_ids, BTreeSet::from([42]));
        assert!(decision.excluded_window_ids.is_empty());
        assert!(!decision.privacy_filter_applied);

        let background_context = MetadataContext {
            active_bundle_id: Some("com.apple.finder".to_string()),
            visible_windows: vec![browser_window],
            ..MetadataContext::default()
        };
        let mut decision = PrivacyFilterDecision::default();

        apply_unverified_visible_browser_window_privacy_guard(
            &mut verified_window_ids,
            &privacy,
            &background_context,
            &mut decision,
        );

        assert_eq!(verified_window_ids, BTreeSet::from([42]));
        assert!(decision.excluded_window_ids.is_empty());
        assert!(!decision.privacy_filter_applied);
    }

    #[test]
    fn active_matching_website_reason_wins_over_unverified_visible_browser_guard() {
        let privacy = website_privacy("*.infinityapp.in");
        let context = MetadataContext {
            active_bundle_id: Some("net.imput.helium".to_string()),
            active_window_id: Some(42),
            active_privacy_window_id: Some(42),
            active_url: Some("https://dashboard.infinityapp.in/app/dashboard".to_string()),
            visible_windows: vec![WindowContext {
                window_id: 42,
                bundle_id: Some("net.imput.helium".to_string()),
                owner_pid: None,
                title: "Dashboard".to_string(),
            }],
            ..MetadataContext::default()
        };
        let mut decision = evaluate_privacy(&privacy, &context);
        let mut verified_window_ids = BTreeSet::new();
        apply_unverified_visible_browser_window_privacy_guard(
            &mut verified_window_ids,
            &privacy,
            &context,
            &mut decision,
        );
        let mut snapshot = FrameMetadataSnapshot {
            app_bundle_id: Some("net.imput.helium".to_string()),
            app_name: Some("Helium".to_string()),
            window_title: Some("Dashboard".to_string()),
            window_id: Some(42),
            browser_url: Some("https://dashboard.infinityapp.in/app/dashboard".to_string()),
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        };

        apply_metadata_redaction(&mut snapshot, &privacy, &context, &decision);

        assert_eq!(
            snapshot.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_WEBSITE_RULE)
        );
    }

    #[test]
    fn active_excluded_browser_metadata_redacts_title_and_url() {
        let mut snapshot = FrameMetadataSnapshot {
            app_bundle_id: Some("net.imput.helium".to_string()),
            app_name: Some("Helium".to_string()),
            window_title: Some("Infinity - Helium".to_string()),
            window_id: None,
            browser_url: Some("https://dashboard.infinityapp.in/app/dashboard".to_string()),
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        };
        let decision = PrivacyFilterDecision {
            excluded_bundle_ids: vec!["net.imput.helium".to_string()],
            metadata_redaction_reason: Some(REDACTION_REASON_WEBSITE_RULE.to_string()),
            privacy_filter_applied: true,
            ..PrivacyFilterDecision::default()
        };

        apply_metadata_redaction(
            &mut snapshot,
            &PrivacySettings::default(),
            &MetadataContext::default(),
            &decision,
        );

        assert_eq!(
            snapshot.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_WEBSITE_RULE)
        );
        assert!(snapshot.window_title.is_none());
        assert!(snapshot.browser_url.is_none());
    }

    #[test]
    fn website_rule_metadata_reason_wins_for_matching_browser_when_other_app_is_excluded() {
        let privacy = PrivacySettings {
            excluded_apps: vec![ExcludedAppEntry {
                id: "app".into(),
                enabled: true,
                bundle_id: "net.whatsapp.WhatsApp".into(),
                display_name: "WhatsApp".into(),
            }],
            excluded_website_rules: vec![parse_website_rule(
                "site",
                true,
                "https://dashboard.infinityapp.in",
            )],
            ..PrivacySettings::default()
        };
        let decision = evaluate_privacy(
            &privacy,
            &MetadataContext {
                active_bundle_id: Some("net.imput.helium".to_string()),
                active_url: Some("https://dashboard.infinityapp.in/app/dashboard".to_string()),
                ..MetadataContext::default()
            },
        );
        let mut snapshot = FrameMetadataSnapshot {
            app_bundle_id: Some("net.imput.helium".to_string()),
            app_name: Some("Helium".to_string()),
            window_title: Some("Infinity - Helium".to_string()),
            window_id: None,
            browser_url: Some("https://dashboard.infinityapp.in/app/dashboard".to_string()),
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        };

        apply_metadata_redaction(
            &mut snapshot,
            &privacy,
            &MetadataContext::default(),
            &decision,
        );

        assert_eq!(
            decision.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_EXCLUDED_APP)
        );
        assert_eq!(
            snapshot.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_WEBSITE_RULE)
        );
        assert!(snapshot.window_title.is_none());
        assert!(snapshot.browser_url.is_none());
    }

    #[test]
    fn website_rule_source_id_is_persisted_after_redaction() {
        let privacy = PrivacySettings {
            excluded_website_rules: vec![parse_website_rule(
                "website-1778659985506-5bb1ir",
                true,
                "https://dashboard.infinityapp.in",
            )],
            ..PrivacySettings::default()
        };
        let context = MetadataContext {
            active_bundle_id: Some("net.imput.helium".to_string()),
            active_url: Some("https://dashboard.infinityapp.in/app/dashboard".to_string()),
            ..MetadataContext::default()
        };
        let decision = evaluate_privacy(&privacy, &context);
        let mut snapshot = FrameMetadataSnapshot {
            app_bundle_id: Some("net.imput.helium".to_string()),
            app_name: Some("Helium".to_string()),
            window_title: Some("Infinity - Helium".to_string()),
            window_id: None,
            browser_url: Some("https://dashboard.infinityapp.in/app/dashboard".to_string()),
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        };

        apply_metadata_redaction(&mut snapshot, &privacy, &context, &decision);

        assert_eq!(
            snapshot.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_WEBSITE_RULE)
        );
        assert_eq!(
            snapshot.metadata_redaction_source_id.as_deref(),
            Some("website-1778659985506-5bb1ir")
        );
    }

    #[test]
    fn title_rule_source_id_is_persisted_after_redaction() {
        let privacy = PrivacySettings {
            private_browser_exclusion_enabled: false,
            browser_title_rules: vec![BrowserTitleRule {
                id: "title-rule-123".to_string(),
                enabled: true,
                match_type: BrowserTitleRuleMatchType::Substring,
                pattern: "secret".to_string(),
            }],
            ..PrivacySettings::default()
        };
        let context = MetadataContext {
            visible_windows: vec![WindowContext {
                window_id: 7,
                bundle_id: Some("com.google.Chrome".to_string()),
                owner_pid: None,
                title: "Secret dashboard".to_string(),
            }],
            ..MetadataContext::default()
        };
        let decision = evaluate_privacy(&privacy, &context);
        let mut snapshot = FrameMetadataSnapshot {
            app_bundle_id: Some("com.google.Chrome".to_string()),
            app_name: Some("Google Chrome".to_string()),
            window_title: Some("Secret dashboard".to_string()),
            window_id: Some(7),
            browser_url: Some("https://example.com/".to_string()),
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        };

        apply_metadata_redaction(&mut snapshot, &privacy, &context, &decision);

        assert_eq!(
            snapshot.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_TITLE_RULE)
        );
        assert_eq!(
            snapshot.metadata_redaction_source_id.as_deref(),
            Some("title-rule-123")
        );
    }

    #[test]
    fn excluded_app_source_id_is_persisted_after_redaction() {
        let privacy = PrivacySettings {
            excluded_apps: vec![ExcludedAppEntry {
                id: "excluded-app-123".to_string(),
                enabled: true,
                bundle_id: "com.secret.App".to_string(),
                display_name: "Secret".to_string(),
            }],
            ..PrivacySettings::default()
        };
        let decision = evaluate_privacy(&privacy, &MetadataContext::default());
        let mut snapshot = FrameMetadataSnapshot {
            app_bundle_id: Some("com.secret.App".to_string()),
            app_name: Some("Secret".to_string()),
            window_title: Some("Private notes".to_string()),
            window_id: None,
            browser_url: None,
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        };

        apply_metadata_redaction(
            &mut snapshot,
            &privacy,
            &MetadataContext::default(),
            &decision,
        );

        assert_eq!(
            snapshot.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_EXCLUDED_APP)
        );
        assert_eq!(
            snapshot.metadata_redaction_source_id.as_deref(),
            Some("excluded-app-123")
        );
    }

    #[test]
    fn different_redaction_source_ids_produce_different_normalized_hashes() {
        let first = FrameMetadataSnapshot {
            app_bundle_id: Some("net.imput.helium".to_string()),
            app_name: Some("Helium".to_string()),
            window_title: None,
            window_id: Some(42),
            browser_url: None,
            display_id: None,
            metadata_redaction_reason: Some(REDACTION_REASON_WEBSITE_RULE.to_string()),
            metadata_redaction_source_id: Some("website-a".to_string()),
        };
        let second = FrameMetadataSnapshot {
            metadata_redaction_source_id: Some("website-b".to_string()),
            ..first.clone()
        };

        assert_ne!(first.normalized_hash(), second.normalized_hash());
        assert!(!first.normalized_json().contains("windowId"));
    }

    #[test]
    fn redaction_source_precedence_uses_title_then_website_then_app() {
        let privacy = PrivacySettings {
            excluded_apps: vec![ExcludedAppEntry {
                id: "app-source".to_string(),
                enabled: true,
                bundle_id: "net.imput.helium".to_string(),
                display_name: "Helium".to_string(),
            }],
            excluded_website_rules: vec![parse_website_rule(
                "website-source",
                true,
                "https://dashboard.infinityapp.in",
            )],
            browser_title_rules: vec![BrowserTitleRule {
                id: "title-source".to_string(),
                enabled: true,
                match_type: BrowserTitleRuleMatchType::Substring,
                pattern: "dashboard".to_string(),
            }],
            private_browser_exclusion_enabled: false,
        };
        let context = MetadataContext {
            active_bundle_id: Some("net.imput.helium".to_string()),
            active_window_id: Some(7),
            active_window_title: Some("Dashboard".to_string()),
            active_privacy_window_id: Some(7),
            active_url: Some("https://dashboard.infinityapp.in/app/dashboard".to_string()),
            visible_windows: vec![WindowContext {
                window_id: 7,
                bundle_id: Some("net.imput.helium".to_string()),
                owner_pid: None,
                title: "Dashboard".to_string(),
            }],
            ..MetadataContext::default()
        };
        let decision = evaluate_privacy(&privacy, &context);
        let mut snapshot = FrameMetadataSnapshot {
            app_bundle_id: Some("net.imput.helium".to_string()),
            app_name: Some("Helium".to_string()),
            window_title: Some("Dashboard".to_string()),
            window_id: Some(7),
            browser_url: Some("https://dashboard.infinityapp.in/app/dashboard".to_string()),
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        };

        apply_metadata_redaction(&mut snapshot, &privacy, &context, &decision);

        assert_eq!(
            snapshot.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_TITLE_RULE)
        );
        assert_eq!(
            snapshot.metadata_redaction_source_id.as_deref(),
            Some("title-source")
        );

        let context_without_title = MetadataContext {
            active_window_title: Some("Public".to_string()),
            visible_windows: vec![WindowContext {
                title: "Public".to_string(),
                ..context.visible_windows[0].clone()
            }],
            ..context
        };
        let decision = evaluate_privacy(&privacy, &context_without_title);
        let mut snapshot = FrameMetadataSnapshot {
            app_bundle_id: Some("net.imput.helium".to_string()),
            app_name: Some("Helium".to_string()),
            window_title: Some("Public".to_string()),
            window_id: Some(7),
            browser_url: Some("https://dashboard.infinityapp.in/app/dashboard".to_string()),
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        };

        apply_metadata_redaction(&mut snapshot, &privacy, &context_without_title, &decision);

        assert_eq!(
            snapshot.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_WEBSITE_RULE)
        );
        assert_eq!(
            snapshot.metadata_redaction_source_id.as_deref(),
            Some("website-source")
        );
    }

    #[test]
    fn equal_precedence_redaction_sources_preserve_first_matching_rule() {
        let privacy = PrivacySettings {
            excluded_website_rules: vec![
                parse_website_rule("website-first", true, "https://dashboard.infinityapp.in"),
                parse_website_rule("website-second", true, "*.infinityapp.in"),
            ],
            browser_title_rules: vec![
                BrowserTitleRule {
                    id: "title-first".to_string(),
                    enabled: true,
                    match_type: BrowserTitleRuleMatchType::Substring,
                    pattern: "dash".to_string(),
                },
                BrowserTitleRule {
                    id: "title-second".to_string(),
                    enabled: true,
                    match_type: BrowserTitleRuleMatchType::Substring,
                    pattern: "dashboard".to_string(),
                },
            ],
            private_browser_exclusion_enabled: false,
            ..PrivacySettings::default()
        };
        let context = MetadataContext {
            active_bundle_id: Some("net.imput.helium".to_string()),
            active_window_title: Some("Dashboard".to_string()),
            active_privacy_window_id: Some(7),
            active_url: Some("https://dashboard.infinityapp.in/app/dashboard".to_string()),
            visible_windows: vec![WindowContext {
                window_id: 7,
                bundle_id: Some("net.imput.helium".to_string()),
                owner_pid: None,
                title: "Dashboard".to_string(),
            }],
            ..MetadataContext::default()
        };
        let decision = evaluate_privacy(&privacy, &context);

        assert_eq!(
            decision
                .excluded_bundle_source_ids
                .get("net.imput.helium")
                .map(String::as_str),
            None
        );
        assert_eq!(
            decision
                .excluded_window_source_ids
                .get(&7)
                .map(String::as_str),
            Some("title-first")
        );
    }

    #[test]
    fn system_redactions_do_not_persist_source_ids() {
        let private_privacy = PrivacySettings::default();
        let private_context = MetadataContext {
            private_browser_window_ids: vec![7],
            ..MetadataContext::default()
        };
        let private_decision = evaluate_privacy(&private_privacy, &private_context);
        let mut private_snapshot = FrameMetadataSnapshot {
            app_bundle_id: Some("com.google.Chrome".to_string()),
            app_name: Some("Google Chrome".to_string()),
            window_title: Some("New Incognito Tab - Google Chrome".to_string()),
            window_id: Some(7),
            browser_url: None,
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        };

        apply_metadata_redaction(
            &mut private_snapshot,
            &private_privacy,
            &private_context,
            &private_decision,
        );

        assert_eq!(
            private_snapshot.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_PRIVATE_BROWSER)
        );
        assert_eq!(private_snapshot.metadata_redaction_source_id, None);

        let website_privacy = website_privacy("*.infinityapp.in");
        let mut held_bundle_reasons = BTreeMap::new();
        let mut held_decision = PrivacyFilterDecision::default();
        let held_context = MetadataContext {
            active_bundle_id: Some("net.imput.helium".to_string()),
            active_url: None,
            ..MetadataContext::default()
        };
        apply_website_privacy_hold(
            &mut held_bundle_reasons,
            &website_privacy,
            &held_context,
            &mut held_decision,
        );
        let mut unavailable_snapshot = FrameMetadataSnapshot {
            app_bundle_id: Some("net.imput.helium".to_string()),
            app_name: Some("Helium".to_string()),
            window_title: Some("Unknown".to_string()),
            window_id: None,
            browser_url: None,
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        };

        apply_metadata_redaction(
            &mut unavailable_snapshot,
            &website_privacy,
            &held_context,
            &held_decision,
        );

        assert_eq!(
            unavailable_snapshot.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_WEBSITE_RULE_URL_UNAVAILABLE)
        );
        assert_eq!(unavailable_snapshot.metadata_redaction_source_id, None);

        let mut verified_window_ids = BTreeSet::new();
        let mut unverified_decision = PrivacyFilterDecision::default();
        let unverified_context = MetadataContext {
            active_bundle_id: Some("com.apple.finder".to_string()),
            visible_windows: vec![WindowContext {
                window_id: 42,
                bundle_id: Some("com.google.Chrome".to_string()),
                owner_pid: None,
                title: "Dashboard".to_string(),
            }],
            ..MetadataContext::default()
        };
        apply_unverified_visible_browser_window_privacy_guard(
            &mut verified_window_ids,
            &website_privacy,
            &unverified_context,
            &mut unverified_decision,
        );
        let mut unverified_snapshot = FrameMetadataSnapshot {
            app_bundle_id: Some("com.google.Chrome".to_string()),
            app_name: Some("Google Chrome".to_string()),
            window_title: Some("Dashboard".to_string()),
            window_id: Some(42),
            browser_url: None,
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        };

        apply_metadata_redaction(
            &mut unverified_snapshot,
            &website_privacy,
            &unverified_context,
            &unverified_decision,
        );

        assert_eq!(
            unverified_snapshot.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_WEBSITE_RULE_UNVERIFIED_VISIBLE_BROWSER)
        );
        assert_eq!(unverified_snapshot.metadata_redaction_source_id, None);
    }

    #[test]
    fn old_snapshot_json_without_redaction_source_id_deserializes_as_none() {
        let snapshot: FrameMetadataSnapshot = serde_json::from_str(
            r#"{
                "appBundleId": "net.imput.helium",
                "appName": "Helium",
                "windowTitle": null,
                "browserUrl": null,
                "displayId": null,
                "metadataRedactionReason": "website_rule"
            }"#,
        )
        .expect("old snapshot JSON should deserialize");

        assert_eq!(snapshot.metadata_redaction_source_id, None);
        assert!(snapshot
            .normalized_json()
            .contains(r#""metadataRedactionSourceId":null"#));
    }

    #[test]
    fn active_excluded_window_metadata_redacts_title_and_url() {
        let mut snapshot = FrameMetadataSnapshot {
            app_bundle_id: Some("net.imput.helium".to_string()),
            app_name: Some("Helium".to_string()),
            window_title: Some("Inbox - Gmail".to_string()),
            window_id: Some(7),
            browser_url: Some("https://mail.google.com/mail/u/0/".to_string()),
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        };
        let decision = PrivacyFilterDecision {
            excluded_window_ids: vec![7],
            metadata_redaction_reason: Some(REDACTION_REASON_TITLE_RULE.to_string()),
            privacy_filter_applied: true,
            ..PrivacyFilterDecision::default()
        };

        apply_metadata_redaction(
            &mut snapshot,
            &PrivacySettings::default(),
            &MetadataContext::default(),
            &decision,
        );

        assert_eq!(
            snapshot.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_TITLE_RULE)
        );
        assert!(snapshot.window_title.is_none());
        assert!(snapshot.browser_url.is_none());
        assert!(!snapshot.normalized_json().contains("windowId"));
    }

    #[test]
    fn bundleless_excluded_window_metadata_redacts_title_and_url() {
        let mut snapshot = FrameMetadataSnapshot {
            app_bundle_id: None,
            app_name: Some("Unknown Browser".to_string()),
            window_title: Some("Inbox - Gmail".to_string()),
            window_id: Some(7),
            browser_url: Some("https://mail.google.com/mail/u/0/".to_string()),
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        };
        let decision = PrivacyFilterDecision {
            excluded_window_ids: vec![7],
            metadata_redaction_reason: Some(REDACTION_REASON_TITLE_RULE.to_string()),
            privacy_filter_applied: true,
            ..PrivacyFilterDecision::default()
        };

        apply_metadata_redaction(
            &mut snapshot,
            &PrivacySettings::default(),
            &MetadataContext::default(),
            &decision,
        );

        assert_eq!(
            snapshot.metadata_redaction_reason.as_deref(),
            Some(REDACTION_REASON_TITLE_RULE)
        );
        assert!(snapshot.window_title.is_none());
        assert!(snapshot.browser_url.is_none());
    }

    #[test]
    fn private_browser_title_matching_covers_common_browser_labels() {
        assert!(is_private_browser_title(
            "Example Site - Mozilla Firefox — (Private Browsing)"
        ));
        assert!(is_private_browser_title(
            "New Incognito Tab - Google Chrome"
        ));
        assert!(is_private_browser_title("Bank - Microsoft Edge InPrivate"));
        assert!(is_private_browser_title("Example - Safari (Private)"));
        assert!(!is_private_browser_title("Example Site - Helium"));
    }

    #[test]
    fn private_browser_window_resolution_ignores_non_browser_titles() {
        let privacy = PrivacySettings {
            private_browser_exclusion_enabled: true,
            ..PrivacySettings::default()
        };
        let visible_windows = vec![
            WindowContext {
                window_id: 7,
                bundle_id: Some("com.apple.finder".to_string()),
                owner_pid: None,
                title: "Incognito project notes".to_string(),
            },
            WindowContext {
                window_id: 9,
                bundle_id: Some("com.google.Chrome".to_string()),
                owner_pid: None,
                title: "New Incognito Tab - Google Chrome".to_string(),
            },
        ];

        assert_eq!(
            resolve_private_browser_window_id(&privacy, &visible_windows),
            Some(9)
        );
    }

    #[test]
    fn private_browser_window_resolution_requires_known_browser_bundle() {
        let privacy = PrivacySettings {
            private_browser_exclusion_enabled: true,
            ..PrivacySettings::default()
        };
        let visible_windows = vec![WindowContext {
            window_id: 7,
            bundle_id: Some("com.apple.finder".to_string()),
            owner_pid: None,
            title: "Incognito project notes".to_string(),
        }];

        assert_eq!(
            resolve_private_browser_window_id(&privacy, &visible_windows),
            None
        );
    }

    #[test]
    fn title_rules_match_non_browser_windows() {
        let privacy = PrivacySettings {
            private_browser_exclusion_enabled: false,
            browser_title_rules: vec![BrowserTitleRule {
                id: "title-rule".to_string(),
                enabled: true,
                match_type: BrowserTitleRuleMatchType::Substring,
                pattern: "secret".to_string(),
            }],
            ..PrivacySettings::default()
        };
        let context = MetadataContext {
            visible_windows: vec![
                WindowContext {
                    window_id: 7,
                    bundle_id: Some("com.apple.finder".to_string()),
                    owner_pid: None,
                    title: "Secret project notes".to_string(),
                },
                WindowContext {
                    window_id: 9,
                    bundle_id: Some("com.google.Chrome".to_string()),
                    owner_pid: None,
                    title: "Public dashboard".to_string(),
                },
            ],
            ..MetadataContext::default()
        };

        let decision = evaluate_privacy(&privacy, &context);

        assert_eq!(decision.excluded_window_ids, vec![7]);
        assert_eq!(decision.matched_rule_ids, vec!["title-rule"]);
        assert!(decision.privacy_filter_applied);
    }

    #[test]
    fn active_title_rule_excludes_bundle_without_visible_window_context() {
        let privacy = PrivacySettings {
            private_browser_exclusion_enabled: false,
            browser_title_rules: vec![BrowserTitleRule {
                id: "title-rule".to_string(),
                enabled: true,
                match_type: BrowserTitleRuleMatchType::Substring,
                pattern: "secret".to_string(),
            }],
            ..PrivacySettings::default()
        };
        let context = MetadataContext {
            active_bundle_id: Some("com.tinyspeck.slackmacgap".to_string()),
            active_window_id: Some(42),
            active_window_title: Some("Secret workspace".to_string()),
            active_privacy_window_id: None,
            visible_windows: Vec::new(),
            ..MetadataContext::default()
        };

        let decision = evaluate_privacy(&privacy, &context);

        assert_eq!(
            decision.excluded_bundle_ids,
            vec!["com.tinyspeck.slackmacgap"]
        );
        assert_eq!(decision.matched_rule_ids, vec!["title-rule"]);
        assert!(decision.privacy_filter_applied);
    }

    #[test]
    fn active_title_rule_excludes_bundle_when_window_id_is_unavailable() {
        let privacy = PrivacySettings {
            private_browser_exclusion_enabled: false,
            browser_title_rules: vec![BrowserTitleRule {
                id: "title-rule".to_string(),
                enabled: true,
                match_type: BrowserTitleRuleMatchType::Substring,
                pattern: "secret".to_string(),
            }],
            ..PrivacySettings::default()
        };
        let context = MetadataContext {
            active_bundle_id: Some("com.tinyspeck.slackmacgap".to_string()),
            active_window_title: Some("Secret workspace".to_string()),
            visible_windows: Vec::new(),
            ..MetadataContext::default()
        };

        let decision = evaluate_privacy(&privacy, &context);

        assert_eq!(
            decision.excluded_bundle_ids,
            vec!["com.tinyspeck.slackmacgap"]
        );
        assert_eq!(decision.matched_rule_ids, vec!["title-rule"]);
        assert!(decision.privacy_filter_applied);
    }

    #[test]
    fn active_title_rule_without_exclusion_target_does_not_report_match() {
        let privacy = PrivacySettings {
            private_browser_exclusion_enabled: false,
            browser_title_rules: vec![BrowserTitleRule {
                id: "title-rule".to_string(),
                enabled: true,
                match_type: BrowserTitleRuleMatchType::Substring,
                pattern: "secret".to_string(),
            }],
            ..PrivacySettings::default()
        };
        let context = MetadataContext {
            active_window_title: Some("Secret workspace".to_string()),
            visible_windows: Vec::new(),
            ..MetadataContext::default()
        };

        let decision = evaluate_privacy(&privacy, &context);

        assert!(decision.excluded_bundle_ids.is_empty());
        assert!(decision.excluded_window_ids.is_empty());
        assert!(decision.matched_rule_ids.is_empty());
        assert_eq!(decision.metadata_redaction_reason, None);
        assert!(!decision.privacy_filter_applied);
    }

    #[test]
    fn metadata_disabled_still_collects_minimum_privacy_context() {
        let metadata = MetadataSettings {
            enabled: false,
            browser_url_mode: BrowserUrlMode::Sanitized,
        };
        let privacy = PrivacySettings {
            private_browser_exclusion_enabled: true,
            excluded_website_rules: vec![parse_website_rule("website-rule", true, "example.com")],
            browser_title_rules: vec![BrowserTitleRule {
                id: "title-rule".to_string(),
                enabled: true,
                match_type: BrowserTitleRuleMatchType::Substring,
                pattern: "secret".to_string(),
            }],
            ..PrivacySettings::default()
        };

        assert_eq!(
            metadata_collection_plan(&metadata, &privacy),
            MetadataCollectionPlan {
                collect_active_window: true,
                collect_browser_url_for_metadata: false,
                collect_browser_url_for_privacy: true,
                collect_visible_windows: true,
            }
        );
    }

    #[test]
    fn metadata_disabled_with_static_privacy_only_skips_platform_collection() {
        let metadata = MetadataSettings {
            enabled: false,
            browser_url_mode: BrowserUrlMode::Sanitized,
        };
        let privacy = PrivacySettings {
            private_browser_exclusion_enabled: false,
            excluded_apps: vec![ExcludedAppEntry {
                id: "app-rule".to_string(),
                enabled: true,
                bundle_id: "com.example.Secret".to_string(),
                display_name: "Secret".to_string(),
            }],
            ..PrivacySettings::default()
        };

        assert_eq!(
            metadata_collection_plan(&metadata, &privacy),
            MetadataCollectionPlan::default()
        );
    }

    #[test]
    fn native_window_selection_chooses_first_matching_frontmost_pid_window() {
        let windows = vec![
            RawWindowInfo {
                owner_pid: 7,
                window_id: 100,
                layer: 0,
                width: 800.0,
                height: 600.0,
                title: Some("Other".to_string()),
            },
            RawWindowInfo {
                owner_pid: 42,
                window_id: 200,
                layer: 0,
                width: 800.0,
                height: 600.0,
                title: Some("First".to_string()),
            },
            RawWindowInfo {
                owner_pid: 42,
                window_id: 201,
                layer: 0,
                width: 800.0,
                height: 600.0,
                title: Some("Second".to_string()),
            },
        ];

        let selected = select_frontmost_pid_window(&windows, 42).expect("window should match");
        assert_eq!(selected.window_id, 200);
        assert_eq!(selected.title.as_deref(), Some("First"));
    }

    #[test]
    fn native_window_selection_ignores_non_zero_layer_windows() {
        let windows = vec![
            RawWindowInfo {
                owner_pid: 42,
                window_id: 200,
                layer: 1,
                width: 800.0,
                height: 600.0,
                title: Some("Menu".to_string()),
            },
            RawWindowInfo {
                owner_pid: 42,
                window_id: 201,
                layer: 0,
                width: 800.0,
                height: 600.0,
                title: Some("Document".to_string()),
            },
        ];

        let selected = select_frontmost_pid_window(&windows, 42).expect("window should match");
        assert_eq!(selected.window_id, 201);
    }

    #[test]
    fn native_window_selection_ignores_empty_bounds_windows() {
        let windows = vec![
            RawWindowInfo {
                owner_pid: 42,
                window_id: 200,
                layer: 0,
                width: 0.0,
                height: 600.0,
                title: Some("Zero Width".to_string()),
            },
            RawWindowInfo {
                owner_pid: 42,
                window_id: 201,
                layer: 0,
                width: 800.0,
                height: 0.0,
                title: Some("Zero Height".to_string()),
            },
            RawWindowInfo {
                owner_pid: 42,
                window_id: 202,
                layer: 0,
                width: 800.0,
                height: 600.0,
                title: None,
            },
        ];

        let selected = select_frontmost_pid_window(&windows, 42).expect("window should match");
        assert_eq!(selected.window_id, 202);
        assert_eq!(selected.title, None);
    }

    #[test]
    fn native_window_selection_returns_none_without_matching_pid() {
        let windows = vec![RawWindowInfo {
            owner_pid: 7,
            window_id: 100,
            layer: 0,
            width: 800.0,
            height: 600.0,
            title: Some("Other".to_string()),
        }];

        assert_eq!(select_frontmost_pid_window(&windows, 42), None);
    }

    #[test]
    fn metadata_enabled_with_url_off_still_collects_active_window() {
        let metadata = MetadataSettings {
            enabled: true,
            browser_url_mode: BrowserUrlMode::Off,
        };

        assert!(
            metadata_collection_plan(&metadata, &PrivacySettings::default()).collect_active_window
        );
    }

    #[test]
    fn metadata_collection_plan_avoids_unused_browser_and_window_probes() {
        let metadata = MetadataSettings {
            enabled: true,
            browser_url_mode: BrowserUrlMode::Off,
        };
        let privacy = PrivacySettings {
            private_browser_exclusion_enabled: false,
            ..PrivacySettings::default()
        };

        assert_eq!(
            metadata_collection_plan(&metadata, &privacy),
            MetadataCollectionPlan {
                collect_active_window: true,
                collect_browser_url_for_metadata: false,
                collect_browser_url_for_privacy: false,
                collect_visible_windows: false,
            }
        );
    }

    #[test]
    fn metadata_browser_url_collection_uses_metadata_lane_without_privacy_lane() {
        let metadata = MetadataSettings {
            enabled: true,
            browser_url_mode: BrowserUrlMode::Sanitized,
        };
        let privacy = PrivacySettings {
            private_browser_exclusion_enabled: false,
            ..PrivacySettings::default()
        };

        assert_eq!(
            metadata_collection_plan(&metadata, &privacy),
            MetadataCollectionPlan {
                collect_active_window: true,
                collect_browser_url_for_metadata: true,
                collect_browser_url_for_privacy: false,
                collect_visible_windows: false,
            }
        );
    }

    #[test]
    fn browser_url_probe_cache_reuses_recent_results_only_for_same_bundle() {
        let now = Instant::now();
        let cache = BrowserUrlProbeCache::from_probe(
            Some("com.google.Chrome".to_string()),
            Some("https://example.com/path".to_string()),
            now,
        );

        assert_eq!(
            cache.cached_url_for("com.google.Chrome", now + Duration::from_secs(1)),
            Some(Some("https://example.com/path".to_string()))
        );
        assert_eq!(
            cache.cached_url_for("com.apple.Safari", now + Duration::from_secs(1)),
            None
        );
        assert_eq!(
            cache.cached_url_for(
                "com.google.Chrome",
                now + BROWSER_URL_METADATA_POLL_INTERVAL
            ),
            None
        );

        let empty_cache =
            BrowserUrlProbeCache::from_probe(Some("com.google.Chrome".to_string()), None, now);
        assert_eq!(
            empty_cache.cached_url_for("com.google.Chrome", now + Duration::from_secs(1)),
            Some(None)
        );
    }

    #[test]
    fn private_browser_exclusion_requests_visible_window_probe() {
        let metadata = MetadataSettings {
            enabled: true,
            browser_url_mode: BrowserUrlMode::Off,
        };
        let privacy = PrivacySettings::default();

        assert_eq!(
            metadata_collection_plan(&metadata, &privacy),
            MetadataCollectionPlan {
                collect_active_window: true,
                collect_browser_url_for_metadata: false,
                collect_browser_url_for_privacy: false,
                collect_visible_windows: true,
            }
        );
        assert!(active_private_browser_detected(
            &privacy,
            Some("com.google.Chrome"),
            Some("New Incognito Tab - Google Chrome"),
        ));
        assert!(!active_private_browser_detected(
            &privacy,
            Some("com.apple.finder"),
            Some("New Incognito Tab - Google Chrome"),
        ));
    }

    #[test]
    fn browser_support_registry_distinguishes_browser_detection_from_url_support() {
        assert!(is_known_browser_bundle("org.mozilla.firefox"));
        assert!(!browser_url_metadata_supported("org.mozilla.firefox"));
        assert!(browser_url_metadata_supported("com.google.Chrome"));
        assert!(!is_known_browser_bundle("com.apple.finder"));
    }

    #[test]
    fn dynamic_privacy_rules_request_required_platform_probes() {
        let metadata = MetadataSettings {
            enabled: true,
            browser_url_mode: BrowserUrlMode::Off,
        };
        let website_privacy = PrivacySettings {
            private_browser_exclusion_enabled: false,
            excluded_website_rules: vec![parse_website_rule("website-rule", true, "example.com")],
            ..PrivacySettings::default()
        };
        let title_privacy = PrivacySettings {
            private_browser_exclusion_enabled: false,
            browser_title_rules: vec![BrowserTitleRule {
                id: "title-rule".to_string(),
                enabled: true,
                match_type: BrowserTitleRuleMatchType::Substring,
                pattern: "secret".to_string(),
            }],
            ..PrivacySettings::default()
        };

        assert!(
            metadata_collection_plan(&metadata, &website_privacy).collect_browser_url_for_privacy
        );
        assert!(metadata_collection_plan(&metadata, &website_privacy).collect_visible_windows);
        assert!(metadata_collection_plan(&metadata, &title_privacy).collect_visible_windows);
        assert!(metadata_collection_plan(&metadata, &title_privacy).collect_active_window);
    }
}
