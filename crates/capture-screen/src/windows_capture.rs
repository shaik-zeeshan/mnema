//! Windows screen-capture backend.
//!
//! Implements the first real Windows capture path for the runtime: record the
//! **primary monitor** with Windows Graphics Capture (WGC) and encode it to a
//! single playable H.264 `.mp4` via the Media Foundation `IMFSinkWriter`.
//!
//! Scope is deliberately the thinnest end-to-end slice (issue #45): no frame
//! export, no resolution/bitrate honoring, no system audio, no privacy filters.
//! Segment rotation *is* implemented because the runtime rotates on a ~60s
//! cadence and the closed segment must be finalized before `rotate()` returns.
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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Instant;

use capture_types::{CapturePermissionState, CaptureOutputFiles, ScreenResolution};
use windows::core::{IInspectable, Interface, Result as WinResult, GUID, HSTRING, PCWSTR};
use windows::Foundation::Metadata::ApiInformation;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, HMONITOR, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::Media::MediaFoundation::{
    IMFMediaBuffer, IMFSample, IMFSinkWriter, MFCreateMediaType, MFCreateMemoryBuffer,
    MFCreateSample, MFCreateSinkWriterFromURL, MFShutdown, MFStartup, MFMediaType_Video,
    MFVideoFormat_H264, MFVideoFormat_NV12, MFVideoInterlace_Progressive, MF_MT_AVG_BITRATE,
    MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
    MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_VERSION, MFSTARTUP_FULL,
};
use windows::Win32::System::Com::{CoCreateGuid, CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use crate::frame_schedule::{frame_cap_min_interval_ticks, should_drop_frame, SegmentTimeline};
use crate::{
    PrivacyFilterApplyOutcome, RotatedCaptureOutputs, ScreenCaptureSession, ScreenCaptureSources,
    ScreenCaptureSessionOptions, StartedCaptureSession,
};
use capture_types::CaptureErrorResponse;

/// 100ns ticks in one second (Media Foundation / WGC time unit).
const TICKS_PER_SECOND: i64 = 10_000_000;

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
        let output_path = screen_output_file.map(Path::to_path_buf).unwrap_or_else(|| {
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
        false
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
    _screen_resolution: &ScreenResolution,
    video_bitrate_bps: Option<u32>,
    _options: ScreenCaptureSessionOptions,
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
    let output_path = screen_output_file.map(Path::to_path_buf).unwrap_or_else(|| {
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
        output_path: thread_output,
        frame_rate: screen_frame_rate,
        video_bitrate_bps,
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
    output_path: PathBuf,
    frame_rate: u32,
    video_bitrate_bps: Option<u32>,
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
                        code: "screen_capture_item_closed".to_string(),
                        message: "The captured monitor became unavailable (display disconnected or session closed)".to_string(),
                    },
                );
                // Keep the loop alive so a subsequent stop() still finalizes the
                // partially-written segment.
            }
            Message::Rotate { output_path, reply } => {
                let result = engine.rotate(&output_path);
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
    context: ID3D11DeviceContext,
    // Held for the lifetime of the session so the captured item (and its
    // `Closed` event registration) stays alive; not read after construction.
    _item: GraphicsCaptureItem,
    frame_pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    writer: Option<SinkWriter>,
    staging: Option<ID3D11Texture2D>,
    width: u32,
    height: u32,
    frame_rate: u32,
    video_bitrate_bps: Option<u32>,
    min_interval_ticks: i64,
    timeline: SegmentTimeline,
    last_kept_ticks: Option<i64>,
    nv12: Vec<u8>,
    /// Wall-clock start of the current segment, used to extend the encoded
    /// duration to match real time when the screen is mostly static (so a
    /// 60s segment is a 60s `.mp4`, not a single 33ms frame).
    segment_start: Instant,
    logged_size_mismatch: bool,
    closed: bool,
}

/// The Media Foundation sink writer plus the stream index it was given.
struct SinkWriter {
    writer: IMFSinkWriter,
    stream_index: u32,
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
            let width = (size.Width.max(0) as u32) & !1;
            let height = (size.Height.max(0) as u32) & !1;
            if width == 0 || height == 0 {
                return Err(CaptureErrorResponse {
                    code: "screen_capture_invalid_size".to_string(),
                    message: format!("Primary monitor reported an unusable size {width}x{height}"),
                });
            }

            let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &d3d_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
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
            let frame_handler = TypedEventHandler::<
                Direct3D11CaptureFramePool,
                IInspectable,
            >::new(move |_pool, _args| {
                let _ = frame_sender.send(Message::Frame);
                Ok(())
            });
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
                context,
                _item: item,
                frame_pool,
                session,
                writer: Some(writer),
                staging: None,
                width,
                height,
                frame_rate: config.frame_rate,
                video_bitrate_bps: config.video_bitrate_bps,
                min_interval_ticks: frame_cap_min_interval_ticks(config.frame_rate),
                timeline: SegmentTimeline::new(),
                last_kept_ticks: None,
                nv12: vec![0u8; (width as usize) * (height as usize) * 3 / 2],
                segment_start: Instant::now(),
                logged_size_mismatch: false,
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
        let content_w = (content_size.Width.max(0) as u32) & !1;
        let content_h = (content_size.Height.max(0) as u32) & !1;
        if content_w != self.width || content_h != self.height {
            // Resolution/DPI change mid-session: skip for the MVP rather than
            // reconfiguring the encoder. Logged once to avoid spam.
            if !self.logged_size_mismatch {
                self.logged_size_mismatch = true;
                capture_runtime::debug_log!(
                    "[capture-screen] skipping frame with content size {content_w}x{content_h}; encoder fixed at {}x{}",
                    self.width,
                    self.height
                );
            }
            return Ok(());
        }

        let absolute_ticks = frame
            .SystemRelativeTime()
            .map_err(|e| win_error("frame.SystemRelativeTime failed", &e))?
            .Duration;
        let relative_ticks = self.timeline.relative_ticks(absolute_ticks);

        if should_drop_frame(self.last_kept_ticks, relative_ticks, self.min_interval_ticks) {
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

        self.encode_texture(&texture, relative_ticks)?;
        self.last_kept_ticks = Some(relative_ticks);
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
                self.width as usize,
                self.height as usize,
                &mut self.nv12,
            );

            self.context.Unmap(&staging, 0);
        }

        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| no_active_writer_error("write sample"))?;
        let duration = frame_duration_ticks(self.frame_rate);
        write_nv12_sample(
            &writer.writer,
            writer.stream_index,
            &self.nv12,
            relative_ticks,
            duration,
        )?;
        Ok(())
    }

    /// Lazily create the CPU-readable staging texture.
    ///
    /// Its dimensions and format are taken from the **source** frame texture, not
    /// the (even-rounded) encoder dimensions: `CopyResource` requires both
    /// resources to be identical in size/format, and the captured surface height
    /// can be odd (e.g. 2039). The even-rounded `self.width`/`self.height` are
    /// used only when reading the mapped sub-region for NV12 conversion.
    fn ensure_staging(&mut self, source: &ID3D11Texture2D) -> Result<(), CaptureErrorResponse> {
        if self.staging.is_some() {
            return Ok(());
        }
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
            self.device
                .CreateTexture2D(&desc, None, Some(&mut staging))
                .map_err(|e| win_error("CreateTexture2D(staging) failed", &e))?;
        }
        if staging.is_none() {
            return Err(CaptureErrorResponse {
                code: "screen_capture_staging_failed".to_string(),
                message: "CreateTexture2D returned a null staging texture".to_string(),
            });
        }
        self.staging = staging;
        Ok(())
    }

    /// Re-emit the last captured frame at the segment's elapsed wall-clock time
    /// so the encoded video spans the real recording duration.
    ///
    /// Capture is change-driven: a static screen may produce a single frame at
    /// t=0, which would otherwise yield a ~33ms `.mp4` for a 60s segment. By
    /// writing the last frame again at `elapsed`, the prior frame's display
    /// duration stretches to fill the segment. No-op when nothing was captured
    /// or the stream already spans the elapsed time.
    fn extend_segment_to_elapsed(&mut self) -> Result<(), CaptureErrorResponse> {
        let Some(last) = self.last_kept_ticks else {
            return Ok(());
        };
        let Some(writer) = self.writer.as_ref() else {
            return Ok(());
        };
        let elapsed_ticks = (self.segment_start.elapsed().as_nanos() / 100) as i64;
        let duration = frame_duration_ticks(self.frame_rate);
        if elapsed_ticks <= last + duration {
            return Ok(());
        }
        write_nv12_sample(
            &writer.writer,
            writer.stream_index,
            &self.nv12,
            elapsed_ticks,
            duration,
        )?;
        self.last_kept_ticks = Some(elapsed_ticks);
        Ok(())
    }

    fn rotate(&mut self, output_path: &Path) -> Result<(), CaptureErrorResponse> {
        // Stretch the closing segment to its real duration, then finalize it so
        // it is playable before the runtime is told the new segment has opened.
        self.extend_segment_to_elapsed()?;
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
        self.segment_start = Instant::now();
        Ok(())
    }

    fn stop(&mut self) -> Result<(), CaptureErrorResponse> {
        self.teardown_capture();
        self.extend_segment_to_elapsed()?;
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
        let _ = self.extend_segment_to_elapsed();
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

        let output_type = MFCreateMediaType().map_err(|e| win_error("MFCreateMediaType (output) failed", &e))?;
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

        let input_type = MFCreateMediaType().map_err(|e| win_error("MFCreateMediaType (input) failed", &e))?;
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

/// Convert a tightly-or-padded BGRA buffer to a contiguous NV12 buffer.
///
/// `src` points to `height * src_stride` bytes of BGRA. `dst` must be sized
/// `width * height * 3 / 2`. Uses BT.601 full-range-ish integer coefficients;
/// good enough for the MVP (screen content, not graded video).
fn bgra_to_nv12(src: *const u8, src_stride: usize, width: usize, height: usize, dst: &mut [u8]) {
    let y_plane_len = width * height;
    debug_assert!(dst.len() >= y_plane_len + width * (height / 2));

    // Luma plane.
    for y in 0..height {
        let row = unsafe { std::slice::from_raw_parts(src.add(y * src_stride), width * 4) };
        let y_out = &mut dst[y * width..y * width + width];
        for x in 0..width {
            let b = row[x * 4] as i32;
            let g = row[x * 4 + 1] as i32;
            let r = row[x * 4 + 2] as i32;
            y_out[x] = clamp_u8((77 * r + 150 * g + 29 * b) >> 8);
        }
    }

    // Chroma plane: one (U,V) pair per 2x2 block, sampled from the top-left
    // pixel of each block.
    let uv_base = y_plane_len;
    for by in 0..(height / 2) {
        let src_row =
            unsafe { std::slice::from_raw_parts(src.add(by * 2 * src_stride), width * 4) };
        let uv_out = &mut dst[uv_base + by * width..uv_base + by * width + width];
        for bx in 0..(width / 2) {
            let px = bx * 2;
            let b = src_row[px * 4] as i32;
            let g = src_row[px * 4 + 1] as i32;
            let r = src_row[px * 4 + 2] as i32;
            let u = ((-43 * r - 84 * g + 127 * b) >> 8) + 128;
            let v = ((127 * r - 106 * g - 21 * b) >> 8) + 128;
            uv_out[bx * 2] = clamp_u8(u);
            uv_out[bx * 2 + 1] = clamp_u8(v);
        }
    }
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

// `OsStr::encode_wide` lives behind this platform extension trait.
use std::os::windows::ffi::OsStrExt;
