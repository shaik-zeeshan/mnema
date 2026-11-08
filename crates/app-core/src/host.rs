//! The seam between shared behaviour and the app shell. `AppCore` asks the host
//! for everything that needs a window, a dialog, the tray or the process: the
//! Tauri app implements it in `src-tauri/src/tauri_host.rs`, the GPUI app in S5.

use tokio::sync::oneshot;

use crate::event::CoreEvent;

pub trait Host: Send + Sync + 'static {
    /// Broadcast a state change. Fire-and-forget: a host with no listener drops it.
    fn emit(&self, event: CoreEvent);
    /// Show an informational dialog. Fire-and-forget.
    fn message(&self, req: MessageDialog);
    /// Ask the user to confirm. Resolves `true` on OK; a dropped sender reads as cancel.
    fn confirm(&self, req: ConfirmDialog) -> oneshot::Receiver<bool>;
    /// Open a URL, or an absolute file/folder path (starts with `/`) with its default app.
    /// `Err` carries the OS's reason when the host can tell synchronously.
    fn open_url(&self, target: &str) -> Result<(), String>;
    /// Rebuild the status-bar menu from current state.
    fn refresh_tray(&self);
    fn open_window(&self, req: WindowRequest);
    fn window_signal(&self, signal: WindowSignal);
    fn exit(&self, kind: ExitKind);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDialog {
    pub title: String,
    pub message: String,
    pub kind: DialogKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmDialog {
    pub title: Option<String>,
    pub message: String,
    pub kind: DialogKind,
    pub ok_label: String,
    pub cancel_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowRequest {
    /// Show and focus the main window.
    Main,
    /// Focus the main window on Settings, optionally at a tab and a field.
    Settings {
        tab: Option<String>,
        focus: Option<String>,
    },
    /// The CLI Access approval window for the pending broker request.
    CliAccessRequest,
    Update,
    /// Open the System Settings pane for `kind` (`screen`, `microphone`,
    /// `systemAudio`, `accessibility`) and float the helper beside it.
    PermissionHelper {
        kind: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSignal {
    /// The capture exclusion list changed; re-apply it to every window.
    CaptureExclusionChanged,
    /// Hide (or restore) every window's rendered views, e.g. across display sleep.
    HideAllViews(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitKind {
    /// Graceful quit: stop capture and workers, then exit.
    Quit,
    /// Graceful quit, then relaunch (settings change or installed update).
    Restart,
}
