//! Windows WASAPI microphone-capture backend.
//!
//! Captures the **default capture endpoint** (shared mode) with WASAPI, converts
//! the mix-format PCM to interleaved 16-bit little-endian PCM, and encodes it to
//! AAC inside a playable `.m4a` via the Media Foundation sink writer in
//! `capture-writers` (`WindowsAacM4aSinkWriter`). This is the Windows half of the
//! cross-platform microphone path; it mirrors the macOS AVFoundation backend's
//! externally-visible contract ([`crate::AudioCaptureSession`] +
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
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use capture_types::CaptureErrorResponse;
use capture_writers::WindowsAacM4aSinkWriter;
use wasapi::{
    get_default_device, initialize_mta, Direction, SampleType, StreamMode, WaveFormat,
};
use windows::Win32::Media::MediaFoundation::{MFShutdown, MFStartup, MFSTARTUP_FULL, MF_VERSION};

use crate::{AudioCaptureSession, MicrophoneOutputFinalization};

/// 100ns ticks in one second (Media Foundation time unit).
const TICKS_PER_SECOND: i64 = 10_000_000;

/// Poll cadence for draining WASAPI packets.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Shared-mode device buffer duration request (100ns units); 0 lets the engine
/// pick its default period.
const SHARED_BUFFER_DURATION_HNS: i64 = 0;

/// AAC supports up to 2 channels; clamp wider endpoints down to stereo.
const MAX_AAC_CHANNELS: u16 = 2;

// ---------------------------------------------------------------------------
// Support gate
// ---------------------------------------------------------------------------

/// True iff a default WASAPI capture endpoint exists.
pub fn microphone_capture_supported() -> bool {
    // The probe touches COM: `initialize_mta` calls `CoInitializeEx(MTA)`, which
    // joins the *calling* thread to a multithreaded apartment. This gate runs on
    // the app's main thread during startup (status-bar setup), and that thread
    // must stay free for `tao` to `OleInitialize` it as a single-threaded
    // apartment when it creates the first window — mixing the two panics with
    // `RPC_E_CHANGED_MODE`. So run the probe on a throwaway thread, never the
    // caller's, and cache the (process-stable) result.
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        std::thread::spawn(|| {
            // COM must be initialized on this thread before touching the device
            // enumerator. The throwaway thread's apartment is torn down when it
            // exits, so the MTA never leaks back to the caller.
            let _ = initialize_mta();
            get_default_device(&Direction::Capture).is_ok()
        })
        .join()
        .unwrap_or(false)
    })
}

// ---------------------------------------------------------------------------
// Control messages + shared state
// ---------------------------------------------------------------------------

enum Message {
    /// Finalize the current segment and begin writing the next at `output_path`.
    Rotate {
        output_path: PathBuf,
        reply: Sender<Result<MicrophoneOutputFinalization, CaptureErrorResponse>>,
    },
    /// Finalize the final segment and tear the session down.
    Stop {
        reply: Sender<Result<MicrophoneOutputFinalization, CaptureErrorResponse>>,
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
    stopped: bool,
}

impl std::fmt::Debug for WasapiMicrophoneCaptureSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasapiMicrophoneCaptureSession")
            .field("live", &self.shared.live.load(Ordering::Relaxed))
            .field("stopped", &self.stopped)
            .finish_non_exhaustive()
    }
}

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

    fn send_stop(&mut self) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        if self.stopped {
            return Ok(MicrophoneOutputFinalization {
                source_file: None,
                output_file: None,
                speech_detected: false,
                trim_start_offset_ms: 0,
                discard_reason: None,
            });
        }
        self.stopped = true;
        self.shared.live.store(false, Ordering::Relaxed);

        let (reply_tx, reply_rx) = mpsc::channel();
        if self.sender.send(Message::Stop { reply: reply_tx }).is_err() {
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

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Drop for WasapiMicrophoneCaptureSession {
    fn drop(&mut self) {
        let _ = self.send_stop();
    }
}

// ---------------------------------------------------------------------------
// Session factory
// ---------------------------------------------------------------------------

/// Start a WASAPI microphone capture session writing the first segment to
/// `output_file`. `device_id` is accepted for API parity but ignored — only the
/// default capture endpoint is captured for now.
pub fn start_wasapi_microphone_capture_session_for_file(
    output_file: &str,
    _device_id: Option<&str>,
) -> Result<WasapiMicrophoneCaptureSession, CaptureErrorResponse> {
    let output_path = PathBuf::from(output_file);
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| CaptureErrorResponse {
                code: "io_error".to_string(),
                message: format!(
                    "Failed to create microphone capture directory {}: {e}",
                    parent.display()
                ),
            })?;
        }
    }

    let shared = Arc::new(SharedState::default());
    let (sender, receiver) = mpsc::channel::<Message>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), CaptureErrorResponse>>();

    let thread_shared = Arc::clone(&shared);
    let join_handle = std::thread::Builder::new()
        .name("windows-microphone".to_string())
        .spawn(move || {
            capture_thread_main(output_path, receiver, thread_shared, ready_tx);
        })
        .map_err(|e| CaptureErrorResponse {
            code: "capture_thread_spawn_failed".to_string(),
            message: format!("Failed to spawn Windows microphone capture thread: {e}"),
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
        stopped: false,
    })
}

// ---------------------------------------------------------------------------
// Capture thread
// ---------------------------------------------------------------------------

fn capture_thread_main(
    first_output_path: PathBuf,
    receiver: Receiver<Message>,
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

    match CaptureEngine::new(&first_output_path) {
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
            Ok(Message::Rotate {
                output_path,
                reply,
            }) => {
                let result = engine.rotate(&output_path);
                let _ = reply.send(result);
            }
            Ok(Message::Stop { reply }) => {
                shared.live.store(false, Ordering::Relaxed);
                let result = engine.stop();
                let _ = reply.send(result);
                break;
            }
            Err(mpsc::TryRecvError::Empty) => {
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                // Handle dropped without an explicit stop; finalize best-effort.
                let _ = engine.stop();
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
    capture_client: wasapi::AudioCaptureClient,
    /// Mix-format sample rate (Hz) used both for client init and MF timing.
    sample_rate_hz: u32,
    /// Channel count delivered by WASAPI (the mix-format channel count).
    source_channels: u16,
    /// Channel count written to the AAC stream (clamped to <= 2).
    output_channels: u16,
    /// Whether the WASAPI mix format is IEEE float (vs. 16-bit int).
    source_is_float: bool,
    /// Bytes per WASAPI source frame (block align).
    source_bytes_per_frame: usize,
    /// Reusable raw-capture scratch buffer.
    raw: VecDeque<u8>,

    /// Active segment writer; `None` once stopped.
    writer: Option<WindowsAacM4aSinkWriter>,
    /// Filesystem path of the active segment.
    current_path: PathBuf,
    /// Frames appended to the active segment (for timing + empty detection).
    frames_in_segment: u64,
    /// First fatal error recorded for the active segment.
    failed: bool,
}

impl CaptureEngine {
    fn new(output_path: &Path) -> Result<Self, CaptureErrorResponse> {
        let device = get_default_device(&Direction::Capture)
            .map_err(|e| wasapi_error("get_default_device(Capture) failed", &e))?;
        let mut audio_client = device
            .get_iaudioclient()
            .map_err(|e| wasapi_error("get_iaudioclient failed", &e))?;
        let mix_format = audio_client
            .get_mixformat()
            .map_err(|e| wasapi_error("get_mixformat failed", &e))?;

        let sample_rate_hz = mix_format.get_samplespersec();
        let source_channels = mix_format.get_nchannels();
        let source_is_float = matches!(
            mix_format.get_subformat(),
            Ok(SampleType::Float)
        );
        let output_channels = source_channels.min(MAX_AAC_CHANNELS).max(1);

        audio_client
            .initialize_client(
                &mix_format,
                &Direction::Capture,
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

        let writer =
            WindowsAacM4aSinkWriter::create(output_path, sample_rate_hz, output_channels)?;

        Ok(Self {
            capture_client,
            sample_rate_hz,
            source_channels,
            output_channels,
            source_is_float,
            source_bytes_per_frame: WaveFormat::get_blockalign(&mix_format) as usize,
            raw: VecDeque::new(),
            writer: Some(writer),
            current_path: output_path.to_path_buf(),
            frames_in_segment: 0,
            failed: false,
        })
    }

    /// Drain all queued WASAPI packets and append them to the active segment.
    fn pump(&mut self) -> Result<(), CaptureErrorResponse> {
        if self.failed || self.writer.is_none() {
            return Ok(());
        }
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
                    self.append_raw_frames(&raw, info.flags.silent)?;
                }
                _ => break,
            }
        }
        Ok(())
    }

    /// Convert a raw WASAPI packet (source mix format) to interleaved 16-bit LE
    /// PCM at the output channel count and append it to the active segment.
    fn append_raw_frames(
        &mut self,
        raw: &[u8],
        silent: bool,
    ) -> Result<(), CaptureErrorResponse> {
        if raw.is_empty() || self.source_bytes_per_frame == 0 {
            return Ok(());
        }
        let frame_count = raw.len() / self.source_bytes_per_frame;
        if frame_count == 0 {
            return Ok(());
        }

        let source_channels = self.source_channels.max(1) as usize;
        let output_channels = self.output_channels as usize;
        let mut pcm = Vec::with_capacity(frame_count * output_channels * 2);

        if silent {
            // Honor the silent flag without trusting the (possibly stale) buffer.
            pcm.resize(frame_count * output_channels * 2, 0);
        } else {
            let bytes_per_sample = if self.source_is_float { 4 } else { 2 };
            for frame in 0..frame_count {
                let frame_base = frame * self.source_bytes_per_frame;
                for out_ch in 0..output_channels {
                    // Map output channel to source channel; both are <= source.
                    let src_ch = out_ch.min(source_channels - 1);
                    let sample_off = frame_base + src_ch * bytes_per_sample;
                    let value = if self.source_is_float {
                        let bytes = [
                            raw[sample_off],
                            raw[sample_off + 1],
                            raw[sample_off + 2],
                            raw[sample_off + 3],
                        ];
                        float_sample_to_i16(f32::from_le_bytes(bytes))
                    } else {
                        let bytes = [raw[sample_off], raw[sample_off + 1]];
                        i16::from_le_bytes(bytes)
                    };
                    pcm.extend_from_slice(&value.to_le_bytes());
                }
            }
        }

        let sample_time_100ns = frames_to_ticks(self.frames_in_segment, self.sample_rate_hz);
        let duration_100ns = frames_to_ticks(frame_count as u64, self.sample_rate_hz);

        if let Some(writer) = self.writer.as_mut() {
            writer.append_pcm_s16(&pcm, sample_time_100ns, duration_100ns)?;
            self.frames_in_segment += frame_count as u64;
        }
        Ok(())
    }

    /// Finalize the active segment and build its finalization record.
    fn finalize_segment(&mut self) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        let closed_path = self.current_path.to_string_lossy().to_string();
        let had_frames = self.frames_in_segment > 0;

        if let Some(writer) = self.writer.take() {
            writer.finalize()?;
        }

        if had_frames {
            Ok(MicrophoneOutputFinalization {
                source_file: Some(closed_path.clone()),
                output_file: Some(closed_path),
                speech_detected: false,
                trim_start_offset_ms: 0,
                discard_reason: None,
            })
        } else {
            Ok(MicrophoneOutputFinalization {
                source_file: Some(closed_path),
                output_file: None,
                speech_detected: false,
                trim_start_offset_ms: 0,
                discard_reason: Some("no_audio_samples".to_string()),
            })
        }
    }

    /// Finalize the current segment and begin a fresh one at `output_path`.
    fn rotate(
        &mut self,
        output_path: &Path,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        let finalization = self.finalize_segment()?;

        let writer =
            WindowsAacM4aSinkWriter::create(output_path, self.sample_rate_hz, self.output_channels)?;
        self.writer = Some(writer);
        self.current_path = output_path.to_path_buf();
        self.frames_in_segment = 0;

        Ok(finalization)
    }

    /// Finalize the current segment and tear down capture.
    fn stop(&mut self) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        self.finalize_segment()
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

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

fn wasapi_error(context: &str, error: &wasapi::WasapiError) -> CaptureErrorResponse {
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
