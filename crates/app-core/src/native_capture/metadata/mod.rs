use capture_metadata::MetadataSettings;
#[cfg(target_os = "macos")]
use capture_metadata::{browser_url_applescript, browser_url_strategy, BrowserUrlStrategy};
use capture_metadata::{
    evaluate_privacy, is_known_browser_bundle, metadata_collection_plan, sanitize_url,
    select_frontmost_pid_window, BrowserUrlProbeCache, FrameMetadataSnapshot,
    MetadataCollectionPlan, MetadataContext, NativeActiveWindowSnapshot, PrivacyFilterDecision,
    PrivacySettings, RawWindowInfo,
};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

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

/// The host's NSWorkspace observer registrations that request a privacy-filter
/// refresh on focus/app changes; held so dropping them deregisters.
#[cfg(target_os = "macos")]
#[derive(Default)]
pub struct MetadataNotifierState(std::sync::Mutex<Vec<cidre::ns::NotificationGuard>>);

#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct MetadataNotifierState(std::sync::Mutex<Vec<()>>);

#[cfg(target_os = "macos")]
impl MetadataNotifierState {
    pub fn replace(&self, guards: Vec<cidre::ns::NotificationGuard>) {
        *self.0.lock().expect("metadata notifier state poisoned") = guards;
    }
}

#[cfg(not(target_os = "macos"))]
impl MetadataNotifierState {
    pub fn replace(&self, guards: Vec<()>) {
        *self.0.lock().expect("metadata notifier state poisoned") = guards;
    }
}

/// Whether a metadata refresh may issue a *live* active-tab browser-URL read.
///
/// The Gecko (Firefox/Zen) URL is read via the Accessibility API
/// ([`super::browser_url_ax::read_active_tab_url`]), which is
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

pub type FrameMetadataSnapshotProvider =
    Arc<dyn Fn(u64) -> Option<FrameMetadataSnapshot> + Send + Sync + 'static>;

pub fn frame_metadata_snapshot_provider(core: &crate::AppCore) -> FrameMetadataSnapshotProvider {
    let state = core.state::<CaptureMetadataState>();
    Arc::new(move |captured_at_unix_ms| {
        frame_metadata_snapshot_in_effect_at(&state, captured_at_unix_ms)
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
    runtime.snapshot_history.clear();
    runtime.latest_decision = PrivacyFilterDecision::default();
    runtime.latest_applied_decision = PrivacyFilterDecision::default();
    runtime.browser_url_probe_cache = BrowserUrlProbeCache::default();
}

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
    runtime.publish_snapshot(super::runtime::now_unix_ms(), snapshot);
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
        let _ = browser_url_read_mode;
        ActiveWindowMetadata {
            snapshot: None,
            context: MetadataContext::default(),
            browser_url_probe_cache: None,
        }
    }
}

#[cfg(target_os = "macos")]
pub fn collect_native_active_window_snapshot() -> NativeActiveWindowSnapshot {
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

/// Whether the lock screen (`loginwindow`) holds the foreground.
#[cfg(target_os = "macos")]
pub fn frontmost_app_is_lock_screen() -> bool {
    frontmost_running_app_snapshot().is_some_and(|(_, app_bundle_id, app_name)| {
        FrameMetadataSnapshot {
            app_bundle_id,
            app_name,
            ..FrameMetadataSnapshot::default()
        }
        .is_lock_screen()
    })
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
            super::browser_url_ax::maybe_prompt_on_gecko_frontmost();
            pid.and_then(super::browser_url_ax::read_active_tab_url)
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

#[cfg(test)]
mod tests;
