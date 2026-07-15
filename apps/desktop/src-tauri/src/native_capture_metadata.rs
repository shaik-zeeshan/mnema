#[cfg(target_os = "macos")]
use capture_metadata::{browser_url_applescript, browser_url_strategy, BrowserUrlStrategy};
#[cfg(any(test, target_os = "macos"))]
use capture_metadata::{
    evaluate_privacy, metadata_collection_plan, MetadataContext, MetadataSettings, PrivacySettings,
};
// `MetadataSettings` above is gated to macOS/test; the Windows metadata-only
// refresh (ADR 0043, issue #139) needs it too, without pulling the macOS
// privacy-engine imports. `not(test)` avoids a double import in a Windows test
// build (where the `any(test, ...)` import above is already active).
#[cfg(all(target_os = "windows", not(test)))]
use capture_metadata::MetadataSettings;
#[cfg(target_os = "macos")]
use capture_metadata::{
    browser_url_script_app_name, is_known_browser_bundle, sanitize_url,
    select_frontmost_pid_window, NativeActiveWindowSnapshot, RawWindowInfo,
};
// `MetadataCollectionPlan` is shared by the macOS collector and the pure Windows
// browser-URL gating fn (`windows_browser_url_probe_engine`), so it needs one
// combined gate rather than the macOS-only import — `any(test, ...)` keeps it in
// scope for the host-agnostic unit tests. `BrowserEngine` +
// `known_browser_engine_for_exe_stem` + `app_display_name_from_exe_path` back only
// the Windows browser-URL path (and its tests), so they are gated to Windows/test
// to avoid unused-import warnings on macOS/Linux.
#[cfg(any(test, target_os = "macos", target_os = "windows"))]
use capture_metadata::MetadataCollectionPlan;
#[cfg(any(test, target_os = "windows"))]
use capture_metadata::{
    app_display_name_from_exe_path, known_browser_engine_for_exe_stem, BrowserEngine,
};
use capture_metadata::{BrowserUrlProbeCache, FrameMetadataSnapshot, PrivacyFilterDecision};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
#[cfg(any(test, target_os = "macos"))]
use std::time::Instant;
use tauri::Manager;

/// How many `(published_at_unix_ms, snapshot)` entries to retain. The ring must
/// span the capture→JPEG-write lag (~100-300ms nominally, up to ~1-2s under load):
/// `snapshot_in_effect_at` is evaluated when the export callback fires, so the
/// entry that was in effect at a frame's capture instant has to survive until
/// then. Publishes come from the ~1 Hz poll plus focus changes — each focus change
/// is ~2 publishes (a Cached pre-pass then a Live pass), coalesced by the
/// single-collector guard in `privacy.rs`. 32 keeps the in-effect entry clear of
/// eviction even under a burst of switches while the lag is stretched; an evicted
/// entry only degrades to `None` (never a wrong label), and entries are <1KB and
/// clear on session reset.
/// ponytail: fixed ring, not a general time-series store.
const SNAPSHOT_HISTORY_CAP: usize = 32;

#[derive(Debug, Clone, Default)]
pub struct CaptureMetadataRuntime {
    /// Recent `(published_at_unix_ms, snapshot)` entries, oldest first. Used to
    /// stamp each frame with the app that was frontmost at the frame's *capture*
    /// instant rather than at JPEG-write-completion (which flips to the app the
    /// user switched TO on boundary frames). The newest entry is the latest
    /// snapshot — there is no separate `latest_snapshot` field to keep in lockstep.
    snapshot_history: std::collections::VecDeque<(u64, Option<FrameMetadataSnapshot>)>,
    latest_decision: PrivacyFilterDecision,
    latest_applied_decision: PrivacyFilterDecision,
    browser_url_probe_cache: BrowserUrlProbeCache,
}

impl CaptureMetadataRuntime {
    pub fn latest_snapshot(&self) -> Option<FrameMetadataSnapshot> {
        self.snapshot_history
            .back()
            .and_then(|(_, snapshot)| snapshot.clone())
    }

    fn publish_snapshot(&mut self, at_unix_ms: u64, snapshot: Option<FrameMetadataSnapshot>) {
        self.snapshot_history.push_back((at_unix_ms, snapshot));
        while self.snapshot_history.len() > SNAPSHOT_HISTORY_CAP {
            self.snapshot_history.pop_front();
        }
    }

    /// The snapshot published at or before `at_unix_ms` (most recent such entry).
    /// Returns `None` for frames captured before the first snapshot was published
    /// (session start), matching the pre-history behavior.
    fn snapshot_in_effect_at(&self, at_unix_ms: u64) -> Option<FrameMetadataSnapshot> {
        self.snapshot_history
            .iter()
            .rev()
            .find(|(published_at, _)| *published_at <= at_unix_ms)
            .and_then(|(_, snapshot)| snapshot.clone())
    }
}

pub type CaptureMetadataState = Mutex<CaptureMetadataRuntime>;

/// Whether a metadata refresh may issue a *live* active-tab browser-URL read.
///
/// The Gecko (Firefox/Zen) URL is read via the Accessibility API
/// ([`crate::native_capture::browser_url_ax::read_active_tab_url`]), which is
/// wall-clock bounded but can still cost up to ~1.4s on a cold/slow read. That
/// read is fine on the periodic metadata-refresh tick (its own background
/// thread, off every capture lock), but several synchronous capture-lifecycle
/// paths — segment rotation, inactivity/user resume, suspension and post-wake
/// recovery — call into metadata collection *while holding the
/// `NativeCaptureState` mutex*. Letting the live read run there would stall
/// stop/refresh/pause/resume for as long as the read takes.
///
/// On those synchronous, lock-holding paths we pass [`Live::Cached`] so the
/// probe serves the last cached URL (or `None`) and never issues an AX/AppleScript
/// read. The privacy decision itself is app-bundle-only ([`evaluate_privacy`]
/// ignores the URL), so the cached value is sufficient for the filter; the live
/// Gecko URL is picked up by the next off-lock metadata tick (~1s cadence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserUrlReadMode {
    /// Issue a live browser-URL read on a cache miss. Only safe off every
    /// capture lock — used by the periodic metadata-refresh tick.
    Live,
    /// Never issue a live read; serve the cached URL (or `None`). Used on
    /// synchronous paths that hold the `NativeCaptureState` mutex.
    Cached,
}

pub(super) type FrameMetadataSnapshotProvider =
    Arc<dyn Fn(u64) -> Option<FrameMetadataSnapshot> + Send + Sync + 'static>;

pub(super) fn frame_metadata_snapshot_provider(
    app_handle: &tauri::AppHandle,
) -> FrameMetadataSnapshotProvider {
    let app_handle = app_handle.clone();
    Arc::new(move |captured_at_unix_ms| {
        frame_metadata_snapshot_in_effect_at(
            app_handle
                .state::<crate::native_capture::CaptureMetadataState>()
                .inner(),
            captured_at_unix_ms,
        )
    })
}

/// The frame-metadata snapshot that was in effect at `captured_at_unix_ms` — the
/// frame's *capture* instant, not JPEG-write-completion. Fixes the boundary-frame
/// mislabel where the last frame before an app switch got stamped with the app
/// switched TO.
pub fn frame_metadata_snapshot_in_effect_at(
    state: &CaptureMetadataState,
    captured_at_unix_ms: u64,
) -> Option<FrameMetadataSnapshot> {
    state
        .lock()
        .expect("capture metadata state poisoned")
        .snapshot_in_effect_at(captured_at_unix_ms)
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

pub fn capture_privacy_debug_info(state: &CaptureMetadataState) -> CapturePrivacyDebugInfo {
    let runtime = state.lock().expect("capture metadata state poisoned");
    CapturePrivacyDebugInfo {
        latest_snapshot: runtime.latest_snapshot(),
        latest_decision: runtime.latest_decision.clone(),
        latest_applied_decision: runtime.latest_applied_decision.clone(),
        currently_excluded_bundle_ids: runtime.latest_applied_decision.excluded_bundle_ids.clone(),
        privacy_filter_applied: runtime.latest_applied_decision.privacy_filter_applied,
    }
}

#[cfg(target_os = "macos")]
pub fn mark_applied_privacy_decision(
    state: &CaptureMetadataState,
    decision: PrivacyFilterDecision,
) {
    state
        .lock()
        .expect("capture metadata state poisoned")
        .latest_applied_decision = decision;
}

#[cfg(target_os = "macos")]
pub fn latest_applied_privacy_decision(state: &CaptureMetadataState) -> PrivacyFilterDecision {
    state
        .lock()
        .expect("capture metadata state poisoned")
        .latest_applied_decision
        .clone()
}

pub fn reset_recording_session_privacy_state(state: &CaptureMetadataState) {
    let mut runtime = state.lock().expect("capture metadata state poisoned");
    runtime.snapshot_history.clear();
    runtime.latest_decision = PrivacyFilterDecision::default();
    runtime.latest_applied_decision = PrivacyFilterDecision::default();
    runtime.browser_url_probe_cache = BrowserUrlProbeCache::default();
}

#[cfg(any(test, target_os = "macos"))]
pub fn refresh_metadata_state(
    state: &CaptureMetadataState,
    metadata: &MetadataSettings,
    privacy: &PrivacySettings,
    browser_url_read_mode: BrowserUrlReadMode,
) -> PrivacyFilterDecision {
    let browser_url_probe_cache = state
        .lock()
        .expect("capture metadata state poisoned")
        .browser_url_probe_cache
        .clone();
    let active = collect_active_window_metadata(
        metadata,
        privacy,
        &browser_url_probe_cache,
        browser_url_read_mode,
    );
    let snapshot = metadata.enabled.then(|| active.snapshot.clone()).flatten();
    let context = active.context;
    let decision = evaluate_privacy(privacy, &context);

    let mut runtime = state.lock().expect("capture metadata state poisoned");
    if let Some(browser_url_probe_cache) = active.browser_url_probe_cache {
        runtime.browser_url_probe_cache = browser_url_probe_cache;
    }
    runtime.publish_snapshot(crate::native_capture::runtime::now_unix_ms(), snapshot);
    runtime.latest_decision = decision.clone();
    decision
}

#[cfg(target_os = "macos")]
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

/// Windows: install the foreground-change listener (ADR 0043, issue #141) so the
/// active-window snapshot refreshes the instant focus changes — keeping per-frame
/// app attribution correct across sub-second app switches. Mirrors the macOS seam:
/// installed once at startup, its refresh no-ops while not recording, and the 1s
/// segment-loop poll (#139) stays as the fallback. Teardown at app exit is wired
/// through `foreground_listener::stop_windows_foreground_listener`.
#[cfg(target_os = "windows")]
pub fn start_metadata_notifier(app_handle: tauri::AppHandle) {
    crate::native_capture::foreground_listener::start_windows_foreground_listener(app_handle);
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn start_metadata_notifier(_app_handle: tauri::AppHandle) {}

#[cfg(target_os = "macos")]
fn replace_metadata_notifier_guards(
    slot: &crate::native_capture::MetadataNotifierState,
    guards: Vec<cidre::ns::NotificationGuard>,
) {
    slot.replace(guards);
}

#[cfg(any(test, target_os = "macos"))]
#[derive(Debug, Clone)]
struct ActiveWindowMetadata {
    snapshot: Option<FrameMetadataSnapshot>,
    context: MetadataContext,
    browser_url_probe_cache: Option<BrowserUrlProbeCache>,
}

#[cfg(target_os = "macos")]
fn browser_url_probe_for_active_bundle(
    bundle_id: Option<&str>,
    window_title: Option<&str>,
    pid: Option<i32>,
    plan: MetadataCollectionPlan,
    cache: &BrowserUrlProbeCache,
    read_mode: BrowserUrlReadMode,
    now: Instant,
) -> (Option<String>, Option<BrowserUrlProbeCache>) {
    let Some(bundle_id) = bundle_id.filter(|bundle_id| is_known_browser_bundle(bundle_id)) else {
        return (None, None);
    };
    // On synchronous, lock-holding paths (`BrowserUrlReadMode::Cached`) a live
    // active-tab read could stall capture stop/refresh/resume for as long as the
    // read takes (Gecko AX reads cost up to ~1.4s). Serve the cached URL and let
    // the next off-lock metadata tick refresh it. The cached value is sufficient:
    // the privacy decision is app-bundle-only and ignores the URL.
    if read_mode == BrowserUrlReadMode::Cached {
        if !plan.collect_browser_url_for_metadata && !plan.collect_browser_url_for_privacy {
            return (None, None);
        }
        // Don't refresh the probe cache from a cached hit/miss — leave the
        // existing entry untouched so the next live tick re-probes normally.
        let cached_url = cache
            .cached_url_for(bundle_id, window_title, now)
            .unwrap_or(None);
        return (cached_url, None);
    }
    if plan.collect_browser_url_for_privacy {
        let raw_url = active_browser_url(bundle_id, pid);
        return (
            raw_url.clone(),
            Some(BrowserUrlProbeCache::from_probe(
                Some(bundle_id.to_string()),
                window_title.map(str::to_string),
                raw_url,
                now,
            )),
        );
    }
    if !plan.collect_browser_url_for_metadata {
        return (None, None);
    }
    if let Some(cached_url) = cache.cached_url_for(bundle_id, window_title, now) {
        return (cached_url, None);
    }
    let raw_url = active_browser_url(bundle_id, pid);
    (
        raw_url.clone(),
        Some(BrowserUrlProbeCache::from_probe(
            Some(bundle_id.to_string()),
            window_title.map(str::to_string),
            raw_url,
            now,
        )),
    )
}

#[cfg(any(test, target_os = "macos"))]
fn collect_active_window_metadata(
    metadata: &MetadataSettings,
    _privacy: &PrivacySettings,
    browser_url_probe_cache: &BrowserUrlProbeCache,
    browser_url_read_mode: BrowserUrlReadMode,
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
        let (raw_browser_url, browser_url_probe_cache) = browser_url_probe_for_active_bundle(
            bundle_id.as_deref(),
            window_title.as_deref(),
            active_window.pid,
            plan,
            browser_url_probe_cache,
            browser_url_read_mode,
            Instant::now(),
        );
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
        let _ = browser_url_probe_cache;
        let _ = browser_url_read_mode;
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
fn active_browser_url(bundle_id: &str, pid: Option<i32>) -> Option<String> {
    match browser_url_strategy(bundle_id) {
        Some(BrowserUrlStrategy::AppleScript(_)) => active_browser_url_applescript(bundle_id),
        Some(BrowserUrlStrategy::Accessibility) => {
            // First-sighting prompt: the first time a Gecko browser is frontmost
            // while browser-URL capture is enabled and trust is missing, ask once.
            crate::native_capture::browser_url_ax::maybe_prompt_on_gecko_frontmost();
            pid.and_then(crate::native_capture::browser_url_ax::read_active_tab_url)
        }
        None => None,
    }
}

#[cfg(target_os = "macos")]
fn active_browser_url_applescript(bundle_id: &str) -> Option<String> {
    let script = browser_url_applescript(bundle_id)?;
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

// ── Windows active-window metadata (ADR 0043, issue #139) ─────────────────────
//
// Windows v1 has no live content filter and no App Privacy Exclusion (ADR 0025),
// so — unlike the macOS engine in `native_capture/privacy.rs` — this path is
// METADATA-ONLY: it writes `latest_snapshot` and computes NO `PrivacyFilterDecision`.
// It is driven by the Windows segment loop's ≤1s poll while a session is active
// (`spawn_segment_loop`), plus one initial refresh at capture start.

/// Collect the foreground app + window title and write it to `latest_snapshot`,
/// mirroring macOS whole-snapshot gating: when `metadata.enabled` is false, no
/// app/window identity is recorded (the snapshot is cleared). Metadata-only — it
/// never touches `latest_decision` / `latest_applied_decision`.
///
/// The Win32 collection (including version-info file reads) is done before
/// locking `CaptureMetadataState`, so the mutex is held only for the store.
#[cfg(target_os = "windows")]
pub fn refresh_windows_metadata_snapshot(
    state: &CaptureMetadataState,
    metadata: &MetadataSettings,
) {
    let snapshot = if metadata.enabled {
        collect_windows_active_window_snapshot(metadata)
    } else {
        None
    };
    state
        .lock()
        .expect("capture metadata state poisoned")
        .publish_snapshot(crate::native_capture::runtime::now_unix_ms(), snapshot);
}

/// Snapshot the current foreground window's app identity + title via Win32.
///
/// Returns `None` when there is no foreground window, the owning PID is 0, or the
/// executable path cannot be resolved (e.g. access-denied on an elevated/system
/// process) — an honest absence of metadata. Mnema's own process is not
/// special-cased: whatever is frontmost is recorded, matching macOS.
///
/// Per ADR 0043 the canonical executable path is stored opaquely in
/// `app_bundle_id` (no schema rename in v1); `app_name` is ALWAYS populated (the
/// version-info `FileDescription`, else the file stem) so a raw path never
/// surfaces as a UI label.
/// Decide whether the Windows metadata refresh should probe the foreground app for
/// a browser URL, and with which engine. `None` ⇒ don't probe (metadata disabled,
/// browser-URL mode Off, or the exe is not a recognized browser). Pure: no Win32,
/// so the gating is unit-tested without a running browser (ADR 0044 testing note).
#[cfg(any(test, target_os = "windows"))]
fn windows_browser_url_probe_engine(
    exe_path: &str,
    plan: MetadataCollectionPlan,
) -> Option<BrowserEngine> {
    if !plan.collect_browser_url_for_metadata {
        return None;
    }
    known_browser_engine_for_exe_stem(&app_display_name_from_exe_path(exe_path))
}

#[cfg(target_os = "windows")]
fn collect_windows_active_window_snapshot(
    metadata: &MetadataSettings,
) -> Option<FrameMetadataSnapshot> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };

    // SAFETY: standard Win32 foreground-window queries. `GetForegroundWindow`
    // returns a borrowed HWND we never free; `GetWindowThreadProcessId` writes a
    // PID into a stack local. The helpers below size every buffer from the API
    // before filling it and close every handle they open. We also surface the raw
    // `HWND` (as `isize`) and PID so the browser-URL read below can target the
    // exact foreground window.
    let (exe_path, window_title, hwnd_isize, pid) = unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }

        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return None;
        }

        let exe_path = windows_process_image_path(pid)?;
        let window_title = windows_window_title(hwnd);
        (exe_path, window_title, hwnd as isize, pid)
    };

    // `app_name` MUST always be populated (see ADR 0043): FileDescription when the
    // exe exposes one, else the pure file-stem fallback.
    let app_name = windows_file_description(&exe_path)
        .filter(|description| !description.trim().is_empty())
        .map(|description| description.trim().to_string())
        .unwrap_or_else(|| capture_metadata::app_display_name_from_exe_path(&exe_path));

    // Active-tab browser URL (ADR 0044). The UI Automation read happens HERE,
    // during collection — off every capture lock. `refresh_windows_metadata_snapshot`
    // takes the `CaptureMetadataState` mutex only afterwards, to store the snapshot;
    // the live read never runs under it. It is driven by the 1s segment-loop poll
    // and the debounced foreground-change refresh (issue #141), so a slow read
    // never stalls a lock-holding capture path.
    let plan = capture_metadata::metadata_collection_plan(metadata);
    let browser_url = windows_browser_url_probe_engine(&exe_path, plan).and_then(|engine| {
        let raw = crate::native_capture::browser_url_uia::read_active_tab_url(
            hwnd_isize, pid, engine,
        )?;
        capture_metadata::sanitize_url(&raw, metadata.browser_url_mode)
    });

    Some(FrameMetadataSnapshot {
        // Store the canonical exe path opaquely in `app_bundle_id`. Trim only — do
        // NOT lowercase (the `app:` index lowercases at query time), keeping the
        // path-safe grouping key intact.
        app_bundle_id: Some(exe_path.trim().to_string()),
        app_name: Some(app_name),
        window_title,
        browser_url,
        ..FrameMetadataSnapshot::default()
    })
}

/// Resolve a process's canonical executable path via
/// `QueryFullProcessImageNameW` (`PROCESS_NAME_WIN32`). `None` on any failure,
/// including access-denied (`OpenProcess` returns null for elevated/system
/// processes at our integrity level).
///
/// # Safety
/// Opens and unconditionally closes a process handle for `pid`.
#[cfg(target_os = "windows")]
unsafe fn windows_process_image_path(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if handle.is_null() {
        return None;
    }

    // MAX_PATH is only a floor — long paths exceed it — so start generous.
    let mut buffer: Vec<u16> = vec![0u16; 1024];
    let mut size: u32 = buffer.len() as u32;
    let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut size);
    CloseHandle(handle);

    if ok == 0 || size == 0 {
        return None;
    }
    let path = String::from_utf16_lossy(&buffer[..size as usize]);
    let path = path.trim();
    (!path.is_empty()).then(|| path.to_string())
}

/// Read the foreground window's title via `GetWindowTextW`, sizing the buffer
/// from `GetWindowTextLengthW`. `None` when the window has no (non-empty) title.
///
/// # Safety
/// `hwnd` must be a valid window handle (as returned by `GetForegroundWindow`).
#[cfg(target_os = "windows")]
unsafe fn windows_window_title(hwnd: windows_sys::Win32::Foundation::HWND) -> Option<String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};

    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return None;
    }
    // +1 for the NUL terminator `GetWindowTextW` writes.
    let mut buffer: Vec<u16> = vec![0u16; len as usize + 1];
    let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
    if copied <= 0 {
        return None;
    }
    let title = String::from_utf16_lossy(&buffer[..copied as usize]);
    let title = title.trim();
    (!title.is_empty()).then(|| title.to_string())
}

/// A NUL-terminated wide-string for the Win32 `*W` version APIs.
#[cfg(target_os = "windows")]
fn windows_wide_nul(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Read the executable's version-info `FileDescription` string (the human-facing
/// display name, e.g. `Google Chrome` for `chrome.exe`). `None` when the file has
/// no version resource or no `FileDescription`; the caller then falls back to the
/// file stem so `app_name` is always populated.
///
/// Reads the first `\VarFileInfo\Translation` lang/codepage, then queries
/// `\StringFileInfo\<lang><codepage>\FileDescription`.
#[cfg(target_os = "windows")]
fn windows_file_description(exe_path: &str) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };

    let wide_path = windows_wide_nul(exe_path);

    // SAFETY: `data` is sized from `GetFileVersionInfoSizeW` and populated by
    // `GetFileVersionInfoW` before any `VerQueryValueW` read. Each `VerQueryValueW`
    // out-pointer/length pair is null- and length-checked before the pointed-to
    // bytes are read, and every read stays within `data`'s lifetime.
    unsafe {
        let mut handle: u32 = 0;
        let size = GetFileVersionInfoSizeW(wide_path.as_ptr(), &mut handle);
        if size == 0 {
            return None;
        }
        let mut data: Vec<u8> = vec![0u8; size as usize];
        if GetFileVersionInfoW(wide_path.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return None;
        }

        // First translation entry: two WORDs (language, codepage).
        let translation_key = windows_wide_nul("\\VarFileInfo\\Translation");
        let mut translation_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut translation_len: u32 = 0;
        if VerQueryValueW(
            data.as_ptr().cast(),
            translation_key.as_ptr(),
            &mut translation_ptr,
            &mut translation_len,
        ) == 0
            || translation_ptr.is_null()
            || translation_len < 4
        {
            return None;
        }
        let language = *translation_ptr.cast::<u16>();
        let codepage = *translation_ptr.cast::<u16>().add(1);

        let sub_block = format!("\\StringFileInfo\\{language:04x}{codepage:04x}\\FileDescription");
        let sub_block_key = windows_wide_nul(&sub_block);
        let mut value_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut value_len: u32 = 0;
        if VerQueryValueW(
            data.as_ptr().cast(),
            sub_block_key.as_ptr(),
            &mut value_ptr,
            &mut value_len,
        ) == 0
            || value_ptr.is_null()
            || value_len == 0
        {
            return None;
        }
        // `value_len` is a character count (may include the trailing NUL); trim it.
        let chars = std::slice::from_raw_parts(value_ptr.cast::<u16>(), value_len as usize);
        let description = String::from_utf16_lossy(chars);
        let description = description.trim_end_matches('\0').trim();
        (!description.is_empty()).then(|| description.to_string())
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
        let decision =
            refresh_metadata_state(&state, &metadata, &privacy, BrowserUrlReadMode::Live);
        let runtime = state.lock().expect("capture metadata state should lock");

        assert_eq!(decision.excluded_bundle_ids, vec!["com.secret"]);
        assert!(decision.metadata_redaction_reason.is_none());
        assert_eq!(runtime.latest_decision, decision);
        assert!(runtime.latest_snapshot().is_none());
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

        let decision =
            refresh_metadata_state(&state, &metadata, &privacy, BrowserUrlReadMode::Live);
        let runtime = state.lock().expect("capture metadata state should lock");

        assert_eq!(decision.excluded_bundle_ids, vec!["com.example.Secret"]);
        assert!(decision.metadata_redaction_reason.is_none());
        assert!(runtime.latest_snapshot().is_none());
        assert_eq!(runtime.latest_decision, decision);
    }

    fn snapshot_named(app: &str) -> FrameMetadataSnapshot {
        FrameMetadataSnapshot {
            app_name: Some(app.to_string()),
            ..FrameMetadataSnapshot::default()
        }
    }

    #[test]
    fn snapshot_in_effect_at_stamps_the_app_frontmost_at_capture_time() {
        let mut runtime = CaptureMetadataRuntime::default();
        runtime.publish_snapshot(1000, Some(snapshot_named("Hitch")));
        runtime.publish_snapshot(2000, Some(snapshot_named("mnema")));

        // Frame captured before the switch (t=1900) must keep Hitch, even though
        // "mnema" is now the latest snapshot.
        assert_eq!(
            runtime.snapshot_in_effect_at(1900).unwrap().app_name.unwrap(),
            "Hitch"
        );
        // Frame captured after the switch gets mnema.
        assert_eq!(
            runtime.snapshot_in_effect_at(2100).unwrap().app_name.unwrap(),
            "mnema"
        );
        // Frame captured before any snapshot was published gets None (session start).
        assert!(runtime.snapshot_in_effect_at(500).is_none());
    }

    #[test]
    fn snapshot_history_is_bounded_and_reset_clears_it() {
        let state = CaptureMetadataState::default();
        {
            let mut runtime = state.lock().expect("lock");
            for t in 0..(SNAPSHOT_HISTORY_CAP as u64 + 5) {
                runtime.publish_snapshot(t, Some(snapshot_named("app")));
            }
            assert_eq!(runtime.snapshot_history.len(), SNAPSHOT_HISTORY_CAP);
        }

        reset_recording_session_privacy_state(&state);

        let runtime = state.lock().expect("lock");
        assert!(runtime.snapshot_history.is_empty());
        assert!(runtime.snapshot_in_effect_at(u64::MAX).is_none());
    }

    #[test]
    fn reset_recording_session_privacy_state_clears_verified_windows_and_website_holds() {
        let state = CaptureMetadataState::default();
        {
            let mut runtime = state.lock().expect("capture metadata state should lock");
            runtime.publish_snapshot(
                1000,
                Some(FrameMetadataSnapshot {
                    app_bundle_id: Some("net.imput.helium".to_string()),
                    app_name: Some("Helium".to_string()),
                    window_title: Some("Private window".to_string()),
                    browser_url: Some("https://secret.example".to_string()),
                    ..FrameMetadataSnapshot::default()
                }),
            );
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
        assert!(runtime.latest_snapshot().is_none());
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
                Some("Old Tab — Example".to_string()),
                Some("https://example.com/old-tab".to_string()),
                Instant::now(),
            );
        }

        reset_recording_session_privacy_state(&state);

        let runtime = state.lock().expect("capture metadata state should lock");
        assert_eq!(
            runtime.browser_url_probe_cache.cached_url_for(
                "com.google.Chrome",
                Some("Old Tab — Example"),
                Instant::now()
            ),
            None
        );
    }

    // ── Windows browser-URL probe gating (ADR 0044) ──────────────────────────
    // These exercise the pure decision fn `windows_browser_url_probe_engine`
    // through the real `metadata_collection_plan`, so they run host-agnostically
    // (no Win32, no running browser) on macOS/Windows/Linux under `cfg(test)`.

    const CHROME_EXE: &str = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
    const ZEN_EXE: &str = r"C:\Users\me\AppData\Local\zen\zen.exe";
    const SLACK_EXE: &str = r"C:\Users\me\AppData\Local\slack\app-1.0\slack.exe";

    #[test]
    fn windows_browser_url_probe_none_when_metadata_disabled() {
        let plan = capture_metadata::metadata_collection_plan(&MetadataSettings {
            enabled: false,
            browser_url_mode: BrowserUrlMode::Full,
        });
        assert_eq!(windows_browser_url_probe_engine(CHROME_EXE, plan), None);
    }

    #[test]
    fn windows_browser_url_probe_none_when_mode_off() {
        let plan = capture_metadata::metadata_collection_plan(&MetadataSettings {
            enabled: true,
            browser_url_mode: BrowserUrlMode::Off,
        });
        assert_eq!(windows_browser_url_probe_engine(CHROME_EXE, plan), None);
    }

    #[test]
    fn windows_browser_url_probe_resolves_engine_when_enabled() {
        for mode in [BrowserUrlMode::Sanitized, BrowserUrlMode::Full] {
            let plan = capture_metadata::metadata_collection_plan(&MetadataSettings {
                enabled: true,
                browser_url_mode: mode,
            });
            assert_eq!(
                windows_browser_url_probe_engine(CHROME_EXE, plan),
                Some(BrowserEngine::Chromium),
                "chrome.exe should resolve to Chromium (mode {mode:?})"
            );
            assert_eq!(
                windows_browser_url_probe_engine(ZEN_EXE, plan),
                Some(BrowserEngine::Gecko),
                "zen.exe should resolve to Gecko (mode {mode:?})"
            );
        }
    }

    #[test]
    fn windows_browser_url_probe_none_for_unrecognized_exe() {
        let plan = capture_metadata::metadata_collection_plan(&MetadataSettings {
            enabled: true,
            browser_url_mode: BrowserUrlMode::Full,
        });
        assert_eq!(windows_browser_url_probe_engine(SLACK_EXE, plan), None);
    }

    // The strategy dispatch in `active_browser_url` routes a Chromium bundle to
    // the AppleScript path and a Gecko bundle to the Accessibility path. We do
    // not run osascript or a live Accessibility read here (those need a running
    // browser + permission — that is a manual verify step); we assert the
    // routing inputs the dispatch keys off. `browser_url_applescript` backs the
    // AppleScript branch, `browser_url_strategy` selects the branch.
    #[cfg(target_os = "macos")]
    #[test]
    fn active_browser_url_routes_chromium_to_applescript_and_gecko_away() {
        // Chromium routes to AppleScript, which has a script to run.
        assert!(matches!(
            browser_url_strategy("com.google.Chrome"),
            Some(BrowserUrlStrategy::AppleScript(_))
        ));
        assert!(browser_url_applescript("com.google.Chrome")
            .is_some_and(|script| script.contains("active tab of front window")));
        // Gecko routes to Accessibility and has no AppleScript surface at all, so
        // dispatch never reaches the osascript path for it.
        assert_eq!(
            browser_url_strategy("org.mozilla.firefox"),
            Some(BrowserUrlStrategy::Accessibility),
        );
        assert_eq!(browser_url_applescript("org.mozilla.firefox"), None);
    }
}
