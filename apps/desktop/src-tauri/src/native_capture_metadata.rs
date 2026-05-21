use capture_metadata::MetadataSettings;
use capture_metadata::{
    browser_url_script_app_name, evaluate_privacy, is_known_browser_bundle,
    metadata_collection_plan, sanitize_url, select_frontmost_pid_window, BrowserUrlProbeCache,
    FrameMetadataSnapshot, MetadataCollectionPlan, MetadataContext, NativeActiveWindowSnapshot,
    PrivacyFilterDecision, PrivacySettings, RawWindowInfo,
};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;
use tauri::Manager;

#[derive(Debug, Clone, Default)]
pub struct CaptureMetadataRuntime {
    latest_snapshot: Option<FrameMetadataSnapshot>,
    latest_decision: PrivacyFilterDecision,
    latest_applied_decision: PrivacyFilterDecision,
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
    pub currently_excluded_bundle_ids: Vec<String>,
    pub privacy_filter_applied: bool,
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
        currently_excluded_bundle_ids: runtime.latest_applied_decision.excluded_bundle_ids.clone(),
        privacy_filter_applied: runtime.latest_applied_decision.privacy_filter_applied,
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
    runtime.latest_snapshot = None;
    runtime.latest_decision = PrivacyFilterDecision::default();
    runtime.latest_applied_decision = PrivacyFilterDecision::default();
    runtime.browser_url_probe_cache = BrowserUrlProbeCache::default();
}

pub fn refresh_metadata_state(
    app_handle: Option<&tauri::AppHandle>,
    state: &CaptureMetadataState,
    metadata: &MetadataSettings,
    privacy: &PrivacySettings,
) -> PrivacyFilterDecision {
    let browser_url_probe_cache = state
        .lock()
        .expect("capture metadata state poisoned")
        .browser_url_probe_cache
        .clone();
    let extension_url = app_handle
        .and_then(crate::native_capture::browser_integration::latest_browser_extension_metadata_url);
    let active = collect_active_window_metadata(
        metadata,
        privacy,
        &browser_url_probe_cache,
        extension_url,
    );
    let snapshot = metadata.enabled.then(|| active.snapshot.clone()).flatten();
    let context = active.context;
    let decision = evaluate_privacy(privacy, &context);

    let mut runtime = state.lock().expect("capture metadata state poisoned");
    if let Some(browser_url_probe_cache) = active.browser_url_probe_cache {
        runtime.browser_url_probe_cache = browser_url_probe_cache;
    }
    runtime.latest_snapshot = snapshot;
    runtime.latest_decision = decision.clone();
    decision
}

pub fn refresh_static_excluded_app_privacy_state(
    state: &CaptureMetadataState,
    privacy: &PrivacySettings,
) -> PrivacyFilterDecision {
    let decision = evaluate_privacy(privacy, &MetadataContext::default());
    state
        .lock()
        .expect("capture metadata state poisoned")
        .latest_decision = decision.clone();
    decision
}

#[cfg(target_os = "macos")]
pub fn start_metadata_notifier(app_handle: tauri::AppHandle) {
    use cidre::ns;

    let mut center = ns::Workspace::shared().notification_center();
    let mut guards = Vec::new();
    for (notification, reason) in [
        (
            ns::workspace::notification::did_activate_app(),
            crate::native_capture::privacy::PrivacyRefreshReason::WorkspaceFocusChanged,
        ),
        (
            ns::workspace::notification::did_deactivate_app(),
            crate::native_capture::privacy::PrivacyRefreshReason::WorkspaceFocusChanged,
        ),
        (
            ns::workspace::notification::active_space_did_change(),
            crate::native_capture::privacy::PrivacyRefreshReason::WorkspaceFocusChanged,
        ),
        (
            ns::workspace::notification::did_launch_app(),
            crate::native_capture::privacy::PrivacyRefreshReason::WorkspaceAppChanged,
        ),
        (
            ns::workspace::notification::did_terminate_app(),
            crate::native_capture::privacy::PrivacyRefreshReason::WorkspaceAppChanged,
        ),
        (
            ns::workspace::notification::did_hide_app(),
            crate::native_capture::privacy::PrivacyRefreshReason::WorkspaceAppChanged,
        ),
        (
            ns::workspace::notification::did_unhide_app(),
            crate::native_capture::privacy::PrivacyRefreshReason::WorkspaceAppChanged,
        ),
    ] {
        guards.push(center.add_observer_guard(notification, None, None, {
            let app_handle = app_handle.clone();
            move |_notification| {
                crate::native_capture::privacy::request_privacy_filter_refresh(&app_handle, reason);
                crate::native_capture::request_capture_safety_check(&app_handle);
            }
        }));
    }

    replace_metadata_notifier_guards(
        app_handle
            .state::<crate::native_capture::MetadataNotifierState>()
            .inner(),
        guards,
    );
}

#[cfg(not(target_os = "macos"))]
pub fn start_metadata_notifier(_app_handle: tauri::AppHandle) {}

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
    _privacy: &PrivacySettings,
    browser_url_probe_cache: &BrowserUrlProbeCache,
    extension_browser_url: Option<String>,
) -> ActiveWindowMetadata {
    let plan = metadata_collection_plan(metadata);
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
        let (raw_browser_url, browser_url_probe_cache) =
            if let Some(extension_browser_url) = extension_browser_url {
                (Some(extension_browser_url), None)
            } else {
                browser_url_probe_for_active_bundle(
                    bundle_id.as_deref(),
                    plan,
                    browser_url_probe_cache,
                    Instant::now(),
                )
            };
        let snapshot_browser_url = raw_browser_url
            .as_deref()
            .and_then(|url| sanitize_url(url, metadata.browser_url_mode));
        let visible_windows = Vec::new();
        let snapshot = Some(FrameMetadataSnapshot {
            app_bundle_id: bundle_id.clone(),
            app_name,
            window_title: window_title.clone(),
            window_id: active_window.window_id,
            browser_url: snapshot_browser_url,
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        });
        let context = MetadataContext {
            active_bundle_id: bundle_id.clone(),
            active_window_id: active_window.window_id,
            active_window_title: window_title,
            active_privacy_window_id: None,
            active_url: raw_browser_url,
            visible_windows,
            private_browser_window_ids: Vec::new(),
            private_browser_ambiguous_bundle_id: None,
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
        let _ = _privacy;
        let _ = plan;
        ActiveWindowMetadata {
            snapshot: None,
            context: MetadataContext::default(),
            browser_url_probe_cache: None,
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn collect_native_active_window_snapshot() -> NativeActiveWindowSnapshot {
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
        };

        let state = CaptureMetadataState::default();
        let decision = refresh_metadata_state(None, &state, &metadata, &privacy);
        let runtime = state.lock().expect("capture metadata state should lock");

        assert_eq!(decision.excluded_bundle_ids, vec!["com.secret"]);
        assert!(decision.metadata_redaction_reason.is_none());
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
        };

        let decision = refresh_metadata_state(None, &state, &metadata, &privacy);
        let runtime = state.lock().expect("capture metadata state should lock");

        assert_eq!(decision.excluded_bundle_ids, vec!["com.example.Secret"]);
        assert!(decision.metadata_redaction_reason.is_none());
        assert!(runtime.latest_snapshot.is_none());
        assert_eq!(runtime.latest_decision, decision);
    }

    #[test]
    fn reset_recording_session_privacy_state_clears_verified_windows_and_website_holds() {
        let state = CaptureMetadataState::default();
        {
            let mut runtime = state.lock().expect("capture metadata state should lock");
            runtime.latest_snapshot = Some(FrameMetadataSnapshot {
                app_bundle_id: Some("net.imput.helium".to_string()),
                app_name: Some("Helium".to_string()),
                window_title: Some("Private window".to_string()),
                browser_url: Some("https://secret.example".to_string()),
                ..FrameMetadataSnapshot::default()
            });
            runtime.latest_decision = PrivacyFilterDecision {
                excluded_bundle_ids: vec!["net.imput.helium".to_string()],
                privacy_filter_applied: true,
                ..PrivacyFilterDecision::default()
            };
            runtime.latest_applied_decision = PrivacyFilterDecision {
                excluded_bundle_ids: vec!["net.imput.helium".to_string()],
                privacy_filter_applied: true,
                ..PrivacyFilterDecision::default()
            };
        }

        reset_recording_session_privacy_state(&state);

        let runtime = state.lock().expect("capture metadata state should lock");
        assert!(runtime.latest_snapshot.is_none());
        assert_eq!(runtime.latest_decision, PrivacyFilterDecision::default());
        assert_eq!(
            runtime.latest_applied_decision,
            PrivacyFilterDecision::default()
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
