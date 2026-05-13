use capture_metadata::MetadataSettings;
use capture_metadata::{
    active_private_browser_detected, apply_metadata_redaction,
    apply_unverified_visible_browser_window_privacy_guard, apply_website_privacy_hold,
    browser_url_script_app_name, evaluate_privacy, is_known_browser_bundle,
    metadata_collection_plan, resolve_active_privacy_window_id, resolve_private_browser_window_id,
    sanitize_url, select_frontmost_pid_window, BrowserUrlProbeCache, FrameMetadataSnapshot,
    MetadataCollectionPlan, MetadataContext, NativeActiveWindowSnapshot, PrivacyFilterDecision,
    PrivacySettings, RawWindowInfo,
};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;
use tauri::Manager;

#[derive(Debug, Clone, Default)]
pub struct CaptureMetadataRuntime {
    latest_snapshot: Option<FrameMetadataSnapshot>,
    latest_decision: PrivacyFilterDecision,
    latest_applied_decision: PrivacyFilterDecision,
    website_privacy_hold_bundle_reasons: BTreeMap<String, String>,
    website_privacy_verified_window_ids: BTreeSet<u32>,
    browser_url_probe_cache: BrowserUrlProbeCache,
}

impl CaptureMetadataRuntime {
    pub fn latest_snapshot(&self) -> Option<FrameMetadataSnapshot> {
        self.latest_snapshot.clone()
    }
}

pub type CaptureMetadataState = Mutex<CaptureMetadataRuntime>;

pub(super) type FrameMetadataSnapshotProvider =
    Arc<dyn Fn() -> Option<FrameMetadataSnapshot> + Send + Sync + 'static>;

pub(super) fn frame_metadata_snapshot_provider(
    app_handle: &tauri::AppHandle,
) -> FrameMetadataSnapshotProvider {
    let app_handle = app_handle.clone();
    Arc::new(move || {
        latest_frame_metadata_snapshot(
            app_handle
                .state::<crate::native_capture::CaptureMetadataState>()
                .inner(),
        )
    })
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePrivacyDebugInfo {
    pub latest_snapshot: Option<FrameMetadataSnapshot>,
    pub latest_decision: PrivacyFilterDecision,
    pub latest_applied_decision: PrivacyFilterDecision,
    pub website_privacy_hold_bundle_ids: Vec<String>,
    pub website_privacy_holds: Vec<WebsitePrivacyHoldDebugInfo>,
    pub currently_excluded_bundle_ids: Vec<String>,
    pub currently_excluded_window_ids: Vec<u32>,
    pub privacy_filter_applied: bool,
    pub metadata_redaction_reason: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebsitePrivacyHoldDebugInfo {
    pub bundle_id: String,
    pub reason: String,
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
            .website_privacy_hold_bundle_reasons
            .keys()
            .cloned()
            .collect(),
        website_privacy_holds: runtime
            .website_privacy_hold_bundle_reasons
            .iter()
            .map(|(bundle_id, reason)| WebsitePrivacyHoldDebugInfo {
                bundle_id: bundle_id.clone(),
                reason: reason.clone(),
            })
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

pub fn latest_applied_privacy_decision(state: &CaptureMetadataState) -> PrivacyFilterDecision {
    state
        .lock()
        .expect("capture metadata state poisoned")
        .latest_applied_decision
        .clone()
}

pub fn reset_recording_session_privacy_state(state: &CaptureMetadataState) {
    let mut runtime = state.lock().expect("capture metadata state poisoned");
    runtime.latest_applied_decision = PrivacyFilterDecision::default();
    runtime.website_privacy_verified_window_ids.clear();
    runtime.browser_url_probe_cache = BrowserUrlProbeCache::default();
}

pub fn refresh_metadata_state(
    state: &CaptureMetadataState,
    metadata: &MetadataSettings,
    privacy: &PrivacySettings,
) -> PrivacyFilterDecision {
    let browser_url_probe_cache = state
        .lock()
        .expect("capture metadata state poisoned")
        .browser_url_probe_cache
        .clone();
    let active = collect_active_window_metadata(metadata, privacy, &browser_url_probe_cache);
    let mut snapshot = metadata.enabled.then(|| active.snapshot.clone()).flatten();
    let context = active.context;
    let mut decision = evaluate_privacy(privacy, &context);

    let mut runtime = state.lock().expect("capture metadata state poisoned");
    apply_website_privacy_hold(
        &mut runtime.website_privacy_hold_bundle_reasons,
        privacy,
        &context,
        &mut decision,
    );
    apply_unverified_visible_browser_window_privacy_guard(
        &mut runtime.website_privacy_verified_window_ids,
        privacy,
        &context,
        &mut decision,
    );
    if let Some(snapshot) = snapshot.as_mut() {
        apply_metadata_redaction(snapshot, privacy, &context, &decision);
    }
    if let Some(browser_url_probe_cache) = active.browser_url_probe_cache {
        runtime.browser_url_probe_cache = browser_url_probe_cache;
    }
    runtime.latest_snapshot = snapshot;
    runtime.latest_decision = decision.clone();
    decision
}

#[cfg(target_os = "macos")]
pub fn start_metadata_notifier(app_handle: tauri::AppHandle) {
    use cidre::ns;

    let mut center = ns::Workspace::shared().notification_center();
    let did_activate_guard = center.add_observer_guard(
        ns::workspace::notification::did_activate_app(),
        None,
        None,
        {
            let app_handle = app_handle.clone();
            move |_notification| {
                refresh_metadata_from_app_settings(&app_handle);
            }
        },
    );
    let active_space_guard = center.add_observer_guard(
        ns::workspace::notification::active_space_did_change(),
        None,
        None,
        {
            let app_handle = app_handle.clone();
            move |_notification| {
                refresh_metadata_from_app_settings(&app_handle);
            }
        },
    );

    replace_metadata_notifier_guards(
        app_handle
            .state::<crate::native_capture::MetadataNotifierState>()
            .inner(),
        vec![did_activate_guard, active_space_guard],
    );
}

#[cfg(not(target_os = "macos"))]
pub fn start_metadata_notifier(_app_handle: tauri::AppHandle) {}

#[cfg(target_os = "macos")]
fn refresh_metadata_from_app_settings(app_handle: &tauri::AppHandle) {
    let settings = app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    refresh_metadata_state(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
        &settings.metadata,
        &settings.privacy,
    );
}

#[cfg(target_os = "macos")]
fn replace_metadata_notifier_guards(
    slot: &crate::native_capture::MetadataNotifierState,
    guards: Vec<cidre::ns::NotificationGuard>,
) {
    slot.replace(guards);
}

#[derive(Debug, Clone)]
struct ActiveWindowMetadata {
    snapshot: Option<FrameMetadataSnapshot>,
    context: MetadataContext,
    browser_url_probe_cache: Option<BrowserUrlProbeCache>,
}

#[cfg(target_os = "macos")]
fn browser_url_probe_for_active_bundle(
    bundle_id: Option<&str>,
    plan: MetadataCollectionPlan,
    cache: &BrowserUrlProbeCache,
    now: Instant,
) -> (Option<String>, Option<BrowserUrlProbeCache>) {
    let Some(bundle_id) = bundle_id.filter(|bundle_id| is_known_browser_bundle(bundle_id)) else {
        return (None, None);
    };
    if plan.collect_browser_url_for_privacy {
        let raw_url = active_browser_url(bundle_id);
        return (
            raw_url.clone(),
            Some(BrowserUrlProbeCache::from_probe(
                Some(bundle_id.to_string()),
                raw_url,
                now,
            )),
        );
    }
    if !plan.collect_browser_url_for_metadata {
        return (None, None);
    }
    if let Some(cached_url) = cache.cached_url_for(bundle_id, now) {
        return (cached_url, None);
    }
    let raw_url = active_browser_url(bundle_id);
    (
        raw_url.clone(),
        Some(BrowserUrlProbeCache::from_probe(
            Some(bundle_id.to_string()),
            raw_url,
            now,
        )),
    )
}

fn collect_active_window_metadata(
    metadata: &MetadataSettings,
    privacy: &PrivacySettings,
    browser_url_probe_cache: &BrowserUrlProbeCache,
) -> ActiveWindowMetadata {
    let plan = metadata_collection_plan(metadata, privacy);
    if !plan.collect_active_window && !plan.collect_visible_windows {
        return ActiveWindowMetadata {
            snapshot: None,
            context: MetadataContext::default(),
            browser_url_probe_cache: None,
        };
    }

    #[cfg(target_os = "macos")]
    {
        let active_window = if plan.collect_active_window {
            collect_native_active_window_snapshot()
        } else {
            NativeActiveWindowSnapshot::default()
        };
        let bundle_id = active_window.bundle_id.clone();
        let app_name = active_window.app_name.clone();
        let window_title = active_window.window_title.clone();
        let (raw_browser_url, browser_url_probe_cache) = browser_url_probe_for_active_bundle(
            bundle_id.as_deref(),
            plan,
            browser_url_probe_cache,
            Instant::now(),
        );
        let snapshot_browser_url = raw_browser_url
            .as_deref()
            .and_then(|url| sanitize_url(url, metadata.browser_url_mode));
        let active_private_browser =
            active_private_browser_detected(privacy, bundle_id.as_deref(), window_title.as_deref());
        let visible_windows = if plan.collect_visible_windows || active_private_browser {
            visible_window_contexts()
        } else {
            Vec::new()
        };
        let private_browser_window_id =
            resolve_private_browser_window_id(privacy, &visible_windows);
        let private_browser_ambiguous_bundle_id = active_private_browser.then(|| {
            bundle_id
                .clone()
                .expect("active private browser requires a known browser bundle id")
        });
        let active_privacy_window_id = resolve_active_privacy_window_id(
            bundle_id.as_deref(),
            active_window.window_id,
            window_title.as_deref(),
            &visible_windows,
        );
        let snapshot = Some(FrameMetadataSnapshot {
            app_bundle_id: bundle_id.clone(),
            app_name,
            window_title: window_title.clone(),
            window_id: active_window.window_id,
            browser_url: snapshot_browser_url,
            display_id: None,
            metadata_redaction_reason: None,
        });
        let context = MetadataContext {
            active_bundle_id: bundle_id.clone(),
            active_window_id: active_window.window_id,
            active_window_title: window_title,
            active_privacy_window_id,
            active_url: raw_browser_url,
            visible_windows,
            private_browser_window_id,
            private_browser_ambiguous_bundle_id,
        };
        return ActiveWindowMetadata {
            snapshot,
            context,
            browser_url_probe_cache,
        };
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = metadata;
        let _ = privacy;
        let _ = plan;
        ActiveWindowMetadata {
            snapshot: None,
            context: MetadataContext::default(),
            browser_url_probe_cache: None,
        }
    }
}

#[cfg(target_os = "macos")]
fn collect_native_active_window_snapshot() -> NativeActiveWindowSnapshot {
    let Some((pid, bundle_id, app_name)) = frontmost_running_app_snapshot() else {
        return NativeActiveWindowSnapshot::default();
    };
    let windows = copy_on_screen_window_infos();
    let active_window = select_frontmost_pid_window(&windows, pid);
    NativeActiveWindowSnapshot {
        bundle_id,
        app_name,
        pid: Some(pid),
        window_id: active_window.map(|window| window.window_id),
        window_title: active_window.and_then(|window| window.title.clone()),
    }
}

#[cfg(target_os = "macos")]
fn frontmost_running_app_snapshot() -> Option<(i32, Option<String>, Option<String>)> {
    let running_apps = cidre::ns::Workspace::shared().running_apps();
    running_apps.iter().find(|app| app.is_active()).map(|app| {
        (
            app.pid(),
            app.bundle_id()
                .map(|bundle_id| bundle_id.to_string())
                .filter(|value| !value.trim().is_empty()),
            app.localized_name()
                .map(|name| name.to_string())
                .filter(|value| !value.trim().is_empty()),
        )
    })
}

#[cfg(target_os = "macos")]
fn copy_on_screen_window_infos() -> Vec<RawWindowInfo> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;
    use core_graphics::window::{
        copy_window_info, kCGWindowBounds, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowName, kCGWindowNumber, kCGWindowOwnerPID,
    };

    let options = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let Some(window_info) = copy_window_info(options, 0) else {
        return Vec::new();
    };

    window_info
        .iter()
        .filter_map(|value| unsafe {
            let dict = CFDictionary::<CFString, CFType>::wrap_under_get_rule(
                *value as core_foundation::dictionary::CFDictionaryRef,
            );
            let owner_pid =
                cf_i64(&dict, kCGWindowOwnerPID).and_then(|value| i32::try_from(value).ok())?;
            let window_id =
                cf_i64(&dict, kCGWindowNumber).and_then(|value| u32::try_from(value).ok())?;
            let layer = cf_i64(&dict, kCGWindowLayer)?;
            let (width, height) = cf_bounds_size(&dict, kCGWindowBounds)?;
            Some(RawWindowInfo {
                owner_pid,
                window_id,
                layer,
                width,
                height,
                title: cf_string(&dict, kCGWindowName),
            })
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn cf_i64(
    dict: &core_foundation::dictionary::CFDictionary<
        core_foundation::string::CFString,
        core_foundation::base::CFType,
    >,
    key: core_foundation::string::CFStringRef,
) -> Option<i64> {
    use core_foundation::base::TCFType;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;

    let key = unsafe { CFString::wrap_under_get_rule(key) };
    let value = dict.find(&key)?;
    let number = unsafe { CFNumber::wrap_under_get_rule(value.as_CFTypeRef() as _) };
    number.to_i64()
}

#[cfg(target_os = "macos")]
fn cf_string(
    dict: &core_foundation::dictionary::CFDictionary<
        core_foundation::string::CFString,
        core_foundation::base::CFType,
    >,
    key: core_foundation::string::CFStringRef,
) -> Option<String> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;

    let key = unsafe { CFString::wrap_under_get_rule(key) };
    let value = dict.find(&key)?;
    let string = unsafe { CFString::wrap_under_get_rule(value.as_CFTypeRef() as _) };
    let string = string.to_string();
    (!string.trim().is_empty()).then_some(string)
}

#[cfg(target_os = "macos")]
fn cf_bounds_size(
    dict: &core_foundation::dictionary::CFDictionary<
        core_foundation::string::CFString,
        core_foundation::base::CFType,
    >,
    key: core_foundation::string::CFStringRef,
) -> Option<(f64, f64)> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;

    let key = unsafe { CFString::wrap_under_get_rule(key) };
    let value = dict.find(&key)?;
    let bounds = unsafe {
        CFDictionary::<CFString, CFType>::wrap_under_get_rule(
            value.as_CFTypeRef() as core_foundation::dictionary::CFDictionaryRef
        )
    };
    Some((cf_f64(&bounds, "Width")?, cf_f64(&bounds, "Height")?))
}

#[cfg(target_os = "macos")]
fn cf_f64(
    dict: &core_foundation::dictionary::CFDictionary<
        core_foundation::string::CFString,
        core_foundation::base::CFType,
    >,
    key: &str,
) -> Option<f64> {
    use core_foundation::base::TCFType;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;

    let key = CFString::new(key);
    let value = dict.find(&key)?;
    let number = unsafe { CFNumber::wrap_under_get_rule(value.as_CFTypeRef() as _) };
    number.to_f64()
}

#[cfg(target_os = "macos")]
fn visible_window_contexts() -> Vec<capture_metadata::WindowContext> {
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
            let bundle_id = window
                .owning_app()
                .map(|app| app.bundle_id().to_string())
                .filter(|value| !value.trim().is_empty());
            let title = window
                .title()
                .map(|title| title.to_string())
                .unwrap_or_default();
            Some(capture_metadata::WindowContext {
                window_id: window.id(),
                bundle_id,
                title,
            })
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn active_browser_url(bundle_id: &str) -> Option<String> {
    let app = browser_url_script_app_name(bundle_id)?;
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
    use capture_metadata::BrowserUrlMode;

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
            private_browser_exclusion_enabled: false,
            ..PrivacySettings::default()
        };

        let state = CaptureMetadataState::default();
        let decision = refresh_metadata_state(&state, &metadata, &privacy);
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
            private_browser_exclusion_enabled: false,
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
    fn reset_recording_session_privacy_state_clears_verified_windows_but_preserves_website_holds() {
        let state = CaptureMetadataState::default();
        {
            let mut runtime = state.lock().expect("capture metadata state should lock");
            runtime.latest_applied_decision = PrivacyFilterDecision {
                excluded_bundle_ids: vec!["net.imput.helium".to_string()],
                privacy_filter_applied: true,
                ..PrivacyFilterDecision::default()
            };
            runtime
                .website_privacy_hold_bundle_reasons
                .insert("net.imput.helium".to_string(), "website_rule".to_string());
            runtime.website_privacy_verified_window_ids.insert(3740);
        }

        reset_recording_session_privacy_state(&state);

        let runtime = state.lock().expect("capture metadata state should lock");
        assert_eq!(
            runtime.latest_applied_decision,
            PrivacyFilterDecision::default()
        );
        assert!(runtime.website_privacy_verified_window_ids.is_empty());
        assert_eq!(
            runtime
                .website_privacy_hold_bundle_reasons
                .get("net.imput.helium")
                .map(String::as_str),
            Some("website_rule")
        );
    }

    #[test]
    fn reset_recording_session_privacy_state_clears_browser_url_probe_cache() {
        let state = CaptureMetadataState::default();
        {
            let mut runtime = state.lock().expect("capture metadata state should lock");
            runtime.browser_url_probe_cache = BrowserUrlProbeCache::from_probe(
                Some("com.google.Chrome".to_string()),
                Some("https://example.com/old-tab".to_string()),
                Instant::now(),
            );
        }

        reset_recording_session_privacy_state(&state);

        let runtime = state.lock().expect("capture metadata state should lock");
        assert_eq!(
            runtime
                .browser_url_probe_cache
                .cached_url_for("com.google.Chrome", Instant::now()),
            None
        );
    }
}
