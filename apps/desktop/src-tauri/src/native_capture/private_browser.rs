use capture_metadata::{is_known_browser_bundle, is_private_browser_title, WindowContext};
use capture_types::CapturePermissionState;
use std::collections::{BTreeMap, BTreeSet};

pub const DETECTION_MODE_TITLE_ONLY: &str = "title_only";
pub const DETECTION_MODE_TITLE_AND_ACCESSIBILITY: &str = "title_and_accessibility";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateBrowserDetection {
    pub accessibility_permission: CapturePermissionState,
    pub mode: String,
    pub detected_window_ids: BTreeSet<u32>,
    pub reasons: BTreeMap<u32, String>,
}

impl Default for PrivateBrowserDetection {
    fn default() -> Self {
        Self {
            accessibility_permission: accessibility_permission_state(),
            mode: DETECTION_MODE_TITLE_ONLY.to_string(),
            detected_window_ids: BTreeSet::new(),
            reasons: BTreeMap::new(),
        }
    }
}

pub fn detect_private_browser_windows(
    visible_windows: &[WindowContext],
) -> PrivateBrowserDetection {
    let mut detection = PrivateBrowserDetection::default();

    for window in visible_windows {
        if is_known_browser_bundle(window.bundle_id.as_deref().unwrap_or_default())
            && is_private_browser_title(&window.title)
        {
            detection.detected_window_ids.insert(window.window_id);
            detection
                .reasons
                .insert(window.window_id, "title_private_browser".to_string());
        }
    }

    #[cfg(target_os = "macos")]
    {
        if matches!(
            detection.accessibility_permission,
            CapturePermissionState::Granted
        ) {
            detection.mode = DETECTION_MODE_TITLE_AND_ACCESSIBILITY.to_string();
            for (window_id, reason) in
                detect_private_browser_windows_with_accessibility(visible_windows)
            {
                detection.detected_window_ids.insert(window_id);
                detection.reasons.insert(window_id, reason);
            }
        }
    }

    detection
}

#[cfg(not(target_os = "macos"))]
pub fn accessibility_permission_state() -> CapturePermissionState {
    CapturePermissionState::Unsupported
}

#[cfg(target_os = "macos")]
pub fn accessibility_permission_state() -> CapturePermissionState {
    if unsafe { AXIsProcessTrusted() != 0 } {
        CapturePermissionState::Granted
    } else {
        CapturePermissionState::Denied
    }
}

#[cfg(not(target_os = "macos"))]
pub fn request_accessibility_permission() -> CapturePermissionState {
    CapturePermissionState::Unsupported
}

#[cfg(target_os = "macos")]
pub fn request_accessibility_permission() -> CapturePermissionState {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;

    let prompt_key = CFString::new("AXTrustedCheckOptionPrompt");
    let options = CFDictionary::from_CFType_pairs(&[(
        prompt_key.as_CFType(),
        CFBoolean::true_value().as_CFType(),
    )]);
    unsafe {
        let _ = AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef() as _);
    }
    accessibility_permission_state()
}

#[cfg(target_os = "macos")]
fn detect_private_browser_windows_with_accessibility(
    visible_windows: &[WindowContext],
) -> Vec<(u32, String)> {
    let mut by_pid: BTreeMap<i32, Vec<&WindowContext>> = BTreeMap::new();
    for window in visible_windows {
        let Some(pid) = window.owner_pid else {
            continue;
        };
        let Some(bundle_id) = window.bundle_id.as_deref() else {
            continue;
        };
        if private_browser_ax_token(bundle_id).is_some() {
            by_pid.entry(pid).or_default().push(window);
        }
    }

    let mut detected = Vec::new();
    for (pid, windows) in by_pid {
        let Some(token) = windows
            .iter()
            .find_map(|window| private_browser_ax_token(window.bundle_id.as_deref()?))
        else {
            continue;
        };
        for ax_window in ax_browser_windows(pid) {
            let Some(window_id) = ax_window.window_id else {
                continue;
            };
            if !windows.iter().any(|window| window.window_id == window_id) {
                continue;
            }
            if ax_window
                .texts
                .iter()
                .any(|value| is_ax_private_browser_signal(value, token))
            {
                detected.push((window_id, format!("accessibility_{token}")));
            }
        }
    }
    detected
}

#[cfg(target_os = "macos")]
fn private_browser_ax_token(bundle_id: &str) -> Option<&'static str> {
    match bundle_id {
        "com.google.Chrome" | "com.google.Chrome.canary" | "com.brave.Browser" => Some("incognito"),
        "com.microsoft.edgemac" | "com.microsoft.edgemac.Canary" | "com.microsoft.edgemac.Dev" => {
            Some("inprivate")
        }
        "org.mozilla.firefox" | "org.mozilla.firefoxdeveloperedition" => Some("private browsing"),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn is_ax_private_browser_signal(value: &str, token: &str) -> bool {
    if !value.contains(token) {
        return false;
    }
    let command_like_patterns = [
        "new incognito",
        "open incognito",
        "new inprivate",
        "open inprivate",
        "new private browsing",
        "open private browsing",
    ];
    !command_like_patterns
        .iter()
        .any(|pattern| value.contains(pattern))
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Default)]
struct AxWindowSnapshot {
    window_id: Option<u32>,
    texts: Vec<String>,
}

#[cfg(target_os = "macos")]
fn ax_browser_windows(pid: i32) -> Vec<AxWindowSnapshot> {
    use core_foundation::base::TCFType;

    let app = unsafe { AXUIElementCreateApplication(pid) };
    if app.is_null() {
        return Vec::new();
    }
    let app = AxElement(app);
    let Some(windows) = ax_copy_array_attribute(app.0, "AXWindows") else {
        return Vec::new();
    };

    windows
        .iter()
        .filter_map(|value| {
            let element = value.as_CFTypeRef() as AXUIElementRef;
            if element.is_null() {
                return None;
            }
            Some(ax_window_snapshot(element))
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn ax_window_snapshot(element: AXUIElementRef) -> AxWindowSnapshot {
    let mut snapshot = AxWindowSnapshot {
        window_id: ax_copy_number_attribute(element, "AXWindowNumber"),
        texts: Vec::new(),
    };
    collect_ax_texts(element, 0, &mut snapshot.texts);
    snapshot
}

#[cfg(target_os = "macos")]
fn collect_ax_texts(element: AXUIElementRef, depth: usize, texts: &mut Vec<String>) {
    if depth > 5 || texts.len() > 64 {
        return;
    }
    if ax_element_role_is_command_surface(element) {
        return;
    }
    for attribute in ["AXDescription", "AXTitle", "AXRoleDescription"] {
        if let Some(value) = ax_copy_string_attribute(element, attribute) {
            texts.push(value.to_ascii_lowercase());
        }
    }

    use core_foundation::base::TCFType;
    let Some(children) = ax_copy_array_attribute(element, "AXChildren") else {
        return;
    };
    for child in children.iter() {
        let child_element = child.as_CFTypeRef() as AXUIElementRef;
        if !child_element.is_null() {
            collect_ax_texts(child_element, depth + 1, texts);
        }
    }
}

#[cfg(target_os = "macos")]
fn ax_element_role_is_command_surface(element: AXUIElementRef) -> bool {
    let Some(role) = ax_copy_string_attribute(element, "AXRole") else {
        return false;
    };
    matches!(
        role.as_str(),
        "AXMenuBar" | "AXMenu" | "AXMenuItem" | "AXMenuButton"
    )
}

#[cfg(target_os = "macos")]
fn ax_copy_string_attribute(element: AXUIElementRef, attribute: &str) -> Option<String> {
    use core_foundation::base::CFType;
    use core_foundation::string::CFString;
    let value: CFType = ax_copy_attribute(element, attribute)?;
    value
        .downcast::<CFString>()
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
}

#[cfg(target_os = "macos")]
fn ax_copy_number_attribute(element: AXUIElementRef, attribute: &str) -> Option<u32> {
    use core_foundation::number::CFNumber;
    let value = ax_copy_attribute(element, attribute)?;
    value
        .downcast::<CFNumber>()
        .and_then(|number| number.to_i64())
        .and_then(|value| u32::try_from(value).ok())
}

#[cfg(target_os = "macos")]
fn ax_copy_array_attribute(
    element: AXUIElementRef,
    attribute: &str,
) -> Option<core_foundation::array::CFArray<core_foundation::base::CFType>> {
    use core_foundation::array::CFArray;
    use core_foundation::base::{CFType, TCFType};
    let value = ax_copy_attribute(element, attribute)?;
    let array = value.downcast_into::<CFArray>()?;
    let value_ref = array.as_CFTypeRef() as core_foundation_sys::array::CFArrayRef;
    std::mem::forget(array);
    Some(unsafe { CFArray::<CFType>::wrap_under_create_rule(value_ref) })
}

#[cfg(target_os = "macos")]
fn ax_copy_attribute(
    element: AXUIElementRef,
    attribute: &str,
) -> Option<core_foundation::base::CFType> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::string::CFString;

    let attribute = CFString::new(attribute);
    let mut value = std::ptr::null();
    let error = unsafe {
        AXUIElementCopyAttributeValue(element, attribute.as_concrete_TypeRef(), &mut value)
    };
    if error != 0 || value.is_null() {
        return None;
    }
    Some(unsafe { CFType::wrap_under_create_rule(value) })
}

#[cfg(target_os = "macos")]
struct AxElement(AXUIElementRef);

#[cfg(target_os = "macos")]
impl Drop for AxElement {
    fn drop(&mut self) {
        unsafe {
            core_foundation_sys::base::CFRelease(self.0 as _);
        }
    }
}

#[cfg(target_os = "macos")]
type AXError = i32;
#[cfg(target_os = "macos")]
type AXUIElementRef = *const std::ffi::c_void;

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> core_foundation_sys::base::Boolean;
    fn AXIsProcessTrustedWithOptions(
        options: core_foundation_sys::dictionary::CFDictionaryRef,
    ) -> core_foundation_sys::base::Boolean;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: core_foundation_sys::string::CFStringRef,
        value: *mut core_foundation_sys::base::CFTypeRef,
    ) -> AXError;
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn ax_private_browser_signal_rejects_browser_commands() {
        assert!(!is_ax_private_browser_signal(
            "new incognito window",
            "incognito"
        ));
        assert!(!is_ax_private_browser_signal(
            "open inprivate window",
            "inprivate"
        ));
        assert!(!is_ax_private_browser_signal(
            "new private browsing window",
            "private browsing"
        ));
    }

    #[test]
    fn ax_private_browser_signal_accepts_mode_labels() {
        assert!(is_ax_private_browser_signal(
            "incognito profile active",
            "incognito"
        ));
        assert!(is_ax_private_browser_signal(
            "private browsing mode",
            "private browsing"
        ));
    }
}
