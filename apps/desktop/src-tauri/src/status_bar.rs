use capture_types::{CaptureErrorResponse, CaptureSources, RecordingSettings};
use std::sync::Mutex;
use tauri::{
    image::Image,
    menu::{CheckMenuItemBuilder, Menu, MenuItemBuilder, PredefinedMenuItem, Submenu},
    tray::{TrayIcon, TrayIconBuilder},
    Manager,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

const TRAY_ID: &str = "mnema-status-bar";
const COMPLETE_SETUP_ID: &str = "tray_complete_setup";
const STATUS_HEADER_ID: &str = "tray_status_header";
const RECORDING_TOGGLE_ID: &str = "tray_recording_toggle";
const PAUSE_TOGGLE_ID: &str = "tray_pause_toggle";
const DELETE_LAST_1_MINUTE_ID: &str = "tray_delete_recent_60";
const DELETE_LAST_5_MINUTES_ID: &str = "tray_delete_recent_300";
const DELETE_LAST_15_MINUTES_ID: &str = "tray_delete_recent_900";
#[cfg(target_os = "macos")]
const EXCLUDE_CURRENT_APP_ID: &str = "tray_exclude_current_app";
const SOURCE_SCREEN_ID: &str = "tray_source_screen";
const SOURCE_MICROPHONE_ID: &str = "tray_source_microphone";
const SOURCE_SYSTEM_AUDIO_ID: &str = "tray_source_system_audio";
const OPEN_MAIN_ID: &str = "tray_open_main";
const OPEN_SETTINGS_ID: &str = "tray_open_settings";
const QUIT_ID: &str = "tray_quit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusBarOperation {
    Idle,
    Starting,
    Stopping,
}

#[derive(Default)]
pub(crate) struct StatusBarRuntime {
    tray: Option<TrayIcon>,
    operation: StatusBarOperation,
}

pub(crate) type StatusBarState = Mutex<StatusBarRuntime>;

fn tray_template_icon() -> tauri::Result<Image<'static>> {
    let decoded = image::load_from_memory_with_format(
        include_bytes!("../icons/tray-template.png"),
        image::ImageFormat::Png,
    )
    .map_err(|error| tauri::Error::Io(std::io::Error::other(error)))?
    .into_rgba8();
    let (width, height) = decoded.dimensions();
    Ok(Image::new_owned(decoded.into_raw(), width, height))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceItemModel {
    id: &'static str,
    label: &'static str,
    checked: bool,
    enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusBarCaptureSupport {
    sources: CaptureSources,
    system_audio_requires_screen: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusBarMenuModel {
    onboarding_complete: bool,
    /// A non-actionable status header shown at the top of the menu. Present only
    /// for the low-disk-suspended state ("Paused — low disk"), which is surfaced
    /// as a disabled label so the tray never reads as plainly "Recording" while
    /// capture is held by the low-disk liveness suspension (ADR 0040).
    status_label: Option<&'static str>,
    recording_label: Option<&'static str>,
    recording_enabled: bool,
    pause_label: Option<&'static str>,
    pause_enabled: bool,
    source_items: Vec<SourceItemModel>,
    system_audio_requires_screen: bool,
    tooltip: &'static str,
}

impl Default for StatusBarOperation {
    fn default() -> Self {
        Self::Idle
    }
}

fn any_source_enabled(sources: &CaptureSources) -> bool {
    sources.screen || sources.microphone || sources.system_audio
}

fn effective_checked_sources(
    settings: &RecordingSettings,
    _support: &StatusBarCaptureSupport,
) -> CaptureSources {
    CaptureSources {
        screen: settings.capture_screen,
        microphone: settings.capture_microphone,
        system_audio: settings.capture_system_audio,
    }
}

fn supported_sources_only(
    sources: &CaptureSources,
    support: &StatusBarCaptureSupport,
) -> CaptureSources {
    CaptureSources {
        screen: sources.screen && support.sources.screen,
        microphone: sources.microphone && support.sources.microphone,
        system_audio: sources.system_audio && support.sources.system_audio,
    }
}

fn computed_toggle_sources(
    current: CaptureSources,
    source_id: &str,
    system_audio_requires_screen: bool,
) -> Option<CaptureSources> {
    let mut next = current;
    match source_id {
        SOURCE_SCREEN_ID => {
            next.screen = !next.screen;
            if !next.screen && system_audio_requires_screen {
                next.system_audio = false;
            }
        }
        SOURCE_MICROPHONE_ID => next.microphone = !next.microphone,
        SOURCE_SYSTEM_AUDIO_ID => {
            if system_audio_requires_screen && !next.screen {
                return None;
            }
            next.system_audio = !next.system_audio;
        }
        _ => return None,
    }

    any_source_enabled(&next).then_some(next)
}

fn source_item_enabled(
    source_id: &str,
    checked: bool,
    current: &CaptureSources,
    support: &StatusBarCaptureSupport,
    operation: StatusBarOperation,
    recording: bool,
) -> bool {
    if recording || operation != StatusBarOperation::Idle {
        return false;
    }

    let supported = match source_id {
        SOURCE_SCREEN_ID => support.sources.screen,
        SOURCE_MICROPHONE_ID => support.sources.microphone,
        SOURCE_SYSTEM_AUDIO_ID => support.sources.system_audio,
        _ => false,
    };
    if !supported {
        return false;
    }
    if source_id == SOURCE_SYSTEM_AUDIO_ID
        && support.system_audio_requires_screen
        && !current.screen
    {
        return false;
    }
    if checked {
        let Some(next) = computed_toggle_sources(
            current.clone(),
            source_id,
            support.system_audio_requires_screen,
        ) else {
            return false;
        };
        if !any_source_enabled(&supported_sources_only(&next, support)) {
            return false;
        }
    }

    true
}

fn build_menu_model(
    onboarding_complete: bool,
    recording: bool,
    user_paused: bool,
    low_disk_suspended: bool,
    settings: &RecordingSettings,
    support: &StatusBarCaptureSupport,
    operation: StatusBarOperation,
) -> StatusBarMenuModel {
    if !onboarding_complete {
        return StatusBarMenuModel {
            onboarding_complete: false,
            status_label: None,
            recording_label: None,
            recording_enabled: false,
            pause_label: None,
            pause_enabled: false,
            system_audio_requires_screen: support.system_audio_requires_screen,
            source_items: Vec::new(),
            tooltip: "Mnema",
        };
    }

    let checked_sources = effective_checked_sources(settings, support);
    let recording_label = match operation {
        StatusBarOperation::Idle if recording => "Stop Recording",
        StatusBarOperation::Idle => "Start Recording",
        StatusBarOperation::Starting => "Starting...",
        StatusBarOperation::Stopping => "Stopping...",
    };
    // The low-disk suspension keeps the session alive (recording == true), so it
    // must take precedence over the generic recording/paused tooltip — otherwise
    // the tray would read "Recording" while capture is actually held (ADR 0040).
    let status_label = (operation == StatusBarOperation::Idle && low_disk_suspended)
        .then_some("Paused — low disk");
    let tooltip = match operation {
        StatusBarOperation::Idle if low_disk_suspended => "Mnema — Paused (low disk)",
        StatusBarOperation::Idle if recording => "Mnema - Recording",
        StatusBarOperation::Idle => "Mnema",
        StatusBarOperation::Starting => "Mnema - Starting...",
        StatusBarOperation::Stopping => "Mnema - Stopping...",
    };

    let source_items = [
        (SOURCE_SCREEN_ID, "Screen", checked_sources.screen),
        (
            SOURCE_MICROPHONE_ID,
            "Microphone",
            checked_sources.microphone,
        ),
        (
            SOURCE_SYSTEM_AUDIO_ID,
            "System Audio",
            checked_sources.system_audio,
        ),
    ]
    .into_iter()
    .map(|(id, label, checked)| SourceItemModel {
        id,
        label,
        checked,
        enabled: source_item_enabled(id, checked, &checked_sources, support, operation, recording),
    })
    .collect();

    // Pause/resume is honoured by the backend on every platform: pause_user_capture
    // is cross-platform and resume_user_capture has a working Windows arm. Expose
    // the tray control everywhere so it agrees with the global pauseResumeRecording
    // keyboard shortcut (which is not platform-gated).
    let pause_supported = true;

    StatusBarMenuModel {
        onboarding_complete: true,
        status_label,
        recording_label: Some(recording_label),
        recording_enabled: operation == StatusBarOperation::Idle,
        pause_label: (pause_supported && recording).then_some(if user_paused {
            "Resume Recording"
        } else {
            "Pause Recording"
        }),
        pause_enabled: pause_supported && recording && operation == StatusBarOperation::Idle,
        source_items,
        system_audio_requires_screen: support.system_audio_requires_screen,
        tooltip,
    }
}

fn operation(app: &tauri::AppHandle) -> StatusBarOperation {
    app.state::<StatusBarState>()
        .lock()
        .expect("status bar state poisoned")
        .operation
}

fn set_operation(app: &tauri::AppHandle, operation: StatusBarOperation) {
    app.state::<StatusBarState>()
        .lock()
        .expect("status bar state poisoned")
        .operation = operation;
}

fn current_model(app: &tauri::AppHandle) -> StatusBarMenuModel {
    let settings = crate::native_capture::current_recording_settings_from_app_handle(app);
    let support_response = crate::native_capture::get_capture_support();
    let support = StatusBarCaptureSupport {
        sources: support_response.supported_sources,
        system_audio_requires_screen: support_response.system_audio_requires_screen,
    };
    let session = crate::native_capture::current_native_capture_session(app);
    let recording = session.is_running;
    build_menu_model(
        crate::windows::is_onboarding_complete(app),
        recording,
        session.is_user_paused,
        session.is_low_disk_suspended,
        &settings,
        &support,
        operation(app),
    )
}

fn build_menu(
    app: &tauri::AppHandle,
    model: &StatusBarMenuModel,
) -> tauri::Result<Menu<tauri::Wry>> {
    if !model.onboarding_complete {
        let complete_setup =
            MenuItemBuilder::with_id(COMPLETE_SETUP_ID, "Complete Setup").build(app)?;
        let settings = MenuItemBuilder::with_id(OPEN_SETTINGS_ID, "Settings").build(app)?;
        let quit = MenuItemBuilder::with_id(QUIT_ID, "Quit Mnema").build(app)?;
        let separator = PredefinedMenuItem::separator(app)?;
        return Menu::with_items(app, &[&complete_setup, &settings, &separator, &quit]);
    }

    let recording = MenuItemBuilder::with_id(
        RECORDING_TOGGLE_ID,
        model.recording_label.unwrap_or("Start Recording"),
    )
    .enabled(model.recording_enabled)
    .build(app)?;
    let screen = CheckMenuItemBuilder::with_id(SOURCE_SCREEN_ID, "Screen")
        .checked(model.source_items[0].checked)
        .enabled(model.source_items[0].enabled)
        .build(app)?;
    let microphone = CheckMenuItemBuilder::with_id(SOURCE_MICROPHONE_ID, "Microphone")
        .checked(model.source_items[1].checked)
        .enabled(model.source_items[1].enabled)
        .build(app)?;
    let system_audio = CheckMenuItemBuilder::with_id(SOURCE_SYSTEM_AUDIO_ID, "System Audio")
        .checked(model.source_items[2].checked)
        .enabled(model.source_items[2].enabled)
        .build(app)?;
    let sources =
        Submenu::with_items(app, "Sources", true, &[&screen, &microphone, &system_audio])?;
    let pause = MenuItemBuilder::with_id(
        PAUSE_TOGGLE_ID,
        model.pause_label.unwrap_or("Pause Recording"),
    )
    .enabled(model.pause_enabled)
    .build(app)?;
    #[cfg(target_os = "macos")]
    let exclude_current =
        MenuItemBuilder::with_id(EXCLUDE_CURRENT_APP_ID, "Exclude Current App From Now On...")
            .build(app)?;
    let delete_1 =
        MenuItemBuilder::with_id(DELETE_LAST_1_MINUTE_ID, "Delete Last 1 Minute...").build(app)?;
    let delete_5 = MenuItemBuilder::with_id(DELETE_LAST_5_MINUTES_ID, "Delete Last 5 Minutes...")
        .build(app)?;
    let delete_15 =
        MenuItemBuilder::with_id(DELETE_LAST_15_MINUTES_ID, "Delete Last 15 Minutes...")
            .build(app)?;
    let delete_recent = Submenu::with_items(
        app,
        "Delete Recent Capture",
        true,
        &[&delete_1, &delete_5, &delete_15],
    )?;
    let open_main = MenuItemBuilder::with_id(OPEN_MAIN_ID, "Open Mnema").build(app)?;
    let settings = MenuItemBuilder::with_id(OPEN_SETTINGS_ID, "Settings").build(app)?;
    let quit = MenuItemBuilder::with_id(QUIT_ID, "Quit Mnema").build(app)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let separator_two = PredefinedMenuItem::separator(app)?;

    // A non-actionable status header surfaced only while capture is held by the
    // low-disk liveness suspension, so the menu never reads as plainly running.
    let status_header = model
        .status_label
        .map(|label| {
            MenuItemBuilder::with_id(STATUS_HEADER_ID, label)
                .enabled(false)
                .build(app)
        })
        .transpose()?;
    let status_separator = status_header
        .as_ref()
        .map(|_| PredefinedMenuItem::separator(app))
        .transpose()?;

    let mut items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = Vec::new();
    if let (Some(header), Some(sep)) = (status_header.as_ref(), status_separator.as_ref()) {
        items.push(header);
        items.push(sep);
    }
    // The pause item stays visible on every platform, disabled while not
    // recording, matching the existing macOS tray behaviour.
    items.extend([
        &recording as &dyn tauri::menu::IsMenuItem<tauri::Wry>,
        &pause,
        &sources,
    ]);
    #[cfg(target_os = "macos")]
    items.push(&exclude_current);
    items.extend([
        &delete_recent as &dyn tauri::menu::IsMenuItem<tauri::Wry>,
        &separator_two,
        &open_main,
        &settings,
        &separator,
        &quit,
    ]);

    Menu::with_items(app, &items)
}

pub(crate) fn initialize(app: &tauri::AppHandle) -> tauri::Result<()> {
    let model = current_model(app);
    let menu = build_menu(app, &model)?;
    let icon = tray_template_icon()?;
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true)
        .tooltip(model.tooltip)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
        .build(app)?;

    app.state::<StatusBarState>()
        .lock()
        .expect("status bar state poisoned")
        .tray = Some(tray);
    Ok(())
}

pub(crate) fn refresh(app: &tauri::AppHandle) {
    let model = current_model(app);
    let menu = match build_menu(app, &model) {
        Ok(menu) => menu,
        Err(error) => {
            crate::native_capture::debug_log::log_warn(format!(
                "failed to rebuild status-bar menu: {error}"
            ));
            return;
        }
    };

    let state = app.state::<StatusBarState>();
    let runtime = state.lock().expect("status bar state poisoned");
    let Some(tray) = runtime.tray.as_ref() else {
        return;
    };
    if let Err(error) = tray.set_menu(Some(menu)) {
        crate::native_capture::debug_log::log_warn(format!(
            "failed to set status-bar menu: {error}"
        ));
    }
    if let Err(error) = tray.set_tooltip(Some(model.tooltip)) {
        crate::native_capture::debug_log::log_warn(format!(
            "failed to set status-bar tooltip: {error}"
        ));
    }
}

fn show_capture_error(app: &tauri::AppHandle, title: &str, error: CaptureErrorResponse) {
    app.dialog()
        .message(error.message)
        .kind(MessageDialogKind::Error)
        .title(title)
        .show(|_| {});
}

fn handle_recording_toggle(app: &tauri::AppHandle) {
    if operation(app) != StatusBarOperation::Idle {
        return;
    }
    let recording = crate::native_capture::current_native_capture_session(app).is_running;
    let next_operation = if recording {
        StatusBarOperation::Stopping
    } else {
        StatusBarOperation::Starting
    };
    set_operation(app, next_operation);
    refresh(app);

    let app_handle = app.clone();
    std::thread::spawn(move || {
        let result = if recording {
            crate::native_capture::stop_native_capture_from_app_handle(&app_handle)
                .map(|_| ())
                .map_err(|error| ("Recording could not stop", error))
        } else {
            crate::native_capture::start_native_capture_from_app_handle("status-bar", &app_handle)
                .map(|_| ())
                .map_err(|error| ("Recording could not start", error))
        };

        set_operation(&app_handle, StatusBarOperation::Idle);
        refresh(&app_handle);
        if let Err((title, error)) = result {
            show_capture_error(&app_handle, title, error);
        }
    });
}

fn handle_source_toggle(app: &tauri::AppHandle, id: &str) {
    let model = current_model(app);
    let Some(item) = model.source_items.iter().find(|item| item.id == id) else {
        return;
    };
    if !item.enabled {
        refresh(app);
        return;
    }

    let settings = crate::native_capture::current_recording_settings_from_app_handle(app);
    let current = CaptureSources {
        screen: settings.capture_screen,
        microphone: settings.capture_microphone,
        system_audio: settings.capture_system_audio,
    };
    let Some(next) = computed_toggle_sources(current, id, model.system_audio_requires_screen)
    else {
        refresh(app);
        return;
    };

    if let Err(error) = crate::native_capture::update_recording_sources_from_app_handle(app, next) {
        crate::native_capture::debug_log::log_warn(format!(
            "failed to update recording sources from status bar: [{}] {}",
            error.code, error.message
        ));
        refresh(app);
    }
}

fn handle_pause_toggle(app: &tauri::AppHandle) {
    let session = crate::native_capture::current_native_capture_session(app);
    if !session.is_running {
        return;
    }
    let result = if session.is_user_paused {
        crate::native_capture::resume_native_capture_from_app_handle(app)
            .map(|_| ())
            .map_err(|error| ("Recording could not resume", error))
    } else {
        crate::native_capture::pause_native_capture_from_app_handle(app)
            .map(|_| ())
            .map_err(|error| ("Recording could not pause", error))
    };
    if let Err((title, error)) = result {
        show_capture_error(app, title, error);
    }
}

fn confirm_delete_recent(app: &tauri::AppHandle, seconds: i64) {
    let label = match seconds {
        60 => "last 1 minute",
        300 => "last 5 minutes",
        900 => "last 15 minutes",
        _ => "recent capture",
    };
    let app_handle = app.clone();
    app.dialog()
        .message(format!(
            "Delete the {label} from Mnema's library? This removes whole overlapping capture segments and cannot be undone."
        ))
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Delete".to_string(),
            "Cancel".to_string(),
        ))
        .kind(MessageDialogKind::Warning)
        .title("Delete Recent Capture")
        .show(move |confirmed| {
            if confirmed {
                tauri::async_runtime::spawn(async move {
                    match crate::app_infra::delete_recent_capture_from_app_handle(
                        &app_handle,
                        seconds,
                    )
                    .await
                    {
                        Ok(summary) if summary.file_delete_errors > 0 => {
                            app_handle
                                .dialog()
                                .message(format!(
                                    "Mnema removed the matching library rows, but {} file path(s) could not be deleted from disk. They have been queued for retry. Pending file tombstones: {}.",
                                    summary.file_delete_errors,
                                    summary.pending_file_tombstones
                                ))
                                .kind(MessageDialogKind::Warning)
                                .title("Delete Recent Capture Incomplete")
                                .show(|_| {});
                        }
                        Ok(_) => {}
                        Err(error) => {
                            app_handle
                                .dialog()
                                .message(error)
                                .kind(MessageDialogKind::Error)
                                .title("Delete Recent Capture Failed")
                                .show(|_| {});
                        }
                    }
                });
            }
        });
}

#[cfg(target_os = "macos")]
fn handle_exclude_current_app(app: &tauri::AppHandle) {
    let snapshot = crate::native_capture::metadata::collect_native_active_window_snapshot();
    let bundle_id = snapshot.bundle_id.unwrap_or_default();
    let display_name = snapshot.app_name.unwrap_or_else(|| bundle_id.clone());
    if bundle_id.trim().is_empty() || bundle_id == "com.shaikzeeshan.mnema" {
        app.dialog()
            .message("Mnema could not identify a frontmost app to exclude from screen capture.")
            .kind(MessageDialogKind::Info)
            .title("Exclude Current App")
            .show(|_| {});
        return;
    }
    let app_handle = app.clone();
    app.dialog()
        .message(format!(
            "Exclude {display_name} from screen content capture from now on? This does not delete historical capture."
        ))
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Exclude".to_string(),
            "Cancel".to_string(),
        ))
        .kind(MessageDialogKind::Warning)
        .title("Exclude Current App")
        .show(move |confirmed| {
            if !confirmed {
                return;
            }
            if let Err(error) = crate::privacy_redaction_sources::add_or_enable_privacy_excluded_app_from_app_handle(
                app_handle.clone(),
                bundle_id,
                display_name,
            ) {
                show_capture_error(&app_handle, "Could not exclude app", error);
            } else {
                app_handle
                    .dialog()
                    .message("The app is excluded from future screen content capture. Historical capture was not deleted.")
                    .kind(MessageDialogKind::Info)
                    .title("App Excluded")
                    .show(|_| {});
            }
        });
}

fn handle_menu_event(app: &tauri::AppHandle, id: &str) {
    match id {
        COMPLETE_SETUP_ID => {
            let _ = crate::windows::open_onboarding_window(app);
        }
        RECORDING_TOGGLE_ID => handle_recording_toggle(app),
        PAUSE_TOGGLE_ID => handle_pause_toggle(app),
        #[cfg(target_os = "macos")]
        EXCLUDE_CURRENT_APP_ID => handle_exclude_current_app(app),
        DELETE_LAST_1_MINUTE_ID => confirm_delete_recent(app, 60),
        DELETE_LAST_5_MINUTES_ID => confirm_delete_recent(app, 300),
        DELETE_LAST_15_MINUTES_ID => confirm_delete_recent(app, 900),
        SOURCE_SCREEN_ID | SOURCE_MICROPHONE_ID | SOURCE_SYSTEM_AUDIO_ID => {
            handle_source_toggle(app, id)
        }
        OPEN_MAIN_ID => {
            let _ = crate::windows::open_main_window(app);
        }
        OPEN_SETTINGS_ID => {
            let _ = crate::windows::focus_main_and_open_settings(app.clone(), None, None);
        }
        QUIT_ID => crate::windows::request_graceful_exit(app),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_capture::settings::default_recording_settings;

    fn support_all() -> StatusBarCaptureSupport {
        support_all_with_system_audio_requires_screen(true)
    }

    fn support_all_with_system_audio_requires_screen(
        system_audio_requires_screen: bool,
    ) -> StatusBarCaptureSupport {
        StatusBarCaptureSupport {
            sources: CaptureSources {
                screen: true,
                microphone: true,
                system_audio: true,
            },
            system_audio_requires_screen,
        }
    }

    fn settings_with_sources(
        screen: bool,
        microphone: bool,
        system_audio: bool,
    ) -> RecordingSettings {
        RecordingSettings {
            capture_screen: screen,
            capture_microphone: microphone,
            capture_system_audio: system_audio,
            ..default_recording_settings()
        }
    }

    #[test]
    fn pre_onboarding_model_shows_setup_items_only() {
        let model = build_menu_model(
            false,
            false,
            false,
            false,
            &settings_with_sources(true, false, false),
            &support_all(),
            StatusBarOperation::Idle,
        );
        assert!(!model.onboarding_complete);
        assert_eq!(model.recording_label, None);
        assert!(model.source_items.is_empty());
    }

    #[test]
    fn post_onboarding_idle_model_shows_start_and_enabled_sources() {
        let model = build_menu_model(
            true,
            false,
            false,
            false,
            &settings_with_sources(true, true, false),
            &support_all(),
            StatusBarOperation::Idle,
        );
        assert_eq!(model.recording_label, Some("Start Recording"));
        assert!(model.recording_enabled);
        assert_eq!(
            model.source_items,
            vec![
                SourceItemModel {
                    id: SOURCE_SCREEN_ID,
                    label: "Screen",
                    checked: true,
                    enabled: true
                },
                SourceItemModel {
                    id: SOURCE_MICROPHONE_ID,
                    label: "Microphone",
                    checked: true,
                    enabled: true
                },
                SourceItemModel {
                    id: SOURCE_SYSTEM_AUDIO_ID,
                    label: "System Audio",
                    checked: false,
                    enabled: true
                },
            ]
        );
    }

    #[test]
    fn running_model_shows_stop_and_disables_sources() {
        let model = build_menu_model(
            true,
            true,
            false,
            false,
            &settings_with_sources(true, true, true),
            &support_all(),
            StatusBarOperation::Idle,
        );
        assert_eq!(model.recording_label, Some("Stop Recording"));
        assert!(model.recording_enabled);
        assert!(model.source_items.iter().all(|item| !item.enabled));
    }

    #[test]
    fn low_disk_suspended_model_reads_paused_not_recording() {
        // The low-disk liveness suspension keeps the session alive, so it still
        // reports recording == true; the tray must surface the paused-low-disk
        // state and never read as plainly "Recording" (ADR 0040).
        let model = build_menu_model(
            true,
            true,
            false,
            true,
            &settings_with_sources(true, true, true),
            &support_all(),
            StatusBarOperation::Idle,
        );
        assert_eq!(model.status_label, Some("Paused — low disk"));
        assert_eq!(model.tooltip, "Mnema — Paused (low disk)");
        assert_ne!(model.tooltip, "Mnema - Recording");
    }

    #[test]
    fn busy_models_disable_recording_command_and_sources() {
        for operation in [StatusBarOperation::Starting, StatusBarOperation::Stopping] {
            let model = build_menu_model(
                true,
                false,
                false,
                false,
                &settings_with_sources(true, true, true),
                &support_all(),
                operation,
            );
            assert!(!model.recording_enabled);
            assert!(model.source_items.iter().all(|item| !item.enabled));
        }
    }

    #[test]
    fn last_remaining_source_cannot_be_unchecked() {
        let screen = build_menu_model(
            true,
            false,
            false,
            false,
            &settings_with_sources(true, false, false),
            &support_all(),
            StatusBarOperation::Idle,
        );
        assert!(!screen.source_items[0].enabled);

        let microphone = build_menu_model(
            true,
            false,
            false,
            false,
            &settings_with_sources(false, true, false),
            &support_all(),
            StatusBarOperation::Idle,
        );
        assert!(!microphone.source_items[1].enabled);
    }

    #[test]
    fn screen_with_only_system_audio_cannot_be_unchecked_when_system_audio_requires_screen() {
        let model = build_menu_model(
            true,
            false,
            false,
            false,
            &settings_with_sources(true, false, true),
            &support_all_with_system_audio_requires_screen(true),
            StatusBarOperation::Idle,
        );
        assert!(!model.source_items[0].enabled);
        assert!(model.system_audio_requires_screen);
    }

    #[test]
    fn screen_with_only_system_audio_can_be_unchecked_when_system_audio_is_independent() {
        let model = build_menu_model(
            true,
            false,
            false,
            false,
            &settings_with_sources(true, false, true),
            &support_all_with_system_audio_requires_screen(false),
            StatusBarOperation::Idle,
        );
        assert!(model.source_items[0].enabled);
        assert!(!model.system_audio_requires_screen);
    }

    #[test]
    fn unchecking_screen_clears_system_audio_when_system_audio_requires_screen() {
        assert_eq!(
            computed_toggle_sources(
                CaptureSources {
                    screen: true,
                    microphone: true,
                    system_audio: true,
                },
                SOURCE_SCREEN_ID,
                true,
            ),
            Some(CaptureSources {
                screen: false,
                microphone: true,
                system_audio: false,
            })
        );
    }

    #[test]
    fn unchecking_screen_keeps_system_audio_when_system_audio_is_independent() {
        assert_eq!(
            computed_toggle_sources(
                CaptureSources {
                    screen: true,
                    microphone: true,
                    system_audio: true,
                },
                SOURCE_SCREEN_ID,
                false,
            ),
            Some(CaptureSources {
                screen: false,
                microphone: true,
                system_audio: true,
            })
        );
    }

    #[test]
    fn system_audio_is_disabled_when_screen_is_unchecked_and_system_audio_requires_screen() {
        let model = build_menu_model(
            true,
            false,
            false,
            false,
            &settings_with_sources(false, true, false),
            &support_all_with_system_audio_requires_screen(true),
            StatusBarOperation::Idle,
        );
        assert!(!model.source_items[2].enabled);
    }

    #[test]
    fn toggling_system_audio_without_screen_is_rejected_when_system_audio_requires_screen() {
        assert_eq!(
            computed_toggle_sources(
                CaptureSources {
                    screen: false,
                    microphone: true,
                    system_audio: false,
                },
                SOURCE_SYSTEM_AUDIO_ID,
                true,
            ),
            None
        );
    }

    #[test]
    fn toggling_system_audio_without_screen_is_allowed_when_system_audio_is_independent() {
        assert_eq!(
            computed_toggle_sources(
                CaptureSources {
                    screen: false,
                    microphone: true,
                    system_audio: false,
                },
                SOURCE_SYSTEM_AUDIO_ID,
                false,
            ),
            Some(CaptureSources {
                screen: false,
                microphone: true,
                system_audio: true,
            })
        );
    }

    #[test]
    fn system_audio_is_enabled_when_screen_is_unchecked_and_system_audio_is_independent() {
        let model = build_menu_model(
            true,
            false,
            false,
            false,
            &settings_with_sources(false, true, false),
            &support_all_with_system_audio_requires_screen(false),
            StatusBarOperation::Idle,
        );
        assert!(model.source_items[2].enabled);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn recording_model_exposes_pause_control_off_windows() {
        let model = build_menu_model(
            true,
            false,
            false,
            false,
            &settings_with_sources(true, true, false),
            &support_all(),
            StatusBarOperation::Idle,
        );
        assert_eq!(model.pause_label, Some("Pause Recording"));
        assert!(model.pause_enabled);

        let paused = build_menu_model(
            true,
            true,
            true,
            false,
            &settings_with_sources(true, true, false),
            &support_all(),
            StatusBarOperation::Idle,
        );
        assert_eq!(paused.pause_label, Some("Resume Recording"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn pause_control_is_exposed_on_windows() {
        // F22: pause/resume is honoured by the backend on every platform
        // (`pause_user_capture` is cross-platform and `resume_user_capture` has a
        // working Windows arm), and the global `pauseResumeRecording` keyboard
        // shortcut is not platform-gated, so the tray must offer it on Windows too
        // — the same way it does on macOS — rather than diverge from the shortcut.
        let recording = build_menu_model(
            true,
            true,
            false,
            false,
            &settings_with_sources(true, true, false),
            &support_all(),
            StatusBarOperation::Idle,
        );
        assert_eq!(recording.pause_label, Some("Pause Recording"));
        assert!(recording.pause_enabled);

        let paused = build_menu_model(
            true,
            true,
            true,
            false,
            &settings_with_sources(true, true, false),
            &support_all(),
            StatusBarOperation::Idle,
        );
        assert_eq!(paused.pause_label, Some("Resume Recording"));
        assert!(paused.pause_enabled);

        // Not recording: nothing to pause, on Windows as elsewhere.
        let idle = build_menu_model(
            true,
            false,
            false,
            false,
            &settings_with_sources(true, true, false),
            &support_all(),
            StatusBarOperation::Idle,
        );
        assert_eq!(idle.pause_label, None);
        assert!(!idle.pause_enabled);
    }

    #[test]
    fn unsupported_sources_are_disabled_without_mutating_checked_state() {
        let model = build_menu_model(
            true,
            false,
            false,
            false,
            &settings_with_sources(true, true, true),
            &StatusBarCaptureSupport {
                sources: CaptureSources {
                    screen: true,
                    microphone: false,
                    system_audio: false,
                },
                system_audio_requires_screen: true,
            },
            StatusBarOperation::Idle,
        );
        assert!(model.source_items[0].checked);
        assert!(model.source_items[1].checked);
        assert!(!model.source_items[1].enabled);
        assert!(model.source_items[2].checked);
        assert!(!model.source_items[2].enabled);
    }
}
