use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use url::Url;

/// Backstop re-probe interval for a browser whose front-window title has *not*
/// changed. The primary freshness signal is the window title (re-probe on every
/// change — see [`BrowserUrlProbeCache::cached_url_for`]); this only bounds
/// staleness for navigations that change the URL without changing the title
/// (e.g. some single-page apps).
pub const BROWSER_URL_PROBE_BACKSTOP_INTERVAL: Duration = Duration::from_secs(5);

/// Minimum spacing between re-probes triggered by a *title change* for the same
/// front window. Title-gating ([`BrowserUrlProbeCache::cached_url_for`]) re-probes
/// on every title change, but pages with dynamic titles (unread counters like
/// "(5) Inbox", live timers, "● Recording", per-second clocks) change their title
/// on nearly every ~1s metadata tick — without a floor that means an `osascript`
/// spawn (Chromium/WebKit) or a blocking AX read (Gecko) every tick. This floor
/// caps title-driven re-probes to at most once per interval, trading a brief
/// (<~1.5s) URL/title desync for not hammering the probe. Kept below
/// [`BROWSER_URL_PROBE_BACKSTOP_INTERVAL`] so the same-title backstop still wins
/// past it.
pub const BROWSER_URL_PROBE_REPROBE_FLOOR: Duration = Duration::from_millis(1500);

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
    window_title: Option<String>,
    raw_url: Option<String>,
    probed_at: Option<Instant>,
}

impl BrowserUrlProbeCache {
    /// Returns the cached URL when it is still trustworthy for `bundle_id`.
    ///
    /// The front-window title is captured fresh every tick and almost always
    /// changes on a tab switch or navigation, so a title mismatch forces an
    /// immediate re-probe — this is what keeps the cached URL from going stale
    /// against the title (the desync that surfaced an old tab's URL under a new
    /// page's title). [`BROWSER_URL_PROBE_BACKSTOP_INTERVAL`] is only a backstop
    /// for URL changes that leave the title untouched.
    ///
    /// A title change forces a re-probe — but no more often than
    /// [`BROWSER_URL_PROBE_REPROBE_FLOOR`], so a dynamic-title page (live counter,
    /// timer, clock) that mutates its title every tick can't hammer the probe.
    /// Within the floor a changed title serves the cached URL (brief desync);
    /// past the floor it re-probes.
    pub fn cached_url_for(
        &self,
        bundle_id: &str,
        window_title: Option<&str>,
        now: Instant,
    ) -> Option<Option<String>> {
        if self.bundle_id.as_deref() != Some(bundle_id) {
            return None;
        }
        let probed_at = self.probed_at?;
        let elapsed = now.saturating_duration_since(probed_at);
        if self.window_title.as_deref() != window_title {
            // Title moved on: re-probe, but not more than once per floor so a
            // dynamic title (changing every tick) can't trigger a probe storm.
            if elapsed < BROWSER_URL_PROBE_REPROBE_FLOOR {
                return Some(self.raw_url.clone());
            }
            return None;
        }
        if elapsed >= BROWSER_URL_PROBE_BACKSTOP_INTERVAL {
            return None;
        }
        Some(self.raw_url.clone())
    }

    pub fn from_probe(
        bundle_id: Option<String>,
        window_title: Option<String>,
        raw_url: Option<String>,
        now: Instant,
    ) -> Self {
        Self {
            bundle_id,
            window_title,
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

/// Derive a human-readable app display name from a Windows executable path, used
/// as the fallback when the executable exposes no version-info `FileDescription`.
/// Strips the directory and a trailing `.exe` (case-insensitively), e.g.
/// `C:\Program Files\Google\Chrome\Application\chrome.exe` -> `chrome`.
///
/// The Windows metadata collector guarantees `app_name` is always populated when
/// a snapshot is produced — a raw path must never surface as a UI label (the
/// timeline only falls back to the raw identity when `app_name` is null). So this
/// never returns the full path: it returns the file stem, or the trimmed input
/// when there is no stem to take (e.g. an empty string). Kept pure (no Win32) so
/// it is unit-testable in this dependency-light crate.
pub fn app_display_name_from_exe_path(exe_path: &str) -> String {
    let trimmed = exe_path.trim();
    // Windows paths use `\`; tolerate `/` too. `rsplit` on both separators yields
    // the final path component (or the whole string when there is no separator).
    let file_name = trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed);
    // Drop a trailing `.exe` case-insensitively. `rsplit_once('.')` is UTF-8-safe
    // (splits on a char boundary), unlike byte-slicing the tail.
    let stem = match file_name.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("exe") => stem,
        _ => file_name,
    }
    .trim();
    if stem.is_empty() {
        trimmed.to_string()
    } else {
        stem.to_string()
    }
}

/// The browser engine family Mnema recognizes for a Windows executable, used to
/// pick the UI Automation read dialect (ADR 0044). This is a Windows-only parallel
/// type to the macOS `BrowserUrlStrategy`; the platforms share no dispatch site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserEngine {
    /// Chromium family (Chrome, Edge, Brave, Vivaldi, Opera, Arc, Helium-as-chrome.exe, …).
    Chromium,
    /// Gecko family (Firefox, Zen, LibreWolf, Waterfox, Floorp).
    Gecko,
}

/// Resolve a Windows executable file stem to its browser engine family, or `None`
/// when the stem is not a browser we recognize.
///
/// `stem` is the exe file stem — the final path component with a trailing `.exe`
/// dropped, exactly what [`app_display_name_from_exe_path`] returns (slice 3 wires
/// the two: extract the stem, then resolve it here). Matching is case-insensitive
/// on the trimmed, ASCII-lowercased stem.
///
/// Recognition is engine-granular and brand-less by design (ADR 0044): we key on
/// the engine family, not the product, so a Chromium fork shipped under a stock
/// stem is one Chromium hit regardless of brand (e.g. Helium ships as `chrome.exe`
/// and resolves to `Chromium`). An unlisted fork with its own stem silently
/// resolves to `None` until that stem is added — the ADR 0044 consequence. Kept a
/// plain `match` on the lowercased stem so it stays Win32-free and trivially
/// unit-testable in this dependency-light crate.
pub fn known_browser_engine_for_exe_stem(stem: &str) -> Option<BrowserEngine> {
    match stem.trim().to_ascii_lowercase().as_str() {
        "chrome" | "msedge" | "brave" | "vivaldi" | "opera" | "opera_gx" | "chromium" | "arc" => {
            Some(BrowserEngine::Chromium)
        }
        "firefox" | "zen" | "librewolf" | "waterfox" | "floorp" => Some(BrowserEngine::Gecko),
        _ => None,
    }
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
pub enum BrowserUrlDialect {
    Chromium, // URL of active tab of front window
    WebKit,   // URL of current tab of front window  (Safari family)
}

/// How Mnema reads a browser's active-tab URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserUrlStrategy {
    /// Chromium/WebKit families — read via AppleScript (`osascript`). No extra permission.
    AppleScript(BrowserUrlDialect),
    /// Gecko family (Firefox/Zen) — read `AXURL` off the focused web area via the
    /// macOS Accessibility API. Requires the Accessibility permission, opt-in.
    Accessibility,
}

/// A recognized browser and how (if at all) Mnema reads its active-tab URL.
///
/// The Chromium/WebKit families expose the URL via AppleScript, so they carry an
/// `url_script_app_name` (the "tell application" target) and an
/// `AppleScript(dialect)` strategy. The Gecko family (Firefox/Zen) has no
/// scriptable URL surface; it reads the URL via the Accessibility API and so has
/// `url_script_app_name: None` with the `Accessibility` strategy. `url_strategy`
/// is `None` only for a browser that is recognized but has no URL surface at all.
///
/// Invariant: `url_script_app_name.is_some()` iff `url_strategy` is
/// `Some(AppleScript(_))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserAppDescriptor {
    pub bundle_id: &'static str,
    pub display_name: &'static str,
    pub url_script_app_name: Option<&'static str>,
    pub url_strategy: Option<BrowserUrlStrategy>,
}

pub const KNOWN_BROWSER_APPS: &[BrowserAppDescriptor] = &[
    BrowserAppDescriptor {
        bundle_id: "com.apple.Safari",
        display_name: "Safari",
        url_script_app_name: Some("Safari"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::WebKit)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.apple.SafariTechnologyPreview",
        display_name: "Safari Technology Preview",
        url_script_app_name: Some("Safari Technology Preview"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::WebKit)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.kagi.kagimacOS",
        display_name: "Orion",
        url_script_app_name: Some("Orion"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::WebKit)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.google.Chrome",
        display_name: "Google Chrome",
        url_script_app_name: Some("Google Chrome"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.google.Chrome.canary",
        display_name: "Google Chrome Canary",
        url_script_app_name: Some("Google Chrome Canary"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.google.Chrome.beta",
        display_name: "Google Chrome Beta",
        url_script_app_name: Some("Google Chrome Beta"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.google.Chrome.dev",
        display_name: "Google Chrome Dev",
        url_script_app_name: Some("Google Chrome Dev"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "org.chromium.Chromium",
        display_name: "Chromium",
        url_script_app_name: Some("Chromium"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.microsoft.edgemac",
        display_name: "Microsoft Edge",
        url_script_app_name: Some("Microsoft Edge"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.microsoft.edgemac.Beta",
        display_name: "Microsoft Edge Beta",
        url_script_app_name: Some("Microsoft Edge Beta"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.microsoft.edgemac.Dev",
        display_name: "Microsoft Edge Dev",
        url_script_app_name: Some("Microsoft Edge Dev"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.microsoft.edgemac.Canary",
        display_name: "Microsoft Edge Canary",
        url_script_app_name: Some("Microsoft Edge Canary"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    // Gecko browsers have no scriptable URL surface; the URL is read via the
    // Accessibility API (permission-gated), so they carry no AppleScript app name.
    BrowserAppDescriptor {
        bundle_id: "org.mozilla.firefox",
        display_name: "Firefox",
        url_script_app_name: None,
        url_strategy: Some(BrowserUrlStrategy::Accessibility),
    },
    BrowserAppDescriptor {
        bundle_id: "app.zen-browser.zen",
        display_name: "Zen",
        url_script_app_name: None,
        url_strategy: Some(BrowserUrlStrategy::Accessibility),
    },
    BrowserAppDescriptor {
        bundle_id: "com.brave.Browser",
        display_name: "Brave Browser",
        url_script_app_name: Some("Brave Browser"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.brave.Browser.beta",
        display_name: "Brave Browser Beta",
        url_script_app_name: Some("Brave Browser Beta"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.brave.Browser.nightly",
        display_name: "Brave Browser Nightly",
        url_script_app_name: Some("Brave Browser Nightly"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "company.thebrowser.Browser",
        display_name: "Arc",
        url_script_app_name: Some("Arc"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "net.imput.helium",
        display_name: "Helium",
        url_script_app_name: Some("Helium"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.vivaldi.Vivaldi",
        display_name: "Vivaldi",
        url_script_app_name: Some("Vivaldi"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.operasoftware.Opera",
        display_name: "Opera",
        url_script_app_name: Some("Opera"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
    },
    BrowserAppDescriptor {
        bundle_id: "com.operasoftware.OperaGX",
        display_name: "Opera GX",
        url_script_app_name: Some("Opera GX"),
        url_strategy: Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium)),
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

/// The resolved strategy for reading this browser's URL, if any.
pub fn browser_url_strategy(bundle_id: &str) -> Option<BrowserUrlStrategy> {
    known_browser_app(bundle_id).and_then(|browser| browser.url_strategy)
}

/// Whether Mnema can read this browser's active-tab URL via AppleScript without
/// an extra permission. The privacy disclosure relies on this. Gecko browsers
/// (Firefox/Zen) return `false` here — their Accessibility path is permission-
/// gated and exposed separately.
pub fn browser_url_metadata_supported(bundle_id: &str) -> bool {
    matches!(
        browser_url_strategy(bundle_id),
        Some(BrowserUrlStrategy::AppleScript(_))
    )
}

pub fn browser_url_applescript(bundle_id: &str) -> Option<String> {
    let descriptor = known_browser_app(bundle_id)?;
    let app = descriptor.url_script_app_name?;
    let dialect = match descriptor.url_strategy? {
        BrowserUrlStrategy::AppleScript(dialect) => dialect,
        // Gecko browsers read the URL via the Accessibility API, not AppleScript.
        BrowserUrlStrategy::Accessibility => return None,
    };
    let target = match dialect {
        BrowserUrlDialect::Chromium => "URL of active tab of front window",
        // `current tab of front window` tracks the visually-frontmost window's
        // selected tab; `front document` is ordered by focus recency and can
        // return a background window's URL. See the Safari URL-accuracy fix.
        BrowserUrlDialect::WebKit => "URL of current tab of front window",
    };
    Some(format!(
        "tell application \"{app}\"\ntry\n  return {target}\non error\n  return \"\"\nend try\nend tell"
    ))
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
    fn app_display_name_strips_directory_and_exe_extension() {
        // The primary case: a fully-qualified Windows exe path collapses to the
        // bare file stem, never `chrome.exe` and never the full path.
        assert_eq!(
            app_display_name_from_exe_path(
                r"C:\Program Files\Google\Chrome\Application\chrome.exe"
            ),
            "chrome"
        );
        assert_eq!(app_display_name_from_exe_path("notepad.exe"), "notepad");
    }

    #[test]
    fn app_display_name_is_case_insensitive_on_exe_and_tolerates_forward_slashes() {
        assert_eq!(
            app_display_name_from_exe_path(r"C:\Apps\Thing.EXE"),
            "Thing"
        );
        assert_eq!(app_display_name_from_exe_path("/opt/app/tool.exe"), "tool");
    }

    #[test]
    fn app_display_name_keeps_names_without_an_exe_extension() {
        // No extension -> the whole final path component is the name.
        assert_eq!(
            app_display_name_from_exe_path(r"C:\Windows\System32\cmd"),
            "cmd"
        );
        // A non-exe extension is left intact (only `.exe` is stripped).
        assert_eq!(app_display_name_from_exe_path(r"C:\bin\my.tool"), "my.tool");
    }

    #[test]
    fn app_display_name_handles_empty_or_whitespace_input() {
        // An empty / failed path yields an empty string. The collector never
        // produces a snapshot in that case (it returns `None`), so this only
        // documents the pure fallback's honest behavior.
        assert_eq!(app_display_name_from_exe_path(""), "");
        assert_eq!(app_display_name_from_exe_path("   "), "");
    }

    #[test]
    fn browser_engine_resolves_chromium_and_gecko_stems() {
        use BrowserEngine::*;
        for stem in ["chrome", "msedge", "brave", "vivaldi", "opera", "opera_gx", "chromium", "arc"]
        {
            assert_eq!(
                known_browser_engine_for_exe_stem(stem),
                Some(Chromium),
                "{stem} should resolve to the Chromium engine"
            );
        }
        for stem in ["firefox", "zen", "librewolf", "waterfox", "floorp"] {
            assert_eq!(
                known_browser_engine_for_exe_stem(stem),
                Some(Gecko),
                "{stem} should resolve to the Gecko engine"
            );
        }
    }

    #[test]
    fn browser_engine_lookup_is_case_insensitive() {
        assert_eq!(
            known_browser_engine_for_exe_stem("Chrome"),
            Some(BrowserEngine::Chromium)
        );
        assert_eq!(
            known_browser_engine_for_exe_stem("CHROME"),
            Some(BrowserEngine::Chromium)
        );
        assert_eq!(
            known_browser_engine_for_exe_stem("ZeN"),
            Some(BrowserEngine::Gecko)
        );
    }

    #[test]
    fn browser_engine_is_none_for_unknown_and_electron_stems() {
        // Electron apps and non-browser exes are not a recognized engine, and an
        // empty stem (a failed path extraction) is likewise `None`.
        for stem in ["slack", "code", "discord", "notepad", ""] {
            assert_eq!(
                known_browser_engine_for_exe_stem(stem),
                None,
                "{stem:?} should not resolve to a browser engine"
            );
        }
    }

    #[test]
    fn browser_engine_resolves_from_extracted_exe_stem() {
        // Documents how slice 3 wires this: extract the stem via the pure path
        // helper, then resolve the engine. A brand-less Chromium fork (Helium ships
        // as chrome.exe) still lands as Chromium; Zen resolves as Gecko.
        assert_eq!(
            known_browser_engine_for_exe_stem(&app_display_name_from_exe_path(
                r"C:\Program Files\Google\Chrome\Application\chrome.exe"
            )),
            Some(BrowserEngine::Chromium)
        );
        assert_eq!(
            known_browser_engine_for_exe_stem(&app_display_name_from_exe_path(
                r"C:\Users\me\AppData\Local\zen\zen.exe"
            )),
            Some(BrowserEngine::Gecko)
        );
    }

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

    #[test]
    fn chromium_family_browsers_are_recognized_with_url_support() {
        let expected = [
            ("com.vivaldi.Vivaldi", "Vivaldi"),
            ("com.operasoftware.Opera", "Opera"),
            ("com.operasoftware.OperaGX", "Opera GX"),
            ("org.chromium.Chromium", "Chromium"),
            ("com.google.Chrome.beta", "Google Chrome Beta"),
            ("com.google.Chrome.dev", "Google Chrome Dev"),
            ("com.microsoft.edgemac.Beta", "Microsoft Edge Beta"),
            ("com.microsoft.edgemac.Dev", "Microsoft Edge Dev"),
            ("com.microsoft.edgemac.Canary", "Microsoft Edge Canary"),
            ("com.brave.Browser.beta", "Brave Browser Beta"),
            ("com.brave.Browser.nightly", "Brave Browser Nightly"),
        ];

        for (bundle_id, app_name) in expected {
            assert!(
                is_known_browser_bundle(bundle_id),
                "{bundle_id} should be a known browser bundle"
            );
            assert!(
                browser_url_metadata_supported(bundle_id),
                "{bundle_id} should support browser URL metadata"
            );
            assert_eq!(
                browser_url_script_app_name(bundle_id),
                Some(app_name),
                "{bundle_id} should map to AppleScript app name {app_name}"
            );
        }
    }

    #[test]
    fn known_browser_apps_have_no_duplicate_bundle_ids() {
        let mut seen = std::collections::HashSet::new();
        for browser in KNOWN_BROWSER_APPS {
            assert!(
                seen.insert(browser.bundle_id),
                "duplicate bundle id in KNOWN_BROWSER_APPS: {}",
                browser.bundle_id
            );
        }
    }

    #[test]
    fn safari_applescript_uses_webkit_dialect() {
        let script = browser_url_applescript("com.apple.Safari")
            .expect("Safari should produce a URL AppleScript");
        assert!(
            script.contains("URL of current tab of front window"),
            "Safari script should read the front window's current tab: {script}"
        );
        assert!(
            !script.contains("front document"),
            "Safari script must not use the focus-ordered front document: {script}"
        );
    }

    #[test]
    fn chrome_applescript_uses_chromium_dialect() {
        let script = browser_url_applescript("com.google.Chrome")
            .expect("Chrome should produce a URL AppleScript");
        assert!(
            script.contains("active tab of front window"),
            "Chrome script should target the active tab of the front window: {script}"
        );
    }

    #[test]
    fn firefox_has_no_applescript() {
        assert_eq!(browser_url_applescript("org.mozilla.firefox"), None);
    }

    #[test]
    fn firefox_is_recognized_with_accessibility_but_no_applescript() {
        assert!(is_known_browser_bundle("org.mozilla.firefox"));
        // No AppleScript surface, so the no-extra-permission flag is false.
        assert!(!browser_url_metadata_supported("org.mozilla.firefox"));
        assert_eq!(browser_url_script_app_name("org.mozilla.firefox"), None);
        // Its URL is read via the Accessibility API instead.
        assert_eq!(
            browser_url_strategy("org.mozilla.firefox"),
            Some(BrowserUrlStrategy::Accessibility)
        );
    }

    #[test]
    fn zen_is_registered_with_accessibility_strategy() {
        assert!(is_known_browser_bundle("app.zen-browser.zen"));
        assert_eq!(
            browser_url_strategy("app.zen-browser.zen"),
            Some(BrowserUrlStrategy::Accessibility)
        );
        assert_eq!(browser_url_applescript("app.zen-browser.zen"), None);
        assert_eq!(
            known_browser_app("app.zen-browser.zen").map(|browser| browser.display_name),
            Some("Zen")
        );
    }

    #[test]
    fn gecko_browsers_use_accessibility_and_chromium_webkit_use_applescript() {
        for bundle_id in ["org.mozilla.firefox", "app.zen-browser.zen"] {
            assert_eq!(
                browser_url_strategy(bundle_id),
                Some(BrowserUrlStrategy::Accessibility),
                "{bundle_id} should read its URL via the Accessibility API"
            );
        }
        assert_eq!(
            browser_url_strategy("com.google.Chrome"),
            Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::Chromium))
        );
        assert_eq!(
            browser_url_strategy("com.apple.Safari"),
            Some(BrowserUrlStrategy::AppleScript(BrowserUrlDialect::WebKit))
        );
        assert_eq!(browser_url_strategy("com.unknown.browser"), None);
    }

    #[test]
    fn new_webkit_browsers_are_recognized_with_url_support() {
        for bundle_id in ["com.apple.SafariTechnologyPreview", "com.kagi.kagimacOS"] {
            assert!(
                is_known_browser_bundle(bundle_id),
                "{bundle_id} should be a known browser bundle"
            );
            assert!(
                browser_url_metadata_supported(bundle_id),
                "{bundle_id} should support browser URL metadata"
            );
            let script = browser_url_applescript(bundle_id)
                .unwrap_or_else(|| panic!("{bundle_id} should produce a URL AppleScript"));
            assert!(
                script.contains("URL of current tab of front window"),
                "{bundle_id} should read the front window's current tab: {script}"
            );
            assert!(
                !script.contains("front document"),
                "{bundle_id} must not use the focus-ordered front document: {script}"
            );
        }
    }

    #[test]
    fn browser_url_cache_reprobes_when_window_title_changes() {
        let base = Instant::now();
        let cache = BrowserUrlProbeCache::from_probe(
            Some("com.apple.Safari".to_string()),
            Some("OxLM — GitHub".to_string()),
            Some("https://github.com/pauldb89/OxLM".to_string()),
            base,
        );

        // Same browser, same title, within the backstop -> reuse the cached URL.
        assert_eq!(
            cache.cached_url_for("com.apple.Safari", Some("OxLM — GitHub"), base),
            Some(Some("https://github.com/pauldb89/OxLM".to_string()))
        );

        // Title moved on (e.g. switched to the Start Page) but still within the
        // re-probe floor -> serve the cached URL rather than re-probing. The floor
        // keeps a dynamic title from triggering a probe on every tick (brief desync).
        assert_eq!(
            cache.cached_url_for("com.apple.Safari", Some("Personal — Start Page"), base),
            Some(Some("https://github.com/pauldb89/OxLM".to_string()))
        );

        // Title moved on and the floor has elapsed -> force a re-probe so the stale
        // URL is never served indefinitely under the new title. This is the desync fix.
        assert_eq!(
            cache.cached_url_for(
                "com.apple.Safari",
                Some("Personal — Start Page"),
                base + BROWSER_URL_PROBE_REPROBE_FLOOR
            ),
            None
        );

        // A different browser never hits this cache.
        assert_eq!(
            cache.cached_url_for("com.google.Chrome", Some("OxLM — GitHub"), base),
            None
        );

        // Same title but past the backstop -> re-probe to catch title-stable (SPA)
        // navigations that change the URL without changing the title.
        assert_eq!(
            cache.cached_url_for(
                "com.apple.Safari",
                Some("OxLM — GitHub"),
                base + BROWSER_URL_PROBE_BACKSTOP_INTERVAL
            ),
            None
        );
    }

    #[test]
    fn browser_url_cache_floor_throttles_dynamic_title_reprobes() {
        let base = Instant::now();
        let cache = BrowserUrlProbeCache::from_probe(
            Some("com.apple.Safari".to_string()),
            Some("(1) Inbox".to_string()),
            Some("https://mail.example.com/inbox".to_string()),
            base,
        );

        // A dynamic title (unread counter) mutates every ~1s tick. While each tick
        // lands inside the re-probe floor the cached URL is reused — no probe storm.
        for (offset_ms, title) in [
            (0_u64, "(1) Inbox"),
            (1000, "(2) Inbox"),
            (1499, "(3) Inbox"),
        ] {
            assert_eq!(
                cache.cached_url_for(
                    "com.apple.Safari",
                    Some(title),
                    base + Duration::from_millis(offset_ms)
                ),
                Some(Some("https://mail.example.com/inbox".to_string())),
                "title change within the floor (t+{offset_ms}ms) should reuse the cached URL"
            );
        }

        // Once a changed-title tick lands past the floor, we re-probe.
        assert_eq!(
            cache.cached_url_for(
                "com.apple.Safari",
                Some("(4) Inbox"),
                base + BROWSER_URL_PROBE_REPROBE_FLOOR
            ),
            None,
            "title change past the floor should force a re-probe"
        );
    }

    #[test]
    fn browser_url_strategy_fields_are_consistent() {
        for browser in KNOWN_BROWSER_APPS {
            let applescript = matches!(
                browser.url_strategy,
                Some(BrowserUrlStrategy::AppleScript(_))
            );
            // url_script_app_name is set iff the AppleScript strategy applies.
            assert_eq!(
                browser.url_script_app_name.is_some(),
                applescript,
                "url_script_app_name must agree with the AppleScript strategy for {}",
                browser.bundle_id
            );
            // browser_url_metadata_supported is true iff the AppleScript strategy applies.
            assert_eq!(
                browser_url_metadata_supported(browser.bundle_id),
                applescript,
                "metadata-supported flag must reflect the resolved strategy for {}",
                browser.bundle_id
            );
            // AppleScript is produced only for the AppleScript strategy.
            assert_eq!(
                browser_url_applescript(browser.bundle_id).is_some(),
                applescript,
                "AppleScript presence must match the AppleScript strategy for {}",
                browser.bundle_id
            );
        }
    }

    /// Drives the probe cache exactly the way the Live-mode metadata caller does
    /// (`browser_url_probe_for_active_bundle`): each ~1s metadata tick calls
    /// `cached_url_for`; a `None` return is a probe (osascript spawn / AX read)
    /// and rebuilds the cache via `from_probe`. This counts the real probes over
    /// a one-hour frontmost session with a per-second-changing title, which is
    /// the workload INV-P1 (the floor) is meant to bound.
    fn count_probes_over_session(
        tick: Duration,
        session: Duration,
        mut title_at: impl FnMut(u64) -> String,
    ) -> u64 {
        let bundle = "com.apple.Safari";
        let base = Instant::now();
        // Seed the cache with the first probe (tick 0).
        let mut cache = BrowserUrlProbeCache::from_probe(
            Some(bundle.to_string()),
            Some(title_at(0)),
            Some("https://mail.example.com/inbox".to_string()),
            base,
        );
        let mut probes = 1; // the seed probe at tick 0
        let ticks = (session.as_millis() / tick.as_millis()) as u64;
        for n in 1..=ticks {
            let now = base + tick * (n as u32);
            let title = title_at(n);
            match cache.cached_url_for(bundle, Some(&title), now) {
                Some(_cached) => {} // served from cache, no probe
                None => {
                    // A real caller re-probes and rebuilds the cache here.
                    probes += 1;
                    cache = BrowserUrlProbeCache::from_probe(
                        Some(bundle.to_string()),
                        Some(title),
                        Some("https://mail.example.com/inbox".to_string()),
                        now,
                    );
                }
            }
        }
        probes
    }

    #[test]
    fn dynamic_title_probe_rate_is_floor_bounded_not_per_tick() {
        // Gmail-style per-second unread counter, held frontmost for one hour at a
        // 1s metadata tick (3600 ticks). The title changes on EVERY tick.
        let tick = Duration::from_secs(1);
        let session = Duration::from_secs(3600);
        let probes =
            count_probes_over_session(tick, session, |n| format!("({n}) Inbox — Gmail"));

        // INV-P1: a dynamic-title page cannot probe faster than once per
        // BROWSER_URL_PROBE_REPROBE_FLOOR. Over a 3600s session that is an upper
        // bound of ceil(3600s / 1.5s) = 2400 probes. We assert the realized count
        // matches the floor cadence (every other 1s tick, since 1.5s floor lands
        // a re-probe on every 2nd tick): 1 seed + floor(3599/2) re-probes.
        let floor_secs = BROWSER_URL_PROBE_REPROBE_FLOOR.as_secs_f64();
        let max_probes_per_floor = (session.as_secs_f64() / floor_secs).ceil() as u64 + 1;
        assert!(
            probes <= max_probes_per_floor,
            "probe count {probes} must not exceed the floor bound {max_probes_per_floor} \
             (1 probe / {floor_secs}s over {}s)",
            session.as_secs()
        );

        // Quantify the realized cadence: with a 1s tick and a 1.5s floor, a changed
        // title re-probes on every 2nd tick (the 1.5s floor is only cleared at the
        // 2s tick boundary), i.e. ~1800 probes/hour = ~30 probes/min.
        let per_min = (probes as f64) / 60.0;
        assert!(
            (29.0..=31.0).contains(&per_min),
            "expected ~30 probes/min under a 1s tick + 1.5s floor, got {per_min:.1} \
             ({probes} probes/hour)"
        );

        // Contrast: the pre-PR flat 15s poll probed once per 15s = 4 probes/min =
        // 240 probes/hour. The new title-gating is ~7.5x MORE probes for this
        // dynamic-title workload. Documented here so the tradeoff is explicit.
        let pre_pr_probes_per_hour = 3600 / 15;
        assert!(
            probes > pre_pr_probes_per_hour * 5,
            "sanity: title-gating should be a large multiple of the old 15s poll \
             ({probes} vs {pre_pr_probes_per_hour})"
        );
    }
}
