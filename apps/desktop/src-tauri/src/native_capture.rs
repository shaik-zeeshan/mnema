mod activity;
#[cfg(target_os = "macos")]
#[path = "native_capture_browser_url_ax.rs"]
pub(crate) mod browser_url_ax;
#[path = "native_capture_debug_log.rs"]
pub(crate) mod debug_log;
pub(crate) mod disk_space;
#[path = "native_capture_inactivity.rs"]
pub(crate) mod inactivity;
mod lifecycle;
#[path = "native_capture_metadata.rs"]
pub(crate) mod metadata;
mod microphone;
#[path = "native_capture_output.rs"]
pub(crate) mod output;
pub(crate) mod privacy;
mod runtime;
mod segments;
#[path = "native_capture_settings.rs"]
pub(crate) mod settings;
#[path = "native_capture_system_idle.rs"]
pub(crate) mod system_idle;
#[cfg(test)]
mod tests;

use capture_microphone as microphone_capture;
use capture_types::{
    AudioTranscriptionProvider, AudioTranscriptionSettings, CaptureErrorResponse,
    CaptureOutputFiles, CapturePermissionState, CapturePermissions, CapturePermissionsResponse,
    CaptureSources, CaptureSupportResponse, InactivityActivityMode, MicrophoneControllerState,
    NativeCaptureDebugLogStatus, NativeCaptureSessionResponse, OcrProvider, OcrSettings,
    RecordingSettings, RecordingSettingsDomainUpdateResponse, ScreenResolution,
    ScreenResolutionPreset, SettingsOwnershipDomain, StartNativeCaptureRequest,
    UpdateAccessSettingsRequest, UpdateAiRuntimeSettingsRequest,
    UpdateCaptureSourceSettingsRequest, UpdateCaptureTimingSettingsRequest,
    UpdateDeveloperSettingsRequest,
    UpdateDisplaySettingsRequest, UpdateInactivitySettingsRequest, UpdateMetadataSettingsRequest,
    UpdateMicrophoneControllerRequest, UpdateProcessingSettingsRequest,
    UpdateRecordingSettingsRequest, UpdateStorageSettingsRequest,
    UpdateUserContextSettingsRequest, UpdateVideoSettingsRequest,
    VideoBitrateMode, VideoBitratePreset, VideoBitrateSettings,
};
use capture_vad::configured_adapter_as_str;
use settings::{
    apply_recording_settings_domain_mutation, apply_recording_settings_domain_patch,
    apply_recording_settings_update, current_auto_start,
    current_native_capture_debug_logging_enabled, current_recording_settings,
    initialize_recording_settings_state_from_disk, RecordingSettingsDomainPatch,
};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(target_os = "macos")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
#[cfg(target_os = "macos")]
use std::time::Duration;
use tauri::{path::BaseDirectory, Emitter, Manager};

pub use capture_types::IdleDebugInfo;
pub(crate) use debug_log::install_panic_hook;
use lifecycle::{RecordingLifecycle, StartRecordingLifecycleOutcome};
use microphone::{
    resolve_capture_microphone_device_id, should_wait_for_same_microphone_device,
    update_microphone_controller as update_microphone_controller_impl,
};
pub use microphone::{
    start_microphone_device_change_notifier, MicrophoneControllerPreferencesState,
    MicrophoneDeviceChangeNotifierState,
};
use runtime::validate_start_request;
pub type NativeCaptureState = Mutex<RecordingLifecycle>;
pub use privacy::PrivacyFilterRefreshState;
pub use settings::RecordingSettingsState;
// Re-exported so adapter-level Tauri commands (e.g. `open_debug_window`) can
// read the persisted recording settings through the same seam used by the
// rest of `native_capture` without bypassing it to touch persistence directly.
pub(crate) use settings::current_recording_settings as read_recording_settings;

#[cfg(target_os = "macos")]
#[derive(Default)]
pub struct SystemWakeNotifierState(std::sync::Mutex<Vec<cidre::ns::NotificationGuard>>);

#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct SystemWakeNotifierState(std::sync::Mutex<Vec<()>>);

// Holds the registration for the Core Graphics display-reconfiguration callback
// used as the primary, polling-free wake-recovery signal (dark/deep-idle wakes
// and monitor reconnects power the panel back on without posting
// `NSWorkspaceDidWake`). The guard removes the callback on drop for symmetry.
#[cfg(target_os = "macos")]
#[derive(Default)]
pub struct DisplayReconfigurationNotifierState(
    // The guard is held only so its `Drop` deregisters the callback on teardown.
    #[allow(dead_code)] std::sync::Mutex<Option<DisplayReconfigurationCallbackGuard>>,
);

#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct DisplayReconfigurationNotifierState(std::sync::Mutex<Option<()>>);

#[cfg(target_os = "macos")]
#[derive(Default)]
pub struct MetadataNotifierState(std::sync::Mutex<Vec<cidre::ns::NotificationGuard>>);

#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct MetadataNotifierState(std::sync::Mutex<Vec<()>>);

#[cfg(target_os = "macos")]
impl MetadataNotifierState {
    pub(crate) fn replace(&self, guards: Vec<cidre::ns::NotificationGuard>) {
        *self.0.lock().expect("metadata notifier state poisoned") = guards;
    }
}

#[cfg(not(target_os = "macos"))]
impl MetadataNotifierState {
    pub(crate) fn replace(&self, guards: Vec<()>) {
        *self.0.lock().expect("metadata notifier state poisoned") = guards;
    }
}

pub const SYSTEM_DID_WAKE_EVENT: &str = "system_did_wake";
#[cfg(target_os = "macos")]
// ScreenCaptureKit can report no displays for several seconds after macOS
// wake; keep recovery alive long enough that the backend does not depend on a
// later frontend permissions poll to restart capture.
const SYSTEM_WAKE_RECOVERY_RETRY_DELAYS_MS: &[u64] = &[
    500, 1_500, 3_000, 5_000, 10_000, 10_000, 10_000, 10_000, 10_000,
];
pub const AUDIO_SEGMENTS_CHANGED_EVENT: &str = "audio_segments_changed";
pub const RECORDING_SETTINGS_CHANGED_EVENT: &str = "recording_settings_changed";
pub const RECORDING_SETTINGS_DOMAIN_CHANGED_EVENT: &str = "recording_settings_domain_changed";
pub const NATIVE_CAPTURE_SESSION_CHANGED_EVENT: &str = "native_capture_session_changed";
pub const APP_NOTIFICATIONS_CHANGED_EVENT: &str = "app_notifications_changed";
const AUDIO_TRANSCRIPTION_UNAVAILABLE_NOTIFICATION_ID: &str = "audio-transcription-unavailable";
const OCR_UNAVAILABLE_NOTIFICATION_ID: &str = "ocr-unavailable";
const SPEECH_DETECTOR_UNAVAILABLE_NOTIFICATION_ID: &str = "speech-detector-unavailable";
const SPEAKER_ANALYSIS_UNAVAILABLE_NOTIFICATION_ID: &str = "speaker-analysis-unavailable";
const PRIVACY_RECOVERY_RESTART_REQUIRED_NOTIFICATION_ID: &str = "privacy-recovery-restart-required";
const PROCESSING_SETTINGS_TAB_ID: &str = "processing";
const TRANSCRIPTION_SETTINGS_TAB_ID: &str = "transcription";
const SPEAKER_SETTINGS_TAB_ID: &str = "speakers";
#[cfg(target_os = "macos")]
const APP_ICON_CACHE_DIR: &str = "app-icons";
// Point size we render cached app icons at. Displayed at 20–24 CSS px, so a
// larger source keeps the icon crisp on Retina (2x → ~48 device px) by always
// downscaling instead of upscaling a small bitmap. Baked into the cache
// filename so bumping this size supersedes previously cached lower-res PNGs.
#[cfg(target_os = "macos")]
const APP_ICON_RENDER_POINT_SIZE: u32 = 128;

#[derive(Debug, Clone, serde::Serialize)]
#[allow(dead_code)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppNotificationAction {
    OpenSettingsTab { tab: String },
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppNotification {
    pub id: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    pub created_at_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<AppNotificationAction>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyAppCandidate {
    pub bundle_id: String,
    pub display_name: String,
    pub running: bool,
    pub icon_path: Option<String>,
    #[serde(skip)]
    pub bundle_path: Option<PathBuf>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveAppIconsRequest {
    pub bundle_ids: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppIconResolution {
    pub bundle_id: String,
    pub icon_path: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckBrowserUrlSupportRequest {
    pub bundle_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserUrlSupportResponse {
    pub bundle_id: String,
    pub supported: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePrivacyDebugResponse {
    pub metadata_enabled: bool,
    pub browser_url_mode: capture_metadata::BrowserUrlMode,
    pub browser_url_metadata_source: BrowserUrlMetadataDebugSource,
    pub privacy_debug: metadata::CapturePrivacyDebugInfo,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserUrlMetadataDebugSource {
    NativeBrowserUrlProbe,
    Unavailable,
}

#[derive(Debug, Default)]
pub struct AppNotificationsRuntime {
    notifications: Vec<AppNotification>,
}

impl AppNotificationsRuntime {
    fn list(&self) -> Vec<AppNotification> {
        self.notifications.clone()
    }

    fn push_session_notification(&mut self, notification: AppNotification) -> Vec<AppNotification> {
        self.notifications.retain(|item| item.id != notification.id);
        self.notifications.push(notification);
        self.list()
    }

    fn clear_one(&mut self, id: &str) -> Vec<AppNotification> {
        self.notifications.retain(|item| item.id != id);
        self.list()
    }

    fn clear_all(&mut self) -> Vec<AppNotification> {
        self.notifications.clear();
        self.list()
    }
}

pub type AppNotificationsState = Mutex<AppNotificationsRuntime>;
pub use metadata::{start_metadata_notifier, CaptureMetadataState};

#[tauri::command]
pub async fn list_privacy_app_candidates() -> Result<Vec<PrivacyAppCandidate>, String> {
    let mut candidates = BTreeMap::new();
    insert_privacy_app_candidate(
        &mut candidates,
        PrivacyAppCandidate {
            bundle_id: "com.shaikzeeshan.mnema".to_string(),
            display_name: "Mnema".to_string(),
            running: true,
            icon_path: None,
            #[cfg(target_os = "macos")]
            bundle_path: main_bundle_path(),
            #[cfg(not(target_os = "macos"))]
            bundle_path: None,
        },
    );

    #[cfg(target_os = "macos")]
    {
        let running_candidates = running_privacy_app_candidates();
        add_installed_privacy_app_candidates(&mut candidates);
        merge_running_privacy_app_candidates(&mut candidates, running_candidates);
    }

    let mut candidates: Vec<_> = candidates.into_values().collect();
    candidates.sort_by(|left, right| {
        left.display_name
            .to_ascii_lowercase()
            .cmp(&right.display_name.to_ascii_lowercase())
            .then_with(|| left.bundle_id.cmp(&right.bundle_id))
    });
    Ok(candidates)
}

#[tauri::command]
pub async fn resolve_app_icons(
    app_handle: tauri::AppHandle,
    request: ResolveAppIconsRequest,
) -> Result<Vec<AppIconResolution>, String> {
    let requested_bundle_ids: BTreeSet<String> = request
        .bundle_ids
        .into_iter()
        .map(|bundle_id| bundle_id.trim().to_string())
        .filter(|bundle_id| !bundle_id.is_empty())
        .collect();

    if requested_bundle_ids.is_empty() {
        return Ok(Vec::new());
    }

    #[cfg(target_os = "macos")]
    {
        // Each requested identifier resolves to an app bundle path. They are
        // usually bundle ids, but some callers (e.g. the Ask-AI timeline chip)
        // pass a human display name like "Zen Browser"; when the bundle-id
        // lookup misses, fall back to a case-insensitive match against the
        // installed/running app catalog so those chips still get a real icon.
        // Non-app labels (e.g. a website name) stay unresolved, and the frontend
        // renders its letter fallback.
        let mut bundle_paths: BTreeMap<String, Option<PathBuf>> = requested_bundle_ids
            .iter()
            .map(|identifier| (identifier.clone(), app_icon_bundle_path(identifier)))
            .collect();
        if bundle_paths.values().any(Option::is_none) {
            let catalog = app_display_name_bundle_path_catalog();
            for (identifier, path) in bundle_paths.iter_mut() {
                if path.is_none() {
                    *path = catalog.get(&canonical_app_display_name(identifier)).cloned();
                }
            }
        }

        let mut candidates = BTreeMap::new();
        for identifier in &requested_bundle_ids {
            insert_privacy_app_candidate(
                &mut candidates,
                PrivacyAppCandidate {
                    bundle_id: identifier.clone(),
                    display_name: identifier.clone(),
                    running: false,
                    icon_path: None,
                    bundle_path: bundle_paths.get(identifier).cloned().flatten(),
                },
            );
        }
        materialize_app_candidate_icons(&app_handle, &mut candidates);

        let icons = requested_bundle_ids
            .into_iter()
            .map(|bundle_id| AppIconResolution {
                icon_path: candidates
                    .get(&bundle_id)
                    .and_then(|candidate| candidate.icon_path.clone()),
                bundle_id,
            })
            .collect();
        Ok(icons)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = app_handle;
        Ok(requested_bundle_ids
            .into_iter()
            .map(|bundle_id| AppIconResolution {
                bundle_id,
                icon_path: None,
            })
            .collect())
    }
}

fn insert_privacy_app_candidate(
    candidates: &mut BTreeMap<String, PrivacyAppCandidate>,
    candidate: PrivacyAppCandidate,
) {
    let bundle_id = candidate.bundle_id.trim();
    if bundle_id.is_empty() {
        return;
    }

    let display_name = candidate.display_name.trim();
    let candidate = PrivacyAppCandidate {
        bundle_id: bundle_id.to_string(),
        display_name: if display_name.is_empty() {
            bundle_id.to_string()
        } else {
            display_name.to_string()
        },
        running: candidate.running,
        icon_path: candidate.icon_path,
        bundle_path: candidate.bundle_path,
    };

    if let Some(existing) = candidates.get_mut(&candidate.bundle_id) {
        existing.running |= candidate.running;
        if candidate.running || existing.display_name == existing.bundle_id {
            existing.display_name = candidate.display_name;
        }
        if existing.icon_path.is_none() {
            existing.icon_path = candidate.icon_path;
        }
        if existing.bundle_path.is_none() {
            existing.bundle_path = candidate.bundle_path;
        }
        return;
    }

    candidates.insert(candidate.bundle_id.clone(), candidate);
}

#[cfg(target_os = "macos")]
fn running_privacy_app_candidates() -> Vec<PrivacyAppCandidate> {
    let mut candidates = Vec::new();
    let running_apps = cidre::ns::Workspace::shared().running_apps();
    for app in running_apps.iter() {
        let Some(bundle_id) = app.bundle_id().map(|value| value.to_string()) else {
            continue;
        };
        let Some(bundle_path) = app
            .bundle_url()
            .and_then(|url| url.path())
            .map(|path| PathBuf::from(path.to_string()))
            .filter(|path| path_has_app_extension(path))
        else {
            continue;
        };
        let display_name = app
            .localized_name()
            .map(|name| name.to_string())
            .filter(|name| !name.trim().is_empty())
            .or_else(|| {
                bundle_path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .filter(|name| !name.trim().is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| bundle_id.clone());
        candidates.push(PrivacyAppCandidate {
            bundle_id,
            display_name,
            running: true,
            icon_path: None,
            bundle_path: Some(bundle_path),
        });
    }
    candidates
}

fn mark_running_privacy_app_candidates(
    candidates: &mut BTreeMap<String, PrivacyAppCandidate>,
    running_bundle_ids: &BTreeSet<String>,
) {
    for bundle_id in running_bundle_ids {
        if let Some(candidate) = candidates.get_mut(bundle_id) {
            candidate.running = true;
        }
    }
}

fn merge_running_privacy_app_candidates(
    candidates: &mut BTreeMap<String, PrivacyAppCandidate>,
    running_candidates: impl IntoIterator<Item = PrivacyAppCandidate>,
) {
    let running_bundle_ids = running_candidates
        .into_iter()
        .map(|candidate| {
            let bundle_id = candidate.bundle_id.clone();
            insert_privacy_app_candidate(candidates, candidate);
            bundle_id
        })
        .collect::<BTreeSet<_>>();
    mark_running_privacy_app_candidates(candidates, &running_bundle_ids);
}

#[cfg(target_os = "macos")]
fn add_installed_privacy_app_candidates(candidates: &mut BTreeMap<String, PrivacyAppCandidate>) {
    for root in privacy_app_search_roots() {
        collect_privacy_apps_from_dir(&root, 0, candidates);
    }
}

#[cfg(target_os = "macos")]
fn privacy_app_search_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = std::env::home_dir() {
        roots.push(home.join("Applications"));
    }
    roots
}

#[cfg(target_os = "macos")]
fn collect_privacy_apps_from_dir(
    dir: &Path,
    depth: usize,
    candidates: &mut BTreeMap<String, PrivacyAppCandidate>,
) {
    const MAX_DEPTH: usize = 4;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let path = entry.path();
        if path_has_app_extension(&path) {
            if let Some(candidate) = privacy_app_candidate_from_bundle_path(&path) {
                insert_privacy_app_candidate(candidates, candidate);
            }
            continue;
        }

        if depth < MAX_DEPTH {
            collect_privacy_apps_from_dir(&path, depth + 1, candidates);
        }
    }
}

#[cfg(target_os = "macos")]
fn privacy_app_candidate_from_bundle_path(path: &Path) -> Option<PrivacyAppCandidate> {
    let path = path.to_str()?;
    let ns_path = cidre::ns::String::with_str(path);
    let bundle = cidre::ns::Bundle::with_path(&ns_path)?;
    let bundle_id = bundle.bundle_id()?.to_string();
    let display_name = Path::new(path)
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&bundle_id)
        .to_string();

    Some(PrivacyAppCandidate {
        bundle_id,
        display_name,
        running: false,
        icon_path: None,
        bundle_path: Some(PathBuf::from(path)),
    })
}

#[cfg(target_os = "macos")]
fn path_has_app_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
}

#[cfg(target_os = "macos")]
fn main_bundle_path() -> Option<PathBuf> {
    let bundle_path = cidre::ns::Bundle::main().bundle_path().to_string();
    (!bundle_path.trim().is_empty()).then(|| PathBuf::from(bundle_path))
}

#[cfg(target_os = "macos")]
fn app_icon_bundle_path(bundle_id: &str) -> Option<PathBuf> {
    macos_application_bundle_path_for_bundle_id(bundle_id).or_else(|| {
        (bundle_id == "com.shaikzeeshan.mnema")
            .then(main_bundle_path)
            .flatten()
    })
}

#[cfg(target_os = "macos")]
fn canonical_app_display_name(value: &str) -> String {
    value.trim().to_lowercase()
}

/// Index installed/running apps by canonical display name → bundle path, so an
/// icon request that carries a human name (e.g. "Zen Browser") rather than a
/// bundle id can still resolve to a real app icon. Built lazily and only when a
/// bundle-id lookup has already missed, since it scans the application folders.
#[cfg(target_os = "macos")]
fn app_display_name_bundle_path_catalog() -> std::collections::HashMap<String, PathBuf> {
    let mut candidates = BTreeMap::new();
    add_installed_privacy_app_candidates(&mut candidates);
    merge_running_privacy_app_candidates(&mut candidates, running_privacy_app_candidates());

    let mut by_name = std::collections::HashMap::new();
    for candidate in candidates.into_values() {
        let Some(bundle_path) = candidate.bundle_path else {
            continue;
        };
        let key = canonical_app_display_name(&candidate.display_name);
        if key.is_empty() {
            continue;
        }
        // First match wins; a display-name collision across two installed apps is
        // rare and any matching real icon is acceptable for a decorative chip.
        by_name.entry(key).or_insert(bundle_path);
    }
    by_name
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn macos_application_bundle_path_for_bundle_id(bundle_id: &str) -> Option<PathBuf> {
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::CStr;
    use std::os::raw::c_char;

    if bundle_id.trim().is_empty() {
        return None;
    }

    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    cidre::ns::try_catch(|| unsafe {
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        if workspace == nil {
            return None;
        }

        let ns_bundle_id = NSString::alloc(nil).init_str(bundle_id);
        let app_url: id = msg_send![workspace, URLForApplicationWithBundleIdentifier: ns_bundle_id];
        if app_url == nil {
            return None;
        }

        let path: id = msg_send![app_url, path];
        if path == nil {
            return None;
        }

        let raw_path: *const c_char = msg_send![path, UTF8String];
        if raw_path.is_null() {
            return None;
        }

        let path = CStr::from_ptr(raw_path).to_string_lossy().into_owned();
        (!path.trim().is_empty()).then(|| PathBuf::from(path))
    })
    .ok()
    .flatten()
}

#[cfg(target_os = "macos")]
fn ensure_app_icon_cache_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let cache_dir = app_handle
        .path()
        .resolve(APP_ICON_CACHE_DIR, BaseDirectory::AppCache)
        .map_err(|error| format!("failed to resolve app icon cache directory: {error}"))?;
    std::fs::create_dir_all(&cache_dir).map_err(|error| {
        format!(
            "failed to create app icon cache directory {}: {error}",
            cache_dir.display()
        )
    })?;
    app_handle
        .asset_protocol_scope()
        .allow_directory(&cache_dir, true)
        .map_err(|error| {
            format!(
                "failed to allow app icon cache directory {} in asset scope: {error}",
                cache_dir.display()
            )
        })?;
    Ok(cache_dir)
}

#[cfg(target_os = "macos")]
fn materialize_app_candidate_icons(
    app_handle: &tauri::AppHandle,
    candidates: &mut BTreeMap<String, PrivacyAppCandidate>,
) {
    let Ok(cache_dir) = ensure_app_icon_cache_dir(app_handle) else {
        return;
    };

    for candidate in candidates.values_mut() {
        let Some(bundle_path) = candidate.bundle_path.as_deref() else {
            continue;
        };
        let Some(icon_path) = app_icon_cache_path(&cache_dir, &candidate.bundle_id) else {
            continue;
        };
        let cached_icon_available = icon_path
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0);
        if cached_icon_available || write_macos_app_icon_png(bundle_path, &icon_path).is_ok() {
            candidate.icon_path = Some(icon_path.to_string_lossy().to_string());
        }
    }
}

#[cfg(target_os = "macos")]
fn app_icon_cache_path(cache_dir: &Path, bundle_id: &str) -> Option<PathBuf> {
    let file_stem = sanitize_app_icon_file_stem(bundle_id);
    if file_stem.is_empty() {
        return None;
    }
    // Size suffix invalidates older lower-res caches when the render size bumps.
    Some(cache_dir.join(format!("{file_stem}@{APP_ICON_RENDER_POINT_SIZE}.png")))
}

#[cfg(target_os = "macos")]
fn sanitize_app_icon_file_stem(bundle_id: &str) -> String {
    bundle_id
        .trim()
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                Some(ch)
            } else if ch.is_whitespace() || matches!(ch, '/' | ':' | '\\') {
                Some('_')
            } else {
                None
            }
        })
        .collect()
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn write_macos_app_icon_png(bundle_path: &Path, output_path: &Path) -> Result<(), String> {
    use cocoa::base::{id, nil};
    use cocoa::foundation::{NSData, NSDictionary, NSSize, NSString, NSUInteger};
    use objc::{class, msg_send, sel, sel_impl};

    const NS_PNG_FILE_TYPE: NSUInteger = 4;
    let icon_point_size = f64::from(APP_ICON_RENDER_POINT_SIZE);

    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    let bundle_path = bundle_path.to_string_lossy();

    let png_bytes = match cidre::ns::try_catch(|| unsafe {
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        if workspace == nil {
            return Err("failed to access NSWorkspace while loading app icon".to_string());
        }

        let ns_bundle_path = NSString::alloc(nil).init_str(&bundle_path);
        let icon: id = msg_send![workspace, iconForFile: ns_bundle_path];
        if icon == nil {
            return Err(format!("failed to load app icon for {}", bundle_path));
        }

        let _: () = msg_send![icon, setSize: NSSize::new(icon_point_size, icon_point_size)];
        let tiff_data: id = msg_send![icon, TIFFRepresentation];
        if tiff_data == nil {
            return Err(format!(
                "failed to render app icon TIFF for {}",
                bundle_path
            ));
        }

        let bitmap_rep: id = msg_send![class!(NSBitmapImageRep), imageRepWithData: tiff_data];
        if bitmap_rep == nil {
            return Err(format!(
                "failed to decode app icon bitmap for {}",
                bundle_path
            ));
        }

        let properties = NSDictionary::dictionary(nil);
        let png_data: id = msg_send![
            bitmap_rep,
            representationUsingType: NS_PNG_FILE_TYPE
            properties: properties
        ];
        if png_data == nil {
            return Err(format!("failed to encode app icon PNG for {}", bundle_path));
        }

        let len = png_data.length();
        let bytes = png_data.bytes();
        if bytes.is_null() || len == 0 {
            return Err(format!("app icon PNG was empty for {}", bundle_path));
        }
        Ok(std::slice::from_raw_parts(bytes.cast::<u8>(), len as usize).to_vec())
    }) {
        Ok(result) => result?,
        Err(exception) => {
            let name = format!("{}", &**exception.name());
            let reason = exception
                .reason()
                .map(|reason| format!("{}", reason.as_ref()))
                .unwrap_or_else(|| "unknown reason".to_string());
            return Err(format!(
                "ObjC exception while loading app icon for {}: {} - {}",
                bundle_path, name, reason
            ));
        }
    };

    std::fs::write(output_path, png_bytes).map_err(|error| {
        format!(
            "failed to write app icon {}: {error}",
            output_path.display()
        )
    })
}

#[tauri::command]
pub async fn check_browser_url_support(
    request: CheckBrowserUrlSupportRequest,
) -> Result<BrowserUrlSupportResponse, String> {
    let supported = capture_metadata::browser_url_metadata_supported(&request.bundle_id);
    Ok(BrowserUrlSupportResponse {
        bundle_id: request.bundle_id,
        supported,
        warning: (!supported).then(|| {
            "URL metadata support is unknown for this browser. When website privacy rules are enabled, this browser may be redacted because its URL cannot be checked."
                .to_string()
        }),
    })
}

#[tauri::command]
pub fn get_capture_privacy_debug(
    _app_handle: tauri::AppHandle,
    metadata_state: tauri::State<'_, CaptureMetadataState>,
    settings_state: tauri::State<'_, RecordingSettingsState>,
) -> CapturePrivacyDebugResponse {
    let settings = current_recording_settings(settings_state.inner());
    CapturePrivacyDebugResponse {
        metadata_enabled: settings.metadata.enabled,
        browser_url_mode: settings.metadata.browser_url_mode,
        browser_url_metadata_source: browser_url_metadata_source(&settings.metadata),
        privacy_debug: metadata::capture_privacy_debug_info(metadata_state.inner()),
    }
}

fn browser_url_metadata_source(
    metadata: &capture_metadata::MetadataSettings,
) -> BrowserUrlMetadataDebugSource {
    if metadata.enabled && metadata.browser_url_mode != capture_metadata::BrowserUrlMode::Off {
        BrowserUrlMetadataDebugSource::NativeBrowserUrlProbe
    } else {
        BrowserUrlMetadataDebugSource::Unavailable
    }
}

fn emit_system_did_wake(app_handle: &tauri::AppHandle) {
    let _ = app_handle.emit(SYSTEM_DID_WAKE_EVENT, ());
}

pub(super) fn emit_audio_segments_changed(app_handle: &tauri::AppHandle) {
    let _ = app_handle.emit(AUDIO_SEGMENTS_CHANGED_EVENT, ());
}

pub(crate) fn emit_recording_settings_changed(
    app_handle: &tauri::AppHandle,
    settings: &RecordingSettings,
) {
    let _ = app_handle.emit(RECORDING_SETTINGS_CHANGED_EVENT, settings);
}

pub(crate) fn emit_recording_settings_domain_changed(
    app_handle: &tauri::AppHandle,
    response: &RecordingSettingsDomainUpdateResponse,
) {
    let _ = app_handle.emit(RECORDING_SETTINGS_DOMAIN_CHANGED_EVENT, response);
}

pub(crate) fn emit_native_capture_session_changed(
    app_handle: &tauri::AppHandle,
    session: &capture_types::NativeCaptureSession,
) {
    let _ = app_handle.emit(NATIVE_CAPTURE_SESSION_CHANGED_EVENT, session);
    crate::app_updates::on_capture_session_changed(app_handle);
}

fn emit_app_notifications_changed(
    app_handle: &tauri::AppHandle,
    notifications: &[AppNotification],
) {
    let _ = app_handle.emit(APP_NOTIFICATIONS_CHANGED_EVENT, notifications);
}

fn push_app_notification(
    app_handle: &tauri::AppHandle,
    state: &AppNotificationsState,
    notification: AppNotification,
) {
    let notifications = {
        let mut runtime = state.lock().expect("app notifications state poisoned");
        runtime.push_session_notification(notification)
    };
    emit_app_notifications_changed(app_handle, &notifications);
}

pub(super) fn push_privacy_recovery_restart_required_notification(app_handle: &tauri::AppHandle) {
    let Some(state) = app_handle.try_state::<AppNotificationsState>() else {
        debug_log::log_warn(
            "app notifications state unavailable while reporting privacy recovery restart requirement",
        );
        return;
    };
    push_app_notification(
        app_handle,
        state.inner(),
        AppNotification {
            id: PRIVACY_RECOVERY_RESTART_REQUIRED_NOTIFICATION_ID.to_string(),
            severity: "warning".to_string(),
            title: "Screen capture paused for privacy".to_string(),
            message: "Screen and system audio capture were paused after privacy filter recovery failed. Stop and start recording to resume those sources.".to_string(),
            created_at_unix_ms: runtime::now_unix_ms(),
            action: None,
        },
    );
}

pub(crate) fn push_warning_app_notification(
    app_handle: &tauri::AppHandle,
    id: &str,
    title: &str,
    message: &str,
    settings_tab: Option<&str>,
    created_at_unix_ms: u64,
) {
    let action = settings_tab.map(|tab| AppNotificationAction::OpenSettingsTab {
        tab: tab.to_string(),
    });
    push_app_notification(
        app_handle,
        app_handle.state::<AppNotificationsState>().inner(),
        AppNotification {
            id: id.to_string(),
            severity: "warning".to_string(),
            title: title.to_string(),
            message: message.to_string(),
            created_at_unix_ms,
            action,
        },
    );
}

/// Clear (remove) a single app notification by its stable id and broadcast the
/// updated list. Mirrors [`push_warning_app_notification`] for the suspend/resume
/// pair: the low-disk warning is pushed on suspend and cleared here on resume.
/// Best-effort — silently no-ops if the notifications state is unavailable.
pub(crate) fn clear_app_notification_by_id(app_handle: &tauri::AppHandle, id: &str) {
    let Some(state) = app_handle.try_state::<AppNotificationsState>() else {
        return;
    };
    let notifications = {
        let mut runtime = state.lock().expect("app notifications state poisoned");
        runtime.clear_one(id)
    };
    emit_app_notifications_changed(app_handle, &notifications);
}

pub(crate) fn push_info_app_notification(
    app_handle: &tauri::AppHandle,
    id: &str,
    title: &str,
    message: &str,
    settings_tab: Option<&str>,
    created_at_unix_ms: u64,
) {
    let action = settings_tab.map(|tab| AppNotificationAction::OpenSettingsTab {
        tab: tab.to_string(),
    });
    push_app_notification(
        app_handle,
        app_handle.state::<AppNotificationsState>().inner(),
        AppNotification {
            id: id.to_string(),
            severity: "info".to_string(),
            title: title.to_string(),
            message: message.to_string(),
            created_at_unix_ms,
            action,
        },
    );
}

/// Push an `error`-severity app notification. Mirrors
/// [`push_warning_app_notification`] but with `severity: "error"`; used for the
/// low-disk graceful-stop notice (ADR 0040), where capture has stopped and the
/// user must free space before recording can be restarted. The frontend already
/// types `severity` as `"info" | "warning" | "error"` and styles the `error`
/// case; an unstyled severity would degrade to the neutral base card.
pub(crate) fn push_error_app_notification(
    app_handle: &tauri::AppHandle,
    id: &str,
    title: &str,
    message: &str,
    settings_tab: Option<&str>,
    created_at_unix_ms: u64,
) {
    let action = settings_tab.map(|tab| AppNotificationAction::OpenSettingsTab {
        tab: tab.to_string(),
    });
    push_app_notification(
        app_handle,
        app_handle.state::<AppNotificationsState>().inner(),
        AppNotification {
            id: id.to_string(),
            severity: "error".to_string(),
            title: title.to_string(),
            message: message.to_string(),
            created_at_unix_ms,
            action,
        },
    );
}

fn should_warn_audio_transcription_unavailable_at_start(settings: &RecordingSettings) -> bool {
    settings.transcription.enabled
        && ((settings.capture_microphone && settings.transcription.microphone_enabled)
            || (settings.capture_system_audio && settings.transcription.system_audio_enabled))
}

fn should_warn_audio_transcription_unavailable_at_startup(settings: &RecordingSettings) -> bool {
    should_warn_audio_transcription_unavailable_at_start(settings)
}

fn audio_transcription_provider_label(provider: AudioTranscriptionProvider) -> &'static str {
    match provider {
        AudioTranscriptionProvider::LocalWhisper => "Local Whisper",
        AudioTranscriptionProvider::AppleSpeechOnDevice => "Apple Speech on-device recognition",
        AudioTranscriptionProvider::Parakeet => "Parakeet",
    }
}

fn audio_transcription_selection_label(settings: &AudioTranscriptionSettings) -> String {
    let provider = audio_transcription_provider_label(settings.provider);
    match settings.model_id.as_deref() {
        Some(model_id) if !model_id.is_empty() => format!("{provider} `{model_id}`"),
        _ => provider.to_string(),
    }
}

fn audio_transcription_unavailable_notification(
    settings: &RecordingSettings,
    created_at_unix_ms: u64,
) -> AppNotification {
    let selection = audio_transcription_selection_label(&settings.transcription);
    AppNotification {
        id: AUDIO_TRANSCRIPTION_UNAVAILABLE_NOTIFICATION_ID.to_string(),
        severity: "warning".to_string(),
        title: "Transcription model unavailable".to_string(),
        message: format!(
            "{selection} is not available. Requested audio will not be transcribed until you install or choose an available model."
        ),
        created_at_unix_ms,
        action: Some(AppNotificationAction::OpenSettingsTab {
            tab: TRANSCRIPTION_SETTINGS_TAB_ID.to_string(),
        }),
    }
}

fn speech_detector_unavailable_notification(created_at_unix_ms: u64) -> AppNotification {
    AppNotification {
        id: SPEECH_DETECTOR_UNAVAILABLE_NOTIFICATION_ID.to_string(),
        severity: "warning".to_string(),
        title: "Speech detector unavailable".to_string(),
        message: "The selected speech detector is unavailable. Choose an available detector before starting this recording.".to_string(),
        created_at_unix_ms,
        action: Some(AppNotificationAction::OpenSettingsTab {
            tab: TRANSCRIPTION_SETTINGS_TAB_ID.to_string(),
        }),
    }
}

fn speaker_analysis_unavailable_notification(created_at_unix_ms: u64) -> AppNotification {
    AppNotification {
        id: SPEAKER_ANALYSIS_UNAVAILABLE_NOTIFICATION_ID.to_string(),
        severity: "warning".to_string(),
        title: "Speaker analysis model unavailable".to_string(),
        message: "The selected speaker analysis model is unavailable. Install or choose an available model before starting this recording.".to_string(),
        created_at_unix_ms,
        action: Some(AppNotificationAction::OpenSettingsTab {
            tab: SPEAKER_SETTINGS_TAB_ID.to_string(),
        }),
    }
}

fn maybe_push_audio_transcription_unavailable_warning(
    app_handle: &tauri::AppHandle,
    app_notifications_state: &AppNotificationsState,
    settings: &RecordingSettings,
    context: &str,
) {
    let app_data_dir = match app_handle.path().app_data_dir() {
        Ok(dir) => dir,
        Err(error) => {
            debug_log::log_warn(format!(
                "failed to resolve app data directory for {context} audio transcription warning: {error}"
            ));
            return;
        }
    };

    match crate::audio_transcription_models::selected_audio_transcription_model_available(
        &app_data_dir,
        &settings.transcription,
    ) {
        Ok(true) => {}
        Ok(false) => {
            let selection = audio_transcription_selection_label(&settings.transcription);
            debug_log::log_warn(format!(
                "audio transcription unavailable at {context} (selection={selection})"
            ));
            push_app_notification(
                app_handle,
                app_notifications_state,
                audio_transcription_unavailable_notification(settings, runtime::now_unix_ms()),
            );
        }
        Err(error) => {
            let selection = audio_transcription_selection_label(&settings.transcription);
            debug_log::log_warn(format!(
                "failed to inspect selected audio transcription model at {context} (selection={selection}): {error}"
            ));
            push_app_notification(
                app_handle,
                app_notifications_state,
                audio_transcription_unavailable_notification(settings, runtime::now_unix_ms()),
            );
        }
    }
}

fn maybe_push_audio_transcription_unavailable_start_warning(
    app_handle: &tauri::AppHandle,
    app_notifications_state: &AppNotificationsState,
    settings: &RecordingSettings,
) {
    if !should_warn_audio_transcription_unavailable_at_start(settings) {
        return;
    }

    maybe_push_audio_transcription_unavailable_warning(
        app_handle,
        app_notifications_state,
        settings,
        "recording start",
    );
}

fn recording_requires_speech_detector(settings: &RecordingSettings) -> bool {
    settings.audio_speech_detection.detector != capture_types::AudioSpeechDetector::Off
        && settings.capture_system_audio
        && settings.transcription.enabled
        && settings.transcription.system_audio_enabled
}

fn selected_speech_detector_available(settings: &RecordingSettings) -> Result<bool, String> {
    if settings.audio_speech_detection.detector == capture_types::AudioSpeechDetector::Off {
        return Ok(false);
    }
    capture_vad::AudioSpeechDetectorRuntime::new(settings.audio_speech_detection.detector)
        .map(|_| true)
        .map_err(|error| error.to_string())
}

fn recording_requires_transcription_model(settings: &RecordingSettings) -> bool {
    settings.transcription.enabled
        && ((settings.capture_microphone && settings.transcription.microphone_enabled)
            || (settings.capture_system_audio && settings.transcription.system_audio_enabled))
}

fn recording_requires_speaker_analysis_model(settings: &RecordingSettings) -> bool {
    settings.speaker_analysis.separate_speakers && recording_requires_transcription_model(settings)
}

fn selected_speaker_analysis_model_available(
    app_data_dir: &std::path::Path,
    settings: &RecordingSettings,
) -> Result<bool, String> {
    let models_dir = speaker_analysis::speaker_analysis_models_dir(app_data_dir);
    let manifest = speaker_analysis::builtin_model_manifest();
    let Some(descriptor) = speaker_analysis::find_model_descriptor(
        &manifest,
        &settings.speaker_analysis.provider,
        settings.speaker_analysis.model_id.as_deref(),
    ) else {
        return Ok(false);
    };
    speaker_analysis::detect_model_status(&models_dir, descriptor)
        .map(|status| status.status == speaker_analysis::ModelStatusKind::Installed)
        .map_err(|error| error.to_string())
}

fn should_warn_ocr_unavailable_at_start(settings: &RecordingSettings) -> bool {
    settings.capture_screen && settings.ocr.enabled
}

fn should_warn_ocr_unavailable_at_startup(settings: &RecordingSettings) -> bool {
    should_warn_ocr_unavailable_at_start(settings)
}

fn ocr_provider_label(provider: OcrProvider) -> &'static str {
    match provider {
        OcrProvider::AppleVision => "Apple Vision",
        OcrProvider::Tesseract => "Tesseract",
        OcrProvider::PaddleOcr => "PaddleOCR",
    }
}

fn ocr_selection_label(settings: &OcrSettings) -> String {
    let provider = ocr_provider_label(settings.provider);
    match settings
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(model_id) => format!("{provider} `{model_id}`"),
        None => provider.to_string(),
    }
}

fn ocr_unavailable_notification(
    settings: &RecordingSettings,
    created_at_unix_ms: u64,
) -> AppNotification {
    let selection = ocr_selection_label(&settings.ocr);
    AppNotification {
        id: OCR_UNAVAILABLE_NOTIFICATION_ID.to_string(),
        severity: "warning".to_string(),
        title: "OCR engine unavailable".to_string(),
        message: format!(
            "{selection} is not available. Screen recording is blocked until you install or choose an available OCR engine."
        ),
        created_at_unix_ms,
        action: Some(AppNotificationAction::OpenSettingsTab {
            tab: PROCESSING_SETTINGS_TAB_ID.to_string(),
        }),
    }
}

fn maybe_push_ocr_unavailable_warning(
    app_handle: &tauri::AppHandle,
    app_notifications_state: &AppNotificationsState,
    settings: &RecordingSettings,
    context: &str,
) {
    let app_data_dir = match app_handle.path().app_data_dir() {
        Ok(dir) => dir,
        Err(error) => {
            debug_log::log_warn(format!(
                "failed to resolve app data directory for {context} OCR warning: {error}"
            ));
            return;
        }
    };

    match crate::ocr_models::selected_ocr_model_available(&app_data_dir, &settings.ocr) {
        Ok(true) => {}
        Ok(false) => {
            let selection = ocr_selection_label(&settings.ocr);
            debug_log::log_warn(format!(
                "ocr unavailable at {context} (selection={selection})"
            ));
            push_app_notification(
                app_handle,
                app_notifications_state,
                ocr_unavailable_notification(settings, runtime::now_unix_ms()),
            );
        }
        Err(error) => {
            let selection = ocr_selection_label(&settings.ocr);
            debug_log::log_warn(format!(
                "failed to inspect selected OCR model at {context} (selection={selection}): {error}"
            ));
            push_app_notification(
                app_handle,
                app_notifications_state,
                ocr_unavailable_notification(settings, runtime::now_unix_ms()),
            );
        }
    }
}

fn maybe_push_ocr_unavailable_start_warning(
    app_handle: &tauri::AppHandle,
    app_notifications_state: &AppNotificationsState,
    settings: &RecordingSettings,
) {
    if !should_warn_ocr_unavailable_at_start(settings) {
        return;
    }

    maybe_push_ocr_unavailable_warning(
        app_handle,
        app_notifications_state,
        settings,
        "recording start",
    );
}

pub fn maybe_push_audio_transcription_unavailable_startup_warning(app_handle: &tauri::AppHandle) {
    let settings_state = app_handle.state::<RecordingSettingsState>();
    let settings = current_recording_settings(settings_state.inner());
    if !should_warn_audio_transcription_unavailable_at_startup(&settings) {
        return;
    }

    maybe_push_audio_transcription_unavailable_warning(
        app_handle,
        app_handle.state::<AppNotificationsState>().inner(),
        &settings,
        "app startup",
    );
}

pub fn maybe_push_ocr_unavailable_startup_warning(app_handle: &tauri::AppHandle) {
    let settings_state = app_handle.state::<RecordingSettingsState>();
    let settings = current_recording_settings(settings_state.inner());
    if !should_warn_ocr_unavailable_at_startup(&settings) {
        return;
    }

    maybe_push_ocr_unavailable_warning(
        app_handle,
        app_handle.state::<AppNotificationsState>().inner(),
        &settings,
        "app startup",
    );
}

#[cfg(target_os = "macos")]
fn handle_system_will_sleep(app_handle: &tauri::AppHandle) {
    let state = app_handle.state::<NativeCaptureState>();
    let mut runtime = match state.lock() {
        Ok(runtime) => runtime,
        Err(_) => return,
    };

    if runtime.handle_system_will_sleep() {
        let runtime_state = runtime.runtime();
        debug_log::log_info(format!(
            "marked screen capture inactive for system sleep (session_id='{}', requested_sources={})",
            runtime_log_session_id(runtime_state),
            format_optional_capture_source_flags(runtime_state.requested_sources.as_ref())
        ));
    }
}

#[cfg(target_os = "macos")]
fn recover_screen_capture_after_system_wake_once(
    app_handle: &tauri::AppHandle,
) -> Result<bool, CaptureErrorResponse> {
    let state = app_handle.state::<NativeCaptureState>();
    let mut runtime = state.lock().map_err(|_| CaptureErrorResponse {
        code: "native_capture_state_poisoned".to_string(),
        message: "Native capture state is unavailable while recovering after system wake"
            .to_string(),
    })?;

    let outcome = runtime.recover_after_wake(Some(app_handle));
    let runtime_state = runtime.runtime();
    match &outcome {
        Ok(true) => {
            debug_log::log_info(format!(
                "recovered screen capture after system wake (session_id='{}', requested_sources={})",
                runtime_log_session_id(runtime_state),
                format_optional_capture_source_flags(runtime_state.requested_sources.as_ref())
            ));
        }
        Ok(false) => {}
        Err(error) => {
            debug_log::log_error(format!(
                "failed to recover screen capture after system wake (session_id='{}', requested_sources={}): [{}] {}",
                runtime_log_session_id(runtime_state),
                format_optional_capture_source_flags(runtime_state.requested_sources.as_ref()),
                error.code,
                error.message
            ));
        }
    }

    outcome
}

#[cfg(target_os = "macos")]
fn is_recover_after_wake_retryable_error(error: &CaptureErrorResponse) -> bool {
    matches!(
        error.code.as_str(),
        "capture_stream_start_failed"
            | "capture_stream_start_timeout"
            | "capture_shareable_content_failed"
            | "capture_shareable_content_timeout"
            | "capture_shareable_content_unavailable"
            | "capture_display_unavailable"
    ) || error
        .message
        .contains("Failed to find any displays or windows")
        || error.message.contains("code: -3815")
}

#[cfg(target_os = "macos")]
fn log_scheduled_system_wake_recovery_retry(error: &CaptureErrorResponse, delay_ms: u64) {
    debug_log::log_warn(format!(
        "screen capture wake recovery hit a transient ScreenCaptureKit error; retrying in {}ms: [{}] {}",
        delay_ms, error.code, error.message
    ));
}

#[cfg(target_os = "macos")]
fn system_wake_recovery_in_progress() -> &'static AtomicBool {
    static IN_PROGRESS: OnceLock<AtomicBool> = OnceLock::new();
    IN_PROGRESS.get_or_init(|| AtomicBool::new(false))
}

#[cfg(target_os = "macos")]
fn begin_system_wake_recovery() -> bool {
    system_wake_recovery_in_progress()
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

#[cfg(target_os = "macos")]
fn finish_system_wake_recovery() {
    system_wake_recovery_in_progress().store(false, Ordering::Release);
}

#[cfg(target_os = "macos")]
fn retry_screen_capture_recovery_after_system_wake(
    app_handle: tauri::AppHandle,
    mut last_error: CaptureErrorResponse,
) {
    std::thread::spawn(move || {
        for delay_ms in SYSTEM_WAKE_RECOVERY_RETRY_DELAYS_MS {
            log_scheduled_system_wake_recovery_retry(&last_error, *delay_ms);
            std::thread::sleep(Duration::from_millis(*delay_ms));

            match recover_screen_capture_after_system_wake_once(&app_handle) {
                Ok(_) => {
                    finish_system_wake_recovery();
                    emit_system_did_wake(&app_handle);
                    return;
                }
                Err(error) if is_recover_after_wake_retryable_error(&error) => {
                    last_error = error;
                }
                Err(_) => {
                    finish_system_wake_recovery();
                    emit_system_did_wake(&app_handle);
                    return;
                }
            }
        }

        finish_system_wake_recovery();
        emit_system_did_wake(&app_handle);
    });
}

#[cfg(target_os = "macos")]
fn recover_screen_capture_after_system_wake(app_handle: tauri::AppHandle) {
    if !begin_system_wake_recovery() {
        return;
    }

    match recover_screen_capture_after_system_wake_once(&app_handle) {
        Ok(_) => {
            finish_system_wake_recovery();
            emit_system_did_wake(&app_handle);
        }
        Err(error) if is_recover_after_wake_retryable_error(&error) => {
            retry_screen_capture_recovery_after_system_wake(app_handle, error);
        }
        Err(_) => {
            finish_system_wake_recovery();
            emit_system_did_wake(&app_handle);
        }
    }
}

#[cfg(target_os = "macos")]
fn recover_screen_capture_after_possible_missed_wake(app_handle: tauri::AppHandle) {
    let state = app_handle.state::<NativeCaptureState>();
    let should_recover = state
        .lock()
        .map(|runtime| runtime.should_attempt_recovery_after_possible_wake())
        .unwrap_or(false);

    if !should_recover {
        return;
    }

    debug_log::log_info(
        "attempting screen capture recovery during session resync after possible missed system wake notification"
            .to_string(),
    );
    recover_screen_capture_after_system_wake(app_handle);
}

#[cfg(target_os = "macos")]
pub fn start_system_wake_notifier(app_handle: tauri::AppHandle) {
    use cidre::ns;

    let mut center = ns::Workspace::shared().notification_center();
    let will_sleep_guard =
        center.add_observer_guard(ns::workspace::notification::will_sleep(), None, None, {
            let app_handle = app_handle.clone();
            move |_notification| {
                handle_system_will_sleep(&app_handle);
            }
        });
    let did_wake_guard =
        center.add_observer_guard(ns::workspace::notification::did_wake(), None, None, {
            let app_handle = app_handle.clone();
            move |_notification| {
                recover_screen_capture_after_system_wake(app_handle.clone());
            }
        });

    let notifier_state = app_handle.state::<SystemWakeNotifierState>();
    let mut notifier_slot = notifier_state
        .0
        .lock()
        .expect("system wake notifier state poisoned");
    notifier_slot.clear();
    notifier_slot.push(will_sleep_guard);
    notifier_slot.push(did_wake_guard);
}

#[cfg(not(target_os = "macos"))]
pub fn start_system_wake_notifier(_app_handle: tauri::AppHandle) {}

// `NSWorkspaceDidWake` is only posted on a *full* wake; dark wakes, Power Nap,
// and "Wake from Deep Idle" never post it, so capture would silently stay
// paused until the next frontend permissions poll. The display panel powering
// back up *does* drive a Core Graphics display reconfiguration, which we listen
// on as the definitive, polling-free re-arm signal. This also covers external
// monitor disconnect/reconnect — the other half of ADR 0021's
// "display-unavailable as transient liveness".
#[cfg(target_os = "macos")]
fn display_reconfiguration_recovery_app_handle() -> &'static Mutex<Option<tauri::AppHandle>> {
    static APP_HANDLE: OnceLock<Mutex<Option<tauri::AppHandle>>> = OnceLock::new();
    APP_HANDLE.get_or_init(|| Mutex::new(None))
}

// Decide whether a reconfiguration callback's flags represent a display coming
// (back) online at the *end* of a configuration pass — the moment recovery
// should fire. We deliberately ignore the begin-configuration notification (the
// pre-change half of every begin/end pair) and pure offline transitions
// (remove/disable with nothing bringing a display online), so recovery runs
// exactly once per reconfiguration and never on a display going away.
#[cfg(target_os = "macos")]
fn display_reconfiguration_flags_indicate_display_online(flags: u32) -> bool {
    use core_graphics::display::CGDisplayChangeSummaryFlags as F;

    let flags = F::from_bits_retain(flags);

    // The begin-configuration notification is the "about to change" half; wait
    // for the matching end notification (begin flag clear) before re-arming.
    if flags.contains(F::kCGDisplayBeginConfigurationFlag) {
        return false;
    }

    // A display is now present/active if it was added, enabled, became main, or
    // (re)acquired a mode. A reconfiguration that only removes/disables a
    // display is a display going away, not a wake — skip it.
    flags.intersects(
        F::kCGDisplayAddFlag
            | F::kCGDisplayEnabledFlag
            | F::kCGDisplaySetMainFlag
            | F::kCGDisplaySetModeFlag,
    )
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn display_reconfiguration_callback(
    _display: core_graphics::display::CGDirectDisplayID,
    flags: u32,
    _user_info: *const std::ffi::c_void,
) {
    if !display_reconfiguration_flags_indicate_display_online(flags) {
        return;
    }

    let app_handle = display_reconfiguration_recovery_app_handle()
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    if let Some(app_handle) = app_handle {
        // Funnel through the shared recovery path: it is atomic-guarded
        // (`begin/finish_system_wake_recovery`) and re-checks `!session_is_live`,
        // so firing alongside the `NSWorkspaceDidWake` fallback is idempotent.
        recover_screen_capture_after_system_wake(app_handle);
    }
}

#[cfg(target_os = "macos")]
struct DisplayReconfigurationCallbackGuard;

#[cfg(target_os = "macos")]
impl Drop for DisplayReconfigurationCallbackGuard {
    fn drop(&mut self) {
        unsafe {
            core_graphics::display::CGDisplayRemoveReconfigurationCallback(
                display_reconfiguration_callback,
                std::ptr::null(),
            );
        }
    }
}

#[cfg(target_os = "macos")]
pub fn start_display_reconfiguration_notifier(app_handle: tauri::AppHandle) {
    use core_graphics::base::{kCGErrorSuccess, CGError};

    // The C callback can't capture state, so stash the handle where it can reach
    // it. Set before registering so no early callback races a missing handle.
    if let Ok(mut slot) = display_reconfiguration_recovery_app_handle().lock() {
        *slot = Some(app_handle.clone());
    }

    let error: CGError = unsafe {
        core_graphics::display::CGDisplayRegisterReconfigurationCallback(
            display_reconfiguration_callback,
            std::ptr::null(),
        )
    };
    if error != kCGErrorSuccess {
        debug_log::log_warn(format!(
            "failed to register display reconfiguration callback for wake recovery (CGError {error}); \
             relying on NSWorkspaceDidWake fallback"
        ));
        return;
    }

    let notifier_state = app_handle.state::<DisplayReconfigurationNotifierState>();
    let mut slot = notifier_state
        .0
        .lock()
        .expect("display reconfiguration notifier state poisoned");
    *slot = Some(DisplayReconfigurationCallbackGuard);
}

#[cfg(not(target_os = "macos"))]
pub fn start_display_reconfiguration_notifier(_app_handle: tauri::AppHandle) {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CaptureSupportSnapshot {
    platform: String,
    native_capture_supported: bool,
    supported_sources: CaptureSources,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturePermissionsSnapshot {
    screen: &'static str,
    microphone: &'static str,
    system_audio: &'static str,
}

fn capture_support_log_snapshot_state() -> &'static std::sync::Mutex<Option<CaptureSupportSnapshot>>
{
    static LAST_CAPTURE_SUPPORT_SNAPSHOT: OnceLock<
        std::sync::Mutex<Option<CaptureSupportSnapshot>>,
    > = OnceLock::new();

    LAST_CAPTURE_SUPPORT_SNAPSHOT.get_or_init(|| std::sync::Mutex::new(None))
}

fn capture_permissions_log_snapshot_state(
) -> &'static std::sync::Mutex<Option<CapturePermissionsSnapshot>> {
    static LAST_CAPTURE_PERMISSIONS_SNAPSHOT: OnceLock<
        std::sync::Mutex<Option<CapturePermissionsSnapshot>>,
    > = OnceLock::new();

    LAST_CAPTURE_PERMISSIONS_SNAPSHOT.get_or_init(|| std::sync::Mutex::new(None))
}

fn reset_capture_log_snapshots() {
    *capture_support_log_snapshot_state()
        .lock()
        .expect("capture support log snapshot poisoned") = None;
    *capture_permissions_log_snapshot_state()
        .lock()
        .expect("capture permissions log snapshot poisoned") = None;
}

fn capture_sources_from_settings(settings: &RecordingSettings) -> CaptureSources {
    CaptureSources {
        screen: settings.capture_screen,
        microphone: settings.capture_microphone,
        system_audio: settings.capture_system_audio,
    }
}

fn capture_sources_from_start_request(request: &StartNativeCaptureRequest) -> CaptureSources {
    CaptureSources {
        screen: request.capture_screen,
        microphone: request.capture_microphone,
        system_audio: request.capture_system_audio,
    }
}

fn format_capture_source_flags(sources: &CaptureSources) -> String {
    format!(
        "screen={}, microphone={}, system_audio={}",
        sources.screen, sources.microphone, sources.system_audio
    )
}

fn format_optional_capture_source_flags(sources: Option<&CaptureSources>) -> String {
    sources
        .map(format_capture_source_flags)
        .unwrap_or_else(|| "screen=unknown, microphone=unknown, system_audio=unknown".to_string())
}

fn runtime_log_session_id(runtime: &runtime::NativeCaptureRuntime) -> &str {
    runtime
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.screen.as_ref())
        .map(|session| session.session_id.as_str())
        .unwrap_or("unknown")
}

fn session_log_session_id(session: &capture_types::NativeCaptureSession) -> &str {
    session
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.screen.as_ref())
        .map(|source| source.session_id.as_str())
        .unwrap_or("unknown")
}

fn permission_state_label(state: &CapturePermissionState) -> &'static str {
    match state {
        CapturePermissionState::Granted => "granted",
        CapturePermissionState::Denied => "denied",
        CapturePermissionState::NotDetermined => "not_determined",
        CapturePermissionState::Unsupported => "unsupported",
        CapturePermissionState::Unknown => "unknown",
    }
}

fn format_screen_resolution(resolution: &ScreenResolution) -> String {
    match resolution {
        ScreenResolution::Preset { preset } => match preset {
            ScreenResolutionPreset::Original => "original".to_string(),
            ScreenResolutionPreset::P1080 => "1080p".to_string(),
            ScreenResolutionPreset::P720 => "720p".to_string(),
            ScreenResolutionPreset::P540 => "540p".to_string(),
        },
        ScreenResolution::Custom { width, height } => format!("{width}x{height}"),
    }
}

fn format_video_bitrate(settings: &VideoBitrateSettings) -> String {
    match settings.mode {
        VideoBitrateMode::Preset => {
            let preset = settings
                .preset
                .clone()
                .unwrap_or(VideoBitratePreset::Medium);
            let label = match preset {
                VideoBitratePreset::Low => "low",
                VideoBitratePreset::Medium => "medium",
                VideoBitratePreset::High => "high",
            };

            format!("preset:{label}")
        }
        VideoBitrateMode::Custom => format!("custom:{}mbps", settings.custom_mbps.unwrap_or(0)),
    }
}

fn inactivity_activity_mode_label(mode: &InactivityActivityMode) -> &'static str {
    match mode {
        InactivityActivityMode::SystemInputOnly => "system_input_only",
        InactivityActivityMode::SystemInputOrScreen => "system_input_or_screen",
        InactivityActivityMode::SystemInputOrScreenOrAudio => "system_input_or_screen_or_audio",
    }
}

fn recording_settings_overview(settings: &RecordingSettings) -> String {
    format!(
        "sources={}, auto_start={}, save_directory='{}', debug_logging={}, preview_cache_ttl_seconds={}, follow_timeline_live={}, segment_duration_seconds={}, screen_frame_rate={}, screen_resolution={}, video_bitrate={}, pause_on_inactivity={}, idle_timeout_seconds={}, microphone_activity_sensitivity={}, system_audio_activity_sensitivity={}, activity_mode={}",
        format_capture_source_flags(&capture_sources_from_settings(settings)),
        settings.auto_start,
        settings.save_directory,
        settings.native_capture_debug_logging_enabled,
        settings.preview_cache_ttl_seconds,
        settings.follow_timeline_live,
        settings.segment_duration_seconds,
        settings.screen_frame_rate,
        format_screen_resolution(&settings.screen_resolution),
        format_video_bitrate(&settings.video_bitrate),
        settings.pause_capture_on_inactivity,
        settings.idle_timeout_seconds,
        settings.microphone_activity_sensitivity,
        settings.system_audio_activity_sensitivity,
        inactivity_activity_mode_label(&settings.inactivity_activity_mode)
    )
}

fn describe_recording_settings_changes(
    previous: &RecordingSettings,
    next: &RecordingSettings,
) -> Vec<String> {
    let mut changes = Vec::new();
    let previous_sources = capture_sources_from_settings(previous);
    let next_sources = capture_sources_from_settings(next);

    if previous_sources != next_sources {
        changes.push(format!(
            "sources {} -> {}",
            format_capture_source_flags(&previous_sources),
            format_capture_source_flags(&next_sources)
        ));
    }

    if previous.auto_start != next.auto_start {
        changes.push(format!(
            "auto_start {} -> {}",
            previous.auto_start, next.auto_start
        ));
    }

    if previous.native_capture_debug_logging_enabled != next.native_capture_debug_logging_enabled {
        changes.push(format!(
            "debug_logging {} -> {}",
            previous.native_capture_debug_logging_enabled,
            next.native_capture_debug_logging_enabled
        ));
    }

    if previous.preview_cache_ttl_seconds != next.preview_cache_ttl_seconds {
        changes.push(format!(
            "preview_cache_ttl_seconds {} -> {}",
            previous.preview_cache_ttl_seconds, next.preview_cache_ttl_seconds
        ));
    }

    if previous.follow_timeline_live != next.follow_timeline_live {
        changes.push(format!(
            "follow_timeline_live {} -> {}",
            previous.follow_timeline_live, next.follow_timeline_live
        ));
    }

    if previous.segment_duration_seconds != next.segment_duration_seconds {
        changes.push(format!(
            "segment_duration_seconds {} -> {}",
            previous.segment_duration_seconds, next.segment_duration_seconds
        ));
    }

    if previous.screen_frame_rate != next.screen_frame_rate {
        changes.push(format!(
            "screen_frame_rate {} -> {}",
            previous.screen_frame_rate, next.screen_frame_rate
        ));
    }

    if previous.screen_resolution != next.screen_resolution {
        changes.push(format!(
            "screen_resolution {} -> {}",
            format_screen_resolution(&previous.screen_resolution),
            format_screen_resolution(&next.screen_resolution)
        ));
    }

    if previous.video_bitrate != next.video_bitrate {
        changes.push(format!(
            "video_bitrate {} -> {}",
            format_video_bitrate(&previous.video_bitrate),
            format_video_bitrate(&next.video_bitrate)
        ));
    }

    if previous.pause_capture_on_inactivity != next.pause_capture_on_inactivity {
        changes.push(format!(
            "pause_on_inactivity {} -> {}",
            previous.pause_capture_on_inactivity, next.pause_capture_on_inactivity
        ));
    }

    if previous.idle_timeout_seconds != next.idle_timeout_seconds {
        changes.push(format!(
            "idle_timeout_seconds {} -> {}",
            previous.idle_timeout_seconds, next.idle_timeout_seconds
        ));
    }

    if previous.inactivity_activity_mode != next.inactivity_activity_mode {
        changes.push(format!(
            "activity_mode {} -> {}",
            inactivity_activity_mode_label(&previous.inactivity_activity_mode),
            inactivity_activity_mode_label(&next.inactivity_activity_mode)
        ));
    }

    if previous.microphone_activity_sensitivity != next.microphone_activity_sensitivity {
        changes.push(format!(
            "microphone_activity_sensitivity {} -> {}",
            previous.microphone_activity_sensitivity, next.microphone_activity_sensitivity
        ));
    }

    if previous.system_audio_activity_sensitivity != next.system_audio_activity_sensitivity {
        changes.push(format!(
            "system_audio_activity_sensitivity {} -> {}",
            previous.system_audio_activity_sensitivity, next.system_audio_activity_sensitivity
        ));
    }

    changes
}

fn format_output_file_counts(output_files: Option<&CaptureOutputFiles>) -> String {
    output_files
        .map(|output_files| {
            format!(
                "screen_files={}, microphone_files={}, system_audio_files={}",
                output_files.screen_files.len(),
                output_files.microphone_files.len(),
                output_files.system_audio_files.len()
            )
        })
        .unwrap_or_else(|| "screen_files=0, microphone_files=0, system_audio_files=0".to_string())
}

fn log_capture_support_if_changed(response: &CaptureSupportResponse) {
    let snapshot = CaptureSupportSnapshot {
        platform: response.platform.clone(),
        native_capture_supported: response.native_capture_supported,
        supported_sources: response.supported_sources.clone(),
    };
    let mut last_snapshot = capture_support_log_snapshot_state()
        .lock()
        .expect("capture support log snapshot poisoned");

    if last_snapshot.as_ref() == Some(&snapshot) {
        return;
    }

    *last_snapshot = Some(snapshot.clone());

    debug_log::log(format!(
        "observed native capture support (platform='{}', native_supported={}, supported_sources={})",
        snapshot.platform,
        snapshot.native_capture_supported,
        format_capture_source_flags(&snapshot.supported_sources)
    ));
}

fn log_capture_permissions_if_changed(permissions: &CapturePermissions) {
    let snapshot = CapturePermissionsSnapshot {
        screen: permission_state_label(&permissions.screen),
        microphone: permission_state_label(&permissions.microphone),
        system_audio: permission_state_label(&permissions.system_audio),
    };
    let mut last_snapshot = capture_permissions_log_snapshot_state()
        .lock()
        .expect("capture permissions log snapshot poisoned");

    if last_snapshot.as_ref() == Some(&snapshot) {
        return;
    }

    *last_snapshot = Some(snapshot.clone());

    debug_log::log(format!(
        "observed native capture permissions (screen={}, microphone={}, system_audio={})",
        snapshot.screen, snapshot.microphone, snapshot.system_audio
    ));
}

fn log_loaded_recording_settings(source: &str, settings: &RecordingSettings) {
    debug_log::log_info(format!(
        "loaded recording settings from {source} ({})",
        recording_settings_overview(settings)
    ));
}

fn log_recording_settings_changes(previous: &RecordingSettings, next: &RecordingSettings) {
    let changes = describe_recording_settings_changes(previous, next);

    if changes.is_empty() {
        return;
    }

    debug_log::log_info(format!(
        "updated recording settings ({})",
        changes.join(", ")
    ));
}

fn start_native_capture_inner(
    origin: &str,
    request: StartNativeCaptureRequest,
    state: tauri::State<'_, NativeCaptureState>,
    microphone_controller_preferences_state: tauri::State<'_, MicrophoneControllerPreferencesState>,
    recording_settings_state: tauri::State<'_, RecordingSettingsState>,
    app_notifications_state: tauri::State<'_, AppNotificationsState>,
    app_handle: tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let incoming_sources = capture_sources_from_start_request(&request);
    let settings = recording_settings_state.inner();
    let settings = current_recording_settings(settings);

    let resolved_request = StartNativeCaptureRequest {
        capture_screen: settings.capture_screen,
        capture_microphone: settings.capture_microphone,
        capture_system_audio: settings.capture_system_audio,
    };
    let resolved_sources = capture_sources_from_start_request(&resolved_request);

    debug_log::log_info(format!(
        "attempting native capture {origin} start (incoming_sources={}, resolved_sources={}, save_directory='{}')",
        format_capture_source_flags(&incoming_sources),
        format_capture_source_flags(&resolved_sources),
        settings.save_directory
    ));

    let support = get_capture_support();
    let sources = match validate_start_request(&resolved_request, &support) {
        Ok(sources) => sources,
        Err(error) => {
            debug_log::log_warn(format!(
                "rejected native capture {origin} start during source validation (resolved_sources={}, supported_sources={}): [{}] {}",
                format_capture_source_flags(&resolved_sources),
                format_capture_source_flags(&support.supported_sources),
                error.code,
                error.message
            ));
            return Err(error);
        }
    };

    if resolved_request.capture_screen && settings.ocr.enabled {
        let app_data_dir =
            app_handle
                .path()
                .app_data_dir()
                .map_err(|error| CaptureErrorResponse {
                    code: "ocr_model_unavailable".to_string(),
                    message: format!(
                        "failed to resolve app data directory for OCR preflight: {error}"
                    ),
                })?;
        match crate::ocr_models::selected_ocr_model_available(&app_data_dir, &settings.ocr) {
            Ok(true) => {}
            Ok(false) => {
                maybe_push_ocr_unavailable_start_warning(
                    &app_handle,
                    app_notifications_state.inner(),
                    &settings,
                );
                let error = CaptureErrorResponse {
                    code: "ocr_model_unavailable".to_string(),
                    message: format!(
                        "{} is unavailable. Install or choose an available OCR engine before recording screen capture.",
                        ocr_selection_label(&settings.ocr)
                    ),
                };
                debug_log::log_warn(format!(
                    "rejected native capture {origin} start because OCR is unavailable: [{}] {}",
                    error.code, error.message
                ));
                return Err(error);
            }
            Err(status_error) => {
                maybe_push_ocr_unavailable_start_warning(
                    &app_handle,
                    app_notifications_state.inner(),
                    &settings,
                );
                let error = CaptureErrorResponse {
                    code: "ocr_model_unavailable".to_string(),
                    message: format!(
                        "failed to verify OCR availability for {}: {status_error}",
                        ocr_selection_label(&settings.ocr)
                    ),
                };
                debug_log::log_warn(format!(
                    "rejected native capture {origin} start because OCR availability check failed: [{}] {}",
                    error.code, error.message
                ));
                return Err(error);
            }
        }
    }

    if recording_requires_speech_detector(&settings) {
        match selected_speech_detector_available(&settings) {
            Ok(true) => {}
            Ok(false) => {
                push_app_notification(
                    &app_handle,
                    app_notifications_state.inner(),
                    speech_detector_unavailable_notification(runtime::now_unix_ms()),
                );
                return Err(CaptureErrorResponse {
                    code: "speech_detector_unavailable".to_string(),
                    message: "Selected speech detector is unavailable for the requested recording sources.".to_string(),
                });
            }
            Err(error) => {
                push_app_notification(
                    &app_handle,
                    app_notifications_state.inner(),
                    speech_detector_unavailable_notification(runtime::now_unix_ms()),
                );
                return Err(CaptureErrorResponse {
                    code: "speech_detector_unavailable".to_string(),
                    message: format!("Failed to verify selected speech detector: {error}"),
                });
            }
        }
    }

    let app_data_dir_for_processing = if recording_requires_transcription_model(&settings)
        || recording_requires_speaker_analysis_model(&settings)
    {
        Some(
            app_handle
                .path()
                .app_data_dir()
                .map_err(|error| CaptureErrorResponse {
                    code: "processing_model_unavailable".to_string(),
                    message: format!(
                        "failed to resolve app data directory for processing preflight: {error}"
                    ),
                })?,
        )
    } else {
        None
    };

    if recording_requires_transcription_model(&settings) {
        let app_data_dir = app_data_dir_for_processing
            .as_deref()
            .expect("processing dir should exist");
        match crate::audio_transcription_models::selected_audio_transcription_model_available(
            app_data_dir,
            &settings.transcription,
        ) {
            Ok(true) => {}
            Ok(false) => {
                maybe_push_audio_transcription_unavailable_start_warning(
                    &app_handle,
                    app_notifications_state.inner(),
                    &settings,
                );
                return Err(CaptureErrorResponse {
                    code: "audio_transcription_model_unavailable".to_string(),
                    message: format!(
                        "{} is unavailable. Install or choose an available transcription model before recording requested audio.",
                        audio_transcription_selection_label(&settings.transcription)
                    ),
                });
            }
            Err(error) => {
                maybe_push_audio_transcription_unavailable_start_warning(
                    &app_handle,
                    app_notifications_state.inner(),
                    &settings,
                );
                return Err(CaptureErrorResponse {
                    code: "audio_transcription_model_unavailable".to_string(),
                    message: format!("failed to verify transcription model availability: {error}"),
                });
            }
        }
    }

    if recording_requires_speaker_analysis_model(&settings) {
        let app_data_dir = app_data_dir_for_processing
            .as_deref()
            .expect("processing dir should exist");
        match selected_speaker_analysis_model_available(app_data_dir, &settings) {
            Ok(true) => {}
            Ok(false) => {
                push_app_notification(
                    &app_handle,
                    app_notifications_state.inner(),
                    speaker_analysis_unavailable_notification(runtime::now_unix_ms()),
                );
                return Err(CaptureErrorResponse {
                    code: "speaker_analysis_model_unavailable".to_string(),
                    message: "Selected speaker analysis model is unavailable. Install or choose an available model before recording requested audio.".to_string(),
                });
            }
            Err(error) => {
                push_app_notification(
                    &app_handle,
                    app_notifications_state.inner(),
                    speaker_analysis_unavailable_notification(runtime::now_unix_ms()),
                );
                return Err(CaptureErrorResponse {
                    code: "speaker_analysis_model_unavailable".to_string(),
                    message: format!(
                        "failed to verify speaker analysis model availability: {error}"
                    ),
                });
            }
        }
    }

    let microphone_device_id_for_capture = if resolved_request.capture_microphone {
        let preferences_runtime = microphone_controller_preferences_state
            .lock()
            .expect("microphone controller preferences state poisoned");
        let controller_state = match microphone_capture::microphone_controller_state(
            preferences_runtime.preference.clone(),
            preferences_runtime.disconnect_policy.clone(),
        ) {
            Ok(state) => state,
            Err(error) => {
                debug_log::log_error(format!(
                    "failed to resolve microphone controller state for native capture {origin} start: [{}] {}",
                    error.code, error.message
                ));
                return Err(error);
            }
        };

        if should_wait_for_same_microphone_device(&controller_state) {
            let error = CaptureErrorResponse {
                code: "microphone_device_unavailable_waiting_for_selected_device".to_string(),
                message: "The selected microphone is unavailable. Reconnect the same device or change microphone preference."
                    .to_string(),
            };
            debug_log::log_warn(format!(
                "rejected native capture {origin} start because the selected microphone is unavailable and wait-for-same-device is active: [{}] {}",
                error.code, error.message
            ));
            return Err(error);
        }

        resolve_capture_microphone_device_id(&controller_state)
    } else {
        None
    };

    let mut runtime = state.lock().expect("native capture state poisoned");
    if runtime.runtime().is_running {
        let existing_sources =
            format_optional_capture_source_flags(runtime.runtime().requested_sources.as_ref());
        let session_id = runtime_log_session_id(runtime.runtime());

        if runtime.runtime().requested_sources.as_ref() != Some(&sources) {
            let error = CaptureErrorResponse {
                code: "capture_session_already_running".to_string(),
                message: "A native capture session is already running with different sources"
                    .to_string(),
            };
            debug_log::log_warn(format!(
                "rejected native capture {origin} start because another session is already running (session_id='{}', existing_sources={}, requested_sources={}): [{}] {}",
                session_id,
                existing_sources,
                format_capture_source_flags(&sources),
                error.code,
                error.message
            ));
            return Err(error);
        }

        debug_log::log_info(format!(
            "native capture {origin} start requested while session is already running; returning existing session (session_id='{}', requested_sources={})",
            session_id, existing_sources
        ));

        return Ok(NativeCaptureSessionResponse {
            session: runtime.session(),
        });
    }

    let requested_sources_for_log = sources.clone();
    let started_session = match runtime.start(
        app_handle.clone(),
        &settings,
        sources,
        microphone_device_id_for_capture,
    ) {
        Ok(StartRecordingLifecycleOutcome::Started(session)) => session,
        Ok(StartRecordingLifecycleOutcome::AlreadyRunning(session)) => {
            debug_log::log_info(format!(
                "native capture {origin} start requested while session is already running; returning existing session (session_id='{}', requested_sources={})",
                session_log_session_id(&session),
                format_optional_capture_source_flags(session.requested_sources.as_ref())
            ));

            return Ok(NativeCaptureSessionResponse { session });
        }
        Err(error) => {
            debug_log::log_error(format!(
                "failed to start native capture ({origin}, requested_sources={}): [{}] {}",
                format_capture_source_flags(&requested_sources_for_log),
                error.code,
                error.message
            ));
            return Err(error);
        }
    };

    debug_log::log_info(format!(
        "started native capture successfully ({origin}, session_id='{}', requested_sources={}, segment_index={}, save_directory='{}')",
        runtime_log_session_id(runtime.runtime()),
        format_optional_capture_source_flags(runtime.runtime().requested_sources.as_ref()),
        runtime.runtime().current_segment_index,
        settings.save_directory
    ));

    maybe_push_audio_transcription_unavailable_start_warning(
        &app_handle,
        app_notifications_state.inner(),
        &settings,
    );
    if let Some(notice) = runtime.take_microphone_vad_fallback_notification() {
        let message = format!(
            "Configured microphone VAD '{}' could not run. Using '{}' for this recording session.",
            configured_adapter_as_str(notice.configured_adapter),
            notice.effective_adapter.as_str(),
        );
        debug_log::log_warn(format!(
            "microphone VAD fallback active: configured_adapter={}, effective_adapter={}, reason={}",
            configured_adapter_as_str(notice.configured_adapter),
            notice.effective_adapter.as_str(),
            notice.reason
        ));
        push_app_notification(
            &app_handle,
            app_notifications_state.inner(),
            AppNotification {
                id: format!(
                    "microphone-vad-fallback-{}",
                    configured_adapter_as_str(notice.configured_adapter)
                ),
                severity: "warning".to_string(),
                title: "Microphone VAD fallback".to_string(),
                message,
                created_at_unix_ms: runtime::now_unix_ms(),
                action: None,
            },
        );
    }

    Ok(NativeCaptureSessionResponse {
        session: started_session,
    })
}

#[tauri::command]
pub fn get_capture_support() -> CaptureSupportResponse {
    let screen_support = capture_screen::support_for_current_platform();
    let microphone_supported = !matches!(
        microphone_capture::microphone_permission_state(),
        CapturePermissionState::Unsupported
    );

    let response = CaptureSupportResponse {
        platform: screen_support.platform,
        native_capture_supported: screen_support.native_capture_supported,
        supported_sources: CaptureSources {
            screen: screen_support.screen,
            microphone: microphone_supported,
            system_audio: screen_support.system_audio,
        },
    };

    log_capture_support_if_changed(&response);
    response
}

#[tauri::command]
pub fn get_capture_permissions(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, NativeCaptureState>,
) -> CapturePermissionsResponse {
    #[cfg(target_os = "macos")]
    recover_screen_capture_after_possible_missed_wake(app_handle);

    let runtime = state.lock().expect("native capture state poisoned");
    let permissions = CapturePermissions {
        screen: capture_screen::screen_permission_state(),
        microphone: microphone_capture::microphone_permission_state(),
        system_audio: capture_screen::system_audio_permission_state(),
    };

    log_capture_permissions_if_changed(&permissions);

    CapturePermissionsResponse {
        permissions,
        session: runtime.session(),
    }
}

/// Trigger the native macOS permission prompt for a capture source, then return
/// the refreshed permission state. `kind` is "screen", "microphone", or
/// "systemAudio" (system audio shares the screen-recording permission). The
/// underlying request blocks (microphone waits on a system callback), so it
/// runs on a blocking thread to keep the UI responsive.
#[tauri::command]
pub async fn request_capture_permission(
    kind: String,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, NativeCaptureState>,
) -> Result<CapturePermissionsResponse, String> {
    match kind.as_str() {
        "screen" | "systemAudio" => {
            tauri::async_runtime::spawn_blocking(|| {
                capture_screen::ensure_screen_permission();
            })
            .await
            .map_err(|error| format!("screen permission request failed: {error}"))?;
        }
        "microphone" => {
            tauri::async_runtime::spawn_blocking(|| {
                microphone_capture::ensure_microphone_permission();
            })
            .await
            .map_err(|error| format!("microphone permission request failed: {error}"))?;
        }
        other => return Err(format!("unknown permission kind: {other}")),
    }

    Ok(get_capture_permissions(app_handle, state))
}

/// Open the macOS Privacy & Security pane for a capture source so the user can
/// flip a permission that was already denied (macOS will not re-prompt once
/// denied).
#[tauri::command]
pub fn open_capture_privacy_settings(
    kind: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    let url = match kind.as_str() {
        "screen" | "systemAudio" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        "microphone" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
        other => return Err(format!("unknown permission kind: {other}")),
    };

    app_handle
        .opener()
        .open_url(url, None::<String>)
        .map_err(|error| format!("failed to open privacy settings: {error}"))
}

/// One Gecko browser Mnema knows about (reads its active-tab URL via the macOS
/// Accessibility API) and whether it is installed on this machine.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeckoBrowserInstall {
    pub bundle_id: String,
    pub display_name: String,
    pub installed: bool,
}

/// Whether Mnema holds the macOS Accessibility permission plus the Gecko
/// browsers whose active-tab URL capture depends on it. Drives the onboarding
/// and settings surfaces that gate Gecko URL capture behind the permission.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserUrlAccessibilityStatus {
    /// Whether Mnema currently holds the macOS Accessibility permission.
    pub trusted: bool,
    /// The Gecko browsers Mnema knows about and whether each is installed.
    pub gecko_browsers: Vec<GeckoBrowserInstall>,
}

/// Builds the current Accessibility-permission + installed-Gecko status. The
/// Gecko list is exactly the `KNOWN_BROWSER_APPS` whose URL strategy is the
/// Accessibility API; `trusted`/`installed` are environment-dependent (always
/// `false` off macOS).
fn browser_url_accessibility_status() -> BrowserUrlAccessibilityStatus {
    use capture_metadata::BrowserUrlStrategy;

    #[cfg(target_os = "macos")]
    let trusted = browser_url_ax::accessibility_trusted();
    #[cfg(not(target_os = "macos"))]
    let trusted = false;

    let gecko_browsers = capture_metadata::KNOWN_BROWSER_APPS
        .iter()
        .filter(|app| matches!(app.url_strategy, Some(BrowserUrlStrategy::Accessibility)))
        .map(|app| {
            #[cfg(target_os = "macos")]
            let installed = macos_application_bundle_path_for_bundle_id(app.bundle_id).is_some();
            #[cfg(not(target_os = "macos"))]
            let installed = false;
            GeckoBrowserInstall {
                bundle_id: app.bundle_id.to_string(),
                display_name: app.display_name.to_string(),
                installed,
            }
        })
        .collect();

    BrowserUrlAccessibilityStatus {
        trusted,
        gecko_browsers,
    }
}

/// Current macOS Accessibility-permission state plus the known Gecko browsers
/// and whether each is installed. Gecko active-tab URL capture is gated on this
/// permission.
#[tauri::command]
pub fn get_browser_url_accessibility_status() -> BrowserUrlAccessibilityStatus {
    browser_url_accessibility_status()
}

/// Raise the macOS Accessibility permission prompt (adds Mnema to the
/// Accessibility list and points the user at System Settings), then return the
/// refreshed status. The grant itself is asynchronous — the user flips the
/// toggle in System Settings, so `trusted` may still be `false` right after.
#[tauri::command]
pub fn request_browser_url_accessibility() -> BrowserUrlAccessibilityStatus {
    #[cfg(target_os = "macos")]
    {
        let _ = browser_url_ax::request_accessibility_with_prompt();
    }
    browser_url_accessibility_status()
}

/// Open the macOS Privacy & Security → Accessibility pane so the user can grant
/// (or re-grant) Mnema the Accessibility permission.
#[tauri::command]
pub fn open_browser_url_accessibility_settings(app_handle: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    app_handle
        .opener()
        .open_url(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            None::<String>,
        )
        .map_err(|error| format!("failed to open accessibility settings: {error}"))
}

#[tauri::command]
pub fn get_idle_debug(state: tauri::State<'_, NativeCaptureState>) -> IdleDebugInfo {
    activity::get_idle_debug(state)
}

#[tauri::command]
pub fn get_app_notifications(
    state: tauri::State<'_, AppNotificationsState>,
) -> Vec<AppNotification> {
    state
        .lock()
        .expect("app notifications state poisoned")
        .list()
}

#[tauri::command]
pub fn clear_app_notification(
    id: String,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppNotificationsState>,
) -> Vec<AppNotification> {
    let notifications = {
        let mut runtime = state.lock().expect("app notifications state poisoned");
        runtime.clear_one(&id)
    };
    emit_app_notifications_changed(&app_handle, &notifications);
    notifications
}

#[tauri::command]
pub fn clear_app_notifications(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppNotificationsState>,
) -> Vec<AppNotification> {
    let notifications = {
        let mut runtime = state.lock().expect("app notifications state poisoned");
        runtime.clear_all()
    };
    emit_app_notifications_changed(&app_handle, &notifications);
    notifications
}

#[tauri::command]
pub fn get_microphone_controller_state(
    state: tauri::State<'_, MicrophoneControllerPreferencesState>,
) -> Result<MicrophoneControllerState, CaptureErrorResponse> {
    let runtime = state
        .lock()
        .expect("microphone controller preferences state poisoned");
    microphone_capture::microphone_controller_state(
        runtime.preference.clone(),
        runtime.disconnect_policy.clone(),
    )
}

#[tauri::command]
pub fn update_microphone_controller(
    request: UpdateMicrophoneControllerRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, MicrophoneControllerPreferencesState>,
) -> Result<MicrophoneControllerState, CaptureErrorResponse> {
    update_microphone_controller_impl(request, &app_handle, state)
}

pub fn initialize_recording_settings_from_disk(app_handle: &tauri::AppHandle) {
    reset_capture_log_snapshots();
    let settings_state = app_handle.state::<RecordingSettingsState>();
    let loaded = initialize_recording_settings_state_from_disk(app_handle, settings_state.inner());

    debug_log::set_app_debug_logging_enabled(loaded.settings.developer_options_enabled);
    debug_log::configure(
        app_handle,
        loaded.settings.native_capture_debug_logging_enabled,
    );
    log_loaded_recording_settings(loaded.source, &loaded.settings);
}

pub fn maybe_auto_start_native_capture(app_handle: &tauri::AppHandle) {
    let settings_state = app_handle.state::<RecordingSettingsState>();
    let auto_start_enabled = current_auto_start(settings_state.inner());

    if !auto_start_enabled {
        return;
    }

    let _ = start_native_capture_from_app_handle("auto-start", app_handle);
}

pub(crate) fn current_native_capture_session(
    app_handle: &tauri::AppHandle,
) -> capture_types::NativeCaptureSession {
    let state = app_handle.state::<NativeCaptureState>();
    let runtime = state.lock().expect("native capture state poisoned");
    runtime.session()
}

pub(crate) fn current_recording_settings_from_app_handle(
    app_handle: &tauri::AppHandle,
) -> RecordingSettings {
    let state = app_handle.state::<RecordingSettingsState>();
    current_recording_settings(state.inner())
}

fn finish_recording_settings_update(
    app_handle: &tauri::AppHandle,
    update: settings::AppliedRecordingSettingsUpdate,
    domain: Option<SettingsOwnershipDomain>,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    let settings = update.settings;
    let previous_settings = update.previous_settings;
    let previous_save_directory = update.previous_save_directory;
    let save_directory_changed = update.save_directory_changed;
    let debug_logging_enabled_changed = update.debug_logging_enabled_changed;

    if previous_settings.native_capture_debug_logging_enabled
        && !settings.native_capture_debug_logging_enabled
    {
        log_recording_settings_changes(&previous_settings, &settings);

        if save_directory_changed {
            debug_log::log_info(format!(
                "recording save directory changed from '{}' to '{}'; app infrastructure database location will update on next app start",
                previous_save_directory, settings.save_directory
            ));
        }
    }

    debug_log::set_app_debug_logging_enabled(settings.developer_options_enabled);
    debug_log::configure(app_handle, settings.native_capture_debug_logging_enabled);

    if !previous_settings.native_capture_debug_logging_enabled
        && settings.native_capture_debug_logging_enabled
    {
        reset_capture_log_snapshots();
    }

    if settings.native_capture_debug_logging_enabled {
        if debug_logging_enabled_changed {
            debug_log::log_info(format!(
                "native capture debug logging {}",
                if previous_settings.native_capture_debug_logging_enabled {
                    "re-enabled"
                } else {
                    "enabled"
                }
            ));
        }

        log_recording_settings_changes(&previous_settings, &settings);
    }

    if save_directory_changed && settings.native_capture_debug_logging_enabled {
        debug_log::log_info(format!(
            "recording save directory changed from '{}' to '{}'; app infrastructure database location will update on next app start",
            previous_save_directory, settings.save_directory
        ));
    }

    if previous_settings.ocr.enabled && !settings.ocr.enabled {
        if let Some(infra) = app_handle.try_state::<crate::app_infra::AppInfraState>() {
            // Fire-and-forget off the main thread: this only marks already-queued
            // OCR jobs failed now that OCR is disabled (cleanup whose result is
            // merely logged). Settings commands run on the main thread, so we must
            // not block the UI awaiting this capture-index write.
            let infra = std::sync::Arc::clone(&*infra);
            tauri::async_runtime::spawn(async move {
                match infra.fail_queued_ocr_jobs_because_disabled().await {
                    Ok(failed_count) => debug_log::log_info(format!(
                        "marked queued OCR jobs failed because OCR was disabled (count={failed_count})"
                    )),
                    Err(error) => debug_log::log_error(format!(
                        "failed to mark queued OCR jobs failed after disabling OCR: {error}"
                    )),
                }
            });
        } else {
            debug_log::log_warn(
                "app infrastructure state unavailable while disabling OCR; queued OCR jobs were not updated",
            );
        }
    }

    if previous_settings.retention_policy != settings.retention_policy {
        if let Some(background_workers) =
            app_handle.try_state::<crate::app_infra::BackgroundWorkersState>()
        {
            background_workers.notify_retention_schedule_changed();
        } else {
            debug_log::log_warn(
                "background workers state unavailable while updating retention policy; retention cleanup schedule was not woken",
            );
        }
    }

    emit_recording_settings_changed(app_handle, &settings);
    if let Some(domain) = domain {
        emit_recording_settings_domain_changed(
            app_handle,
            &RecordingSettingsDomainUpdateResponse {
                domain,
                settings: settings.clone(),
            },
        );
    }
    let privacy_changed = previous_settings.privacy != settings.privacy;
    let metadata_changed = previous_settings.metadata != settings.metadata;
    if metadata_changed {
        privacy::request_privacy_filter_refresh(
            app_handle,
            privacy::PrivacyRefreshReason::MetadataSettingsMutation,
        );
    } else if privacy_changed {
        privacy::request_privacy_filter_refresh(
            app_handle,
            privacy::PrivacyRefreshReason::StaticAppRuleMutation,
        );
    }
    crate::status_bar::refresh(app_handle);

    Ok(settings)
}

fn finish_recording_settings_domain_update(
    app_handle: &tauri::AppHandle,
    domain: SettingsOwnershipDomain,
    update: settings::AppliedRecordingSettingsUpdate,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    let settings = finish_recording_settings_update(app_handle, update, Some(domain))?;
    Ok(RecordingSettingsDomainUpdateResponse { domain, settings })
}

pub(crate) fn apply_recording_settings_domain_mutation_from_app_handle(
    app_handle: &tauri::AppHandle,
    domain: SettingsOwnershipDomain,
    mutate: impl FnOnce(&mut RecordingSettings) -> Result<(), CaptureErrorResponse>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    let state = app_handle.state::<RecordingSettingsState>();
    let update =
        apply_recording_settings_domain_mutation(app_handle, state.inner(), domain, mutate)?;
    finish_recording_settings_domain_update(app_handle, domain, update)
}

pub(crate) fn update_recording_sources_from_app_handle(
    app_handle: &tauri::AppHandle,
    sources: CaptureSources,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    let response = apply_recording_settings_domain_mutation_from_app_handle(
        app_handle,
        SettingsOwnershipDomain::CaptureSources,
        |settings| {
            settings.capture_screen = sources.screen;
            settings.capture_microphone = sources.microphone;
            settings.capture_system_audio = sources.system_audio;
            Ok(())
        },
    )?;
    Ok(response.settings)
}

pub(crate) fn start_native_capture_from_app_handle(
    origin: &str,
    app_handle: &tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let response = start_native_capture_inner(
        origin,
        StartNativeCaptureRequest {
            capture_screen: false,
            capture_microphone: false,
            capture_system_audio: false,
        },
        app_handle.state::<NativeCaptureState>(),
        app_handle.state::<MicrophoneControllerPreferencesState>(),
        app_handle.state::<RecordingSettingsState>(),
        app_handle.state::<AppNotificationsState>(),
        app_handle.clone(),
    )?;
    emit_native_capture_session_changed(app_handle, &response.session);
    crate::status_bar::refresh(app_handle);
    Ok(response)
}

pub(crate) fn stop_native_capture_from_app_handle(
    app_handle: &tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let response =
        stop_native_capture_with_state(app_handle.state::<NativeCaptureState>(), app_handle)?;
    emit_native_capture_session_changed(app_handle, &response.session);
    crate::status_bar::refresh(app_handle);
    Ok(response)
}

pub(crate) fn pause_native_capture_from_app_handle(
    app_handle: &tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let state = app_handle.state::<NativeCaptureState>();
    let mut runtime = state.lock().expect("native capture state poisoned");
    let session = runtime.pause_user_capture(app_handle)?;
    drop(runtime);
    emit_native_capture_session_changed(app_handle, &session);
    crate::status_bar::refresh(app_handle);
    Ok(NativeCaptureSessionResponse { session })
}

pub(crate) fn resume_native_capture_from_app_handle(
    app_handle: &tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let state = app_handle.state::<NativeCaptureState>();
    let mut runtime = state.lock().expect("native capture state poisoned");
    let session = runtime.resume_user_capture(app_handle)?;
    drop(runtime);
    emit_native_capture_session_changed(app_handle, &session);
    crate::status_bar::refresh(app_handle);
    Ok(NativeCaptureSessionResponse { session })
}

#[tauri::command]
pub fn get_recording_settings(
    state: tauri::State<'_, RecordingSettingsState>,
) -> RecordingSettings {
    current_recording_settings(state.inner())
}

#[tauri::command]
pub fn get_native_capture_debug_log_status(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> NativeCaptureDebugLogStatus {
    let enabled = current_native_capture_debug_logging_enabled(state.inner());

    debug_log::status(&app_handle, enabled)
}

#[tauri::command]
pub fn open_native_capture_debug_log(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<NativeCaptureDebugLogStatus, CaptureErrorResponse> {
    let enabled = current_native_capture_debug_logging_enabled(state.inner());

    debug_log::open(&app_handle, enabled)
}

#[tauri::command]
pub fn delete_native_capture_debug_log(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<NativeCaptureDebugLogStatus, CaptureErrorResponse> {
    let enabled = current_native_capture_debug_logging_enabled(state.inner());

    debug_log::delete(&app_handle, enabled)
}

#[tauri::command]
pub fn update_recording_settings(
    request: UpdateRecordingSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    let update = apply_recording_settings_update(&app_handle, state.inner(), request)?;
    finish_recording_settings_update(&app_handle, update, None)
}

fn update_recording_settings_domain(
    app_handle: &tauri::AppHandle,
    state: &RecordingSettingsState,
    patch: RecordingSettingsDomainPatch,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    let (domain, update) = apply_recording_settings_domain_patch(app_handle, state, patch)?;
    finish_recording_settings_domain_update(app_handle, domain, update)
}

#[tauri::command]
pub fn update_capture_source_settings(
    request: UpdateCaptureSourceSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        &app_handle,
        state.inner(),
        RecordingSettingsDomainPatch::CaptureSources(request),
    )
}

#[tauri::command]
pub fn update_capture_timing_settings(
    request: UpdateCaptureTimingSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        &app_handle,
        state.inner(),
        RecordingSettingsDomainPatch::CaptureTiming(request),
    )
}

#[tauri::command]
pub fn update_video_settings(
    request: UpdateVideoSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        &app_handle,
        state.inner(),
        RecordingSettingsDomainPatch::Video(request),
    )
}

#[tauri::command]
pub fn update_storage_settings(
    request: UpdateStorageSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        &app_handle,
        state.inner(),
        RecordingSettingsDomainPatch::Storage(request),
    )
}

#[tauri::command]
pub fn update_display_settings(
    request: UpdateDisplaySettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        &app_handle,
        state.inner(),
        RecordingSettingsDomainPatch::Display(request),
    )
}

#[tauri::command]
pub fn update_metadata_settings(
    request: UpdateMetadataSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        &app_handle,
        state.inner(),
        RecordingSettingsDomainPatch::Metadata(request),
    )
}

#[tauri::command]
pub fn update_inactivity_settings(
    request: UpdateInactivitySettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        &app_handle,
        state.inner(),
        RecordingSettingsDomainPatch::Inactivity(request),
    )
}

#[tauri::command]
pub fn update_processing_settings(
    request: UpdateProcessingSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        &app_handle,
        state.inner(),
        RecordingSettingsDomainPatch::Processing(request),
    )
}

#[tauri::command]
pub fn update_developer_settings(
    request: UpdateDeveloperSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        &app_handle,
        state.inner(),
        RecordingSettingsDomainPatch::Developer(request),
    )
}

#[tauri::command]
pub fn update_access_settings(
    request: UpdateAccessSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        &app_handle,
        state.inner(),
        RecordingSettingsDomainPatch::Access(request),
    )
}

#[tauri::command]
pub fn update_ai_runtime_settings(
    request: UpdateAiRuntimeSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    let response = update_recording_settings_domain(
        &app_handle,
        state.inner(),
        RecordingSettingsDomainPatch::AiRuntime(request),
    )?;
    // MCP teardown (ADR 0048): reconcile the connection cache against the
    // just-saved settings so removed/disabled connectors' handles — and their
    // child processes — are dropped. Fire-and-forget: teardown never blocks the
    // save, and an EDITED connector is reaped lazily on next use.
    if let Some(manager) = app_handle.try_state::<crate::ask_ai::mcp::McpManager>() {
        let manager = (*manager).clone();
        let app_handle = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            manager.reconcile(&app_handle).await;
        });
    }
    Ok(response)
}

#[tauri::command]
pub fn update_user_context_settings(
    request: UpdateUserContextSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        &app_handle,
        state.inner(),
        RecordingSettingsDomainPatch::UserContext(request),
    )
}

/// Update the **Semantic Search** settings that do NOT change the active vector
/// dimension: the enabled toggle (issue #125). A model *switch* goes through the
/// atomic `select_semantic_search_model` command instead — it rebuilds the `vec0`
/// table at the new model's dimension and persists the selection together, so the
/// persisted model and the live table dimension can never disagree. The
/// **Semantic Index Backfill** worker reloads the embedder on its next pass when
/// the provider/model id changes.
#[tauri::command]
pub fn update_semantic_search_settings(
    request: capture_types::UpdateSemanticSearchSettingsRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        &app_handle,
        state.inner(),
        RecordingSettingsDomainPatch::SemanticSearch(request),
    )
}

/// Persist a **Semantic Search Model Tier** patch from outside the command layer
/// (the atomic `select_semantic_search_model` switch, which rebuilds the `vec0`
/// table *before* persisting so the persisted `model_id` and the live table
/// dimension never disagree). Reuses the same domain-update path as
/// [`update_semantic_search_settings`].
pub(crate) fn persist_semantic_search_settings(
    app_handle: &tauri::AppHandle,
    state: &RecordingSettingsState,
    request: capture_types::UpdateSemanticSearchSettingsRequest,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    // Trusted path: the atomic switch has already rebuilt the `vec0` table to the
    // new model's dimension before this persist, so use the trusted variant that
    // honors `model_id`/`provider`. The generic IPC command
    // (`update_semantic_search_settings`) uses `SemanticSearch`, which ignores
    // those dimension-affecting fields. See review finding low #4 (PR #126).
    update_recording_settings_domain(
        app_handle,
        state,
        RecordingSettingsDomainPatch::SemanticSearchModelSwitch(request),
    )
}

#[tauri::command]
pub fn start_native_capture(
    request: StartNativeCaptureRequest,
    state: tauri::State<'_, NativeCaptureState>,
    microphone_controller_preferences_state: tauri::State<'_, MicrophoneControllerPreferencesState>,
    recording_settings_state: tauri::State<'_, RecordingSettingsState>,
    app_notifications_state: tauri::State<'_, AppNotificationsState>,
    app_handle: tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let response = start_native_capture_inner(
        "command",
        request,
        state,
        microphone_controller_preferences_state,
        recording_settings_state,
        app_notifications_state,
        app_handle.clone(),
    )?;
    emit_native_capture_session_changed(&app_handle, &response.session);
    crate::status_bar::refresh(&app_handle);
    Ok(response)
}

#[tauri::command]
pub fn pause_native_capture(
    app_handle: tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    pause_native_capture_from_app_handle(&app_handle)
}

#[tauri::command]
pub fn resume_native_capture(
    app_handle: tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    resume_native_capture_from_app_handle(&app_handle)
}

fn stop_native_capture_with_state(
    state: tauri::State<'_, NativeCaptureState>,
    app_handle: &tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let mut runtime = state.lock().expect("native capture state poisoned");
    let session_id = runtime_log_session_id(runtime.runtime()).to_string();
    let requested_sources = runtime.runtime().requested_sources.clone();
    let output_files_before_stop = runtime.runtime().output_files.clone();
    let source_session_ids_before_stop = runtime
        .runtime()
        .source_sessions
        .clone()
        .map(|source_sessions| {
            [
                source_sessions.screen,
                source_sessions.microphone,
                source_sessions.system_audio,
            ]
            .into_iter()
            .flatten()
            .map(|source_session| source_session.session_id)
            .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    debug_log::log_info(format!(
        "received native capture stop request (is_running={}, session_id='{}', requested_sources={}, output_files_before_stop={})",
        runtime.runtime().is_running,
        session_id,
        format_optional_capture_source_flags(requested_sources.as_ref()),
        format_output_file_counts(output_files_before_stop.as_ref())
    ));

    let session = match runtime.stop(app_handle) {
        Ok(session) => session,
        Err(error) => {
            if capture_screen::should_preserve_runtime_on_stop_error(&error) {
                debug_log::log_error(format!(
                    "failed to stop native capture but preserved runtime for recovery (session_id='{}'): [{}] {}",
                    session_id,
                    error.code,
                    error.message
                ));
            } else {
                debug_log::log_error(format!(
                    "failed to stop native capture; runtime marked stopped (session_id='{}'): [{}] {}",
                    session_id, error.code, error.message
                ));
            }

            return Err(error);
        }
    };
    if let Some(metadata_state) = app_handle.try_state::<CaptureMetadataState>() {
        metadata::reset_recording_session_privacy_state(metadata_state.inner());
    }

    debug_log::log_info(format!(
        "stopped native capture successfully (session_id='{}', requested_sources={}, finalized_outputs={})",
        session_log_session_id(&session),
        format_optional_capture_source_flags(session.requested_sources.as_ref()),
        format_output_file_counts(session.output_files.as_ref())
    ));

    if !source_session_ids_before_stop.is_empty() {
        if let Some(infra) = app_handle.try_state::<crate::app_infra::AppInfraState>() {
            let stopped_at = time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());
            let source_session_ids = source_session_ids_before_stop;
            crate::ocr_budget::clear_sessions_for_base_dir(infra.base_dir(), &source_session_ids);
            let infra = std::sync::Arc::clone(&*infra);
            if let Err(error) = tauri::async_runtime::block_on(async move {
                infra
                    .capture_retention()
                    .complete_capture_sessions_for_source_session_ids(
                        &source_session_ids,
                        &stopped_at,
                        "completed",
                    )
                    .await
            }) {
                debug_log::log_error(format!(
                    "failed to mark capture session completed after stop: {error}"
                ));
            }
        }
    }

    Ok(NativeCaptureSessionResponse { session })
}

#[tauri::command]
pub async fn stop_native_capture(
    app_handle: tauri::AppHandle,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    // Stopping does a synchronous capture-index write (marking sessions
    // completed). This command runs on the main thread, so run the stop on the
    // blocking pool and await it — keeping the UI responsive while preserving
    // ordering (the completion write finishes before we emit the change event).
    // The tray/relaunch/shortcut entry points keep calling the sync helper
    // directly so they still complete in-line.
    let handle = app_handle.clone();
    let response = tauri::async_runtime::spawn_blocking(move || {
        stop_native_capture_with_state(handle.state::<NativeCaptureState>(), &handle)
    })
    .await
    .map_err(|error| CaptureErrorResponse {
        code: "stop_native_capture_join".to_string(),
        message: format!("stop native capture task failed: {error}"),
    })??;
    emit_native_capture_session_changed(&app_handle, &response.session);
    crate::status_bar::refresh(&app_handle);
    Ok(response)
}
