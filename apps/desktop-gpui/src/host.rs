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

    fn open_url(&self, url: &str) {
        self.send(HostMsg::OpenUrl(url.to_string()));
    }

    fn refresh_tray(&self) {
        self.send(HostMsg::RefreshTray);
    }

    fn open_window(&self, req: WindowRequest) {
        self.send(HostMsg::OpenWindow(req));
    }

    fn window_signal(&self, signal: WindowSignal) {
        self.send(HostMsg::WindowSignal(signal));
    }

    fn exit(&self, kind: ExitKind) {
        self.send(HostMsg::Exit(kind));
    }
}
