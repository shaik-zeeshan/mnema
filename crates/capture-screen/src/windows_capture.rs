//! Windows screen-capture backend.
//!
//! Implements the first real Windows capture path for the runtime: record the
//! **primary monitor** with Windows Graphics Capture (WGC) and encode it to a
//! single playable H.264 `.mp4` via the Media Foundation `IMFSinkWriter`.
//!
//! Scope is deliberately narrow: primary-monitor screen video, segment rotation,
//! and low-cadence frame export. Resolution/bitrate honoring, system audio, and
//! privacy filters are still out of scope.
//!
//! ## Threading model
//!
//! A single dedicated **capture thread** owns every COM / D3D11 / Media
//! Foundation object. None of those interfaces are `Send`, so they never leave
//! that thread. Public entry points ([`ActiveCaptureSession::rotate`],
//! [`ActiveCaptureSession::stop`]) and the WGC event handlers communicate with
//! the capture thread over a single `mpsc` channel of [`Message`]s; control
//! messages carry a reply channel the caller blocks on. This keeps the whole
//! backend single-apartment (COM MTA) while exposing a `Send` session handle to
//! the runtime.

use std::any::Any;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use capture_types::{CaptureOutputFiles, CapturePermissionState, ScreenResolution};
use windows::core::{IInspectable, Interface, Result as WinResult, GUID, HSTRING, PCWSTR};
use windows::Foundation::Metadata::ApiInformation;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, HMONITOR, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::Media::MediaFoundation::{
    IMFMediaBuffer, IMFSample, IMFSinkWriter, MFCreateMediaType, MFCreateMemoryBuffer,
    MFCreateSample, MFCreateSinkWriterFromURL, MFMediaType_Video, MFShutdown, MFStartup,
    MFVideoFormat_H264, MFVideoFormat_NV12, MFVideoInterlace_Progressive, MFSTARTUP_FULL,
    MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
    MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_VERSION,
};
use windows::Win32::System::Com::{
    CoCreateGuid, CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED,
};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CMONITORS};

use crate::frame_schedule::{
    boundary_clamped_lookahead_duration_ticks, frame_cap_min_interval_ticks,
    lookahead_sample_duration_ticks, should_drop_frame, SegmentTimeline,
};
use crate::{
    captured_frame_equivalence_from_interleaved_bytes, captured_frame_equivalence_proofs_match,
    resolve_stream_resolution, screen_frame_artifact_path, CapturedFrameEquivalence,
    CapturedFrameEquivalenceOutcome, PrivacyFilterApplyOutcome, RotatedCaptureOutputs,
    ScreenCaptureSession, ScreenCaptureSessionOptions, ScreenCaptureSources, ScreenFrameArtifact,
    ScreenFrameArtifactHandler, ScreenFrameExportConfig, StartedCaptureSession,
};
use capture_types::CaptureErrorResponse;

/// 100ns ticks in one second (Media Foundation / WGC time unit).
const TICKS_PER_SECOND: i64 = 10_000_000;

const FRAME_EXPORT_JPEG_QUALITY: u8 = 85;
const FRAME_POOL_BUFFER_COUNT: i32 = 2;
const SCREEN_ACTIVITY_SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// WinRT type name whose `IsBorderRequired` property gates Win11 22000+ support.
const GRAPHICS_CAPTURE_SESSION_TYPE: &str = "Windows.Graphics.Capture.GraphicsCaptureSession";

// ---------------------------------------------------------------------------
// Support gate & permissions
// ---------------------------------------------------------------------------

/// Whether Windows Graphics Capture is usable on this machine.
///
/// Requires both `GraphicsCaptureSession::IsSupported()` and the presence of the
/// `IsBorderRequired` property (Win11 build 22000+). Tying support to that
/// property — rather than an OS build string — is what excludes Windows 10,
/// since the MVP depends on being able to turn the capture border off.
pub fn native_capture_supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| detect_native_capture_supported().unwrap_or(false))
}

fn detect_native_capture_supported() -> WinResult<bool> {
    if !GraphicsCaptureSession::IsSupported()? {
        return Ok(false);
    }
    let border_property_present = ApiInformation::IsPropertyPresent(
        &HSTRING::from(GRAPHICS_CAPTURE_SESSION_TYPE),
        &HSTRING::from("IsBorderRequired"),
    )?;
    Ok(border_property_present)
}

pub fn screen_permission_state() -> CapturePermissionState {
    if native_capture_supported() {
        CapturePermissionState::Granted
    } else {
        CapturePermissionState::Unsupported
    }
}

/// Windows Graphics Capture of the primary monitor needs no per-app prompt, so
/// "ensuring" permission is just reporting whether the platform supports it.
pub fn ensure_screen_permission() -> bool {
    native_capture_supported()
}

pub fn new_session_id() -> Result<String, CaptureErrorResponse> {
    // COM may not be initialized on the calling thread; `CoCreateGuid` does not
    // require it, but guard the call regardless.
    let guid = unsafe { CoCreateGuid() }.map_err(|e| win_error("CoCreateGuid failed", &e))?;
    Ok(format!("native-session-{}", format_guid_lower(&guid)))
}

fn format_guid_lower(guid: &GUID) -> String {
    let d4 = guid.data4;
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        guid.data1, guid.data2, guid.data3, d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]
    )
}

// ---------------------------------------------------------------------------
// Transient-liveness classification
// ---------------------------------------------------------------------------

/// Stop-error code recorded when the captured monitor goes away mid-recording
/// (`GraphicsCaptureItem.Closed`, e.g. monitor disconnect, lid close, session
/// lock blanking WGC). Per ADR 0023 this is a *transient liveness* loss the
/// runtime should ride out by suspending screen capture and auto-resuming, not
/// a genuine failure that ends the session — so it is named here once and shared
/// between the `Message::Closed` handler and the classification predicate rather
/// than duplicated as a bare string literal across the crate boundary.
pub const SCREEN_CAPTURE_ITEM_CLOSED_ERROR_CODE: &str = "screen_capture_item_closed";

/// Whether a screen stop-error code denotes a transient liveness loss (the
/// display went away) rather than a genuine capture failure.
///
/// The desktop lifecycle uses this to decide whether to enter a
/// `TransientLiveness` suspension (suspend screen, keep the session alive, probe
/// for a returning display via [`windows_display_present`]) instead of failing
/// the session. Keeping the predicate next to the producer means callers never
/// re-encode the error-code string (ADR 0023).
pub fn screen_capture_stop_error_is_transient_liveness(code: &str) -> bool {
    code == SCREEN_CAPTURE_ITEM_CLOSED_ERROR_CODE
}

// ---------------------------------------------------------------------------
// Cheap display-present probe
// ---------------------------------------------------------------------------

/// Whether at least one display monitor is present/attachable right now.
///
/// This is the Windows liveness signal for ADR 0023's display-unavailable
/// trigger: while every monitor is asleep, the lid is closed, the session is
/// locked, or the only monitor is unplugged, this returns `false`, so a
/// `TransientLiveness` suspension can wait quietly and only re-attempt WGC
/// capture once a display returns — mirroring macOS's `screen_display_available`
/// (`CGGetActiveDisplayList`) gate.
///
/// It uses `GetSystemMetrics(SM_CMONITORS)`, a single non-allocating Win32 call
/// that returns the number of display monitors, deliberately *not*
/// `EnumDisplayMonitors` (which runs a per-monitor callback) and with no COM /
/// WinRT / D3D session setup, so it is cheap enough to poll every ~2s from the
/// 1s segment-loop tick.
pub fn windows_display_present() -> bool {
    // SAFETY: `GetSystemMetrics` is a pure read of a system metric with no
    // pointer arguments and no initialization requirements.
    unsafe { GetSystemMetrics(SM_CMONITORS) > 0 }
}

// ---------------------------------------------------------------------------
// Capture-thread message protocol
// ---------------------------------------------------------------------------

/// Messages delivered to the capture thread over the single shared channel.
///
/// Frame/Closed are pushed by the WGC event handlers (running on free-threaded
/// frame-pool threads); Rotate/Stop are sent by the runtime via
/// [`ActiveCaptureSession`] and carry a reply channel.
enum Message {
    /// A new frame is available (`FrameArrived`).
    Frame,
    /// The capture item was closed (`GraphicsCaptureItem.Closed`, e.g. monitor
    /// disconnect).
    Closed,
    /// Finalize the current segment and begin writing the next one.
    Rotate {
        segment_dir: PathBuf,
        output_path: PathBuf,
        reply: Sender<Result<(), CaptureErrorResponse>>,
    },
    /// Finalize the final segment and tear the session down.
    Stop {
        reply: Sender<Result<(), CaptureErrorResponse>>,
    },
}

/// Liveness / error state shared between the capture thread and the session
/// handle held by the runtime.
#[derive(Default)]
struct SharedState {
    live: AtomicBool,
    stop_error: Mutex<Option<CaptureErrorResponse>>,
}

// ---------------------------------------------------------------------------
// Public session handle (lives on the runtime thread)
// ---------------------------------------------------------------------------

/// Handle to a running Windows capture session.
///
/// Holds no COM state itself — it forwards rotate/stop onto the capture thread
/// and reads liveness from [`SharedState`]. Implements the cross-platform
/// [`ScreenCaptureSession`] seam.
pub struct ActiveCaptureSession {
    sender: Sender<Message>,
    join_handle: Option<JoinHandle<()>>,
    shared: Arc<SharedState>,
    sources: ScreenCaptureSources,
    stopped: bool,
}

impl std::fmt::Debug for ActiveCaptureSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveCaptureSession")
            .field("live", &self.shared.live.load(Ordering::Relaxed))
            .field("stopped", &self.stopped)
            .finish_non_exhaustive()
    }
}

impl ActiveCaptureSession {
    fn send_stop(&mut self) -> Result<(), CaptureErrorResponse> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        self.shared.live.store(false, Ordering::Relaxed);
        let (reply_tx, reply_rx) = mpsc::channel();
        if self.sender.send(Message::Stop { reply: reply_tx }).is_err() {
            // Capture thread already gone; nothing more to finalize.
            return Ok(());
        }
        let result = reply_rx
            .recv()
            .unwrap_or_else(|_| Err(capture_thread_gone_error()));
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
        result
    }
}

impl ScreenCaptureSession for ActiveCaptureSession {
    fn rotate(
        &mut self,
        segment_dir: &Path,
        screen_output_file: Option<&Path>,
        _system_audio_output_path: Option<&Path>,
    ) -> Result<RotatedCaptureOutputs, CaptureErrorResponse> {
        let output_path = screen_output_file
            .map(Path::to_path_buf)
            .unwrap_or_else(|| {
                segment_dir.join(format!(
                    "screen.{}",
                    capture_runtime::screen_segment_extension()
                ))
            });

        if let Some(parent) = output_path.parent() {
            create_dir(parent)?;
        }

        let (reply_tx, reply_rx) = mpsc::channel();
        self.sender
            .send(Message::Rotate {
                segment_dir: segment_dir.to_path_buf(),
                output_path: output_path.clone(),
                reply: reply_tx,
            })
            .map_err(|_| capture_thread_gone_error())?;
        reply_rx
            .recv()
            .unwrap_or_else(|_| Err(capture_thread_gone_error()))?;

        let recording_file = output_path.to_string_lossy().to_string();
        Ok(RotatedCaptureOutputs {
            recording_file: recording_file.clone(),
            system_audio_recording_file: None,
            output_files: screen_only_output_files(&recording_file, &self.sources),
        })
    }

    fn stop(&mut self, _inactivity_tail_trim_seconds: u64) -> Result<(), CaptureErrorResponse> {
        // The Windows MVP has no audio / inactivity tail, so the trim argument is
        // intentionally ignored.
        self.send_stop()
    }

    fn is_live(&self) -> bool {
        self.shared.live.load(Ordering::Relaxed)
    }

    fn take_stop_error(&mut self) -> Option<CaptureErrorResponse> {
        self.shared
            .stop_error
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
    }

    fn supports_frame_export(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Drop for ActiveCaptureSession {
    fn drop(&mut self) {
        let _ = self.send_stop();
    }
}

// ---------------------------------------------------------------------------
// Session factory
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn start_capture_session_with_options(
    session_dir: &Path,
    screen_output_file: Option<&Path>,
    _system_audio_output_path: Option<&Path>,
    sources: &ScreenCaptureSources,
    screen_frame_rate: u32,
    screen_resolution: &ScreenResolution,
    video_bitrate_bps: Option<u32>,
    options: ScreenCaptureSessionOptions,
) -> Result<StartedCaptureSession, CaptureErrorResponse> {
    if !sources.screen {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Windows capture requires the screen source to be enabled".to_string(),
        });
    }

    if !native_capture_supported() {
        return Err(CaptureErrorResponse {
            code: "screen_capture_unsupported".to_string(),
            message: "Windows Graphics Capture requires Windows 11 (build 22000+)".to_string(),
        });
    }

    create_dir(session_dir)?;
    let output_path = screen_output_file
        .map(Path::to_path_buf)
        .unwrap_or_else(|| {
            session_dir.join(format!(
                "screen.{}",
                capture_runtime::screen_segment_extension()
            ))
        });
    if let Some(parent) = output_path.parent() {
        create_dir(parent)?;
    }

    let shared = Arc::new(SharedState::default());
    let (sender, receiver) = mpsc::channel::<Message>();
    let handler_sender = sender.clone();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), CaptureErrorResponse>>();

    let thread_shared = Arc::clone(&shared);
    let thread_output = output_path.clone();
    let config = CaptureThreadConfig {
        segment_dir: session_dir.to_path_buf(),
        output_path: thread_output,
        frame_rate: screen_frame_rate,
        screen_resolution: screen_resolution.clone(),
        video_bitrate_bps,
        frame_export: options.frame_export,
    };

    let join_handle = std::thread::Builder::new()
        .name("windows-capture".to_string())
        .spawn(move || {
            capture_thread_main(config, receiver, handler_sender, thread_shared, ready_tx);
        })
        .map_err(|e| CaptureErrorResponse {
            code: "capture_thread_spawn_failed".to_string(),
            message: format!("Failed to spawn Windows capture thread: {e}"),
        })?;

    // Wait for the capture thread to finish COM/D3D/MF setup before reporting
    // the session as started.
    match ready_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = join_handle.join();
            return Err(error);
        }
        Err(_) => {
            let _ = join_handle.join();
            return Err(capture_thread_gone_error());
        }
    }

    shared.live.store(true, Ordering::Relaxed);

    let recording_file = output_path.to_string_lossy().to_string();
    Ok(StartedCaptureSession {
        session: ActiveCaptureSession {
            sender,
            join_handle: Some(join_handle),
            shared,
            sources: *sources,
            stopped: false,
        },
        recording_file: recording_file.clone(),
        system_audio_recording_file: None,
        output_files: screen_only_output_files(&recording_file, sources),
        initial_privacy_filter_outcome: None::<PrivacyFilterApplyOutcome>,
    })
}

// ---------------------------------------------------------------------------
// Capture thread
// ---------------------------------------------------------------------------

struct CaptureThreadConfig {
    segment_dir: PathBuf,
    output_path: PathBuf,
    frame_rate: u32,
    screen_resolution: ScreenResolution,
    video_bitrate_bps: Option<u32>,
    frame_export: Option<ScreenFrameExportConfig>,
}

/// Entry point for the dedicated capture thread.
///
/// Performs COM/D3D/MF setup, signals readiness, then runs the message loop.
/// All native teardown (`MFShutdown`, `CoUninitialize`) happens here so it stays
/// on the same apartment-initialized thread.
fn capture_thread_main(
    config: CaptureThreadConfig,
    receiver: Receiver<Message>,
    handler_sender: Sender<Message>,
    shared: Arc<SharedState>,
    ready_tx: Sender<Result<(), CaptureErrorResponse>>,
) {
    // COM MTA for the whole thread lifetime.
    let com_hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if com_hr.is_err() {
        let _ = ready_tx.send(Err(CaptureErrorResponse {
            code: "com_init_failed".to_string(),
            message: format!("CoInitializeEx(MTA) failed: 0x{:08x}", com_hr.0),
        }));
        return;
    }

    match CaptureEngine::new(&config, &handler_sender) {
        Ok(mut engine) => {
            let _ = ready_tx.send(Ok(()));
            run_message_loop(&mut engine, receiver, &shared);
            engine.shutdown();
        }
        Err(error) => {
            let _ = ready_tx.send(Err(error));
        }
    }

    unsafe {
        MFShutdown().ok();
        CoUninitialize();
    }
}

fn run_message_loop(
    engine: &mut CaptureEngine,
    receiver: Receiver<Message>,
    shared: &Arc<SharedState>,
) {
    while let Ok(message) = receiver.recv() {
        match message {
            Message::Frame => {
                if let Err(error) = engine.process_next_frame() {
                    record_stop_error(shared, error);
                }
            }
            Message::Closed => {
                shared.live.store(false, Ordering::Relaxed);
                record_stop_error(
                    shared,
                    CaptureErrorResponse {
                        code: SCREEN_CAPTURE_ITEM_CLOSED_ERROR_CODE.to_string(),
                        message: "The captured monitor became unavailable (display disconnected or session closed)".to_string(),
                    },
                );
                // Keep the loop alive so a subsequent stop() still finalizes the
                // partially-written segment.
            }
            Message::Rotate {
                segment_dir,
                output_path,
                reply,
            } => {
                let result = engine.rotate(&segment_dir, &output_path);
                let _ = reply.send(result);
            }
            Message::Stop { reply } => {
                shared.live.store(false, Ordering::Relaxed);
                let result = engine.stop();
                let _ = reply.send(result);
                break;
            }
        }
    }
}

fn record_stop_error(shared: &Arc<SharedState>, error: CaptureErrorResponse) {
    shared.live.store(false, Ordering::Relaxed);
    let mut slot = shared.stop_error.lock().unwrap_or_else(|p| p.into_inner());
    if slot.is_none() {
        *slot = Some(error);
    }
}

// ---------------------------------------------------------------------------
// Native capture engine (capture-thread-owned COM/D3D/MF state)
// ---------------------------------------------------------------------------

struct CaptureEngine {
    device: ID3D11Device,
    d3d_device: IDirect3DDevice,
    context: ID3D11DeviceContext,
    // Held for the lifetime of the session so the captured item (and its
    // `Closed` event registration) stays alive; not read after construction.
    _item: GraphicsCaptureItem,
    frame_pool: Direct3D11CaptureFramePool,
    frame_pool_size: SizeInt32,
    session: GraphicsCaptureSession,
    writer: Option<SinkWriter>,
    staging: Option<ID3D11Texture2D>,
    frame_export: Option<WindowsFrameExportRuntime>,
    screen_activity: WindowsScreenActivityRuntime,
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
    scale_map: ScaleMap,
    frame_rate: u32,
    video_bitrate_bps: Option<u32>,
    min_interval_ticks: i64,
    timeline: SegmentTimeline,
    last_kept_ticks: Option<i64>,
    pending_frame: Option<PendingEncodedFrame>,
    nv12: Vec<u8>,
    /// Wall-clock start of the current segment, used to extend the encoded
    /// duration to match real time when the screen is mostly static (so a
    /// 60s segment is a 60s `.mp4`, not a single 33ms frame).
    segment_start: Instant,
    logged_invalid_content_size: bool,
    closed: bool,
}

/// The Media Foundation sink writer plus the stream index it was given.
struct SinkWriter {
    writer: IMFSinkWriter,
    stream_index: u32,
}

struct PendingEncodedFrame {
    relative_ticks: i64,
    nv12: Vec<u8>,
}

struct WindowsFrameExportRuntime {
    artifact_dir: PathBuf,
    on_frame_exported: ScreenFrameArtifactHandler,
    minimum_interval: Duration,
    last_exported_at: Option<Instant>,
    next_frame_index: u64,
    staging: Option<ID3D11Texture2D>,
    rgb: Vec<u8>,
    rgba_for_equivalence: Vec<u8>,
}

struct WindowsScreenActivityRuntime {
    minimum_interval: Duration,
    last_sampled_at: Option<Instant>,
    staging: Option<ID3D11Texture2D>,
    rgba_for_equivalence: Vec<u8>,
    last_equivalence: Option<CapturedFrameEquivalence>,
}

impl WindowsScreenActivityRuntime {
    fn new(width: u32, height: u32) -> Self {
        Self {
            minimum_interval: SCREEN_ACTIVITY_SAMPLE_INTERVAL,
            last_sampled_at: None,
            staging: None,
            rgba_for_equivalence: vec![0u8; (width as usize) * (height as usize) * 4],
            last_equivalence: None,
        }
    }
}

impl WindowsFrameExportRuntime {
    fn reset_for_segment(&mut self, segment_dir: &Path) -> Result<(), CaptureErrorResponse> {
        self.artifact_dir = windows_frame_artifact_dir(segment_dir)?;
        self.last_exported_at = None;
        self.next_frame_index = 0;
        Ok(())
    }
}

fn windows_frame_export_runtime(
    segment_dir: &Path,
    config: Option<ScreenFrameExportConfig>,
    width: u32,
    height: u32,
) -> Result<Option<WindowsFrameExportRuntime>, CaptureErrorResponse> {
    let Some(config) = config else {
        return Ok(None);
    };

    let artifact_dir = windows_frame_artifact_dir(segment_dir)?;
    Ok(Some(WindowsFrameExportRuntime {
        artifact_dir,
        on_frame_exported: config.on_frame_exported,
        minimum_interval: config.minimum_interval,
        last_exported_at: None,
        next_frame_index: 0,
        staging: None,
        rgb: vec![0u8; (width as usize) * (height as usize) * 3],
        rgba_for_equivalence: vec![0u8; (width as usize) * (height as usize) * 4],
    }))
}

fn windows_frame_artifact_dir(segment_dir: &Path) -> Result<PathBuf, CaptureErrorResponse> {
    let artifact_dir = segment_dir.join("frames");
    std::fs::create_dir_all(&artifact_dir).map_err(|error| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!(
            "Failed to create screen frame artifact directory {}: {error}",
            artifact_dir.display()
        ),
    })?;
    Ok(artifact_dir)
}

impl CaptureEngine {
    fn new(
        config: &CaptureThreadConfig,
        handler_sender: &Sender<Message>,
    ) -> Result<Self, CaptureErrorResponse> {
        unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| win_error("MFStartup failed", &e))?;

            let (device, context) = create_d3d_device()?;
            let d3d_device = direct3d_device_from_d3d11(&device)?;

            let hmonitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let item = capture_item_for_monitor(hmonitor)?;
            let size = item
                .Size()
                .map_err(|e| win_error("GraphicsCaptureItem.Size failed", &e))?;
            let source =
                normalized_source_dimensions(size).ok_or_else(|| CaptureErrorResponse {
                    code: "screen_capture_invalid_size".to_string(),
                    message: format!(
                        "Primary monitor reported an unusable size {}x{}",
                        size.Width, size.Height
                    ),
                })?;
            let source_width = source.width;
            let source_height = source.height;
            if source_width == 0 || source_height == 0 {
                return Err(CaptureErrorResponse {
                    code: "screen_capture_invalid_size".to_string(),
                    message: format!(
                        "Primary monitor reported an unusable size {source_width}x{source_height}"
                    ),
                });
            }
            let output_resolution =
                resolve_stream_resolution(&config.screen_resolution, source_width, source_height);
            let width = output_resolution.width;
            let height = output_resolution.height;
            let scale_map = ScaleMap::new(
                source_width as usize,
                source_height as usize,
                width as usize,
                height as usize,
            );

            let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &d3d_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                FRAME_POOL_BUFFER_COUNT,
                size,
            )
            .map_err(|e| win_error("CreateFreeThreaded frame pool failed", &e))?;

            let session = frame_pool
                .CreateCaptureSession(&item)
                .map_err(|e| win_error("CreateCaptureSession failed", &e))?;
            session
                .SetIsBorderRequired(false)
                .map_err(|e| win_error("SetIsBorderRequired(false) failed", &e))?;
            session
                .SetIsCursorCaptureEnabled(true)
                .map_err(|e| win_error("SetIsCursorCaptureEnabled(true) failed", &e))?;

            // FrameArrived / Closed handlers only ever touch the channel sender,
            // so the COM objects stay confined to this thread.
            let frame_sender = handler_sender.clone();
            let frame_handler = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
                move |_pool, _args| {
                    let _ = frame_sender.send(Message::Frame);
                    Ok(())
                },
            );
            frame_pool
                .FrameArrived(&frame_handler)
                .map_err(|e| win_error("FrameArrived registration failed", &e))?;

            let closed_sender = handler_sender.clone();
            let closed_handler =
                TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_item, _args| {
                    let _ = closed_sender.send(Message::Closed);
                    Ok(())
                });
            item.Closed(&closed_handler)
                .map_err(|e| win_error("Closed registration failed", &e))?;

            let writer = create_sink_writer(
                &config.output_path,
                width,
                height,
                config.frame_rate,
                config.video_bitrate_bps,
            )?;

            session
                .StartCapture()
                .map_err(|e| win_error("StartCapture failed", &e))?;

            Ok(Self {
                device,
                d3d_device,
                context,
                _item: item,
                frame_pool,
                frame_pool_size: size,
                session,
                writer: Some(writer),
                staging: None,
                frame_export: windows_frame_export_runtime(
                    &config.segment_dir,
                    config.frame_export.clone(),
                    width,
                    height,
                )?,
                screen_activity: WindowsScreenActivityRuntime::new(width, height),
                source_width,
                source_height,
                width,
                height,
                scale_map,
                frame_rate: config.frame_rate,
                video_bitrate_bps: config.video_bitrate_bps,
                min_interval_ticks: frame_cap_min_interval_ticks(config.frame_rate),
                timeline: SegmentTimeline::new(),
                last_kept_ticks: None,
                pending_frame: None,
                nv12: vec![0u8; (width as usize) * (height as usize) * 3 / 2],
                segment_start: Instant::now(),
                logged_invalid_content_size: false,
                closed: false,
            })
        }
    }

    fn process_next_frame(&mut self) -> Result<(), CaptureErrorResponse> {
        let frame = match self.frame_pool.TryGetNextFrame() {
            Ok(frame) => frame,
            // No frame currently queued; nothing to do.
            Err(_) => return Ok(()),
        };

        let content_size = frame
            .ContentSize()
            .map_err(|e| win_error("frame.ContentSize failed", &e))?;
        let Some(source_dimensions) = self.source_dimensions_for_content_size(content_size) else {
            return Ok(());
        };
        if capture_size_changed(self.frame_pool_size, content_size) {
            drop(frame);
            self.recreate_frame_pool(content_size, source_dimensions)?;
            return Ok(());
        }

        let absolute_ticks = frame
            .SystemRelativeTime()
            .map_err(|e| win_error("frame.SystemRelativeTime failed", &e))?
            .Duration;
        let relative_ticks = self.timeline.relative_ticks(absolute_ticks);

        let now = Instant::now();
        let should_export_frame = self.frame_export.as_ref().is_some_and(|runtime| {
            crate::should_export_screen_frame(
                runtime.last_exported_at,
                now,
                runtime.minimum_interval,
            )
        });
        let should_sample_screen_activity = crate::should_export_screen_frame(
            self.screen_activity.last_sampled_at,
            now,
            self.screen_activity.minimum_interval,
        );
        let should_encode_frame = !should_drop_frame(
            self.last_kept_ticks,
            relative_ticks,
            self.min_interval_ticks,
        );

        if !should_encode_frame && !should_export_frame && !should_sample_screen_activity {
            return Ok(());
        }

        let surface = frame
            .Surface()
            .map_err(|e| win_error("frame.Surface failed", &e))?;
        let access: IDirect3DDxgiInterfaceAccess = surface
            .cast()
            .map_err(|e| win_error("surface cast to IDirect3DDxgiInterfaceAccess failed", &e))?;
        let texture: ID3D11Texture2D = unsafe { access.GetInterface() }
            .map_err(|e| win_error("GetInterface::<ID3D11Texture2D> failed", &e))?;

        if should_encode_frame {
            self.encode_texture(&texture, relative_ticks)?;
            self.last_kept_ticks = Some(relative_ticks);
        }

        let exported_equivalence = if should_export_frame {
            match self.export_frame_artifact(&texture, now_unix_ms()) {
                Ok(equivalence) => Some(equivalence),
                Err(error) => {
                    capture_runtime::debug_log!(
                        "[capture-screen] failed to export Windows screen frame artifact: [{}] {}",
                        error.code,
                        error.message
                    );
                    None
                }
            }
        } else {
            None
        };

        if should_sample_screen_activity {
            if let Some(equivalence) = exported_equivalence.as_ref() {
                self.mark_screen_activity_for_equivalence(equivalence);
                self.screen_activity.last_sampled_at = Some(now);
            } else if let Err(error) = self.sample_screen_activity(&texture, now) {
                capture_runtime::debug_log!(
                    "[capture-screen] failed to sample Windows screen activity: [{}] {}",
                    error.code,
                    error.message
                );
            }
        }
        Ok(())
    }

    fn source_dimensions_for_content_size(
        &mut self,
        content_size: SizeInt32,
    ) -> Option<SourceDimensions> {
        let source = normalized_source_dimensions(content_size);
        if source.is_none() {
            if !self.logged_invalid_content_size {
                self.logged_invalid_content_size = true;
                capture_runtime::debug_log!(
                    "[capture-screen] skipping frame with unusable Windows content size {}x{}",
                    content_size.Width,
                    content_size.Height
                );
            }
        }
        source
    }

    /// Recreate the free-threaded WGC frame pool after resolution, DPI, or
    /// display-mode changes. The encoder output size stays fixed for the open
    /// segment; subsequent frames are scaled from the new source size.
    fn recreate_frame_pool(
        &mut self,
        content_size: SizeInt32,
        source: SourceDimensions,
    ) -> Result<(), CaptureErrorResponse> {
        self.frame_pool
            .Recreate(
                &self.d3d_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                FRAME_POOL_BUFFER_COUNT,
                content_size,
            )
            .map_err(|e| win_error("Direct3D11CaptureFramePool.Recreate failed", &e))?;

        capture_runtime::debug_log!(
            "[capture-screen] recreated Windows frame pool for content size {}x{} (was {}x{})",
            content_size.Width,
            content_size.Height,
            self.frame_pool_size.Width,
            self.frame_pool_size.Height
        );

        self.frame_pool_size = content_size;
        self.source_width = source.width;
        self.source_height = source.height;
        self.scale_map = ScaleMap::new(
            self.source_width as usize,
            self.source_height as usize,
            self.width as usize,
            self.height as usize,
        );
        self.staging = None;
        self.screen_activity.staging = None;
        if let Some(frame_export) = self.frame_export.as_mut() {
            frame_export.staging = None;
        }
        self.logged_invalid_content_size = false;

        Ok(())
    }

    fn encode_texture(
        &mut self,
        texture: &ID3D11Texture2D,
        relative_ticks: i64,
    ) -> Result<(), CaptureErrorResponse> {
        self.ensure_staging(texture)?;
        let staging = self
            .staging
            .as_ref()
            .expect("staging texture initialized above")
            .clone();

        unsafe {
            self.context.CopyResource(&staging, texture);

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| win_error("ID3D11DeviceContext.Map failed", &e))?;

            // SAFETY: `mapped.pData` points to at least height*RowPitch bytes of
            // BGRA data; we only read the even-rounded sub-region within bounds.
            bgra_to_nv12(
                mapped.pData as *const u8,
                mapped.RowPitch as usize,
                self.source_width as usize,
                self.source_height as usize,
                self.width as usize,
                self.height as usize,
                &self.scale_map,
                &mut self.nv12,
            );

            self.context.Unmap(&staging, 0);
        }

        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| no_active_writer_error("write sample"))?;
        if let Some(previous) = self.pending_frame.take() {
            let duration = lookahead_sample_duration_ticks(
                previous.relative_ticks,
                relative_ticks,
                frame_duration_ticks(self.frame_rate),
            );
            write_nv12_sample(
                &writer.writer,
                writer.stream_index,
                &previous.nv12,
                previous.relative_ticks,
                duration,
            )?;
        }
        self.pending_frame = Some(PendingEncodedFrame {
            relative_ticks,
            nv12: self.nv12.clone(),
        });
        Ok(())
    }

    fn export_frame_artifact(
        &mut self,
        texture: &ID3D11Texture2D,
        captured_at_unix_ms: u64,
    ) -> Result<CapturedFrameEquivalenceOutcome, CaptureErrorResponse> {
        let Some(runtime) = self.frame_export.as_mut() else {
            return Ok(CapturedFrameEquivalenceOutcome::quarantined(
                "Windows frame export runtime is not configured",
            ));
        };
        runtime.last_exported_at = Some(Instant::now());

        let frame_index = runtime.next_frame_index;
        runtime.next_frame_index = runtime.next_frame_index.saturating_add(1);
        let file_path =
            screen_frame_artifact_path(&runtime.artifact_dir, frame_index, captured_at_unix_ms);

        let captured_frame_equivalence = read_scaled_frame_equivalence(
            &self.device,
            &self.context,
            texture,
            &mut runtime.staging,
            self.source_width as usize,
            self.source_height as usize,
            self.width as usize,
            self.height as usize,
            &self.scale_map,
            Some(&mut runtime.rgb),
            &mut runtime.rgba_for_equivalence,
            "frame export",
        )?;

        save_rgb_as_jpeg(&file_path, self.width, self.height, &runtime.rgb)?;
        (runtime.on_frame_exported)(ScreenFrameArtifact {
            file_path: file_path.to_string_lossy().to_string(),
            captured_at_unix_ms,
            width: Some(self.width),
            height: Some(self.height),
            captured_frame_equivalence: captured_frame_equivalence.clone(),
        });
        Ok(captured_frame_equivalence)
    }

    fn sample_screen_activity(
        &mut self,
        texture: &ID3D11Texture2D,
        sampled_at: Instant,
    ) -> Result<(), CaptureErrorResponse> {
        self.screen_activity.last_sampled_at = Some(sampled_at);
        let captured_frame_equivalence = read_scaled_frame_equivalence(
            &self.device,
            &self.context,
            texture,
            &mut self.screen_activity.staging,
            self.source_width as usize,
            self.source_height as usize,
            self.width as usize,
            self.height as usize,
            &self.scale_map,
            None,
            &mut self.screen_activity.rgba_for_equivalence,
            "screen activity",
        )?;
        self.mark_screen_activity_for_equivalence(&captured_frame_equivalence);
        Ok(())
    }

    fn mark_screen_activity_for_equivalence(
        &mut self,
        captured_frame_equivalence: &CapturedFrameEquivalenceOutcome,
    ) {
        let CapturedFrameEquivalenceOutcome::Ready(current) = captured_frame_equivalence else {
            return;
        };

        let previous_equivalence = self.screen_activity.last_equivalence.as_ref();
        let first_ready_sample = previous_equivalence.is_none();
        let changed = match previous_equivalence {
            None => true,
            Some(previous) => {
                previous.version != current.version
                    || !captured_frame_equivalence_proofs_match(
                        current.version,
                        &previous.proof,
                        &current.proof,
                    )
            }
        };
        self.screen_activity.last_equivalence = Some(current.clone());

        if changed && crate::mark_screen_activity_now() {
            if first_ready_sample {
                capture_runtime::debug_log!(
                    "[capture-screen] Windows screen activity baseline established; equivalence_hint={}",
                    current.hint
                );
            } else {
                capture_runtime::debug_log!(
                    "[capture-screen] Windows screen activity changed; equivalence_hint={}",
                    current.hint
                );
            }
        }

        if !changed {
            capture_runtime::debug_log!(
                "[capture-screen] Windows screen activity unchanged; equivalence_hint={}",
                current.hint
            );
        }
    }

    /// Lazily create the CPU-readable staging texture.
    ///
    /// Its dimensions and format are taken from the **source** frame texture, not
    /// the (even-rounded) encoder dimensions: `CopyResource` requires both
    /// resources to be identical in size/format, and the captured surface height
    /// can be odd (e.g. 2039). `self.source_width`/`self.source_height` select
    /// the readable source area, while `self.width`/`self.height` are the
    /// configured output-resolution conversion target.
    fn ensure_staging(&mut self, source: &ID3D11Texture2D) -> Result<(), CaptureErrorResponse> {
        if self.staging.is_some() {
            return Ok(());
        }
        self.staging = Some(create_staging_texture(
            &self.device,
            source,
            "CreateTexture2D(staging)",
        )?);
        Ok(())
    }

    /// Flush the held lookahead frame at the segment boundary so the encoded
    /// video spans the real recording duration.
    ///
    /// Capture is change-driven: a static screen may produce a single frame at
    /// t=0. Holding that frame until rotation lets us write it once with a
    /// duration clamped exactly to the boundary, rather than writing a duplicate
    /// frame past the segment edge.
    fn flush_pending_frame_at_boundary(
        &mut self,
        boundary_ticks: i64,
    ) -> Result<(), CaptureErrorResponse> {
        let Some(pending) = self.pending_frame.take() else {
            return Ok(());
        };
        let Some(writer) = self.writer.as_ref() else {
            return Ok(());
        };
        if let Some(duration) =
            boundary_clamped_lookahead_duration_ticks(pending.relative_ticks, boundary_ticks)
        {
            write_nv12_sample(
                &writer.writer,
                writer.stream_index,
                &pending.nv12,
                pending.relative_ticks,
                duration,
            )?;
        }
        Ok(())
    }

    fn rotate(
        &mut self,
        segment_dir: &Path,
        output_path: &Path,
    ) -> Result<(), CaptureErrorResponse> {
        // Flush the closing segment at its boundary, then finalize it so it is
        // playable before the runtime is told the new segment has opened.
        let boundary_ticks = (self.segment_start.elapsed().as_nanos() / 100) as i64;
        self.flush_pending_frame_at_boundary(boundary_ticks)?;
        if let Some(writer) = self.writer.take() {
            unsafe {
                writer
                    .writer
                    .Finalize()
                    .map_err(|e| win_error("IMFSinkWriter.Finalize (rotate) failed", &e))?;
            }
        }

        let writer = create_sink_writer(
            output_path,
            self.width,
            self.height,
            self.frame_rate,
            self.video_bitrate_bps,
        )?;
        self.writer = Some(writer);
        self.timeline.reset();
        self.last_kept_ticks = None;
        self.pending_frame = None;
        self.segment_start = Instant::now();
        if let Some(frame_export) = self.frame_export.as_mut() {
            frame_export.reset_for_segment(segment_dir)?;
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), CaptureErrorResponse> {
        self.teardown_capture();
        let boundary_ticks = (self.segment_start.elapsed().as_nanos() / 100) as i64;
        self.flush_pending_frame_at_boundary(boundary_ticks)?;
        if let Some(writer) = self.writer.take() {
            unsafe {
                writer
                    .writer
                    .Finalize()
                    .map_err(|e| win_error("IMFSinkWriter.Finalize (stop) failed", &e))?;
            }
        }
        Ok(())
    }

    /// Close the WGC session and frame pool. Safe to call more than once.
    fn teardown_capture(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let _ = self.session.Close();
        let _ = self.frame_pool.Close();
    }

    /// Final teardown if the loop exits without an explicit stop (e.g. the
    /// sender was dropped). Finalizes any open writer so the file is playable.
    fn shutdown(&mut self) {
        self.teardown_capture();
        let boundary_ticks = (self.segment_start.elapsed().as_nanos() / 100) as i64;
        let _ = self.flush_pending_frame_at_boundary(boundary_ticks);
        if let Some(writer) = self.writer.take() {
            unsafe {
                let _ = writer.writer.Finalize();
            }
        }
    }
}

impl Drop for CaptureEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------------------------------------------------------------------------
// Native helpers
// ---------------------------------------------------------------------------

fn create_staging_texture(
    device: &ID3D11Device,
    source: &ID3D11Texture2D,
    context: &str,
) -> Result<ID3D11Texture2D, CaptureErrorResponse> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { source.GetDesc(&mut desc) };
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.SampleDesc = DXGI_SAMPLE_DESC {
        Count: 1,
        Quality: 0,
    };
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    desc.MiscFlags = 0;

    let mut staging: Option<ID3D11Texture2D> = None;
    unsafe {
        device
            .CreateTexture2D(&desc, None, Some(&mut staging))
            .map_err(|e| win_error(context, &e))?;
    }
    staging.ok_or_else(|| CaptureErrorResponse {
        code: "screen_capture_staging_failed".to_string(),
        message: format!("{context} returned a null staging texture"),
    })
}

#[allow(clippy::too_many_arguments)]
fn read_scaled_frame_equivalence(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
    staging: &mut Option<ID3D11Texture2D>,
    source_width: usize,
    source_height: usize,
    width: usize,
    height: usize,
    scale_map: &ScaleMap,
    rgb: Option<&mut [u8]>,
    rgba_for_equivalence: &mut [u8],
    readback_context: &str,
) -> Result<CapturedFrameEquivalenceOutcome, CaptureErrorResponse> {
    if staging.is_none() {
        *staging = Some(create_staging_texture(
            device,
            texture,
            &format!("CreateTexture2D({readback_context} staging)"),
        )?);
    }
    let staging_texture = staging
        .as_ref()
        .expect("readback staging texture initialized above")
        .clone();

    let equivalence = unsafe {
        context.CopyResource(&staging_texture, texture);

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context
            .Map(&staging_texture, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(|e| {
                win_error(
                    &format!("ID3D11DeviceContext.Map({readback_context}) failed"),
                    &e,
                )
            })?;

        bgra_to_rgb_and_rgba_scaled(
            mapped.pData as *const u8,
            mapped.RowPitch as usize,
            source_width,
            source_height,
            width,
            height,
            scale_map,
            rgb,
            rgba_for_equivalence,
        );
        let equivalence = captured_frame_equivalence_from_interleaved_bytes(
            rgba_for_equivalence,
            width * 4,
            width,
            height,
            [0, 1, 2, 3],
        )
        .map(CapturedFrameEquivalenceOutcome::ready)
        .unwrap_or_else(|| {
            CapturedFrameEquivalenceOutcome::quarantined(
                "failed to derive captured frame equivalence from downscaled Windows frame",
            )
        });

        context.Unmap(&staging_texture, 0);
        equivalence
    };

    Ok(equivalence)
}

fn create_d3d_device() -> Result<(ID3D11Device, ID3D11DeviceContext), CaptureErrorResponse> {
    // Try a hardware device first, fall back to WARP (software) so capture still
    // works in VMs / headless GPUs.
    for driver_type in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let result = unsafe {
            D3D11CreateDevice(
                None,
                driver_type,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        };
        if result.is_ok() {
            if let (Some(device), Some(context)) = (device, context) {
                return Ok((device, context));
            }
        }
    }
    Err(CaptureErrorResponse {
        code: "d3d_device_create_failed".to_string(),
        message: "D3D11CreateDevice failed for both hardware and WARP drivers".to_string(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceDimensions {
    width: u32,
    height: u32,
}

fn normalized_source_dimensions(size: SizeInt32) -> Option<SourceDimensions> {
    let width = (size.Width.max(0) as u32) & !1;
    let height = (size.Height.max(0) as u32) & !1;
    (width > 0 && height > 0).then_some(SourceDimensions { width, height })
}

fn capture_size_changed(previous: SizeInt32, next: SizeInt32) -> bool {
    previous.Width != next.Width || previous.Height != next.Height
}

fn direct3d_device_from_d3d11(
    device: &ID3D11Device,
) -> Result<IDirect3DDevice, CaptureErrorResponse> {
    let dxgi_device: IDXGIDevice = device
        .cast()
        .map_err(|e| win_error("ID3D11Device cast to IDXGIDevice failed", &e))?;
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }
        .map_err(|e| win_error("CreateDirect3D11DeviceFromDXGIDevice failed", &e))?;
    inspectable
        .cast()
        .map_err(|e| win_error("cast to IDirect3DDevice failed", &e))
}

fn capture_item_for_monitor(
    hmonitor: HMONITOR,
) -> Result<GraphicsCaptureItem, CaptureErrorResponse> {
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|e| win_error("GraphicsCaptureItem interop factory failed", &e))?;
    unsafe {
        interop
            .CreateForMonitor(hmonitor)
            .map_err(|e| win_error("CreateForMonitor failed", &e))
    }
}

fn create_sink_writer(
    output_path: &Path,
    width: u32,
    height: u32,
    frame_rate: u32,
    video_bitrate_bps: Option<u32>,
) -> Result<SinkWriter, CaptureErrorResponse> {
    let rate = if frame_rate == 0 { 30 } else { frame_rate };
    let bitrate = video_bitrate_bps.unwrap_or_else(|| default_bitrate_bps(width, height, rate));
    let url: Vec<u16> = output_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let writer = MFCreateSinkWriterFromURL(PCWSTR(url.as_ptr()), None, None)
            .map_err(|e| win_error("MFCreateSinkWriterFromURL failed", &e))?;

        let output_type =
            MFCreateMediaType().map_err(|e| win_error("MFCreateMediaType (output) failed", &e))?;
        output_type
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|e| win_error("set output major type failed", &e))?;
        output_type
            .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)
            .map_err(|e| win_error("set output subtype failed", &e))?;
        output_type
            .SetUINT32(&MF_MT_AVG_BITRATE, bitrate)
            .map_err(|e| win_error("set output bitrate failed", &e))?;
        output_type
            .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
            .map_err(|e| win_error("set output interlace mode failed", &e))?;
        output_type
            .SetUINT64(&MF_MT_FRAME_SIZE, pack_u32_pair(width, height))
            .map_err(|e| win_error("set output frame size failed", &e))?;
        output_type
            .SetUINT64(&MF_MT_FRAME_RATE, pack_u32_pair(rate, 1))
            .map_err(|e| win_error("set output frame rate failed", &e))?;
        output_type
            .SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u32_pair(1, 1))
            .map_err(|e| win_error("set output pixel aspect ratio failed", &e))?;

        let stream_index = writer
            .AddStream(&output_type)
            .map_err(|e| win_error("AddStream failed", &e))?;

        let input_type =
            MFCreateMediaType().map_err(|e| win_error("MFCreateMediaType (input) failed", &e))?;
        input_type
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|e| win_error("set input major type failed", &e))?;
        input_type
            .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)
            .map_err(|e| win_error("set input subtype failed", &e))?;
        input_type
            .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
            .map_err(|e| win_error("set input interlace mode failed", &e))?;
        input_type
            .SetUINT64(&MF_MT_FRAME_SIZE, pack_u32_pair(width, height))
            .map_err(|e| win_error("set input frame size failed", &e))?;
        input_type
            .SetUINT64(&MF_MT_FRAME_RATE, pack_u32_pair(rate, 1))
            .map_err(|e| win_error("set input frame rate failed", &e))?;
        input_type
            .SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u32_pair(1, 1))
            .map_err(|e| win_error("set input pixel aspect ratio failed", &e))?;

        writer
            .SetInputMediaType(stream_index, &input_type, None)
            .map_err(|e| win_error("SetInputMediaType failed", &e))?;

        writer
            .BeginWriting()
            .map_err(|e| win_error("BeginWriting failed", &e))?;

        Ok(SinkWriter {
            writer,
            stream_index,
        })
    }
}

/// Build and write a single NV12 frame as an `IMFSample`.
fn write_nv12_sample(
    writer: &IMFSinkWriter,
    stream_index: u32,
    nv12: &[u8],
    sample_time_ticks: i64,
    duration_ticks: i64,
) -> Result<(), CaptureErrorResponse> {
    unsafe {
        let buffer: IMFMediaBuffer = MFCreateMemoryBuffer(nv12.len() as u32)
            .map_err(|e| win_error("MFCreateMemoryBuffer failed", &e))?;

        let mut data_ptr: *mut u8 = std::ptr::null_mut();
        buffer
            .Lock(&mut data_ptr, None, None)
            .map_err(|e| win_error("IMFMediaBuffer.Lock failed", &e))?;
        std::ptr::copy_nonoverlapping(nv12.as_ptr(), data_ptr, nv12.len());
        buffer
            .Unlock()
            .map_err(|e| win_error("IMFMediaBuffer.Unlock failed", &e))?;
        buffer
            .SetCurrentLength(nv12.len() as u32)
            .map_err(|e| win_error("SetCurrentLength failed", &e))?;

        let sample: IMFSample =
            MFCreateSample().map_err(|e| win_error("MFCreateSample failed", &e))?;
        sample
            .AddBuffer(&buffer)
            .map_err(|e| win_error("IMFSample.AddBuffer failed", &e))?;
        sample
            .SetSampleTime(sample_time_ticks)
            .map_err(|e| win_error("SetSampleTime failed", &e))?;
        sample
            .SetSampleDuration(duration_ticks)
            .map_err(|e| win_error("SetSampleDuration failed", &e))?;

        writer
            .WriteSample(stream_index, &sample)
            .map_err(|e| win_error("WriteSample failed", &e))?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ScaleMap {
    x: Vec<usize>,
    y: Vec<usize>,
}

impl ScaleMap {
    fn new(src_width: usize, src_height: usize, dst_width: usize, dst_height: usize) -> Self {
        Self {
            x: (0..dst_width)
                .map(|x| scaled_source_index(x, dst_width, src_width))
                .collect(),
            y: (0..dst_height)
                .map(|y| scaled_source_index(y, dst_height, src_height))
                .collect(),
        }
    }
}

fn scaled_source_index(dst_index: usize, dst_len: usize, src_len: usize) -> usize {
    if src_len == 0 || dst_len == 0 {
        return 0;
    }

    let numerator = (dst_index as u128)
        .saturating_mul(src_len as u128)
        .saturating_add((src_len / 2) as u128);
    ((numerator / dst_len as u128) as usize).min(src_len.saturating_sub(1))
}

/// Convert a tightly-or-padded BGRA buffer to a contiguous NV12 buffer.
///
/// `src` points to `src_height * src_stride` bytes of BGRA. `dst` must be sized
/// `dst_width * dst_height * 3 / 2`. Uses nearest-neighbor scaling and BT.601
/// full-range-ish integer coefficients; good enough for screen content.
fn bgra_to_nv12(
    src: *const u8,
    src_stride: usize,
    src_width: usize,
    src_height: usize,
    dst_width: usize,
    dst_height: usize,
    scale_map: &ScaleMap,
    dst: &mut [u8],
) {
    let y_plane_len = dst_width * dst_height;
    debug_assert!(dst.len() >= y_plane_len + dst_width * (dst_height / 2));
    debug_assert_eq!(scale_map.x.len(), dst_width);
    debug_assert_eq!(scale_map.y.len(), dst_height);

    // Luma plane.
    for y in 0..dst_height {
        let src_y = scale_map.y[y].min(src_height.saturating_sub(1));
        let row = unsafe { std::slice::from_raw_parts(src.add(src_y * src_stride), src_width * 4) };
        let y_out = &mut dst[y * dst_width..y * dst_width + dst_width];
        for x in 0..dst_width {
            let src_x = scale_map.x[x].min(src_width.saturating_sub(1));
            let b = row[src_x * 4] as i32;
            let g = row[src_x * 4 + 1] as i32;
            let r = row[src_x * 4 + 2] as i32;
            y_out[x] = clamp_u8((77 * r + 150 * g + 29 * b) >> 8);
        }
    }

    // Chroma plane: one (U,V) pair per 2x2 block, sampled from the top-left
    // pixel of each block.
    let uv_base = y_plane_len;
    for by in 0..(dst_height / 2) {
        let dst_y = by * 2;
        let src_y = scale_map.y[dst_y].min(src_height.saturating_sub(1));
        let src_row =
            unsafe { std::slice::from_raw_parts(src.add(src_y * src_stride), src_width * 4) };
        let uv_out = &mut dst[uv_base + by * dst_width..uv_base + by * dst_width + dst_width];
        for bx in 0..(dst_width / 2) {
            let dst_x = bx * 2;
            let src_x = scale_map.x[dst_x].min(src_width.saturating_sub(1));
            let b = src_row[src_x * 4] as i32;
            let g = src_row[src_x * 4 + 1] as i32;
            let r = src_row[src_x * 4 + 2] as i32;
            let u = ((-43 * r - 84 * g + 127 * b) >> 8) + 128;
            let v = ((127 * r - 106 * g - 21 * b) >> 8) + 128;
            uv_out[bx * 2] = clamp_u8(u);
            uv_out[bx * 2 + 1] = clamp_u8(v);
        }
    }
}

/// Convert a tightly-or-padded BGRA buffer to scaled RGB bytes for JPEG and
/// scaled RGBA bytes for captured-frame equivalence.
fn bgra_to_rgb_and_rgba_scaled(
    src: *const u8,
    src_stride: usize,
    src_width: usize,
    src_height: usize,
    dst_width: usize,
    dst_height: usize,
    scale_map: &ScaleMap,
    mut rgb: Option<&mut [u8]>,
    rgba: &mut [u8],
) {
    debug_assert!(rgb
        .as_ref()
        .map_or(true, |rgb| rgb.len() >= dst_width * dst_height * 3));
    debug_assert!(rgba.len() >= dst_width * dst_height * 4);
    debug_assert_eq!(scale_map.x.len(), dst_width);
    debug_assert_eq!(scale_map.y.len(), dst_height);

    for y in 0..dst_height {
        let src_y = scale_map.y[y].min(src_height.saturating_sub(1));
        let row = unsafe { std::slice::from_raw_parts(src.add(src_y * src_stride), src_width * 4) };
        let mut rgb_out = rgb
            .as_deref_mut()
            .map(|rgb| &mut rgb[y * dst_width * 3..y * dst_width * 3 + dst_width * 3]);
        let rgba_out = &mut rgba[y * dst_width * 4..y * dst_width * 4 + dst_width * 4];
        for x in 0..dst_width {
            let src_x = scale_map.x[x].min(src_width.saturating_sub(1));
            let b = row[src_x * 4];
            let g = row[src_x * 4 + 1];
            let r = row[src_x * 4 + 2];
            if let Some(rgb_out) = rgb_out.as_deref_mut() {
                rgb_out[x * 3] = r;
                rgb_out[x * 3 + 1] = g;
                rgb_out[x * 3 + 2] = b;
            }
            rgba_out[x * 4] = r;
            rgba_out[x * 4 + 1] = g;
            rgba_out[x * 4 + 2] = b;
            rgba_out[x * 4 + 3] = 255;
        }
    }
}

fn save_rgb_as_jpeg(
    output_path: &Path,
    width: u32,
    height: u32,
    rgb: &[u8],
) -> Result<(), CaptureErrorResponse> {
    if let Some(parent) = output_path.parent() {
        create_dir(parent)?;
    }

    let file = File::create(output_path).map_err(|error| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!(
            "Failed to create screen frame artifact {}: {error}",
            output_path.display()
        ),
    })?;
    let mut output = BufWriter::new(file);
    let mut encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, FRAME_EXPORT_JPEG_QUALITY);
    encoder
        .encode(rgb, width, height, image::ColorType::Rgb8.into())
        .map_err(|error| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: format!(
                "Failed to encode Windows screen frame artifact {}: {error}",
                output_path.display()
            ),
        })
}

#[inline]
fn clamp_u8(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

/// Pack two `u32`s into the high/low halves of a `u64`, as Media Foundation
/// expects for size and ratio attributes (`MFSetAttribute2UINT32asUINT64`).
fn pack_u32_pair(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

fn frame_duration_ticks(frame_rate: u32) -> i64 {
    let rate = if frame_rate == 0 { 30 } else { frame_rate };
    TICKS_PER_SECOND / rate as i64
}

/// Default average H.264 bitrate when the runtime does not specify one.
///
/// ~0.1 bits/pixel/frame with a 2 Mbps floor and 60 Mbps ceiling.
fn default_bitrate_bps(width: u32, height: u32, frame_rate: u32) -> u32 {
    let pixels = width as u64 * height as u64;
    let raw = pixels * frame_rate as u64 / 10;
    raw.clamp(2_000_000, 60_000_000) as u32
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Small shared utilities
// ---------------------------------------------------------------------------

fn screen_only_output_files(
    recording_file: &str,
    sources: &ScreenCaptureSources,
) -> CaptureOutputFiles {
    let screen_file = sources.screen.then(|| recording_file.to_string());
    CaptureOutputFiles {
        screen_file: screen_file.clone(),
        screen_files: screen_file.into_iter().collect(),
        microphone_file: None,
        microphone_files: Vec::new(),
        system_audio_file: None,
        system_audio_files: Vec::new(),
    }
}

fn create_dir(path: &Path) -> Result<(), CaptureErrorResponse> {
    std::fs::create_dir_all(path).map_err(|e| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!("Failed to create capture directory {}: {e}", path.display()),
    })
}

fn win_error(context: &str, error: &windows::core::Error) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "windows_capture_failed".to_string(),
        message: format!("{context}: {error}"),
    }
}

fn no_active_writer_error(action: &str) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "invalid_runtime_state".to_string(),
        message: format!("No active Media Foundation writer to {action}"),
    }
}

fn capture_thread_gone_error() -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "capture_thread_unavailable".to_string(),
        message: "Windows capture thread is no longer running".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_bgra_frame(width: usize, height: usize) -> Vec<u8> {
        let mut frame = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                frame.push((x + y * width) as u8);
                frame.push((50 + x + y * width) as u8);
                frame.push((100 + x + y * width) as u8);
                frame.push(255);
            }
        }
        frame
    }

    fn y_from_bgr(b: u8, g: u8, r: u8) -> u8 {
        clamp_u8((77 * r as i32 + 150 * g as i32 + 29 * b as i32) >> 8)
    }

    #[test]
    fn scale_map_samples_nearest_source_centers() {
        let map = ScaleMap::new(4, 2, 2, 2);

        assert_eq!(map.x, vec![1, 3]);
        assert_eq!(map.y, vec![0, 1]);
    }

    #[test]
    fn bgra_to_rgb_and_rgba_scaled_uses_output_dimensions() {
        let source = test_bgra_frame(4, 2);
        let scale_map = ScaleMap::new(4, 2, 2, 2);
        let mut rgb = vec![0u8; 2 * 2 * 3];
        let mut rgba = vec![0u8; 2 * 2 * 4];

        bgra_to_rgb_and_rgba_scaled(
            source.as_ptr(),
            4 * 4,
            4,
            2,
            2,
            2,
            &scale_map,
            Some(&mut rgb),
            &mut rgba,
        );

        assert_eq!(rgb, vec![101, 51, 1, 103, 53, 3, 105, 55, 5, 107, 57, 7,]);
        assert_eq!(
            rgba,
            vec![101, 51, 1, 255, 103, 53, 3, 255, 105, 55, 5, 255, 107, 57, 7, 255,]
        );
    }

    #[test]
    fn bgra_to_rgb_and_rgba_scaled_can_skip_rgb_output() {
        let source = test_bgra_frame(4, 2);
        let scale_map = ScaleMap::new(4, 2, 2, 2);
        let mut rgba = vec![0u8; 2 * 2 * 4];

        bgra_to_rgb_and_rgba_scaled(
            source.as_ptr(),
            4 * 4,
            4,
            2,
            2,
            2,
            &scale_map,
            None,
            &mut rgba,
        );

        assert_eq!(
            rgba,
            vec![101, 51, 1, 255, 103, 53, 3, 255, 105, 55, 5, 255, 107, 57, 7, 255,]
        );
    }

    #[test]
    fn bgra_to_nv12_scaled_writes_downscaled_luma_plane() {
        let source = test_bgra_frame(4, 2);
        let scale_map = ScaleMap::new(4, 2, 2, 2);
        let mut nv12 = vec![0u8; 2 * 2 * 3 / 2];

        bgra_to_nv12(source.as_ptr(), 4 * 4, 4, 2, 2, 2, &scale_map, &mut nv12);

        assert_eq!(
            &nv12[..4],
            &[
                y_from_bgr(1, 51, 101),
                y_from_bgr(3, 53, 103),
                y_from_bgr(5, 55, 105),
                y_from_bgr(7, 57, 107),
            ]
        );
    }

    #[test]
    fn normalized_source_dimensions_rounds_down_to_even_nonzero_size() {
        assert_eq!(
            normalized_source_dimensions(SizeInt32 {
                Width: 1919,
                Height: 1079,
            }),
            Some(SourceDimensions {
                width: 1918,
                height: 1078,
            })
        );
    }

    #[test]
    fn normalized_source_dimensions_rejects_zero_or_negative_size() {
        assert_eq!(
            normalized_source_dimensions(SizeInt32 {
                Width: 0,
                Height: 1080,
            }),
            None
        );
        assert_eq!(
            normalized_source_dimensions(SizeInt32 {
                Width: 1920,
                Height: -1,
            }),
            None
        );
    }

    #[test]
    fn transient_liveness_predicate_matches_item_closed_code() {
        assert!(screen_capture_stop_error_is_transient_liveness(
            SCREEN_CAPTURE_ITEM_CLOSED_ERROR_CODE
        ));
        assert!(!screen_capture_stop_error_is_transient_liveness(
            "windows_capture_failed"
        ));
        assert!(!screen_capture_stop_error_is_transient_liveness(""));
    }

    // Smoke-level: any machine running this test has a display attached, so the
    // cheap `GetSystemMetrics(SM_CMONITORS)` probe must report one present.
    #[test]
    fn windows_display_present_reports_attached_display() {
        assert!(windows_display_present());
    }

    #[test]
    fn capture_size_changed_compares_raw_frame_pool_size() {
        assert!(!capture_size_changed(
            SizeInt32 {
                Width: 1920,
                Height: 1080,
            },
            SizeInt32 {
                Width: 1920,
                Height: 1080,
            }
        ));
        assert!(capture_size_changed(
            SizeInt32 {
                Width: 1920,
                Height: 1080,
            },
            SizeInt32 {
                Width: 1920,
                Height: 1079,
            }
        ));
    }
}

// `OsStr::encode_wide` lives behind this platform extension trait.
use std::os::windows::ffi::OsStrExt;
