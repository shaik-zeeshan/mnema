use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, MutexGuard, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::native_capture;

const ONBOARDING_STATE_FILE_NAME: &str = "onboarding-state.json";
const OPEN_SETTINGS_TAB_EVENT: &str = "open_settings_tab";

#[cfg(target_os = "macos")]
static MACOS_TERMINATE_APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();
#[cfg(target_os = "macos")]
static MACOS_TERMINATE_HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct OpenSettingsTabPayload {
    tab: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    focus: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppWindow {
    Onboarding,
    Main,
    Settings,
    CliAccessRequest,
    Debug,
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

#[derive(Debug, Default)]
pub struct OnboardingStateRuntime {
    state: Option<OnboardingState>,
}

pub type OnboardingStateStore = Mutex<OnboardingStateRuntime>;

#[derive(Default)]
pub struct AppExitCoordinatorState {
    exit_requested: AtomicBool,
    final_graceful_exit_ready: AtomicBool,
}

impl AppExitCoordinatorState {
    fn begin_exit(&self) -> bool {
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
    macos_corner_radius: Option<f64>,
}

impl AppWindow {
    const fn config(self) -> AppWindowConfig {
        match self {
            Self::Onboarding => AppWindowConfig {
                label: "onboarding",
                path: "onboarding",
                title: "mnema · Onboarding",
                inner_size: (960.0, 760.0),
                min_inner_size: (820.0, 620.0),
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
            Self::Settings => AppWindowConfig {
                label: "settings",
                path: "settings",
                title: "mnema · Settings",
                inner_size: (1040.0, 820.0),
                min_inner_size: (820.0, 620.0),
                gated_by_dev_options: false,
                decorations: false,
                overlay_title_bar: false,
                transparent: true,
                shadow: true,
                macos_corner_radius: Some(12.0),
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
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "onboarding" => Some(Self::Onboarding),
            "main" => Some(Self::Main),
            "settings" => Some(Self::Settings),
            "cli-access-request" => Some(Self::CliAccessRequest),
            "debug" => Some(Self::Debug),
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

pub fn current_onboarding_state_for_app(
    app: &tauri::AppHandle,
    store: &OnboardingStateStore,
) -> OnboardingState {
    current_onboarding_state(app, store)
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

    let mut builder =
        WebviewWindowBuilder::new(app, config.label, WebviewUrl::App(config.path.into()));
    builder = builder
        .title(config.title)
        .inner_size(config.inner_size.0, config.inner_size.1)
        .min_inner_size(config.min_inner_size.0, config.min_inner_size.1)
        .decorations(config.decorations)
        .transparent(config.transparent)
        .shadow(config.shadow);

    if config.overlay_title_bar {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true);
    }

    let built = builder.build().map_err(|err| err.to_string())?;

    #[cfg(target_os = "macos")]
    if let Some(radius) = config.macos_corner_radius {
        apply_macos_rounded_content_view(&built, radius);
    }

    show_and_focus_window(&built);

    Ok(())
}

fn normalize_settings_tab(tab: &str) -> Option<&'static str> {
    match tab {
        "capture" | "behavior" => Some("capture"),
        "privacy" | "metadata" => Some("privacy"),
        "access" | "cliAccess" | "cli-access" => Some("access"),
        "video" => Some("video"),
        "audio" | "microphone" => Some("audio"),
        "processing" | "ocr" | "transcription" | "speakers" => Some("processing"),
        "storage" => Some("storage"),
        "appearance" => Some("appearance"),
        "developer" => Some("developer"),
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

fn open_or_focus_settings_window_to_tab(
    app: &tauri::AppHandle,
    tab: &str,
    focus: Option<&str>,
) -> Result<(), String> {
    let path = settings_tab_focus_path(tab, focus)?;
    let config = AppWindow::Settings.config();
    let tab = normalize_settings_tab(tab).ok_or_else(|| format!("unknown settings tab: {tab}"))?;
    let focus = match focus {
        Some(value) => Some(
            normalize_settings_focus(value)
                .ok_or_else(|| format!("unknown settings focus: {value}"))?
                .to_string(),
        ),
        None => None,
    };
    let payload = OpenSettingsTabPayload {
        tab: tab.to_string(),
        focus,
    };

    if let Some(existing) = app.get_webview_window(config.label) {
        show_and_focus_window(&existing);
        existing
            .emit(OPEN_SETTINGS_TAB_EVENT, payload)
            .map_err(|err| err.to_string())?;
        return Ok(());
    }

    let mut builder = WebviewWindowBuilder::new(app, config.label, WebviewUrl::App(path.into()));
    builder = builder
        .title(config.title)
        .inner_size(config.inner_size.0, config.inner_size.1)
        .min_inner_size(config.min_inner_size.0, config.min_inner_size.1)
        .decorations(config.decorations)
        .transparent(config.transparent)
        .shadow(config.shadow);

    let built = builder.build().map_err(|err| err.to_string())?;

    #[cfg(target_os = "macos")]
    if let Some(radius) = config.macos_corner_radius {
        apply_macos_rounded_content_view(&built, radius);
    }

    show_and_focus_window(&built);

    Ok(())
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
    let exit_state = app.state::<AppExitCoordinatorState>();
    if !exit_state.begin_exit() {
        return;
    }

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::native_capture::debug_log::log_info(
            "starting graceful app exit; stopping capture and background workers before unloading cached Local Whisper contexts",
        );

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
    app.state::<AppExitCoordinatorState>()
        .mark_final_graceful_exit_ready();

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
        Some(AppWindow::Settings | AppWindow::Debug) => DestroyedWindowAction::FocusMainWindow,
        Some(AppWindow::CliAccessRequest) => DestroyedWindowAction::None,
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
        Some(
            AppWindow::Onboarding
            | AppWindow::Settings
            | AppWindow::CliAccessRequest
            | AppWindow::Debug,
        ) => window.close().map_err(|err| err.to_string()),
        Some(AppWindow::Main) => Err("main window cannot be closed from this command".into()),
        None => window.close().map_err(|err| err.to_string()),
    }
}

fn close_window_focuses_main_before_close(label: &str) -> bool {
    matches!(
        AppWindow::from_label(label),
        Some(AppWindow::Settings | AppWindow::Debug)
    )
}

pub fn handle_window_event(
    app: &tauri::AppHandle,
    label: &str,
    event: &WindowEvent,
    window: Option<&WebviewWindow>,
) {
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
    if state.is_complete() {
        open_or_focus_window(app, AppWindow::Main, None)?;
        Ok(true)
    } else {
        open_or_focus_window(app, AppWindow::Onboarding, None)?;
        Ok(false)
    }
}

pub(crate) fn is_onboarding_complete(app: &tauri::AppHandle) -> bool {
    let store = app.state::<OnboardingStateStore>();
    current_onboarding_state(app, store.inner()).is_complete()
}

#[tauri::command]
pub fn open_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    open_or_focus_window(&app, AppWindow::Settings, None)
}

pub(crate) fn open_cli_access_request_window(app: &tauri::AppHandle) -> Result<(), String> {
    open_or_focus_window(app, AppWindow::CliAccessRequest, None)
}

#[tauri::command]
pub fn open_settings_window_to_tab(
    app: tauri::AppHandle,
    tab: String,
    focus: Option<String>,
) -> Result<(), String> {
    open_or_focus_settings_window_to_tab(&app, &tab, focus.as_deref())
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

#[tauri::command]
pub fn toggle_main_window_visibility_command(app: tauri::AppHandle) {
    toggle_main_window_visibility(&app);
}

#[tauri::command]
pub fn get_onboarding_state(
    app: tauri::AppHandle,
    state: tauri::State<'_, OnboardingStateStore>,
) -> OnboardingState {
    current_onboarding_state(&app, state.inner())
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
    open_or_focus_window(&app, AppWindow::Main, None)?;
    crate::status_bar::refresh(&app);
    window.close().map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        close_window_focuses_main_before_close, destroyed_window_action, is_known_settings_tab,
        load_onboarding_state_from_path, normalize_settings_focus, normalize_settings_tab,
        settings_tab_focus_path, AppExitCoordinatorState, DestroyedWindowAction,
    };

    #[test]
    fn secondary_window_destruction_refocuses_main_window() {
        assert_eq!(
            destroyed_window_action("settings"),
            DestroyedWindowAction::FocusMainWindow
        );
        assert_eq!(
            destroyed_window_action("debug"),
            DestroyedWindowAction::FocusMainWindow
        );
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
        assert!(close_window_focuses_main_before_close("settings"));
        assert!(close_window_focuses_main_before_close("debug"));
        assert!(!close_window_focuses_main_before_close(
            "cli-access-request"
        ));
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

        assert!(coordinator.begin_exit());
        assert!(!coordinator.begin_exit());
        assert!(coordinator.is_exit_requested());
        assert!(!coordinator.is_final_graceful_exit_ready());

        coordinator.mark_final_graceful_exit_ready();

        assert!(coordinator.is_final_graceful_exit_ready());
    }

    #[test]
    fn settings_tab_deeplink_accepts_known_tabs_only() {
        assert!(is_known_settings_tab("processing"));
        assert!(is_known_settings_tab("transcription"));
        assert!(is_known_settings_tab("microphone"));
        assert!(is_known_settings_tab("capture"));
        assert!(is_known_settings_tab("privacy"));
        assert!(!is_known_settings_tab("transcripts"));
        assert!(!is_known_settings_tab("../developer"));
    }

    #[test]
    fn settings_tab_aliases_normalize_to_canonical_tabs() {
        assert_eq!(normalize_settings_tab("ocr"), Some("processing"));
        assert_eq!(normalize_settings_tab("transcription"), Some("processing"));
        assert_eq!(normalize_settings_tab("speakers"), Some("processing"));
        assert_eq!(normalize_settings_tab("microphone"), Some("audio"));
        assert_eq!(normalize_settings_tab("behavior"), Some("capture"));
        assert_eq!(normalize_settings_tab("metadata"), Some("privacy"));
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
            Ok("/settings?tab=processing")
        );
        assert_eq!(
            settings_tab_focus_path("audio", None).as_deref(),
            Ok("/settings?tab=audio")
        );
        assert_eq!(
            settings_tab_focus_path("privacy", None).as_deref(),
            Ok("/settings?tab=privacy")
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
    fn missing_onboarding_state_is_incomplete() {
        let path = std::env::temp_dir().join(format!(
            "mnema-missing-onboarding-state-{}.json",
            super::now_unix_ms()
        ));

        assert!(!load_onboarding_state_from_path(path).is_complete());
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
