//! macOS shared-access-group key store (ADR 0057): a data-protection keychain
//! generic-password item in the team-prefixed group both the app and the
//! `mnema-cli` sidecar are entitled to read silently. Out-of-group processes
//! (and unsigned/dev-signed builds — the entitlement only validates under a
//! provisioning profile or Developer ID) get flat denial with no prompt.

use core_foundation::array::CFArray;
use core_foundation::base::{kCFAllocatorDefault, CFAllocatorRef, CFGetTypeID, CFType, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use security_framework::passwords::{generic_password, set_generic_password_options};
use security_framework::passwords_options::PasswordOptions;
use std::os::raw::c_void;

use super::{CaptureIndexKeyStoreAdapter, KEYCHAIN_SERVICE};
use crate::error::{AppInfraError, Result};

pub(super) const SHARED_ACCESS_GROUP: &str = "RJYMY4RR97.day.mnema.capture-index";
const APP_GROUPS_ENTITLEMENT: &str = "com.apple.security.application-groups";

/// `errSecItemNotFound`: nothing matched — genuinely absent, or invisible
/// because it lives outside our access group.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
/// `errSecMissingEntitlement`: this build's signature cannot claim the group
/// (unsigned, ad-hoc, or dev-signed without a provisioning profile).
const ERR_SEC_MISSING_ENTITLEMENT: i32 = -34018;

// security-framework-sys 2.x exports the accessibility *values* but not the
// `kSecAttrAccessible` dictionary key, and neither crate wraps SecTask; declare
// the needed Security.framework symbols directly.
#[link(name = "Security", kind = "framework")]
extern "C" {
    static kSecAttrAccessible: CFStringRef;
    static kSecAttrAccessibleAfterFirstUnlock: CFStringRef;
    fn SecTaskCreateFromSelf(allocator: CFAllocatorRef) -> CFTypeRef;
    fn SecTaskCopyValueForEntitlement(
        task: CFTypeRef,
        entitlement: CFStringRef,
        error: *mut c_void,
    ) -> CFTypeRef;
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SharedGroupKeychainAdapter;

fn group_options(index_id: &str) -> PasswordOptions {
    let mut options = PasswordOptions::new_generic_password(KEYCHAIN_SERVICE, index_id);
    options.use_protected_keychain();
    options.set_access_group(SHARED_ACCESS_GROUP);
    options
}

fn store_options(index_id: &str) -> PasswordOptions {
    let mut options = group_options(index_id);
    // kSecAttrAccessibleAfterFirstUnlock: a login-item launch must open the
    // index before any user interaction.
    #[allow(deprecated)]
    options.query.push((
        unsafe { CFString::wrap_under_get_rule(kSecAttrAccessible) },
        unsafe { CFString::wrap_under_get_rule(kSecAttrAccessibleAfterFirstUnlock) }.into_CFType(),
    ));
    options
}

impl CaptureIndexKeyStoreAdapter for SharedGroupKeychainAdapter {
    fn load_key(&self, index_id: &str) -> Result<Option<String>> {
        match generic_password(group_options(index_id)) {
            Ok(bytes) => {
                let key = String::from_utf8_lossy(&bytes).trim().to_string();
                if key.is_empty() {
                    return Ok(None);
                }
                Ok(Some(key))
            }
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(error) if error.code() == ERR_SEC_MISSING_ENTITLEMENT => {
                Err(AppInfraError::CaptureIndexEncryption(
                    "this build cannot access Mnema's keychain group (errSecMissingEntitlement)"
                        .to_string(),
                ))
            }
            Err(error) => Err(AppInfraError::CaptureIndexEncryption(format!(
                "shared-group keychain read failed: {error}"
            ))),
        }
    }

    fn store_key(&self, index_id: &str, key: &str) -> Result<()> {
        // `set_generic_password_options` adds, and on `errSecDuplicateItem`
        // updates the existing item in place via `SecItemUpdate` — never
        // delete+recreate (ADR 0057: recreation loses entitlement bindings and
        // was a prompt source historically).
        set_generic_password_options(key.as_bytes(), store_options(index_id)).map_err(|error| {
            AppInfraError::CaptureIndexEncryption(format!(
                "shared-group keychain write failed: {error}"
            ))
        })
    }

    fn missing_key_error(&self, index_id: &str) -> AppInfraError {
        // The data-protection keychain cannot distinguish denied from missing,
        // so name the real cause when this build lacks the group entitlement.
        if !process_has_group_entitlement() {
            return AppInfraError::CaptureIndexEncryption(
                "this build cannot access Mnema's keychain group — set MNEMA_CAPTURE_INDEX_KEY_DIR or use a signed build"
                    .to_string(),
            );
        }
        AppInfraError::CaptureIndexEncryption(format!(
            "capture index key for {index_id} is missing from Keychain"
        ))
    }
}

/// Whether this process's own code signature carries the capture-index access
/// group in its `com.apple.security.application-groups` entitlement.
fn process_has_group_entitlement() -> bool {
    unsafe {
        let task = SecTaskCreateFromSelf(kCFAllocatorDefault);
        if task.is_null() {
            return false;
        }
        let task = CFType::wrap_under_create_rule(task);
        let entitlement_key = CFString::new(APP_GROUPS_ENTITLEMENT);
        let value = SecTaskCopyValueForEntitlement(
            task.as_CFTypeRef(),
            entitlement_key.as_concrete_TypeRef(),
            std::ptr::null_mut(),
        );
        if value.is_null() {
            return false;
        }
        let value = CFType::wrap_under_create_rule(value);
        let Some(groups) = value.downcast::<CFArray<*const c_void>>() else {
            return false;
        };
        groups.iter().any(|item| {
            let item: *const c_void = *item;
            !item.is_null()
                && CFGetTypeID(item) == CFString::type_id()
                && CFString::wrap_under_get_rule(item as CFStringRef).to_string()
                    == SHARED_ACCESS_GROUP
        })
    }
}
