//! The updater's install gate (BUILD-PROMPT §9.2): an Apple code-signature check
//! on a downloaded, extracted `.app` before it may replace the running one.
//!
//! The new bundle must be validly signed (all architectures, nested code, strict),
//! carry the running app's own team and bundle id, and its
//! `Contents/Helpers/mnema-cli.app` must be valid with that same team. The own
//! team is read from the running process (`SecCodeCopySelf`), never hard-coded,
//! so Apple Development builds can update each other in testing. A running app
//! without a team (unsigned or ad-hoc dev build) can't install updates at all.
//!
//! Tests build their fixture bundles at test time with `/usr/bin/codesign`. The
//! ones that need a real signing identity are `#[ignore]`d; run them with an
//! Apple Development identity from `security find-identity -v -p codesigning`
//! (its SHA-1 or full name):
//!
//! ```sh
//! MNEMA_TEST_SIGNING_IDENTITY="<identity>" \
//!   cargo test -p app-core updates::install_gate -- --ignored
//! ```

use std::path::Path;
use std::ptr;

use core_foundation::base::{CFType, TCFType};
use core_foundation::bundle::CFBundle;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_foundation::url::CFURL;
use core_foundation_sys::base::OSStatus;
use core_foundation_sys::dictionary::CFDictionaryRef;
use core_foundation_sys::string::CFStringRef;
use security_framework::os::macos::code_signing::{Flags, SecCode, SecRequirement, SecStaticCode};
use security_framework_sys::code_signing::{
    kSecCSCheckAllArchitectures, kSecCSCheckNestedCode, kSecCSStrictValidate, SecCSFlags,
    SecStaticCodeCheckValidity, SecStaticCodeRef,
};

/// Why the gate refused a bundle. Logged by variant name only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateRefusal {
    /// The bundle's (or its helper's) signature doesn't validate.
    InvalidSignature,
    /// Signed by a different team than the running app (or by none).
    TeamMismatch,
    /// Its `CFBundleIdentifier` isn't the running app's.
    BundleIdMismatch,
    /// `Contents/Helpers/mnema-cli.app` isn't signed by the running app's team.
    HelperTeamMismatch,
    /// The running app has no team (unsigned or ad-hoc), so installs are disabled.
    NoOwnTeam,
    /// The bundle or its helper couldn't be opened as code.
    Unreadable,
}

/// The running app's signing identity, which an update must match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnIdentity {
    pub team: Option<String>,
    pub bundle_id: String,
}

impl OwnIdentity {
    /// Reads the running process's team and the main bundle's identifier.
    pub fn current() -> Self {
        let team = SecCode::for_self(Flags::NONE)
            .ok()
            .and_then(|code| signing_info(code.as_concrete_TypeRef().cast()).ok())
            .and_then(|info| team_of(&info));
        let bundle_id = CFBundle::main_bundle()
            .info_dictionary()
            .find(CFString::from_static_string("CFBundleIdentifier"))
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string())
            // Only a bare (non-bundled) binary lacks one; it then matches nothing.
            .unwrap_or_default();
        Self { team, bundle_id }
    }
}

/// Checks a downloaded `.app` against the running app's own identity.
pub fn check_update_bundle(new_app: &Path) -> Result<(), GateRefusal> {
    check_update_bundle_as(new_app, &OwnIdentity::current())
}

/// [`check_update_bundle`] against an explicit identity (tests inject one).
pub fn check_update_bundle_as(new_app: &Path, own: &OwnIdentity) -> Result<(), GateRefusal> {
    let result = gate(new_app, own);
    let name = new_app.file_name().unwrap_or_default().to_string_lossy();
    match result {
        Ok(()) => {}
        Err(GateRefusal::NoOwnTeam) => {
            log::warn!("update install disabled for {name}: NoOwnTeam (running app has no team)")
        }
        Err(code) => log::error!("update install gate refused {name}: {code:?}"),
    }
    result
}

fn gate(new_app: &Path, own: &OwnIdentity) -> Result<(), GateRefusal> {
    let Some(own_team) = own.team.as_deref() else {
        return Err(GateRefusal::NoOwnTeam);
    };
    let info = validated_info(new_app)?;
    let bundle_id = info
        .find(unsafe { kSecCodeInfoPList })
        .and_then(|plist| plist.downcast::<CFDictionary>())
        .and_then(|plist| plist_string(&plist, "CFBundleIdentifier"));
    if bundle_id.as_deref() != Some(own.bundle_id.as_str()) {
        return Err(GateRefusal::BundleIdMismatch);
    }
    // The CodeDirectory's team id is self-asserted (an ad-hoc signature can claim
    // any team), so the team is proven with an Apple-anchored requirement on the
    // leaf certificate; the plain comparison is only a fast early refusal.
    let team_requirement = apple_team_requirement(own_team)?;
    if team_of(&info).as_deref() != Some(own_team) || !satisfies(new_app, &team_requirement)? {
        return Err(GateRefusal::TeamMismatch);
    }
    let helper_path = new_app.join("Contents/Helpers/mnema-cli.app");
    let helper = validated_info(&helper_path)?;
    if team_of(&helper).as_deref() != Some(own_team) || !satisfies(&helper_path, &team_requirement)?
    {
        return Err(GateRefusal::HelperTeamMismatch);
    }
    Ok(())
}

/// `anchor apple generic` = a certificate chain issued by Apple (Developer ID or
/// Apple Development); `leaf[subject.OU]` = the team that certificate belongs to.
fn apple_team_requirement(team: &str) -> Result<SecRequirement, GateRefusal> {
    // Team ids are 10 uppercase alphanumerics; refuse anything that could alter
    // the requirement text.
    if team.is_empty() || !team.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(GateRefusal::NoOwnTeam);
    }
    format!("anchor apple generic and certificate leaf[subject.OU] = \"{team}\"")
        .parse()
        .map_err(|_| GateRefusal::Unreadable)
}

/// Whether already-validated code at `path` also meets `requirement`.
fn satisfies(path: &Path, requirement: &SecRequirement) -> Result<bool, GateRefusal> {
    let url = CFURL::from_path(path, true).ok_or(GateRefusal::Unreadable)?;
    let code = SecStaticCode::from_path(&url, Flags::NONE).map_err(|_| GateRefusal::Unreadable)?;
    let flags = Flags::CHECK_ALL_ARCHITECTURES | Flags::CHECK_NESTED_CODE | Flags::STRICT_VALIDATE;
    Ok(code.check_validity(flags, requirement).is_ok())
}

/// Strict static validation, then the code's signing information.
fn validated_info(path: &Path) -> Result<CFDictionary<CFString, CFType>, GateRefusal> {
    let url = CFURL::from_path(path, true).ok_or(GateRefusal::Unreadable)?;
    let code = SecStaticCode::from_path(&url, Flags::NONE).map_err(|_| GateRefusal::Unreadable)?;
    let flags = kSecCSCheckAllArchitectures | kSecCSCheckNestedCode | kSecCSStrictValidate;
    // A null requirement: validity only; identity is compared below.
    let status =
        unsafe { SecStaticCodeCheckValidity(code.as_concrete_TypeRef(), flags, ptr::null_mut()) };
    if status != 0 {
        return Err(GateRefusal::InvalidSignature);
    }
    signing_info(code.as_concrete_TypeRef()).map_err(|_| GateRefusal::Unreadable)
}

fn team_of(info: &CFDictionary<CFString, CFType>) -> Option<String> {
    info.find(unsafe { kSecCodeInfoTeamIdentifier })
        .and_then(|v| v.downcast::<CFString>())
        .map(|s| s.to_string())
}

fn plist_string(plist: &CFDictionary, key: &'static str) -> Option<String> {
    let key = CFString::from_static_string(key);
    plist
        .find(key.as_CFTypeRef())
        .map(|v| unsafe { CFType::wrap_under_get_rule(*v) })
        .and_then(|v| v.downcast::<CFString>())
        .map(|s| s.to_string())
}

/// `SecCodeCopySigningInformation`; accepts a `SecCodeRef` too, per Apple's docs.
fn signing_info(code: SecStaticCodeRef) -> Result<CFDictionary<CFString, CFType>, OSStatus> {
    let mut info: CFDictionaryRef = ptr::null();
    let status =
        unsafe { SecCodeCopySigningInformation(code, kSecCSSigningInformation, &mut info) };
    if status != 0 || info.is_null() {
        return Err(status);
    }
    Ok(unsafe { CFDictionary::wrap_under_create_rule(info) })
}

// Not bound by security-framework-sys 2.17.
#[allow(non_upper_case_globals)]
const kSecCSSigningInformation: SecCSFlags = 1 << 1;

#[link(name = "Security", kind = "framework")]
extern "C" {
    static kSecCodeInfoTeamIdentifier: CFStringRef;
    static kSecCodeInfoPList: CFStringRef;
    fn SecCodeCopySigningInformation(
        code: SecStaticCodeRef,
        flags: SecCSFlags,
        information: *mut CFDictionaryRef,
    ) -> OSStatus;
}

#[cfg(test)]
mod tests;
