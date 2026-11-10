//! `GpuiHost`: app-core's seam into this app. Every call may come from any thread,
//! so it only forwards a message; `bridge::events` handles them on the main thread.

use app_core::{
    ConfirmDialog, CoreEvent, ExitKind, Host, MessageDialog, WindowRequest, WindowSignal,
};
use tokio::sync::oneshot;

pub enum HostMsg {
    Event(CoreEvent),
    Message(MessageDialog),
    Confirm(ConfirmDialog, oneshot::Sender<bool>),
    OpenUrl(String),
    RefreshTray,
    OpenWindow(WindowRequest),
    WindowSignal(WindowSignal),
    Exit(ExitKind),
}

pub struct GpuiHost {
    tx: async_channel::Sender<HostMsg>,
}

impl GpuiHost {
    pub fn new() -> (Self, async_channel::Receiver<HostMsg>) {
        let (tx, rx) = async_channel::unbounded();
        (Self { tx }, rx)
    }

    fn send(&self, msg: HostMsg) {
        // Closed only after the app's event loop is gone (quitting): nothing to tell.
        let _ = self.tx.try_send(msg);
    }
}

impl Host for GpuiHost {
    fn emit(&self, event: CoreEvent) {
        self.send(HostMsg::Event(event));
    }

    fn message(&self, req: MessageDialog) {
        self.send(HostMsg::Message(req));
    }

    fn confirm(&self, req: ConfirmDialog) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        self.send(HostMsg::Confirm(req, tx));
        rx
    }

    fn open_url(&self, target: &str) -> Result<(), String> {
        // GPUI opens asynchronously on the main thread and reports no failure.
        self.send(HostMsg::OpenUrl(target.to_string()));
        Ok(())
    }

    fn refresh_tray(&self) {
        self.send(HostMsg::RefreshTray);
    }

    fn open_window(&self, req: WindowRequest) -> Result<(), String> {
        // Like `open_url`: shown asynchronously on the main thread, no failure reported.
        self.send(HostMsg::OpenWindow(req));
        Ok(())
    }

    fn window_signal(&self, signal: WindowSignal) {
        // GPUI reads preview files directly; there is no asset scope to grant.
        if matches!(signal, WindowSignal::AllowAssetPath { .. }) {
            return;
        }
        self.send(HostMsg::WindowSignal(signal));
    }

    fn exit(&self, kind: ExitKind) {
        match kind {
            // Ended right here, on the shutdown task's thread (as the Tauri host
            // does): a hop to the main thread waited behind whatever it was doing.
            ExitKind::Quit => crate::native::quit::exit_after_graceful_exit(),
            // A relaunch needs `cx.restart()`, on the main thread.
            ExitKind::Restart => self.send(HostMsg::Exit(kind)),
        }
    }
}
