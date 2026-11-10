//! The status-bar item (BUILD-PROMPT §3.5): a template icon whose tooltip and
//! menu follow app-core's pure `status_bar_model` (same `tray_*` ids as the
//! Tauri app). tray-icon 0.26.1, created on the main thread inside `run`;
//! menu clicks go std thread → channel → `handle` on the main thread.

use std::sync::{Arc, Mutex};

use app_core::status_bar_model::*;
use app_core::{AppCore, ConfirmDialog, DialogKind, MessageDialog};
use capture_types::CaptureSources;
use gpui_kit::{App, Global};
use tray_icon::menu::{
    CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// One menu row, as data (what `build` turns into muda items; what tests read).
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Item {
        id: &'static str,
        label: String,
        enabled: bool,
    },
    Check {
        id: &'static str,
        label: &'static str,
        checked: bool,
        enabled: bool,
    },
    Submenu {
        label: &'static str,
        items: Vec<Entry>,
    },
    Separator,
}

fn item(id: &'static str, label: impl Into<String>, enabled: bool) -> Entry {
    Entry::Item {
        id,
        label: label.into(),
        enabled,
    }
}

/// The menu for `model`, in the Tauri app's order.
pub fn entries(model: &StatusBarMenuModel) -> Vec<Entry> {
    if !model.onboarding_complete {
        return vec![
            item(COMPLETE_SETUP_ID, "Complete Setup", true),
            item(OPEN_SETTINGS_ID, "Settings", true),
            Entry::Separator,
            item(QUIT_ID, "Quit Mnema", true),
        ];
    }
    let mut out = Vec::new();
    if let Some(label) = model.status_label {
        out.extend([item(STATUS_HEADER_ID, label, false), Entry::Separator]);
    }
    out.extend([
        item(
            RECORDING_TOGGLE_ID,
            model.recording_label.unwrap_or("Start Recording"),
            model.recording_enabled,
        ),
        item(
            PAUSE_TOGGLE_ID,
            model.pause_label.unwrap_or("Pause Recording"),
            model.pause_enabled,
        ),
        Entry::Submenu {
            label: "Sources",
            items: model
                .source_items
                .iter()
                .map(|s| Entry::Check {
                    id: s.id,
                    label: s.label,
                    checked: s.checked,
                    enabled: s.enabled,
                })
                .collect(),
        },
        item(
            EXCLUDE_CURRENT_APP_ID,
            "Exclude Current App From Now On...",
            true,
        ),
        Entry::Submenu {
            label: "Delete Recent Capture",
            items: vec![
                item(DELETE_LAST_1_MINUTE_ID, "Delete Last 1 Minute...", true),
                item(DELETE_LAST_5_MINUTES_ID, "Delete Last 5 Minutes...", true),
                item(DELETE_LAST_15_MINUTES_ID, "Delete Last 15 Minutes...", true),
            ],
        },
        Entry::Separator,
    ]);
    // Above "Open Mnema": the one surface a windowless user has can announce
    // the update and install it.
    if let Some((label, enabled)) = &model.update_item {
        out.extend([item(UPDATE_ID, label.clone(), *enabled), Entry::Separator]);
    }
    out.extend([
        item(OPEN_MAIN_ID, "Open Mnema", true),
        item(OPEN_SETTINGS_ID, "Settings", true),
        Entry::Separator,
        item(QUIT_ID, "Quit Mnema", true),
    ]);
    out
}

fn to_muda(entry: &Entry) -> Box<dyn IsMenuItem> {
    match entry {
        Entry::Item { id, label, enabled } => {
            Box::new(MenuItem::with_id(*id, label, *enabled, None))
        }
        Entry::Check {
            id,
            label,
            checked,
            enabled,
        } => Box::new(CheckMenuItem::with_id(
            *id, *label, *enabled, *checked, None,
        )),
        Entry::Submenu { label, items } => {
            let sub = Submenu::new(*label, true);
            for e in items {
                let _ = sub.append(to_muda(e).as_ref());
            }
            Box::new(sub)
        }
        Entry::Separator => Box::new(PredefinedMenuItem::separator()),
    }
}

fn build(model: &StatusBarMenuModel) -> Menu {
    let menu = Menu::new();
    for e in entries(model) {
        let _ = menu.append(to_muda(&e).as_ref());
    }
    menu
}

pub struct Tray {
    icon: TrayIcon,
    core: Arc<AppCore>,
    operation: StatusBarOperation,
}

impl Global for Tray {}

/// The live model: capture, settings, licensing and the update row.
fn current_model(core: &AppCore, operation: StatusBarOperation, cx: &App) -> StatusBarMenuModel {
    use app_core::native_capture::{control, permissions};
    let settings = control::current_recording_settings_from_core(core);
    let support = permissions::get_capture_support().supported_sources;
    let session = control::current_native_capture_session(core);
    let mut model = build_menu_model(
        super::onboarding_complete(core),
        session.is_running,
        session.is_user_paused,
        session.is_low_disk_suspended,
        &settings,
        &support,
        operation,
    );
    // `None` until the deferred license gate runs once: unknown reads as allow.
    let status = app_core::licensing::cached_status(core);
    let blocked = status
        .as_ref()
        .is_some_and(|s| !s.capture_allowed_at(app_core::licensing::now_ms()));
    apply_read_only_status(&mut model, blocked, status.as_ref());
    model.update_item = crate::small::update::last_status(cx).and_then(|s| {
        let version = s.update.as_ref().map(|u| u.version.as_str());
        tray_update_menu_item(s.state, version, app_core::updates::installable(core))
    });
    model
}

fn template_icon() -> Result<Icon, String> {
    let png = image::load_from_memory_with_format(
        include_bytes!("../../assets/tray-template.png"),
        image::ImageFormat::Png,
    )
    .map_err(|e| e.to_string())?
    .into_rgba8();
    let (w, h) = png.dimensions();
    Icon::from_rgba(png.into_raw(), w, h).map_err(|e| e.to_string())
}

/// Create the status item. Main thread, inside `run` (`AppCore::start`'s host setup).
pub fn create(
    core: &Arc<AppCore>,
    clicks: async_channel::Sender<String>,
    cx: &App,
) -> Result<Tray, String> {
    let model = current_model(core, StatusBarOperation::Idle, cx);
    let icon = TrayIconBuilder::new()
        .with_id(TRAY_ID)
        .with_icon_templated(template_icon()?)
        .with_tooltip(model.tooltip)
        .with_menu(Box::new(build(&model)))
        .with_menu_on_left_click(true)
        .build()
        .map_err(|e| e.to_string())?;
    let receiver = MenuEvent::receiver().clone();
    std::thread::Builder::new()
        .name("mnema-tray".into())
        .spawn(move || {
            while let Ok(event) = receiver.recv() {
                if clicks.send_blocking(event.id.0).is_err() {
                    break;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Tray {
        icon,
        core: core.clone(),
        operation: StatusBarOperation::Idle,
    })
}

/// Rebuild the menu and tooltip from live state (`Host::refresh_tray`).
pub fn refresh(cx: &mut App) {
    let Some(tray) = cx.try_global::<Tray>() else {
        return;
    };
    let model = current_model(&tray.core, tray.operation, cx);
    tray.icon.set_menu(Some(Box::new(build(&model))));
    if let Err(e) = tray.icon.set_tooltip(Some(model.tooltip)) {
        log::warn!("failed to set the status-bar tooltip: {e}");
    }
}

fn set_operation(operation: StatusBarOperation, cx: &mut App) {
    if cx.has_global::<Tray>() {
        cx.global_mut::<Tray>().operation = operation;
    }
    refresh(cx);
}

/// Serializes source toggles: each reads the sources it flips.
static SOURCE_TOGGLE: Mutex<()> = Mutex::new(());

/// A menu click, on the main thread.
pub fn handle(id: &str, cx: &mut App) {
    let Some(core) = cx.try_global::<Tray>().map(|t| t.core.clone()) else {
        return;
    };
    match id {
        COMPLETE_SETUP_ID => crate::onboarding::open(cx),
        // `ensure_main` lands on onboarding while it isn't done.
        OPEN_MAIN_ID => {
            crate::window::ensure_main(cx);
        }
        OPEN_SETTINGS_ID => crate::window::open_settings(None, cx),
        RECORDING_TOGGLE_ID => recording_toggle(core, cx),
        PAUSE_TOGGLE_ID => {
            std::thread::spawn(move || {
                if let Err((title, e)) = super::toggle_pause(&core) {
                    super::report_capture_error(&core, title, &e, false);
                    show_error(&core, title, e.message);
                }
            });
        }
        SOURCE_SCREEN_ID | SOURCE_MICROPHONE_ID | SOURCE_SYSTEM_AUDIO_ID => {
            // Off the main thread: the save asks the capture lock, which a stop
            // holds for seconds. The save rebuilds the tray itself.
            let id = id.to_string();
            std::thread::spawn(move || source_toggle(&core, &id));
        }
        EXCLUDE_CURRENT_APP_ID => exclude_current_app(core),
        DELETE_LAST_1_MINUTE_ID => delete_recent(core, 60),
        DELETE_LAST_5_MINUTES_ID => delete_recent(core, 300),
        DELETE_LAST_15_MINUTES_ID => delete_recent(core, 900),
        // Install / Retry / Restart to Finish (a staged restart just relaunches).
        UPDATE_ID => crate::small::update::install(true, cx),
        QUIT_ID => core.shutdown(app_core::ExitKind::Quit),
        _ => {}
    }
}

fn recording_toggle(core: Arc<AppCore>, cx: &mut App) {
    let busy = cx
        .try_global::<Tray>()
        .is_some_and(|t| t.operation != StatusBarOperation::Idle);
    if busy {
        return;
    }
    let recording =
        app_core::native_capture::control::current_native_capture_session(&core).is_running;
    set_operation(
        if recording {
            StatusBarOperation::Stopping
        } else {
            StatusBarOperation::Starting
        },
        cx,
    );
    let (done_tx, done_rx) = async_channel::bounded(1);
    let worker = core.clone();
    std::thread::spawn(move || {
        let _ = done_tx.send_blocking(super::toggle_recording(&worker, "status-bar"));
    });
    cx.spawn(async move |cx| {
        let result = done_rx.recv().await;
        let _ = cx.update(|cx| set_operation(StatusBarOperation::Idle, cx));
        if let Ok(Err((title, e))) = result {
            super::report_capture_error(&core, title, &e, false);
            show_error(&core, title, e.message);
        }
    })
    .detach();
}

fn show_error(core: &AppCore, title: &str, message: String) {
    core.host().message(MessageDialog {
        title: title.into(),
        message,
        kind: DialogKind::Error,
    });
}

fn source_toggle(core: &Arc<AppCore>, id: &str) {
    let _serial = SOURCE_TOGGLE.lock().unwrap_or_else(|p| p.into_inner());
    let settings = app_core::native_capture::control::current_recording_settings_from_core(core);
    let support = app_core::native_capture::permissions::get_capture_support().supported_sources;
    let session = app_core::native_capture::control::current_native_capture_session(core);
    let model = build_menu_model(
        true,
        session.is_running,
        session.is_user_paused,
        session.is_low_disk_suspended,
        &settings,
        &support,
        StatusBarOperation::Idle,
    );
    let enabled = model.source_items.iter().any(|s| s.id == id && s.enabled);
    let current = CaptureSources {
        screen: settings.capture_screen,
        microphone: settings.capture_microphone,
        system_audio: settings.capture_system_audio,
    };
    let next = enabled
        .then(|| computed_toggle_sources(current, id))
        .flatten();
    let Some(next) = next else {
        core.host().refresh_tray();
        return;
    };
    if let Err(e) =
        app_core::native_capture::settings_update::update_recording_sources_from_core(core, next)
    {
        log::warn!(
            "failed to update recording sources from the status bar: [{}] {}",
            e.code,
            e.message
        );
        core.host().refresh_tray();
    }
}

fn confirm(
    core: &AppCore,
    title: &str,
    message: String,
    ok: &str,
) -> tokio::sync::oneshot::Receiver<bool> {
    core.host().confirm(ConfirmDialog {
        title: Some(title.into()),
        message,
        kind: DialogKind::Warning,
        ok_label: ok.into(),
        cancel_label: "Cancel".into(),
    })
}

fn delete_recent(core: Arc<AppCore>, seconds: i64) {
    let label = match seconds {
        60 => "last 1 minute",
        300 => "last 5 minutes",
        900 => "last 15 minutes",
        _ => "recent capture",
    };
    let answer = confirm(
        &core,
        "Delete Recent Capture",
        format!("Delete the {label} from Mnema's library? This removes whole overlapping capture segments and cannot be undone."),
        "Delete",
    );
    let task_core = core.clone();
    core.spawn(async move {
        if answer.await != Ok(true) {
            return;
        }
        let core = task_core;
        match app_core::app_infra::delete_recent_capture_from_core(&core, seconds).await {
            Ok(summary) if summary.file_delete_errors > 0 => core.host().message(MessageDialog {
                title: "Delete Recent Capture Incomplete".into(),
                message: format!(
                    "Mnema removed the matching library rows, but {} file path(s) could not be deleted from disk. They have been queued for retry. Pending file tombstones: {}.",
                    summary.file_delete_errors, summary.pending_file_tombstones
                ),
                kind: DialogKind::Warning,
            }),
            Ok(_) => {}
            Err(e) => core.host().message(MessageDialog {
                title: "Delete Recent Capture Failed".into(),
                message: e,
                kind: DialogKind::Error,
            }),
        }
    });
}

fn exclude_current_app(core: Arc<AppCore>) {
    let snapshot = app_core::native_capture::metadata::collect_native_active_window_snapshot();
    let bundle_id = snapshot.bundle_id.unwrap_or_default();
    let name = snapshot.app_name.unwrap_or_else(|| bundle_id.clone());
    let info = |message: &str, title: &str| {
        core.host().message(MessageDialog {
            title: title.into(),
            message: message.into(),
            kind: DialogKind::Info,
        })
    };
    // Mnema itself (either app) is never a candidate.
    if bundle_id.trim().is_empty() || bundle_id == "day.mnema" || bundle_id == crate::BUNDLE_ID {
        info(
            "Mnema could not identify a frontmost app to exclude from screen capture.",
            "Exclude Current App",
        );
        return;
    }
    let answer = confirm(
        &core,
        "Exclude Current App",
        format!("Exclude {name} from screen content capture from now on? This does not delete historical capture."),
        "Exclude",
    );
    let task_core = core.clone();
    core.spawn(async move {
        if answer.await != Ok(true) {
            return;
        }
        let core = task_core;
        use app_core::privacy_redaction_sources::apply::add_or_enable_privacy_excluded_app_from_core as exclude;
        // Applying the filter reaches the capture runtime: off the async workers.
        let apply_core = core.clone();
        let Ok(result) = core
            .spawn_blocking(move || exclude(&apply_core, bundle_id, name))
            .await
        else {
            return;
        };
        match result {
            Err(e) => show_error(&core, "Could not exclude app", e.message),
            Ok(_) => core.host().message(MessageDialog {
                title: "App Excluded".into(),
                message: "The app is excluded from future screen content capture. Historical capture was not deleted.".into(),
                kind: DialogKind::Info,
            }),
        }
    });
}

#[cfg(test)]
#[path = "tray_tests.rs"]
mod tests;
