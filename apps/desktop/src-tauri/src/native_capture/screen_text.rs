#[cfg(target_os = "macos")]
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use capture_types::{CaptureSources, RecordingSettings};
use serde::{Deserialize, Serialize};
use tauri::Manager;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScreenTextSnapshot {
    pub normalized_text: String,
    pub captured_at_unix_ms: i64,
    pub source_app_bundle_id: Option<String>,
    pub source_app_name: Option<String>,
    pub source_window_title: Option<String>,
    pub source_window_id: Option<i64>,
    pub snapshot_age_ms: i64,
    pub node_count: Option<u32>,
    pub truncated: bool,
    pub timed_out: bool,
    #[serde(default)]
    pub text_clipped: bool,
    #[serde(default)]
    pub content_scope: AccessibilitySnapshotScope,
    #[serde(default)]
    pub content_root_role: Option<String>,
    #[serde(default)]
    pub content_root_subrole: Option<String>,
    #[serde(default)]
    pub content_root_strategy: Option<String>,
    pub refresh_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AccessibilitySnapshotScope {
    PrimaryContent,
    GenericVisibleRoot,
    ChromeOnly,
}

impl Default for AccessibilitySnapshotScope {
    fn default() -> Self {
        Self::GenericVisibleRoot
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScreenTextSnapshotEvaluation {
    pub usable: bool,
    pub rejection_reason: Option<ScreenTextSnapshotRejectionReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum ScreenTextSnapshotRejectionReason {
    TimedOut,
    StructurallyTruncated,
    TooOld,
    FromFuture,
    TooShort,
    SourceAppMismatch,
    AppRequiresOcrFallback,
    ChromeOnly,
}

impl ScreenTextSnapshotRejectionReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TimedOut => "timed_out",
            Self::StructurallyTruncated => "structurally_truncated",
            Self::TooOld => "too_old",
            Self::FromFuture => "from_future",
            Self::TooShort => "too_short",
            Self::SourceAppMismatch => "source_app_mismatch",
            Self::AppRequiresOcrFallback => "app_requires_ocr_fallback",
            Self::ChromeOnly => "chrome_only",
        }
    }
}

pub type ScreenTextSnapshotProvider =
    Arc<dyn Fn() -> Option<ScreenTextSnapshot> + Send + Sync + 'static>;

#[derive(Default)]
pub struct ScreenTextSnapshotState {
    runtime: Mutex<ScreenTextRuntimeState>,
}

#[derive(Default)]
struct ScreenTextRuntimeState {
    latest_snapshot: Option<ScreenTextSnapshot>,
    stop: Option<Arc<AtomicBool>>,
    worker: Option<thread::JoinHandle<()>>,
}

pub(crate) fn screen_text_snapshot_provider(
    app_handle: &tauri::AppHandle,
) -> ScreenTextSnapshotProvider {
    let app_handle = app_handle.clone();
    Arc::new(move || {
        app_handle
            .try_state::<ScreenTextSnapshotState>()
            .and_then(|state| state.latest_snapshot())
            .map(|mut snapshot| {
                snapshot.snapshot_age_ms = now_unix_ms_i64() - snapshot.captured_at_unix_ms;
                snapshot
            })
    })
}

impl ScreenTextSnapshotState {
    fn latest_snapshot(&self) -> Option<ScreenTextSnapshot> {
        self.runtime
            .lock()
            .ok()
            .and_then(|runtime| runtime.latest_snapshot.clone())
    }

    fn set_latest_snapshot(&self, snapshot: Option<ScreenTextSnapshot>) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.latest_snapshot = snapshot;
        }
    }
}

pub(crate) fn start_or_stop_for_recording(
    app_handle: &tauri::AppHandle,
    settings: &RecordingSettings,
    sources: &CaptureSources,
) {
    let enabled = sources.screen
        && settings.screen_text_extraction.enabled
        && settings.screen_text_extraction.accessibility_enabled;

    if enabled {
        start_runtime(app_handle);
    } else {
        stop_runtime(app_handle);
    }
}

pub(crate) fn stop_runtime(app_handle: &tauri::AppHandle) {
    let Some(state) = app_handle.try_state::<ScreenTextSnapshotState>() else {
        return;
    };

    let worker = {
        let Ok(mut runtime) = state.runtime.lock() else {
            return;
        };
        if let Some(stop) = runtime.stop.take() {
            stop.store(true, Ordering::SeqCst);
        }
        runtime.latest_snapshot = None;
        runtime.worker.take()
    };

    if let Some(worker) = worker {
        let _ = worker.join();
    }
}

fn start_runtime(app_handle: &tauri::AppHandle) {
    let Some(state) = app_handle.try_state::<ScreenTextSnapshotState>() else {
        return;
    };

    let Ok(mut runtime) = state.runtime.lock() else {
        return;
    };
    if runtime.worker.is_some() {
        return;
    }

    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_worker = Arc::clone(&stop);
    let app_handle_for_worker = app_handle.clone();
    let worker = thread::Builder::new()
        .name("mnema-screen-text-ax".to_string())
        .spawn(move || {
            #[cfg(target_os = "macos")]
            let mut observer = macos_ax::ObserverRuntime::new();
            #[cfg(target_os = "macos")]
            let mut last_poll = Instant::now() - Duration::from_millis(1_000);

            while !stop_for_worker.load(Ordering::SeqCst) {
                #[cfg(target_os = "macos")]
                {
                    let reason = observer.next_refresh_reason(&stop_for_worker, &mut last_poll);
                    if let Some(reason) = reason {
                        let trace_enabled =
                            accessibility_snapshot_trace_enabled(&app_handle_for_worker);
                        let snapshot =
                            collect_frontmost_accessibility_snapshot(&reason, trace_enabled);
                        if let Some(state) =
                            app_handle_for_worker.try_state::<ScreenTextSnapshotState>()
                        {
                            state.set_latest_snapshot(snapshot);
                        }
                    }
                }

                #[cfg(not(target_os = "macos"))]
                {
                    thread::sleep(Duration::from_millis(1_000));
                }
            }

            if let Some(state) = app_handle_for_worker.try_state::<ScreenTextSnapshotState>() {
                state.set_latest_snapshot(None);
            }
        });

    let Ok(worker) = worker else {
        return;
    };

    runtime.stop = Some(stop);
    runtime.worker = Some(worker);
}

fn collect_frontmost_accessibility_snapshot(
    refresh_reason: &str,
    trace_enabled: bool,
) -> Option<ScreenTextSnapshot> {
    #[cfg(target_os = "macos")]
    {
        macos_ax::collect_frontmost_accessibility_snapshot(refresh_reason, trace_enabled)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (refresh_reason, trace_enabled);
        None
    }
}

pub(crate) struct NormalizedAccessibilityText {
    pub text: String,
    pub text_clipped: bool,
}

pub(crate) fn normalize_accessibility_text(strings: &[String]) -> NormalizedAccessibilityText {
    let mut parts = Vec::new();
    let mut previous: Option<String> = None;

    for value in strings {
        let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
        let normalized = normalized.trim();
        if normalized.is_empty() {
            continue;
        }
        if previous.as_deref() == Some(normalized) {
            continue;
        }
        previous = Some(normalized.to_string());
        parts.push(normalized.to_string());
    }

    let joined = parts.join(" ");
    let mut clipped = String::new();
    let mut text_clipped = false;
    for (index, ch) in joined.chars().enumerate() {
        if index >= 20_000 {
            text_clipped = true;
            break;
        }
        clipped.push(ch);
    }
    NormalizedAccessibilityText {
        text: clipped,
        text_clipped,
    }
}

pub(crate) fn accessibility_snapshot_trace_enabled(app_handle: &tauri::AppHandle) -> bool {
    cfg!(feature = "accessibility-snapshot-trace")
        && app_handle
            .try_state::<crate::native_capture::RecordingSettingsState>()
            .is_some_and(|state| {
                crate::native_capture::settings::current_native_capture_debug_logging_enabled(
                    state.inner(),
                )
            })
}

pub(crate) fn role_is_primary_content(role: &str) -> bool {
    matches!(
        role,
        "AXWebArea"
            | "AXDocument"
            | "AXTextArea"
            | "AXTextField"
            | "AXTable"
            | "AXOutline"
            | "AXList"
            | "AXCollection"
    )
}

pub(crate) fn role_is_high_confidence_primary_content(role: &str) -> bool {
    role == "AXWebArea"
}

pub(crate) fn role_is_conditional_content_container(role: &str) -> bool {
    role == "AXScrollArea"
}

pub(crate) fn role_is_chrome(role: &str) -> bool {
    matches!(
        role,
        "AXToolbar" | "AXTabGroup" | "AXMenuBar" | "AXMenu" | "AXMenuItem"
    )
}

pub(crate) fn evaluate_snapshot_for_frame(
    snapshot: &ScreenTextSnapshot,
    frame_metadata: Option<&capture_metadata::FrameMetadataSnapshot>,
) -> ScreenTextSnapshotEvaluation {
    let reject = |reason| ScreenTextSnapshotEvaluation {
        usable: false,
        rejection_reason: Some(reason),
    };

    if snapshot.timed_out {
        return reject(ScreenTextSnapshotRejectionReason::TimedOut);
    }
    if snapshot.truncated {
        return reject(ScreenTextSnapshotRejectionReason::StructurallyTruncated);
    }
    if snapshot.content_scope == AccessibilitySnapshotScope::ChromeOnly {
        return reject(ScreenTextSnapshotRejectionReason::ChromeOnly);
    }
    if snapshot.snapshot_age_ms > 1_500 {
        return reject(ScreenTextSnapshotRejectionReason::TooOld);
    }
    if snapshot.snapshot_age_ms < -250 {
        return reject(ScreenTextSnapshotRejectionReason::FromFuture);
    }
    if snapshot.normalized_text.trim().chars().count() < 40 {
        return reject(ScreenTextSnapshotRejectionReason::TooShort);
    }
    if let (Some(expected), Some(actual)) = (
        frame_metadata.and_then(|metadata| metadata.app_bundle_id.as_deref()),
        snapshot.source_app_bundle_id.as_deref(),
    ) {
        if expected != actual {
            return reject(ScreenTextSnapshotRejectionReason::SourceAppMismatch);
        }
    }
    if snapshot
        .source_app_bundle_id
        .as_deref()
        .is_some_and(app_requires_ocr_fallback)
    {
        return reject(ScreenTextSnapshotRejectionReason::AppRequiresOcrFallback);
    }

    ScreenTextSnapshotEvaluation {
        usable: true,
        rejection_reason: None,
    }
}

#[allow(dead_code)]
pub(crate) fn snapshot_usable_for_frame(
    snapshot: &ScreenTextSnapshot,
    frame_metadata: Option<&capture_metadata::FrameMetadataSnapshot>,
) -> bool {
    evaluate_snapshot_for_frame(snapshot, frame_metadata).usable
}

pub(crate) fn refresh_snapshot_age(snapshot: &mut ScreenTextSnapshot) {
    snapshot.snapshot_age_ms = now_unix_ms_i64() - snapshot.captured_at_unix_ms;
}

fn app_requires_ocr_fallback(bundle_id: &str) -> bool {
    matches!(
        bundle_id,
        "com.microsoft.rdc.macos"
            | "com.apple.ScreenSharing"
            | "com.valvesoftware.steam"
            | "com.blizzard.battle.net"
            | "com.adobe.Photoshop"
            | "com.adobe.Illustrator"
            | "com.adobe.Acrobat.Pro"
    )
}

fn now_unix_ms_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[cfg(target_os = "macos")]
mod macos_ax {
    use super::*;
    use core_foundation_sys::{
        array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex},
        base::{Boolean, CFGetTypeID, CFRelease, CFRetain, CFTypeID, CFTypeRef},
        number::{kCFNumberSInt64Type, CFNumberGetTypeID, CFNumberGetValue},
        runloop::{
            kCFRunLoopDefaultMode, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRef,
            CFRunLoopRemoveSource, CFRunLoopRunInMode, CFRunLoopSourceRef,
        },
        string::{
            kCFStringEncodingUTF8, CFStringCreateWithCString, CFStringGetCString,
            CFStringGetLength, CFStringGetMaximumSizeForEncoding, CFStringGetTypeID, CFStringRef,
        },
    };
    use objc::{class, msg_send, runtime::Object, sel, sel_impl};
    use std::sync::mpsc;

    type AXError = i32;
    type AXUIElementRef = *const std::ffi::c_void;
    type AXObserverRef = *const std::ffi::c_void;
    type AXObserverCallback = extern "C" fn(
        observer: AXObserverRef,
        element: AXUIElementRef,
        notification: CFStringRef,
        refcon: *mut std::ffi::c_void,
    );

    const K_AX_ERROR_SUCCESS: AXError = 0;
    const AX_TIMEOUT_MS: u128 = 150;
    const AX_NODE_CAP: usize = 500;
    const AX_DEPTH_CAP: usize = 20;
    const AX_ROOT_SCAN_NODE_CAP: usize = 250;
    const AX_ROOT_SCAN_DEPTH_CAP: usize = 12;
    const AX_MIN_CONDITIONAL_ROOT_TEXT_CHARS: usize = 40;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> Boolean;
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> AXError;
        fn AXUIElementGetTypeID() -> CFTypeID;
        fn AXObserverCreate(
            application: i32,
            callback: AXObserverCallback,
            out_observer: *mut AXObserverRef,
        ) -> AXError;
        fn AXObserverAddNotification(
            observer: AXObserverRef,
            element: AXUIElementRef,
            notification: CFStringRef,
            refcon: *mut std::ffi::c_void,
        ) -> AXError;
        fn AXObserverGetRunLoopSource(observer: AXObserverRef) -> CFRunLoopSourceRef;
    }

    struct CfString(*const std::ffi::c_void);

    impl CfString {
        fn new(value: &str) -> Option<Self> {
            let c = std::ffi::CString::new(value).ok()?;
            let ptr = unsafe {
                CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), kCFStringEncodingUTF8)
            };
            (!ptr.is_null()).then_some(Self(ptr as *const std::ffi::c_void))
        }

        fn as_string_ref(&self) -> CFStringRef {
            self.0 as CFStringRef
        }
    }

    impl Drop for CfString {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) };
        }
    }

    struct CfOwned(CFTypeRef);

    impl Clone for CfOwned {
        fn clone(&self) -> Self {
            let retained = unsafe { CFRetain(self.0) };
            Self(retained)
        }
    }

    impl Drop for CfOwned {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) };
        }
    }

    struct AxWalk {
        started: Instant,
        node_count: usize,
        truncated: bool,
        timed_out: bool,
        strings: Vec<String>,
        visited: HashSet<usize>,
    }

    pub(super) fn collect_frontmost_accessibility_snapshot(
        refresh_reason: &str,
        trace_enabled: bool,
    ) -> Option<ScreenTextSnapshot> {
        if unsafe { AXIsProcessTrusted() == 0 } {
            return None;
        }

        let app = frontmost_application()?;
        let app_element = unsafe { AXUIElementCreateApplication(app.pid) };
        if app_element.is_null() {
            return None;
        }
        let app_element = CfOwned(app_element as CFTypeRef);

        let roots = root_candidates(app_element.0);
        let root_refs = if roots.is_empty() {
            vec![app_element.0]
        } else {
            roots.iter().map(|root| root.0).collect()
        };
        let selected_root = select_content_root(&root_refs, Instant::now());
        let root_ref = selected_root
            .element
            .as_ref()
            .map(|element| element.0)
            .unwrap_or(app_element.0);

        let mut walk = AxWalk {
            started: Instant::now(),
            node_count: 0,
            truncated: false,
            timed_out: false,
            strings: Vec::new(),
            visited: HashSet::new(),
        };
        walk_element(root_ref, 0, &mut walk);

        let normalized = normalize_accessibility_text(&walk.strings);
        if normalized.text.is_empty() {
            return None;
        }

        if trace_enabled {
            crate::native_capture::debug_log::log(format!(
                "accessibility snapshot trace: refresh_reason={} selected_role={:?} selected_subrole={:?} strategy={:?} scope={:?} nodes={} truncated={} timed_out={} root_scan_truncated={} root_scan_timed_out={} text_len={} text_clipped={}",
                refresh_reason,
                selected_root.role,
                selected_root.subrole,
                selected_root.strategy,
                selected_root.scope,
                walk.node_count,
                walk.truncated,
                walk.timed_out,
                selected_root.structurally_truncated,
                selected_root.timed_out,
                normalized.text.chars().count(),
                normalized.text_clipped
            ));
        }

        Some(ScreenTextSnapshot {
            normalized_text: normalized.text,
            captured_at_unix_ms: now_unix_ms_i64(),
            source_app_bundle_id: app.bundle_id,
            source_app_name: app.name,
            source_window_title: None,
            source_window_id: None,
            snapshot_age_ms: 0,
            node_count: Some(walk.node_count.min(u32::MAX as usize) as u32),
            truncated: walk.truncated || selected_root.structurally_truncated,
            timed_out: walk.timed_out || selected_root.timed_out,
            text_clipped: normalized.text_clipped,
            content_scope: selected_root.scope,
            content_root_role: selected_root.role,
            content_root_subrole: selected_root.subrole,
            content_root_strategy: selected_root.strategy,
            refresh_reason: Some(refresh_reason.to_string()),
        })
    }

    pub(super) struct ObserverRuntime {
        active_pid: Option<i32>,
        registration: Option<ObserverRegistration>,
        event_rx: mpsc::Receiver<String>,
        event_tx: mpsc::Sender<String>,
    }

    impl ObserverRuntime {
        pub(super) fn new() -> Self {
            let (event_tx, event_rx) = mpsc::channel();
            Self {
                active_pid: None,
                registration: None,
                event_rx,
                event_tx,
            }
        }

        pub(super) fn next_refresh_reason(
            &mut self,
            stop: &AtomicBool,
            last_poll: &mut Instant,
        ) -> Option<String> {
            self.reattach_if_frontmost_app_changed();

            for _ in 0..10 {
                if stop.load(Ordering::SeqCst) {
                    return None;
                }
                unsafe {
                    CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, 1);
                }
                if let Ok(reason) = self.event_rx.try_recv() {
                    *last_poll = Instant::now();
                    return Some(reason);
                }
                if last_poll.elapsed() >= Duration::from_millis(1_000) {
                    *last_poll = Instant::now();
                    return Some("fallback_poll".to_string());
                }
            }

            None
        }

        fn reattach_if_frontmost_app_changed(&mut self) {
            let Some(app) = frontmost_application() else {
                self.active_pid = None;
                self.registration = None;
                return;
            };

            if self.active_pid == Some(app.pid) && self.registration.is_some() {
                return;
            }

            let registration = ObserverRegistration::attach(app.pid, self.event_tx.clone());
            self.active_pid = Some(app.pid);
            if registration.is_some() {
                let _ = self.event_tx.send("workspace_app_changed".to_string());
            }
            self.registration = registration;
        }
    }

    struct ObserverRegistration {
        observer: CfOwned,
        app_element: CfOwned,
        root_element: Option<CfOwned>,
        run_loop: CFRunLoopRef,
        source: CFRunLoopSourceRef,
        refcon: *mut std::ffi::c_void,
    }

    impl ObserverRegistration {
        fn attach(pid: i32, event_tx: mpsc::Sender<String>) -> Option<Self> {
            if unsafe { AXIsProcessTrusted() == 0 } {
                return None;
            }

            let app_element = unsafe { AXUIElementCreateApplication(pid) };
            if app_element.is_null() {
                return None;
            }
            let app_element = CfOwned(app_element as CFTypeRef);

            let root_element = copy_first_ax_attribute(
                app_element.0,
                &["AXFocusedWindow", "AXMainWindow", "AXFocusedUIElement"],
            );

            let mut observer: AXObserverRef = std::ptr::null();
            let result = unsafe { AXObserverCreate(pid, ax_observer_callback, &mut observer) };
            if result != K_AX_ERROR_SUCCESS || observer.is_null() {
                return None;
            }
            let observer = CfOwned(observer as CFTypeRef);
            let source = unsafe { AXObserverGetRunLoopSource(observer.0 as AXObserverRef) };
            if source.is_null() {
                return None;
            }

            let refcon = Box::into_raw(Box::new(event_tx)) as *mut std::ffi::c_void;
            for notification in [
                "AXFocusedWindowChanged",
                "AXFocusedUIElementChanged",
                "AXTitleChanged",
                "AXValueChanged",
            ] {
                add_notification(
                    observer.0 as AXObserverRef,
                    app_element.0,
                    notification,
                    refcon,
                );
            }
            if let Some(root) = root_element.as_ref() {
                for notification in [
                    "AXFocusedUIElementChanged",
                    "AXTitleChanged",
                    "AXValueChanged",
                    "AXSelectedTextChanged",
                    "AXUIElementDestroyed",
                ] {
                    add_notification(observer.0 as AXObserverRef, root.0, notification, refcon);
                }
            }

            let run_loop = unsafe { CFRunLoopGetCurrent() };
            unsafe {
                CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode);
            }

            Some(Self {
                observer,
                app_element,
                root_element,
                run_loop,
                source,
                refcon,
            })
        }
    }

    impl Drop for ObserverRegistration {
        fn drop(&mut self) {
            unsafe {
                CFRunLoopRemoveSource(self.run_loop, self.source, kCFRunLoopDefaultMode);
                drop(Box::from_raw(self.refcon as *mut mpsc::Sender<String>));
            }
            let _ = &self.observer;
            let _ = &self.app_element;
            let _ = &self.root_element;
        }
    }

    fn add_notification(
        observer: AXObserverRef,
        element: CFTypeRef,
        notification: &str,
        refcon: *mut std::ffi::c_void,
    ) {
        if let Some(notification) = CfString::new(notification) {
            unsafe {
                let _ = AXObserverAddNotification(
                    observer,
                    element as AXUIElementRef,
                    notification.as_string_ref(),
                    refcon,
                );
            }
        }
    }

    extern "C" fn ax_observer_callback(
        _observer: AXObserverRef,
        _element: AXUIElementRef,
        notification: CFStringRef,
        refcon: *mut std::ffi::c_void,
    ) {
        if refcon.is_null() {
            return;
        }
        let reason = cf_string_to_string(notification as CFTypeRef)
            .unwrap_or_else(|| "ax_notification".to_string());
        let tx = unsafe { &*(refcon as *const mpsc::Sender<String>) };
        let _ = tx.send(format!("ax_{reason}"));
    }

    struct SelectedContentRoot {
        element: Option<CfOwned>,
        score: i32,
        scope: AccessibilitySnapshotScope,
        role: Option<String>,
        subrole: Option<String>,
        strategy: Option<String>,
        structurally_truncated: bool,
        timed_out: bool,
    }

    fn root_candidates(app_element: CFTypeRef) -> Vec<CfOwned> {
        ["AXFocusedWindow", "AXMainWindow", "AXFocusedUIElement"]
            .iter()
            .filter_map(|attr| copy_ax_attribute(app_element, attr))
            .collect()
    }

    fn select_content_root(roots: &[CFTypeRef], started: Instant) -> SelectedContentRoot {
        let mut best = SelectedContentRoot {
            element: roots.first().and_then(|root| retain_cf_type(*root)),
            score: 0,
            scope: AccessibilitySnapshotScope::GenericVisibleRoot,
            role: None,
            subrole: None,
            strategy: Some("generic_visible_root".to_string()),
            structurally_truncated: false,
            timed_out: false,
        };
        let mut scan = RootScan {
            started,
            node_count: 0,
            truncated: false,
            timed_out: false,
            visited: HashSet::new(),
            best_score: best.score,
            best_element: best.element.clone(),
            best_scope: best.scope,
            best_role: best.role.clone(),
            best_subrole: best.subrole.clone(),
            best_strategy: best.strategy.clone(),
        };
        for root in roots {
            scan_root(*root, 0, false, &mut scan);
            if scan.best_score >= 100 {
                break;
            }
        }
        best.element = scan.best_element;
        best.score = scan.best_score;
        best.scope = scan.best_scope;
        best.role = scan.best_role;
        best.subrole = scan.best_subrole;
        best.strategy = scan.best_strategy;
        best.structurally_truncated = scan.truncated;
        best.timed_out = scan.timed_out;
        best
    }

    struct RootScan {
        started: Instant,
        node_count: usize,
        truncated: bool,
        timed_out: bool,
        visited: HashSet<usize>,
        best_score: i32,
        best_element: Option<CfOwned>,
        best_scope: AccessibilitySnapshotScope,
        best_role: Option<String>,
        best_subrole: Option<String>,
        best_strategy: Option<String>,
    }

    fn scan_root(element: CFTypeRef, depth: usize, chrome_ancestor: bool, scan: &mut RootScan) {
        if depth > AX_ROOT_SCAN_DEPTH_CAP || scan.node_count >= AX_ROOT_SCAN_NODE_CAP {
            scan.truncated = true;
            return;
        }
        if scan.started.elapsed().as_millis() > AX_TIMEOUT_MS {
            scan.timed_out = true;
            return;
        }
        if !scan.visited.insert(element as usize) {
            return;
        }
        scan.node_count += 1;

        let role = copy_attribute(element, "AXRole").and_then(|value| cf_string_to_string(value.0));
        let subrole =
            copy_attribute(element, "AXSubrole").and_then(|value| cf_string_to_string(value.0));
        let role_description = copy_attribute(element, "AXRoleDescription")
            .and_then(|value| cf_string_to_string(value.0));
        let is_chrome = chrome_ancestor || role.as_deref().is_some_and(role_is_chrome);
        let (score, scope, strategy) = classify_root(
            role.as_deref(),
            is_chrome,
            descendant_text_len(element, scan.started),
        );
        if score > scan.best_score {
            scan.best_score = score;
            scan.best_element = retain_cf_type(element);
            scan.best_scope = scope;
            scan.best_role = role.clone().or(role_description);
            scan.best_subrole = subrole;
            scan.best_strategy = Some(strategy.to_string());
        }
        if score >= 100 {
            return;
        }

        for attr in ["AXContents", "AXVisibleChildren", "AXChildren"] {
            for child in copy_child_elements(element, attr) {
                scan_root(child.0, depth + 1, is_chrome, scan);
                if scan.best_score >= 100 || scan.timed_out {
                    return;
                }
            }
        }
    }

    fn copy_child_elements(element: CFTypeRef, attr: &str) -> Vec<CfOwned> {
        let Some(children) = copy_attribute(element, attr) else {
            return Vec::new();
        };
        retained_ax_children_from_array(children.0)
    }

    fn retained_ax_children_from_array(value: CFTypeRef) -> Vec<CfOwned> {
        unsafe {
            if value.is_null() || CFGetTypeID(value) != CFArrayGetTypeID() {
                return Vec::new();
            }

            let count = CFArrayGetCount(value as *const _);
            let mut children = Vec::with_capacity(count.max(0) as usize);
            for index in 0..count {
                let child = CFArrayGetValueAtIndex(value as *const _, index) as CFTypeRef;
                if !is_ax_ui_element(child) {
                    continue;
                }
                let retained = CFRetain(child);
                if !retained.is_null() {
                    children.push(CfOwned(retained));
                }
            }
            children
        }
    }

    fn retain_cf_type(value: CFTypeRef) -> Option<CfOwned> {
        if value.is_null() {
            return None;
        }
        let retained = unsafe { CFRetain(value) };
        (!retained.is_null()).then_some(CfOwned(retained))
    }

    fn is_ax_ui_element(value: CFTypeRef) -> bool {
        unsafe { !value.is_null() && CFGetTypeID(value) == AXUIElementGetTypeID() }
    }

    fn copy_ax_attribute(element: CFTypeRef, attr: &str) -> Option<CfOwned> {
        copy_attribute(element, attr).filter(|value| is_ax_ui_element(value.0))
    }

    fn copy_first_ax_attribute(element: CFTypeRef, attrs: &[&str]) -> Option<CfOwned> {
        attrs
            .iter()
            .find_map(|attr| copy_ax_attribute(element, attr))
    }

    fn copy_attribute(element: CFTypeRef, attr: &str) -> Option<CfOwned> {
        if !is_ax_ui_element(element) {
            return None;
        }
        let attr = CfString::new(attr)?;
        let mut value: CFTypeRef = std::ptr::null();
        let result =
            unsafe { AXUIElementCopyAttributeValue(element, attr.as_string_ref(), &mut value) };
        if result == K_AX_ERROR_SUCCESS && !value.is_null() {
            Some(CfOwned(value))
        } else {
            None
        }
    }

    fn classify_root(
        role: Option<&str>,
        chrome_ancestor: bool,
        descendant_text_len: usize,
    ) -> (i32, AccessibilitySnapshotScope, &'static str) {
        let Some(role) = role else {
            return (
                0,
                AccessibilitySnapshotScope::GenericVisibleRoot,
                "generic_visible_root",
            );
        };
        if chrome_ancestor {
            return (
                10,
                AccessibilitySnapshotScope::ChromeOnly,
                "chrome_ancestor",
            );
        }
        if role_is_high_confidence_primary_content(role) {
            return (100, AccessibilitySnapshotScope::PrimaryContent, "web_area");
        }
        if role_is_primary_content(role) {
            return (
                80,
                AccessibilitySnapshotScope::PrimaryContent,
                "primary_role",
            );
        }
        if role_is_conditional_content_container(role)
            && descendant_text_len >= AX_MIN_CONDITIONAL_ROOT_TEXT_CHARS
        {
            return (
                60,
                AccessibilitySnapshotScope::PrimaryContent,
                "content_scroll_area",
            );
        }
        (
            20,
            AccessibilitySnapshotScope::GenericVisibleRoot,
            "generic_visible_root",
        )
    }

    fn descendant_text_len(element: CFTypeRef, started: Instant) -> usize {
        let mut walk = AxWalk {
            started,
            node_count: 0,
            truncated: false,
            timed_out: false,
            strings: Vec::new(),
            visited: HashSet::new(),
        };
        walk_element(element, 0, &mut walk);
        normalize_accessibility_text(&walk.strings)
            .text
            .chars()
            .count()
    }

    fn walk_element(element: CFTypeRef, depth: usize, walk: &mut AxWalk) {
        if depth > AX_DEPTH_CAP || walk.node_count >= AX_NODE_CAP {
            walk.truncated = true;
            return;
        }
        if walk.started.elapsed().as_millis() > AX_TIMEOUT_MS {
            walk.timed_out = true;
            return;
        }
        if !walk.visited.insert(element as usize) {
            return;
        }

        walk.node_count += 1;
        for attr in [
            "AXTitle",
            "AXValue",
            "AXDescription",
            "AXHelp",
            "AXDocument",
        ] {
            if let Some(value) = copy_attribute(element, attr) {
                if let Some(text) = cf_string_to_string(value.0) {
                    walk.strings.push(text);
                }
            }
        }

        for attr in ["AXContents", "AXVisibleChildren", "AXChildren"] {
            for child in copy_child_elements(element, attr) {
                walk_element(child.0, depth + 1, walk);
            }
        }
    }

    fn cf_string_to_string(value: CFTypeRef) -> Option<String> {
        unsafe {
            if CFGetTypeID(value) != CFStringGetTypeID() {
                return None;
            }
            let len = CFStringGetLength(value as *const _);
            let max_len = CFStringGetMaximumSizeForEncoding(len, kCFStringEncodingUTF8) + 1;
            let mut buffer = vec![0i8; max_len as usize];
            if CFStringGetCString(
                value as *const _,
                buffer.as_mut_ptr(),
                max_len,
                kCFStringEncodingUTF8,
            ) == 0
            {
                return None;
            }
            Some(
                std::ffi::CStr::from_ptr(buffer.as_ptr())
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    }

    #[allow(dead_code)]
    fn cf_number_to_i64(value: CFTypeRef) -> Option<i64> {
        unsafe {
            if CFGetTypeID(value) != CFNumberGetTypeID() as CFTypeID {
                return None;
            }
            let mut out = 0i64;
            if CFNumberGetValue(
                value as *const _,
                kCFNumberSInt64Type,
                &mut out as *mut _ as *mut _,
            ) {
                Some(out)
            } else {
                None
            }
        }
    }

    struct FrontmostApp {
        pid: i32,
        bundle_id: Option<String>,
        name: Option<String>,
    }

    fn frontmost_application() -> Option<FrontmostApp> {
        unsafe {
            let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
            if workspace.is_null() {
                return None;
            }
            let app: *mut Object = msg_send![workspace, frontmostApplication];
            if app.is_null() {
                return None;
            }
            let pid: i32 = msg_send![app, processIdentifier];
            let bundle: *mut Object = msg_send![app, bundleIdentifier];
            let name: *mut Object = msg_send![app, localizedName];
            Some(FrontmostApp {
                pid,
                bundle_id: ns_string_to_string(bundle),
                name: ns_string_to_string(name),
            })
        }
    }

    unsafe fn ns_string_to_string(value: *mut Object) -> Option<String> {
        if value.is_null() {
            return None;
        }
        let c: *const std::os::raw::c_char = msg_send![value, UTF8String];
        if c.is_null() {
            return None;
        }
        Some(std::ffi::CStr::from_ptr(c).to_string_lossy().into_owned())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn retained_ax_children_ignores_non_array_values() {
            let value = CfString::new("not an array").expect("test CFString should be created");

            assert!(retained_ax_children_from_array(value.0 as CFTypeRef).is_empty());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_dedupes_adjacent_strings() {
        let text = normalize_accessibility_text(&[
            "  Hello\nworld ".to_string(),
            "Hello world".to_string(),
            "Next\tvalue".to_string(),
        ]);

        assert_eq!(text.text, "Hello world Next value");
        assert!(!text.text_clipped);
    }

    #[test]
    fn normalizes_and_reports_text_clipping() {
        let text = normalize_accessibility_text(&["x".repeat(20_001)]);

        assert_eq!(text.text.chars().count(), 20_000);
        assert!(text.text_clipped);
    }

    fn representative_snapshot() -> ScreenTextSnapshot {
        ScreenTextSnapshot {
            normalized_text: "This accessibility snapshot has enough representative visible content for the captured frame.".to_string(),
            captured_at_unix_ms: 0,
            source_app_bundle_id: Some("com.example.App".to_string()),
            source_app_name: None,
            source_window_title: None,
            source_window_id: None,
            snapshot_age_ms: 0,
            node_count: Some(12),
            truncated: false,
            timed_out: false,
            text_clipped: false,
            content_scope: AccessibilitySnapshotScope::PrimaryContent,
            content_root_role: Some("AXWebArea".to_string()),
            content_root_subrole: None,
            content_root_strategy: Some("web_area".to_string()),
            refresh_reason: None,
        }
    }

    #[test]
    fn rejects_thin_or_timed_out_snapshots() {
        let snapshot = ScreenTextSnapshot {
            normalized_text: "short".to_string(),
            ..representative_snapshot()
        };

        assert!(!snapshot_usable_for_frame(&snapshot, None));
    }

    #[test]
    fn evaluates_primary_and_generic_visible_snapshots_as_usable() {
        let primary = representative_snapshot();
        assert!(evaluate_snapshot_for_frame(&primary, None).usable);

        let generic = ScreenTextSnapshot {
            content_scope: AccessibilitySnapshotScope::GenericVisibleRoot,
            content_root_role: Some("AXWindow".to_string()),
            content_root_strategy: Some("generic_visible_root".to_string()),
            ..representative_snapshot()
        };
        assert!(evaluate_snapshot_for_frame(&generic, None).usable);
    }

    #[test]
    fn rejects_chrome_only_snapshots() {
        let snapshot = ScreenTextSnapshot {
            content_scope: AccessibilitySnapshotScope::ChromeOnly,
            content_root_role: Some("AXToolbar".to_string()),
            content_root_strategy: Some("chrome_ancestor".to_string()),
            ..representative_snapshot()
        };

        let evaluation = evaluate_snapshot_for_frame(&snapshot, None);
        assert!(!evaluation.usable);
        assert_eq!(
            evaluation.rejection_reason,
            Some(ScreenTextSnapshotRejectionReason::ChromeOnly)
        );
    }

    #[test]
    fn rejects_timed_out_and_structurally_truncated_snapshots() {
        let timed_out = ScreenTextSnapshot {
            timed_out: true,
            ..representative_snapshot()
        };
        assert_eq!(
            evaluate_snapshot_for_frame(&timed_out, None).rejection_reason,
            Some(ScreenTextSnapshotRejectionReason::TimedOut)
        );

        let truncated = ScreenTextSnapshot {
            truncated: true,
            ..representative_snapshot()
        };
        assert_eq!(
            evaluate_snapshot_for_frame(&truncated, None).rejection_reason,
            Some(ScreenTextSnapshotRejectionReason::StructurallyTruncated)
        );
    }

    #[test]
    fn accepts_text_clipped_representative_snapshot() {
        let snapshot = ScreenTextSnapshot {
            text_clipped: true,
            ..representative_snapshot()
        };

        assert!(evaluate_snapshot_for_frame(&snapshot, None).usable);
    }

    #[test]
    fn classifies_content_and_chrome_roles() {
        assert!(role_is_high_confidence_primary_content("AXWebArea"));
        assert!(role_is_primary_content("AXDocument"));
        assert!(role_is_primary_content("AXTable"));
        assert!(role_is_conditional_content_container("AXScrollArea"));
        assert!(role_is_chrome("AXToolbar"));
        assert!(role_is_chrome("AXTabGroup"));
        assert!(!role_is_chrome("AXButton"));
    }
}
