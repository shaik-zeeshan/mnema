//! Test doubles for app-core's unit tests.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use tokio::sync::oneshot;

use crate::{
    AppCore, ConfirmDialog, CoreEvent, ExitKind, Host, MessageDialog, Paths, WindowRequest,
    WindowSignal,
};

/// Records every emit; confirms answer yes; everything else is a no-op.
#[derive(Default)]
pub(crate) struct FakeHost {
    pub(crate) events: Mutex<Vec<CoreEvent>>,
}

impl Host for FakeHost {
    fn emit(&self, event: CoreEvent) {
        self.events.lock().unwrap().push(event);
    }
    fn message(&self, _req: MessageDialog) {}
    fn confirm(&self, _req: ConfirmDialog) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(true);
        rx
    }
    fn open_url(&self, _url: &str) {}
    fn refresh_tray(&self) {}
    fn open_window(&self, _req: WindowRequest) {}
    fn window_signal(&self, _signal: WindowSignal) {}
    fn exit(&self, _kind: ExitKind) {}
}

/// An `AppCore` on `host` whose data/config/cache/log dirs all live under `dir`.
pub(crate) fn core_in(host: Arc<FakeHost>, dir: &Path) -> AppCore {
    let paths = Paths {
        data: dir.join("data"),
        config: dir.join("config"),
        cache: dir.join("cache"),
        log: dir.join("log"),
    };
    AppCore::new(host, paths).expect("runtime starts")
}
