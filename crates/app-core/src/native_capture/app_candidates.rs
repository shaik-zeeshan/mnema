//! The app list behind the privacy-exclusion picker and the app icons the UI
//! shows beside app names (bundle-id or display-name lookups, cached as PNGs).

use std::collections::{BTreeMap, BTreeSet};
#[cfg(target_os = "macos")]
use std::path::Path;
use std::path::PathBuf;

use crate::privacy_redaction_sources::PrivacyAppCandidate;
use crate::AppCore;
#[cfg(target_os = "macos")]
use crate::WindowSignal;

#[cfg(target_os = "macos")]
const APP_ICON_CACHE_DIR: &str = "app-icons";
// Point size we render cached app icons at. Displayed at 20–24 CSS px, so a
// larger source keeps the icon crisp on Retina (2x → ~48 device px) by always
// downscaling instead of upscaling a small bitmap. Baked into the cache
// filename so bumping this size supersedes previously cached lower-res PNGs.
#[cfg(target_os = "macos")]
const APP_ICON_RENDER_POINT_SIZE: u32 = 128;

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

pub async fn list_privacy_app_candidates() -> Result<Vec<PrivacyAppCandidate>, String> {
    let mut candidates = BTreeMap::new();
    insert_privacy_app_candidate(
        &mut candidates,
        PrivacyAppCandidate {
            bundle_id: "day.mnema".to_string(),
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

pub async fn resolve_app_icons(
    core: &AppCore,
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
                    *path = catalog
                        .get(&canonical_app_display_name(identifier))
                        .cloned();
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
        materialize_app_candidate_icons(core, &mut candidates);

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
        let _ = core;
        Ok(requested_bundle_ids
            .into_iter()
            .map(|bundle_id| AppIconResolution {
                bundle_id,
                icon_path: None,
            })
            .collect())
    }
}

pub(crate) fn insert_privacy_app_candidate(
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

pub(crate) fn merge_running_privacy_app_candidates(
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
    macos_application_bundle_path_for_bundle_id(bundle_id)
        .or_else(|| (bundle_id == "day.mnema").then(main_bundle_path).flatten())
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
pub(crate) fn macos_application_bundle_path_for_bundle_id(bundle_id: &str) -> Option<PathBuf> {
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
fn ensure_app_icon_cache_dir(core: &AppCore) -> Result<PathBuf, String> {
    let cache_dir = core.paths().cache.join(APP_ICON_CACHE_DIR);
    std::fs::create_dir_all(&cache_dir).map_err(|error| {
        format!(
            "failed to create app icon cache directory {}: {error}",
            cache_dir.display()
        )
    })?;
    core.host().window_signal(WindowSignal::AllowAssetPath {
        path: cache_dir.clone(),
        recursive: true,
    });
    Ok(cache_dir)
}

#[cfg(target_os = "macos")]
fn materialize_app_candidate_icons(
    core: &AppCore,
    candidates: &mut BTreeMap<String, PrivacyAppCandidate>,
) {
    let Ok(cache_dir) = ensure_app_icon_cache_dir(core) else {
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
