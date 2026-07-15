use serde::{Deserialize, Serialize};
#[cfg(target_os = "macos")]
use std::sync::OnceLock;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, MutexGuard,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::native_capture;
#[cfg(target_os = "windows")]
use std::panic::{catch_unwind, AssertUnwindSafe};
#[cfg(target_os = "windows")]
use windows_sys::core::GUID;
#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::RemoteDesktop::{
        WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
    },
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            DEVICE_NOTIFY_WINDOW_HANDLE, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMECRITICAL,
            PBT_APMRESUMESTANDBY, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE,
            WM_NCDESTROY, WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK,
            WTS_SESSION_UNLOCK,
        },
    },
};

#[cfg(target_os = "windows")]
static WINDOWS_SESSION_NOTIFICATION_HWND: Mutex<Option<isize>> = Mutex::new(None);
#[cfg(target_os = "windows")]
const WINDOWS_SESSION_NOTIFICATION_SUBCLASS_ID: usize = 1;

// ── Console display-state power notifications (Stream 3 — DPMS-off sleep) ──────
//
// `RegisterPowerSettingNotification`, `UnregisterPowerSettingNotification`,
// `POWERBROADCAST_SETTING`, and `GUID_CONSOLE_DISPLAY_STATE` live behind
// windows-sys' `Win32_System_Power` / `Win32_System_SystemServices` features,
// which this crate does not enable. Rather than widen the dependency's feature
// set (and to keep this slice contained to `windows.rs`), declare the minimal FFI
// surface locally. Both functions are exported by user32.dll — exactly how
// windows-sys itself links them.

/// `GUID_CONSOLE_DISPLAY_STATE` (`6fe69556-704a-47a0-8f24-c28d936fda47`).
///
/// The modern console display-state power setting: it reflects the *overall*
/// console display, so one sleeping monitor among several awake ones still reads
/// "on" — unlike the deprecated per-monitor `GUID_MONITOR_POWER_ON`.
#[cfg(target_os = "windows")]
const GUID_CONSOLE_DISPLAY_STATE: GUID =
    GUID::from_u128(0x6fe69556_704a_47a0_8f24_c28d936fda47);

/// Mirror of windows-sys' `POWERBROADCAST_SETTING`: the payload behind `lparam`
/// for a `PBT_POWERSETTINGCHANGE` message. `data` is a trailing variable-length
/// byte array (`[u8; 1]` here, matching windows-sys); for
/// `GUID_CONSOLE_DISPLAY_STATE` the first byte is the display-state value.
#[cfg(target_os = "windows")]
#[repr(C)]
struct PowerBroadcastSetting {
    power_setting: GUID,
    data_length: u32,
    data: [u8; 1],
}

#[cfg(target_os = "windows")]
#[link(name = "user32")]
extern "system" {
    fn RegisterPowerSettingNotification(
        hrecipient: *mut std::ffi::c_void,
        powersettingguid: *const GUID,
        flags: u32,
    ) -> isize;
    fn UnregisterPowerSettingNotification(handle: isize) -> i32;
}

/// `GUID` has no `PartialEq`, so compare the two power-setting GUIDs field-by-field.
#[cfg(target_os = "windows")]
fn guids_equal(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

/// The decoded `GUID_CONSOLE_DISPLAY_STATE` value.
#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConsoleDisplayState {
    Off,
    On,
    Dimmed,
}

/// Pure decode of a `GUID_CONSOLE_DISPLAY_STATE` `POWERBROADCAST_SETTING.Data`
/// byte: `0` = display off, `1` = display on, `2` = display dimmed. Any other
/// value is unknown and yields `None`, so the caller drops it rather than guessing
/// a capture transition. Kept free of any HWND / Win32 handle so it is unit
/// testable in isolation.
#[cfg(target_os = "windows")]
fn decode_console_display_state(data: u32) -> Option<ConsoleDisplayState> {
    match data {
        0 => Some(ConsoleDisplayState::Off),
        1 => Some(ConsoleDisplayState::On),
        2 => Some(ConsoleDisplayState::Dimmed),
        _ => None,
    }
}

/// Live `HPOWERNOTIFY` (an `isize` handle) for the console display-state
/// registration, so teardown can `UnregisterPowerSettingNotification` it. Sits
/// beside `WINDOWS_SESSION_NOTIFICATION_HWND`; both are owned by the same Main
/// window HWND.
#[cfg(target_os = "windows")]
static WINDOWS_DISPLAY_POWER_NOTIFY: Mutex<Option<isize>> = Mutex::new(None);

#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowsPowerBroadcastEvent {
    Suspend,
    Resume,
    /// A `GUID_CONSOLE_DISPLAY_STATE` transition decoded from a
    /// `PBT_POWERSETTINGCHANGE`. Forwarded to the lifecycle as display on/off;
    /// `Dimmed` is decoded but treated as a no-op (the display is still drawable).
    DisplayPowerChanged(ConsoleDisplayState),
}

/// Decode a `WM_POWERBROADCAST` message into a capture-relevant power event.
///
/// `wparam` is the broadcast type. APM suspend/resume is fully described by
/// `wparam`; for `PBT_POWERSETTINGCHANGE` the payload is a `POWERBROADCAST_SETTING`
/// behind `lparam`, so that arm dereferences `lparam` (null-checked) and, when the
/// setting is `GUID_CONSOLE_DISPLAY_STATE`, delegates the raw display byte → enum
/// mapping to the pure [`decode_console_display_state`].
///
/// # Safety
/// For `PBT_POWERSETTINGCHANGE`, `lparam` must be null or a valid pointer to a
/// `POWERBROADCAST_SETTING` (as the window procedure delivers it). Every other
/// `wparam` ignores `lparam`, so a `0`/null `lparam` is always sound.
#[cfg(target_os = "windows")]
unsafe fn windows_power_broadcast_event(
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<WindowsPowerBroadcastEvent> {
    match wparam as u32 {
        PBT_APMSUSPEND => Some(WindowsPowerBroadcastEvent::Suspend),
        PBT_APMRESUMEAUTOMATIC
        | PBT_APMRESUMECRITICAL
        | PBT_APMRESUMESTANDBY
        | PBT_APMRESUMESUSPEND => Some(WindowsPowerBroadcastEvent::Resume),
        PBT_POWERSETTINGCHANGE => {
            // SAFETY: per the function contract `lparam` is null or a valid
            // `POWERBROADCAST_SETTING`; `as_ref()` null-checks before deref.
            let setting = (lparam as *const PowerBroadcastSetting).as_ref()?;
            if !guids_equal(&setting.power_setting, &GUID_CONSOLE_DISPLAY_STATE)
                || setting.data_length < 1
            {
                return None;
            }
            decode_console_display_state(setting.data[0] as u32)
                .map(WindowsPowerBroadcastEvent::DisplayPowerChanged)
        }
        _ => None,
    }
}

/// SLICE 8 INTEGRATION POINT (Stream 3 — DPMS-off sleep policy).
///
/// Forward a decoded console-display power change to the capture lifecycle. This
/// mirrors how `WindowsPowerBroadcastEvent::{Suspend,Resume}` reach the lifecycle
/// through `crate::native_capture::handle_windows_system_{suspend,resume}_from_app_handle`
/// (native_capture.rs:1364 / :1389), each of which locks `NativeCaptureState` and
/// calls a `lifecycle.handle_windows_system_*` method, then emits the changed
/// session + refreshes the status bar.
///
/// `display_on == false` means the console display slept (pause); `true` means it
/// woke (guarded resume). `Dimmed` is dropped here — the display is still
/// drawable, so it is not a capture transition.
///
/// Slice 8 wires the lifecycle side and turns the body below into a one-line
/// forward:
///   1. Add `TransientLivenessTrigger::DisplayAsleep` (native_capture/inactivity.rs),
///      alongside `DisplayUnavailable` / `SessionLock` / `SystemSuspend`.
///   2. Add the lifecycle handlers (native_capture/lifecycle.rs):
///        - display-off → pause via
///          `pause_screen_for_transient_liveness(.., TransientLivenessTrigger::DisplayAsleep)`
///        - display-on  → guarded resume: resume only when the current pause
///          reason is `DisplayAsleep` AND the session is not locked AND not
///          suspended (the resume is event-driven; the poll probe cannot observe
///          DPMS).
///   3. Add the forwarder in native_capture.rs that locks `NativeCaptureState`
///      and dispatches — mirror the suspend/resume forwarders exactly:
///        `pub(crate) fn handle_windows_display_power_changed_from_app_handle(
///             app_handle: &tauri::AppHandle, display_on: bool)`
///   4. Replace the body below with the single call:
///        `crate::native_capture::handle_windows_display_power_changed_from_app_handle(app_handle, display_on);`
#[cfg(target_os = "windows")]
fn forward_windows_display_power_change(app_handle: &tauri::AppHandle, state: ConsoleDisplayState) {
    let display_on = match state {
        ConsoleDisplayState::On => true,
        ConsoleDisplayState::Off => false,
        // A dimmed display is still drawable, so it is not a capture transition.
        ConsoleDisplayState::Dimmed => return,
    };

    crate::native_capture::handle_windows_display_power_changed_from_app_handle(
        app_handle, display_on,
    );
}

#[cfg(target_os = "windows")]
fn register_windows_display_power_notifications(hwnd: HWND) {
    let mut stored = match WINDOWS_DISPLAY_POWER_NOTIFY.lock() {
        Ok(stored) => stored,
        Err(_) => {
            crate::native_capture::debug_log::log_warn(
                "Windows display power notification state poisoned; skipping registration",
            );
            return;
        }
    };

    // Drop any handle left over from a prior HWND before registering on the new
    // one, so we never leak an `HPOWERNOTIFY`.
    if let Some(previous) = stored.take() {
        unsafe {
            unregister_windows_display_power_notifications(previous);
        }
    }

    let handle = unsafe {
        RegisterPowerSettingNotification(
            hwnd as *mut std::ffi::c_void,
            &GUID_CONSOLE_DISPLAY_STATE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
    };
    if handle == 0 {
        crate::native_capture::debug_log::log_warn(format!(
            "failed to register Windows console display-state power notifications: {}",
            std::io::Error::last_os_error()
        ));
        return;
    }

    *stored = Some(handle);
}

#[cfg(target_os = "windows")]
unsafe fn unregister_windows_display_power_notifications(handle: isize) {
    if handle == 0 {
        return;
    }
    if UnregisterPowerSettingNotification(handle) == 0 {
        crate::native_capture::debug_log::log_warn(format!(
            "failed to unregister Windows display power notifications: {}",
            std::io::Error::last_os_error()
        ));
    }
}

const ONBOARDING_STATE_FILE_NAME: &str = "onboarding-state.json";
const OPEN_SETTINGS_TAB_EVENT: &str = "open_settings_tab";
const QUICK_RECALL_WINDOW_LABEL: &str = "quick-recall";
// Emitted to the Quick Recall webview whenever the panel is dismissed (ordered
// out / hidden). The webview is reused across summons rather than destroyed, so
// the Svelte `onDestroy` teardown never runs on dismiss; the panel listens for
// this to cancel any resident Ask AI PI session.
const QUICK_RECALL_DISMISSED_EVENT: &str = "quick_recall_dismissed";

// The Quick Recall surface is a non-activating NSPanel that emits a spurious
// `Focused(false)` while AppKit promotes its webview to first responder on the
// first summon. A blur within this grace window of the last summon is treated as
// that transient setup blur, not a genuine click-away, so the freshly-summoned
// launcher is not torn down out from under the user.
const QUICK_RECALL_SUMMON_BLUR_GRACE: Duration = Duration::from_millis(300);

// The wall-clock instant of the most recent Quick Recall summon, used to honor
// the `QUICK_RECALL_SUMMON_BLUR_GRACE` window in the `Focused(false)` handler.
static LAST_QUICK_RECALL_SUMMON: Mutex<Option<Instant>> = Mutex::new(None);

// One-shot suppression of the very next Quick Recall blur-dismiss. The frontend
// sets this (via `quick_recall_suppress_blur_dismiss`) immediately before opening
// an answer link in the OS browser: activating the browser blurs the panel, and
// without this flag that blur would dismiss the launcher and tear down the
// in-flight Ask AI session the user is reading. Consumed by the next blur only,
// so ordinary click-away dismissal is unaffected.
static SUPPRESS_NEXT_QUICK_RECALL_BLUR_DISMISS: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
static MACOS_TERMINATE_APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();
#[cfg(target_os = "macos")]
static MACOS_TERMINATE_HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenSettingsTabPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    tab: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    focus: Option<String>,
}

/// Pending Settings deeplink(s) for a cold main window. Mirrors
/// `InsightsOpenConversationState` in `lib.rs`: the live `open_settings_tab`
/// event drives a warm window, but a freshly-built (cold) main window hasn't
/// attached its `listen("open_settings_tab")` yet (that registers in a
/// `+layout.svelte` mount effect) and Tauri doesn't buffer events with no
/// listener — so the cold-start tray "Open Settings" would be dropped and strand
/// the user on Timeline. We queue the normalized payload here when Main had to be
/// BUILT and the layout drains it on mount via `drain_pending_open_settings`.
#[derive(Default)]
pub struct PendingOpenSettingsState {
    pending: Mutex<VecDeque<OpenSettingsTabPayload>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppWindow {
    Onboarding,
    Main,
    CliAccessRequest,
    Debug,
    QuickRecall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DestroyedWindowAction {
    FocusMainWindow,
    ExitApp,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingState {
    schema_version: u32,
    completed_at_unix_ms: Option<u64>,
}

impl OnboardingState {
    fn incomplete() -> Self {
        Self {
            schema_version: 1,
            completed_at_unix_ms: None,
        }
    }

    fn completed_now() -> Self {
        Self {
            schema_version: 1,
            completed_at_unix_ms: Some(now_unix_ms()),
        }
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.completed_at_unix_ms.is_some()
    }
}

/// Command-only return shape for `get_onboarding_state`. Mirrors the persisted
/// `OnboardingState` fields and adds `recording_settings_ever_saved` — a
/// LIVE-COMPUTED, NON-PERSISTED signal (the existence of `recording-settings.json`
/// on disk). It lives on a SEPARATE type so the persisted file shape
/// (`OnboardingState`) stays unchanged: the computed flag is never written into
/// `onboarding-state.json` and never required when deserializing existing files.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingStateView {
    schema_version: u32,
    completed_at_unix_ms: Option<u64>,
    recording_settings_ever_saved: bool,
}

impl OnboardingStateView {
    fn from_state_and_disk(state: OnboardingState, recording_settings_ever_saved: bool) -> Self {
        Self {
            schema_version: state.schema_version,
            completed_at_unix_ms: state.completed_at_unix_ms,
            recording_settings_ever_saved,
        }
    }
}

#[derive(Debug, Default)]
pub struct OnboardingStateRuntime {
    state: Option<OnboardingState>,
}

pub type OnboardingStateStore = Mutex<OnboardingStateRuntime>;

#[derive(Default)]
pub struct AppExitCoordinatorState {
    exit_requested: AtomicBool,
    final_graceful_exit_ready: AtomicBool,
    restart_after_graceful_exit: AtomicBool,
}

impl AppExitCoordinatorState {
    fn begin_exit(&self, restart_after_graceful_exit: bool) -> bool {
        // Merge restart intent on every call so a restart-to-update request is
        // honored even when a plain graceful exit is already in progress. We
        // only ever raise the flag (never downgrade true -> false), so a later
        // plain quit cannot cancel a pending update restart.
        self.restart_after_graceful_exit
            .fetch_or(restart_after_graceful_exit, Ordering::SeqCst);
        !self.exit_requested.swap(true, Ordering::SeqCst)
    }

    fn is_exit_requested(&self) -> bool {
        self.exit_requested.load(Ordering::SeqCst)
    }

    fn mark_final_graceful_exit_ready(&self) {
        self.final_graceful_exit_ready.store(true, Ordering::SeqCst);
    }

    fn is_final_graceful_exit_ready(&self) -> bool {
        self.final_graceful_exit_ready.load(Ordering::SeqCst)
    }

    fn should_restart_after_graceful_exit(&self) -> bool {
        self.restart_after_graceful_exit.load(Ordering::SeqCst)
    }
}

struct AppWindowConfig {
    label: &'static str,
    path: &'static str,
    title: &'static str,
    inner_size: (f64, f64),
    min_inner_size: (f64, f64),
    gated_by_dev_options: bool,
    decorations: bool,
    overlay_title_bar: bool,
    transparent: bool,
    shadow: bool,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    macos_corner_radius: Option<f64>,
}

impl AppWindow {
    const fn config(self) -> AppWindowConfig {
        match self {
            Self::Onboarding => AppWindowConfig {
                label: "onboarding",
                path: "onboarding",
                title: "mnema · Onboarding",
                inner_size: (1120.0, 800.0),
                min_inner_size: (920.0, 620.0),
                gated_by_dev_options: false,
                decorations: false,
                overlay_title_bar: false,
                transparent: true,
                shadow: true,
                macos_corner_radius: Some(12.0),
            },
            Self::Main => AppWindowConfig {
                label: "main",
                path: "/",
                title: "mnema",
                inner_size: (800.0, 600.0),
                min_inner_size: (640.0, 480.0),
                gated_by_dev_options: false,
                decorations: true,
                overlay_title_bar: true,
                transparent: false,
                shadow: false,
                macos_corner_radius: None,
            },
            Self::CliAccessRequest => AppWindowConfig {
                label: "cli-access-request",
                path: "access/request",
                title: "mnema · CLI Access",
                inner_size: (520.0, 560.0),
                min_inner_size: (460.0, 480.0),
                gated_by_dev_options: false,
                decorations: false,
                overlay_title_bar: false,
                transparent: true,
                shadow: true,
                macos_corner_radius: Some(12.0),
            },
            Self::Debug => AppWindowConfig {
                label: "debug",
                path: "debug",
                title: "mnema · Debug",
                inner_size: (980.0, 680.0),
                min_inner_size: (800.0, 560.0),
                gated_by_dev_options: true,
                decorations: false,
                overlay_title_bar: false,
                transparent: true,
                shadow: true,
                macos_corner_radius: Some(12.0),
            },
            Self::QuickRecall => AppWindowConfig {
                label: "quick-recall",
                path: "quick-recall",
                title: "mnema · Quick Recall",
                inner_size: (1120.0, 720.0),
                min_inner_size: (960.0, 600.0),
                gated_by_dev_options: false,
                decorations: false,
                overlay_title_bar: false,
                transparent: true,
                shadow: true,
                macos_corner_radius: Some(12.0),
            },
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "onboarding" => Some(Self::Onboarding),
            "main" => Some(Self::Main),
            "cli-access-request" => Some(Self::CliAccessRequest),
            "debug" => Some(Self::Debug),
            "quick-recall" => Some(Self::QuickRecall),
            _ => None,
        }
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn onboarding_state_file_path(app_handle: &tauri::AppHandle) -> PathBuf {
    if let Ok(config_dir) = app_handle.path().app_config_dir() {
        return config_dir.join(ONBOARDING_STATE_FILE_NAME);
    }

    PathBuf::from(".mnema").join(ONBOARDING_STATE_FILE_NAME)
}

fn load_onboarding_state_from_path(path: PathBuf) -> OnboardingState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<OnboardingState>(&raw).ok())
        .filter(OnboardingState::is_complete)
        .unwrap_or_else(OnboardingState::incomplete)
}

fn lock_onboarding_state(store: &OnboardingStateStore) -> MutexGuard<'_, OnboardingStateRuntime> {
    store.lock().expect("onboarding state store poisoned")
}

fn current_onboarding_state(
    app: &tauri::AppHandle,
    store: &OnboardingStateStore,
) -> OnboardingState {
    let mut runtime = lock_onboarding_state(store);
    if let Some(state) = runtime.state.clone() {
        return state;
    }

    let state = load_onboarding_state_from_path(onboarding_state_file_path(app));
    runtime.state = Some(state.clone());
    state
}

fn persist_onboarding_state(
    app: &tauri::AppHandle,
    store: &OnboardingStateStore,
    state: OnboardingState,
) -> Result<OnboardingState, String> {
    let file_path = onboarding_state_file_path(app);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create onboarding state directory: {error}"))?;
    }

    let serialized = serde_json::to_string_pretty(&state)
        .map_err(|error| format!("Failed to serialize onboarding state: {error}"))?;
    std::fs::write(file_path, serialized)
        .map_err(|error| format!("Failed to persist onboarding state: {error}"))?;

    let mut runtime = lock_onboarding_state(store);
    runtime.state = Some(state.clone());

    Ok(state)
}

fn ensure_window_allowed(
    window: AppWindow,
    state: Option<&native_capture::RecordingSettingsState>,
) -> Result<(), String> {
    let config = window.config();
    if !config.gated_by_dev_options {
        return Ok(());
    }

    let Some(state) = state else {
        return Err("developer options state unavailable".into());
    };

    let settings = native_capture::read_recording_settings(state);
    if settings.developer_options_enabled {
        Ok(())
    } else {
        Err("developer options disabled".into())
    }
}

fn show_and_focus_window(window: &WebviewWindow) {
    show_macos_dock_icon(window.app_handle());
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    refresh_macos_dock_icon_visibility(window.app_handle());
}

#[cfg(target_os = "windows")]
fn register_windows_session_notifications(window: &WebviewWindow) {
    if window.label() != AppWindow::Main.config().label {
        return;
    }

    let hwnd = match window.hwnd() {
        Ok(hwnd) => hwnd.0 as HWND,
        Err(error) => {
            crate::native_capture::debug_log::log_warn(format!(
                "failed to get main window HWND for Windows session notifications: {error}"
            ));
            return;
        }
    };
    if hwnd.is_null() {
        crate::native_capture::debug_log::log_warn(
            "main window HWND was null while registering Windows session notifications",
        );
        return;
    }

    let raw_hwnd = hwnd as isize;
    let mut registered_hwnd = match WINDOWS_SESSION_NOTIFICATION_HWND.lock() {
        Ok(registered_hwnd) => registered_hwnd,
        Err(_) => {
            crate::native_capture::debug_log::log_warn(
                "Windows session notification state poisoned; skipping registration",
            );
            return;
        }
    };
    if registered_hwnd.is_some_and(|registered| registered == raw_hwnd) {
        return;
    }
    if let Some(registered) = registered_hwnd.take() {
        unsafe {
            unregister_windows_session_notifications(registered as HWND);
        }
    }

    let app_handle = Box::into_raw(Box::new(window.app_handle().clone())) as usize;
    let registered = unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) != 0 };
    if !registered {
        unsafe {
            drop(Box::from_raw(app_handle as *mut tauri::AppHandle));
        }
        crate::native_capture::debug_log::log_warn(format!(
            "failed to register Windows session notifications: {}",
            std::io::Error::last_os_error()
        ));
        return;
    }

    let subclassed = unsafe {
        SetWindowSubclass(
            hwnd,
            Some(windows_session_notification_subclass_proc),
            WINDOWS_SESSION_NOTIFICATION_SUBCLASS_ID,
            app_handle,
        ) != 0
    };
    if !subclassed {
        unsafe {
            unregister_windows_session_notifications(hwnd);
            drop(Box::from_raw(app_handle as *mut tauri::AppHandle));
        }
        crate::native_capture::debug_log::log_warn(format!(
            "failed to subclass main window for Windows session notifications: {}",
            std::io::Error::last_os_error()
        ));
        return;
    }

    // Register for console display-state power notifications on the same HWND so
    // the subclass proc receives WM_POWERBROADCAST / PBT_POWERSETTINGCHANGE for
    // GUID_CONSOLE_DISPLAY_STATE (Stream 3 — DPMS-off sleep policy). Done only
    // after the subclass is installed, since the subclass proc is what decodes
    // these messages. A failure here is logged but non-fatal: sleep/wake +
    // lock/unlock notifications are already live.
    register_windows_display_power_notifications(hwnd);

    *registered_hwnd = Some(raw_hwnd);
}

#[cfg(target_os = "windows")]
unsafe fn unregister_windows_session_notifications(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    if WTSUnRegisterSessionNotification(hwnd) == 0 {
        crate::native_capture::debug_log::log_warn(format!(
            "failed to unregister Windows session notifications: {}",
            std::io::Error::last_os_error()
        ));
    }
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn windows_session_notification_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    app_handle_ptr: usize,
) -> LRESULT {
    if msg == WM_POWERBROADCAST {
        // The handlers below lock NativeCaptureState and start/stop real capture,
        // which can block. Running that inline on the HWND message-queue thread
        // would stall DefSubclassProc and the message pump, so offload it to a
        // worker thread (mirroring how macOS offloads wake recovery). The
        // AppHandle is cloned before being moved into the thread; the
        // catch_unwind guard is preserved inside the worker so a panicking
        // handler never tears down the process.
        if let Some(event) = windows_power_broadcast_event(wparam, lparam) {
            let app_handle = (*(app_handle_ptr as *const tauri::AppHandle)).clone();
            std::thread::spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| match event {
                    WindowsPowerBroadcastEvent::Suspend => {
                        crate::native_capture::handle_windows_system_suspend_from_app_handle(
                            &app_handle,
                        );
                    }
                    WindowsPowerBroadcastEvent::Resume => {
                        crate::native_capture::handle_windows_system_resume_from_app_handle(
                            &app_handle,
                        );
                    }
                    WindowsPowerBroadcastEvent::DisplayPowerChanged(state) => {
                        forward_windows_display_power_change(&app_handle, state);
                    }
                }));
                if result.is_err() {
                    crate::native_capture::debug_log::log_error(
                        "Windows power broadcast callback panicked; continuing without aborting window procedure",
                    );
                }
            });
        }
    }

    if msg == WM_WTSSESSION_CHANGE {
        // See the WM_POWERBROADCAST note above: lock/unlock restart work runs on a
        // worker thread so the subclass proc returns promptly and the message
        // pump keeps draining.
        let session_event = wparam as u32;
        if matches!(session_event, WTS_SESSION_LOCK | WTS_SESSION_UNLOCK) {
            let app_handle = (*(app_handle_ptr as *const tauri::AppHandle)).clone();
            std::thread::spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| match session_event {
                    WTS_SESSION_LOCK => {
                        crate::native_capture::handle_windows_session_lock_from_app_handle(
                            &app_handle,
                        );
                    }
                    WTS_SESSION_UNLOCK => {
                        crate::native_capture::handle_windows_session_unlock_from_app_handle(
                            &app_handle,
                        );
                    }
                    _ => {}
                }));
                if result.is_err() {
                    crate::native_capture::debug_log::log_error(
                        "Windows session notification callback panicked; continuing without aborting window procedure",
                    );
                }
            });
        }
    }

    if msg == WM_NCDESTROY {
        unregister_windows_session_notifications(hwnd);
        // Tear down the console display-state power registration alongside the
        // session notifications (Stream 3 — DPMS-off sleep policy).
        if let Ok(mut display_notify) = WINDOWS_DISPLAY_POWER_NOTIFY.lock() {
            if let Some(handle) = display_notify.take() {
                unregister_windows_display_power_notifications(handle);
            }
        } else {
            crate::native_capture::debug_log::log_warn(
                "Windows display power notification state poisoned while clearing registration",
            );
        }
        if let Ok(mut registered_hwnd) = WINDOWS_SESSION_NOTIFICATION_HWND.lock() {
            if registered_hwnd.is_some_and(|registered| registered == hwnd as isize) {
                *registered_hwnd = None;
            }
        } else {
            crate::native_capture::debug_log::log_warn(
                "Windows session notification state poisoned while clearing registration",
            );
        }
        RemoveWindowSubclass(
            hwnd,
            Some(windows_session_notification_subclass_proc),
            subclass_id,
        );
        drop(Box::from_raw(app_handle_ptr as *mut tauri::AppHandle));
    }

    DefSubclassProc(hwnd, msg, wparam, lparam)
}

pub(crate) fn open_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    open_or_focus_window(app, AppWindow::Main, None)
}

pub(crate) fn open_onboarding_window(app: &tauri::AppHandle) -> Result<(), String> {
    open_or_focus_window(app, AppWindow::Onboarding, None)
}

fn open_or_focus_window(
    app: &tauri::AppHandle,
    window: AppWindow,
    state: Option<&native_capture::RecordingSettingsState>,
) -> Result<(), String> {
    ensure_window_allowed(window, state)?;

    let config = window.config();
    if let Some(existing) = app.get_webview_window(config.label) {
        show_and_focus_window(&existing);
        return Ok(());
    }

    open_new_app_window(app, window, config.path.to_string())
}

/// Builds (but does not show) one of the app's windows pointing at `url_path`.
///
/// `url_path` is normally the window's configured path; the settings window
/// overrides it to deep-link a specific tab.
fn build_app_window(
    app: &tauri::AppHandle,
    window: AppWindow,
    url_path: &str,
) -> Result<WebviewWindow, String> {
    let config = window.config();
    let mut builder =
        WebviewWindowBuilder::new(app, config.label, WebviewUrl::App(url_path.into()));

    // macOS draws the overlay title bar over the webview (native traffic lights
    // stay), so an overlay window keeps its native decorations. Windows/Linux
    // have no overlay equivalent — an overlay window is made frameless and the
    // frontend draws the whole title bar, including its own minimize / maximize
    // / close caption controls (see `WindowsCaptionControls.svelte`).
    #[cfg(target_os = "macos")]
    let decorations = config.decorations;
    #[cfg(not(target_os = "macos"))]
    let decorations = config.decorations && !config.overlay_title_bar;

    builder = builder
        .title(config.title)
        .inner_size(config.inner_size.0, config.inner_size.1)
        .min_inner_size(config.min_inner_size.0, config.min_inner_size.1)
        .decorations(decorations)
        .transparent(config.transparent)
        .shadow(config.shadow);

    #[cfg(target_os = "macos")]
    if config.overlay_title_bar {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true);
    }

    let built = builder.build().map_err(|err| err.to_string())?;

    #[cfg(target_os = "windows")]
    {
        // `SetWindowSubclass` / `WTSRegisterSessionNotification` must run on the
        // thread that owns the window's message queue. Tauri marshals the actual
        // window creation onto the main event-loop thread, so for runtime opens
        // (which build off a worker thread to dodge the WebView2 deadlock) the
        // HWND is owned by the main thread while we are on a soon-to-exit worker.
        // Hop back to the main thread to install the subclass; calling it
        // cross-thread is not contractually guaranteed and can silently drop the
        // sleep/wake + lock/unlock session notifications transient-liveness
        // recovery depends on (ADR 0023). The synchronous startup open already
        // runs on the main thread; `run_on_main_thread` just defers a tick there.
        let built_for_registration = built.clone();
        if let Err(error) = app.run_on_main_thread(move || {
            register_windows_session_notifications(&built_for_registration);
        }) {
            crate::native_capture::debug_log::log_warn(format!(
                "failed to schedule Windows session-notification registration on the main thread: {error}"
            ));
        }
    }

    #[cfg(target_os = "macos")]
    if let Some(radius) = config.macos_corner_radius {
        apply_macos_rounded_content_view(&built, radius);
    }

    Ok(built)
}

/// Creates a brand-new app window and shows it.
///
/// On Windows, `WebviewWindowBuilder::build()` deadlocks when it runs inside a
/// synchronous command or an event-loop callback: the WebView2 controller is
/// created through a callback that needs the main event loop to keep pumping,
/// but the synchronous caller is blocking that very loop. The native window
/// frame appears while its webview never finishes initializing, so it stays a
/// blank white surface until something else forces it to reload. Building on a
/// separate thread keeps the main loop free to drive WebView2 creation, so the
/// window paints on first open. Other platforms don't have this constraint —
/// and macOS must run the Cocoa corner-radius tweak on the calling main
/// thread — so they keep building inline.
fn open_new_app_window(
    app: &tauri::AppHandle,
    window: AppWindow,
    url_path: String,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let app = app.clone();
        std::thread::spawn(move || match build_app_window(&app, window, &url_path) {
            Ok(built) => show_and_focus_window(&built),
            Err(err) => crate::native_capture::debug_log::log_error(format!(
                "failed to open {} window: {err}",
                window.config().label
            )),
        });
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let built = build_app_window(app, window, &url_path)?;
        show_and_focus_window(&built);
        Ok(())
    }
}

fn normalize_settings_tab(tab: &str) -> Option<&'static str> {
    match tab {
        "capture" | "behavior" => Some("capture"),
        "about" => Some("about"),
        "privacy" | "metadata" => Some("privacy"),
        "access" | "cliAccess" | "cli-access" => Some("access"),
        "shortcuts" | "keyboard" | "keyboard-shortcuts" | "keyboard_bindings" => Some("shortcuts"),
        "video" => Some("video"),
        "audio" | "microphone" => Some("audio"),
        // Granular processing sub-tabs pass through so a notification targeting
        // (e.g.) transcription lands on the transcription section instead of
        // being collapsed to "processing" (which the page resolves to OCR).
        "ocr" => Some("ocr"),
        "transcription" => Some("transcription"),
        "speakers" => Some("speakers"),
        "semanticSearch" | "semantic-search" => Some("semanticSearch"),
        // Legacy "processing" alias kept for back-compat (page maps it to OCR).
        "processing" => Some("processing"),
        "storage" => Some("storage"),
        "appearance" => Some("appearance"),
        "developer" => Some("developer"),
        "intelligence" | "reasoning" | "reasoning-engine" | "ai" | "ai-runtime" => {
            Some("intelligence")
        }
        // User Context has its own Intelligence-group section, so it deep-links
        // 1:1 (the page resolves "userContext" to that section) rather than
        // collapsing onto Providers.
        "user-context" | "userContext" => Some("userContext"),
        _ => None,
    }
}

#[cfg(test)]
fn is_known_settings_tab(tab: &str) -> bool {
    normalize_settings_tab(tab).is_some()
}

fn normalize_settings_focus(focus: &str) -> Option<&'static str> {
    match focus {
        "agentAccess" | "agent-access" | "cliAccess" | "cli-access" => Some("cliAccess"),
        _ => None,
    }
}

// Retained as the deeplink-contract guard (covered by tests below): proves the
// alias→canonical normalization composes into a `/settings` route URL. The
// runtime path now lives in the frontend (`settingsRoutePath` in
// `surface-windows.ts`), so this is test-only.
#[cfg(test)]
fn settings_tab_focus_path(tab: &str, focus: Option<&str>) -> Result<String, String> {
    let normalized =
        normalize_settings_tab(tab).ok_or_else(|| format!("unknown settings tab: {tab}"))?;
    let Some(focus) = focus else {
        return Ok(format!("/settings?tab={normalized}"));
    };
    let focus = normalize_settings_focus(focus)
        .ok_or_else(|| format!("unknown settings focus: {focus}"))?;
    Ok(format!("/settings?tab={normalized}&focus={focus}"))
}

// Settings now lives as the `/settings` route inside the Main window. Focus,
// show, and unminimize the Main window (the same semantics `open_main_window`
// would apply), then emit the `open_settings_tab` deeplink to it. The Main
// layout listens for this and navigates to `/settings?tab=…&focus=…`; the
// settings page reacts to the resulting URL query. Aliases are normalized here
// so the emitted payload always carries canonical values. An unknown tab/focus
// is dropped (the route falls back to its default tab) rather than erroring,
// so a stale deeplink still lands on Settings.
//
// Cold-window handoff (mirrors `open_conversation_in_chat` in `lib.rs`): when the
// Main window has to be BUILT (cold start from the tray / another window), its
// freshly-loaded webview hasn't attached its `listen("open_settings_tab")` yet
// (that registers in a `+layout.svelte` mount effect) and Tauri doesn't buffer
// events with no listener — so a synchronous emit would be dropped and strand the
// user on Timeline. We therefore queue the normalized payload into
// `PendingOpenSettingsState` when (and only when) Main was cold; the layout
// drains it on mount via `drain_pending_open_settings`. The WARM path (Main
// already open) keeps emitting directly. We still emit on the cold path too, so a
// webview that happens to attach before the drain is served by whichever fires
// first; the drain is idempotent because it consumes the queue.
fn focus_main_and_emit_open_settings(
    app: &tauri::AppHandle,
    pending: &PendingOpenSettingsState,
    tab: Option<&str>,
    focus: Option<&str>,
) -> Result<(), String> {
    // Snapshot whether Main exists BEFORE opening, so we can tell a cold build
    // apart from a warm focus. Only a cold build needs the pending queue; queuing
    // on a warm window would leave the entry stranded (the page doesn't remount,
    // so the drain never runs) and replay a stale deeplink on the next genuine
    // mount.
    let main_window_was_open = app
        .get_webview_window(AppWindow::Main.config().label)
        .is_some();

    open_or_focus_window(app, AppWindow::Main, None)?;

    let Some(main) = app.get_webview_window(AppWindow::Main.config().label) else {
        return Err("main window unavailable".into());
    };

    let payload = normalized_open_settings_payload(tab, focus);
    enqueue_cold_open_settings(pending, &payload, main_window_was_open);

    main.emit(OPEN_SETTINGS_TAB_EVENT, payload)
        .map_err(|err| err.to_string())
}

/// Normalize tab/focus aliases into the canonical `open_settings_tab` payload.
/// Unknown values are dropped (the settings route falls back to its default tab)
/// rather than erroring, so a stale deeplink still lands on Settings.
fn normalized_open_settings_payload(
    tab: Option<&str>,
    focus: Option<&str>,
) -> OpenSettingsTabPayload {
    OpenSettingsTabPayload {
        tab: tab.and_then(normalize_settings_tab).map(str::to_string),
        focus: focus.and_then(normalize_settings_focus).map(str::to_string),
    }
}

/// Queue a normalized Settings deeplink for the cold-window mount drain — but
/// ONLY when Main had to be built (`main_window_was_open == false`). Queuing on a
/// warm window would strand the entry (the page doesn't remount, so the drain
/// never runs) and replay a stale deeplink on the next genuine mount.
fn enqueue_cold_open_settings(
    pending: &PendingOpenSettingsState,
    payload: &OpenSettingsTabPayload,
    main_window_was_open: bool,
) {
    if main_window_was_open {
        return;
    }
    if let Ok(mut queue) = pending.pending.lock() {
        queue.push_back(payload.clone());
    }
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn apply_macos_rounded_content_view(window: &WebviewWindow, radius: f64) {
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::{msg_send, sel, sel_impl};

    let Ok(ns_win) = window.ns_window() else {
        return;
    };

    unsafe {
        let ns_win = ns_win as id;
        let content_view: id = msg_send![ns_win, contentView];

        if content_view == nil {
            return;
        }

        let _: () = msg_send![content_view, setWantsLayer: true];

        let layer: id = msg_send![content_view, layer];
        if layer == nil {
            return;
        }

        let _: () = msg_send![layer, setCornerRadius: radius];
        let _: () = msg_send![layer, setMasksToBounds: true];

        let continuous = NSString::alloc(nil).init_str("continuous");
        let _: () = msg_send![layer, setCornerCurve: continuous];
    }
}

// ── Quick Recall non-activating NSPanel ────────────────────────────────
// A plain NSWindow cannot become key while its owning app is inactive, so we
// reclass the tao-created window as an NSPanel subclass that reports it can
// become key, then give it the non-activating style mask + floating level +
// all-Spaces collection behavior. Summoning makes it key WITHOUT activating
// Mnema, matching Spotlight/Raycast.
#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn quick_recall_panel_class() -> *const objc::runtime::Class {
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel, BOOL, YES};
    use objc::{sel, sel_impl};
    use std::sync::OnceLock;

    static CLASS_PTR: OnceLock<usize> = OnceLock::new();
    let ptr = *CLASS_PTR.get_or_init(|| {
        extern "C" fn yes(_this: &Object, _cmd: Sel) -> BOOL {
            YES
        }

        // Suppress NSPanel's built-in "Escape dismisses the panel" so the web
        // layer owns the Escape key. By default an NSPanel closes when Escape
        // reaches `cancelOperation:` via the responder chain — and WebKit forwards
        // Escape there even when the page calls `preventDefault()` (Escape is a
        // special key), so the whole Quick Recall window was closing instead of
        // letting the launcher close just its open filter sub-surface first. With
        // this no-op override, Escape stays in the web layer: the Filter Picker /
        // Value List close themselves, and a plain-search Escape is closed by the
        // shell's own `dismissQuickRecallOnEscape` window handler.
        extern "C" fn cancel_operation(_this: &Object, _cmd: Sel, _sender: *mut Object) {}

        let superclass = objc::class!(NSPanel);
        let mut decl = ClassDecl::new("MnemaQuickRecallPanel", superclass)
            .expect("failed to declare MnemaQuickRecallPanel class");
        unsafe {
            decl.add_method(
                sel!(canBecomeKeyWindow),
                yes as extern "C" fn(&Object, Sel) -> BOOL,
            );
            decl.add_method(
                sel!(canBecomeMainWindow),
                yes as extern "C" fn(&Object, Sel) -> BOOL,
            );
            decl.add_method(
                sel!(cancelOperation:),
                cancel_operation as extern "C" fn(&Object, Sel, *mut Object),
            );
        }
        decl.register() as *const Class as usize
    });
    ptr as *const objc::runtime::Class
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn configure_quick_recall_panel(window: &WebviewWindow) {
    use cocoa::base::{id, NO, YES};
    use objc::runtime::Class;
    use objc::{msg_send, sel, sel_impl};

    const NS_WINDOW_STYLE_MASK_NONACTIVATING_PANEL: u64 = 1 << 7;
    const NS_WINDOW_COLLECTION_BEHAVIOR_CAN_JOIN_ALL_SPACES: u64 = 1 << 0;
    const NS_WINDOW_COLLECTION_BEHAVIOR_TRANSIENT: u64 = 1 << 3;
    const NS_WINDOW_COLLECTION_BEHAVIOR_FULL_SCREEN_AUXILIARY: u64 = 1 << 8;
    const NS_FLOATING_WINDOW_LEVEL: i64 = 3; // NSFloatingWindowLevel

    unsafe extern "C" {
        fn object_setClass(obj: id, cls: *const Class) -> *const Class;
    }

    let Ok(ns_window) = window.ns_window() else {
        return;
    };

    unsafe {
        let ns_window = ns_window as id;
        let _: () = msg_send![ns_window, setReleasedWhenClosed: NO];
        object_setClass(ns_window, quick_recall_panel_class());

        let mut style_mask: u64 = msg_send![ns_window, styleMask];
        style_mask |= NS_WINDOW_STYLE_MASK_NONACTIVATING_PANEL;
        let _: () = msg_send![ns_window, setStyleMask: style_mask];

        let _: () = msg_send![ns_window, setLevel: NS_FLOATING_WINDOW_LEVEL];
        let _: () = msg_send![
            ns_window,
            setCollectionBehavior: NS_WINDOW_COLLECTION_BEHAVIOR_CAN_JOIN_ALL_SPACES
                | NS_WINDOW_COLLECTION_BEHAVIOR_TRANSIENT
                | NS_WINDOW_COLLECTION_BEHAVIOR_FULL_SCREEN_AUXILIARY
        ];
        let _: () = msg_send![ns_window, setHidesOnDeactivate: NO];
        let _: () = msg_send![ns_window, setBecomesKeyOnlyIfNeeded: NO];
        let _: () = msg_send![ns_window, setFloatingPanel: YES];
    }
}

// A non-activating panel is summoned while Mnema itself stays inactive, so the
// first click into its WKWebView is an AppKit "first mouse". By default AppKit
// swallows that click just to order the window forward, so the very first press
// of a Quick Recall control (e.g. the Ask AI button) never reaches the web layer
// and WebKit surfaces its context menu instead. Teaching the webview to return
// YES from `acceptsFirstMouse:` delivers that click straight through as a normal
// click.
//
// We add the method to the live webview class with `class_addMethod` rather than
// swapping the instance's class via `object_setClass`: WKWebView relies on its
// own class for KVO / dynamic-property resolution, and reclassing it trips an
// `NSDynamicProperties` assertion (`NSDP_getComputedPropertyValue`). Adding a
// method override leaves the class identity intact.
#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn configure_quick_recall_webview(window: &WebviewWindow) {
    use cocoa::base::{id, BOOL, YES};
    use objc::runtime::{Class, Object, Sel};
    use objc::{sel, sel_impl};
    use std::os::raw::c_char;

    extern "C" fn accepts_first_mouse(_this: &Object, _cmd: Sel, _event: *mut Object) -> BOOL {
        YES
    }

    unsafe extern "C" {
        fn object_getClass(obj: id) -> *mut Class;
        fn class_addMethod(
            cls: *mut Class,
            name: Sel,
            imp: extern "C" fn(&Object, Sel, *mut Object) -> BOOL,
            types: *const c_char,
        ) -> BOOL;
    }

    let _ = window.with_webview(|webview| unsafe {
        let wv = webview.inner() as id;
        if wv.is_null() {
            return;
        }
        let class = object_getClass(wv);
        if class.is_null() {
            return;
        }
        // Objective-C type encoding: BOOL return (`c`), self (`@`), _cmd (`:`),
        // NSEvent* argument (`@`). A no-op if the method is already present.
        class_addMethod(
            class,
            sel!(acceptsFirstMouse:),
            accepts_first_mouse,
            c"c@:@".as_ptr(),
        );
    });
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn make_quick_recall_panel_key(window: &WebviewWindow) {
    use cocoa::base::{id, nil};
    use objc::{msg_send, sel, sel_impl};
    let Ok(ns_window) = window.ns_window() else {
        return;
    };
    unsafe {
        let ns_window = ns_window as id;
        let _: () = msg_send![ns_window, makeKeyAndOrderFront: nil];
    }
}

// Promote the WKWebView to the panel's first responder so keyboard focus (and the
// search field's JS `.focus()`) actually lands. This must run *after* the webview
// has loaded: on the first summon the panel is made key while the webview is still
// loading, so AppKit never routes focus into it and the field opens without a
// caret. The frontend calls `focus_quick_recall_window` once mounted (and on every
// re-summon) so this runs against a ready webview.
#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn make_quick_recall_webview_first_responder(window: &WebviewWindow) {
    use cocoa::base::{id, nil, BOOL};
    use objc::{msg_send, sel, sel_impl};
    let _ = window.with_webview(|webview| unsafe {
        let wv = webview.inner() as id;
        if wv.is_null() {
            return;
        }
        let panel: id = msg_send![wv, window];
        if panel != nil {
            let _: BOOL = msg_send![panel, makeFirstResponder: wv];
        }
    });
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn order_out_quick_recall_panel(window: &WebviewWindow) {
    use cocoa::base::{id, nil};
    use objc::{msg_send, sel, sel_impl};
    let Ok(ns_window) = window.ns_window() else {
        let _ = window.hide();
        return;
    };
    unsafe {
        let ns_window = ns_window as id;
        let _: () = msg_send![ns_window, orderOut: nil];
    }
}

fn build_quick_recall_window(app: &tauri::AppHandle) -> Result<WebviewWindow, String> {
    let config = AppWindow::QuickRecall.config();
    let built = WebviewWindowBuilder::new(app, config.label, WebviewUrl::App(config.path.into()))
        .title(config.title)
        .inner_size(config.inner_size.0, config.inner_size.1)
        .min_inner_size(config.min_inner_size.0, config.min_inner_size.1)
        .decorations(config.decorations)
        .transparent(config.transparent)
        .shadow(config.shadow)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
        .map_err(|err| err.to_string())?;

    #[cfg(target_os = "macos")]
    {
        configure_quick_recall_panel(&built);
        configure_quick_recall_webview(&built);
        if let Some(radius) = config.macos_corner_radius {
            apply_macos_rounded_content_view(&built, radius);
        }
    }

    Ok(built)
}

fn summon_quick_recall_window(window: &WebviewWindow) {
    // Record the summon instant so the `Focused(false)` handler can ignore the
    // transient first-responder-setup blur that fires right after a fresh summon
    // (see `QUICK_RECALL_SUMMON_BLUR_GRACE`).
    if let Ok(mut last) = LAST_QUICK_RECALL_SUMMON.lock() {
        *last = Some(Instant::now());
    }
    let _ = window.center();
    let _ = window.show();
    #[cfg(target_os = "macos")]
    make_quick_recall_panel_key(window);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window.set_focus();
    }
}

fn dismiss_quick_recall_window(window: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    order_out_quick_recall_panel(window);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window.hide();
    }
    // The webview is hidden, not destroyed, so the Svelte `onDestroy` teardown
    // never runs on dismiss. Notify the panel so it can decide whether to cancel
    // its Ask AI PI session.
    //
    // Contract (PLAN.md "Ask AI seen" — background completion): the panel no
    // longer cancels unconditionally on this event. If an Ask AI conversation is
    // still in flight or finished-but-unseen, the panel intentionally KEEPS the
    // PI session resident and the conversation alive so a re-summon lands back on
    // it; the session is torn down only once the conversation is seen (or there
    // is none). A resident PI session after dismiss is therefore expected, not a
    // leak. It is bounded by a 30-minute unseen cap (implemented in a later
    // slice, owned frontend-side) and by app exit / the panel's `onDestroy`.
    let _ = window.emit(QUICK_RECALL_DISMISSED_EVENT, ());
}

// Decide whether a Quick Recall `Focused(false)` should dismiss the launcher.
// Two transient blurs must NOT dismiss it:
//   (a) the first-summon blur: AppKit makes the non-activating panel key before
//       its webview is first responder, firing a spurious `Focused(false)`; a
//       blur within `QUICK_RECALL_SUMMON_BLUR_GRACE` of the last summon is that
//       setup blur, not a click-away.
//   (b) an answer-link click: `handleAnswerClick` flags
//       `SUPPRESS_NEXT_QUICK_RECALL_BLUR_DISMISS` right before activating the OS
//       browser, so the browser-activation blur is consumed here instead of
//       tearing down the in-flight Ask AI session.
// Both guards are one-shot/time-bounded, so ordinary click-away dismissal still
// fires immediately.
fn should_dismiss_quick_recall_on_blur() -> bool {
    if SUPPRESS_NEXT_QUICK_RECALL_BLUR_DISMISS.swap(false, Ordering::SeqCst) {
        return false;
    }

    if let Ok(last) = LAST_QUICK_RECALL_SUMMON.lock() {
        if let Some(summoned_at) = *last {
            if summoned_at.elapsed() < QUICK_RECALL_SUMMON_BLUR_GRACE {
                return false;
            }
        }
    }

    true
}

pub(crate) fn toggle_quick_recall_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window(QUICK_RECALL_WINDOW_LABEL) {
        if existing.is_visible().unwrap_or(false) {
            dismiss_quick_recall_window(&existing);
        } else {
            summon_quick_recall_window(&existing);
        }
        return Ok(());
    }
    // On Windows, `WebviewWindowBuilder::build()` deadlocks when it runs inside a
    // synchronous command or an event-loop callback (see `open_new_app_window`):
    // this path is reached from both `summon_quick_recall_window_command` (a sync
    // command) and `handle_global_shortcut` (an event-loop callback). Build off the
    // main loop so WebView2 controller creation can finish, mirroring
    // `open_new_app_window`. Other platforms build inline — and macOS must run its
    // Cocoa panel configuration (in `build_quick_recall_window`) on the calling
    // main thread.
    #[cfg(target_os = "windows")]
    {
        let app = app.clone();
        std::thread::spawn(move || match build_quick_recall_window(&app) {
            Ok(window) => summon_quick_recall_window(&window),
            Err(err) => crate::native_capture::debug_log::log_error(format!(
                "failed to open quick recall window: {err}"
            )),
        });
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let window = build_quick_recall_window(app)?;
        summon_quick_recall_window(&window);
        Ok(())
    }
}

fn focus_main_window_if_visible(app: &tauri::AppHandle) {
    if let Some(main) = app.get_webview_window(AppWindow::Main.config().label) {
        if main.is_visible().unwrap_or(false) {
            show_and_focus_window(&main);
        }
    }
}

fn hide_main_window(window: &WebviewWindow) {
    let _ = window.hide();
    refresh_macos_dock_icon_visibility(window.app_handle());
}

pub(crate) fn toggle_main_window_visibility(app: &tauri::AppHandle) {
    let config = AppWindow::Main.config();
    if let Some(main) = app.get_webview_window(config.label) {
        let visible = main.is_visible().unwrap_or(false);
        let focused = main.is_focused().unwrap_or(false);
        if visible && focused {
            hide_main_window(&main);
        } else {
            show_and_focus_window(&main);
        }
        return;
    }

    let _ = open_main_window(app);
}

#[cfg(target_os = "macos")]
fn show_macos_dock_icon(app: &tauri::AppHandle) {
    // Tauri's Dock visibility path debounces rapid hide/show transitions that
    // can otherwise leave duplicate Dock icons on macOS.
    let _ = app.set_dock_visibility(true);
}

#[cfg(not(target_os = "macos"))]
fn show_macos_dock_icon(_app: &tauri::AppHandle) {}

#[cfg(target_os = "macos")]
fn refresh_macos_dock_icon_visibility(app: &tauri::AppHandle) {
    let has_visible_window = app
        .webview_windows()
        .values()
        .filter(|window| window.label() != QUICK_RECALL_WINDOW_LABEL)
        .any(|window| window.is_visible().unwrap_or(false));
    let _ = app.set_dock_visibility(has_visible_window);
}

#[cfg(not(target_os = "macos"))]
fn refresh_macos_dock_icon_visibility(_app: &tauri::AppHandle) {}

#[cfg(target_os = "macos")]
pub(crate) fn install_macos_terminate_handler(app: &tauri::AppHandle) {
    use objc::{
        class,
        runtime::{class_getInstanceMethod, method_setImplementation, Object, Sel},
        sel, sel_impl,
    };

    let _ = MACOS_TERMINATE_APP_HANDLE.set(app.clone());
    if MACOS_TERMINATE_HANDLER_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    unsafe extern "C" fn terminate(_application: &Object, _cmd: Sel, _sender: *mut Object) {
        if let Some(app) = MACOS_TERMINATE_APP_HANDLE.get() {
            if is_final_graceful_exit_ready(app) {
                macos_immediate_process_exit(0);
            }

            request_graceful_exit(app);
        } else {
            macos_immediate_process_exit(0);
        }
    }

    unsafe {
        let method = class_getInstanceMethod(class!(NSApplication), sel!(terminate:));
        if method.is_null() {
            crate::native_capture::debug_log::log_error(
                "failed to install macOS terminate handler: NSApplication terminate: method not found",
            );
            return;
        }

        method_setImplementation(
            method.cast_mut(),
            std::mem::transmute(terminate as *const ()),
        );
        crate::native_capture::debug_log::log_info(
            "installed macOS terminate handler for graceful app exit",
        );
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn install_macos_terminate_handler(_app: &tauri::AppHandle) {}

pub(crate) fn request_graceful_exit(app: &tauri::AppHandle) {
    request_graceful_exit_with_completion(app, false);
}

pub(crate) fn request_graceful_restart_after_update(app: &tauri::AppHandle) {
    request_graceful_exit_with_completion(app, true);
}

/// General-purpose **relaunch** for a settings change that only takes effect
/// after a restart (e.g. a new capture save directory). UNGUARDED by design:
/// unlike the update restart (`restart_after_app_update`), which REFUSES while a
/// recording is active, this always proceeds — the graceful path finalizes any
/// in-flight capture before `app.restart()`, so the recording is saved, never
/// lost. Keep these paths separate; do NOT route a settings relaunch through the
/// update guard.
#[tauri::command]
pub fn request_app_relaunch(app_handle: tauri::AppHandle) {
    request_graceful_exit_with_completion(&app_handle, true);
}

fn request_graceful_exit_with_completion(app: &tauri::AppHandle, restart_after_exit: bool) {
    let exit_state = app.state::<AppExitCoordinatorState>();
    if !exit_state.begin_exit(restart_after_exit) {
        return;
    }

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::native_capture::debug_log::log_info(if restart_after_exit {
            "starting graceful app relaunch; stopping capture and background workers before unloading cached Local Whisper contexts"
        } else {
            "starting graceful app exit; stopping capture and background workers before unloading cached Local Whisper contexts"
        });

        let stop_app_handle = app_handle.clone();
        match tauri::async_runtime::spawn_blocking(move || {
            if crate::native_capture::current_native_capture_session(&stop_app_handle).is_running {
                Some(crate::native_capture::stop_native_capture_from_app_handle(
                    &stop_app_handle,
                ))
            } else {
                None
            }
        })
        .await
        {
            Ok(Some(Ok(_))) => crate::native_capture::debug_log::log_info(
                "stopped active native capture during graceful app exit",
            ),
            Ok(Some(Err(error))) => crate::native_capture::debug_log::log_error(format!(
                "failed to stop active native capture during graceful app exit: [{}] {}",
                error.code, error.message
            )),
            Ok(None) => {}
            Err(error) => {
                crate::native_capture::debug_log::log_error(format!(
                    "failed to join native capture stop during graceful app exit: {error}"
                ));
            }
        }
        crate::native_capture::debug_log::log_info(
            "stopping background workers before terminating",
        );
        crate::app_infra::shutdown_background_workers_for_app_exit(&app_handle).await;

        match audio_transcription::providers::local_whisper::unload_all_cached_contexts() {
            Ok(unloaded) => {
                crate::native_capture::debug_log::log_info(format!(
                    "unloaded {unloaded} cached Local Whisper context(s) after background worker shutdown"
                ));
            }
            Err(error) => {
                crate::native_capture::debug_log::log_warn(format!(
                    "failed to unload cached Local Whisper contexts after background worker shutdown: {error}"
                ));
            }
        }

        complete_graceful_exit(&app_handle);
    });
}

fn complete_graceful_exit(app: &tauri::AppHandle) {
    let exit_state = app.state::<AppExitCoordinatorState>();
    let restart_requested = exit_state.should_restart_after_graceful_exit();
    exit_state.mark_final_graceful_exit_ready();

    // Release the Windows foreground-change listener (WinEvent hook + its thread)
    // before the process exits or restarts, so neither leaks (ADR 0043, issue #141).
    // Runs here (not on the listener thread) so its WM_QUIT signal + join complete.
    #[cfg(target_os = "windows")]
    crate::native_capture::foreground_listener::stop_windows_foreground_listener();

    if restart_requested {
        crate::native_capture::debug_log::log_info(
            "completed graceful app exit; relaunching",
        );
        app.restart();
    }

    #[cfg(target_os = "macos")]
    {
        crate::native_capture::debug_log::log_info(
            "completed graceful app exit; exiting without running process static destructors",
        );
        macos_immediate_process_exit(0);
    }

    #[cfg(not(target_os = "macos"))]
    {
        app.exit(0);
    }
}

#[cfg(target_os = "macos")]
fn macos_immediate_process_exit(code: i32) -> ! {
    unsafe extern "C" {
        fn _exit(status: std::os::raw::c_int) -> !;
    }

    unsafe { _exit(code) }
}

pub(crate) fn is_graceful_exit_in_progress(app: &tauri::AppHandle) -> bool {
    app.state::<AppExitCoordinatorState>().is_exit_requested()
}

pub(crate) fn is_final_graceful_exit_ready(app: &tauri::AppHandle) -> bool {
    app.state::<AppExitCoordinatorState>()
        .is_final_graceful_exit_ready()
}

fn destroyed_window_action(label: &str) -> DestroyedWindowAction {
    match AppWindow::from_label(label) {
        Some(AppWindow::Onboarding) => DestroyedWindowAction::ExitApp,
        Some(AppWindow::Debug) => DestroyedWindowAction::FocusMainWindow,
        Some(AppWindow::CliAccessRequest | AppWindow::QuickRecall) => DestroyedWindowAction::None,
        Some(AppWindow::Main) => DestroyedWindowAction::ExitApp,
        None => DestroyedWindowAction::None,
    }
}

fn close_window(window: WebviewWindow) -> Result<(), String> {
    let label = window.label().to_string();
    if close_window_focuses_main_before_close(&label) {
        focus_main_window_if_visible(window.app_handle());
    }

    match AppWindow::from_label(&label) {
        Some(AppWindow::QuickRecall) => {
            dismiss_quick_recall_window(&window);
            Ok(())
        }
        Some(
            AppWindow::Onboarding | AppWindow::CliAccessRequest | AppWindow::Debug,
        ) => window.close().map_err(|err| err.to_string()),
        Some(AppWindow::Main) => Err("main window cannot be closed from this command".into()),
        None => window.close().map_err(|err| err.to_string()),
    }
}

fn close_window_focuses_main_before_close(label: &str) -> bool {
    matches!(AppWindow::from_label(label), Some(AppWindow::Debug))
}

pub fn handle_window_event(
    app: &tauri::AppHandle,
    label: &str,
    event: &WindowEvent,
    window: Option<&WebviewWindow>,
) {
    if let WindowEvent::Focused(false) = event {
        crate::webview_cache::purge_webview_memory_cache_on_blur(app);
        if AppWindow::from_label(label) == Some(AppWindow::QuickRecall) {
            if let Some(window) = window {
                if should_dismiss_quick_recall_on_blur() {
                    dismiss_quick_recall_window(window);
                }
            }
        }
        return;
    }

    if let WindowEvent::CloseRequested { api, .. } = event {
        if AppWindow::from_label(label) == Some(AppWindow::Main) {
            api.prevent_close();
            if let Some(window) = window {
                hide_main_window(window);
            }
        }
        return;
    }

    if !matches!(event, WindowEvent::Destroyed) {
        return;
    }

    let action = match AppWindow::from_label(label) {
        Some(AppWindow::Onboarding) => {
            let store = app.state::<OnboardingStateStore>();
            if current_onboarding_state(app, store.inner()).is_complete() {
                DestroyedWindowAction::None
            } else {
                DestroyedWindowAction::ExitApp
            }
        }
        _ => destroyed_window_action(label),
    };

    match action {
        DestroyedWindowAction::FocusMainWindow => focus_main_window_if_visible(app),
        DestroyedWindowAction::ExitApp => request_graceful_exit(app),
        DestroyedWindowAction::None => {}
    }
    refresh_macos_dock_icon_visibility(app);
}

pub fn open_startup_window(
    app: &tauri::AppHandle,
    store: &OnboardingStateStore,
) -> Result<bool, String> {
    let state = current_onboarding_state(app, store);
    let window = if state.is_complete() {
        AppWindow::Main
    } else {
        AppWindow::Onboarding
    };

    // Build synchronously here: this runs from `setup()` before the event loop
    // starts blocking, so the Windows WebView2 deadlock that `open_new_app_window`
    // guards against doesn't apply, and a synchronous build guarantees a window
    // exists before the loop begins.
    let built = build_app_window(app, window, window.config().path)?;
    show_and_focus_window(&built);

    Ok(state.is_complete())
}

pub(crate) fn is_onboarding_complete(app: &tauri::AppHandle) -> bool {
    let store = app.state::<OnboardingStateStore>();
    current_onboarding_state(app, store.inner()).is_complete()
}

/// Focus the Main window and ask it to open the `/settings` route. Settings is
/// no longer a dedicated window; callers outside the Main window (the tray,
/// Quick Recall, …) invoke this so Rust focuses Main and emits the
/// `open_settings_tab` deeplink that the Main layout turns into a `/settings`
/// navigation. `tab`/`focus` are optional aliases, normalized before emit. A cold
/// start that has to BUILD Main also queues the normalized payload into
/// `PendingOpenSettingsState` so the layout's on-mount drain still lands the
/// deeplink even though the live event fires before the listener attaches.
///
/// The managed pending state is resolved from `app` rather than taken as a
/// `tauri::State` parameter so this stays directly callable from Rust — the tray's
/// "Open Settings" handler (`status_bar.rs`) invokes it with an `AppHandle`, not
/// over IPC. (That tray path is the very cold-start case the pending queue
/// fixes.)
#[tauri::command]
pub fn focus_main_and_open_settings(
    app: tauri::AppHandle,
    tab: Option<String>,
    focus: Option<String>,
) -> Result<(), String> {
    let pending = app.state::<PendingOpenSettingsState>();
    focus_main_and_emit_open_settings(&app, pending.inner(), tab.as_deref(), focus.as_deref())
}

/// Drain any queued Settings deeplink(s) for a cold main window. The Main layout
/// calls this once on mount: a freshly-built main window boots on Timeline and
/// the live `open_settings_tab` event may have already fired before the layout's
/// listener attached, so the queued payload is the only way the cold-start tray
/// "Open Settings" reaches `/settings`. Mirrors
/// `drain_pending_insights_open_conversations` in `lib.rs`. Returns the queued
/// payloads (normally at most one) in arrival order; an empty vec means nothing
/// was pending (warm window, or the lock was poisoned).
#[tauri::command]
pub fn drain_pending_open_settings(
    pending: tauri::State<'_, PendingOpenSettingsState>,
) -> Vec<OpenSettingsTabPayload> {
    let Ok(mut queue) = pending.pending.lock() else {
        return Vec::new();
    };
    queue.drain(..).collect()
}

pub(crate) fn open_cli_access_request_window(app: &tauri::AppHandle) -> Result<(), String> {
    open_or_focus_window(app, AppWindow::CliAccessRequest, None)
}

#[tauri::command]
pub fn open_debug_window(
    app: tauri::AppHandle,
    state: tauri::State<'_, native_capture::RecordingSettingsState>,
) -> Result<(), String> {
    open_or_focus_window(&app, AppWindow::Debug, Some(state.inner()))
}

#[tauri::command]
pub fn close_current_window(window: WebviewWindow) -> Result<(), String> {
    close_window(window)
}

/// Re-assert keyboard focus for the Quick Recall window from the web layer once
/// it is mounted. On the first summon the panel is made key before its webview
/// finishes loading, so focus never reaches the search field; the frontend calls
/// this after mount (and on every re-summon) to route focus into the now-ready
/// webview.
#[tauri::command]
pub fn focus_quick_recall_window(window: WebviewWindow) {
    if window.label() != QUICK_RECALL_WINDOW_LABEL {
        return;
    }
    #[cfg(target_os = "macos")]
    make_quick_recall_webview_first_responder(&window);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window.set_focus();
    }
}

/// Suppress the very next Quick Recall blur-dismiss. The frontend calls this from
/// `handleAnswerClick` immediately before opening an answer link in the OS
/// browser: that activation blurs the non-activating panel, and without this
/// one-shot flag the resulting `Focused(false)` would dismiss the launcher and
/// tear down the in-flight Ask AI session the user is reading. Only the next blur
/// consumes the flag, so ordinary click-away dismissal still works.
#[tauri::command]
pub fn quick_recall_suppress_blur_dismiss() {
    SUPPRESS_NEXT_QUICK_RECALL_BLUR_DISMISS.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub fn toggle_main_window_visibility_command(app: tauri::AppHandle) {
    toggle_main_window_visibility(&app);
}

/// Summon (toggle) the Quick Recall panel from in-app UI — the titlebar
/// "Search / Recall" affordance. Mirrors what the ⌥Space global shortcut does so
/// the launcher is discoverable by mouse, not only by chord.
#[tauri::command]
pub fn summon_quick_recall_window_command(app: tauri::AppHandle) -> Result<(), String> {
    toggle_quick_recall_window(&app)
}

#[tauri::command]
pub fn get_onboarding_state(
    app: tauri::AppHandle,
    state: tauri::State<'_, OnboardingStateStore>,
) -> OnboardingStateView {
    // `recording-settings.json` is written ONLY by explicit user saves (never at
    // install/startup), so its existence == "the user has saved settings at least
    // once" == returning user. It must be read LIVE per call (NOT cached in
    // OnboardingStateStore) so a save performed between two onboarding entries in
    // one process lifetime is visible. Use the same path resolver saves land in.
    let ever_saved = crate::native_capture::settings::recording_settings_file_path(&app).exists();
    OnboardingStateView::from_state_and_disk(
        current_onboarding_state(&app, state.inner()),
        ever_saved,
    )
}

#[tauri::command]
pub fn complete_onboarding(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, OnboardingStateStore>,
) -> Result<(), String> {
    if AppWindow::from_label(window.label()) != Some(AppWindow::Onboarding) {
        return Err("onboarding can only be completed from the onboarding window".into());
    }

    persist_onboarding_state(&app, state.inner(), OnboardingState::completed_now())?;
    crate::status_bar::refresh(&app);
    // Open the main window, then close onboarding once main exists so the app
    // never momentarily drops to zero windows. On Windows the build must run
    // off the event-loop thread (see `open_new_app_window`), so the close is
    // sequenced after the build on that same worker thread.
    open_main_window_then_close(&app, AppWindow::Onboarding.config().label);
    Ok(())
}

fn open_main_window_then_close(app: &tauri::AppHandle, close_label: &'static str) {
    fn build_show_and_close(app: &tauri::AppHandle, close_label: &str) {
        let main = AppWindow::Main;
        match build_app_window(app, main, main.config().path) {
            Ok(built) => {
                show_and_focus_window(&built);
                if let Some(previous) = app.get_webview_window(close_label) {
                    let _ = previous.close();
                }
            }
            Err(err) => crate::native_capture::debug_log::log_error(format!(
                "failed to open main window after onboarding: {err}"
            )),
        }
    }

    #[cfg(target_os = "windows")]
    {
        let app = app.clone();
        std::thread::spawn(move || build_show_and_close(&app, close_label));
    }

    #[cfg(not(target_os = "windows"))]
    build_show_and_close(app, close_label);
}

#[cfg(test)]
mod tests {
    use super::{
        close_window_focuses_main_before_close, destroyed_window_action, enqueue_cold_open_settings,
        is_known_settings_tab, load_onboarding_state_from_path, normalize_settings_focus,
        normalize_settings_tab, normalized_open_settings_payload, settings_tab_focus_path,
        AppExitCoordinatorState, DestroyedWindowAction, OnboardingState, OnboardingStateView,
        OpenSettingsTabPayload, PendingOpenSettingsState,
    };
    #[cfg(target_os = "windows")]
    use super::{
        decode_console_display_state, windows_power_broadcast_event, ConsoleDisplayState,
        PowerBroadcastSetting, WindowsPowerBroadcastEvent, GUID_CONSOLE_DISPLAY_STATE,
    };
    #[cfg(target_os = "windows")]
    use windows_sys::core::GUID;
    #[cfg(target_os = "windows")]
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PBT_APMBATTERYLOW, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMECRITICAL, PBT_APMRESUMESTANDBY,
        PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE,
    };

    // Suspend/resume ignore `lparam`, so a null (`0`) `lparam` keeps these decode
    // assertions pure; only the `PBT_POWERSETTINGCHANGE` arm reads `lparam`.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_power_broadcast_suspend_maps_to_system_suspend() {
        assert_eq!(
            unsafe { windows_power_broadcast_event(PBT_APMSUSPEND as usize, 0) },
            Some(WindowsPowerBroadcastEvent::Suspend)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_power_broadcast_resume_variants_map_to_system_resume() {
        for event in [
            PBT_APMRESUMEAUTOMATIC,
            PBT_APMRESUMECRITICAL,
            PBT_APMRESUMESTANDBY,
            PBT_APMRESUMESUSPEND,
        ] {
            assert_eq!(
                unsafe { windows_power_broadcast_event(event as usize, 0) },
                Some(WindowsPowerBroadcastEvent::Resume)
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_power_broadcast_ignores_unrelated_power_events() {
        assert_eq!(
            unsafe { windows_power_broadcast_event(PBT_APMBATTERYLOW as usize, 0) },
            None
        );
    }

    // PBT-decode → display on/off mapping (the pure core the plan requires).
    #[cfg(target_os = "windows")]
    #[test]
    fn decode_console_display_state_maps_known_bytes() {
        assert_eq!(
            decode_console_display_state(0),
            Some(ConsoleDisplayState::Off)
        );
        assert_eq!(decode_console_display_state(1), Some(ConsoleDisplayState::On));
        assert_eq!(
            decode_console_display_state(2),
            Some(ConsoleDisplayState::Dimmed)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn decode_console_display_state_rejects_unexpected_bytes() {
        assert_eq!(decode_console_display_state(3), None);
        assert_eq!(decode_console_display_state(255), None);
        assert_eq!(decode_console_display_state(u32::MAX), None);
    }

    // The `PBT_POWERSETTINGCHANGE` arm reads the `POWERBROADCAST_SETTING` behind
    // `lparam`: it must match the console-display GUID and decode the state byte.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_power_broadcast_decodes_console_display_transitions() {
        for (byte, expected) in [
            (0u8, ConsoleDisplayState::Off),
            (1, ConsoleDisplayState::On),
            (2, ConsoleDisplayState::Dimmed),
        ] {
            let setting = PowerBroadcastSetting {
                power_setting: GUID_CONSOLE_DISPLAY_STATE,
                data_length: 1,
                data: [byte],
            };
            let event = unsafe {
                windows_power_broadcast_event(
                    PBT_POWERSETTINGCHANGE as usize,
                    &setting as *const PowerBroadcastSetting as isize,
                )
            };
            assert_eq!(
                event,
                Some(WindowsPowerBroadcastEvent::DisplayPowerChanged(expected))
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_power_broadcast_ignores_power_setting_change_with_null_payload() {
        assert_eq!(
            unsafe { windows_power_broadcast_event(PBT_POWERSETTINGCHANGE as usize, 0) },
            None
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_power_broadcast_ignores_other_power_setting_guids() {
        // A non-console-display power setting (arbitrary GUID) must not be decoded
        // as a display transition.
        let other = GUID::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0001);
        let setting = PowerBroadcastSetting {
            power_setting: other,
            data_length: 1,
            data: [1],
        };
        let event = unsafe {
            windows_power_broadcast_event(
                PBT_POWERSETTINGCHANGE as usize,
                &setting as *const PowerBroadcastSetting as isize,
            )
        };
        assert_eq!(event, None);
    }

    #[test]
    fn secondary_window_destruction_refocuses_main_window() {
        assert_eq!(
            destroyed_window_action("debug"),
            DestroyedWindowAction::FocusMainWindow
        );
    }

    #[test]
    fn settings_is_no_longer_a_known_window_label() {
        // Settings folded into the `/settings` route inside the Main window, so
        // the dedicated `settings` window label no longer maps to any AppWindow
        // and its destruction has no window-level side effect.
        assert_eq!(
            destroyed_window_action("settings"),
            DestroyedWindowAction::None
        );
        assert!(!close_window_focuses_main_before_close("settings"));
    }

    #[test]
    fn cli_access_request_destruction_does_not_refocus_main_window() {
        assert_eq!(
            destroyed_window_action("cli-access-request"),
            DestroyedWindowAction::None
        );
    }

    #[test]
    fn cli_access_request_close_command_does_not_refocus_main_window() {
        assert!(close_window_focuses_main_before_close("debug"));
        assert!(!close_window_focuses_main_before_close(
            "cli-access-request"
        ));
    }

    #[test]
    fn quick_recall_destruction_has_no_side_effect() {
        assert_eq!(
            destroyed_window_action("quick-recall"),
            DestroyedWindowAction::None
        );
    }

    #[test]
    fn main_window_destruction_exits_the_app() {
        assert_eq!(
            destroyed_window_action("main"),
            DestroyedWindowAction::ExitApp
        );
    }

    #[test]
    fn onboarding_window_destruction_exits_by_default() {
        assert_eq!(
            destroyed_window_action("onboarding"),
            DestroyedWindowAction::ExitApp
        );
    }

    #[test]
    fn unknown_window_destruction_has_no_side_effect() {
        assert_eq!(
            destroyed_window_action("other"),
            DestroyedWindowAction::None
        );
    }

    #[test]
    fn app_exit_coordinator_marks_final_exit_only_after_graceful_work_is_done() {
        let coordinator = AppExitCoordinatorState::default();

        assert!(coordinator.begin_exit(false));
        assert!(!coordinator.begin_exit(false));
        assert!(coordinator.is_exit_requested());
        assert!(!coordinator.is_final_graceful_exit_ready());

        coordinator.mark_final_graceful_exit_ready();

        assert!(coordinator.is_final_graceful_exit_ready());
    }

    #[test]
    fn restart_intent_is_preserved_when_requested_after_exit_begins() {
        let coordinator = AppExitCoordinatorState::default();

        // A plain graceful exit (quit / window close) starts first.
        assert!(coordinator.begin_exit(false));
        assert!(!coordinator.should_restart_after_graceful_exit());

        // The user triggers restart-to-update before shutdown finishes; the
        // request is dropped for spawning purposes but its intent is retained.
        assert!(!coordinator.begin_exit(true));
        assert!(coordinator.should_restart_after_graceful_exit());
    }

    #[test]
    fn restart_intent_is_not_downgraded_by_a_later_plain_exit() {
        let coordinator = AppExitCoordinatorState::default();

        assert!(coordinator.begin_exit(true));
        assert!(coordinator.should_restart_after_graceful_exit());

        // A subsequent plain exit must not cancel the pending update restart.
        assert!(!coordinator.begin_exit(false));
        assert!(coordinator.should_restart_after_graceful_exit());
    }

    #[test]
    fn settings_tab_deeplink_accepts_known_tabs_only() {
        assert!(is_known_settings_tab("processing"));
        assert!(is_known_settings_tab("about"));
        assert!(is_known_settings_tab("transcription"));
        assert!(is_known_settings_tab("microphone"));
        assert!(is_known_settings_tab("capture"));
        assert!(is_known_settings_tab("privacy"));
        assert!(is_known_settings_tab("shortcuts"));
        assert!(!is_known_settings_tab("transcripts"));
        assert!(!is_known_settings_tab("../developer"));
    }

    #[test]
    fn settings_tab_aliases_normalize_to_canonical_tabs() {
        // Granular processing sub-tabs pass through (no longer collapsed to
        // "processing") so notifications can target a specific section.
        assert_eq!(normalize_settings_tab("ocr"), Some("ocr"));
        assert_eq!(normalize_settings_tab("transcription"), Some("transcription"));
        assert_eq!(normalize_settings_tab("speakers"), Some("speakers"));
        // Legacy "processing" alias is still accepted for back-compat.
        assert_eq!(normalize_settings_tab("processing"), Some("processing"));
        assert_eq!(normalize_settings_tab("microphone"), Some("audio"));
        assert_eq!(normalize_settings_tab("behavior"), Some("capture"));
        assert_eq!(normalize_settings_tab("metadata"), Some("privacy"));
        assert_eq!(normalize_settings_tab("shortcuts"), Some("shortcuts"));
        assert_eq!(normalize_settings_tab("keyboard"), Some("shortcuts"));
        assert_eq!(
            normalize_settings_tab("keyboard-shortcuts"),
            Some("shortcuts")
        );
        assert_eq!(
            normalize_settings_tab("keyboard_bindings"),
            Some("shortcuts")
        );
        assert_eq!(normalize_settings_tab("about"), Some("about"));
        assert_eq!(normalize_settings_tab("intelligence"), Some("intelligence"));
        assert_eq!(normalize_settings_tab("reasoning"), Some("intelligence"));
        assert_eq!(
            normalize_settings_tab("reasoning-engine"),
            Some("intelligence")
        );
        assert_eq!(normalize_settings_tab("ai"), Some("intelligence"));
        assert_eq!(normalize_settings_tab("ai-runtime"), Some("intelligence"));
        // User Context and Semantic Search deep-link 1:1 to their own sections
        // rather than collapsing onto Providers / OCR.
        assert_eq!(normalize_settings_tab("user-context"), Some("userContext"));
        assert_eq!(normalize_settings_tab("userContext"), Some("userContext"));
        assert_eq!(
            normalize_settings_tab("semanticSearch"),
            Some("semanticSearch")
        );
        assert_eq!(
            normalize_settings_tab("semantic-search"),
            Some("semanticSearch")
        );
    }

    #[test]
    fn settings_focus_aliases_normalize_to_canonical_focus() {
        assert_eq!(normalize_settings_focus("agentAccess"), Some("cliAccess"));
        assert_eq!(normalize_settings_focus("agent-access"), Some("cliAccess"));
        assert_eq!(normalize_settings_focus("cliAccess"), Some("cliAccess"));
        assert_eq!(normalize_settings_focus("cli-access"), Some("cliAccess"));
        assert_eq!(normalize_settings_focus("other"), None);
    }

    #[test]
    fn settings_tab_deeplink_path_targets_canonical_tab() {
        assert_eq!(
            settings_tab_focus_path("transcription", None).as_deref(),
            Ok("/settings?tab=transcription")
        );
        assert_eq!(
            settings_tab_focus_path("audio", None).as_deref(),
            Ok("/settings?tab=audio")
        );
        assert_eq!(
            settings_tab_focus_path("privacy", None).as_deref(),
            Ok("/settings?tab=privacy")
        );
        assert_eq!(
            settings_tab_focus_path("about", None).as_deref(),
            Ok("/settings?tab=about")
        );
        assert!(settings_tab_focus_path("../developer", None).is_err());
    }

    #[test]
    fn settings_focus_deeplink_path_targets_canonical_focus() {
        assert_eq!(
            settings_tab_focus_path("privacy", Some("agent-access")).as_deref(),
            Ok("/settings?tab=privacy&focus=cliAccess")
        );
        assert_eq!(
            settings_tab_focus_path("access", Some("cliAccess")).as_deref(),
            Ok("/settings?tab=access&focus=cliAccess")
        );
        assert!(settings_tab_focus_path("privacy", Some("../agent")).is_err());
    }

    #[test]
    fn open_settings_payload_normalizes_aliases() {
        let payload = normalized_open_settings_payload(Some("ocr"), Some("agent-access"));
        assert_eq!(payload.tab.as_deref(), Some("ocr"));
        assert_eq!(payload.focus.as_deref(), Some("cliAccess"));

        // Unknown values are dropped, not errored, so a stale deeplink still lands
        // on Settings (the route falls back to its default tab).
        let dropped = normalized_open_settings_payload(Some("transcripts"), Some("nope"));
        assert_eq!(dropped, OpenSettingsTabPayload::default());
    }

    #[test]
    fn cold_open_settings_is_queued_for_the_mount_drain() {
        // A cold-start tray "Open Settings" has to BUILD the main window, whose
        // fresh webview hasn't attached its `open_settings_tab` listener yet — so
        // the payload must be queued for the on-mount drain.
        let pending = PendingOpenSettingsState::default();
        let payload = normalized_open_settings_payload(Some("privacy"), None);

        enqueue_cold_open_settings(&pending, &payload, /* main_window_was_open */ false);

        let queued: Vec<_> = pending.pending.lock().unwrap().drain(..).collect();
        assert_eq!(queued, vec![payload]);
    }

    #[test]
    fn warm_open_settings_is_not_queued() {
        // A warm window is served by the live event alone; queuing here would
        // strand the entry (the page doesn't remount) and replay a stale deeplink
        // on the next genuine mount.
        let pending = PendingOpenSettingsState::default();
        let payload = normalized_open_settings_payload(Some("privacy"), None);

        enqueue_cold_open_settings(&pending, &payload, /* main_window_was_open */ true);

        assert!(pending.pending.lock().unwrap().is_empty());
    }

    #[test]
    fn missing_onboarding_state_is_incomplete() {
        let path = std::env::temp_dir().join(format!(
            "mnema-missing-onboarding-state-{}.json",
            super::now_unix_ms()
        ));

        assert!(!load_onboarding_state_from_path(path).is_complete());
    }

    #[test]
    fn onboarding_state_view_serializes_recording_settings_ever_saved_in_camel_case() {
        let view = OnboardingStateView::from_state_and_disk(
            OnboardingState {
                schema_version: 1,
                completed_at_unix_ms: Some(42),
            },
            true,
        );
        let json = serde_json::to_value(&view).expect("view serializes");
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["completedAtUnixMs"], 42);
        assert_eq!(json["recordingSettingsEverSaved"], true);
    }

    #[test]
    fn onboarding_state_view_carries_ever_saved_signal_independently() {
        // The signal is independent of completion: a not-yet-completed onboarding
        // can still report a returning user (settings saved), and vice versa.
        let returning = OnboardingStateView::from_state_and_disk(OnboardingState::incomplete(), true);
        assert!(returning.recording_settings_ever_saved);
        assert_eq!(returning.completed_at_unix_ms, None);

        let first_run =
            OnboardingStateView::from_state_and_disk(OnboardingState::incomplete(), false);
        assert!(!first_run.recording_settings_ever_saved);
    }

    #[test]
    fn invalid_onboarding_state_is_incomplete() {
        let path = std::env::temp_dir().join(format!(
            "mnema-invalid-onboarding-state-{}.json",
            super::now_unix_ms()
        ));
        std::fs::write(&path, "{not-json").expect("invalid test state should write");

        assert!(!load_onboarding_state_from_path(path.clone()).is_complete());

        let _ = std::fs::remove_file(path);
    }
}
