//! Every quit path (⌘Q, the app menu, Dock › Quit, tray Quit, logout, a
//! `terminate:` Apple event, `cx.quit()`) goes through `AppCore::shutdown`
//! first. GPUI can't hold quit up (`on_app_quit` gets 200 ms), so we swizzle
//! `-[NSApplication terminate:]` (the S0 spike, ported from `windows.rs`).

use std::sync::{Arc, OnceLock};

use app_core::exit::AppExitCoordinatorState;
use app_core::{AppCore, ExitKind};
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::sel;

static CORE: OnceLock<Arc<AppCore>> = OnceLock::new();
static ORIGINAL_TERMINATE: OnceLock<usize> = OnceLock::new();

type TerminateFn = unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject);

unsafe extern "C-unwind" fn terminate(app: &AnyObject, cmd: Sel, sender: *mut AnyObject) {
    let Some(core) = CORE.get() else {
        // No core yet (startup failed early): AppKit's own quit.
        if let Some(&imp) = ORIGINAL_TERMINATE.get() {
            // SAFETY: `imp` is NSApplication's own `terminate:` IMP saved at install.
            unsafe { std::mem::transmute::<usize, TerminateFn>(imp)(app, cmd, sender) };
        }
        return;
    };
    if core
        .state::<AppExitCoordinatorState>()
        .is_final_graceful_exit_ready()
    {
        exit_now();
    }
    core.shutdown(ExitKind::Quit);
}

/// Install the swizzle. Call once, on the main thread, before anything can quit.
pub fn install(core: Arc<AppCore>) {
    if CORE.set(core).is_err() {
        return;
    }
    let Some(method) =
        AnyClass::get(c"NSApplication").and_then(|c| c.instance_method(sel!(terminate:)))
    else {
        log::error!("failed to install the terminate handler: NSApplication terminate: not found");
        return;
    };
    // SAFETY: `terminate` has the exact `terminate:` signature (self, _cmd, sender) -> void.
    let previous =
        unsafe { method.set_implementation(std::mem::transmute::<TerminateFn, Imp>(terminate)) };
    let _ = ORIGINAL_TERMINATE.set(previous as usize);
}

/// `AppCore::shutdown` finished: its last log line, then end the process.
pub fn exit_after_graceful_exit() -> ! {
    log::info!("completed graceful app exit; exiting without running process static destructors");
    exit_now()
}

/// The graceful exit is done: end the process now, skipping static destructors
/// (as the Tauri app does; ORT and Metal teardown can hang at exit).
pub fn exit_now() -> ! {
    log::logger().flush();
    unsafe extern "C" {
        fn _exit(status: std::os::raw::c_int) -> !;
    }
    // SAFETY: `_exit` takes an int status and never returns.
    unsafe { _exit(0) }
}
