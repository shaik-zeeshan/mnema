//! `Host::confirm` / `Host::message` as native NSAlert sheets on a parent window
//! (GPUI `Window::prompt`), never a parentless alert, plus the folder picker.
//! With nothing visible (a tray action), the main window is brought up first so
//! the sheet has a parent.

use std::path::PathBuf;

use app_core::{ConfirmDialog, DialogKind, MessageDialog};
use gpui_kit::{AnyWindowHandle, App, PathPromptOptions, PromptButton, PromptLevel};
use tokio::sync::oneshot;

fn level(kind: DialogKind) -> PromptLevel {
    match kind {
        DialogKind::Info => PromptLevel::Info,
        DialogKind::Warning => PromptLevel::Warning,
        DialogKind::Error => PromptLevel::Critical,
    }
}

/// The key window if it is one of ours and visible, else the main window (shown).
fn parent(cx: &mut App) -> Option<AnyWindowHandle> {
    if let Some(active) = cx.active_window() {
        let visible = active
            .update(cx, |_, window, _| super::is_visible(window))
            .unwrap_or(false);
        if visible {
            return Some(active);
        }
    }
    crate::window::show_main(cx)
}

/// Show an alert; `on_answer` gets the clicked button's index (`None` when no
/// parent could be found).
fn prompt(
    kind: DialogKind,
    title: Option<&str>,
    message: &str,
    answers: Vec<PromptButton>,
    cx: &mut App,
    on_answer: impl FnOnce(Option<usize>) + 'static,
) {
    let Some(handle) = parent(cx) else {
        log::warn!("dialog skipped: no window to attach it to");
        on_answer(None);
        return;
    };
    // NSAlert: the bold line is the title when there is one, the message below it.
    let (head, detail) = match title {
        Some(t) => (t, Some(message)),
        None => (message, None),
    };
    let rx = handle.update(cx, |_, window, cx| {
        window.prompt(level(kind), head, detail, &answers, cx)
    });
    let Ok(rx) = rx else {
        on_answer(None);
        return;
    };
    cx.spawn(async move |_| on_answer(rx.await.ok())).detach();
}

/// `Host::confirm`: OK first (Return), Cancel second (Esc). Anything but OK reads as cancel.
pub fn confirm(req: ConfirmDialog, tx: oneshot::Sender<bool>, cx: &mut App) {
    let answers = vec![
        PromptButton::ok(req.ok_label),
        PromptButton::cancel(req.cancel_label),
    ];
    prompt(
        req.kind,
        req.title.as_deref(),
        &req.message,
        answers,
        cx,
        move |ix| {
            let _ = tx.send(ix == Some(0));
        },
    );
}

/// `Host::message`: one OK button.
pub fn message(req: MessageDialog, cx: &mut App) {
    prompt(
        req.kind,
        Some(&req.title),
        &req.message,
        vec![PromptButton::ok("OK")],
        cx,
        |_| {},
    );
}

/// An app-modal warning before any window exists (the "already running" exit).
/// Runs a nested modal loop: call it only outside any `App` borrow.
pub fn run_modal_alert(title: &str, message: &str) {
    use objc2_app_kit::{NSAlert, NSAlertStyle};
    use objc2_foundation::{MainThreadMarker, NSString};
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("alert skipped: not on the main thread");
        return;
    };
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Warning);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));
    alert.runModal();
}

/// The native folder picker (NSOpenPanel). `then` gets the chosen folder.
pub fn pick_folder(prompt: &str, cx: &mut App, then: impl FnOnce(PathBuf, &mut App) + 'static) {
    let rx = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some(prompt.to_string().into()),
    });
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(mut paths))) = rx.await {
            if let Some(path) = paths.pop() {
                let _ = cx.update(|cx| then(path, cx));
            }
        }
    })
    .detach();
}
