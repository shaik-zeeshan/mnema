//! The locked half of `get_capture_health`. It moves to app-core with the
//! Recording Lifecycle (S3 part c); the DTOs and the read live in
//! `app_core::native_capture::capture_health`.

use app_core::disk_space;
use app_core::native_capture::capture_health::{read_locked, CaptureHealthDto, Locked};
use tauri::Manager;

use super::NativeCaptureState;

/// The one capture-health seam. Takes the capture lock once, copies out what it
/// needs, then measures free space (a single statfs, no directory walk) after
/// releasing it — on the live recordings root, else the storage root.
pub(crate) fn capture_health_snapshot(
    state: &NativeCaptureState,
    app_handle: &tauri::AppHandle,
) -> CaptureHealthDto {
    let Locked {
        mut dto,
        recordings_root,
    } = {
        let lifecycle = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        read_locked(lifecycle.runtime())
    };

    let root = recordings_root.or_else(|| {
        app_handle
            .try_state::<app_core::AppInfraState>()
            .map(|infra| infra.base_dir().to_path_buf())
    });
    dto.disk.free_bytes = root.and_then(|root| {
        disk_space::measure_free_space(&root, disk_space::default_free_space_probe)
    });
    dto
}
