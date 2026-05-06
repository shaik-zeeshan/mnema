use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::native_capture;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppWindow {
    Main,
    Settings,
    Debug,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DestroyedWindowAction {
    FocusMainWindow,
    ExitApp,
    None,
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
                inner_size: (900.0, 820.0),
                min_inner_size: (640.0, 480.0),
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
            "main" => Some(Self::Main),
            "settings" => Some(Self::Settings),
            "debug" => Some(Self::Debug),
            _ => None,
        }
    }
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

fn open_or_focus_window(
    app: &tauri::AppHandle,
    window: AppWindow,
    state: Option<&native_capture::RecordingSettingsState>,
) -> Result<(), String> {
    ensure_window_allowed(window, state)?;

    let config = window.config();
    if let Some(existing) = app.get_webview_window(config.label) {
        let _ = existing.show();
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(());
    }

    let mut builder = WebviewWindowBuilder::new(app, config.label, WebviewUrl::App(config.path.into()));
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

fn focus_main_window(app: &tauri::AppHandle) {
    if let Some(main) = app.get_webview_window(AppWindow::Main.config().label) {
        let _ = main.show();
        let _ = main.unminimize();
        let _ = main.set_focus();
    }
}

fn destroyed_window_action(label: &str) -> DestroyedWindowAction {
    match AppWindow::from_label(label) {
        Some(AppWindow::Settings | AppWindow::Debug) => DestroyedWindowAction::FocusMainWindow,
        Some(AppWindow::Main) => DestroyedWindowAction::ExitApp,
        None => DestroyedWindowAction::None,
    }
}

fn close_window(window: WebviewWindow) -> Result<(), String> {
    match AppWindow::from_label(window.label()) {
        Some(AppWindow::Settings | AppWindow::Debug) => {
            focus_main_window(window.app_handle());
            window.close().map_err(|err| err.to_string())
        }
        Some(AppWindow::Main) => Err("main window cannot be closed from this command".into()),
        None => window.close().map_err(|err| err.to_string()),
    }
}

pub fn handle_window_event(window: &WebviewWindow, event: &WindowEvent) {
    if !matches!(event, WindowEvent::Destroyed) {
        return;
    }

    match destroyed_window_action(window.label()) {
        DestroyedWindowAction::FocusMainWindow => focus_main_window(window.app_handle()),
        DestroyedWindowAction::ExitApp => window.app_handle().exit(0),
        DestroyedWindowAction::None => {}
    }
}

#[tauri::command]
pub fn open_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    open_or_focus_window(&app, AppWindow::Settings, None)
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

#[cfg(test)]
mod tests {
    use super::{destroyed_window_action, DestroyedWindowAction};

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
    fn main_window_destruction_exits_the_app() {
        assert_eq!(
            destroyed_window_action("main"),
            DestroyedWindowAction::ExitApp
        );
    }

    #[test]
    fn unknown_window_destruction_has_no_side_effect() {
        assert_eq!(destroyed_window_action("other"), DestroyedWindowAction::None);
    }
}
