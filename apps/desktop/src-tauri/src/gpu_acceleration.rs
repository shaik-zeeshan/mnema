//! Windows GPU Acceleration (NVIDIA CUDA Execution Backend) — detection, live
//! Force-CPU/pack state, and CUDA DLL-search wiring (#137, Slice 3 / ADR 0005).
//!
//! The CUDA Execution Backend is orthogonal to identity (one `model_id`, one
//! Voiceprint Space; observable only in provenance). This module owns the
//! *app-side* glue around it:
//!
//!   - [`GpuAccelerationState`] — app-lifetime shared state (managed as Tauri
//!     state). It carries the app data dir (so the GPU-pack dir can be resolved),
//!     the live "Use GPU acceleration" toggle (default ON), a lazily-probed cache
//!     of whether an NVIDIA GPU exists (Settings *offer* only), and the last
//!     job's execution outcome (for the Slice 5 Settings panel). The subprocess
//!     provider reads `force_cpu` + `pack_dir` from it LIVE at every spawn (never
//!     frozen at admission — ADR 0005), so toggling the override takes effect on
//!     the next Speaker Analysis Job.
//!   - [`detect_nvidia_gpu`] — an NVML probe that drives the Settings *offer*
//!     (NOT the CUDA attempt; the pack-gated try/CPU-fallback in `speakrs.rs`
//!     subsumes detection — see `select_execution_mode`).
//!   - [`prepare_cuda_dll_search`] — augments the HELPER process's DLL search path
//!     so `onnxruntime_providers_cuda.dll`'s CUDA/cuDNN dependency DLLs resolve
//!     from the pack dir at load time. This is our realization of ADR 0005's
//!     "redist loaded via ORT preload_dylibs".
//!
//! Everything NVIDIA/CUDA/NVML/DLL-search is `#[cfg(windows)]` with non-Windows
//! no-op shims, so macOS/Linux compile unchanged and macOS behavior is
//! byte-identical (macOS speakrs is always native CoreML — no pack, no toggle).

// This module is the shared GPU-acceleration surface for #137. Slice 3 wires the
// detection + live-toggle + outcome-recording that the SPAWN path needs, but the
// READ/DETECT API — `pack_present`, `set_use_gpu`, `gpu_detected`/`detect_nvidia
// _gpu`, `last_execution_mode`, `last_cuda_fallback_reason` — is consumed by Slice
// 4 (the pack downloader) and Slice 5 (the Settings `get_/set_gpu_acceleration_*`
// Tauri commands), which are deliberately out of scope here. Those items are
// exercised by this module's own unit tests; allow dead_code so the
// intentionally-ahead-of-its-consumers surface does not warn until those slices
// land.
#![allow(dead_code)]

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock, RwLock,
    },
};

/// Filename of the tiny sidecar JSON that persists the "Use GPU acceleration"
/// toggle across restarts, stored directly in the app data dir.
///
/// WHY a self-contained sidecar JSON and NOT a new settings-DB domain: the toggle
/// is a single Windows-only *hardware* switch, read LIVE by the subprocess provider
/// at each helper spawn (never frozen at admission — ADR 0005). It is owned end to
/// end by [`GpuAccelerationState`], which already holds the app data dir, so a file
/// the state reads/writes itself is strictly simpler than threading a brand-new
/// recording-settings domain (schema + change events + migrations + frontend
/// plumbing) through the whole settings stack for one boolean. Default ON, so an
/// absent or corrupt file simply means "GPU acceleration on" — the safe, advertised
/// default.
const GPU_ACCELERATION_SETTINGS_FILE: &str = "gpu-acceleration.json";

/// On-disk shape of the sidecar. A one-field object (rather than a bare bool) so a
/// future Windows-only GPU toggle can be added without a format break.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct GpuAccelerationSettings {
    use_gpu: bool,
}

/// The sidecar path under `app_data_dir`.
fn gpu_acceleration_settings_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(GPU_ACCELERATION_SETTINGS_FILE)
}

/// Read the persisted "Use GPU acceleration" toggle, defaulting to ON (`true`) when
/// the sidecar is absent or unreadable/corrupt. A missing file is a fresh install
/// (default ON), and a corrupt one must never strand the user without the advertised
/// default. Best-effort: never panics.
fn hydrate_use_gpu(app_data_dir: &Path) -> bool {
    match std::fs::read(gpu_acceleration_settings_path(app_data_dir)) {
        Ok(bytes) => serde_json::from_slice::<GpuAccelerationSettings>(&bytes)
            .map(|settings| settings.use_gpu)
            .unwrap_or(true),
        Err(_) => true,
    }
}

/// App-lifetime, shared GPU-acceleration state.
///
/// Constructed once at app init where the app data dir is resolved
/// (`app_infra::desktop_processing_registry`), handed to the subprocess provider
/// (which reads `force_cpu` + `pack_dir` live at each spawn and records the
/// outcome), and `.manage()`d so the Slice 5 Tauri commands can read/update it via
/// `tauri::State<Arc<GpuAccelerationState>>`.
///
/// All interior mutability is lock-poison-safe and cheap: the toggle is an
/// `AtomicBool`, the detection cache a write-once `OnceLock`, and the last-outcome
/// fields short `RwLock<Option<String>>`s read only by the Settings panel.
#[derive(Debug)]
pub struct GpuAccelerationState {
    /// The app data dir. The GPU-pack dir is derived from it on demand
    /// (`speaker_analysis::gpu_acceleration_pack_dir`) rather than cached, so it
    /// always reflects the canonical layout even if the pack is installed later.
    app_data_dir: PathBuf,
    /// Live "Use GPU acceleration" toggle. Default ON (CUDA is used when the pack
    /// is present); flipping it OFF caps backend selection at CPU. Read live at
    /// each helper spawn — see [`GpuAccelerationState::force_cpu`].
    use_gpu: AtomicBool,
    /// Cached NVML probe result. Lazy + write-once: the probe runs at most once
    /// (first [`GpuAccelerationState::gpu_detected`] call) and the answer is
    /// cached. Drives the Settings *offer* ONLY — never the CUDA attempt.
    gpu_detected: OnceLock<bool>,
    /// The Execution Backend that actually ran the last completed job
    /// (`"cpu"` | `"cuda"` | `"coreml"`). Surfaced by the Slice 5 Settings panel.
    last_execution_mode: RwLock<Option<String>>,
    /// The reason CUDA initialization fell back to CPU on the last job, if any.
    /// `Some` ONLY when the last job init-fell-back from CUDA (ADR 0005); the
    /// single diagnostic the Settings panel shows to answer "why isn't my GPU
    /// used?".
    last_cuda_fallback_reason: RwLock<Option<String>>,
}

impl GpuAccelerationState {
    /// Construct the shared state rooted at `app_data_dir`, with "Use GPU
    /// acceleration" ON and an un-probed detection cache.
    pub fn new(app_data_dir: impl Into<PathBuf>) -> Arc<Self> {
        let app_data_dir = app_data_dir.into();
        // Hydrate the live toggle from the sidecar so the user's choice survives
        // restarts; default ON whenever the file is absent or corrupt (ADR 0005 —
        // GPU acceleration is used whenever the pack is present). See
        // `GPU_ACCELERATION_SETTINGS_FILE` for why this is a sidecar, not a DB domain.
        let use_gpu = hydrate_use_gpu(&app_data_dir);
        Arc::new(Self {
            app_data_dir,
            use_gpu: AtomicBool::new(use_gpu),
            gpu_detected: OnceLock::new(),
            last_execution_mode: RwLock::new(None),
            last_cuda_fallback_reason: RwLock::new(None),
        })
    }

    /// The GPU Acceleration Pack dir under the app data dir (canonical layout).
    pub fn pack_dir(&self) -> PathBuf {
        speaker_analysis::gpu_acceleration_pack_dir(&self.app_data_dir)
    }

    /// Whether the GPU pack is installed (its `.installed.json` marker exists).
    /// The ONLY gate (with `force_cpu`) on whether CUDA is attempted.
    pub fn pack_present(&self) -> bool {
        speaker_analysis::gpu_pack_present(&self.pack_dir())
    }

    /// Live "Use GPU acceleration" toggle.
    pub fn use_gpu(&self) -> bool {
        self.use_gpu.load(Ordering::Relaxed)
    }

    /// Update the live "Use GPU acceleration" toggle (Slice 5 command) and persist
    /// it for restart durability.
    pub fn set_use_gpu(&self, use_gpu: bool) {
        self.use_gpu.store(use_gpu, Ordering::Relaxed);
        // Persist for restart durability. The AtomicBool above is already the
        // session's source of truth (the provider reads it live), so persistence is
        // strictly best-effort — a write failure must only log, never panic and
        // never roll back the live toggle.
        self.persist_use_gpu(use_gpu);
    }

    /// Best-effort write of the toggle to the sidecar JSON. A failure (read-only
    /// dir, disk full, serialize error) is logged and swallowed: the only
    /// consequence is that the choice may not survive the NEXT restart — never a
    /// crash, and never a stale in-memory value for THIS session.
    fn persist_use_gpu(&self, use_gpu: bool) {
        let path = gpu_acceleration_settings_path(&self.app_data_dir);
        let payload = GpuAccelerationSettings { use_gpu };
        match serde_json::to_vec(&payload) {
            Ok(bytes) => {
                if let Err(error) = std::fs::write(&path, bytes) {
                    crate::native_capture::debug_log::log_error(format!(
                        "failed to persist GPU acceleration toggle to {}: {error}",
                        path.display()
                    ));
                }
            }
            Err(error) => {
                crate::native_capture::debug_log::log_error(format!(
                    "failed to serialize GPU acceleration toggle: {error}"
                ));
            }
        }
    }

    /// Whether the next job must be capped at CPU: the inverse of `use_gpu`. Read
    /// live at each helper spawn so the override is execution-time, not
    /// admission-frozen (ADR 0005).
    pub fn force_cpu(&self) -> bool {
        !self.use_gpu()
    }

    /// Whether an NVIDIA GPU exists, probed lazily and cached. Drives the Settings
    /// *offer* ONLY — the CUDA attempt is gated on pack presence + `force_cpu`, and
    /// the try/CPU-fallback subsumes detection (`select_execution_mode`).
    pub fn gpu_detected(&self) -> bool {
        *self.gpu_detected.get_or_init(detect_nvidia_gpu)
    }

    /// Record the Execution Backend outcome parsed from a finished job's
    /// provenance so the Settings panel can show the last mode + any CUDA-fallback
    /// reason. Lock-poison-safe (a poisoned lock simply drops the update).
    pub fn record_execution_outcome(&self, mode: Option<&str>, fallback_reason: Option<&str>) {
        if let Ok(mut guard) = self.last_execution_mode.write() {
            *guard = mode.map(str::to_string);
        }
        if let Ok(mut guard) = self.last_cuda_fallback_reason.write() {
            *guard = fallback_reason.map(str::to_string);
        }
    }

    /// The Execution Backend that ran the last completed job, if one has run.
    pub fn last_execution_mode(&self) -> Option<String> {
        self.last_execution_mode
            .read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// The last job's CUDA-init fallback reason, if it fell back.
    pub fn last_cuda_fallback_reason(&self) -> Option<String> {
        self.last_cuda_fallback_reason
            .read()
            .ok()
            .and_then(|guard| guard.clone())
    }
}

/// Serializable snapshot of the GPU-acceleration state for the Windows Settings
/// panel (Slice 5 / ADR 0005). camelCase to match the frontend
/// `GpuAccelerationState` TypeScript type.
///
/// A pure read-only projection of [`GpuAccelerationState`]: `gpu_detected` triggers
/// the (cached) NVML probe, `pack_installed` is a filesystem check, and the two
/// `last_*` fields echo the most recent job's recorded outcome. The panel mutates
/// the toggle through [`set_use_gpu_acceleration`], never this DTO.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuAccelerationStateDto {
    /// An NVIDIA GPU exists (NVML / `nvcuda` probe). Drives the Settings *offer*.
    pub gpu_detected: bool,
    /// The GPU Acceleration Pack is installed (its install marker exists).
    pub pack_installed: bool,
    /// The live "Use GPU acceleration" toggle (default ON).
    pub use_gpu: bool,
    /// The Execution Backend that ran the last completed job, if any
    /// (`"cpu"` | `"cuda"` | `"coreml"`).
    pub last_execution_mode: Option<String>,
    /// The last job's CUDA-init fallback reason — the single "why isn't my GPU
    /// used?" diagnostic — present ONLY when the last job fell back from CUDA.
    pub last_cuda_fallback_reason: Option<String>,
}

/// Read the current GPU-acceleration state for the Windows Settings panel.
///
/// Works on every platform so the frontend `invoke` type-checks uniformly; off
/// Windows the panel is never rendered (the UI gates on `detectKeyboardPlatform()`)
/// and `gpu_detected` is always `false` (the probe is a non-Windows no-op), so the
/// snapshot is inert there. No `cfg` is needed: the state methods already encode the
/// platform behavior.
#[tauri::command]
pub fn get_gpu_acceleration_state(
    state: tauri::State<'_, Arc<GpuAccelerationState>>,
) -> GpuAccelerationStateDto {
    GpuAccelerationStateDto {
        gpu_detected: state.gpu_detected(),
        pack_installed: state.pack_present(),
        use_gpu: state.use_gpu(),
        last_execution_mode: state.last_execution_mode(),
        last_cuda_fallback_reason: state.last_cuda_fallback_reason(),
    }
}

/// Set the live "Use GPU acceleration" toggle and persist it for restart durability.
///
/// The provider reads `use_gpu` LIVE at the next helper spawn (ADR 0005), so the
/// override takes effect on the next Speaker Analysis Job with no restart needed.
/// Persistence is best-effort inside `set_use_gpu` (a write failure is logged, never
/// fatal).
#[tauri::command]
pub fn set_use_gpu_acceleration(
    use_gpu: bool,
    state: tauri::State<'_, Arc<GpuAccelerationState>>,
) {
    state.set_use_gpu(use_gpu);
}

/// Probe for an NVIDIA GPU (Windows). Returns `false` on ANY failure.
///
/// Primary path: load `nvml.dll` (ships with the NVIDIA driver), resolve
/// `nvmlInit_v2` + `nvmlDeviceGetCount_v2` + `nvmlShutdown`, init, read the device
/// count, shut down, and report `count > 0`. Fallback: if NVML is unavailable
/// (e.g. `nvml.dll` is not on the search path on some driver packagings) but
/// `nvcuda.dll` — the CUDA driver shim that exists only with an NVIDIA driver —
/// loads, treat that as "an NVIDIA GPU/driver is present". A coarse fallback is
/// acceptable because detection only drives the Settings *offer*, never the CUDA
/// attempt.
#[cfg(windows)]
pub fn detect_nvidia_gpu() -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::FreeLibrary;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    // NVML entry-point ABIs. NVML is a C library; on x86_64 Windows there is a
    // single native calling convention, so transmuting the `extern "system"`
    // FARPROC that `GetProcAddress` returns into these `extern "C"` pointers is
    // sound (system and C are ABI-identical on x64). Each returns an
    // `nvmlReturn_t` (`0` == `NVML_SUCCESS`); `nvmlDeviceGetCount_v2` writes the
    // count through its out-pointer.
    type NvmlInitV2 = unsafe extern "C" fn() -> i32;
    type NvmlDeviceGetCountV2 = unsafe extern "C" fn(*mut u32) -> i32;
    type NvmlShutdown = unsafe extern "C" fn() -> i32;
    const NVML_SUCCESS: i32 = 0;

    // Load a DLL by bare name (wide, NUL-terminated). Non-null HMODULE on success.
    fn load_library(name: &str) -> *mut core::ffi::c_void {
        let wide: Vec<u16> = std::ffi::OsStr::new(name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: `wide` is a valid NUL-terminated UTF-16 string for the call's
        // duration; LoadLibraryW returns null on failure, which the caller checks.
        unsafe { LoadLibraryW(wide.as_ptr()) }
    }

    // Primary: authoritative device count via NVML.
    let nvml = load_library("nvml.dll");
    if !nvml.is_null() {
        // SAFETY: resolve the three NVML symbols; each is either a valid function
        // pointer (transmuted from the matching-size FARPROC) or `None` when the
        // symbol is absent, in which case we bail. All calls are NVML's own C ABI.
        let detected = unsafe {
            // PCSTR is `*const u8`; NUL-terminated byte literals match it directly.
            let init: Option<NvmlInitV2> = GetProcAddress(nvml, b"nvmlInit_v2\0".as_ptr())
                .map(|proc| std::mem::transmute::<_, NvmlInitV2>(proc));
            let get_count: Option<NvmlDeviceGetCountV2> =
                GetProcAddress(nvml, b"nvmlDeviceGetCount_v2\0".as_ptr())
                    .map(|proc| std::mem::transmute::<_, NvmlDeviceGetCountV2>(proc));
            let shutdown: Option<NvmlShutdown> = GetProcAddress(nvml, b"nvmlShutdown\0".as_ptr())
                .map(|proc| std::mem::transmute::<_, NvmlShutdown>(proc));

            match (init, get_count, shutdown) {
                (Some(init), Some(get_count), Some(shutdown)) => {
                    if init() != NVML_SUCCESS {
                        false
                    } else {
                        let mut count: u32 = 0;
                        let count_ok = get_count(&mut count) == NVML_SUCCESS;
                        // Always pair init with shutdown so NVML releases its
                        // global handle regardless of the count query result.
                        let _ = shutdown();
                        count_ok && count > 0
                    }
                }
                _ => false,
            }
        };
        // SAFETY: `nvml` was loaded above; release our reference.
        unsafe {
            let _ = FreeLibrary(nvml);
        }
        if detected {
            return true;
        }
    }

    // Fallback: nvcuda.dll presence == an NVIDIA driver is installed.
    let nvcuda = load_library("nvcuda.dll");
    if !nvcuda.is_null() {
        // SAFETY: loaded above; release our reference.
        unsafe {
            let _ = FreeLibrary(nvcuda);
        }
        return true;
    }

    false
}

/// Non-Windows: there is no NVIDIA GPU path to detect (macOS is CoreML; the CUDA
/// backend is Windows-only). Always `false` — the Settings offer never appears.
#[cfg(not(windows))]
pub fn detect_nvidia_gpu() -> bool {
    false
}

/// Augment THIS process's DLL search path so the CUDA provider DLL's transitive
/// CUDA 12 / cuDNN 9 dependencies resolve from the pack dir at load time
/// (Windows).
///
/// Called in the HELPER subprocess only, right before speakrs creates the CUDA
/// pipeline, and only when CUDA will actually be attempted (pack present + not
/// force-CPU). `onnxruntime_providers_cuda.dll` itself sits next to the exe (found
/// via `LOAD_LIBRARY_SEARCH_APPLICATION_DIR`); its deps — `cudart64_12.dll`,
/// `cublasLt64_12.dll`, `cudnn64_9.dll`, … — live FLAT in the pack dir, which this
/// adds to the search list. This is our realization of ADR 0005's "redist loaded
/// via ORT `preload_dylibs`": ORT loads the provider DLL, and these deps load off
/// the augmented search path.
///
/// No-op + safe when the pack dir doesn't exist yet (no-pack machine).
#[cfg(windows)]
pub fn prepare_cuda_dll_search(pack_dir: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::{
        AddDllDirectory, SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
    };

    // Nothing to add when the pack isn't installed: AddDllDirectory on a
    // nonexistent dir just fails, so skip the syscalls entirely. Keeps a no-pack
    // machine side-effect-free.
    if !pack_dir.is_dir() {
        return;
    }

    let wide: Vec<u16> = pack_dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: FFI into kernel32 with valid arguments. `SetDefaultDllDirectories`
    // opts this process into the AddDllDirectory user-directory search list —
    // without it AddDllDirectory has no effect on the default search.
    // `LOAD_LIBRARY_SEARCH_DEFAULT_DIRS` still covers the application dir + System32
    // + user dirs, so the provider DLL (next to the exe) and system DLLs keep
    // resolving. `wide` is a valid NUL-terminated UTF-16 path for the call's
    // duration. The returned DLL_DIRECTORY_COOKIE is intentionally dropped: the
    // pack dir stays on the search path for the (short-lived helper) process
    // lifetime and we never remove it. Idempotent — calling again is harmless. We
    // do this in the helper (not the parent) because only the helper loads speakrs
    // /ORT and thus the CUDA DLLs.
    unsafe {
        let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
        let _cookie = AddDllDirectory(wide.as_ptr());
    }
}

/// Non-Windows: macOS/Linux never load CUDA provider DLLs, so there is no search
/// path to augment.
#[cfg(not(windows))]
pub fn prepare_cuda_dll_search(_pack_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_gpu_on_and_no_outcome_recorded() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = GpuAccelerationState::new(temp.path().to_path_buf());
        // Default ON => not force_cpu.
        assert!(state.use_gpu());
        assert!(!state.force_cpu());
        // No pack marker => not present.
        assert!(!state.pack_present());
        assert_eq!(
            state.pack_dir(),
            speaker_analysis::gpu_acceleration_pack_dir(temp.path())
        );
        // No job has run yet.
        assert_eq!(state.last_execution_mode(), None);
        assert_eq!(state.last_cuda_fallback_reason(), None);
    }

    #[test]
    fn toggle_flips_force_cpu_live() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = GpuAccelerationState::new(temp.path().to_path_buf());
        state.set_use_gpu(false);
        assert!(!state.use_gpu());
        assert!(state.force_cpu());
        state.set_use_gpu(true);
        assert!(!state.force_cpu());
    }

    #[test]
    fn toggle_persists_and_rehydrates_across_restart() {
        let temp = tempfile::tempdir().expect("tempdir");
        // Fresh install: no sidecar → default ON.
        let first = GpuAccelerationState::new(temp.path().to_path_buf());
        assert!(first.use_gpu(), "default ON when sidecar absent");
        // Turn it OFF; that must write the sidecar.
        first.set_use_gpu(false);
        assert!(
            gpu_acceleration_settings_path(temp.path()).is_file(),
            "set_use_gpu persists the sidecar"
        );
        // A NEW state (simulating an app restart) rooted at the same dir must
        // rehydrate the OFF choice rather than reverting to the default.
        let restarted = GpuAccelerationState::new(temp.path().to_path_buf());
        assert!(!restarted.use_gpu(), "OFF choice survives restart");
        // Flip back ON and confirm the next restart rehydrates ON.
        restarted.set_use_gpu(true);
        let restarted_again = GpuAccelerationState::new(temp.path().to_path_buf());
        assert!(restarted_again.use_gpu(), "ON choice survives restart");
    }

    #[test]
    fn corrupt_sidecar_falls_back_to_default_on() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            gpu_acceleration_settings_path(temp.path()),
            b"this is not json",
        )
        .expect("write corrupt sidecar");
        // A corrupt file must never strand the user — default ON.
        let state = GpuAccelerationState::new(temp.path().to_path_buf());
        assert!(state.use_gpu(), "corrupt sidecar => default ON");
    }

    #[test]
    fn pack_present_tracks_install_marker() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = GpuAccelerationState::new(temp.path().to_path_buf());
        assert!(!state.pack_present(), "no pack dir/marker yet");
        let pack_dir = state.pack_dir();
        std::fs::create_dir_all(&pack_dir).expect("create pack dir");
        assert!(!state.pack_present(), "dir alone is not provisioned");
        std::fs::write(
            pack_dir.join(speaker_analysis::GPU_ACCELERATION_PACK_MARKER),
            "{}",
        )
        .expect("write marker");
        assert!(state.pack_present(), "install marker means provisioned");
    }

    #[test]
    fn record_execution_outcome_round_trips() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = GpuAccelerationState::new(temp.path().to_path_buf());
        state.record_execution_outcome(Some("cpu"), Some("cuda init failed"));
        assert_eq!(state.last_execution_mode().as_deref(), Some("cpu"));
        assert_eq!(
            state.last_cuda_fallback_reason().as_deref(),
            Some("cuda init failed")
        );
        // A subsequent plain run clears the fallback reason.
        state.record_execution_outcome(Some("cuda"), None);
        assert_eq!(state.last_execution_mode().as_deref(), Some("cuda"));
        assert_eq!(state.last_cuda_fallback_reason(), None);
    }

    #[test]
    fn prepare_cuda_dll_search_is_safe_when_dir_missing() {
        // No-pack machine: pointing at a nonexistent dir must be a harmless no-op
        // (and on non-Windows the whole thing is a no-op shim).
        let temp = tempfile::tempdir().expect("tempdir");
        prepare_cuda_dll_search(&temp.path().join("does-not-exist"));
    }

    #[test]
    fn detect_nvidia_gpu_is_callable_and_total() {
        // On CI/dev without an NVIDIA driver this is `false`; on the RTX dev box it
        // is `true`. Either way the probe must not panic and must return.
        let _ = detect_nvidia_gpu();
    }
}
