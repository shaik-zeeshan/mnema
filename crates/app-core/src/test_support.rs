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

/// Records every emit and opened URL; confirms answer yes; everything else is a no-op.
#[derive(Default)]
pub(crate) struct FakeHost {
    pub(crate) events: Mutex<Vec<CoreEvent>>,
    pub(crate) opened_urls: Mutex<Vec<String>>,
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
    fn open_url(&self, url: &str) -> Result<(), String> {
        self.opened_urls.lock().unwrap().push(url.to_string());
        Ok(())
    }
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

/// Pins the process-global secret vault slot to one shared, file-key-backed
/// scratch vault so tests of the `app_infra` secret store never touch the real
/// keychain. One per test process (the slot is process-wide), so tests must use
/// accounts/ids unique to themselves. Same as the desktop crate's copy.
pub(crate) fn install_shared_test_secret_vault() {
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INSTALLED.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("mnema-app-core-test-vault-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("shared test vault dir should be created");
        app_infra::install_process_secret_vault(app_infra::SecretVaultHandle::with_source(
            &dir,
            Arc::new(app_infra::FileMasterKeySource::new(dir.join("master.key"))),
        ));
    });
}
