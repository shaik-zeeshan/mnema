//! The idle-debug read under the capture lock. It moves to app-core with the
//! Recording Lifecycle (S3 part c); the rest is `app_core::native_capture::activity`.

use super::lifecycle::RecordingLifecycle;
use super::NativeCaptureState;
use capture_types::IdleDebugInfo;
use std::sync::MutexGuard;

pub(super) fn lock_runtime_for_idle_debug(
    state: &NativeCaptureState,
) -> MutexGuard<'_, RecordingLifecycle> {
    match state.lock() {
        Ok(runtime) => runtime,
        Err(poisoned) => {
            app_core::debug_log::log_warn(
                "native capture state poisoned while reading idle debug; returning best-effort snapshot",
            );
            poisoned.into_inner()
        }
    }
}

pub(super) fn get_idle_debug(state: tauri::State<'_, NativeCaptureState>) -> IdleDebugInfo {
    let mut runtime = lock_runtime_for_idle_debug(&state);
    app_core::native_capture::activity::idle_debug_for_runtime(runtime.runtime_mut())
}
