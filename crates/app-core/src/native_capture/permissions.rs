//! What capture can do on this Mac and what it is allowed to: platform support,
//! the screen/microphone/system-audio permissions (and their prompts and System
//! Settings panes), and the Accessibility permission Gecko URL capture needs.

use std::sync::Arc;

use capture_microphone as microphone_capture;
use capture_types::{
    CapturePermissionState, CapturePermissions, CapturePermissionsResponse, CaptureSources,
    CaptureSupportResponse,
};

#[cfg(target_os = "macos")]
use crate::native_capture::browser_url_ax;
use crate::native_capture::control::current_recording_settings_from_core;
use crate::native_capture::lifecycle::NativeCaptureState;
use crate::native_capture::log_format::{
    log_capture_permissions_if_changed, log_capture_support_if_changed,
};
#[cfg(target_os = "macos")]
use crate::native_capture::system_audio;
#[cfg(target_os = "macos")]
use crate::native_capture::system_wake::recover_screen_capture_after_possible_missed_wake;
use crate::AppCore;

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
            system_audio: capture_system_audio::supports_system_audio_capture(),
        },
    };

    log_capture_support_if_changed(&response);
    response
}

/// System audio's permission is inferred from what its taps have delivered
/// (ADR 0052) — it is no longer an alias of the screen grant, which it never
/// shared a TCC category with in the first place.
#[cfg(target_os = "macos")]
fn system_audio_permission_state() -> CapturePermissionState {
    system_audio::permission_state()
}

#[cfg(not(target_os = "macos"))]
fn system_audio_permission_state() -> CapturePermissionState {
    CapturePermissionState::Unsupported
}

/// Loads the persisted denial evidence into the permission cache, on the
/// deferred-startup thread (ADR 0052).
#[cfg(target_os = "macos")]
pub async fn hydrate_system_audio_permission_evidence(infra: &::app_infra::AppInfra) {
    system_audio::hydrate_evidence(infra).await;
}

/// The prompt id for the "system audio may be blocked" hint. Versioned like the
/// recommended-exclusions prompt: bumping it re-shows the hint to users who
/// dismissed the previous wording.
pub const SYSTEM_AUDIO_BLOCKED_PROMPT_ID: &str = "system-audio-blocked/v1";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemAudioAccessHint {
    pub prompt_id: String,
    pub should_show: bool,
}

/// Whether to warn that system audio may be blocked (ADR 0052).
///
/// Every half of the decision lives here, mirroring
/// `get_sensitive_capture_recommendations`: system audio is switched on, every
/// tap so far has delivered only silence, and the user has not already waved the
/// hint away. A denied tap produces no OS-level signal at all, so this is the
/// only thing standing between a denial and hours of silent recordings — and
/// because a genuinely quiet Mac is indistinguishable from a denied one, it is
/// dismissible rather than blocking.
pub fn get_system_audio_access_hint(core: &AppCore) -> SystemAudioAccessHint {
    let dismissed = crate::one_time_prompts::current_state(core)
        .prompts
        .get(SYSTEM_AUDIO_BLOCKED_PROMPT_ID)
        .is_some_and(|record| record.dismissed_at.is_some());
    // Evidence outlives the switch: a user who turned system audio off after a
    // silent session must not keep being warned about a source they no longer
    // record.
    let enabled = current_recording_settings_from_core(core).capture_system_audio;

    SystemAudioAccessHint {
        prompt_id: SYSTEM_AUDIO_BLOCKED_PROMPT_ID.to_string(),
        should_show: enabled
            && !dismissed
            && system_audio_permission_state() == CapturePermissionState::PossiblyBlocked,
    }
}

/// Raises the "Screen & System Audio Recording" prompt by running a throwaway
/// tap. A failure to *build* the tap is worth reporting; the prompt's answer is
/// not knowable here either way.
#[cfg(target_os = "macos")]
async fn request_system_audio_permission(core: &AppCore) -> Result<(), String> {
    if !capture_system_audio::supports_system_audio_capture() {
        return Ok(());
    }

    core.spawn_blocking(capture_system_audio::prompt_for_system_audio_permission)
        .await
        .map_err(|error| format!("system audio permission request failed: {error}"))?
        .map_err(|error| format!("system audio permission request failed: {}", error.message))
}

#[cfg(not(target_os = "macos"))]
async fn request_system_audio_permission(_core: &AppCore) -> Result<(), String> {
    Ok(())
}

pub fn get_capture_permissions(core: &Arc<AppCore>) -> CapturePermissionsResponse {
    #[cfg(target_os = "macos")]
    recover_screen_capture_after_possible_missed_wake(core.clone());

    let state = core.state::<NativeCaptureState>();

    let runtime = state.lock().expect("native capture state poisoned");
    let permissions = CapturePermissions {
        screen: capture_screen::screen_permission_state(),
        microphone: microphone_capture::microphone_permission_state(),
        // Inferred, not read: a process tap has its own TCC category and no
        // authorization query at all (ADR 0052).
        system_audio: system_audio_permission_state(),
    };

    log_capture_permissions_if_changed(&permissions);

    CapturePermissionsResponse {
        permissions,
        session: runtime.session(),
    }
}

/// Trigger the native macOS permission prompt for a capture source, then return
/// the refreshed permission state. `kind` is "screen", "microphone", or
/// "systemAudio". The underlying request blocks (microphone waits on a system
/// callback, system audio runs a throwaway tap), so it runs on a blocking thread
/// to keep the UI responsive.
pub async fn request_capture_permission(
    kind: String,
    core: &Arc<AppCore>,
) -> Result<CapturePermissionsResponse, String> {
    match kind.as_str() {
        "screen" => {
            core.spawn_blocking(|| {
                capture_screen::ensure_screen_permission();
            })
            .await
            .map_err(|error| format!("screen permission request failed: {error}"))?;
        }
        // System audio has its own TCC category and no authorization query, so
        // the only way to raise its prompt is to build a tap and read it — the
        // grant is never reported back, only inferred later (ADR 0052).
        "systemAudio" => {
            request_system_audio_permission(core).await?;
        }
        "microphone" => {
            core.spawn_blocking(|| {
                microphone_capture::ensure_microphone_permission();
            })
            .await
            .map_err(|error| format!("microphone permission request failed: {error}"))?;
        }
        other => return Err(format!("unknown permission kind: {other}")),
    }

    Ok(get_capture_permissions(core))
}

/// Open the macOS Privacy & Security pane for a capture source so the user can
/// flip a permission that was already denied (macOS will not re-prompt once
/// denied).
pub fn open_capture_privacy_settings(kind: String, core: &AppCore) -> Result<(), String> {
    let url = match kind.as_str() {
        "screen" | "systemAudio" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        "microphone" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
        other => return Err(format!("unknown permission kind: {other}")),
    };

    core.host()
        .open_url(url)
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
pub(crate) fn browser_url_accessibility_status() -> BrowserUrlAccessibilityStatus {
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
            let installed =
                super::app_candidates::macos_application_bundle_path_for_bundle_id(app.bundle_id)
                    .is_some();
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
pub fn get_browser_url_accessibility_status() -> BrowserUrlAccessibilityStatus {
    browser_url_accessibility_status()
}

/// Raise the macOS Accessibility permission prompt (adds Mnema to the
/// Accessibility list and points the user at System Settings), then return the
/// refreshed status. The grant itself is asynchronous — the user flips the
/// toggle in System Settings, so `trusted` may still be `false` right after.
pub fn request_browser_url_accessibility() -> BrowserUrlAccessibilityStatus {
    #[cfg(target_os = "macos")]
    {
        let _ = browser_url_ax::request_accessibility_with_prompt();
    }
    browser_url_accessibility_status()
}

/// Open the macOS Privacy & Security → Accessibility pane so the user can grant
/// (or re-grant) Mnema the Accessibility permission.
pub fn open_browser_url_accessibility_settings(core: &AppCore) -> Result<(), String> {
    core.host()
        .open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .map_err(|error| format!("failed to open accessibility settings: {error}"))
}
