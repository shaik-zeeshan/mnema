// Permission helper window: every "Open System Settings" opens the right
// Privacy & Security pane AND floats this small card beside it — the app icon,
// "Turn on Mnema in the <list> list" and a live check (the page polls the
// permission and closes itself on grant).
//
// It reuses the Quick Recall NSPanel reclass (non-activating, floating level,
// all Spaces) and is ordered front WITHOUT becoming key, so System Settings
// keeps focus while the user flips the switch.
//
// ponytail: no drag-out of the app icon into the list. After a Deny macOS
// already lists Mnema with its switch off; drag-out (a native
// `NSDraggingSession` carrying the bundle URL) is only for the not-listed case.

use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use super::AppWindow;

/// The kinds the helper route understands. Capture kinds match
/// `open_capture_privacy_settings`; Accessibility has its own pane opener.
const KINDS: [&str; 4] = ["screen", "microphone", "systemAudio", "accessibility"];

/// Gap above the work area's bottom edge — the kit's `mx-helper { bottom: 28px }`.
const BOTTOM_GAP: f64 = 28.0;

/// Open the System Settings pane for `kind` and float the helper beside it.
/// A helper already open for another kind is re-pointed, not duplicated.
#[tauri::command]
pub fn show_permission_helper(app: tauri::AppHandle, kind: String) -> Result<(), String> {
    if !KINDS.contains(&kind.as_str()) {
        return Err(format!("unknown permission kind: {kind}"));
    }

    // Pane first: System Settings activates on top of everything at normal
    // level; the helper's floating level keeps it above that.
    if kind == "accessibility" {
        crate::native_capture::open_browser_url_accessibility_settings(app.clone())?;
    } else {
        crate::native_capture::open_capture_privacy_settings(kind.clone(), app.clone())?;
    }

    let config = AppWindow::PermissionHelper.config();
    let query = format!("kind={kind}");
    let window = match app.get_webview_window(config.label) {
        Some(existing) => {
            let mut url = existing.url().map_err(|err| err.to_string())?;
            url.set_query(Some(&query));
            existing.navigate(url).map_err(|err| err.to_string())?;
            existing
        }
        None => build(&app, format!("{}?{query}", config.path))?,
    };

    place_bottom_centre(&window);
    order_front_without_focus(&window);
    Ok(())
}

fn build(app: &tauri::AppHandle, path: String) -> Result<WebviewWindow, String> {
    let config = AppWindow::PermissionHelper.config();
    let built = WebviewWindowBuilder::new(app, config.label, WebviewUrl::App(path.into()))
        .title(config.title)
        .inner_size(config.inner_size.0, config.inner_size.1)
        .min_inner_size(config.min_inner_size.0, config.min_inner_size.1)
        .resizable(false)
        .decorations(config.decorations)
        .transparent(config.transparent)
        .shadow(config.shadow)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()
        .map_err(|err| err.to_string())?;

    #[cfg(target_os = "macos")]
    {
        super::configure_quick_recall_panel(&built);
        // The panel is never key when clicked, so without first-mouse the
        // Dismiss click would only order it forward.
        super::configure_quick_recall_webview(&built);
        if let Some(radius) = config.macos_corner_radius {
            super::apply_macos_rounded_content_view(&built, radius);
        }
    }

    Ok(built)
}

// ponytail: the helper's own monitor (the primary on first build), not the one
// System Settings opened on; follow the cursor's monitor if that ever misses.
fn place_bottom_centre(window: &WebviewWindow) {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let (Some(monitor), Ok(size)) = (monitor, window.outer_size()) else {
        return;
    };
    let area = monitor.work_area();
    let gap = (BOTTOM_GAP * monitor.scale_factor()) as i32;
    let x = area.position.x + (area.size.width as i32 - size.width as i32) / 2;
    let y = area.position.y + area.size.height as i32 - size.height as i32 - gap;
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn order_front_without_focus(window: &WebviewWindow) {
    use cocoa::base::id;
    use objc::{msg_send, sel, sel_impl};
    let Ok(ns_window) = window.ns_window() else {
        let _ = window.show();
        return;
    };
    unsafe {
        let _: () = msg_send![ns_window as id, orderFrontRegardless];
    }
}

#[cfg(not(target_os = "macos"))]
fn order_front_without_focus(window: &WebviewWindow) {
    let _ = window.show();
}
