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

/// Records every emit, message dialog, tray refresh, opened URL, window request
/// and window signal; confirms answer yes; `open_window` fails with
/// `open_window_error` when set; everything else is a no-op.
#[derive(Default)]
pub(crate) struct FakeHost {
    pub(crate) events: Mutex<Vec<CoreEvent>>,
    pub(crate) messages: Mutex<Vec<MessageDialog>>,
    pub(crate) tray_refreshes: std::sync::atomic::AtomicUsize,
    pub(crate) opened_urls: Mutex<Vec<String>>,
    pub(crate) opened_windows: Mutex<Vec<WindowRequest>>,
    pub(crate) window_signals: Mutex<Vec<WindowSignal>>,
    pub(crate) open_window_error: Mutex<Option<String>>,
}

impl Host for FakeHost {
    fn emit(&self, event: CoreEvent) {
        self.events.lock().unwrap().push(event);
    }
    fn message(&self, req: MessageDialog) {
        self.messages.lock().unwrap().push(req);
    }
    fn confirm(&self, _req: ConfirmDialog) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(true);
        rx
    }
    fn open_url(&self, url: &str) -> Result<(), String> {
        self.opened_urls.lock().unwrap().push(url.to_string());
        Ok(())
    }
    fn refresh_tray(&self) {
        self.tray_refreshes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    fn open_window(&self, req: WindowRequest) -> Result<(), String> {
        self.opened_windows.lock().unwrap().push(req);
        match self.open_window_error.lock().unwrap().clone() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    fn window_signal(&self, signal: WindowSignal) {
        self.window_signals.lock().unwrap().push(signal);
    }
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

/// Runs `future` to completion on one shared multi-thread runtime, the way the
/// desktop tests' `tauri::async_runtime::block_on` did (Tauri lazily builds one
/// global runtime for it).
pub(crate) fn block_on<F: std::future::Future>(future: F) -> F::Output {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME
        .get_or_init(|| tokio::runtime::Runtime::new().expect("test runtime starts"))
        .block_on(future)
}
