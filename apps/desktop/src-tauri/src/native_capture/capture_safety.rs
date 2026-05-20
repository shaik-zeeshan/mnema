#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum CaptureSafetyDetectorState {
    Available { credential_entry_active: bool },
    AccessibilityPermissionMissing,
    UnsupportedPlatform,
    DetectorError,
}

#[cfg(target_os = "macos")]
pub(crate) fn detect_credential_entry() -> CaptureSafetyDetectorState {
    macos::detect_credential_entry()
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn detect_credential_entry() -> CaptureSafetyDetectorState {
    CaptureSafetyDetectorState::UnsupportedPlatform
}

#[cfg(target_os = "macos")]
mod macos {
    use core_foundation::{
        base::{CFRelease, CFTypeRef, TCFType},
        string::CFString,
    };
    use core_foundation_sys::{
        base::{Boolean, OSStatus},
        string::CFStringRef,
    };

    use super::CaptureSafetyDetectorState;

    type AXUIElementRef = *const std::ffi::c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> Boolean;
        fn AXUIElementCreateSystemWide() -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> OSStatus;
    }

    const AX_ERROR_SUCCESS: OSStatus = 0;

    pub(super) fn detect_credential_entry() -> CaptureSafetyDetectorState {
        unsafe {
            if AXIsProcessTrusted() == 0 {
                return CaptureSafetyDetectorState::AccessibilityPermissionMissing;
            }

            let system = AXUIElementCreateSystemWide();
            if system.is_null() {
                return CaptureSafetyDetectorState::DetectorError;
            }

            let focused_elements = match focused_element_candidates(system) {
                Ok(elements) => elements,
                Err(()) => return CaptureSafetyDetectorState::DetectorError,
            };
            let secure = focused_elements
                .iter()
                .copied()
                .any(|element| element_or_parent_is_secure_text_field(element));
            for element in focused_elements {
                CFRelease(element);
            }

            CaptureSafetyDetectorState::Available {
                credential_entry_active: secure,
            }
        }
    }

    unsafe fn focused_element_candidates(system: AXUIElementRef) -> Result<Vec<CFTypeRef>, ()> {
        let mut candidates = Vec::new();
        if let Some(focused) = copy_attribute(system, "AXFocusedUIElement")? {
            candidates.push(focused);
        }
        if let Some(app) = copy_attribute(system, "AXFocusedApplication")? {
            if let Some(focused) = copy_attribute(app.cast(), "AXFocusedUIElement")? {
                candidates.push(focused);
            }
            for window_attribute in ["AXFocusedWindow", "AXMainWindow"] {
                if let Some(window) = copy_attribute(app.cast(), window_attribute)? {
                    if let Some(focused) = copy_attribute(window.cast(), "AXFocusedUIElement")? {
                        candidates.push(focused);
                    }
                    CFRelease(window);
                }
            }
            CFRelease(app);
        }
        Ok(candidates)
    }

    unsafe fn element_or_parent_is_secure_text_field(element: CFTypeRef) -> bool {
        let mut current = element;
        let mut owns_current = false;
        for _ in 0..4 {
            if element_is_secure_text_field(current) {
                if owns_current {
                    CFRelease(current);
                }
                return true;
            }
            let parent = copy_attribute(current.cast(), "AXParent").ok().flatten();
            if owns_current {
                CFRelease(current);
            }
            let Some(parent) = parent else {
                return false;
            };
            current = parent;
            owns_current = true;
        }
        if owns_current {
            CFRelease(current);
        }
        false
    }

    unsafe fn element_is_secure_text_field(element: CFTypeRef) -> bool {
        let role = copy_string_attribute(element, "AXRole");
        let subrole = copy_string_attribute(element, "AXSubrole");
        let role_description = copy_string_attribute(element, "AXRoleDescription");
        if subrole
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("AXSecureTextField"))
        {
            return true;
        }
        let Some(role_description) = role_description else {
            return false;
        };
        let normalized = role_description.to_ascii_lowercase();
        normalized == "secure text field"
            || (role.as_deref() == Some("AXTextField") && normalized.contains("password"))
    }

    unsafe fn copy_string_attribute(element: CFTypeRef, attribute: &str) -> Option<String> {
        let value = copy_attribute(element.cast(), attribute).ok().flatten()?;
        let string = CFString::wrap_under_create_rule(value.cast()).to_string();
        Some(string)
    }

    unsafe fn copy_attribute(
        element: AXUIElementRef,
        attribute: &str,
    ) -> Result<Option<CFTypeRef>, ()> {
        let attribute = CFString::new(attribute);
        let mut value: CFTypeRef = std::ptr::null();
        let status = AXUIElementCopyAttributeValue(
            element,
            attribute.as_concrete_TypeRef(),
            &mut value as *mut CFTypeRef,
        );
        if status == AX_ERROR_SUCCESS {
            Ok((!value.is_null()).then_some(value))
        } else {
            Ok(None)
        }
    }
}
