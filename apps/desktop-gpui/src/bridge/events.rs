//! Host messages -> one `cx.spawn` loop on the main thread -> entity updates.
//! `CoreEvent`s that change what the shell shows reload its model off-thread.

use std::sync::Arc;

use app_core::{AppCore, CoreEvent, ExitKind, WindowRequest, WindowSignal};
use gpui_kit::{App, Entity, Global, WeakEntity};

use super::tokio::Tokio;
use crate::host::HostMsg;
use crate::native;
use crate::shell::{data, Shell};

/// The one `AppCore`, for views that need to start backend work.
pub struct Core(pub Arc<AppCore>);

impl Global for Core {}

/// Refill the shell's model from app-core on tokio, then hand it to the view.
pub fn reload_shell(shell: &Entity<Shell>, cx: &mut App) {
    let Some(core) = cx.try_global::<Core>().map(|c| c.0.clone()) else {
        return;
    };
    let chat = shell.read(cx).chat.clone();
    chat.update(cx, |chat, cx| chat.reload(cx));
    let task = Tokio::spawn(cx, async move { data::load(&core).await });
    let shell = shell.downgrade();
    cx.spawn(async move |cx| {
        if let Ok(model) = task.await {
            let _ = shell.update(cx, |shell, cx| shell.set_model(model, cx));
        }
    })
    .detach();
}

/// Does this event change anything the shell shows?
fn touches_shell(event: &CoreEvent) -> bool {
    matches!(
        event,
        CoreEvent::NativeCaptureSessionChanged(_)
            | CoreEvent::RecordingSettingsChanged(_)
            | CoreEvent::RecordingSettingsDomainChanged(_)
            | CoreEvent::AppNotificationsChanged(_)
            | CoreEvent::LicenseStatus(_)
    )
}

pub fn spawn(rx: async_channel::Receiver<HostMsg>, shell: WeakEntity<Shell>, cx: &mut App) {
    cx.spawn(async move |cx| {
        while let Ok(msg) = rx.recv().await {
            let _ = cx.update(|cx| handle(msg, &shell, cx));
        }
    })
    .detach();
}

fn handle(msg: HostMsg, shell: &WeakEntity<Shell>, cx: &mut App) {
    match msg {
        HostMsg::Event(CoreEvent::OpenAppNotifications) => {
            if let Some(shell) = shell.upgrade() {
                shell.update(cx, |s, cx| {
                    s.overlay = Some(crate::shell::Overlay::Notifications);
                    cx.notify();
                });
            }
        }
        HostMsg::Event(event) if touches_shell(&event) => {
            // Ask AI's switch / model / auto-answer and meaning search re-probe the bar.
            if matches!(
                event,
                CoreEvent::RecordingSettingsDomainChanged(_)
                    | CoreEvent::NativeCaptureSessionChanged(_)
            ) {
                crate::quick_recall::panel::settings_changed(cx);
            }
            if let Some(shell) = shell.upgrade() {
                reload_shell(&shell, cx);
            }
            // Read-Only Mode flips the tray header and Start Recording.
            if matches!(event, CoreEvent::LicenseStatus(_)) {
                native::tray::refresh(cx);
            }
        }
        HostMsg::Event(CoreEvent::AskAiUpdate(ev)) => {
            crate::quick_recall::panel::ask_update(ev.clone(), cx);
            if let Some(shell) = shell.upgrade() {
                let chat = shell.read(cx).chat.clone();
                chat.update(cx, |chat, cx| chat.on_update(ev, cx));
            }
        }
        HostMsg::Event(CoreEvent::ConversationChanged) => {
            if let Some(shell) = shell.upgrade() {
                let chat = shell.read(cx).chat.clone();
                chat.update(cx, |chat, cx| chat.reload(cx));
            }
        }
        HostMsg::Event(CoreEvent::AppUpdateStatusChanged(status)) => {
            crate::small::update::status_changed(status, cx);
            native::tray::refresh(cx);
        }
        HostMsg::Event(CoreEvent::KeyboardBindingsSettingsChanged(payload)) => {
            if let (Some(core), Ok(settings)) = (
                cx.try_global::<Core>().map(|c| c.0.clone()),
                serde_json::from_value(payload),
            ) {
                native::hotkeys::settings_changed(&core, &settings, cx);
            }
            if let Some(shell) = shell.upgrade() {
                reload_shell(&shell, cx);
            }
        }
        // Surface entities subscribe to theirs as S8+ land.
        HostMsg::Event(_) => {}
        HostMsg::OpenWindow(req) if crate::small::open_request(&req, cx) => {}
        HostMsg::OpenWindow(WindowRequest::Main) => {
            crate::window::show_main(cx);
        }
        HostMsg::OpenWindow(WindowRequest::Settings { tab, .. }) => {
            crate::window::open_settings(tab.as_deref(), cx)
        }
        HostMsg::OpenWindow(_) => {}
        HostMsg::WindowSignal(signal) if crate::small::signal(&signal, cx) => {}
        HostMsg::WindowSignal(signal) => window_signal(signal, cx),
        HostMsg::OpenUrl(target) if target.starts_with('/') => {
            cx.open_with_system(std::path::Path::new(&target))
        }
        HostMsg::OpenUrl(url) => cx.open_url(&url),
        HostMsg::Message(req) => native::dialogs::message(req, cx),
        HostMsg::Confirm(req, tx) => native::dialogs::confirm(req, tx, cx),
        HostMsg::RefreshTray => native::tray::refresh(cx),
        // The graceful exit is done (`AppCore::shutdown`): end now, or relaunch.
        // `restart` spawns the relaunch, then `terminate:` reaches the swizzle,
        // which sees the exit is final and ends the process.
        HostMsg::Exit(ExitKind::Quit) => native::quit::exit_now(),
        HostMsg::Exit(ExitKind::Restart) => {
            log::info!("completed graceful app exit; relaunching");
            cx.restart()
        }
    }
}

fn window_signal(signal: WindowSignal, cx: &mut App) {
    match signal {
        WindowSignal::CaptureExclusionChanged => {
            if let Some(core) = cx.try_global::<Core>().map(|c| c.0.clone()) {
                native::chrome::reapply_sharing(native::developer_options(&core), cx);
            }
        }
        WindowSignal::HideAllViews(asleep) => crate::window::set_screens_asleep(asleep, cx),
        WindowSignal::NotRecordingReminder { post: true } => native::notification::post(cx),
        WindowSignal::NotRecordingReminder { post: false } => native::notification::remove(cx),
        // GPUI parks no views; asset grants are filtered out in `GpuiHost`.
        WindowSignal::RefreshParkedViews | WindowSignal::AllowAssetPath { .. } => {}
        // shortcut: the GPUI updater (feed check every 2 h) is S16.
        WindowSignal::StartUpdateCheckTimer => log::debug!("update check timer: lands with S16"),
        // Handled by `small::signal`.
        WindowSignal::CloseCliAccessRequest => {}
    }
}
