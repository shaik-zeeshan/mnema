//! Windows foreground-change listener (ADR 0043, issue #141).
//!
//! The Windows metadata path (#139) refreshes `latest_snapshot` on the segment
//! loop's ≤1s poll. A frame captured in the gap after a sub-second focus switch
//! would otherwise carry the *previous* app's label. This module closes that gap:
//! a `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` fires the instant focus changes,
//! driving a prompt metadata-only refresh. The 1s poll stays as the fallback.
//!
//! Concurrency shape (all on ONE dedicated thread):
//! - `SetWinEventHook` is installed with `WINEVENT_OUTOFCONTEXT`, so the OS
//!   delivers the callback on THIS thread while it pumps messages
//!   (`GetMessageW`/`TranslateMessage`/`DispatchMessageW`).
//! - The callback ([`foreground_event_proc`]) is **signal-only**: it sets an
//!   atomic flag and (re)arms a single NULL-hwnd thread timer. It does no metadata
//!   collection, holds no lock, and allocates nothing.
//! - The heavy work (Win32 process/window queries, version-info file reads, the
//!   `CaptureMetadataState` store) runs OFF the callback, in the `WM_TIMER`
//!   handler. Re-arming the timer on each event collapses a burst of switches into
//!   ONE refresh ~[`FOREGROUND_DEBOUNCE_MS`] after the last event (debounce).
//!
//! Because the callback and the message loop run on the same thread, the timer id
//! and dirty flag are shared without locks. The only cross-thread signal is
//! teardown, which posts `WM_QUIT` via [`PostThreadMessageW`] (thread-safe).
//!
//! No macOS token/generation coalescer is ported: it guards a live content filter
//! Windows lacks (ADR 0043). The `CaptureMetadataState` mutex serializes this
//! writer with the poll writer — last-writer-wins with fresh data is correct.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread::JoinHandle;

use tauri::Manager;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, PeekMessageW, PostThreadMessageW, SetTimer,
    TranslateMessage, EVENT_SYSTEM_FOREGROUND, MSG, PM_NOREMOVE, WINEVENT_OUTOFCONTEXT, WM_QUIT,
    WM_TIMER, WM_USER,
};

/// Debounce window: refresh once, ~this long after the last foreground switch in a
/// burst. Well under the 1s poll so per-frame app attribution stays fresh, yet long
/// enough to coalesce the rapid multi-switch bursts Alt-Tab produces.
const FOREGROUND_DEBOUNCE_MS: u32 = 120;

/// Set true by the WinEvent callback on every foreground change; consumed
/// (swapped false) by the `WM_TIMER` handler. Touched only on the listener thread,
/// so `Relaxed` ordering is sufficient.
static FOREGROUND_DIRTY: AtomicBool = AtomicBool::new(false);

/// Id of the pending NULL-hwnd debounce timer (0 = none). The callback reuses this
/// id when re-arming so a burst collapses into one timer; the `WM_TIMER` handler
/// kills it and resets to 0. Listener-thread-only, hence `Relaxed`.
static DEBOUNCE_TIMER_ID: AtomicUsize = AtomicUsize::new(0);

/// Live listener handle for clean teardown at app exit. `None` until started and
/// after stop. Guards against a second listener thread (start is idempotent).
static FOREGROUND_LISTENER: Mutex<Option<ForegroundListener>> = Mutex::new(None);

struct ForegroundListener {
    /// OS thread id of the listener, for `PostThreadMessageW(WM_QUIT)` teardown.
    thread_id: u32,
    join: JoinHandle<()>,
}

/// Install the foreground-change listener on a dedicated thread. Idempotent: a
/// second call while a listener is live is a no-op. Blocks briefly until the
/// listener thread has created its message queue and reported its id, so a later
/// teardown can never `PostThreadMessageW` ahead of queue creation.
pub(crate) fn start_windows_foreground_listener(app_handle: tauri::AppHandle) {
    let mut listener = match FOREGROUND_LISTENER.lock() {
        Ok(listener) => listener,
        Err(_) => {
            super::debug_log::log_warn(
                "Windows foreground listener state poisoned; skipping registration",
            );
            return;
        }
    };
    if listener.is_some() {
        return;
    }

    let (ready_tx, ready_rx) = mpsc::channel::<u32>();
    let join = match std::thread::Builder::new()
        .name("mnema-fg-listener".to_string())
        .spawn(move || run_foreground_listener(app_handle, ready_tx))
    {
        Ok(join) => join,
        Err(error) => {
            super::debug_log::log_warn(format!(
                "failed to spawn Windows foreground listener thread: {error}"
            ));
            return;
        }
    };

    // Wait for the thread to report its (queue-ready) id. `recv` returns `Err` only
    // if the thread ended before reporting (it never does), in which case join it.
    match ready_rx.recv() {
        Ok(thread_id) => {
            *listener = Some(ForegroundListener { thread_id, join });
        }
        Err(_) => {
            super::debug_log::log_warn(
                "Windows foreground listener thread exited before reporting readiness",
            );
            let _ = join.join();
        }
    }
}

/// Tear down the listener: signal the thread to quit (`WM_QUIT`), which makes
/// `GetMessageW` return 0 so the thread unhooks the WinEvent hook and exits, then
/// join it. No-op if no listener is live. Called from the app-exit finalization so
/// neither the hook nor the thread leaks.
pub(crate) fn stop_windows_foreground_listener() {
    let taken = match FOREGROUND_LISTENER.lock() {
        Ok(mut listener) => listener.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    };
    let Some(listener) = taken else {
        return;
    };

    // SAFETY: `PostThreadMessageW` is thread-safe; the target thread's queue was
    // forced into existence before its id was reported, so this cannot race queue
    // creation. `WM_QUIT` makes the listener's `GetMessageW` return 0 and exit.
    let posted = unsafe { PostThreadMessageW(listener.thread_id, WM_QUIT, 0, 0) };
    if posted == 0 {
        super::debug_log::log_warn(format!(
            "failed to post WM_QUIT to Windows foreground listener thread: {}",
            std::io::Error::last_os_error()
        ));
        // Do not join: without the quit message the thread would block in
        // `GetMessageW` forever and hang the exit. The process is terminating, so
        // the OS reclaims the (idle) thread and hook.
        return;
    }
    if listener.join.join().is_err() {
        super::debug_log::log_warn("Windows foreground listener thread panicked during teardown");
    }
}

/// The WinEvent hook callback (the `WINEVENTPROC` signature). SIGNAL-ONLY: it flags
/// the foreground change and (re)arms the debounce timer. No collection, no lock,
/// no allocation. Runs on the listener thread (OUTOFCONTEXT), so it shares
/// `DEBOUNCE_TIMER_ID` / `FOREGROUND_DIRTY` with the message loop without locking.
unsafe extern "system" fn foreground_event_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _id_event_thread: u32,
    _dwms_event_time: u32,
) {
    FOREGROUND_DIRTY.store(true, Ordering::Relaxed);
    // (Re)arm a single NULL-hwnd thread timer. Passing the stored id replaces the
    // pending timer (restarting its countdown) instead of stacking a new one, so a
    // rapid burst of switches coalesces into one `WM_TIMER`. A fresh (0) id makes
    // `SetTimer` allocate and return a new id.
    let existing = DEBOUNCE_TIMER_ID.load(Ordering::Relaxed);
    let id = SetTimer(std::ptr::null_mut(), existing, FOREGROUND_DEBOUNCE_MS, None);
    DEBOUNCE_TIMER_ID.store(id, Ordering::Relaxed);
}

/// Listener thread body: force the message queue to exist, report readiness,
/// install the hook, then pump messages until `WM_QUIT`, refreshing on each
/// debounced `WM_TIMER`. Unhooks the WinEvent hook and kills any pending timer on
/// exit so nothing leaks.
fn run_foreground_listener(app_handle: tauri::AppHandle, ready_tx: mpsc::Sender<u32>) {
    // SAFETY: standard Win32 message-loop + WinEvent-hook usage, all on this
    // thread. Buffers are stack `MSG`s; the hook handle is unhooked before return.
    unsafe {
        // Force the thread message queue to be created before we hand out the
        // thread id, so a teardown `PostThreadMessageW(WM_QUIT)` cannot race ahead
        // of queue creation. `PM_NOREMOVE` inspects without dispatching.
        let mut probe: MSG = std::mem::zeroed();
        let _ = PeekMessageW(
            &mut probe,
            std::ptr::null_mut(),
            WM_USER,
            WM_USER,
            PM_NOREMOVE,
        );

        let thread_id = GetCurrentThreadId();

        let hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            std::ptr::null_mut(),
            Some(foreground_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        if hook.is_null() {
            // Hook failed: the 1s poll still refreshes as the fallback. Keep pumping
            // so teardown still joins cleanly via WM_QUIT.
            super::debug_log::log_warn(format!(
                "failed to install EVENT_SYSTEM_FOREGROUND hook; foreground refresh degraded to the 1s poll: {}",
                std::io::Error::last_os_error()
            ));
        }

        // Report readiness only after the queue exists and the id is known.
        let _ = ready_tx.send(thread_id);

        loop {
            let mut msg: MSG = std::mem::zeroed();
            let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if ret == 0 {
                // WM_QUIT: teardown requested.
                break;
            }
            if ret == -1 {
                super::debug_log::log_warn(format!(
                    "Windows foreground listener GetMessageW failed; stopping listener: {}",
                    std::io::Error::last_os_error()
                ));
                break;
            }
            if msg.message == WM_TIMER {
                // Debounce elapsed. Kill the timer, then collect off-callback if a
                // foreground change is pending and capture is running.
                KillTimer(std::ptr::null_mut(), msg.wParam);
                DEBOUNCE_TIMER_ID.store(0, Ordering::Relaxed);
                if FOREGROUND_DIRTY.swap(false, Ordering::Relaxed) {
                    refresh_if_recording(&app_handle);
                }
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if !hook.is_null() {
            UnhookWinEvent(hook);
        }
        // Kill any timer still armed when we were told to quit, so it never leaks.
        let pending = DEBOUNCE_TIMER_ID.swap(0, Ordering::Relaxed);
        if pending != 0 {
            KillTimer(std::ptr::null_mut(), pending);
        }
    }
}

/// Refresh the metadata-only snapshot from the current foreground app — the SAME
/// writer the 1s poll uses — but only while recording. This mirrors macOS's
/// `request_privacy_filter_refresh`, which no-ops when capture is stopped: an idle
/// foreground switch must never write `latest_snapshot` (start-of-capture does its
/// own initial refresh and stop clears it). The `CaptureMetadataState` mutex
/// serializes this with the poll writer; last-writer-wins with fresh data.
fn refresh_if_recording(app_handle: &tauri::AppHandle) {
    if !crate::native_capture::current_native_capture_session(app_handle).is_running {
        return;
    }
    let metadata = app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .metadata
        .clone();
    crate::native_capture::metadata::refresh_windows_metadata_snapshot(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
        &metadata,
    );
}
