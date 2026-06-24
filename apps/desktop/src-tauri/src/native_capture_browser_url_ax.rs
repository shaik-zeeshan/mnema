//! Reads the active-tab URL of a Gecko browser (Firefox/Zen) via the macOS
//! Accessibility (AX) API.
//!
//! Gecko has no scriptable URL surface, so we cannot read it via AppleScript the
//! way Chromium/WebKit browsers are read. Instead we read the `AXURL` attribute
//! off the *focused web area* of the browser's accessibility tree. We climb the
//! focused element's parent chain and keep the URL of the outermost `AXWebArea`
//! (the top document, never an iframe). We never scan windows or the address bar
//! for a URL — if no focused web area is found we return `None` rather than
//! guess (preferring no URL over a wrong one is the correctness core).
//!
//! Requires the Accessibility permission. The reader gates on
//! [`accessibility_trusted`]; the one-time first-sighting prompt
//! ([`maybe_prompt_on_gecko_frontmost`]) is fired by the metadata dispatch when
//! a Gecko browser is frontmost and trust is missing.
//!
//! A 0.5s AX messaging timeout bounds a hung browser so a stuck app can't stall
//! the metadata tick. The first read may find the a11y engine dormant (cold);
//! we poll for up to 500ms (50ms steps) to wake it — measured cold→live is
//! ~100–150ms. The whole read runs on the metadata refresh tick's background
//! thread, so the poll/sleep is acceptable and these AX reads are fine off the
//! main thread.

use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::url::CFURL;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

type AXUIElementRef = CFTypeRef; // an opaque, reference-counted CF object
type AXError = i32; // kAXErrorSuccess == 0

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8; // C `Boolean` (unsigned char); 0 = not trusted
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef; // Create rule: caller owns (+1)
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError; // Copy rule: out-value is +1, caller owns
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> AXError;
    // Trust check that can also raise the system prompt (and add Mnema to the
    // Accessibility list) when the options dict sets `kAXTrustedCheckOptionPrompt`.
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
}

/// Whether Mnema currently holds the macOS Accessibility (AX) permission. This
/// is the single trust check the reader and the command surface share.
pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Asks macOS for the Accessibility permission, raising the system prompt that
/// adds Mnema to the Accessibility list and points the user at System Settings.
/// Returns whether trust is held after the call (immediate; the grant itself is
/// asynchronous — the user flips the toggle in System Settings later). macOS
/// dedupes the dialog itself; [`maybe_prompt_on_gecko_frontmost`] additionally
/// avoids calling this every metadata tick.
pub fn request_accessibility_with_prompt() -> bool {
    let key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
    let options =
        CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFBoolean::true_value().as_CFType())]);
    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) != 0 }
}

/// Process-global latch so the first-sighting prompt fires at most once per
/// process run.
static PROMPTED: AtomicBool = AtomicBool::new(false);

/// Fires the Accessibility system prompt at most once per process run, and only
/// when trust is not already held. The first time a Gecko browser is frontmost
/// while browser-URL capture is enabled and trust is missing, the user is
/// prompted once; thereafter reads quietly return `None` (no URL, no crash, no
/// nag-loop) until the permission is granted.
pub fn maybe_prompt_on_gecko_frontmost() {
    if accessibility_trusted() {
        return;
    }
    // Already fired this run — don't nag.
    if PROMPTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = request_accessibility_with_prompt();
}

/// Total budget for waking a dormant a11y engine on the first read.
const COLD_READ_POLL_BUDGET: Duration = Duration::from_millis(500);
/// Step between polls while waking the a11y engine.
const COLD_READ_POLL_STEP: Duration = Duration::from_millis(50);
/// Per-message AX timeout; bounds a hung browser.
const AX_MESSAGING_TIMEOUT_SECS: f32 = 0.5;
/// Upper bound on parent-chain hops while searching for the web area.
const MAX_PARENT_HOPS: u32 = 16;

/// Outcome of a single focused-web-area read attempt.
enum ReadOutcome {
    /// Found a URL on the focused web area.
    Url(String),
    /// The a11y engine is cold (no focused element yet, or focus is still the
    /// window) — worth polling.
    Dormant,
    /// A real non-web element is focused (e.g. the address bar) — don't poll;
    /// there is no web-area URL to read.
    NoWeb,
}

/// Reads the active-tab URL of the Gecko browser running as `pid` via the
/// Accessibility API, or `None` if Accessibility is not granted, the browser
/// exposes no focused web area, or the read times out.
pub fn read_active_tab_url(pid: i32) -> Option<String> {
    // One shared trust check; the first-sighting prompt is fired by the
    // dispatch (`maybe_prompt_on_gecko_frontmost`) before this read runs.
    if !accessibility_trusted() {
        return None;
    }

    let app = unsafe { AXUIElementCreateApplication(pid) };
    if app.is_null() {
        return None;
    }
    let app = unsafe { CFType::wrap_under_create_rule(app) };

    // Bound a hung browser; ignore the AXError.
    unsafe {
        AXUIElementSetMessagingTimeout(app.as_CFTypeRef(), AX_MESSAGING_TIMEOUT_SECS);
    }

    // First read; if the a11y engine is dormant (cold), poll briefly.
    let started = Instant::now();
    loop {
        match read_focused_outermost_url(app.as_CFTypeRef()) {
            ReadOutcome::Url(url) => return Some(url),
            // a11y engine cold — keep polling.
            ReadOutcome::Dormant => {}
            // chrome focused (e.g. address bar) — DON'T poll.
            ReadOutcome::NoWeb => return None,
        }
        if started.elapsed() >= COLD_READ_POLL_BUDGET {
            return None;
        }
        std::thread::sleep(COLD_READ_POLL_STEP);
    }
}

/// Reads the URL of the outermost `AXWebArea` on the focused element's parent
/// chain. Returns [`ReadOutcome::Dormant`] when the a11y engine looks cold and
/// [`ReadOutcome::NoWeb`] when a real non-web element is focused.
fn read_focused_outermost_url(app: AXUIElementRef) -> ReadOutcome {
    // No focused element yet = the a11y engine is cold.
    let Some(focused) = copy_attribute(app, "AXFocusedUIElement") else {
        return ReadOutcome::Dormant;
    };
    let role = string_attribute(focused.as_CFTypeRef(), "AXRole").unwrap_or_default();

    // Climb the parent chain from `focused`, collecting the URL of every
    // AXWebArea seen. Keep the OUTERMOST one (the last web-area URL found while
    // climbing up toward the window) — that is the top document, not an iframe.
    let mut outermost: Option<String> = None;
    let mut cur = Some(focused);
    let mut hops = 0;
    while let Some(el) = cur {
        if hops >= MAX_PARENT_HOPS {
            break;
        }
        let el_role = string_attribute(el.as_CFTypeRef(), "AXRole").unwrap_or_default();
        if el_role == "AXWebArea" {
            if let Some(url) = url_attribute(el.as_CFTypeRef(), "AXURL") {
                outermost = Some(url);
            }
        }
        cur = copy_attribute(el.as_CFTypeRef(), "AXParent");
        hops += 1;
    }

    if let Some(url) = outermost {
        return ReadOutcome::Url(url);
    }

    // No web area on the focus chain.
    if role == "AXWindow" {
        // Cold a11y engine: focus is still the window.
        return ReadOutcome::Dormant;
    }
    // A real non-web element is focused (chrome) — no guess.
    ReadOutcome::NoWeb
}

/// Copies an AX attribute as a raw CF object under the create rule, so Drop
/// releases it. `None` when the attribute is missing or the call errors.
fn copy_attribute(element: AXUIElementRef, name: &str) -> Option<CFType> {
    let attr = CFString::new(name);
    let mut value: CFTypeRef = std::ptr::null();
    let err =
        unsafe { AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value) };
    if err != 0 || value.is_null() {
        return None;
    }
    Some(unsafe { CFType::wrap_under_create_rule(value) })
}

/// Reads a string-valued AX attribute (e.g. `AXRole`). Returns `None` when the
/// attribute is missing or is not a CFString.
fn string_attribute(element: AXUIElementRef, name: &str) -> Option<String> {
    let value = copy_attribute(element, name)?;
    if value.type_of() != CFString::type_id() {
        return None;
    }
    value
        .downcast::<CFString>()
        .map(|string| string.to_string())
}

/// Reads `AXURL`, which may come back as a CFURL (most common) or a CFString.
/// Returns the non-empty, trimmed absolute string.
fn url_attribute(element: AXUIElementRef, name: &str) -> Option<String> {
    let value = copy_attribute(element, name)?;
    let url = if value.type_of() == CFString::type_id() {
        value
            .downcast::<CFString>()
            .map(|string| string.to_string())
    } else if value.type_of() == CFURL::type_id() {
        value
            .downcast::<CFURL>()
            .map(|url| url.get_string().to_string())
    } else {
        None
    }?;
    let trimmed = url.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
