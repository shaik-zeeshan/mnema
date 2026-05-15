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
    pub refresh_reason: Option<String>,
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
                        let snapshot = collect_frontmost_accessibility_snapshot(&reason);
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

fn collect_frontmost_accessibility_snapshot(refresh_reason: &str) -> Option<ScreenTextSnapshot> {
    #[cfg(target_os = "macos")]
    {
        macos_ax::collect_frontmost_accessibility_snapshot(refresh_reason)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = refresh_reason;
        None
    }
}

pub(crate) fn normalize_accessibility_text(strings: &[String]) -> String {
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

    parts.join(" ").chars().take(20_000).collect()
}

pub(crate) fn snapshot_usable_for_frame(
    snapshot: &ScreenTextSnapshot,
    frame_metadata: Option<&capture_metadata::FrameMetadataSnapshot>,
) -> bool {
    if snapshot.timed_out || snapshot.truncated {
        return false;
    }
    if snapshot.snapshot_age_ms > 1_500 || snapshot.snapshot_age_ms < -250 {
        return false;
    }
    if snapshot.normalized_text.trim().chars().count() < 40 {
        return false;
    }
    if let (Some(expected), Some(actual)) = (
        frame_metadata.and_then(|metadata| metadata.app_bundle_id.as_deref()),
        snapshot.source_app_bundle_id.as_deref(),
    ) {
        if expected != actual {
            return false;
        }
    }
    if snapshot
        .source_app_bundle_id
        .as_deref()
        .is_some_and(app_requires_ocr_fallback)
    {
        return false;
    }

    true
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
        array::{CFArrayGetCount, CFArrayGetValueAtIndex},
        base::{Boolean, CFGetTypeID, CFRelease, CFTypeID, CFTypeRef},
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
    const AX_TEXT_CAP: usize = 20_000;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> Boolean;
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> AXError;
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
    }

    pub(super) fn collect_frontmost_accessibility_snapshot(
        refresh_reason: &str,
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

        let root = copy_first_attribute(
            app_element.0,
            &["AXFocusedWindow", "AXMainWindow", "AXFocusedUIElement"],
        );
        let root_ref = root.as_ref().map_or(app_element.0, |root| root.0);

        let mut walk = AxWalk {
            started: Instant::now(),
            node_count: 0,
            truncated: false,
            timed_out: false,
            strings: Vec::new(),
        };
        walk_element(root_ref, 0, &mut walk);

        let normalized_text = normalize_accessibility_text(&walk.strings);
        if normalized_text.is_empty() {
            return None;
        }

        Some(ScreenTextSnapshot {
            normalized_text,
            captured_at_unix_ms: now_unix_ms_i64(),
            source_app_bundle_id: app.bundle_id,
            source_app_name: app.name,
            source_window_title: None,
            source_window_id: None,
            snapshot_age_ms: 0,
            node_count: Some(walk.node_count.min(u32::MAX as usize) as u32),
            truncated: walk.truncated,
            timed_out: walk.timed_out,
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

            let root_element = copy_first_attribute(
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

    fn walk_element(element: CFTypeRef, depth: usize, walk: &mut AxWalk) {
        if depth > AX_DEPTH_CAP || walk.node_count >= AX_NODE_CAP {
            walk.truncated = true;
            return;
        }
        if walk.started.elapsed().as_millis() > AX_TIMEOUT_MS {
            walk.timed_out = true;
            return;
        }
        if walk.strings.iter().map(|s| s.len()).sum::<usize>() >= AX_TEXT_CAP {
            walk.truncated = true;
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

        let Some(children) = copy_attribute(element, "AXChildren") else {
            return;
        };
        unsafe {
            let count = CFArrayGetCount(children.0 as *const _);
            for index in 0..count {
                let child = CFArrayGetValueAtIndex(children.0 as *const _, index);
                if !child.is_null() {
                    walk_element(child as CFTypeRef, depth + 1, walk);
                }
            }
        }
    }

    fn copy_first_attribute(element: CFTypeRef, attrs: &[&str]) -> Option<CfOwned> {
        attrs.iter().find_map(|attr| copy_attribute(element, attr))
    }

    fn copy_attribute(element: CFTypeRef, attr: &str) -> Option<CfOwned> {
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

        assert_eq!(text, "Hello world Next value");
    }

    #[test]
    fn rejects_thin_or_timed_out_snapshots() {
        let snapshot = ScreenTextSnapshot {
            normalized_text: "short".to_string(),
            captured_at_unix_ms: 0,
            source_app_bundle_id: None,
            source_app_name: None,
            source_window_title: None,
            source_window_id: None,
            snapshot_age_ms: 0,
            node_count: None,
            truncated: false,
            timed_out: false,
            refresh_reason: None,
        };

        assert!(!snapshot_usable_for_frame(&snapshot, None));
    }
}
