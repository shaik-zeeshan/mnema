//! In-app, opt-in NVIDIA GPU Acceleration Pack downloader (#137, Slice 4 / ADR
//! 0005).
//!
//! ## What this is — and what it is NOT
//!
//! The Windows CUDA Execution Backend needs the NVIDIA CUDA 12 + cuDNN 9
//! *redistributable* DLLs that `onnxruntime_providers_cuda.dll` links against.
//! Those are EULA-encumbered and GB-scale, so they are **never bundled** in our
//! installer and **never hosted by us**. Instead this module orchestrates an
//! opt-in, on-demand fetch **directly from NVIDIA's official redist endpoints**
//! into the app data dir's GPU pack dir (`gpu_acceleration_pack_dir`). We are an
//! *orchestrator*, not a redistributor: NVIDIA remains the distributor, the bytes
//! come from `developer.download.nvidia.com`, and the user accepts NVIDIA's
//! license in-app (the `accepted_license` flag on `start_*`) before a single byte
//! is fetched. See ADR 0005 ("Distribution is in-app, on-demand, opt-in
//! provisioning — not bundling").
//!
//! ## How it activates CUDA
//!
//! On success the needed DLLs land **flat** in the pack dir and a
//! `.installed.json` marker ([`speaker_analysis::GPU_ACCELERATION_PACK_MARKER`])
//! is written. That marker is the ONLY gate the helper checks
//! ([`speaker_analysis::gpu_pack_present`]): once it exists, the next Speaker
//! Analysis Job *attempts* CUDA, loading the pack DLLs off the search path that
//! `gpu_acceleration::prepare_cuda_dll_search` sets up (Slice 3). On any failure
//! the marker is absent, so CPU keeps working with no fallback noise — "not
//! provisioned" is not a failure.
//!
//! ## Version coupling (pins live next to the `ort` pin)
//!
//! The provider DLL is version-locked to `ort = =2.0.0-rc.12` (ONNX Runtime
//! **1.24**), which is built against **CUDA 12.x + cuDNN 9.x** (cuDNN 8.x is
//! incompatible; CUDA 12.x are mutually compatible across minors, and the DLL
//! SONAMEs are major-versioned — `cudart64_12.dll`, `cudnn64_9.dll`, …). The exact
//! redist versions are pinned in [`GPU_PACK_ARCHIVES`] and bumped deliberately
//! whenever the `ort` pin moves. The precise DLL subset is finalized on-device
//! (plan Slice 6) — structure correctness + real NVIDIA URLs matter most here.
//!
//! ## Shape
//!
//! This mirrors `speaker_analysis_models.rs` deliberately (managed download state,
//! `reqwest` streaming with per-chunk progress over a Tauri event, sha256 verify,
//! `.download.tmp` staging, cancellation via `AtomicBool`, an `.installed.json`
//! marker) so the two downloaders read the same and a reviewer who knows one knows
//! both. The one structural difference: NVIDIA's archives are large (the cuDNN and
//! cuBLAS zips are hundreds of MB each), so each archive is streamed to the
//! staging file on disk rather than buffered into a `Vec<u8>` — bounded memory for
//! a GB-scale fetch — and the wanted DLLs are then extracted out of that zip.

use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use speaker_analysis::{
    gpu_acceleration_pack_dir, validate_artifact_sha256, GPU_ACCELERATION_PACK_MARKER,
};
use tauri::{Emitter, Manager};

/// Tauri event carrying per-chunk download progress for the GPU Acceleration Pack
/// (mirrors `SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT`). Slice 5's Settings
/// panel listens on this to drive the progress bar.
pub const GPU_ACCELERATION_PACK_DOWNLOAD_PROGRESS_EVENT: &str =
    "gpu_acceleration_pack_download_progress";

/// Manifest schema version stamped into the install marker, so a future pin bump
/// that changes the DLL set can detect + re-provision a stale pack.
const GPU_PACK_MANIFEST_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Pinned NVIDIA redist manifest (coupled to ort = =2.0.0-rc.12 / ONNX Runtime 1.24)
// ---------------------------------------------------------------------------
//
// Every value below is read straight from NVIDIA's machine-readable redist
// manifests so the URLs/sizes/checksums are authentic and verifiable:
//   CUDA : https://developer.download.nvidia.com/compute/cuda/redist/redistrib_12.9.1.json
//   cuDNN: https://developer.download.nvidia.com/compute/cudnn/redist/redistrib_9.10.2.json
// The DLL subset is the transitive dependency closure of
// `onnxruntime_providers_cuda.dll` on ORT 1.24: cudart + cuBLAS(+Lt) + cuFFT from
// CUDA, and the cuDNN 9 graph/engines/ops/cnn/adv set. cuFFT keeps the SONAME
// `cufft64_11.dll` even inside CUDA 12 — that is intentional, not a typo.

/// The pinned CUDA redist release these archives are drawn from (`release_label`
/// in `redistrib_12.9.1.json`). Surfaced in the status DTO + install marker.
const PINNED_CUDA_VERSION: &str = "12.9.1";
/// The pinned cuDNN redist release (`release_label` in `redistrib_9.10.2.json`,
/// `cuda12` variant). cuDNN 9.x ↔ CUDA 12.x is mandatory (ADR 0005).
const PINNED_CUDNN_VERSION: &str = "9.10.2";
/// The `ort`/ONNX Runtime pin this pack is coupled to. Recorded in the marker as
/// provenance so a future reader can see WHY these exact NVIDIA versions were
/// chosen (and a mismatch with the running binary's `ort` pin is a red flag).
const PINNED_ORT_VERSION: &str = "2.0.0-rc.12 (ONNX Runtime 1.24)";

/// NVIDIA CUDA EULA, surfaced to the UI so the user can read the terms they are
/// accepting before the download (`accepted_license`).
const NVIDIA_CUDA_LICENSE_URL: &str = "https://docs.nvidia.com/cuda/eula/index.html";
/// NVIDIA cuDNN Software License Agreement, surfaced alongside the CUDA EULA.
const NVIDIA_CUDNN_LICENSE_URL: &str =
    "https://docs.nvidia.com/deeplearning/cudnn/sla/index.html";

/// One NVIDIA redist archive plus the exact DLLs we lift out of it. `byte_size` is
/// the compressed `.zip` size NVIDIA advertises (used for the size cap + the
/// progress total); `dll_names` are extracted FLAT into the pack dir regardless of
/// their internal `bin/` (or `bin/<cuda_ver>/`) nesting.
#[derive(Debug, Clone, Copy)]
struct GpuPackArchive {
    /// NVIDIA redist component id (e.g. `cuda_cudart`); used only for messages.
    component: &'static str,
    /// Full NVIDIA redist URL of the `.zip`.
    url: &'static str,
    /// Hex SHA256 of the `.zip`, from NVIDIA's redist manifest. Verified after
    /// download; a mismatch refuses the install.
    sha256: &'static str,
    /// Compressed archive size in bytes, from NVIDIA's redist manifest.
    byte_size: u64,
    /// The DLL file names to extract from this archive's `bin/` into the flat pack
    /// dir. Matched by basename (case-insensitive) so the internal nesting is
    /// irrelevant.
    dll_names: &'static [&'static str],
}

/// The pinned set of NVIDIA redist archives. `// TODO(operator)` markers flag the
/// few fields that can only be finalized against the on-device pull (plan Slice
/// 6): the sha256/size are copied from NVIDIA's manifests above, but the precise
/// DLL subset ORT 1.24 demands is confirmed on the RTX dev box.
const GPU_PACK_ARCHIVES: &[GpuPackArchive] = &[
    // CUDA runtime — `cudart64_12.dll`.
    GpuPackArchive {
        component: "cuda_cudart",
        url: "https://developer.download.nvidia.com/compute/cuda/redist/cuda_cudart/windows-x86_64/cuda_cudart-windows-x86_64-12.9.79-archive.zip",
        sha256: "179e9c43b0735ffe67207b3da556eb5a0c50f3047961882b7657d3b822d34ef8",
        byte_size: 3_521_238,
        dll_names: &["cudart64_12.dll"],
    },
    // cuBLAS — `cublas64_12.dll` + `cublasLt64_12.dll` ship in one archive.
    GpuPackArchive {
        component: "libcublas",
        url: "https://developer.download.nvidia.com/compute/cuda/redist/libcublas/windows-x86_64/libcublas-windows-x86_64-12.9.1.4-archive.zip",
        sha256: "d534d98b0b453a98914dbf3adf47d7e84b55037abf02f87466439e1dcef581ed",
        byte_size: 549_755_186,
        dll_names: &["cublas64_12.dll", "cublasLt64_12.dll"],
    },
    // cuFFT — `cufft64_11.dll` (SONAME 11 even under CUDA 12).
    GpuPackArchive {
        component: "libcufft",
        url: "https://developer.download.nvidia.com/compute/cuda/redist/libcufft/windows-x86_64/libcufft-windows-x86_64-11.4.1.4-archive.zip",
        sha256: "f26f80bb9abff3269c548e1559e8c2b4ba58ccb8acc6095bbc6404fc962d4b80",
        byte_size: 198_361_265,
        dll_names: &["cufft64_11.dll"],
    },
    // cuDNN 9 (cuda12 variant) — the loader DLL plus its sub-libraries. ORT loads
    // `cudnn64_9.dll`, which in turn dlopens the graph/engines/ops/cnn/adv set, so
    // all must be present in the pack dir.
    // TODO(operator): confirm on-device — the exact cuDNN sub-DLL set ORT 1.24
    // requires is finalized on the RTX 5070 Ti box (plan Slice 6). Prune any
    // sub-DLL that turns out unused, or add one ORT 1.24 newly demands.
    GpuPackArchive {
        component: "cudnn",
        url: "https://developer.download.nvidia.com/compute/cudnn/redist/cudnn/windows-x86_64/cudnn-windows-x86_64-9.10.2.21_cuda12-archive.zip",
        sha256: "c1a4567d822ebda7373fa1f19255dff4942302de741f830160b6c7d1fb31af23",
        byte_size: 683_336_095,
        dll_names: &[
            "cudnn64_9.dll",
            "cudnn_graph64_9.dll",
            "cudnn_engines_precompiled64_9.dll",
            "cudnn_engines_runtime_compiled64_9.dll",
            "cudnn_heuristic64_9.dll",
            "cudnn_ops64_9.dll",
            "cudnn_adv64_9.dll",
            "cudnn_cnn64_9.dll",
        ],
    },
];

/// The summed compressed size of every archive — the progress bar's total and the
/// `totalBytes` the status DTO advertises before a download starts.
fn gpu_pack_total_bytes() -> u64 {
    GPU_PACK_ARCHIVES.iter().map(|archive| archive.byte_size).sum()
}

/// Every DLL the completed pack must contain, flattened across archives. The
/// post-extract layout check and the install marker both derive from this so they
/// cannot drift from [`GPU_PACK_ARCHIVES`].
fn gpu_pack_expected_dlls() -> Vec<String> {
    GPU_PACK_ARCHIVES
        .iter()
        .flat_map(|archive| archive.dll_names.iter().map(|name| (*name).to_string()))
        .collect()
}

// ---------------------------------------------------------------------------
// Managed download state (mirrors SpeakerAnalysisModelDownloadState)
// ---------------------------------------------------------------------------

pub type GpuAccelerationPackDownloadState = Mutex<Option<ActiveGpuAccelerationPackDownload>>;

/// The single in-flight pack download, if any. Only one runs at a time (the pack
/// is one shared unit), so this is an `Option`, not a map.
#[derive(Debug, Clone)]
pub struct ActiveGpuAccelerationPackDownload {
    cancel_requested: Arc<AtomicBool>,
}

// ---------------------------------------------------------------------------
// DTOs (camelCase over serde, matching the existing model-download DTO style)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GpuAccelerationPackDownloadStatusDto {
    Starting,
    Downloading,
    Installing,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GpuAccelerationPackDownloadProgressDto {
    pub status: GpuAccelerationPackDownloadStatusDto,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    /// The NVIDIA component currently being fetched/installed (e.g. `cudnn`), or
    /// `None` for phase transitions that are not archive-specific.
    pub component: Option<String>,
    pub message: Option<String>,
}

impl GpuAccelerationPackDownloadProgressDto {
    fn new(
        status: GpuAccelerationPackDownloadStatusDto,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        component: Option<String>,
        message: Option<String>,
    ) -> Self {
        Self {
            status,
            downloaded_bytes,
            total_bytes,
            component,
            message,
        }
    }
}

/// The versions recorded in a completed pack's `.installed.json` marker, surfaced
/// back to the UI so it can show exactly what is provisioned.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GpuAccelerationPackInstalledVersionsDto {
    pub cuda_version: String,
    pub cudnn_version: String,
    pub ort_version: String,
    pub installed_dlls: Vec<String>,
}

/// The NVIDIA license URLs the UI links so the user can read the terms before
/// accepting them (the `accepted_license` gate).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GpuAccelerationPackLicenseUrlsDto {
    pub cuda: String,
    pub cudnn: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GpuAccelerationPackStatusDto {
    /// Whether the pack is fully installed (its `.installed.json` marker exists).
    /// The single gate (with the Force-CPU toggle) on whether CUDA is attempted.
    pub pack_installed: bool,
    /// The provisioned versions + DLL list, read from the marker when installed.
    pub installed_versions: Option<GpuAccelerationPackInstalledVersionsDto>,
    /// The CUDA redist version this build will fetch (pinned to the `ort` pin).
    pub required_cuda_version: String,
    /// The cuDNN redist version this build will fetch.
    pub required_cudnn_version: String,
    /// Total compressed download size across all archives (bytes).
    pub total_bytes: u64,
    /// NVIDIA license URLs for the in-app consent surface.
    pub license_urls: GpuAccelerationPackLicenseUrlsDto,
    /// The current in-flight download phase, or `None` when idle. Lets the panel
    /// show an active download immediately on load (before the next progress
    /// event arrives), mirroring how the model panels re-sync.
    pub download_state: Option<GpuAccelerationPackDownloadStatusDto>,
    /// Absolute path of the pack dir (for a "reveal in explorer" affordance / the
    /// reclaim-space copy).
    pub pack_directory: String,
}

// ---------------------------------------------------------------------------
// Install marker (records the pinned versions + the DLLs actually installed)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GpuAccelerationPackMarker {
    manifest_version: u32,
    cuda_version: String,
    cudnn_version: String,
    ort_version: String,
    installed_dlls: Vec<String>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
enum GpuPackDownloadError {
    #[error("a GPU acceleration pack download is already running")]
    AlreadyRunning,
    #[error("no active GPU acceleration pack download")]
    NoActiveDownload,
    #[error("the NVIDIA CUDA + cuDNN license must be accepted before downloading the GPU acceleration pack")]
    LicenseNotAccepted,
    #[error("the GPU acceleration pack is only supported on Windows x86_64")]
    UnsupportedPlatform,
    #[error("failed to resolve app data directory: {0}")]
    AppDataDir(String),
    #[error("download failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write staged archive {path}: {source}")]
    WriteStaged {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to remove {path}: {source}")]
    Remove {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "download for the {component} archive exceeded its expected size of {byte_size} bytes (limit {limit} bytes); aborting"
    )]
    OversizedDownload {
        component: String,
        byte_size: u64,
        limit: u64,
    },
    #[error("integrity check failed for the {component} archive: {source}")]
    Checksum {
        component: String,
        #[source]
        source: speaker_analysis::ModelInstallError,
    },
    #[error("failed to open downloaded archive for {component}: {source}")]
    OpenArchive {
        component: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read the {component} zip archive: {source}")]
    Zip {
        component: String,
        #[source]
        source: zip::result::ZipError,
    },
    #[error("the {component} archive did not contain expected DLL {dll}")]
    MissingDllInArchive { component: String, dll: String },
    #[error("the installed GPU acceleration pack is missing DLLs: {missing:?}")]
    IncompleteInstall { missing: Vec<String> },
    #[error("failed to serialize the GPU acceleration pack marker: {0}")]
    SerializeMarker(#[source] serde_json::Error),
    #[error("failed to write the GPU acceleration pack marker {path}: {source}")]
    WriteMarker {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, thiserror::Error)]
enum GpuPackDownloadTaskError {
    #[error("download cancelled")]
    Cancelled,
    #[error(transparent)]
    Failed(#[from] GpuPackDownloadError),
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_gpu_acceleration_pack_status(
    app_handle: tauri::AppHandle,
    download_state: tauri::State<'_, GpuAccelerationPackDownloadState>,
) -> Result<GpuAccelerationPackStatusDto, String> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    let pack_dir = gpu_acceleration_pack_dir(&app_data_dir);
    let download_active = download_state
        .lock()
        .map(|guard| guard.is_some())
        .unwrap_or(false);
    Ok(build_status(&pack_dir, download_active))
}

#[tauri::command]
pub fn start_gpu_acceleration_pack_download(
    app_handle: tauri::AppHandle,
    accepted_license: bool,
    download_state: tauri::State<'_, GpuAccelerationPackDownloadState>,
) -> Result<GpuAccelerationPackDownloadProgressDto, String> {
    // The CUDA backend is Windows x86_64 only (ADR 0005). The command exists on
    // every platform so the frontend type-checks uniformly, but off Windows it
    // refuses up front — macOS is always CoreML, no pack. Using a runtime
    // `cfg!` (not a `#[cfg]` attribute) keeps the download code below compiled +
    // reachable on macOS so it never bit-rots.
    if !cfg!(target_os = "windows") {
        return Err(GpuPackDownloadError::UnsupportedPlatform.to_string());
    }
    // NVIDIA's redist is fetched under terms the user accepts in-app; refuse
    // before touching the network if consent was not given.
    ensure_license_accepted(accepted_license).map_err(|error| error.to_string())?;

    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| GpuPackDownloadError::AppDataDir(error.to_string()).to_string())?;
    let pack_dir = gpu_acceleration_pack_dir(&app_data_dir);

    let cancel_requested = Arc::new(AtomicBool::new(false));
    claim_download(download_state.inner(), Arc::clone(&cancel_requested))
        .map_err(|error| error.to_string())?;

    let starting = GpuAccelerationPackDownloadProgressDto::new(
        GpuAccelerationPackDownloadStatusDto::Starting,
        0,
        Some(gpu_pack_total_bytes()),
        None,
        None,
    );
    emit_progress(&app_handle, &starting);

    let app_for_task = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        run_download_task(app_for_task, pack_dir, cancel_requested).await;
    });

    Ok(starting)
}

#[tauri::command]
pub fn cancel_gpu_acceleration_pack_download(
    download_state: tauri::State<'_, GpuAccelerationPackDownloadState>,
) -> Result<(), String> {
    let active = download_state
        .lock()
        .map_err(|_| "GPU acceleration pack download state poisoned".to_string())?;
    let Some(active) = active.as_ref() else {
        return Err(GpuPackDownloadError::NoActiveDownload.to_string());
    };
    active.cancel_requested.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub async fn delete_gpu_acceleration_pack(
    app_handle: tauri::AppHandle,
    download_state: tauri::State<'_, GpuAccelerationPackDownloadState>,
    infra: tauri::State<'_, crate::app_infra::AppInfraState>,
) -> Result<(), String> {
    // Guard 1: never delete out from under an in-flight download.
    let download_active = download_state
        .lock()
        .map_err(|_| "GPU acceleration pack download state poisoned".to_string())?
        .is_some();
    if download_active {
        return Err("cannot delete the GPU acceleration pack while it is downloading".to_string());
    }
    // Guard 2: never delete the pack while ANY speaker-analysis job is queued or
    // running — a job may be mid-CUDA-init and loading these DLLs. This mirrors
    // `delete_speaker_analysis_model`'s "jobs queued or running" guard, but keyed
    // on *any* speaker job rather than a specific model: the pack is shared
    // hardware (orthogonal to identity), so there is no per-model key to scope to.
    let active_jobs = infra
        .list_active_speaker_analysis_model_keys()
        .await
        .map_err(|error| {
            format!(
                "failed to inspect queued or running speaker analysis jobs before deleting the GPU acceleration pack: {error}"
            )
        })?;
    if !active_jobs.is_empty() {
        return Err(
            "cannot delete the GPU acceleration pack while speaker analysis jobs are queued or running"
                .to_string(),
        );
    }

    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    let pack_dir = gpu_acceleration_pack_dir(&app_data_dir);
    remove_pack_dir(&pack_dir).map_err(|error| error.to_string())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Pure license-consent gate, factored out so it is unit-testable without a Tauri
/// `AppHandle`. The `start_*` command refuses before any network access unless the
/// user has accepted NVIDIA's CUDA + cuDNN terms in-app.
fn ensure_license_accepted(accepted_license: bool) -> Result<(), GpuPackDownloadError> {
    if accepted_license {
        Ok(())
    } else {
        Err(GpuPackDownloadError::LicenseNotAccepted)
    }
}

fn build_status(pack_dir: &Path, download_active: bool) -> GpuAccelerationPackStatusDto {
    let installed_versions = read_pack_marker(pack_dir).map(|marker| {
        GpuAccelerationPackInstalledVersionsDto {
            cuda_version: marker.cuda_version,
            cudnn_version: marker.cudnn_version,
            ort_version: marker.ort_version,
            installed_dlls: marker.installed_dlls,
        }
    });
    GpuAccelerationPackStatusDto {
        pack_installed: speaker_analysis::gpu_pack_present(pack_dir),
        installed_versions,
        required_cuda_version: PINNED_CUDA_VERSION.to_string(),
        required_cudnn_version: PINNED_CUDNN_VERSION.to_string(),
        total_bytes: gpu_pack_total_bytes(),
        license_urls: GpuAccelerationPackLicenseUrlsDto {
            cuda: NVIDIA_CUDA_LICENSE_URL.to_string(),
            cudnn: NVIDIA_CUDNN_LICENSE_URL.to_string(),
        },
        download_state: download_active
            .then_some(GpuAccelerationPackDownloadStatusDto::Downloading),
        pack_directory: pack_dir.to_string_lossy().to_string(),
    }
}

fn claim_download(
    state: &GpuAccelerationPackDownloadState,
    cancel_requested: Arc<AtomicBool>,
) -> Result<(), GpuPackDownloadError> {
    let mut active = state
        .lock()
        .map_err(|_| GpuPackDownloadError::AlreadyRunning)?;
    if active.is_some() {
        return Err(GpuPackDownloadError::AlreadyRunning);
    }
    *active = Some(ActiveGpuAccelerationPackDownload { cancel_requested });
    Ok(())
}

fn clear_active_download(app_handle: &tauri::AppHandle) {
    if let Ok(mut active) = app_handle
        .state::<GpuAccelerationPackDownloadState>()
        .lock()
    {
        *active = None;
    }
}

fn emit_progress(app_handle: &tauri::AppHandle, progress: &GpuAccelerationPackDownloadProgressDto) {
    let _ = app_handle.emit(GPU_ACCELERATION_PACK_DOWNLOAD_PROGRESS_EVENT, progress);
}

async fn run_download_task(
    app_handle: tauri::AppHandle,
    pack_dir: PathBuf,
    cancel_requested: Arc<AtomicBool>,
) {
    let total = gpu_pack_total_bytes();
    let result = download_and_install_pack(&app_handle, &pack_dir, &cancel_requested).await;
    clear_active_download(&app_handle);
    match result {
        Ok(()) => emit_progress(
            &app_handle,
            &GpuAccelerationPackDownloadProgressDto::new(
                GpuAccelerationPackDownloadStatusDto::Completed,
                total,
                Some(total),
                None,
                None,
            ),
        ),
        Err(GpuPackDownloadTaskError::Cancelled) => {
            // A cancelled install left a partial pack (no marker ⇒ not present ⇒
            // CPU). Remove it so the next attempt starts clean and no half-fetched
            // GB sit on disk. Best-effort: a cleanup failure must not mask cancel.
            let _ = remove_pack_dir(&pack_dir);
            emit_progress(
                &app_handle,
                &GpuAccelerationPackDownloadProgressDto::new(
                    GpuAccelerationPackDownloadStatusDto::Cancelled,
                    0,
                    Some(total),
                    None,
                    Some("download cancelled".to_string()),
                ),
            );
        }
        Err(GpuPackDownloadTaskError::Failed(error)) => {
            // Same cleanup on failure: leave the machine on plain CPU with no
            // partial pack. CPU keeps working — a failed pack fetch is not a
            // user-facing job error (ADR 0005).
            let _ = remove_pack_dir(&pack_dir);
            emit_progress(
                &app_handle,
                &GpuAccelerationPackDownloadProgressDto::new(
                    GpuAccelerationPackDownloadStatusDto::Failed,
                    0,
                    Some(total),
                    None,
                    Some(error.to_string()),
                ),
            );
        }
    }
}

async fn download_and_install_pack(
    app_handle: &tauri::AppHandle,
    pack_dir: &Path,
    cancel_requested: &AtomicBool,
) -> Result<(), GpuPackDownloadTaskError> {
    // Start from a clean dir: a prior partial/old pack is removed so a re-provision
    // never mixes DLLs across pins.
    remove_pack_dir(pack_dir)?;
    std::fs::create_dir_all(pack_dir).map_err(|source| GpuPackDownloadError::CreateDir {
        path: pack_dir.to_path_buf(),
        source,
    })?;

    let total = gpu_pack_total_bytes();
    let staging_path = pack_dir.join(".download.tmp");
    let mut downloaded_total = 0_u64;
    let mut installed_dlls: Vec<String> = Vec::new();

    for archive in GPU_PACK_ARCHIVES {
        if cancel_requested.load(Ordering::SeqCst) {
            return Err(GpuPackDownloadTaskError::Cancelled);
        }
        // Stream the (large) archive to the staging file rather than buffering it
        // in memory, enforcing the size cap as we go.
        stream_archive_to_staging(
            app_handle,
            archive,
            &staging_path,
            downloaded_total,
            total,
            cancel_requested,
        )
        .await?;
        downloaded_total = downloaded_total.saturating_add(archive.byte_size);

        // Verify integrity against NVIDIA's published SHA256 before extracting a
        // single byte. Reuses the shared verifier; a missing/blank pin would make
        // it a no-op, so the manifest values are non-empty consts by construction.
        validate_artifact_sha256(&staging_path, Some(archive.sha256)).map_err(|source| {
            GpuPackDownloadError::Checksum {
                component: archive.component.to_string(),
                source,
            }
        })?;

        emit_progress(
            app_handle,
            &GpuAccelerationPackDownloadProgressDto::new(
                GpuAccelerationPackDownloadStatusDto::Installing,
                downloaded_total,
                Some(total),
                Some(archive.component.to_string()),
                Some(format!("extracting {}", archive.component)),
            ),
        );
        let mut extracted = extract_archive_dlls(archive, &staging_path, pack_dir)?;
        installed_dlls.append(&mut extracted);

        // Drop the staging file before the next archive reuses the path.
        remove_file_if_exists(&staging_path)?;
    }

    // Belt-and-braces: every expected DLL must now sit flat in the pack dir.
    let missing = gpu_pack_expected_dlls()
        .into_iter()
        .filter(|dll| !pack_dir.join(dll).is_file())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(GpuPackDownloadError::IncompleteInstall { missing }.into());
    }

    write_pack_marker(pack_dir, installed_dlls)?;
    Ok(())
}

/// Stream one archive to `staging_path`, emitting per-chunk progress and enforcing
/// a `byte_size + slack` cap so a misbehaving/compromised host cannot stream an
/// unbounded body to disk before the SHA256 check runs. Mirrors the model
/// downloader's cap logic, but writes to disk (bounded memory) for GB-scale zips.
async fn stream_archive_to_staging(
    app_handle: &tauri::AppHandle,
    archive: &GpuPackArchive,
    staging_path: &Path,
    already_downloaded_bytes: u64,
    total_bytes: u64,
    cancel_requested: &AtomicBool,
) -> Result<(), GpuPackDownloadTaskError> {
    const DOWNLOAD_SIZE_SLACK_BYTES: u64 = 64 * 1024;
    let size_limit = archive.byte_size.saturating_add(DOWNLOAD_SIZE_SLACK_BYTES);

    let response = reqwest::get(archive.url)
        .await
        .map_err(GpuPackDownloadError::Http)?
        .error_for_status()
        .map_err(GpuPackDownloadError::Http)?;
    if let Some(content_length) = response.content_length() {
        if content_length > size_limit {
            return Err(GpuPackDownloadError::OversizedDownload {
                component: archive.component.to_string(),
                byte_size: archive.byte_size,
                limit: size_limit,
            }
            .into());
        }
    }

    let mut file =
        std::fs::File::create(staging_path).map_err(|source| GpuPackDownloadError::WriteStaged {
            path: staging_path.to_path_buf(),
            source,
        })?;
    let mut written: u64 = 0;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancel_requested.load(Ordering::SeqCst) {
            return Err(GpuPackDownloadTaskError::Cancelled);
        }
        let chunk = chunk.map_err(GpuPackDownloadError::Http)?;
        written = written.saturating_add(chunk.len() as u64);
        if written > size_limit {
            return Err(GpuPackDownloadError::OversizedDownload {
                component: archive.component.to_string(),
                byte_size: archive.byte_size,
                limit: size_limit,
            }
            .into());
        }
        file.write_all(&chunk)
            .map_err(|source| GpuPackDownloadError::WriteStaged {
                path: staging_path.to_path_buf(),
                source,
            })?;
        emit_progress(
            app_handle,
            &GpuAccelerationPackDownloadProgressDto::new(
                GpuAccelerationPackDownloadStatusDto::Downloading,
                already_downloaded_bytes.saturating_add(written),
                if total_bytes == 0 { None } else { Some(total_bytes) },
                Some(archive.component.to_string()),
                None,
            ),
        );
    }
    file.flush()
        .map_err(|source| GpuPackDownloadError::WriteStaged {
            path: staging_path.to_path_buf(),
            source,
        })?;
    Ok(())
}

/// Extract this archive's wanted DLLs FLAT into the pack dir, matched by basename
/// (case-insensitive) so the internal `bin/` (or `bin/<cuda_ver>/`) nesting is
/// irrelevant. `enclosed_name` guards against zip path-traversal; we only ever
/// write a bare file name into the pack dir. Returns the installed DLL names.
fn extract_archive_dlls(
    archive: &GpuPackArchive,
    zip_path: &Path,
    pack_dir: &Path,
) -> Result<Vec<String>, GpuPackDownloadError> {
    let file = std::fs::File::open(zip_path).map_err(|source| GpuPackDownloadError::OpenArchive {
        component: archive.component.to_string(),
        source,
    })?;
    let mut zip = zip::ZipArchive::new(file).map_err(|source| GpuPackDownloadError::Zip {
        component: archive.component.to_string(),
        source,
    })?;

    let wanted: BTreeSet<String> = archive
        .dll_names
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    let mut installed: Vec<String> = Vec::new();

    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|source| GpuPackDownloadError::Zip {
            component: archive.component.to_string(),
            source,
        })?;
        if !entry.is_file() {
            continue;
        }
        let Some(enclosed) = entry.enclosed_name() else {
            // Skip any entry whose path escapes the archive root (traversal guard).
            continue;
        };
        let Some(file_name) = enclosed
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string())
        else {
            continue;
        };
        if !wanted.contains(&file_name.to_ascii_lowercase()) {
            continue;
        }
        // `file_name` is a bare basename (`Path::file_name`), so joining it onto
        // the pack dir can only write directly inside it — no traversal.
        let destination = pack_dir.join(&file_name);
        let mut out = std::fs::File::create(&destination).map_err(|source| {
            GpuPackDownloadError::WriteStaged {
                path: destination.clone(),
                source,
            }
        })?;
        std::io::copy(&mut entry, &mut out).map_err(|source| {
            GpuPackDownloadError::WriteStaged {
                path: destination.clone(),
                source,
            }
        })?;
        if !installed.iter().any(|name| name.eq_ignore_ascii_case(&file_name)) {
            installed.push(file_name);
        }
    }

    // Every DLL named for this archive must have been found; a missing one means
    // the pin is wrong (or NVIDIA changed the layout) and the pack would be broken.
    for dll in archive.dll_names {
        if !installed.iter().any(|name| name.eq_ignore_ascii_case(dll)) {
            return Err(GpuPackDownloadError::MissingDllInArchive {
                component: archive.component.to_string(),
                dll: (*dll).to_string(),
            });
        }
    }
    Ok(installed)
}

fn write_pack_marker(
    pack_dir: &Path,
    installed_dlls: Vec<String>,
) -> Result<(), GpuPackDownloadError> {
    let marker = GpuAccelerationPackMarker {
        manifest_version: GPU_PACK_MANIFEST_VERSION,
        cuda_version: PINNED_CUDA_VERSION.to_string(),
        cudnn_version: PINNED_CUDNN_VERSION.to_string(),
        ort_version: PINNED_ORT_VERSION.to_string(),
        installed_dlls,
    };
    let path = pack_dir.join(GPU_ACCELERATION_PACK_MARKER);
    let json =
        serde_json::to_vec_pretty(&marker).map_err(GpuPackDownloadError::SerializeMarker)?;
    std::fs::write(&path, json).map_err(|source| GpuPackDownloadError::WriteMarker { path, source })
}

fn read_pack_marker(pack_dir: &Path) -> Option<GpuAccelerationPackMarker> {
    let path = pack_dir.join(GPU_ACCELERATION_PACK_MARKER);
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn remove_pack_dir(pack_dir: &Path) -> Result<(), GpuPackDownloadError> {
    if pack_dir.exists() {
        std::fs::remove_dir_all(pack_dir).map_err(|source| GpuPackDownloadError::Remove {
            path: pack_dir.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> Result<(), GpuPackDownloadError> {
    if path.exists() {
        std::fs::remove_file(path).map_err(|source| GpuPackDownloadError::Remove {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // `super::*` brings the module's top-level `std::io::Write` into scope, so the
    // tests' `writer.write_all(..)` resolves without a redundant local import.
    use super::*;

    /// The manifest's derived totals + DLL list stay coherent with the archive
    /// table, and every archive carries a real NVIDIA redist URL + non-empty
    /// SHA256. This is the cheap guard that a careless pin edit can't pass.
    #[test]
    fn manifest_is_internally_consistent() {
        assert_eq!(gpu_pack_total_bytes(), 3_521_238 + 549_755_186 + 198_361_265 + 683_336_095);
        let expected = gpu_pack_expected_dlls();
        // cudart(1) + cublas(2) + cufft(1) + cudnn(8) = 12 DLLs.
        assert_eq!(expected.len(), 12);
        assert!(expected.iter().any(|d| d == "cudart64_12.dll"));
        assert!(expected.iter().any(|d| d == "cublasLt64_12.dll"));
        assert!(expected.iter().any(|d| d == "cufft64_11.dll"));
        assert!(expected.iter().any(|d| d == "cudnn64_9.dll"));
        for archive in GPU_PACK_ARCHIVES {
            assert!(
                archive.url.starts_with("https://developer.download.nvidia.com/compute/"),
                "archive {} must fetch from NVIDIA's redist endpoint, got {:?}",
                archive.component,
                archive.url
            );
            assert!(archive.url.ends_with(".zip"));
            assert_eq!(archive.sha256.len(), 64, "sha256 must be 32-byte hex");
            assert!(archive.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(!archive.dll_names.is_empty());
            assert!(archive.byte_size > 0);
        }
    }

    #[test]
    fn license_consent_is_required() {
        assert!(ensure_license_accepted(true).is_ok());
        assert!(matches!(
            ensure_license_accepted(false),
            Err(GpuPackDownloadError::LicenseNotAccepted)
        ));
    }

    /// Build a tiny in-memory zip whose DLLs live under nested `bin/...` paths (one
    /// even under a `bin/12.9/` CUDA-version subdir, like cuDNN's layout) plus an
    /// unrelated file, then assert the wanted DLLs are extracted FLAT and the
    /// noise file is ignored.
    #[test]
    fn extract_pulls_wanted_dlls_flat_ignoring_nesting() {
        let temp = tempfile::tempdir().expect("tempdir");
        let zip_path = temp.path().join("archive.zip");
        {
            let file = std::fs::File::create(&zip_path).expect("create zip");
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            writer
                .start_file("cudnn-archive/bin/cudnn64_9.dll", options)
                .unwrap();
            writer.write_all(b"loader dll bytes").unwrap();
            writer
                .start_file("cudnn-archive/bin/12.9/cudnn_graph64_9.dll", options)
                .unwrap();
            writer.write_all(b"graph dll bytes").unwrap();
            // Noise: a header we never want.
            writer
                .start_file("cudnn-archive/include/cudnn.h", options)
                .unwrap();
            writer.write_all(b"#pragma once").unwrap();
            writer.finish().unwrap();
        }

        let archive = GpuPackArchive {
            component: "cudnn",
            url: "https://developer.download.nvidia.com/compute/cudnn/redist/x.zip",
            sha256: "00",
            byte_size: 1,
            dll_names: &["cudnn64_9.dll", "cudnn_graph64_9.dll"],
        };
        let pack_dir = temp.path().join("pack");
        std::fs::create_dir_all(&pack_dir).unwrap();

        let mut installed = extract_archive_dlls(&archive, &zip_path, &pack_dir).expect("extract");
        installed.sort();
        assert_eq!(installed, vec!["cudnn64_9.dll", "cudnn_graph64_9.dll"]);
        assert!(pack_dir.join("cudnn64_9.dll").is_file(), "flat, not nested");
        assert!(pack_dir.join("cudnn_graph64_9.dll").is_file());
        // The header was ignored and nothing nested was recreated.
        assert!(!pack_dir.join("cudnn.h").exists());
        assert!(!pack_dir.join("bin").exists());
    }

    /// A wanted DLL absent from the archive is a hard error (a wrong pin must not
    /// silently produce a half-built pack).
    #[test]
    fn extract_errors_when_a_wanted_dll_is_absent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let zip_path = temp.path().join("archive.zip");
        {
            let file = std::fs::File::create(&zip_path).expect("create zip");
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("cuda/bin/cudart64_12.dll", options).unwrap();
            writer.write_all(b"runtime").unwrap();
            writer.finish().unwrap();
        }
        let archive = GpuPackArchive {
            component: "cuda_cudart",
            url: "https://developer.download.nvidia.com/compute/cuda/redist/x.zip",
            sha256: "00",
            byte_size: 1,
            dll_names: &["cudart64_12.dll", "not_present64_12.dll"],
        };
        let pack_dir = temp.path().join("pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        assert!(matches!(
            extract_archive_dlls(&archive, &zip_path, &pack_dir),
            Err(GpuPackDownloadError::MissingDllInArchive { .. })
        ));
    }

    /// The install marker round-trips: writing it flips `gpu_pack_present` true and
    /// `build_status` reads back the pinned versions + DLL list.
    #[test]
    fn marker_round_trips_into_status() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pack_dir = temp.path().join("gpu-acceleration-pack");
        std::fs::create_dir_all(&pack_dir).unwrap();

        // No marker yet ⇒ not installed.
        let before = build_status(&pack_dir, false);
        assert!(!before.pack_installed);
        assert!(before.installed_versions.is_none());
        assert_eq!(before.required_cuda_version, PINNED_CUDA_VERSION);
        assert_eq!(before.required_cudnn_version, PINNED_CUDNN_VERSION);
        assert_eq!(before.total_bytes, gpu_pack_total_bytes());
        assert!(before.license_urls.cuda.contains("nvidia.com"));

        write_pack_marker(&pack_dir, vec!["cudart64_12.dll".to_string()]).expect("write marker");
        let after = build_status(&pack_dir, true);
        assert!(after.pack_installed, "marker means provisioned");
        let versions = after.installed_versions.expect("versions present");
        assert_eq!(versions.cuda_version, PINNED_CUDA_VERSION);
        assert_eq!(versions.cudnn_version, PINNED_CUDNN_VERSION);
        assert_eq!(versions.installed_dlls, vec!["cudart64_12.dll".to_string()]);
        assert_eq!(
            after.download_state,
            Some(GpuAccelerationPackDownloadStatusDto::Downloading)
        );
    }
}
