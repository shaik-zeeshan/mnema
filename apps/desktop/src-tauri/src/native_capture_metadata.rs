use capture_metadata::{
    evaluate_privacy, sanitize_url, website_rule_matches, BrowserUrlMode, FrameMetadataSnapshot,
    MetadataContext, PrivacyFilterDecision, PrivacySettings,
};
use capture_types::MetadataSettings;
use std::collections::BTreeSet;
use std::process::Command;
#[cfg(target_os = "macos")]
use std::sync::mpsc;
use std::sync::Mutex;

#[derive(Debug, Clone, Default)]
pub struct CaptureMetadataRuntime {
    latest_snapshot: Option<FrameMetadataSnapshot>,
    latest_decision: PrivacyFilterDecision,
    latest_applied_decision: PrivacyFilterDecision,
    website_privacy_hold_bundle_ids: BTreeSet<String>,
}

impl CaptureMetadataRuntime {
    pub fn latest_snapshot(&self) -> Option<FrameMetadataSnapshot> {
        self.latest_snapshot.clone()
    }
}

pub type CaptureMetadataState = Mutex<CaptureMetadataRuntime>;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePrivacyDebugInfo {
    pub latest_snapshot: Option<FrameMetadataSnapshot>,
    pub latest_decision: PrivacyFilterDecision,
    pub latest_applied_decision: PrivacyFilterDecision,
    pub website_privacy_hold_bundle_ids: Vec<String>,
    pub currently_excluded_bundle_ids: Vec<String>,
    pub currently_excluded_window_ids: Vec<u32>,
    pub privacy_filter_applied: bool,
    pub metadata_redaction_reason: Option<String>,
}

pub fn latest_frame_metadata_snapshot(
    state: &CaptureMetadataState,
) -> Option<FrameMetadataSnapshot> {
    state
        .lock()
        .expect("capture metadata state poisoned")
        .latest_snapshot()
}

pub fn capture_privacy_debug_info(state: &CaptureMetadataState) -> CapturePrivacyDebugInfo {
    let runtime = state.lock().expect("capture metadata state poisoned");
    CapturePrivacyDebugInfo {
        latest_snapshot: runtime.latest_snapshot.clone(),
        latest_decision: runtime.latest_decision.clone(),
        latest_applied_decision: runtime.latest_applied_decision.clone(),
        website_privacy_hold_bundle_ids: runtime
            .website_privacy_hold_bundle_ids
            .iter()
            .cloned()
            .collect(),
        currently_excluded_bundle_ids: runtime.latest_applied_decision.excluded_bundle_ids.clone(),
        currently_excluded_window_ids: runtime.latest_applied_decision.excluded_window_ids.clone(),
        privacy_filter_applied: runtime.latest_applied_decision.privacy_filter_applied,
        metadata_redaction_reason: runtime
            .latest_applied_decision
            .metadata_redaction_reason
            .clone(),
    }
}

pub fn mark_applied_privacy_decision(
    state: &CaptureMetadataState,
    decision: PrivacyFilterDecision,
) {
    state
        .lock()
        .expect("capture metadata state poisoned")
        .latest_applied_decision = decision;
}

pub fn refresh_metadata_state(
    state: &CaptureMetadataState,
    metadata: &MetadataSettings,
    privacy: &PrivacySettings,
) -> PrivacyFilterDecision {
    let active = collect_active_window_metadata(metadata, privacy);
    let mut snapshot = metadata.enabled.then(|| active.snapshot.clone()).flatten();
    let context = if metadata.enabled {
        active.context
    } else {
        MetadataContext::default()
    };
    let mut decision = if metadata.enabled {
        evaluate_privacy(privacy, &context)
    } else {
        static_app_privacy_decision(privacy)
    };

    let mut runtime = state.lock().expect("capture metadata state poisoned");
    apply_website_privacy_hold(
        &mut runtime.website_privacy_hold_bundle_ids,
        metadata.enabled,
        privacy,
        &context,
        &mut decision,
    );
    if let Some(snapshot) = snapshot.as_mut() {
        apply_metadata_redaction(snapshot, privacy, &context, &decision);
    }
    runtime.latest_snapshot = snapshot;
    runtime.latest_decision = decision.clone();
    decision
}

fn apply_website_privacy_hold(
    held_bundle_ids: &mut BTreeSet<String>,
    metadata_enabled: bool,
    privacy: &PrivacySettings,
    context: &MetadataContext,
    decision: &mut PrivacyFilterDecision,
) {
    if !metadata_enabled || !has_enabled_website_rules(privacy) {
        held_bundle_ids.clear();
        return;
    }

    if let Some(active_bundle_id) = context
        .active_bundle_id
        .as_deref()
        .filter(|bundle_id| is_known_browser_bundle(bundle_id))
    {
        if let Some(active_url) = context.active_url.as_deref() {
            let matched_website_rule = privacy
                .excluded_website_rules
                .iter()
                .any(|rule| website_rule_matches(rule, active_url));
            if matched_website_rule {
                held_bundle_ids.insert(active_bundle_id.to_string());
            } else {
                held_bundle_ids.remove(active_bundle_id);
            }
        }
    }

    for bundle_id in held_bundle_ids.iter() {
        if !decision
            .excluded_bundle_ids
            .iter()
            .any(|excluded| excluded == bundle_id)
        {
            decision.excluded_bundle_ids.push(bundle_id.clone());
        }
    }

    if !held_bundle_ids.is_empty() {
        decision.excluded_bundle_ids.sort();
        decision.excluded_bundle_ids.dedup();
        decision.privacy_filter_applied = true;
        decision
            .metadata_redaction_reason
            .get_or_insert_with(|| "website_rule".to_string());
    }
}

fn has_enabled_website_rules(privacy: &PrivacySettings) -> bool {
    privacy
        .excluded_website_rules
        .iter()
        .any(|rule| rule.enabled)
}

fn has_enabled_browser_title_rules(privacy: &PrivacySettings) -> bool {
    privacy.browser_title_rules.iter().any(|rule| rule.enabled)
}

fn apply_metadata_redaction(
    snapshot: &mut FrameMetadataSnapshot,
    privacy: &PrivacySettings,
    context: &MetadataContext,
    decision: &PrivacyFilterDecision,
) {
    let Some(bundle_id) = snapshot.app_bundle_id.as_deref() else {
        return;
    };
    let decision_excludes_snapshot_bundle = decision
        .excluded_bundle_ids
        .iter()
        .any(|excluded| excluded == bundle_id);
    let active_private_snapshot = privacy.private_browser_exclusion_enabled
        && (snapshot
            .window_title
            .as_deref()
            .is_some_and(is_private_browser_title)
            || context
                .private_browser_ambiguous_bundle_id
                .as_deref()
                .is_some_and(|private_bundle_id| private_bundle_id == bundle_id));
    if !decision_excludes_snapshot_bundle && !active_private_snapshot {
        return;
    }
    let reason = if active_private_snapshot {
        "private_browser".to_string()
    } else {
        decision
            .metadata_redaction_reason
            .clone()
            .unwrap_or_else(|| "privacy_filter".to_string())
    };
    snapshot.metadata_redaction_reason = Some(reason);
    snapshot.window_title = None;
    snapshot.browser_url = None;
}

pub fn static_app_privacy_decision(privacy: &PrivacySettings) -> PrivacyFilterDecision {
    let mut decision = PrivacyFilterDecision::default();
    for app in &privacy.excluded_apps {
        if app.enabled && !app.bundle_id.trim().is_empty() {
            decision
                .excluded_bundle_ids
                .push(app.bundle_id.trim().to_string());
            decision.matched_rule_ids.push(app.id.clone());
        }
    }
    decision.excluded_bundle_ids.sort();
    decision.excluded_bundle_ids.dedup();
    decision.matched_rule_ids.sort();
    decision.matched_rule_ids.dedup();
    decision.privacy_filter_applied = !decision.excluded_bundle_ids.is_empty();
    decision.metadata_redaction_reason = decision
        .privacy_filter_applied
        .then(|| "excluded_app".to_string());
    decision
}

pub fn initial_privacy_decision(
    state: &CaptureMetadataState,
    metadata: &MetadataSettings,
    privacy: &PrivacySettings,
) -> PrivacyFilterDecision {
    refresh_metadata_state(state, metadata, privacy)
}

#[derive(Debug, Clone)]
struct ActiveWindowMetadata {
    snapshot: Option<FrameMetadataSnapshot>,
    context: MetadataContext,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MetadataCollectionPlan {
    collect_active_window: bool,
    collect_browser_url: bool,
    collect_visible_browser_windows: bool,
}

fn metadata_collection_plan(
    metadata: &MetadataSettings,
    privacy: &PrivacySettings,
) -> MetadataCollectionPlan {
    if !metadata.enabled {
        return MetadataCollectionPlan::default();
    }

    MetadataCollectionPlan {
        collect_active_window: true,
        collect_browser_url: metadata.browser_url_mode != BrowserUrlMode::Off
            || has_enabled_website_rules(privacy),
        collect_visible_browser_windows: has_enabled_browser_title_rules(privacy),
    }
}

fn active_private_browser_detected(
    privacy: &PrivacySettings,
    bundle_id: Option<&str>,
    window_title: Option<&str>,
) -> bool {
    privacy.private_browser_exclusion_enabled
        && bundle_id.is_some_and(is_known_browser_bundle)
        && window_title.is_some_and(is_private_browser_title)
}

fn collect_active_window_metadata(
    metadata: &MetadataSettings,
    privacy: &PrivacySettings,
) -> ActiveWindowMetadata {
    let plan = metadata_collection_plan(metadata, privacy);
    if !plan.collect_active_window && !plan.collect_visible_browser_windows {
        return ActiveWindowMetadata {
            snapshot: None,
            context: MetadataContext::default(),
        };
    }

    #[cfg(target_os = "macos")]
    {
        let output = if plan.collect_active_window {
            let script = r#"tell application "System Events"
	set frontProc to first application process whose frontmost is true
	set appName to name of frontProc
	set bundleId to bundle identifier of frontProc
set windowTitle to ""
try
  set windowTitle to name of front window of frontProc
	end try
	return bundleId & linefeed & appName & linefeed & windowTitle
	end tell"#;
            run_osascript(script)
        } else {
            String::new()
        };
        let mut lines = output.lines();
        let bundle_id = lines
            .next()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        let app_name = lines
            .next()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        let window_title = lines
            .next()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        let raw_browser_url = plan
            .collect_browser_url
            .then(|| bundle_id.as_deref().and_then(active_browser_url))
            .flatten();
        let snapshot_browser_url = raw_browser_url
            .as_deref()
            .and_then(|url| sanitize_url(url, metadata.browser_url_mode));
        let active_private_browser =
            active_private_browser_detected(privacy, bundle_id.as_deref(), window_title.as_deref());
        let visible_browser_windows =
            if plan.collect_visible_browser_windows || active_private_browser {
                visible_browser_windows()
            } else {
                Vec::new()
            };
        let private_browser_window_id = privacy
            .private_browser_exclusion_enabled
            .then(|| {
                visible_browser_windows
                    .iter()
                    .find(|window| is_private_browser_title(&window.title))
                    .map(|window| window.window_id)
            })
            .flatten();
        let private_browser_ambiguous_bundle_id = active_private_browser.then(|| {
            bundle_id
                .clone()
                .expect("active private browser requires a known browser bundle id")
        });
        let snapshot = Some(FrameMetadataSnapshot {
            app_bundle_id: bundle_id.clone(),
            app_name,
            window_title,
            browser_url: snapshot_browser_url,
            display_id: None,
            metadata_redaction_reason: None,
        });
        let context = MetadataContext {
            active_bundle_id: bundle_id.clone(),
            active_url: raw_browser_url,
            visible_browser_windows,
            private_browser_window_id,
            private_browser_ambiguous_bundle_id,
        };
        return ActiveWindowMetadata { snapshot, context };
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = metadata;
        let _ = privacy;
        let _ = plan;
        ActiveWindowMetadata {
            snapshot: None,
            context: MetadataContext::default(),
        }
    }
}

fn is_known_browser_bundle(bundle_id: &str) -> bool {
    matches!(
        bundle_id,
        "com.apple.Safari"
            | "com.google.Chrome"
            | "com.google.Chrome.canary"
            | "com.microsoft.edgemac"
            | "org.mozilla.firefox"
            | "com.brave.Browser"
            | "company.thebrowser.Browser"
            | "net.imput.helium"
    )
}

fn is_private_browser_title(title: &str) -> bool {
    const PRIVATE_TITLE_PATTERNS: &[&str] =
        &["incognito", "private browsing", "inprivate", "(private)"];
    let title = title.to_ascii_lowercase();
    PRIVATE_TITLE_PATTERNS
        .iter()
        .any(|pattern| title.contains(pattern))
}

#[cfg(target_os = "macos")]
fn visible_browser_windows() -> Vec<capture_metadata::BrowserWindowContext> {
    const BROWSER_BUNDLES: &[&str] = &[
        "com.apple.Safari",
        "com.google.Chrome",
        "com.google.Chrome.canary",
        "com.microsoft.edgemac",
        "org.mozilla.firefox",
        "com.brave.Browser",
        "company.thebrowser.Browser",
        "net.imput.helium",
    ];

    let (tx, rx) = mpsc::channel();
    cidre::sc::ShareableContent::current_with_ch(move |content, error| {
        let result = match (content, error) {
            (Some(content), None) => Ok(content.retained()),
            _ => Err(()),
        };
        let _ = tx.send(result);
    });
    let Ok(Ok(content)) = rx.recv_timeout(std::time::Duration::from_secs(1)) else {
        return Vec::new();
    };

    content
        .windows()
        .iter()
        .filter(|window| window.is_on_screen())
        .filter_map(|window| {
            let app = window.owning_app()?;
            let bundle_id = app.bundle_id().to_string();
            if !BROWSER_BUNDLES.contains(&bundle_id.as_str()) {
                return None;
            }
            let title = window
                .title()
                .map(|title| title.to_string())
                .unwrap_or_default();
            Some(capture_metadata::BrowserWindowContext {
                window_id: window.id(),
                title,
            })
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn active_browser_url(bundle_id: &str) -> Option<String> {
    let app = match bundle_id {
        "com.apple.Safari" => "Safari",
        "com.google.Chrome" => "Google Chrome",
        "com.google.Chrome.canary" => "Google Chrome Canary",
        "com.microsoft.edgemac" => "Microsoft Edge",
        "com.brave.Browser" => "Brave Browser",
        "company.thebrowser.Browser" => "Arc",
        "net.imput.helium" => "Helium",
        _ => return None,
    };
    let script = format!(
        r#"tell application "{app}"
try
  return URL of active tab of front window
on error
  return ""
end try
end tell"#
    );
    run_osascript(&script)
        .trim()
        .split('\n')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(target_os = "macos")]
fn run_osascript(script: &str) -> String {
    let Ok(mut child) = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return String::new();
    };

    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let Ok(output) = child.wait_with_output() else {
                    return String::new();
                };
                return String::from_utf8_lossy(&output.stdout).to_string();
            }
            Ok(Some(_)) => return String::new(),
            Ok(None) if started.elapsed() >= std::time::Duration::from_secs(1) => {
                let _ = child.kill();
                let _ = child.wait();
                return String::new();
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
            Err(_) => return String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_metadata::{parse_website_rule, FrameMetadataSnapshot, MetadataContext};

    fn website_privacy(pattern: &str) -> PrivacySettings {
        PrivacySettings {
            excluded_website_rules: vec![parse_website_rule("website-rule", true, pattern)],
            ..PrivacySettings::default()
        }
    }

    #[test]
    fn website_privacy_hold_keeps_browser_excluded_after_leaving_browser() {
        let privacy = website_privacy("*.infinityapp.in");
        let mut held_bundle_ids = BTreeSet::new();
        let mut decision = PrivacyFilterDecision::default();
        let browser_context = MetadataContext {
            active_bundle_id: Some("net.imput.helium".to_string()),
            active_url: Some("https://dashboard.infinityapp.in/app/dashboard".to_string()),
            ..MetadataContext::default()
        };

        apply_website_privacy_hold(
            &mut held_bundle_ids,
            true,
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
            &mut held_bundle_ids,
            true,
            &privacy,
            &non_browser_context,
            &mut decision,
        );

        assert_eq!(decision.excluded_bundle_ids, vec!["net.imput.helium"]);
        assert!(decision.privacy_filter_applied);
        assert_eq!(
            decision.metadata_redaction_reason.as_deref(),
            Some("website_rule")
        );
    }

    #[test]
    fn website_privacy_hold_clears_after_successful_non_matching_browser_probe() {
        let privacy = website_privacy("*.infinityapp.in");
        let mut held_bundle_ids = BTreeSet::from(["net.imput.helium".to_string()]);
        let mut decision = PrivacyFilterDecision::default();
        let browser_context = MetadataContext {
            active_bundle_id: Some("net.imput.helium".to_string()),
            active_url: Some("https://example.com/".to_string()),
            ..MetadataContext::default()
        };

        apply_website_privacy_hold(
            &mut held_bundle_ids,
            true,
            &privacy,
            &browser_context,
            &mut decision,
        );

        assert!(held_bundle_ids.is_empty());
        assert!(decision.excluded_bundle_ids.is_empty());
        assert!(!decision.privacy_filter_applied);
    }

    #[test]
    fn active_excluded_browser_metadata_redacts_title_and_url() {
        let mut snapshot = FrameMetadataSnapshot {
            app_bundle_id: Some("net.imput.helium".to_string()),
            app_name: Some("Helium".to_string()),
            window_title: Some("Infinity - Helium".to_string()),
            browser_url: Some("https://dashboard.infinityapp.in/app/dashboard".to_string()),
            display_id: None,
            metadata_redaction_reason: None,
        };
        let decision = PrivacyFilterDecision {
            excluded_bundle_ids: vec!["net.imput.helium".to_string()],
            metadata_redaction_reason: Some("website_rule".to_string()),
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
            Some("website_rule")
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
    fn initial_privacy_decision_includes_static_apps_when_metadata_is_disabled() {
        let metadata = MetadataSettings {
            enabled: false,
            browser_url_mode: BrowserUrlMode::Sanitized,
        };
        let privacy = PrivacySettings {
            excluded_apps: vec![capture_metadata::ExcludedAppEntry {
                id: "app".to_string(),
                enabled: true,
                bundle_id: "com.secret".to_string(),
                display_name: "Secret".to_string(),
            }],
            ..PrivacySettings::default()
        };

        let state = CaptureMetadataState::default();
        let decision = initial_privacy_decision(&state, &metadata, &privacy);
        let runtime = state.lock().expect("capture metadata state should lock");

        assert_eq!(decision.excluded_bundle_ids, vec!["com.secret"]);
        assert_eq!(
            decision.metadata_redaction_reason.as_deref(),
            Some("excluded_app")
        );
        assert_eq!(runtime.latest_decision, decision);
        assert!(runtime.latest_snapshot.is_none());
    }

    #[test]
    fn metadata_disabled_skips_all_platform_collection() {
        let metadata = MetadataSettings {
            enabled: false,
            browser_url_mode: BrowserUrlMode::Sanitized,
        };
        let privacy = PrivacySettings {
            private_browser_exclusion_enabled: true,
            excluded_website_rules: vec![parse_website_rule("website-rule", true, "example.com")],
            browser_title_rules: vec![capture_metadata::BrowserTitleRule {
                id: "title-rule".to_string(),
                enabled: true,
                match_type: capture_metadata::BrowserTitleRuleMatchType::Substring,
                pattern: "secret".to_string(),
            }],
            ..PrivacySettings::default()
        };

        assert_eq!(
            metadata_collection_plan(&metadata, &privacy),
            MetadataCollectionPlan::default()
        );
    }

    #[test]
    fn refresh_with_metadata_disabled_keeps_static_privacy_without_snapshot() {
        let state = CaptureMetadataState::default();
        let metadata = MetadataSettings {
            enabled: false,
            browser_url_mode: BrowserUrlMode::Sanitized,
        };
        let privacy = PrivacySettings {
            excluded_apps: vec![capture_metadata::ExcludedAppEntry {
                id: "app-rule".to_string(),
                enabled: true,
                bundle_id: "com.example.Secret".to_string(),
                display_name: "Secret".to_string(),
            }],
            private_browser_exclusion_enabled: true,
            excluded_website_rules: vec![parse_website_rule("website-rule", true, "example.com")],
            ..PrivacySettings::default()
        };

        let decision = refresh_metadata_state(&state, &metadata, &privacy);
        let runtime = state.lock().expect("capture metadata state should lock");

        assert_eq!(decision.excluded_bundle_ids, vec!["com.example.Secret"]);
        assert_eq!(
            decision.metadata_redaction_reason.as_deref(),
            Some("excluded_app")
        );
        assert!(runtime.latest_snapshot.is_none());
        assert_eq!(runtime.latest_decision, decision);
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
                collect_browser_url: false,
                collect_visible_browser_windows: false,
            }
        );
    }

    #[test]
    fn private_browser_detection_does_not_request_all_window_probe_by_default() {
        let metadata = MetadataSettings {
            enabled: true,
            browser_url_mode: BrowserUrlMode::Off,
        };
        let privacy = PrivacySettings::default();

        assert_eq!(
            metadata_collection_plan(&metadata, &privacy),
            MetadataCollectionPlan {
                collect_active_window: true,
                collect_browser_url: false,
                collect_visible_browser_windows: false,
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
            browser_title_rules: vec![capture_metadata::BrowserTitleRule {
                id: "title-rule".to_string(),
                enabled: true,
                match_type: capture_metadata::BrowserTitleRuleMatchType::Substring,
                pattern: "secret".to_string(),
            }],
            ..PrivacySettings::default()
        };

        assert!(metadata_collection_plan(&metadata, &website_privacy).collect_browser_url);
        assert!(
            metadata_collection_plan(&metadata, &title_privacy).collect_visible_browser_windows
        );
    }
}
