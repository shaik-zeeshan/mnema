//! Host messages -> one `cx.spawn` loop on the main thread -> entity updates.
//! `CoreEvent`s that change what the shell shows reload its model off-thread.

use std::sync::Arc;

use app_core::{AppCore, CoreEvent, ExitKind};
use gpui_kit::{App, Entity, Global, WeakEntity};

use super::tokio::Tokio;
use crate::host::HostMsg;
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
            | CoreEvent::KeyboardBindingsSettingsChanged(_)
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
            if let Some(shell) = shell.upgrade() {
                reload_shell(&shell, cx);
            }
        }
        HostMsg::Event(CoreEvent::AskAiUpdate(ev)) => {
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
        // Surface entities subscribe to theirs as S8+ land.
        HostMsg::Event(_) => {}
        HostMsg::OpenUrl(target) if target.starts_with('/') => {
            cx.open_with_system(std::path::Path::new(&target))
        }
        HostMsg::OpenUrl(url) => cx.open_url(&url),
        HostMsg::Exit(ExitKind::Quit) => cx.quit(),
        HostMsg::Exit(ExitKind::Restart) => cx.restart(),
        // shortcut: dialogs (with a parent window), tray, extra windows and
        // window signals are S7 / S15; a dropped confirm sender reads as cancel.
        HostMsg::Message(_)
        | HostMsg::Confirm(..)
        | HostMsg::RefreshTray
        | HostMsg::OpenWindow(_)
        | HostMsg::WindowSignal(_) => {
            log::debug!("host: request not handled until S7/S15");
        }
    }
}
