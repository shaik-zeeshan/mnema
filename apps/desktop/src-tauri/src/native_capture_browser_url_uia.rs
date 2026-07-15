//! Reads the active-tab URL of the foreground browser window via **UI Automation**
//! (UIA) — the third Browser URL Strategy, Windows-only and permission-free
//! (ADR 0044). It joins the macOS AppleScript and Accessibility strategies; the
//! three are never unified across platforms. Same-integrity UIA reads need no
//! consent prompt, manifest change, or elevation, so there is no Windows analog
//! of the macOS Accessibility grant UX.
//!
//! Reads are **engine-dialected**, the direct analog of the macOS strategies:
//!
//! - **Chromium** (Chrome/Edge/Brave/Vivaldi/Opera/Arc, incl. Helium-as-chrome.exe):
//!   `ElementFromHandle(hwnd)` → `FindFirst(Document)` → ValuePattern value. The
//!   renderer's accessibility is **dormant** until the first UIA client connects:
//!   the first read finds no `Document`, and the act of connecting wakes it, so a
//!   re-read milliseconds later succeeds. This is the macOS Gecko-dormancy case,
//!   mirrored here for Chromium, and is why a cold-poll loop wraps the read
//!   ([`COLD_READ_POLL_BUDGET`] total, [`COLD_READ_POLL_STEP`] steps).
//! - **Gecko** (Firefox/Zen/LibreWolf/Waterfox/Floorp): `GetFocusedElement` →
//!   climb the control-view ancestors to the enclosing (outermost) `Document` →
//!   ValuePattern value. This is the direct analog of the macOS climb to the
//!   outermost `AXWebArea` and is correct under Zen split view. Gecko's tree is
//!   always live, so it never cold-polls.
//!
//! The **no-guess invariant** carries over from macOS (ADR 0039): when the focused
//! element resolves to no document (e.g. the user is typing in the address bar)
//! we yield no URL for that tick rather than scan offscreen documents or read the
//! (lossy) address bar. Preferring no URL over a guessed one is a cross-platform
//! invariant of every Browser URL Strategy.
//!
//! **Bounded cost (mirrors the AX reader).** Two independent bounds keep a
//! hung/slow browser from stalling a read:
//!   1. `IUIAutomation2` connection + transaction timeouts ([`UIA_CALL_TIMEOUT_MS`])
//!      bound any single cross-process UIA call — the analog of the macOS
//!      `AXUIElementSetMessagingTimeout`. If the `IUIAutomation2` cast fails on some
//!      OS we fall back to the base `IUIAutomation` without timeouts.
//!   2. A wall-clock budget ([`READ_ATTEMPT_BUDGET`]) bounds one whole read attempt,
//!      since one attempt issues many cross-process calls (`ElementFromHandle`,
//!      `FindFirst`, `GetFocusedElement`, each `GetParentElement`/`CurrentControlType`,
//!      `GetCurrentPattern`, `CurrentValue`). The deadline is checked BEFORE every
//!      such call so that once the budget is spent at most ONE more call can already
//!      be in flight. The Gecko climb is additionally capped at [`MAX_PARENT_HOPS`].
//! Traversal shapes are chosen so page weight cannot matter: `FindFirst(Document)`
//! early-exits at a shallow fixed position, the climb costs one call per ancestor
//! hop, and nothing enumerates tabs or page content.
//!
//! **Live reads, no probe cache.** UIA reads cost ~1–7 ms, so Windows does not
//! adopt the title-gated `BrowserUrlProbeCache` (a cost workaround for macOS
//! `osascript`/AX reads). Reading live on every metadata refresh is strictly
//! fresher (SPA navigations that change the URL without the title are caught every
//! tick). These live reads always run OFF every capture lock: the 1 s metadata
//! poll thread and the debounced foreground-listener thread — never a
//! capture-lifecycle path holding `NativeCaptureState`.
//!
//! **COM.** Both reading threads live for the whole process, so COM is
//! MTA-initialized once per thread via a `thread_local!` guard; `S_FALSE`
//! (already initialized) and `RPC_E_CHANGED_MODE` (thread already in a different
//! apartment) are tolerated and we never `CoUninitialize`.
//!
//! This module returns the RAW URL string; sanitization (`sanitize_url`) and
//! gating happen in the caller (slice 3).

use capture_metadata::BrowserEngine;
use std::cell::Cell;
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomation2, IUIAutomationElement, IUIAutomationValuePattern,
    TreeScope_Subtree, UIA_ControlTypePropertyId, UIA_DocumentControlTypeId, UIA_ValuePatternId,
};

/// Total budget for waking a dormant Chromium renderer accessibility tree on the
/// first read (the macOS Gecko cold-poll analog).
const COLD_READ_POLL_BUDGET: Duration = Duration::from_millis(500);
/// Step between polls while waking the renderer a11y.
const COLD_READ_POLL_STEP: Duration = Duration::from_millis(50);
/// Wall-clock budget for a SINGLE read attempt. The per-call UIA timeout below
/// only bounds one cross-process call, but one attempt issues several; this caps
/// the whole attempt so a slow-but-responding browser can't stall a read. Checked
/// before every UIA call so at most one call can be in flight past the budget.
const READ_ATTEMPT_BUDGET: Duration = Duration::from_millis(400);
/// Connection + transaction timeout for a single cross-process UIA call, in
/// milliseconds — the analog of the macOS `AXUIElementSetMessagingTimeout`. Only
/// applied when the `IUIAutomation2` cast succeeds.
const UIA_CALL_TIMEOUT_MS: u32 = 500;
/// Upper bound on control-view parent hops while climbing to the outermost
/// `Document` (Gecko).
const MAX_PARENT_HOPS: u32 = 16;

/// Outcome of a single UIA read attempt (mirrors the macOS AX reader).
enum ReadOutcome {
    /// Found a URL on the (Chromium) Document or the (Gecko) outermost Document.
    Url(String),
    /// The Chromium renderer a11y is dormant (no `Document` yet) — worth polling;
    /// connecting wakes it. Gecko never returns this.
    Dormant,
    /// Focus resolves to no web document (e.g. the address bar) — don't poll,
    /// there is no URL to read this tick (the no-guess invariant).
    NoWeb,
}

thread_local! {
    /// Per-thread latch so `CoInitializeEx` runs at most once per calling thread.
    static COM_INITIALIZED: Cell<bool> = const { Cell::new(false) };
}

/// Reads the active-tab URL of the foreground browser window `hwnd` (engine
/// `engine`) via UI Automation, or `None` if no URL can be read within budget.
/// `hwnd` is the raw foreground HWND as an `isize` (cast from the Win32 handle by
/// the caller); `pid` is retained for parity with the macOS AX reader / diagnostics.
/// Returns the RAW URL — the caller sanitizes via `sanitize_url`.
pub fn read_active_tab_url(hwnd: isize, pid: u32, engine: BrowserEngine) -> Option<String> {
    // Reserved for parity with the macOS AX reader / future diagnostics.
    let _ = pid;

    ensure_com_initialized();
    let automation = create_automation()?;
    let hwnd = HWND(hwnd as *mut core::ffi::c_void);

    match engine {
        // Chromium: cold-poll for a dormant renderer, exactly like the macOS AX
        // reader. Each attempt carries its own wall-clock deadline.
        BrowserEngine::Chromium => {
            let started = Instant::now();
            loop {
                let attempt_deadline = Instant::now() + READ_ATTEMPT_BUDGET;
                match read_chromium(&automation, hwnd, attempt_deadline) {
                    ReadOutcome::Url(url) => return Some(url),
                    // Renderer a11y cold — keep polling; connecting wakes it.
                    ReadOutcome::Dormant => {}
                    // Chrome focused (e.g. address bar) — DON'T poll.
                    ReadOutcome::NoWeb => return None,
                }
                if started.elapsed() >= COLD_READ_POLL_BUDGET {
                    return None;
                }
                std::thread::sleep(COLD_READ_POLL_STEP);
            }
        }
        // Gecko: always live, never cold-polls — a single bounded attempt.
        BrowserEngine::Gecko => {
            let attempt_deadline = Instant::now() + READ_ATTEMPT_BUDGET;
            match read_gecko(&automation, attempt_deadline) {
                ReadOutcome::Url(url) => Some(url),
                ReadOutcome::Dormant | ReadOutcome::NoWeb => None,
            }
        }
    }
}

/// Initializes COM (MTA) at most once per calling thread. Tolerates `S_FALSE`
/// (already initialized) and `RPC_E_CHANGED_MODE` (thread already in a different
/// apartment) by proceeding regardless, and never calls `CoUninitialize`.
fn ensure_com_initialized() {
    COM_INITIALIZED.with(|initialized| {
        if initialized.get() {
            return;
        }
        // Proceed regardless of the HRESULT: S_OK / S_FALSE / RPC_E_CHANGED_MODE
        // all leave this thread able to make UIA calls.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        initialized.set(true);
    });
}

/// Creates the `IUIAutomation` root object and, when available, bounds every
/// single cross-process call via `IUIAutomation2` timeouts. Falls back to the
/// base object without timeouts if the `IUIAutomation2` cast fails on some OS.
fn create_automation() -> Option<IUIAutomation> {
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok()?;
    if let Ok(automation2) = automation.cast::<IUIAutomation2>() {
        unsafe {
            let _ = automation2.SetConnectionTimeout(UIA_CALL_TIMEOUT_MS);
            let _ = automation2.SetTransactionTimeout(UIA_CALL_TIMEOUT_MS);
        }
    }
    Some(automation)
}

/// Chromium read: `ElementFromHandle` → `FindFirst(Document)` → ValuePattern.
/// A missing `Document` (renderer a11y still dormant) is [`ReadOutcome::Dormant`]
/// — the connection itself wakes it, so the caller polls. The deadline is checked
/// before every cross-process call.
fn read_chromium(automation: &IUIAutomation, hwnd: HWND, deadline: Instant) -> ReadOutcome {
    if Instant::now() >= deadline {
        return ReadOutcome::Dormant;
    }
    let Ok(root) = (unsafe { automation.ElementFromHandle(hwnd) }) else {
        return ReadOutcome::Dormant;
    };

    // Condition build is local (no cross-process call); still cheap to guard.
    let Ok(condition) = (unsafe {
        automation.CreatePropertyCondition(
            UIA_ControlTypePropertyId,
            &VARIANT::from(UIA_DocumentControlTypeId.0),
        )
    }) else {
        return ReadOutcome::Dormant;
    };

    if Instant::now() >= deadline {
        return ReadOutcome::Dormant;
    }
    // FindFirst early-exits shallow. Not-found / Err ⇒ renderer a11y dormant;
    // connecting wakes it, so this is worth polling.
    let Ok(document) = (unsafe { root.FindFirst(TreeScope_Subtree, &condition) }) else {
        return ReadOutcome::Dormant;
    };

    match read_value(&document, deadline) {
        Some(url) => ReadOutcome::Url(url),
        None => ReadOutcome::Dormant,
    }
}

/// Gecko read: `GetFocusedElement` → climb control-view ancestors to the OUTERMOST
/// `Document` → ValuePattern. Returns [`ReadOutcome::NoWeb`] when no Document is on
/// the focus→root chain (the no-guess invariant); never returns
/// [`ReadOutcome::Dormant`]. The deadline is checked before every cross-process
/// call and the climb is capped at [`MAX_PARENT_HOPS`].
fn read_gecko(automation: &IUIAutomation, deadline: Instant) -> ReadOutcome {
    if Instant::now() >= deadline {
        return ReadOutcome::NoWeb;
    }
    let Ok(focused) = (unsafe { automation.GetFocusedElement() }) else {
        return ReadOutcome::NoWeb;
    };
    // ControlViewWalker itself is local; the GetParentElement hops are the
    // cross-process calls we bound below.
    let Ok(walker) = (unsafe { automation.ControlViewWalker() }) else {
        return ReadOutcome::NoWeb;
    };

    // Keep the OUTERMOST element whose control type is Document — the last
    // Document seen climbing toward the root (the top document, the direct analog
    // of AX's outermost AXWebArea).
    let mut outermost: Option<IUIAutomationElement> = None;
    let mut current = focused;
    let mut hops = 0u32;
    loop {
        if Instant::now() >= deadline {
            break;
        }
        if let Ok(control_type) = unsafe { current.CurrentControlType() } {
            if control_type.0 == UIA_DocumentControlTypeId.0 {
                outermost = Some(current.clone());
            }
        }
        if hops >= MAX_PARENT_HOPS {
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        // No parent (null / Err) ⇒ reached the root; stop.
        let Ok(parent) = (unsafe { walker.GetParentElement(&current) }) else {
            break;
        };
        current = parent;
        hops += 1;
    }

    match outermost {
        Some(document) => match read_value(&document, deadline) {
            Some(url) => ReadOutcome::Url(url),
            None => ReadOutcome::NoWeb,
        },
        // Focus in chrome (no document on the focus→root chain) — no guess.
        None => ReadOutcome::NoWeb,
    }
}

/// Reads an element's ValuePattern value as a trimmed, non-empty `String`, or
/// `None` if the element has no ValuePattern or the value is empty. The deadline
/// is checked before each cross-process call.
fn read_value(element: &IUIAutomationElement, deadline: Instant) -> Option<String> {
    if Instant::now() >= deadline {
        return None;
    }
    let pattern = unsafe { element.GetCurrentPattern(UIA_ValuePatternId) }.ok()?;
    let value_pattern = pattern.cast::<IUIAutomationValuePattern>().ok()?;
    if Instant::now() >= deadline {
        return None;
    }
    let value = unsafe { value_pattern.CurrentValue() }.ok()?.to_string();
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
