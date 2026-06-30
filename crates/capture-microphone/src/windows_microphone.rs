//! Windows WASAPI audio-capture backend.
//!
//! Captures either the **default capture endpoint** (microphone, shared mode) or
//! the **default render endpoint** as WASAPI loopback (system audio, shared
//! mode), converts the mix-format PCM to interleaved 16-bit little-endian PCM,
//! and encodes it to AAC inside a playable `.m4a` via the Media Foundation sink
//! writer in `capture-writers` (`WindowsAacM4aSinkWriter`). This is the Windows
//! half of the cross-platform audio path; it mirrors the macOS AVFoundation
//! backend's externally-visible contract ([`crate::AudioCaptureSession`] +
//! [`crate::MicrophoneOutputFinalization`]).
//!
//! ## Threading model
//!
//! A single dedicated **capture thread** owns every COM / WASAPI / Media
//! Foundation object — none of those are `Send`, so they never leave that thread.
//! The public [`WasapiMicrophoneCaptureSession`] handle is a `Send` control
//! surface holding an `mpsc::Sender<Message>`, the `JoinHandle`, and an
//! `Arc<SharedState>` (atomic liveness + an async stop-error slot). Control
//! messages (`Rotate`, `Stop`) carry a reply channel the caller blocks on. The
//! capture thread runs a poll loop: drain WASAPI packets, convert, and append to
//! the current AAC `.m4a` segment. This mirrors
//! `capture-screen::windows_capture`, the canonical Windows capture backend.

use std::any::Any;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{JoinHandle, ThreadId};
use std::time::Duration;

use capture_types::{CaptureErrorResponse, MicrophoneDevice};
use capture_writers::{WindowsAacM4aSinkWriter, WindowsAudioTailHoldbackSink};
use wasapi::{
    get_default_device, initialize_mta, DeviceCollection, Direction, SampleType, StreamMode,
    WaveFormat,
};
use windows::Win32::Foundation::{E_ACCESSDENIED, PROPERTYKEY};
use windows::Win32::Media::Audio::{
    eCapture, eRender, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
    IMMNotificationClient_Impl, MMDeviceEnumerator, DEVICE_STATE, DEVICE_STATEMASK_ALL,
};
use windows::Win32::Media::MediaFoundation::{MFShutdown, MFStartup, MFSTARTUP_FULL, MF_VERSION};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
use windows_core::{implement, Interface, PCWSTR};

use crate::{
    AudioCaptureSession, MicrophoneInactivityTailTrimActivityMode, MicrophoneOutputFinalization,
};

/// 100ns ticks in one second (Media Foundation time unit).
const TICKS_PER_SECOND: i64 = 10_000_000;

/// Poll cadence for draining WASAPI packets.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Shared-mode device buffer duration request (100ns units).
///
/// `0` would let the engine allocate the minimum buffer — one engine period
/// (~10ms). With our ~10ms polling cadence and a *synchronous* AAC encode on
/// the same capture thread, that tiny buffer overflows almost every cycle under
/// real recording load (concurrent screen capture / H.264 encoding steal CPU),
/// so WASAPI drops the overflowed frames and flags `DATA_DISCONTINUITY` —
/// audible as continuous crackling. Request a 1s buffer so scheduling jitter of
/// up to ~1s can never overflow it. The buffer is plain memory (~350KB at
/// 44.1kHz stereo float); the engine period (drain cadence) is unchanged.
const SHARED_BUFFER_DURATION_HNS: i64 = 10_000_000;

/// AAC supports up to 2 channels; clamp wider endpoints down to stereo.
const MAX_AAC_CHANNELS: u16 = 2;

/// Per-sample storage format of the WASAPI mix format. Shared-mode mixers
/// almost always hand back 32-bit IEEE float, but a device can negotiate a
/// packed-int format; we decode each correctly rather than reading the low 16
/// bits at the wrong stride. Anything else is rejected explicitly at
/// `open_capture_stream` so corrupt PCM/VAD/peak never reaches the encoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SourceSampleFormat {
    /// 32-bit IEEE float in `-1.0..=1.0`.
    Float32,
    /// 16-bit signed little-endian int.
    Int16,
    /// 24-bit signed little-endian int (3 bytes, packed).
    Int24,
    /// 32-bit signed little-endian int.
    Int32,
}

impl SourceSampleFormat {
    /// Bytes occupied by one source sample (one channel) in the mix format.
    fn bytes_per_sample(self) -> usize {
        match self {
            SourceSampleFormat::Float32 | SourceSampleFormat::Int32 => 4,
            SourceSampleFormat::Int24 => 3,
            SourceSampleFormat::Int16 => 2,
        }
    }

    /// Resolve the mix format's subformat + bit depth into a decoder format,
    /// rejecting depths we cannot decode (so they surface as an explicit error
    /// instead of silent corruption).
    fn from_mix_format(mix_format: &WaveFormat) -> Result<Self, CaptureErrorResponse> {
        let bits = mix_format.get_bitspersample();
        match mix_format.get_subformat() {
            Ok(SampleType::Float) if bits == 32 => Ok(SourceSampleFormat::Float32),
            Ok(SampleType::Int) if bits == 16 => Ok(SourceSampleFormat::Int16),
            Ok(SampleType::Int) if bits == 24 => Ok(SourceSampleFormat::Int24),
            Ok(SampleType::Int) if bits == 32 => Ok(SourceSampleFormat::Int32),
            other => Err(CaptureErrorResponse {
                code: "windows_microphone_capture_failed".to_string(),
                message: format!(
                    "Unsupported WASAPI mix format (subformat: {other:?}, bits per sample: {bits})"
                ),
            }),
        }
    }

    /// Decode one source sample at `bytes` (length == `bytes_per_sample`) into a
    /// normalized f32 in `-1.0..=1.0`.
    fn sample_to_f32(self, bytes: &[u8]) -> f32 {
        match self {
            SourceSampleFormat::Float32 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            SourceSampleFormat::Int16 => {
                i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32_768.0
            }
            SourceSampleFormat::Int24 => {
                // Sign-extend the 3 LE bytes into an i32, then normalize.
                let raw = (bytes[0] as i32) | ((bytes[1] as i32) << 8) | ((bytes[2] as i32) << 16);
                let signed = (raw << 8) >> 8;
                signed as f32 / 8_388_608.0
            }
            SourceSampleFormat::Int32 => {
                i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32 / 2_147_483_648.0
            }
        }
    }

    /// Decode one source sample at `bytes` directly to the i16 the AAC encoder
    /// input expects.
    fn sample_to_i16(self, bytes: &[u8]) -> i16 {
        match self {
            SourceSampleFormat::Int16 => i16::from_le_bytes([bytes[0], bytes[1]]),
            _ => float_sample_to_i16(self.sample_to_f32(bytes)),
        }
    }
}

// ---------------------------------------------------------------------------
// Support gate
// ---------------------------------------------------------------------------

/// True iff a default WASAPI capture endpoint exists *right now*.
pub fn microphone_capture_supported() -> bool {
    // Audio endpoints hot-plug: a mic connected after launch must flip this gate
    // to `true` without an app restart (and vice versa), so the probe runs on
    // every call — never latch the result in a static.
    default_endpoint_exists_now(Direction::Capture)
}

/// True iff a default WASAPI render endpoint exists *right now* for
/// system-audio loopback.
pub fn system_audio_loopback_capture_supported() -> bool {
    default_endpoint_exists_now(Direction::Render)
}

fn default_endpoint_exists_now(direction: Direction) -> bool {
    // The probe touches COM: `initialize_mta` calls `CoInitializeEx(MTA)`, which
    // joins the *calling* thread to a multithreaded apartment. This gate runs on
    // the app's main thread during startup (status-bar setup), and that thread
    // must stay free for `tao` to `OleInitialize` it as a single-threaded
    // apartment when it creates the first window — mixing the two panics with
    // `RPC_E_CHANGED_MODE`. So run the probe on a throwaway thread, never the
    // caller's. The thread's apartment is torn down when it exits, so the MTA
    // never leaks back to the caller.
    std::thread::spawn(move || {
        let _ = initialize_mta();
        get_default_device(&direction).is_ok()
    })
    .join()
    .unwrap_or(false)
}

pub fn list_microphone_devices() -> Result<Vec<MicrophoneDevice>, CaptureErrorResponse> {
    std::thread::Builder::new()
        .name("windows-microphone-enumerate".to_string())
        .spawn(|| {
            let com_hr = initialize_mta();
            if com_hr.is_err() {
                return Err(CaptureErrorResponse {
                    code: "com_init_failed".to_string(),
                    message: format!("WASAPI COM MTA init failed: 0x{:08x}", com_hr.0),
                });
            }
            list_microphone_devices_on_initialized_thread()
        })
        .map_err(|e| CaptureErrorResponse {
            code: "capture_thread_spawn_failed".to_string(),
            message: format!("Failed to spawn Windows microphone enumeration thread: {e}"),
        })?
        .join()
        .unwrap_or_else(|_| {
            Err(CaptureErrorResponse {
                code: "windows_microphone_enumeration_failed".to_string(),
                message: "Windows microphone enumeration thread panicked".to_string(),
            })
        })
}

fn list_microphone_devices_on_initialized_thread(
) -> Result<Vec<MicrophoneDevice>, CaptureErrorResponse> {
    let default_id = get_default_device(&Direction::Capture)
        .ok()
        .and_then(|device| device.get_id().ok());
    let collection = DeviceCollection::new(&Direction::Capture)
        .map_err(|e| wasapi_error("EnumAudioEndpoints(Capture, ACTIVE) failed", &e))?;
    let count = collection
        .get_nbr_devices()
        .map_err(|e| wasapi_error("IMMDeviceCollection::GetCount failed", &e))?;
    let mut devices = Vec::with_capacity(count as usize);

    for index in 0..count {
        let device = collection
            .get_device_at_index(index)
            .map_err(|e| wasapi_error("IMMDeviceCollection::Item failed", &e))?;
        let id = device
            .get_id()
            .map_err(|e| wasapi_error("IMMDevice::GetId failed", &e))?;
        let name = device
            .get_friendlyname()
            .map_err(|e| wasapi_error("IMMDevice friendly name lookup failed", &e))?;
        let is_default = default_id.as_deref() == Some(id.as_str());
        devices.push(MicrophoneDevice {
            id,
            name,
            is_default,
        });
    }

    Ok(devices)
}

// ---------------------------------------------------------------------------
// Control messages + shared state
// ---------------------------------------------------------------------------

enum Message {
    /// Configure the audio writer's inactivity tail hold-back. Mirrors the macOS
    /// `set_audio_writer_inactivity_tail_trim_seconds` /
    /// `set_audio_writer_activity_threshold`: a non-zero `tail_trim_seconds`
    /// makes the active segment withhold its last N seconds of PCM ahead of the
    /// AAC encoder so an inactivity stop can discard the dead idle tail.
    ConfigureTailHoldback {
        tail_trim_seconds: u64,
        activity_threshold: f32,
        tail_activity_mode: MicrophoneInactivityTailTrimActivityMode,
    },
    /// Finalize the current segment and begin writing the next at `output_path`.
    /// A rotation is a normal segment boundary, so the withheld tail is flushed.
    Rotate {
        output_path: PathBuf,
        reply: Sender<Result<MicrophoneOutputFinalization, CaptureErrorResponse>>,
    },
    /// Finalize the current segment (discarding the withheld tail) and stop
    /// writing to disk. WASAPI capture keeps running so activity atomics stay
    /// live; a subsequent `Resume` reopens a fresh sink.
    Pause {
        reply: Sender<Result<MicrophoneOutputFinalization, CaptureErrorResponse>>,
    },
    /// Reopen a fresh sink at `new_path` and resume writing captured audio to
    /// disk. Must be sent after a `Pause` that completed successfully.
    Resume {
        new_path: PathBuf,
    },
    /// Finalize the final segment and tear the session down. `discard_inactivity_tail`
    /// is `true` when the stop is an inactivity pause (drop the withheld tail) and
    /// `false` for a normal stop (flush the withheld tail).
    Stop {
        discard_inactivity_tail: bool,
        reply: Sender<Result<MicrophoneOutputFinalization, CaptureErrorResponse>>,
    },
    DefaultRenderDeviceChanged {
        endpoint_id: Option<String>,
    },
}

/// Liveness / async-error state shared between the capture thread and the handle.
#[derive(Default)]
struct SharedState {
    live: AtomicBool,
    stop_error: Mutex<Option<CaptureErrorResponse>>,
}

// ---------------------------------------------------------------------------
// Public session handle (Send; lives on the runtime thread)
// ---------------------------------------------------------------------------

/// Handle to a running WASAPI microphone capture session.
///
/// Holds no COM state itself — it forwards rotate/stop onto the capture thread
/// and reads liveness from [`SharedState`]. Implements the cross-platform
/// [`AudioCaptureSession`] seam.
pub struct WasapiMicrophoneCaptureSession {
    sender: Sender<Message>,
    join_handle: Option<JoinHandle<()>>,
    shared: Arc<SharedState>,
    source_label: &'static str,
    stopped: bool,
}

impl std::fmt::Debug for WasapiMicrophoneCaptureSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasapiMicrophoneCaptureSession")
            .field("source", &self.source_label)
            .field("live", &self.shared.live.load(Ordering::Relaxed))
            .field("stopped", &self.stopped)
            .finish_non_exhaustive()
    }
}

/// System-audio loopback sessions use the same control handle and capture
/// thread as microphone sessions; only endpoint resolution and activity side
/// effects differ.
pub type WasapiSystemAudioCaptureSession = WasapiMicrophoneCaptureSession;

impl WasapiMicrophoneCaptureSession {
    fn send_rotate(
        &mut self,
        output_file: &str,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.sender
            .send(Message::Rotate {
                output_path: PathBuf::from(output_file),
                reply: reply_tx,
            })
            .map_err(|_| capture_thread_gone_error())?;
        reply_rx
            .recv()
            .unwrap_or_else(|_| Err(capture_thread_gone_error()))
    }

    fn send_stop(
        &mut self,
        discard_inactivity_tail: bool,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        if self.stopped {
            return Ok(MicrophoneOutputFinalization {
                source_file: None,
                output_file: None,
                speech_detected: false,
                trim_start_offset_ms: 0,
                discard_reason: None,
                duration_ms: None,
            });
        }
        self.stopped = true;
        self.shared.live.store(false, Ordering::Relaxed);

        let (reply_tx, reply_rx) = mpsc::channel();
        if self
            .sender
            .send(Message::Stop {
                discard_inactivity_tail,
                reply: reply_tx,
            })
            .is_err()
        {
            // Capture thread already gone; nothing more to finalize.
            if let Some(handle) = self.join_handle.take() {
                let _ = handle.join();
            }
            return Ok(MicrophoneOutputFinalization {
                source_file: None,
                output_file: None,
                speech_detected: false,
                trim_start_offset_ms: 0,
                discard_reason: None,
                duration_ms: None,
            });
        }
        let result = reply_rx
            .recv()
            .unwrap_or_else(|_| Err(capture_thread_gone_error()));
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
        result
    }

    /// Enable inactivity tail hold-back on the live segment writer. Mirrors the
    /// macOS `configure_microphone_writer_tail_buffer`: a non-zero
    /// `tail_trim_seconds` makes the active `.m4a` withhold its last N seconds of
    /// PCM ahead of the AAC encoder, with the trim boundary refined by peak level
    /// (`PeakLevel`) or VAD speech (`VadSpeech`). Best-effort — a dropped capture
    /// thread leaves the (already finalized) session untouched.
    pub fn configure_inactivity_tail_holdback(
        &mut self,
        tail_trim_seconds: u64,
        activity_threshold: f32,
        tail_activity_mode: MicrophoneInactivityTailTrimActivityMode,
    ) {
        let _ = self.sender.send(Message::ConfigureTailHoldback {
            tail_trim_seconds,
            activity_threshold,
            tail_activity_mode,
        });
    }

    /// Stop the session for an inactivity pause, DISCARDING the withheld tail so
    /// the committed final segment never carries the dead idle tail. Mirrors the
    /// macOS `finish_audio_asset_writer_discarding_inactivity_tail` path.
    pub fn stop_for_inactivity_returning_finalization(
        &mut self,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        self.send_stop(true)
    }

    pub fn pause_for_inactivity(
        &mut self,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.sender
            .send(Message::Pause { reply: reply_tx })
            .map_err(|_| capture_thread_gone_error())?;
        reply_rx
            .recv()
            .unwrap_or_else(|_| Err(capture_thread_gone_error()))
    }

    pub fn resume_from_inactivity(
        &mut self,
        new_output_path: &str,
    ) {
        let _ = self.sender.send(Message::Resume {
            new_path: PathBuf::from(new_output_path),
        });
    }
}

impl AudioCaptureSession for WasapiMicrophoneCaptureSession {
    fn rotate_output_file_returning_finalization(
        &mut self,
        output_file: &str,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        self.send_rotate(output_file)
    }

    fn stop_returning_finalization(
        &mut self,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        self.send_stop(false)
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

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Drop for WasapiMicrophoneCaptureSession {
    fn drop(&mut self) {
        // A dropped session without an explicit stop is a normal teardown, so
        // flush the withheld tail rather than discard it.
        let _ = self.send_stop(false);
    }
}

// ---------------------------------------------------------------------------
// Session factory
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum AudioCaptureSource {
    Microphone { device_id: Option<String> },
    SystemAudioLoopback,
}

impl AudioCaptureSource {
    fn label(&self) -> &'static str {
        match self {
            Self::Microphone { .. } => "microphone",
            Self::SystemAudioLoopback => "system audio",
        }
    }

    fn output_label(&self) -> &'static str {
        match self {
            Self::Microphone { .. } => "microphone",
            Self::SystemAudioLoopback => "system audio",
        }
    }

    fn thread_name(&self) -> &'static str {
        match self {
            Self::Microphone { .. } => "windows-microphone",
            Self::SystemAudioLoopback => "windows-system-audio-loopback",
        }
    }

    fn records_microphone_activity(&self) -> bool {
        matches!(self, Self::Microphone { .. })
    }

    fn records_system_audio_activity(&self) -> bool {
        matches!(self, Self::SystemAudioLoopback)
    }

    fn endpoint_direction(&self) -> Direction {
        match self {
            Self::Microphone { .. } => Direction::Capture,
            Self::SystemAudioLoopback => Direction::Render,
        }
    }

    fn client_direction(&self) -> Direction {
        match self {
            Self::Microphone { .. } | Self::SystemAudioLoopback => Direction::Capture,
        }
    }
}

fn create_output_parent_dir(
    output_path: &Path,
    source_label: &str,
) -> Result<(), CaptureErrorResponse> {
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| CaptureErrorResponse {
                code: "io_error".to_string(),
                message: format!(
                    "Failed to create {source_label} capture directory {}: {e}",
                    parent.display()
                ),
            })?;
        }
    }
    Ok(())
}

/// Start a WASAPI microphone capture session writing the first segment to
/// `output_file`. If `device_id` is `Some`, capture that active WASAPI endpoint
/// by exact endpoint id; otherwise capture the current default capture endpoint.
pub fn start_wasapi_microphone_capture_session_for_file(
    output_file: &str,
    device_id: Option<&str>,
) -> Result<WasapiMicrophoneCaptureSession, CaptureErrorResponse> {
    start_wasapi_audio_capture_session_for_file(
        output_file,
        AudioCaptureSource::Microphone {
            device_id: device_id.map(str::to_owned),
        },
    )
}

/// Start a WASAPI system-audio loopback capture session from the default render
/// endpoint. This is endpoint loopback, not process loopback: no process include
/// or exclude filter is applied.
pub fn start_wasapi_system_audio_capture_session_for_file(
    output_file: &str,
) -> Result<WasapiSystemAudioCaptureSession, CaptureErrorResponse> {
    start_wasapi_audio_capture_session_for_file(
        output_file,
        AudioCaptureSource::SystemAudioLoopback,
    )
}

fn start_wasapi_audio_capture_session_for_file(
    output_file: &str,
    source: AudioCaptureSource,
) -> Result<WasapiMicrophoneCaptureSession, CaptureErrorResponse> {
    if source.records_microphone_activity() {
        // Reset cross-platform activity/VAD state at the top of a fresh
        // microphone session, mirroring the macOS
        // `start_avfoundation_microphone_capture_session_with_output_file`.
        crate::reset_last_microphone_activity_unix_ms();
        crate::reset_microphone_vad_pcm_feed();
        crate::reset_microphone_vad_tail_activity();
    } else if source.records_system_audio_activity() {
        crate::reset_last_system_audio_activity_unix_ms();
    }

    let output_path = PathBuf::from(output_file);
    create_output_parent_dir(&output_path, source.output_label())?;

    let shared = Arc::new(SharedState::default());
    let (sender, receiver) = mpsc::channel::<Message>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), CaptureErrorResponse>>();
    let thread_name = source.thread_name();
    let source_label = source.label();

    let thread_shared = Arc::clone(&shared);
    let notification_sender = sender.clone();
    let join_handle = std::thread::Builder::new()
        .name(thread_name.to_string())
        .spawn(move || {
            capture_thread_main(
                output_path,
                source,
                receiver,
                notification_sender,
                thread_shared,
                ready_tx,
            );
        })
        .map_err(|e| CaptureErrorResponse {
            code: "capture_thread_spawn_failed".to_string(),
            message: format!("Failed to spawn Windows {source_label} capture thread: {e}"),
        })?;

    // Block until the capture thread finishes device + writer setup.
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

    Ok(WasapiMicrophoneCaptureSession {
        sender,
        join_handle: Some(join_handle),
        shared,
        source_label,
        stopped: false,
    })
}

// ---------------------------------------------------------------------------
// Capture thread
// ---------------------------------------------------------------------------

fn capture_thread_main(
    first_output_path: PathBuf,
    source: AudioCaptureSource,
    receiver: Receiver<Message>,
    notification_sender: Sender<Message>,
    shared: Arc<SharedState>,
    ready_tx: Sender<Result<(), CaptureErrorResponse>>,
) {
    // COM MTA for the whole thread lifetime.
    let com_hr = initialize_mta();
    if com_hr.is_err() {
        let _ = ready_tx.send(Err(CaptureErrorResponse {
            code: "com_init_failed".to_string(),
            message: format!("WASAPI COM MTA init failed: 0x{:08x}", com_hr.0),
        }));
        return;
    }

    // Media Foundation startup for the AAC writer; balanced by MFShutdown below.
    if let Err(e) = unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) } {
        let _ = ready_tx.send(Err(CaptureErrorResponse {
            code: "windows_audio_writer_failed".to_string(),
            message: format!("MFStartup failed: {e}"),
        }));
        return;
    }

    match CaptureEngine::new(&first_output_path, source, notification_sender) {
        Ok(mut engine) => {
            let _ = ready_tx.send(Ok(()));
            run_capture_loop(&mut engine, receiver, &shared);
        }
        Err(error) => {
            let _ = ready_tx.send(Err(error));
        }
    }

    unsafe {
        MFShutdown().ok();
    }
}

fn run_capture_loop(
    engine: &mut CaptureEngine,
    receiver: Receiver<Message>,
    shared: &Arc<SharedState>,
) {
    loop {
        // Drain any captured audio before handling control messages so the
        // freshly closed segment carries the latest frames.
        if let Err(error) = engine.pump() {
            record_stop_error(shared, error);
        }

        match receiver.try_recv() {
            Ok(Message::ConfigureTailHoldback {
                tail_trim_seconds,
                activity_threshold,
                tail_activity_mode,
            }) => {
                engine.configure_tail_holdback(
                    tail_trim_seconds,
                    activity_threshold,
                    tail_activity_mode,
                );
            }
            Ok(Message::Rotate { output_path, reply }) => {
                let result = engine.rotate(&output_path);
                let _ = reply.send(result);
            }
            Ok(Message::Pause { reply }) => {
                let result = engine.pause_segment();
                let _ = reply.send(result);
            }
            Ok(Message::Resume { new_path }) => {
                if let Err(error) = engine.resume_to_new_path(&new_path) {
                    record_stop_error(shared, error);
                }
            }
            Ok(Message::DefaultRenderDeviceChanged { endpoint_id }) => {
                if let Err(error) = engine.handle_default_render_device_changed(endpoint_id) {
                    record_stop_error(shared, error);
                }
            }
            Ok(Message::Stop {
                discard_inactivity_tail,
                reply,
            }) => {
                shared.live.store(false, Ordering::Relaxed);
                let result = engine.stop(discard_inactivity_tail);
                let _ = reply.send(result);
                break;
            }
            Err(mpsc::TryRecvError::Empty) => {
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                // Handle dropped without an explicit stop; finalize best-effort
                // as a normal stop (flush the withheld tail).
                let _ = engine.stop(false);
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
// Native capture engine (capture-thread-owned WASAPI / MF state)
// ---------------------------------------------------------------------------

/// Owns the WASAPI capture client and the current AAC `.m4a` segment writer.
struct CaptureEngine {
    audio_client: wasapi::AudioClient,
    capture_client: wasapi::AudioCaptureClient,
    /// Mix-format sample rate (Hz) used both for client init and MF timing.
    sample_rate_hz: u32,
    /// Channel count delivered by WASAPI (the mix-format channel count).
    source_channels: u16,
    /// Channel count written to the AAC stream (clamped to <= 2).
    output_channels: u16,
    /// Per-sample storage format of the WASAPI mix format.
    source_format: SourceSampleFormat,
    /// Bytes per WASAPI source frame (block align).
    source_bytes_per_frame: usize,
    /// Reusable raw-capture scratch buffer.
    raw: VecDeque<u8>,
    /// Whether captured packets should update microphone activity/VAD state.
    records_microphone_activity: bool,
    /// Whether captured packets should update system-audio activity state.
    records_system_audio_activity: bool,
    source: AudioCaptureSource,
    current_render_endpoint_id: Option<String>,
    _default_render_notifier: Option<SystemAudioDefaultDeviceChangeRegistration>,

    /// Active segment writer wrapped with the rolling inactivity tail hold-back;
    /// `None` once stopped.
    sink: Option<WindowsAudioTailHoldbackSink>,
    /// Filesystem path of the active segment.
    current_path: PathBuf,
    /// Frames appended to the active segment (for timing + empty detection).
    /// Advances by both real captured frames AND any dropped-frame gap inserted
    /// for a `DATA_DISCONTINUITY`, so MF sample times stay wall-clock aligned.
    frames_in_segment: u64,
    /// Device-position of the frame WASAPI is expected to deliver next (the
    /// previous packet's `index + frames`). A jump or a `DATA_DISCONTINUITY`
    /// flag means frames were dropped; the gap is folded into the MF timeline so
    /// audio does not compress against video (A/V desync). `None` until the
    /// first packet of a segment is read.
    next_expected_device_pos: Option<u64>,
    /// First fatal error recorded for the active segment.
    failed: bool,
    /// Inactivity tail hold-back configuration applied to each new segment's
    /// sink. `tail_trim_seconds == 0` disables hold-back.
    tail_trim_seconds: u64,
    /// Peak-level activity threshold (0.0..=1.0) for `PeakLevel` boundary mode.
    activity_threshold: f32,
    /// Tail boundary refinement mode (peak level vs VAD speech).
    tail_activity_mode: MicrophoneInactivityTailTrimActivityMode,
    /// Last VAD tail-speech sequence value observed by this engine, so a new
    /// speech pulse can be turned into a one-shot "mark tail active" exactly
    /// like the macOS `microphone_tail_activity_override` path.
    observed_vad_tail_speech_sequence: u64,
    paused: bool,
}

struct CaptureStream {
    audio_client: wasapi::AudioClient,
    capture_client: wasapi::AudioCaptureClient,
    endpoint_id: Option<String>,
    sample_rate_hz: u32,
    source_channels: u16,
    output_channels: u16,
    source_format: SourceSampleFormat,
    source_bytes_per_frame: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CaptureStreamFormat {
    sample_rate_hz: u32,
    source_channels: u16,
    output_channels: u16,
    source_format: SourceSampleFormat,
    source_bytes_per_frame: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DefaultRenderEndpointChange {
    Ignore,
    Reattach,
}

fn classify_default_render_endpoint_change(
    current_endpoint_id: Option<&str>,
    notified_endpoint_id: Option<&str>,
) -> DefaultRenderEndpointChange {
    if let Some(notified_endpoint_id) = notified_endpoint_id {
        if Some(notified_endpoint_id) == current_endpoint_id {
            return DefaultRenderEndpointChange::Ignore;
        }
    }

    DefaultRenderEndpointChange::Reattach
}

fn can_continue_active_writer(
    active: CaptureStreamFormat,
    replacement: CaptureStreamFormat,
) -> bool {
    let CaptureStreamFormat {
        sample_rate_hz: active_sample_rate_hz,
        source_channels: _,
        output_channels: active_output_channels,
        source_format: _,
        source_bytes_per_frame: _,
    } = active;
    let CaptureStreamFormat {
        sample_rate_hz: replacement_sample_rate_hz,
        source_channels: _,
        output_channels: replacement_output_channels,
        source_format: _,
        source_bytes_per_frame: _,
    } = replacement;

    active_sample_rate_hz == replacement_sample_rate_hz
        && active_output_channels == replacement_output_channels
}

impl CaptureStream {
    fn format(&self) -> CaptureStreamFormat {
        CaptureStreamFormat {
            sample_rate_hz: self.sample_rate_hz,
            source_channels: self.source_channels,
            output_channels: self.output_channels,
            source_format: self.source_format,
            source_bytes_per_frame: self.source_bytes_per_frame,
        }
    }
}

/// Run the shared VAD speech boundary trim over a just-closed segment `.m4a`.
///
/// Mirrors the macOS `finalize_microphone_output_context` ->
/// `finalize_microphone_vad_boundary_trim` discipline: the trim needs a distinct
/// input and output file (`trim_audio_file_to_m4a` cannot trim a file onto itself).
/// On macOS the writer wrote into a sibling temp *source* file and the trim wrote the
/// real output; here the WASAPI sink already produced the full (live-tail-trimmed)
/// clip at `closed_path` (the rotation planner's expected output path), so we first
/// stage it into the same sibling temp source and then let the shared finalize
/// tighten it back onto `closed_path`. The shared path drains the recorded
/// boundaries, trims to `first_start - padding .. last_end + padding` when they are
/// valid, plain-moves the staged source back into place when they are
/// missing/invalid, and discards when no speech was detected. A staging failure keeps
/// the full clip at its original path and drains the boundaries to avoid a
/// cross-segment leak.
///
/// Free function (not a `CaptureEngine` method) so it is exercisable without live
/// WASAPI: it depends only on `closed_path` and the shared finalize state.
pub(crate) fn finalize_windows_segment_boundary_trim(
    closed_path: String,
) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
    let source_path = crate::microphone_vad_temp_source_file(&closed_path);
    if let Err(error) = std::fs::rename(&closed_path, &source_path) {
        let _ = crate::take_microphone_vad_speech_boundaries();
        capture_runtime::debug_log!(
            "[capture-microphone] failed to stage microphone source for boundary trim; keeping full clip path={closed_path}: {error}"
        );
        return Ok(MicrophoneOutputFinalization {
            source_file: Some(closed_path.clone()),
            output_file: Some(closed_path),
            speech_detected: false,
            trim_start_offset_ms: 0,
            discard_reason: None,
            duration_ms: None,
        });
    }

    let mut context = crate::MicrophoneOutputContext {
        source_output_file: Some(source_path),
        output_file: Some(closed_path),
        boundary_trim_enabled: true,
        // Windows has no AVFoundation double-trim coordination, so the trim is never
        // pre-disabled (see `finalize_microphone_vad_boundary_trim`).
        boundary_trim_disabled: false,
        first_speech_start_secs: None,
        last_speech_end_secs: None,
        detected_vad_speech: false,
    };
    crate::finalize_microphone_vad_boundary_trim(&mut context)
}

impl CaptureEngine {
    fn new(
        output_path: &Path,
        source: AudioCaptureSource,
        notification_sender: Sender<Message>,
    ) -> Result<Self, CaptureErrorResponse> {
        let records_microphone_activity = source.records_microphone_activity();
        let records_system_audio_activity = source.records_system_audio_activity();
        let stream = Self::open_capture_stream(&source)?;
        let writer = WindowsAacM4aSinkWriter::create(
            output_path,
            stream.sample_rate_hz,
            stream.output_channels,
        )?;
        let sink = WindowsAudioTailHoldbackSink::new(writer, stream.sample_rate_hz);
        let default_render_notifier = if matches!(source, AudioCaptureSource::SystemAudioLoopback) {
            Some(SystemAudioDefaultDeviceChangeRegistration::register(
                notification_sender,
            )?)
        } else {
            None
        };

        Ok(Self {
            audio_client: stream.audio_client,
            capture_client: stream.capture_client,
            sample_rate_hz: stream.sample_rate_hz,
            source_channels: stream.source_channels,
            output_channels: stream.output_channels,
            source_format: stream.source_format,
            source_bytes_per_frame: stream.source_bytes_per_frame,
            raw: VecDeque::new(),
            records_microphone_activity,
            records_system_audio_activity,
            source,
            current_render_endpoint_id: stream.endpoint_id,
            _default_render_notifier: default_render_notifier,
            sink: Some(sink),
            current_path: output_path.to_path_buf(),
            frames_in_segment: 0,
            next_expected_device_pos: None,
            failed: false,
            tail_trim_seconds: 0,
            activity_threshold: 0.0,
            tail_activity_mode: MicrophoneInactivityTailTrimActivityMode::PeakLevel,
            observed_vad_tail_speech_sequence: 0,
            paused: false,
        })
    }

    /// Apply the inactivity tail hold-back configuration to the active sink and
    /// remember it for sinks created on later rotations. Resets the observed VAD
    /// speech sequence baseline so a pulse that predates this configuration does
    /// not spuriously mark the tail active.
    fn configure_tail_holdback(
        &mut self,
        tail_trim_seconds: u64,
        activity_threshold: f32,
        tail_activity_mode: MicrophoneInactivityTailTrimActivityMode,
    ) {
        self.tail_trim_seconds = tail_trim_seconds;
        self.activity_threshold = activity_threshold;
        self.tail_activity_mode = tail_activity_mode;
        self.observed_vad_tail_speech_sequence =
            crate::current_microphone_vad_tail_speech_sequence();
        self.apply_tail_holdback_to_sink();
    }

    fn apply_tail_holdback_to_sink(&mut self) {
        let vad_speech_mode = matches!(
            self.tail_activity_mode,
            MicrophoneInactivityTailTrimActivityMode::VadSpeech
        );
        if let Some(sink) = self.sink.as_mut() {
            sink.configure_tail_holdback(
                self.tail_trim_seconds,
                self.activity_threshold,
                vad_speech_mode,
            );
        }
    }

    fn open_capture_stream(
        source: &AudioCaptureSource,
    ) -> Result<CaptureStream, CaptureErrorResponse> {
        let client_direction = source.client_direction();
        let device = resolve_audio_capture_device(source)?;
        let endpoint_id = if matches!(source, AudioCaptureSource::SystemAudioLoopback) {
            Some(
                device
                    .get_id()
                    .map_err(|e| wasapi_error("IMMDevice::GetId failed", &e))?,
            )
        } else {
            None
        };
        let mut audio_client = device
            .get_iaudioclient()
            .map_err(|e| wasapi_error("get_iaudioclient failed", &e))?;
        let mix_format = audio_client
            .get_mixformat()
            .map_err(|e| wasapi_error("get_mixformat failed", &e))?;

        let sample_rate_hz = mix_format.get_samplespersec();
        let source_channels = mix_format.get_nchannels();
        let source_format = SourceSampleFormat::from_mix_format(&mix_format)?;
        let output_channels = source_channels.min(MAX_AAC_CHANNELS).max(1);
        let source_bytes_per_frame = WaveFormat::get_blockalign(&mix_format) as usize;
        // The block align must equal channels * per-sample stride for our
        // interleaved per-sample offset math; reject a format where it doesn't
        // rather than reading samples across frame boundaries.
        if source_bytes_per_frame != source_channels.max(1) as usize * source_format.bytes_per_sample()
        {
            return Err(CaptureErrorResponse {
                code: "windows_microphone_capture_failed".to_string(),
                message: format!(
                    "Unexpected WASAPI block align {source_bytes_per_frame} for {source_channels} channels of {source_format:?}"
                ),
            });
        }

        audio_client
            .initialize_client(
                &mix_format,
                &client_direction,
                &StreamMode::PollingShared {
                    autoconvert: false,
                    buffer_duration_hns: SHARED_BUFFER_DURATION_HNS,
                },
            )
            .map_err(|e| wasapi_error("initialize_client failed", &e))?;

        let capture_client = audio_client
            .get_audiocaptureclient()
            .map_err(|e| wasapi_error("get_audiocaptureclient failed", &e))?;
        audio_client
            .start_stream()
            .map_err(|e| wasapi_error("start_stream failed", &e))?;

        Ok(CaptureStream {
            audio_client,
            capture_client,
            endpoint_id,
            sample_rate_hz,
            source_channels,
            output_channels,
            source_format,
            source_bytes_per_frame,
        })
    }

    /// Drain all queued WASAPI packets and append them to the active segment.
    ///
    /// A mid-session WASAPI fault is terminal for this engine: latch `failed` so
    /// every subsequent `pump()` is a no-op rather than re-entering the same
    /// fault every poll cycle (~10ms) and spinning the capture loop. This
    /// mirrors how `handle_default_render_device_changed` latches `failed`.
    fn pump(&mut self) -> Result<(), CaptureErrorResponse> {
        if self.failed {
            return Ok(());
        }
        let result = self.pump_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn pump_inner(&mut self) -> Result<(), CaptureErrorResponse> {
        loop {
            let next = self
                .capture_client
                .get_next_packet_size()
                .map_err(|e| wasapi_error("get_next_packet_size failed", &e))?;
            match next {
                Some(frames) if frames > 0 => {
                    self.raw.clear();
                    let info = self
                        .capture_client
                        .read_from_device_to_deque(&mut self.raw)
                        .map_err(|e| wasapi_error("read_from_device_to_deque failed", &e))?;
                    let raw: Vec<u8> = self.raw.drain(..).collect();
                    self.append_raw_frames(
                        &raw,
                        info.flags.silent,
                        info.index,
                        info.flags.data_discontinuity,
                    )?;
                }
                _ => break,
            }
        }
        Ok(())
    }

    /// Convert a raw WASAPI packet (source mix format) to interleaved 16-bit LE
    /// PCM at the output channel count and append it to the active segment.
    /// Microphone sessions additionally emit the debug-visible Audio Activity
    /// Sample and feed mono VAD PCM; system-audio loopback emits only the
    /// independent system-audio Audio Activity Sample.
    fn append_raw_frames(
        &mut self,
        raw: &[u8],
        silent: bool,
        device_pos: u64,
        data_discontinuity: bool,
    ) -> Result<(), CaptureErrorResponse> {
        // WASAPI dropped frames between the previous packet and this one when the
        // device position jumps ahead of where we expected the next frame, or
        // when it raises `DATA_DISCONTINUITY`. macOS preserves such a gap via the
        // real sample-buffer PTS; we mirror that by folding the dropped-frame
        // count into `frames_in_segment` BEFORE stamping this packet, so the MF
        // sample time advances past the gap and audio stays wall-clock aligned
        // (otherwise the timeline silently compresses and A/V drifts apart).
        if let Some(expected) = self.next_expected_device_pos {
            if data_discontinuity && device_pos > expected {
                self.frames_in_segment = self
                    .frames_in_segment
                    .saturating_add(device_pos - expected);
            }
        }
        // Record where the next packet should continue. Tracked for every packet
        // (including silent/empty/paused ones) so we never mistake a normal
        // boundary for a gap. `source_bytes_per_frame` is non-zero here.
        let source_frames = if self.source_bytes_per_frame > 0 {
            (raw.len() / self.source_bytes_per_frame) as u64
        } else {
            0
        };
        self.next_expected_device_pos = Some(device_pos.saturating_add(source_frames));

        // Segment-relative start time of THIS packet, captured before we advance
        // `frames_in_segment`, so the VAD frame's media timeline matches the
        // writer's sample-time stamp.
        let media_start_secs = if self.sample_rate_hz > 0 {
            Some(self.frames_in_segment as f64 / self.sample_rate_hz as f64)
        } else {
            None
        };

        let (pcm, mono, peak) = decode_packet_to_pcm_and_mono(
            raw,
            self.source_bytes_per_frame,
            self.source_channels.max(1) as usize,
            self.output_channels as usize,
            self.source_format,
            silent,
            self.records_microphone_activity,
            self.records_microphone_activity || self.records_system_audio_activity,
        );

        if pcm.is_empty() {
            return Ok(());
        }

        if self.records_microphone_activity {
            // Emit the raw debug samples for every non-empty microphone packet
            // (including a silent one — that is a real activity sample of level
            // 0 and feeds silence into the VAD, matching macOS which records
            // every buffer).
            crate::note_microphone_activity_level(peak);
            crate::feed_microphone_vad_pcm(
                crate::MicrophoneVadSourceFormat::linear_pcm_mono(self.sample_rate_hz),
                crate::now_microphone_activity_unix_ms(),
                media_start_secs,
                &mono,
            );
        } else if self.records_system_audio_activity {
            // Loopback packets are system-audio activity samples only. They must
            // not mutate microphone state and must not feed microphone VAD.
            crate::note_system_audio_activity_level(peak);
        }

        if self.paused {
            return Ok(());
        }
        let output_channels = self.output_channels as usize;
        let frame_count = (pcm.len() / (output_channels * 2)) as u64;
        let sample_time_100ns = frames_to_ticks(self.frames_in_segment, self.sample_rate_hz);
        let duration_100ns = frames_to_ticks(frame_count, self.sample_rate_hz);

        // In VadSpeech mode the speech decision is computed off the capture
        // thread (the activity poller feeds the writer's VAD and pulses the
        // shared tail-speech sequence). A pulse since this engine last looked
        // means the chunk that was already buffered must be preserved up to the
        // speech boundary, so mark the latest buffered chunk active and release
        // the backlog BEFORE buffering the current (still no-speech) chunk —
        // mirroring the macOS `microphone_tail_activity_override` one-shot mark
        // followed by an `activity_override == Some(false)` append.
        let vad_speech_pulse = self.consume_vad_tail_speech_pulse();
        if vad_speech_pulse {
            if let Some(sink) = self.sink.as_mut() {
                sink.mark_tail_active()?;
            }
        }

        if let Some(sink) = self.sink.as_mut() {
            // `vad_speech` is always `false` here: in VadSpeech mode the pulse was
            // already consumed above against the prior chunk; in PeakLevel mode the
            // sink ignores this flag and uses the per-chunk peak instead.
            sink.append_pcm_s16(&pcm, sample_time_100ns, duration_100ns, peak, false)?;
            self.frames_in_segment += frame_count;
        }
        Ok(())
    }

    /// Returns `true` once per newly observed VAD tail-speech pulse. Only the
    /// microphone source participates: system-audio loopback does not feed
    /// microphone VAD, so it always trims by peak level. Mirrors the macOS
    /// `microphone_tail_activity_override` sequence comparison.
    fn consume_vad_tail_speech_pulse(&mut self) -> bool {
        if self.tail_trim_seconds == 0
            || !self.records_microphone_activity
            || !matches!(
                self.tail_activity_mode,
                MicrophoneInactivityTailTrimActivityMode::VadSpeech
            )
        {
            return false;
        }
        let sequence = crate::current_microphone_vad_tail_speech_sequence();
        if sequence > self.observed_vad_tail_speech_sequence {
            self.observed_vad_tail_speech_sequence = sequence;
            true
        } else {
            false
        }
    }

    /// Finalize the active segment and build its finalization record.
    ///
    /// `discard_inactivity_tail` selects the tail policy: a normal stop or a
    /// rotation flushes the withheld tail into the encoder (whole segment), while
    /// an inactivity stop discards it so the committed `.m4a` is shorter than
    /// wall-clock and never carries the dead idle tail.
    fn finalize_segment(
        &mut self,
        discard_inactivity_tail: bool,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        let closed_path = self.current_path.to_string_lossy().to_string();

        // The sink is the source of truth for whether anything actually reached
        // the encoder: `frames_in_segment` counts buffered chunks even while they
        // are only HELD in the tail, so on an inactivity stop that discards the
        // entire held tail nothing is ever written. Treating that as a file would
        // both lie (positive duration over an empty `.m4a`) and make `Finalize()`
        // fail with `MF_E_SINK_NO_SAMPLES_PROCESSED`. Mirror the macOS
        // `appended_samples == 0` guard: no samples written -> empty segment.
        let produced_output = if let Some(sink) = self.sink.take() {
            if discard_inactivity_tail {
                sink.finalize_discarding_inactivity_tail()?
            } else {
                sink.finalize_flushing()?
            }
        } else {
            false
        };

        let had_frames = produced_output && self.frames_in_segment > 0;
        let duration_ms = if self.sample_rate_hz > 0 && had_frames {
            Some(self.frames_in_segment * 1000 / self.sample_rate_hz as u64)
        } else {
            None
        };

        let boundary_trim_enabled = self.boundary_trim_enabled();

        if had_frames {
            // Layer the cross-platform VAD speech boundary trim on top of the live
            // tail-holdback-trimmed clip, matching the macOS finalize. The sink wrote
            // the full (already tail-trimmed) clip to `closed_path`; the boundary trim
            // then tightens it to the spoken sub-range.
            if boundary_trim_enabled {
                return finalize_windows_segment_boundary_trim(closed_path);
            }
            Ok(MicrophoneOutputFinalization {
                source_file: Some(closed_path.clone()),
                output_file: Some(closed_path),
                speech_detected: false,
                trim_start_offset_ms: 0,
                discard_reason: None,
                duration_ms,
            })
        } else {
            // No `.m4a` was produced, so there is nothing to trim. Still drain the
            // recorded boundaries (mirroring the macOS empty-output path) so they can
            // never leak into the next segment's trim window.
            if boundary_trim_enabled {
                let _ = crate::take_microphone_vad_speech_boundaries();
            }
            Ok(MicrophoneOutputFinalization {
                source_file: Some(closed_path),
                output_file: None,
                speech_detected: false,
                trim_start_offset_ms: 0,
                discard_reason: Some("no_audio_samples".to_string()),
                duration_ms: None,
            })
        }
    }

    /// Whether this engine's segment finalize should apply the VAD speech boundary
    /// trim. Mirrors the macOS `boundary_trim_enabled = matches!(tail_activity_mode,
    /// VadSpeech)`, additionally gated on the microphone source: only microphone
    /// capture feeds the shared VAD speech boundaries, so a system-audio loopback
    /// session must never trim by them.
    fn boundary_trim_enabled(&self) -> bool {
        self.records_microphone_activity
            && matches!(
                self.tail_activity_mode,
                MicrophoneInactivityTailTrimActivityMode::VadSpeech
            )
    }

    /// Finalize the current segment and begin a fresh one at `output_path`. A
    /// rotation is a normal segment boundary, so the withheld tail is flushed
    /// into the closing segment rather than discarded.
    fn rotate(
        &mut self,
        output_path: &Path,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        let finalization = self.finalize_segment(false)?;

        let writer = WindowsAacM4aSinkWriter::create(
            output_path,
            self.sample_rate_hz,
            self.output_channels,
        )?;
        self.sink = Some(WindowsAudioTailHoldbackSink::new(writer, self.sample_rate_hz));
        self.current_path = output_path.to_path_buf();
        self.frames_in_segment = 0;
        // Fresh segment timeline: the next packet seeds the gap baseline anew.
        self.next_expected_device_pos = None;
        // Carry the tail hold-back configuration onto the fresh segment's sink.
        self.apply_tail_holdback_to_sink();

        Ok(finalization)
    }

    fn handle_default_render_device_changed(
        &mut self,
        notified_endpoint_id: Option<String>,
    ) -> Result<(), CaptureErrorResponse> {
        if self.failed || !matches!(self.source, AudioCaptureSource::SystemAudioLoopback) {
            return Ok(());
        }
        if classify_default_render_endpoint_change(
            self.current_render_endpoint_id.as_deref(),
            notified_endpoint_id.as_deref(),
        ) == DefaultRenderEndpointChange::Ignore
        {
            return Ok(());
        }
        let mut stream = match Self::open_capture_stream(&self.source) {
            Ok(stream) => stream,
            Err(error) => {
                self.failed = true;
                return Err(error);
            }
        };
        if self.current_render_endpoint_id.as_deref() == stream.endpoint_id.as_deref() {
            let _ = stream.audio_client.stop_stream();
            return Ok(());
        }

        if !can_continue_active_writer(
            CaptureStreamFormat {
                sample_rate_hz: self.sample_rate_hz,
                source_channels: self.source_channels,
                output_channels: self.output_channels,
                source_format: self.source_format,
                source_bytes_per_frame: self.source_bytes_per_frame,
            },
            stream.format(),
        ) {
            let _ = stream.audio_client.stop_stream();
            self.failed = true;
            return Err(CaptureErrorResponse {
                code: "windows_system_audio_format_changed".to_string(),
                message: format!(
                    "Windows system audio default endpoint changed from {} Hz/{} channel(s) to {} Hz/{} channel(s); continuing the active .m4a segment would require resampling/remuxing that this backend does not perform",
                    self.sample_rate_hz,
                    self.output_channels,
                    stream.sample_rate_hz,
                    stream.output_channels
                ),
            });
        }

        let _ = self.audio_client.stop_stream();
        self.audio_client = stream.audio_client;
        self.capture_client = stream.capture_client;
        self.source_channels = stream.source_channels;
        self.source_format = stream.source_format;
        self.source_bytes_per_frame = stream.source_bytes_per_frame;
        self.current_render_endpoint_id = stream.endpoint_id.take();
        self.raw.clear();
        // The replacement endpoint has its own device-position clock unrelated to
        // the old one; reseed the gap baseline so the first packet from it is not
        // mistaken for a multi-billion-frame discontinuity.
        self.next_expected_device_pos = None;

        Ok(())
    }

    /// Finalize the current segment and tear down capture. `discard_inactivity_tail`
    /// drops the withheld tail (inactivity stop) instead of flushing it (normal
    /// stop).
    fn pause_segment(
        &mut self,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        let finalization = self.finalize_segment(true)?;
        self.paused = true;
        Ok(finalization)
    }

    fn resume_to_new_path(
        &mut self,
        new_path: &Path,
    ) -> Result<(), CaptureErrorResponse> {
        let writer = WindowsAacM4aSinkWriter::create(
            new_path,
            self.sample_rate_hz,
            self.output_channels,
        )?;
        self.sink = Some(WindowsAudioTailHoldbackSink::new(writer, self.sample_rate_hz));
        self.current_path = new_path.to_path_buf();
        self.frames_in_segment = 0;
        self.next_expected_device_pos = None;
        self.paused = false;
        self.apply_tail_holdback_to_sink();
        Ok(())
    }

    fn stop(
        &mut self,
        discard_inactivity_tail: bool,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        let _ = self.audio_client.stop_stream();
        self.finalize_segment(discard_inactivity_tail)
    }
}

// ---------------------------------------------------------------------------
// Device change notifier
// ---------------------------------------------------------------------------

type DeviceChangeCallback = dyn Fn() + Send + Sync + 'static;

pub struct MicrophoneDeviceChangeNotifier {
    stop_tx: Option<Sender<()>>,
    join_handle: Option<JoinHandle<()>>,
    thread_id: ThreadId,
}

impl std::fmt::Debug for MicrophoneDeviceChangeNotifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MicrophoneDeviceChangeNotifier")
            .field("running", &self.stop_tx.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for MicrophoneDeviceChangeNotifier {
    fn default() -> Self {
        Self {
            stop_tx: None,
            join_handle: None,
            thread_id: std::thread::current().id(),
        }
    }
}

impl Drop for MicrophoneDeviceChangeNotifier {
    fn drop(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if self.thread_id != std::thread::current().id() {
            if let Some(handle) = self.join_handle.take() {
                let _ = handle.join();
            }
        }
    }
}

pub fn start_microphone_device_change_notifier(
    callback: impl Fn() + Send + Sync + 'static,
) -> MicrophoneDeviceChangeNotifier {
    let callback: Arc<DeviceChangeCallback> = Arc::new(callback);
    let (stop_tx, stop_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let thread_callback = Arc::clone(&callback);

    let Ok(join_handle) = std::thread::Builder::new()
        .name("windows-microphone-device-notifier".to_string())
        .spawn(move || {
            device_change_notifier_thread(thread_callback, stop_rx, ready_tx);
        })
    else {
        return MicrophoneDeviceChangeNotifier::default();
    };

    let thread_id = join_handle.thread().id();
    match ready_rx.recv() {
        Ok(true) => MicrophoneDeviceChangeNotifier {
            stop_tx: Some(stop_tx),
            join_handle: Some(join_handle),
            thread_id,
        },
        _ => {
            let _ = join_handle.join();
            MicrophoneDeviceChangeNotifier::default()
        }
    }
}

fn device_change_notifier_thread(
    callback: Arc<DeviceChangeCallback>,
    stop_rx: Receiver<()>,
    ready_tx: Sender<bool>,
) {
    let com_hr = initialize_mta();
    if com_hr.is_err() {
        let _ = ready_tx.send(false);
        return;
    }

    let enumerator = match create_device_enumerator() {
        Ok(enumerator) => enumerator,
        Err(_) => {
            let _ = ready_tx.send(false);
            return;
        }
    };
    let known_capture_ids = Arc::new(Mutex::new(capture_endpoint_ids(&enumerator)));
    let client: IMMNotificationClient = MicrophoneNotificationClient {
        callback,
        known_capture_ids,
    }
    .into();

    if unsafe { enumerator.RegisterEndpointNotificationCallback(&client) }.is_err() {
        let _ = ready_tx.send(false);
        return;
    }

    let _ = ready_tx.send(true);
    let _ = stop_rx.recv();
    let _ = unsafe { enumerator.UnregisterEndpointNotificationCallback(&client) };
}

struct SystemAudioDefaultDeviceChangeRegistration {
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
}

impl SystemAudioDefaultDeviceChangeRegistration {
    fn register(sender: Sender<Message>) -> Result<Self, CaptureErrorResponse> {
        let enumerator = create_device_enumerator().map_err(|e| CaptureErrorResponse {
            code: "windows_system_audio_device_notifier_failed".to_string(),
            message: format!("MMDeviceEnumerator creation failed for system audio notifier: {e}"),
        })?;
        let client: IMMNotificationClient =
            SystemAudioDefaultRenderNotificationClient { sender }.into();
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client) }.map_err(|e| {
            CaptureErrorResponse {
                code: "windows_system_audio_device_notifier_failed".to_string(),
                message: format!(
                    "RegisterEndpointNotificationCallback failed for system audio notifier: {e}"
                ),
            }
        })?;
        Ok(Self { enumerator, client })
    }
}

impl Drop for SystemAudioDefaultDeviceChangeRegistration {
    fn drop(&mut self) {
        let _ = unsafe {
            self.enumerator
                .UnregisterEndpointNotificationCallback(&self.client)
        };
    }
}

#[implement(IMMNotificationClient)]
struct SystemAudioDefaultRenderNotificationClient {
    sender: Sender<Message>,
}

impl IMMNotificationClient_Impl for SystemAudioDefaultRenderNotificationClient_Impl {
    fn OnDeviceStateChanged(
        &self,
        _pwstrdeviceid: &PCWSTR,
        _dwnewstate: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnDeviceAdded(&self, _pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        _role: ERole,
        pwstrdefaultdeviceid: &PCWSTR,
    ) -> windows::core::Result<()> {
        if flow == eRender {
            let endpoint_id = unsafe { pcwstr_to_string(pwstrdefaultdeviceid) };
            let _ = self
                .sender
                .send(Message::DefaultRenderDeviceChanged { endpoint_id });
        }
        Ok(())
    }

    fn OnPropertyValueChanged(
        &self,
        _pwstrdeviceid: &PCWSTR,
        _key: &PROPERTYKEY,
    ) -> windows::core::Result<()> {
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct MicrophoneNotificationClient {
    callback: Arc<DeviceChangeCallback>,
    known_capture_ids: Arc<Mutex<HashSet<String>>>,
}

impl MicrophoneNotificationClient {
    fn notify(&self) {
        let callback = Arc::clone(&self.callback);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback()));
    }

    fn notify_if_capture_endpoint(&self, device_id: &PCWSTR) -> windows::core::Result<()> {
        let Some(device_id) = (unsafe { pcwstr_to_string(device_id) }) else {
            return Ok(());
        };
        let is_capture = device_id_is_capture_endpoint(&device_id).unwrap_or_else(|| {
            self.known_capture_ids
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .contains(&device_id)
        });
        if is_capture {
            self.known_capture_ids
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(device_id);
            self.notify();
        }
        Ok(())
    }
}

impl IMMNotificationClient_Impl for MicrophoneNotificationClient_Impl {
    fn OnDeviceStateChanged(
        &self,
        pwstrdeviceid: &PCWSTR,
        _dwnewstate: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        self.notify_if_capture_endpoint(pwstrdeviceid)
    }

    fn OnDeviceAdded(&self, pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
        self.notify_if_capture_endpoint(pwstrdeviceid)
    }

    fn OnDeviceRemoved(&self, pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
        let Some(device_id) = (unsafe { pcwstr_to_string(pwstrdeviceid) }) else {
            return Ok(());
        };
        if self
            .known_capture_ids
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&device_id)
        {
            self.notify();
        }
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        _role: ERole,
        _pwstrdefaultdeviceid: &PCWSTR,
    ) -> windows::core::Result<()> {
        if flow == eCapture {
            self.notify();
        }
        Ok(())
    }

    fn OnPropertyValueChanged(
        &self,
        pwstrdeviceid: &PCWSTR,
        _key: &PROPERTYKEY,
    ) -> windows::core::Result<()> {
        self.notify_if_capture_endpoint(pwstrdeviceid)
    }
}

fn create_device_enumerator() -> windows::core::Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

fn capture_endpoint_ids(enumerator: &IMMDeviceEnumerator) -> HashSet<String> {
    let Ok(collection) =
        (unsafe { enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE(DEVICE_STATEMASK_ALL)) })
    else {
        return HashSet::new();
    };
    let Ok(count) = (unsafe { collection.GetCount() }) else {
        return HashSet::new();
    };
    let mut ids = HashSet::with_capacity(count as usize);
    for index in 0..count {
        let Ok(device) = (unsafe { collection.Item(index) }) else {
            continue;
        };
        let Ok(id) = (unsafe { device.GetId() }) else {
            continue;
        };
        if let Some(id) = unsafe { pcwstr_to_string(&PCWSTR(id.0)) } {
            ids.insert(id);
        }
    }
    ids
}

fn device_id_is_capture_endpoint(device_id: &str) -> Option<bool> {
    let enumerator = create_device_enumerator().ok()?;
    let wide: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
    let device = unsafe { enumerator.GetDevice(PCWSTR(wide.as_ptr())) }.ok()?;
    let endpoint: windows::Win32::Media::Audio::IMMEndpoint = device.cast().ok()?;
    let flow = unsafe { endpoint.GetDataFlow() }.ok()?;
    Some(flow == eCapture)
}

unsafe fn pcwstr_to_string(value: &PCWSTR) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let mut len = 0usize;
    while *value.0.add(len) != 0 {
        len += 1;
    }
    Some(String::from_utf16_lossy(std::slice::from_raw_parts(
        value.0, len,
    )))
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn resolve_audio_capture_device(
    source: &AudioCaptureSource,
) -> Result<wasapi::Device, CaptureErrorResponse> {
    match source {
        AudioCaptureSource::Microphone { device_id } => resolve_audio_endpoint_device(
            source.endpoint_direction(),
            device_id.as_deref(),
            "selected_microphone_unavailable",
            "Selected Windows microphone endpoint is not active",
        ),
        AudioCaptureSource::SystemAudioLoopback => {
            resolve_audio_endpoint_device(source.endpoint_direction(), None, "", "")
        }
    }
}

fn resolve_audio_endpoint_device(
    endpoint_direction: Direction,
    device_id: Option<&str>,
    selected_unavailable_code: &str,
    selected_unavailable_message: &str,
) -> Result<wasapi::Device, CaptureErrorResponse> {
    let Some(device_id) = device_id else {
        return get_default_device(&endpoint_direction).map_err(|e| {
            wasapi_error(
                &format!("get_default_device({endpoint_direction}) failed"),
                &e,
            )
        });
    };

    let collection = DeviceCollection::new(&endpoint_direction).map_err(|e| {
        wasapi_error(
            &format!("EnumAudioEndpoints({endpoint_direction}, ACTIVE) failed"),
            &e,
        )
    })?;
    let count = collection
        .get_nbr_devices()
        .map_err(|e| wasapi_error("IMMDeviceCollection::GetCount failed", &e))?;

    for index in 0..count {
        let device = collection
            .get_device_at_index(index)
            .map_err(|e| wasapi_error("IMMDeviceCollection::Item failed", &e))?;
        let id = device
            .get_id()
            .map_err(|e| wasapi_error("IMMDevice::GetId failed", &e))?;
        if id == device_id {
            return Ok(device);
        }
    }

    Err(CaptureErrorResponse {
        code: selected_unavailable_code.to_string(),
        message: format!("{selected_unavailable_message}: {device_id}"),
    })
}

/// Decode one raw WASAPI packet (source mix format) into:
/// - interleaved 16-bit LE PCM at `output_channels` (for the AAC writer),
/// - optionally a mono f32 downmix (averaged across the SOURCE channels) for the
///   microphone VAD feed, and
/// - optionally the peak absolute mono level in 0.0..=1.0 for Audio Activity
///   Samples.
///
/// Mirrors the macOS downmix-to-mono used for activity/VAD. A `silent` packet
/// yields zeroed PCM, an optional all-zero mono buffer, and peak 0.0 without
/// trusting the (possibly stale) buffer contents.
fn decode_packet_to_pcm_and_mono(
    raw: &[u8],
    source_bytes_per_frame: usize,
    source_channels: usize,
    output_channels: usize,
    source_format: SourceSampleFormat,
    silent: bool,
    include_mono: bool,
    include_peak: bool,
) -> (Vec<u8>, Vec<f32>, f32) {
    if source_bytes_per_frame == 0 {
        return (Vec::new(), Vec::new(), 0.0);
    }
    let frame_count = raw.len() / source_bytes_per_frame;
    if frame_count == 0 {
        return (Vec::new(), Vec::new(), 0.0);
    }

    let source_channels = source_channels.max(1);
    let bytes_per_sample = source_format.bytes_per_sample();
    // The block align must be an exact multiple of the per-sample stride times
    // the channel count, or our per-sample offsets would read across frame
    // boundaries. This is upheld at `open_capture_stream`; assert it here so a
    // mismatched format fails loudly in tests rather than corrupting PCM.
    debug_assert_eq!(
        source_bytes_per_frame,
        source_channels * bytes_per_sample,
        "WASAPI block align must equal channels * bytes-per-sample"
    );

    if silent {
        // Honor the silent flag without trusting the (possibly stale) buffer.
        let mono = if include_mono {
            vec![0.0f32; frame_count]
        } else {
            Vec::new()
        };
        return (vec![0u8; frame_count * output_channels * 2], mono, 0.0);
    }

    let mut pcm = Vec::with_capacity(frame_count * output_channels * 2);
    let mut mono = if include_mono {
        Vec::with_capacity(frame_count)
    } else {
        Vec::new()
    };
    let mut peak = 0.0f32;

    for frame in 0..frame_count {
        let frame_base = frame * source_bytes_per_frame;

        // Build the writer's interleaved i16 output channels (unchanged behavior).
        for out_ch in 0..output_channels {
            // Map output channel to source channel; both are <= source.
            let src_ch = out_ch.min(source_channels - 1);
            let sample_off = frame_base + src_ch * bytes_per_sample;
            let value = source_format.sample_to_i16(&raw[sample_off..sample_off + bytes_per_sample]);
            pcm.extend_from_slice(&value.to_le_bytes());
        }

        if include_mono || include_peak {
            // Mono downmix: average over ALL source channels of this frame, in f32.
            let mut sum = 0.0f32;
            for src_ch in 0..source_channels {
                let sample_off = frame_base + src_ch * bytes_per_sample;
                sum += source_format
                    .sample_to_f32(&raw[sample_off..sample_off + bytes_per_sample]);
            }
            let value = (sum / source_channels as f32).clamp(-1.0, 1.0);
            peak = peak.max(value.abs());
            if include_mono {
                mono.push(value);
            }
        }
    }

    (pcm, mono, peak)
}

fn frames_to_ticks(frames: u64, sample_rate_hz: u32) -> i64 {
    if sample_rate_hz == 0 {
        return 0;
    }
    (frames as i128 * TICKS_PER_SECOND as i128 / sample_rate_hz as i128) as i64
}

fn float_sample_to_i16(sample: f32) -> i16 {
    let clamped = sample.clamp(-1.0, 1.0);
    (clamped * i16::MAX as f32).round() as i16
}

/// Map a WASAPI failure HRESULT to a recoverable Mnema error code, if it is one
/// we surface specially. Returns `None` for generic failures.
///
/// When the user blocks microphone access under Windows Settings -> Privacy &
/// security -> Microphone ("Let desktop apps access your microphone"), WASAPI
/// reports it as `E_ACCESSDENIED` (0x80070005) at `IAudioClient` activation /
/// `initialize_client`. Surfacing that as a distinct, recoverable code lets the
/// UI prompt the user to re-enable access rather than showing an opaque failure.
fn recoverable_code_for_hresult(code: windows_core::HRESULT) -> Option<&'static str> {
    if code == E_ACCESSDENIED {
        Some("microphone_access_denied")
    } else {
        None
    }
}

/// Centralize WASAPI error classification here so that both start-time errors
/// (`CaptureEngine::new`, `resolve_capture_device`) and mid-session pump-loop
/// errors classify consistently: a privacy denial surfaces as the recoverable
/// `microphone_access_denied` regardless of where in the lifecycle WASAPI raised
/// it, and everything else stays the opaque `windows_microphone_capture_failed`.
fn wasapi_error(context: &str, error: &wasapi::WasapiError) -> CaptureErrorResponse {
    if let wasapi::WasapiError::Windows(win) = error {
        if let Some(code) = recoverable_code_for_hresult(win.code()) {
            if code == "microphone_access_denied" {
                return CaptureErrorResponse {
                    code: code.to_string(),
                    message: "Microphone access is blocked in Windows privacy settings. Allow microphone access for desktop apps, then start recording again.".to_string(),
                };
            }
        }
    }
    CaptureErrorResponse {
        code: "windows_microphone_capture_failed".to_string(),
        message: format!("{context}: {error}"),
    }
}

fn capture_thread_gone_error() -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "capture_thread_gone".to_string(),
        message: "Windows microphone capture thread is no longer running".to_string(),
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use windows::Win32::Foundation::E_FAIL;

    fn capture_stream_format(
        sample_rate_hz: u32,
        source_channels: u16,
        output_channels: u16,
        source_format: SourceSampleFormat,
        source_bytes_per_frame: usize,
    ) -> CaptureStreamFormat {
        CaptureStreamFormat {
            sample_rate_hz,
            source_channels,
            output_channels,
            source_format,
            source_bytes_per_frame,
        }
    }

    #[test]
    fn duplicate_default_render_endpoint_notification_is_ignored() {
        assert_eq!(
            classify_default_render_endpoint_change(Some("endpoint-a"), Some("endpoint-a")),
            DefaultRenderEndpointChange::Ignore
        );
    }

    #[test]
    fn different_default_render_endpoint_notification_requests_reattach() {
        assert_eq!(
            classify_default_render_endpoint_change(Some("endpoint-a"), Some("endpoint-b")),
            DefaultRenderEndpointChange::Reattach
        );
    }

    #[test]
    fn writer_format_changes_are_incompatible_with_active_segment() {
        let active = capture_stream_format(48_000, 2, 2, SourceSampleFormat::Int16, 4);

        assert!(!can_continue_active_writer(
            active,
            capture_stream_format(44_100, 2, 2, SourceSampleFormat::Int16, 4)
        ));
        assert!(!can_continue_active_writer(
            active,
            capture_stream_format(48_000, 2, 1, SourceSampleFormat::Int16, 4)
        ));
    }

    #[test]
    fn source_format_changes_are_compatible_when_writer_format_is_stable() {
        assert!(can_continue_active_writer(
            capture_stream_format(48_000, 2, 2, SourceSampleFormat::Int16, 4),
            capture_stream_format(48_000, 6, 2, SourceSampleFormat::Float32, 24)
        ));
    }

    #[test]
    fn access_denied_classifies_as_recoverable() {
        assert_eq!(
            recoverable_code_for_hresult(E_ACCESSDENIED),
            Some("microphone_access_denied")
        );
    }

    #[test]
    fn unrelated_hresult_is_generic() {
        assert_eq!(recoverable_code_for_hresult(E_FAIL), None);
    }

    #[test]
    fn wasapi_error_surfaces_access_denied() {
        let err = wasapi::WasapiError::Windows(windows_core::Error::from(E_ACCESSDENIED));
        let response = wasapi_error("ctx", &err);
        assert_eq!(response.code, "microphone_access_denied");
        assert!(response.message.contains("privacy settings"));
    }

    #[test]
    fn wasapi_error_stays_generic_for_other_failures() {
        let err = wasapi::WasapiError::Windows(windows_core::Error::from(E_FAIL));
        let response = wasapi_error("ctx", &err);
        assert_eq!(response.code, "windows_microphone_capture_failed");
        assert!(response.message.starts_with("ctx: "));
    }

    #[test]
    fn microphone_source_uses_capture_endpoint_and_activity() {
        let source = AudioCaptureSource::Microphone { device_id: None };
        assert_eq!(source.endpoint_direction(), Direction::Capture);
        assert_eq!(source.client_direction(), Direction::Capture);
        assert!(source.records_microphone_activity());
        assert!(!source.records_system_audio_activity());
    }

    #[test]
    fn system_audio_source_uses_render_endpoint_loopback_with_system_audio_activity_only() {
        let source = AudioCaptureSource::SystemAudioLoopback;
        assert_eq!(source.endpoint_direction(), Direction::Render);
        assert_eq!(source.client_direction(), Direction::Capture);
        assert!(!source.records_microphone_activity());
        assert!(source.records_system_audio_activity());
    }

    /// Decode `pcm` back into i16 samples for round-trip assertions.
    fn pcm_to_i16(pcm: &[u8]) -> Vec<i16> {
        pcm.chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    #[test]
    fn decode_stereo_i16_zero_mono_and_peak() {
        // Stereo i16 source, 2 output channels. Frame [16384, -16384] averages to 0.
        let mut raw = Vec::new();
        raw.extend_from_slice(&16384i16.to_le_bytes());
        raw.extend_from_slice(&(-16384i16).to_le_bytes());
        let (pcm, mono, peak) = decode_packet_to_pcm_and_mono(
            &raw, /* bytes_per_frame */ 4, /* src ch */ 2, /* out ch */ 2,
            SourceSampleFormat::Int16, /* silent */ false, /* include mono */ true,
            /* include peak */ true,
        );
        assert_eq!(pcm_to_i16(&pcm), vec![16384, -16384]);
        assert_eq!(mono.len(), 1);
        assert!(mono[0].abs() < 1e-6, "mono {} should be ~0", mono[0]);
        assert!(peak.abs() < 1e-6, "peak {peak} should be ~0");
    }

    #[test]
    fn decode_stereo_i16_half_level_peak() {
        // Frame [16384, 16384] averages to 16384 / 32768 = 0.5.
        let mut raw = Vec::new();
        raw.extend_from_slice(&16384i16.to_le_bytes());
        raw.extend_from_slice(&16384i16.to_le_bytes());
        let (pcm, mono, peak) =
            decode_packet_to_pcm_and_mono(&raw, 4, 2, 2, SourceSampleFormat::Int16, false, true, true);
        assert_eq!(pcm_to_i16(&pcm), vec![16384, 16384]);
        assert_eq!(mono.len(), 1);
        assert!((mono[0] - 0.5).abs() < 1e-6, "mono {} != 0.5", mono[0]);
        assert!((peak - 0.5).abs() < 1e-6, "peak {peak} != 0.5");
    }

    #[test]
    fn decode_float_mono_source() {
        // Float source, 1 channel: values [0.75, -0.5] across two frames.
        let mut raw = Vec::new();
        raw.extend_from_slice(&0.75f32.to_le_bytes());
        raw.extend_from_slice(&(-0.5f32).to_le_bytes());
        let (pcm, mono, peak) =
            decode_packet_to_pcm_and_mono(&raw, 4, 1, 1, SourceSampleFormat::Float32, false, true, true);
        // mono equals the source samples for a single channel.
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 0.75).abs() < 1e-6, "mono0 {}", mono[0]);
        assert!((mono[1] - (-0.5)).abs() < 1e-6, "mono1 {}", mono[1]);
        assert!((peak - 0.75).abs() < 1e-6, "peak {peak} != 0.75");
        // PCM matches float_sample_to_i16 of each sample.
        assert_eq!(
            pcm_to_i16(&pcm),
            vec![float_sample_to_i16(0.75), float_sample_to_i16(-0.5)]
        );
    }

    #[test]
    fn decode_silent_returns_zeroed_without_reading_raw() {
        // Pass junk raw of the right length (2 frames * 4 bytes); must be ignored.
        let raw = vec![0xABu8; 8];
        let (pcm, mono, peak) =
            decode_packet_to_pcm_and_mono(&raw, 4, 2, 2, SourceSampleFormat::Int16, true, true, true);
        // 2 frames * 2 output channels * 2 bytes = 8 zero bytes.
        assert_eq!(pcm, vec![0u8; 8]);
        assert_eq!(mono, vec![0.0f32, 0.0f32]);
        assert_eq!(peak, 0.0);
    }

    #[test]
    fn decode_channel_clamp_mono_source_stereo_output() {
        // 1 source channel but 2 output channels: both output channels map to ch 0.
        let mut raw = Vec::new();
        raw.extend_from_slice(&8192i16.to_le_bytes());
        let (pcm, mono, peak) =
            decode_packet_to_pcm_and_mono(&raw, 2, 1, 2, SourceSampleFormat::Int16, false, true, true);
        // Both output channels carry the single source sample.
        assert_eq!(pcm_to_i16(&pcm), vec![8192, 8192]);
        assert_eq!(mono.len(), 1);
        assert!((mono[0] - 0.25).abs() < 1e-6, "mono {} != 0.25", mono[0]);
        assert!((peak - 0.25).abs() < 1e-6, "peak {peak} != 0.25");
    }

    #[test]
    fn decode_can_skip_mono_while_preserving_peak_for_system_audio() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&16384i16.to_le_bytes());
        raw.extend_from_slice(&16384i16.to_le_bytes());
        let (pcm, mono, peak) =
            decode_packet_to_pcm_and_mono(&raw, 4, 2, 2, SourceSampleFormat::Int16, false, false, true);
        assert_eq!(pcm_to_i16(&pcm), vec![16384, 16384]);
        assert!(mono.is_empty());
        assert!((peak - 0.5).abs() < 1e-6, "peak {peak} != 0.5");
    }

    #[test]
    fn decode_empty_inputs_return_empties() {
        // Zero bytes-per-frame guard.
        let (pcm, mono, peak) =
            decode_packet_to_pcm_and_mono(&[0u8; 4], 0, 2, 2, SourceSampleFormat::Int16, false, true, true);
        assert!(pcm.is_empty() && mono.is_empty() && peak == 0.0);
        // Fewer bytes than one frame -> frame_count == 0.
        let (pcm, mono, peak) =
            decode_packet_to_pcm_and_mono(&[0u8; 2], 4, 2, 2, SourceSampleFormat::Int16, false, true, true);
        assert!(pcm.is_empty() && mono.is_empty() && peak == 0.0);
    }
}
