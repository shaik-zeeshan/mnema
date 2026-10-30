use capture_types::{
    CaptureErrorResponse, CapturePermissionState, MicrophoneControllerState, MicrophoneDevice,
    MicrophoneDisconnectPolicy, MicrophonePreference, MicrophonePreferenceMode,
};

#[cfg(target_os = "macos")]
use capture_writers::{
    audio_sample_format_is_compatible_with_writer_format,
    create_audio_asset_writer_for_sample_format, derive_audio_activity_level_from_sample_buf,
    derive_audio_sample_format_from_sample_buf,
    finalize_microphone_output_context as writers_finalize_microphone_output_context,
    finish_audio_asset_writer_discarding_inactivity_tail, record_audio_writer_tail_activity,
    set_audio_writer_activity_threshold, set_audio_writer_inactivity_tail_trim_seconds,
    try_append_audio_sample_to_writer_with_activity_override, AudioAssetWriterState,
    AudioSampleAppendDisposition, AudioSampleFormat,
};

#[cfg(target_os = "macos")]
use cidre::{av, dispatch};
#[cfg(target_os = "macos")]
use cidre::{ns, objc};
#[cfg(target_os = "macos")]
use std::collections::VecDeque;
#[cfg(target_os = "macos")]
use std::ffi::c_void;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
#[cfg(target_os = "macos")]
use std::sync::mpsc;
#[cfg(target_os = "macos")]
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
mod vad_trim;
#[cfg(target_os = "macos")]
use vad_trim::{
    finalize_microphone_vad_boundary_trim, microphone_output_finalization_for_context,
    microphone_resume_stamp_correction_ms, take_microphone_vad_speech_boundaries_for_context,
};

#[cfg(target_os = "macos")]
static LAST_MICROPHONE_ACTIVITY_UNIX_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "macos")]
static LAST_MICROPHONE_ACTIVITY_MONOTONIC_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "macos")]
static LAST_MICROPHONE_ACTIVITY_LEVEL_BITS: AtomicU32 = AtomicU32::new(0);
#[cfg(target_os = "macos")]
static LAST_MICROPHONE_ACTIVITY_WINDOW_PEAK_LEVEL_BITS: AtomicU32 = AtomicU32::new(0);
#[cfg(target_os = "macos")]
static LAST_MICROPHONE_ACTIVITY_WINDOW_SAMPLE_COUNT: AtomicU32 = AtomicU32::new(0);
#[cfg(target_os = "macos")]
static MICROPHONE_VAD_TAIL_SPEECH_SEQUENCE: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "macos")]
static MICROPHONE_VAD_BOUNDARY_TRIM_DISABLED_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "macos")]
#[derive(Debug, Default, Clone, Copy)]
struct MicrophoneVadSpeechBoundaryState {
    first_start_secs: Option<f64>,
    last_end_secs: Option<f64>,
    detected_speech: bool,
    invalid_timing: bool,
}

#[cfg(target_os = "macos")]
fn microphone_vad_speech_boundaries() -> &'static Mutex<MicrophoneVadSpeechBoundaryState> {
    static BOUNDARIES: OnceLock<Mutex<MicrophoneVadSpeechBoundaryState>> = OnceLock::new();
    BOUNDARIES.get_or_init(|| Mutex::new(MicrophoneVadSpeechBoundaryState::default()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophoneInactivityTailTrimActivityMode {
    PeakLevel,
    VadSpeech,
}

pub const MICROPHONE_VAD_PCM_SAMPLE_RATE_HZ: u32 = 16_000;
pub const MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT: usize = 320;
/// 10 s of 20 ms frames (~320 KB). The capture loop drains once a second, and
/// a rotation tick (trim plus writer finalization) can run several seconds;
/// any frame dropped here disables boundary trimming for the whole file.
#[cfg(target_os = "macos")]
const MAX_MICROPHONE_VAD_PCM_FRAMES: usize = 500;

#[derive(Debug, Clone, PartialEq)]
pub struct MicrophoneVadPcmFrame {
    pub sample_rate_hz: u32,
    pub captured_at_unix_ms: u64,
    pub normalized_peak_level: f32,
    pub media_start_secs: Option<f64>,
    pub media_end_secs: Option<f64>,
    pub samples: Vec<i16>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MicrophoneVadSpeechEvent {
    pub media_start_secs: Option<f64>,
    pub media_end_secs: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicrophoneOutputFinalization {
    pub source_file: Option<String>,
    pub output_file: Option<String>,
    pub speech_detected: bool,
    /// Where the output file's first sample sits relative to the start its path
    /// is stamped with: later when leading silence was trimmed, earlier when a
    /// resume flushed audio queued before it.
    pub start_offset_ms: i64,
    pub discard_reason: Option<String>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
struct MicrophoneVadPcmFeedState {
    source_format: Option<MicrophoneVadSourceFormat>,
    pending_source_samples: Vec<f32>,
    pending_source_start_secs: Option<f64>,
    output_frames: VecDeque<MicrophoneVadPcmFrame>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq)]
struct MicrophoneVadSourceFormat {
    sample_rate_hz: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    bytes_per_frame: u32,
    format_id: u32,
    format_flags: u32,
}

#[cfg(target_os = "macos")]
fn microphone_activity_monotonic_epoch() -> &'static Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now)
}

#[cfg(target_os = "macos")]
fn now_microphone_activity_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
fn now_microphone_activity_monotonic_ms() -> u64 {
    microphone_activity_monotonic_epoch()
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(target_os = "macos")]
fn now_microphone_activity_marker_ms() -> u64 {
    now_microphone_activity_monotonic_ms().saturating_add(1)
}

#[cfg(target_os = "macos")]
fn store_microphone_activity(level: f32, now_monotonic_ms: u64, now_unix_ms: u64) {
    let level = level.clamp(0.0, 1.0);
    LAST_MICROPHONE_ACTIVITY_LEVEL_BITS.store(level.to_bits(), Ordering::Relaxed);
    LAST_MICROPHONE_ACTIVITY_MONOTONIC_MS.store(now_monotonic_ms, Ordering::Relaxed);
    LAST_MICROPHONE_ACTIVITY_UNIX_MS.store(now_unix_ms, Ordering::Relaxed);
    record_microphone_activity_window_peak(level);
}

#[cfg(target_os = "macos")]
fn record_microphone_activity_window_peak(level: f32) {
    LAST_MICROPHONE_ACTIVITY_WINDOW_SAMPLE_COUNT.fetch_add(1, Ordering::Relaxed);

    let level_bits = level.to_bits();
    let mut observed_bits = LAST_MICROPHONE_ACTIVITY_WINDOW_PEAK_LEVEL_BITS.load(Ordering::Relaxed);
    while f32::from_bits(observed_bits) < level {
        match LAST_MICROPHONE_ACTIVITY_WINDOW_PEAK_LEVEL_BITS.compare_exchange_weak(
            observed_bits,
            level_bits,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(next_bits) => observed_bits = next_bits,
        }
    }
}

#[cfg(target_os = "macos")]
fn maybe_track_microphone_activity(sample_buf: &cidre::cm::SampleBuf) {
    let Some(level) = derive_audio_activity_level_from_sample_buf(sample_buf) else {
        return;
    };

    store_microphone_activity(
        level,
        now_microphone_activity_marker_ms(),
        now_microphone_activity_unix_ms(),
    );
}

#[cfg(target_os = "macos")]
fn microphone_vad_pcm_feed() -> &'static Mutex<MicrophoneVadPcmFeedState> {
    static FEED: OnceLock<Mutex<MicrophoneVadPcmFeedState>> = OnceLock::new();
    FEED.get_or_init(|| Mutex::new(MicrophoneVadPcmFeedState::default()))
}

#[cfg(target_os = "macos")]
pub fn reset_microphone_vad_pcm_feed() {
    let mut feed = microphone_vad_pcm_feed()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *feed = MicrophoneVadPcmFeedState::default();
}

#[cfg(not(target_os = "macos"))]
pub fn reset_microphone_vad_pcm_feed() {}

#[cfg(target_os = "macos")]
fn reset_microphone_vad_pcm_partial_frame() {
    let mut feed = microphone_vad_pcm_feed()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    feed.pending_source_samples.clear();
    feed.pending_source_start_secs = None;
}

#[cfg(target_os = "macos")]
pub fn reset_microphone_vad_tail_activity() {
    MICROPHONE_VAD_TAIL_SPEECH_SEQUENCE.store(0, Ordering::Relaxed);
    MICROPHONE_VAD_BOUNDARY_TRIM_DISABLED_SEQUENCE.store(0, Ordering::Relaxed);
    let mut boundaries = microphone_vad_speech_boundaries()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *boundaries = MicrophoneVadSpeechBoundaryState::default();
}

#[cfg(not(target_os = "macos"))]
pub fn reset_microphone_vad_tail_activity() {}

#[cfg(target_os = "macos")]
pub fn record_microphone_vad_tail_speech() {
    record_microphone_vad_speech_event(MicrophoneVadSpeechEvent {
        media_start_secs: None,
        media_end_secs: None,
    });
}

#[cfg(not(target_os = "macos"))]
pub fn record_microphone_vad_tail_speech() {}

#[cfg(target_os = "macos")]
pub fn record_microphone_vad_speech_event(_event: MicrophoneVadSpeechEvent) {
    MICROPHONE_VAD_TAIL_SPEECH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut boundaries = microphone_vad_speech_boundaries()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    boundaries.detected_speech = true;
    match (_event.media_start_secs, _event.media_end_secs) {
        (Some(start), Some(end)) if start.is_finite() && end.is_finite() && end >= start => {
            boundaries.first_start_secs = Some(
                boundaries
                    .first_start_secs
                    .map(|current| current.min(start))
                    .unwrap_or(start),
            );
            boundaries.last_end_secs = Some(
                boundaries
                    .last_end_secs
                    .map(|current| current.max(end))
                    .unwrap_or(end),
            );
        }
        _ => boundaries.invalid_timing = true,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn record_microphone_vad_speech_event(_event: MicrophoneVadSpeechEvent) {}

#[cfg(target_os = "macos")]
pub fn disable_microphone_vad_boundary_trim_for_current_output() {
    MICROPHONE_VAD_BOUNDARY_TRIM_DISABLED_SEQUENCE.fetch_add(1, Ordering::Relaxed);
}

#[cfg(not(target_os = "macos"))]
pub fn disable_microphone_vad_boundary_trim_for_current_output() {}

#[cfg(target_os = "macos")]
fn current_microphone_vad_tail_speech_sequence() -> u64 {
    MICROPHONE_VAD_TAIL_SPEECH_SEQUENCE.load(Ordering::Relaxed)
}

#[cfg(target_os = "macos")]
fn current_microphone_vad_boundary_trim_disabled_sequence() -> u64 {
    MICROPHONE_VAD_BOUNDARY_TRIM_DISABLED_SEQUENCE.load(Ordering::Relaxed)
}

#[cfg(all(target_os = "macos", test))]
fn current_microphone_vad_speech_boundaries() -> MicrophoneVadSpeechBoundaryState {
    *microphone_vad_speech_boundaries()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(target_os = "macos")]
fn take_microphone_vad_speech_boundaries() -> MicrophoneVadSpeechBoundaryState {
    let mut boundaries = microphone_vad_speech_boundaries()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let current = *boundaries;
    *boundaries = MicrophoneVadSpeechBoundaryState::default();
    current
}

#[cfg(target_os = "macos")]
pub fn take_microphone_vad_pcm_frames(max_frames: usize) -> Vec<MicrophoneVadPcmFrame> {
    let mut feed = microphone_vad_pcm_feed()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let frame_count = max_frames.min(feed.output_frames.len());
    feed.output_frames.drain(..frame_count).collect()
}

#[cfg(not(target_os = "macos"))]
pub fn take_microphone_vad_pcm_frames(_max_frames: usize) -> Vec<MicrophoneVadPcmFrame> {
    Vec::new()
}

#[cfg(target_os = "macos")]
pub fn microphone_vad_pcm_frame_count() -> usize {
    let feed = microphone_vad_pcm_feed()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    feed.output_frames.len()
}

#[cfg(not(target_os = "macos"))]
pub fn microphone_vad_pcm_frame_count() -> usize {
    0
}

#[cfg(target_os = "macos")]
fn maybe_feed_microphone_vad_pcm(sample_buf: &cidre::cm::SampleBuf) {
    if !sample_buf.data_is_ready() {
        return;
    }

    let Some(sample_format) = derive_audio_sample_format_from_sample_buf(sample_buf) else {
        return;
    };
    let mut audio_buf_list = cidre::cat::AudioBufListN::default();
    let Ok(audio_buf_list) = sample_buf.audio_buf_list_n(&mut audio_buf_list) else {
        return;
    };

    let Some(samples) =
        mono_pcm_samples_from_audio_buffers(audio_buf_list.list.buffers(), sample_format)
    else {
        return;
    };

    let mut feed = microphone_vad_pcm_feed()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    feed_microphone_vad_pcm_samples(
        &mut feed,
        sample_format,
        now_microphone_activity_unix_ms(),
        sample_buf_start_secs(sample_buf),
        &samples,
    );
}

#[cfg(target_os = "macos")]
fn sample_buf_start_secs(sample_buf: &cidre::cm::SampleBuf) -> Option<f64> {
    let pts = sample_buf.pts();
    pts.is_numeric()
        .then(|| pts.as_secs())
        .filter(|value| value.is_finite())
}

#[cfg(target_os = "macos")]
pub fn reset_last_microphone_activity_unix_ms() {
    LAST_MICROPHONE_ACTIVITY_UNIX_MS.store(0, Ordering::Relaxed);
    LAST_MICROPHONE_ACTIVITY_MONOTONIC_MS.store(0, Ordering::Relaxed);
    LAST_MICROPHONE_ACTIVITY_LEVEL_BITS.store(0, Ordering::Relaxed);
    LAST_MICROPHONE_ACTIVITY_WINDOW_PEAK_LEVEL_BITS.store(0, Ordering::Relaxed);
    LAST_MICROPHONE_ACTIVITY_WINDOW_SAMPLE_COUNT.store(0, Ordering::Relaxed);
}

#[cfg(not(target_os = "macos"))]
pub fn reset_last_microphone_activity_unix_ms() {}

#[cfg(target_os = "macos")]
pub fn last_microphone_activity_unix_ms() -> Option<u64> {
    let ts = LAST_MICROPHONE_ACTIVITY_UNIX_MS.load(Ordering::Relaxed);
    (ts > 0).then_some(ts)
}

#[cfg(not(target_os = "macos"))]
pub fn last_microphone_activity_unix_ms() -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
pub fn microphone_activity_idle_ms() -> Option<u64> {
    let ts = LAST_MICROPHONE_ACTIVITY_MONOTONIC_MS.load(Ordering::Relaxed);
    (ts > 0).then_some(now_microphone_activity_marker_ms().saturating_sub(ts))
}

#[cfg(not(target_os = "macos"))]
pub fn microphone_activity_idle_ms() -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
pub fn microphone_activity_level() -> Option<f32> {
    last_microphone_activity_unix_ms()
        .map(|_| f32::from_bits(LAST_MICROPHONE_ACTIVITY_LEVEL_BITS.load(Ordering::Relaxed)))
}

#[cfg(target_os = "macos")]
pub fn take_microphone_activity_window_peak_level() -> Option<f32> {
    let sample_count = LAST_MICROPHONE_ACTIVITY_WINDOW_SAMPLE_COUNT.swap(0, Ordering::Relaxed);
    let level_bits = LAST_MICROPHONE_ACTIVITY_WINDOW_PEAK_LEVEL_BITS.swap(0, Ordering::Relaxed);
    (sample_count > 0).then_some(f32::from_bits(level_bits))
}

#[cfg(target_os = "macos")]
pub fn peek_microphone_activity_window_peak_level() -> Option<f32> {
    let sample_count = LAST_MICROPHONE_ACTIVITY_WINDOW_SAMPLE_COUNT.load(Ordering::Relaxed);
    let level_bits = LAST_MICROPHONE_ACTIVITY_WINDOW_PEAK_LEVEL_BITS.load(Ordering::Relaxed);
    (sample_count > 0).then_some(f32::from_bits(level_bits))
}

#[cfg(not(target_os = "macos"))]
pub fn take_microphone_activity_window_peak_level() -> Option<f32> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn peek_microphone_activity_window_peak_level() -> Option<f32> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn microphone_activity_level() -> Option<f32> {
    None
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct MicrophoneOutputContext {
    writer: Option<AudioAssetWriterState>,
    output_url: Option<cidre::arc::R<cidre::ns::Url>>,
    source_output_file: Option<String>,
    output_file: Option<String>,
    first_error: Option<CaptureErrorResponse>,
    format_state: MicFormatStabilityState,
    pending_samples: VecDeque<BufferedMicSample>,
    inactivity_tail_trim_seconds: u64,
    activity_threshold: f32,
    tail_activity_mode: MicrophoneInactivityTailTrimActivityMode,
    observed_vad_tail_speech_sequence: u64,
    boundary_trim_enabled: bool,
    boundary_trim_disabled: bool,
    observed_boundary_trim_disabled_sequence: u64,
    first_speech_start_secs: Option<f64>,
    last_speech_end_secs: Option<f64>,
    detected_vad_speech: bool,
    /// Host-clock instant the file was opened by a resume — the moment its path
    /// is stamped with. `None` for a file stamped by the segment schedule.
    resumed_at_host_secs: Option<f64>,
    /// Seconds of queued audio the resume carried in, by sample count: how far
    /// before `resumed_at_host_secs` the file's first sample can honestly sit.
    resume_carried_secs: f64,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default, Clone, Copy)]
struct MicFormatStabilityState {
    observed_format_count: u32,
    candidate_format: Option<AudioSampleFormat>,
    candidate_format_streak: u32,
    stable_format: Option<AudioSampleFormat>,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct BufferedMicSample {
    sample_buf: cidre::arc::R<cidre::cm::SampleBuf>,
    format: AudioSampleFormat,
}

#[cfg(target_os = "macos")]
const FORMAT_STABILITY_REQUIRED_CONSECUTIVE: u32 = 3;
#[cfg(target_os = "macos")]
const FORMAT_STABILITY_MIN_OBSERVED: u32 = 5;
#[cfg(target_os = "macos")]
// Keep enough queued microphone samples to bridge the ~1s inactivity poll
// interval plus writer startup backpressure, without carrying several seconds
// of probe-only silence into the resumed file.
const MAX_PENDING_MIC_SAMPLES: usize = 128;

#[cfg(target_os = "macos")]
fn record_observed_audio_format(
    context: &mut MicrophoneOutputContext,
    sample_format: AudioSampleFormat,
) {
    let was_stable = context.format_state.stable_format.is_some();
    observe_microphone_format(&mut context.format_state, sample_format);

    if !was_stable {
        let Some(stable_format) = context.format_state.stable_format else {
            return;
        };
        capture_runtime::debug_log!(
            "[capture-microphone] sample_format_stabilized observed={} streak={} sample_rate_hz={} channels={} bits_per_channel={} bytes_per_frame={} format_id={} format_flags={}",
            context.format_state.observed_format_count,
            context.format_state.candidate_format_streak,
            stable_format.sample_rate_hz,
            stable_format.channels_per_frame,
            stable_format.bits_per_channel,
            stable_format.bytes_per_frame,
            stable_format.format_id,
            stable_format.format_flags,
        );
    }
}

#[cfg(target_os = "macos")]
fn fallback_microphone_format(context: &MicrophoneOutputContext) -> Option<AudioSampleFormat> {
    resolve_microphone_finalize_format(&context.format_state)
}

#[cfg(all(test, target_os = "macos"))]
mod tests;

#[cfg(target_os = "macos")]
fn observe_microphone_format(
    state: &mut MicFormatStabilityState,
    sample_format: AudioSampleFormat,
) {
    state.observed_format_count += 1;

    match state.candidate_format {
        Some(current_candidate) if current_candidate == sample_format => {
            state.candidate_format_streak += 1;
        }
        _ => {
            state.candidate_format = Some(sample_format);
            state.candidate_format_streak = 1;
        }
    }

    if state.stable_format.is_none()
        && state.observed_format_count >= FORMAT_STABILITY_MIN_OBSERVED
        && state.candidate_format_streak >= FORMAT_STABILITY_REQUIRED_CONSECUTIVE
    {
        state.stable_format = state.candidate_format;
    }
}

#[cfg(target_os = "macos")]
fn resolve_microphone_live_format(state: &MicFormatStabilityState) -> Option<AudioSampleFormat> {
    state.stable_format.or(state.candidate_format)
}

#[cfg(target_os = "macos")]
fn resolve_microphone_finalize_format(
    state: &MicFormatStabilityState,
) -> Option<AudioSampleFormat> {
    state.stable_format.or(state.candidate_format)
}

#[cfg(target_os = "macos")]
fn feed_microphone_vad_pcm_samples(
    state: &mut MicrophoneVadPcmFeedState,
    sample_format: AudioSampleFormat,
    captured_at_unix_ms: u64,
    media_start_secs: Option<f64>,
    samples: &[f32],
) {
    if samples.is_empty() {
        return;
    }

    let Some(source_format) = MicrophoneVadSourceFormat::from_sample_format(sample_format) else {
        state.pending_source_samples.clear();
        state.source_format = None;
        return;
    };

    if state.source_format != Some(source_format) {
        state.pending_source_samples.clear();
        state.pending_source_start_secs = None;
        state.source_format = Some(source_format);
    }

    if state.pending_source_samples.is_empty() {
        state.pending_source_start_secs = media_start_secs;
    }
    state.pending_source_samples.extend_from_slice(samples);

    let input_frame_count = vad_input_frame_count_for_output_frame(source_format.sample_rate_hz);
    while state.pending_source_samples.len() >= input_frame_count {
        let frame_media_start_secs = state.pending_source_start_secs;
        let frame_duration_secs = input_frame_count as f64 / source_format.sample_rate_hz as f64;
        let frame_media_end_secs = frame_media_start_secs.map(|start| start + frame_duration_secs);
        let output_samples = resample_microphone_vad_frame(
            &state.pending_source_samples[..input_frame_count],
            source_format.sample_rate_hz,
        );
        state.pending_source_samples.drain(..input_frame_count);
        state.pending_source_start_secs = frame_media_end_secs;

        if output_samples.len() != MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT {
            continue;
        }
        let normalized_peak_level = output_samples
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
            .clamp(0.0, 1.0);
        let samples = output_samples
            .into_iter()
            .map(normalized_microphone_vad_sample_to_i16)
            .collect();

        state.output_frames.push_back(MicrophoneVadPcmFrame {
            sample_rate_hz: MICROPHONE_VAD_PCM_SAMPLE_RATE_HZ,
            captured_at_unix_ms,
            normalized_peak_level,
            media_start_secs: frame_media_start_secs,
            media_end_secs: frame_media_end_secs,
            samples,
        });

        while state.output_frames.len() > MAX_MICROPHONE_VAD_PCM_FRAMES {
            let _ = state.output_frames.pop_front();
            // A frame the VAD never sees could hold speech, so the boundaries
            // are no longer the whole story: keep this output whole.
            disable_microphone_vad_boundary_trim_for_current_output();
        }
    }
}

#[cfg(target_os = "macos")]
impl MicrophoneVadSourceFormat {
    fn from_sample_format(sample_format: AudioSampleFormat) -> Option<Self> {
        let sample_rate_hz = rounded_positive_sample_rate(sample_format.sample_rate_hz)?;
        Some(Self {
            sample_rate_hz,
            channels_per_frame: sample_format.channels_per_frame,
            bits_per_channel: sample_format.bits_per_channel,
            bytes_per_frame: sample_format.bytes_per_frame,
            format_id: sample_format.format_id,
            format_flags: sample_format.format_flags,
        })
    }
}

#[cfg(target_os = "macos")]
fn rounded_positive_sample_rate(sample_rate_hz: f64) -> Option<u32> {
    if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 || sample_rate_hz > f64::from(u32::MAX)
    {
        return None;
    }

    Some(sample_rate_hz.round() as u32)
}

#[cfg(target_os = "macos")]
fn vad_input_frame_count_for_output_frame(source_sample_rate_hz: u32) -> usize {
    let numerator = source_sample_rate_hz as usize * MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT;
    numerator.div_ceil(MICROPHONE_VAD_PCM_SAMPLE_RATE_HZ as usize)
}

#[cfg(target_os = "macos")]
fn resample_microphone_vad_frame(samples: &[f32], source_sample_rate_hz: u32) -> Vec<f32> {
    if source_sample_rate_hz == MICROPHONE_VAD_PCM_SAMPLE_RATE_HZ {
        return samples
            .iter()
            .copied()
            .take(MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT)
            .collect();
    }

    let source_rate = source_sample_rate_hz as f64;
    let target_rate = MICROPHONE_VAD_PCM_SAMPLE_RATE_HZ as f64;
    let mut output = Vec::with_capacity(MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT);

    for output_index in 0..MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT {
        let source_pos = output_index as f64 * source_rate / target_rate;
        let lower_index = source_pos.floor() as usize;
        let upper_index = lower_index
            .saturating_add(1)
            .min(samples.len().saturating_sub(1));
        let fraction = (source_pos - lower_index as f64) as f32;
        let lower = samples.get(lower_index).copied().unwrap_or(0.0);
        let upper = samples.get(upper_index).copied().unwrap_or(lower);
        output.push((lower + (upper - lower) * fraction).clamp(-1.0, 1.0));
    }

    output
}

#[cfg(target_os = "macos")]
fn normalized_microphone_vad_sample_to_i16(sample: f32) -> i16 {
    let sample = sample.clamp(-1.0, 1.0);
    if sample >= 0.0 {
        (sample * i16::MAX as f32).round() as i16
    } else {
        (sample * i16::MAX as f32).round().max(i16::MIN as f32) as i16
    }
}

#[cfg(target_os = "macos")]
fn mono_pcm_samples_from_audio_buffers(
    buffers: &[cidre::cat::AudioBuf],
    sample_format: AudioSampleFormat,
) -> Option<Vec<f32>> {
    let format_id = cidre::cat::AudioFormat(sample_format.format_id);
    if format_id != cidre::cat::AudioFormat::LINEAR_PCM {
        return None;
    }

    let format_flags = cidre::cat::AudioFormatFlags(sample_format.format_flags);
    let is_packed = format_flags.contains(cidre::cat::AudioFormatFlags::IS_PACKED);
    let is_big_endian = format_flags.contains(cidre::cat::AudioFormatFlags::IS_BIG_ENDIAN);
    let bytes_per_sample = sample_format.bits_per_channel.saturating_add(7) / 8;
    let bytes_per_sample = bytes_per_sample as usize;
    let channel_count = sample_format.channels_per_frame as usize;

    if !is_packed || is_big_endian || bytes_per_sample == 0 || channel_count == 0 {
        return None;
    }

    let active_buffers: Vec<&cidre::cat::AudioBuf> = buffers
        .iter()
        .filter(|buffer| !buffer.data.is_null() && buffer.data_bytes_size > 0)
        .collect();

    match active_buffers.as_slice() {
        [] => None,
        [buffer] => mono_pcm_samples_from_interleaved_buffer(buffer, sample_format),
        _ => mono_pcm_samples_from_planar_buffers(&active_buffers, sample_format),
    }
}

#[cfg(target_os = "macos")]
fn mono_pcm_samples_from_interleaved_buffer(
    buffer: &cidre::cat::AudioBuf,
    sample_format: AudioSampleFormat,
) -> Option<Vec<f32>> {
    let bytes = audio_buffer_bytes(buffer)?;
    let bytes_per_sample = (sample_format.bits_per_channel.saturating_add(7) / 8) as usize;
    let channel_count = sample_format.channels_per_frame as usize;
    let bytes_per_frame = sample_format.bytes_per_frame as usize;
    let bytes_per_frame = bytes_per_frame.max(bytes_per_sample.saturating_mul(channel_count));

    if bytes_per_frame == 0 || bytes.len() < bytes_per_frame {
        return None;
    }

    let frame_count = bytes.len() / bytes_per_frame;
    let mut samples = Vec::with_capacity(frame_count);
    for frame_index in 0..frame_count {
        let frame_offset = frame_index * bytes_per_frame;
        let mut sum = 0.0_f32;
        for channel_index in 0..channel_count {
            let sample_offset = frame_offset + channel_index * bytes_per_sample;
            let sample = bytes.get(sample_offset..sample_offset + bytes_per_sample)?;
            sum += normalized_microphone_pcm_sample(sample, sample_format)?;
        }
        samples.push((sum / channel_count as f32).clamp(-1.0, 1.0));
    }

    Some(samples)
}

#[cfg(target_os = "macos")]
fn mono_pcm_samples_from_planar_buffers(
    buffers: &[&cidre::cat::AudioBuf],
    sample_format: AudioSampleFormat,
) -> Option<Vec<f32>> {
    let bytes_per_sample = (sample_format.bits_per_channel.saturating_add(7) / 8) as usize;
    if bytes_per_sample == 0 {
        return None;
    }

    let channel_bytes = buffers
        .iter()
        .map(|buffer| audio_buffer_bytes(buffer))
        .collect::<Option<Vec<_>>>()?;
    let frame_count = channel_bytes
        .iter()
        .map(|bytes| bytes.len() / bytes_per_sample)
        .min()?;
    if frame_count == 0 {
        return None;
    }

    let mut samples = Vec::with_capacity(frame_count);
    for frame_index in 0..frame_count {
        let mut sum = 0.0_f32;
        for bytes in &channel_bytes {
            let sample_offset = frame_index * bytes_per_sample;
            let sample = bytes.get(sample_offset..sample_offset + bytes_per_sample)?;
            sum += normalized_microphone_pcm_sample(sample, sample_format)?;
        }
        samples.push((sum / channel_bytes.len() as f32).clamp(-1.0, 1.0));
    }

    Some(samples)
}

#[cfg(target_os = "macos")]
fn audio_buffer_bytes(buffer: &cidre::cat::AudioBuf) -> Option<&[u8]> {
    if buffer.data.is_null() || buffer.data_bytes_size == 0 {
        return None;
    }

    Some(unsafe {
        std::slice::from_raw_parts(buffer.data as *const u8, buffer.data_bytes_size as usize)
    })
}

#[cfg(target_os = "macos")]
fn normalized_microphone_pcm_sample(
    sample: &[u8],
    sample_format: AudioSampleFormat,
) -> Option<f32> {
    let format_flags = cidre::cat::AudioFormatFlags(sample_format.format_flags);
    if format_flags.contains(cidre::cat::AudioFormatFlags::IS_FLOAT) {
        normalized_microphone_float_pcm_sample(sample, sample_format.bits_per_channel)
    } else if format_flags.contains(cidre::cat::AudioFormatFlags::IS_SIGNED_INTEGER) {
        normalized_microphone_signed_pcm_sample(sample, sample_format.bits_per_channel)
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn normalized_microphone_float_pcm_sample(sample: &[u8], bits_per_channel: u32) -> Option<f32> {
    let value = match bits_per_channel {
        32 if sample.len() >= 4 => f32::from_ne_bytes(sample[..4].try_into().ok()?),
        64 if sample.len() >= 8 => f64::from_ne_bytes(sample[..8].try_into().ok()?) as f32,
        _ => return None,
    };

    value.is_finite().then_some(value.clamp(-1.0, 1.0))
}

#[cfg(target_os = "macos")]
fn normalized_microphone_signed_pcm_sample(sample: &[u8], bits_per_channel: u32) -> Option<f32> {
    let value = match bits_per_channel {
        8 if !sample.is_empty() => sample[0] as i8 as f32 / i8::MAX as f32,
        16 if sample.len() >= 2 => {
            i16::from_ne_bytes(sample[..2].try_into().ok()?) as f32 / i16::MAX as f32
        }
        24 if sample.len() >= 3 => {
            let value = if cfg!(target_endian = "little") {
                i32::from_le_bytes([
                    sample[0],
                    sample[1],
                    sample[2],
                    if sample[2] & 0x80 != 0 { 0xFF } else { 0x00 },
                ])
            } else {
                i32::from_be_bytes([
                    if sample[0] & 0x80 != 0 { 0xFF } else { 0x00 },
                    sample[0],
                    sample[1],
                    sample[2],
                ])
            };
            value as f32 / 8_388_607.0
        }
        32 if sample.len() >= 4 => {
            i32::from_ne_bytes(sample[..4].try_into().ok()?) as f32 / i32::MAX as f32
        }
        _ => return None,
    };

    value.is_finite().then_some(value.clamp(-1.0, 1.0))
}

#[cfg(target_os = "macos")]
fn configure_microphone_writer_tail_buffer(context: &mut MicrophoneOutputContext) {
    if let Some(writer) = context.writer.as_mut() {
        set_audio_writer_inactivity_tail_trim_seconds(writer, context.inactivity_tail_trim_seconds);
        set_audio_writer_activity_threshold(writer, context.activity_threshold);
    }
}

#[cfg(target_os = "macos")]
fn microphone_tail_activity_override(context: &mut MicrophoneOutputContext) -> Option<bool> {
    if context.inactivity_tail_trim_seconds == 0 {
        return None;
    }

    match context.tail_activity_mode {
        MicrophoneInactivityTailTrimActivityMode::PeakLevel => None,
        MicrophoneInactivityTailTrimActivityMode::VadSpeech => {
            // VAD processing runs outside the AVFoundation callback. Treat each
            // observed speech decision as a one-sample activity pulse so the
            // rolling tail buffer preserves audio after speech, while the final
            // no-speech tail can still be discarded on inactivity pause.
            let sequence = current_microphone_vad_tail_speech_sequence();
            if sequence > context.observed_vad_tail_speech_sequence {
                context.observed_vad_tail_speech_sequence = sequence;
                Some(true)
            } else {
                Some(false)
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn append_microphone_sample_to_writer(
    context: &mut MicrophoneOutputContext,
    sample_buf: &cidre::cm::SampleBuf,
) -> Result<AudioSampleAppendDisposition, CaptureErrorResponse> {
    let activity_override = microphone_tail_activity_override(context);
    let Some(writer) = context.writer.as_mut() else {
        return Ok(AudioSampleAppendDisposition::Dropped);
    };

    let activity_override =
        if activity_override == Some(true) && record_audio_writer_tail_activity(writer)? {
            Some(false)
        } else {
            activity_override
        };

    try_append_audio_sample_to_writer_with_activity_override(writer, sample_buf, activity_override)
}

#[cfg(target_os = "macos")]
fn push_bounded_pending_microphone_sample<T>(pending_samples: &mut VecDeque<T>, sample: T) {
    pending_samples.push_back(sample);
    while pending_samples.len() > MAX_PENDING_MIC_SAMPLES {
        let _ = pending_samples.pop_front();
    }
}

#[cfg(target_os = "macos")]
fn transfer_pending_microphone_samples<T>(
    from_pending_samples: &mut VecDeque<T>,
    into_pending_samples: &mut VecDeque<T>,
) {
    into_pending_samples.extend(from_pending_samples.drain(..));
    while into_pending_samples.len() > MAX_PENDING_MIC_SAMPLES {
        let _ = into_pending_samples.pop_front();
    }
}

#[cfg(target_os = "macos")]
fn carry_probe_only_microphone_context(
    previous_context: &mut MicrophoneOutputContext,
    next_context: &mut MicrophoneOutputContext,
) {
    if previous_context.output_url.is_some() || previous_context.writer.is_some() {
        return;
    }

    if next_context.format_state.observed_format_count == 0 {
        next_context.format_state = previous_context.format_state;
    }

    transfer_pending_microphone_samples(
        &mut previous_context.pending_samples,
        &mut next_context.pending_samples,
    );
}

#[cfg(target_os = "macos")]
fn resume_microphone_output_context(
    current_context: &mut MicrophoneOutputContext,
    mut next_context: MicrophoneOutputContext,
    resumed_at_host_secs: f64,
) -> MicrophoneOutputContext {
    next_context.resumed_at_host_secs = Some(resumed_at_host_secs);
    carry_probe_only_microphone_context(current_context, &mut next_context);
    next_context.resume_carried_secs = next_context
        .pending_samples
        .iter()
        .map(buffered_microphone_sample_secs)
        .sum();
    std::mem::replace(current_context, next_context)
}

#[cfg(target_os = "macos")]
fn buffered_microphone_sample_secs(sample: &BufferedMicSample) -> f64 {
    let secs = sample.sample_buf.num_samples() as f64 / sample.format.sample_rate_hz;
    if secs.is_finite() {
        secs.max(0.0)
    } else {
        0.0
    }
}

/// A paused (probe-only) output's ring is audio from before the device stopped,
/// which a later resume would carry into the next file. A live output's ring
/// still belongs to its own file and is flushed when that file finalizes.
#[cfg(target_os = "macos")]
fn discard_probe_only_pending_samples(context: &mut MicrophoneOutputContext) {
    if context.output_url.is_none() && context.writer.is_none() {
        context.pending_samples.clear();
    }
}

#[cfg(target_os = "macos")]
fn replace_microphone_output_context_for_rotation(
    current_context: &mut MicrophoneOutputContext,
    next_context: MicrophoneOutputContext,
) -> MicrophoneOutputContext {
    let mut previous_context = std::mem::replace(current_context, next_context);
    previous_context.inactivity_tail_trim_seconds = 0;
    previous_context
}

#[cfg(target_os = "macos")]
fn flush_pending_microphone_sample_queue<T, E>(
    pending_samples: &mut VecDeque<T>,
    selected_format: Option<AudioSampleFormat>,
    mut sample_format: impl FnMut(&T) -> AudioSampleFormat,
    mut append_sample: impl FnMut(&T) -> Result<AudioSampleAppendDisposition, E>,
) -> Result<(), E> {
    while let Some(sample) = pending_samples.pop_front() {
        if selected_format.is_some_and(|format| {
            !audio_sample_format_is_compatible_with_writer_format(format, sample_format(&sample))
        }) {
            continue;
        }

        match append_sample(&sample)? {
            AudioSampleAppendDisposition::Appended | AudioSampleAppendDisposition::Dropped => {}
            AudioSampleAppendDisposition::Deferred => {
                pending_samples.push_front(sample);
                break;
            }
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn flush_pending_microphone_samples(
    context: &mut MicrophoneOutputContext,
) -> Result<(), CaptureErrorResponse> {
    let selected_format = fallback_microphone_format(context);
    if context.writer.is_none() {
        return Ok(());
    }

    let mut pending_samples = std::mem::take(&mut context.pending_samples);
    let result = flush_pending_microphone_sample_queue(
        &mut pending_samples,
        selected_format,
        |sample| sample.format,
        |sample| append_microphone_sample_to_writer(context, sample.sample_buf.as_ref()),
    );
    context.pending_samples = pending_samples;

    result.map(|_| ())
}

#[cfg(target_os = "macos")]
fn microphone_output_callback_panic_error(
    payload: Box<dyn std::any::Any + Send>,
) -> CaptureErrorResponse {
    let message = if let Some(message) = payload.downcast_ref::<&'static str>() {
        format!("Microphone output callback panicked: {message}")
    } else if let Some(message) = payload.downcast_ref::<String>() {
        format!("Microphone output callback panicked: {message}")
    } else {
        "Microphone output callback panicked with a non-string payload".to_string()
    };

    capture_runtime::error_log!(
        "[capture-microphone] panic boundary captured in microphone output callback: {message}"
    );

    CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message,
    }
}

#[cfg(target_os = "macos")]
fn microphone_output_callback_objc_exception_error(
    exception: &ns::Exception,
) -> CaptureErrorResponse {
    let name_ref = exception.name();
    let name = format!("{}", &**name_ref);
    let reason = exception
        .reason()
        .map(|r| format!("{}", r.as_ref()))
        .unwrap_or_else(|| "unknown reason".to_string());

    let message = format!("Microphone output callback ObjC exception: {name} - {reason}");

    capture_runtime::error_log!(
        "[capture-microphone] ObjC exception boundary captured in microphone output callback: {message}"
    );

    CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message,
    }
}

#[cfg(target_os = "macos")]
mod microphone_delegate {
    #![allow(clippy::useless_transmute)]

    use super::{
        create_audio_asset_writer_for_sample_format, derive_audio_sample_format_from_sample_buf,
        flush_pending_microphone_samples, maybe_feed_microphone_vad_pcm,
        maybe_track_microphone_activity, microphone_output_callback_objc_exception_error,
        microphone_output_callback_panic_error, ns, objc, push_bounded_pending_microphone_sample,
        record_observed_audio_format, resolve_microphone_live_format, BufferedMicSample,
        MicrophoneOutputContext,
    };
    use cidre::av::capture::AudioDataOutputSampleBufDelegate;

    cidre::define_obj_type!(
        pub(super) MicAudioDataOutputDelegate
            + cidre::av::capture::AudioDataOutputSampleBufDelegateImpl,
        MicrophoneOutputContext,
        ZMicAudioDataOutputDelegate
    );

    impl cidre::av::capture::AudioDataOutputSampleBufDelegate for MicAudioDataOutputDelegate {}

    #[cidre::objc::add_methods]
    impl cidre::av::capture::AudioDataOutputSampleBufDelegateImpl for MicAudioDataOutputDelegate {
        extern "C" fn impl_capture_output_did_output_sample_buf_from_connection(
            &mut self,
            _cmd: Option<&cidre::objc::Sel>,
            _output: &cidre::av::CaptureOutput,
            sample_buf: &cidre::cm::SampleBuf,
            _connection: &cidre::av::CaptureConnection,
        ) {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let objc_result = ns::try_catch(|| {
                    let ctx = self.inner_mut();
                    if ctx.first_error.is_some() {
                        return;
                    }

                    maybe_track_microphone_activity(sample_buf);
                    maybe_feed_microphone_vad_pcm(sample_buf);

                    let Some(sample_format) =
                        derive_audio_sample_format_from_sample_buf(sample_buf)
                    else {
                        return;
                    };

                    if ctx.writer.is_none() {
                        record_observed_audio_format(ctx, sample_format);
                    }

                    push_bounded_pending_microphone_sample(
                        &mut ctx.pending_samples,
                        BufferedMicSample {
                            sample_buf: sample_buf.retained(),
                            format: sample_format,
                        },
                    );

                    if ctx.output_url.is_none() {
                        return;
                    }

                    if ctx.writer.is_none() {
                        let Some(stable_format) = resolve_microphone_live_format(&ctx.format_state)
                        else {
                            return;
                        };

                        match create_audio_asset_writer_for_sample_format(
                            ctx.output_url
                                .as_ref()
                                .expect("microphone output URL should exist when writer is active")
                                .as_ref(),
                            "microphone",
                            stable_format,
                        ) {
                            Ok(writer) => {
                                ctx.writer = Some(writer);
                                super::configure_microphone_writer_tail_buffer(ctx);
                            }
                            Err(error) => {
                                ctx.first_error = Some(error);
                                return;
                            }
                        }
                    }

                    if let Err(error) = flush_pending_microphone_samples(ctx) {
                        ctx.first_error = Some(error);
                    }
                });

                if let Err(exception) = objc_result {
                    self.inner_mut().first_error =
                        Some(microphone_output_callback_objc_exception_error(exception));
                }
            }));

            if let Err(payload) = result {
                self.inner_mut().first_error =
                    Some(microphone_output_callback_panic_error(payload));
            }
        }
    }
}

#[cfg(target_os = "macos")]
use microphone_delegate::MicAudioDataOutputDelegate;

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct AvFoundationMicrophoneCaptureSession {
    capture_session: cidre::arc::R<cidre::av::capture::Session>,
    _audio_output: cidre::arc::R<cidre::av::capture::AudioDataOutput>,
    output_delegate: cidre::arc::R<MicAudioDataOutputDelegate>,
    output_queue: cidre::arc::R<dispatch::Queue>,
}

#[cfg(target_os = "macos")]
impl AvFoundationMicrophoneCaptureSession {
    pub fn stop(&mut self) -> Result<(), CaptureErrorResponse> {
        self.stop_with_inactivity_tail_trim_seconds(0, 0.0)
            .map(|_| ())
    }

    pub fn stop_returning_finalization(
        &mut self,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        self.stop_with_inactivity_tail_trim_seconds(0, 0.0)
    }

    pub fn stop_for_inactivity(
        &mut self,
        tail_trim_seconds: u64,
        activity_threshold: f32,
    ) -> Result<(), CaptureErrorResponse> {
        self.stop_with_inactivity_tail_trim_seconds(tail_trim_seconds, activity_threshold)
            .map(|_| ())
    }

    pub fn stop_for_inactivity_with_tail_activity_mode(
        &mut self,
        tail_trim_seconds: u64,
        activity_threshold: f32,
        tail_activity_mode: MicrophoneInactivityTailTrimActivityMode,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        self.capture_session.stop_running();
        self.pause_output_file_for_inactivity_with_tail_activity_mode(
            tail_trim_seconds,
            activity_threshold,
            tail_activity_mode,
        )
    }

    fn stop_with_inactivity_tail_trim_seconds(
        &mut self,
        tail_trim_seconds: u64,
        activity_threshold: f32,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        self.stop_for_inactivity_with_tail_activity_mode(
            tail_trim_seconds,
            activity_threshold,
            MicrophoneInactivityTailTrimActivityMode::PeakLevel,
        )
    }

    /// Run `f` on the context from the output queue, the serial queue the
    /// sample callback runs on, so it never overlaps a callback mutating the
    /// same context. Keep `f` to a swap; finalize what it hands back off-queue.
    fn with_output_context<R>(&mut self, f: impl FnOnce(&mut MicrophoneOutputContext) -> R) -> R {
        // `R` already holds the delegate as `&'static mut`; this reborrow only
        // lives long enough to become the pointer the queue block dereferences.
        let delegate = std::ptr::NonNull::from(&mut *self.output_delegate);
        with_output_context_on_queue(&self.output_queue, delegate, f)
    }

    /// Stop or restart the device without touching the output file, so the mic
    /// hardware, its indicator and coreaudiod's input I/O go quiet. Stop before
    /// pausing the output, so nothing reaches the paused ring while the old file
    /// finalizes; a writer left open across the gap would splice two takes.
    /// Stopping drops a paused output's queued audio, so a later resume opens
    /// with what was heard after the restart, not before the stop. Returns
    /// whether the device reached the requested state: `startRunning` reports
    /// no error, so a device that failed to come back reads as not running.
    ///
    /// Blocks: `startRunning`/`stopRunning` are synchronous and can take
    /// seconds on a Bluetooth device. A `true` from a restart is not a promise
    /// either: the session can come up and die moments later (a runtime error
    /// or interruption flips `isRunning` back), so callers must re-check
    /// [`Self::capture_running`] rather than trust this result.
    pub fn set_capture_running(&mut self, running: bool) -> bool {
        if running {
            // A stopped device's file must be paused before the restart. The
            // stop itself can't close it: the caller books that finalization.
            debug_assert!(
                self.capture_session.is_running()
                    || self.with_output_context(|context| context.writer.is_none()),
                "restarting the microphone into an open file would splice two takes"
            );
            self.capture_session.start_running();
            return self.capture_session.is_running();
        }
        self.capture_session.stop_running();
        self.with_output_context(discard_probe_only_pending_samples);
        // The first buffer after a restart re-anchors VAD frame times; a partial
        // frame left over would carry the pre-stop times across the gap. Whole
        // frames already queued stay for the caller to drain.
        reset_microphone_vad_pcm_partial_frame();
        !self.capture_session.is_running()
    }

    /// Whether the device is delivering audio right now. A session that exists
    /// but reads `false` here (a restart that never came up, or one that died
    /// after `set_capture_running(true)` returned) is capturing nothing.
    pub fn capture_running(&self) -> bool {
        self.capture_session.is_running()
    }

    pub fn rotate_output_file(&mut self, output_file: &str) -> Result<(), CaptureErrorResponse> {
        self.rotate_output_file_returning_finalization(output_file)
            .map(|_| ())
    }

    pub fn rotate_output_file_returning_finalization(
        &mut self,
        output_file: &str,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        let output_url = cidre::ns::Url::with_fs_path_str(output_file, false);

        // Build off-queue: a panic inside the queue block aborts the process.
        // Only this thread changes these settings, so they can't move between
        // the two blocks.
        let (tail_trim_seconds, activity_threshold, tail_activity_mode) =
            self.with_output_context(|context| {
                (
                    context.inactivity_tail_trim_seconds,
                    context.activity_threshold,
                    context.tail_activity_mode,
                )
            });
        let mut next_context = microphone_output_context_for_output_url(
            &output_url,
            Some(output_file.to_string()),
            tail_trim_seconds,
            activity_threshold,
            tail_activity_mode,
        );
        let mut previous_context = self.with_output_context(|current_context| {
            // VAD drops from callbacks queued ahead of this block are the old
            // file's; observe them as already seen, as a build here would.
            next_context.observed_vad_tail_speech_sequence =
                current_microphone_vad_tail_speech_sequence();
            next_context.observed_boundary_trim_disabled_sequence =
                current_microphone_vad_boundary_trim_disabled_sequence();
            replace_microphone_output_context_for_rotation(current_context, next_context)
        });
        finalize_microphone_output_context(&mut previous_context)
    }

    pub fn pause_output_file(&mut self) -> Result<(), CaptureErrorResponse> {
        self.pause_output_file_for_inactivity(0, 0.0).map(|_| ())
    }

    pub fn pause_output_file_for_inactivity(
        &mut self,
        tail_trim_seconds: u64,
        activity_threshold: f32,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        self.pause_output_file_for_inactivity_with_tail_activity_mode(
            tail_trim_seconds,
            activity_threshold,
            MicrophoneInactivityTailTrimActivityMode::PeakLevel,
        )
    }

    pub fn pause_output_file_for_inactivity_with_tail_activity_mode(
        &mut self,
        tail_trim_seconds: u64,
        activity_threshold: f32,
        tail_activity_mode: MicrophoneInactivityTailTrimActivityMode,
    ) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
        let probe_only_context = microphone_probe_only_context();
        let mut previous_context =
            self.with_output_context(|context| std::mem::replace(context, probe_only_context));
        previous_context.inactivity_tail_trim_seconds = tail_trim_seconds;
        previous_context.activity_threshold =
            normalized_audio_activity_threshold(activity_threshold);
        previous_context.tail_activity_mode = tail_activity_mode;
        finalize_microphone_output_context(&mut previous_context)
    }

    pub fn resume_output_file(&mut self, output_file: &str) -> Result<(), CaptureErrorResponse> {
        self.resume_output_file_with_inactivity_tail_trim_seconds(output_file, 0, 0.0)
    }

    pub fn resume_output_file_with_inactivity_tail_trim_seconds(
        &mut self,
        output_file: &str,
        tail_trim_seconds: u64,
        activity_threshold: f32,
    ) -> Result<(), CaptureErrorResponse> {
        self.resume_output_file_with_inactivity_tail_trim_activity_mode(
            output_file,
            tail_trim_seconds,
            activity_threshold,
            MicrophoneInactivityTailTrimActivityMode::PeakLevel,
        )
    }

    pub fn resume_output_file_with_inactivity_tail_trim_activity_mode(
        &mut self,
        output_file: &str,
        tail_trim_seconds: u64,
        activity_threshold: f32,
        tail_activity_mode: MicrophoneInactivityTailTrimActivityMode,
    ) -> Result<(), CaptureErrorResponse> {
        // The caller stamped `output_file` with "now" just before this call.
        let resumed_at_host_secs = cidre::cm::Clock::host_time_clock().time().as_secs();
        let output_url = cidre::ns::Url::with_fs_path_str(output_file, false);

        let next_context = microphone_output_context_for_output_url(
            &output_url,
            Some(output_file.to_string()),
            tail_trim_seconds,
            activity_threshold,
            tail_activity_mode,
        );
        let mut previous_context = self.with_output_context(|current_context| {
            resume_microphone_output_context(current_context, next_context, resumed_at_host_secs)
        });
        finalize_microphone_output_context(&mut previous_context)?;

        Ok(())
    }
}

/// cidre registers the delegate class lazily with an unguarded check-then-
/// `objc_allocateClassPair(..).unwrap()` (cidre 0.15 `objc.rs:503-506`), so two
/// threads creating the first delegate at once (a recording and an enrollment
/// session) panic on the loser's nil class. Register it exactly once instead.
#[cfg(target_os = "macos")]
fn new_mic_output_delegate(
    context: MicrophoneOutputContext,
) -> cidre::arc::R<MicAudioDataOutputDelegate> {
    static REGISTER: std::sync::Once = std::sync::Once::new();
    REGISTER.call_once(|| {
        MicAudioDataOutputDelegate::cls();
    });
    MicAudioDataOutputDelegate::with(context)
}

/// The delegate's context is reached through an unsynchronised raw cast, and
/// AVFoundation calls the delegate on `queue` while the session runs, so the
/// only safe place to touch it from another thread is a block on that serial
/// queue. Queued callbacks run first, and none can start until `f` returns.
/// Takes a pointer rather than `&mut`, so no reference to the delegate is live
/// while those queued callbacks form their own `&mut self`.
#[cfg(target_os = "macos")]
fn with_output_context_on_queue<R>(
    queue: &dispatch::Queue,
    delegate: std::ptr::NonNull<MicAudioDataOutputDelegate>,
    f: impl FnOnce(&mut MicrophoneOutputContext) -> R,
) -> R {
    queue.sync_once(|| {
        // SAFETY: the caller keeps the delegate retained across this call, and
        // the block runs on the serial queue that is the only other place a
        // `&mut` to it is formed, so this one is unique while it lives.
        let delegate = unsafe { &mut *delegate.as_ptr() };
        f(delegate.inner_mut())
    })
}

#[cfg(target_os = "macos")]
fn finalize_microphone_output_context(
    context: &mut MicrophoneOutputContext,
) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
    if context.writer.is_none() && !context.pending_samples.is_empty() {
        if let (Some(output_url), Some(format)) = (
            context.output_url.as_ref(),
            resolve_microphone_finalize_format(&context.format_state),
        ) {
            match create_audio_asset_writer_for_sample_format(
                output_url.as_ref(),
                "microphone",
                format,
            ) {
                Ok(writer) => {
                    context.writer = Some(writer);
                    configure_microphone_writer_tail_buffer(context);
                    if let Err(error) = flush_pending_microphone_samples(context) {
                        context.first_error.get_or_insert(error);
                    }
                }
                Err(error) => {
                    context.first_error.get_or_insert(error);
                }
            }
        }
    }

    configure_microphone_writer_tail_buffer(context);

    let finalize_result = if context.inactivity_tail_trim_seconds > 0 {
        let mut failures = Vec::new();
        if let Some(error) = context.first_error.take() {
            failures.push(format!(
                "microphone stream output failed: [{}] {}",
                error.code, error.message
            ));
        }
        let pending_vad_tail_activity = matches!(
            context.tail_activity_mode,
            MicrophoneInactivityTailTrimActivityMode::VadSpeech
        ) && microphone_tail_activity_override(context)
            == Some(true);

        if let Some(writer) = context.writer.as_mut() {
            if pending_vad_tail_activity {
                if let Err(error) = record_audio_writer_tail_activity(writer) {
                    failures.push(format!("microphone writer failed: {}", error.message));
                }
            }
            if let Err(error) = finish_audio_asset_writer_discarding_inactivity_tail(writer) {
                failures.push(format!("microphone writer failed: {}", error.message));
            }
        } else {
            failures.push(capture_writers::no_audio_samples_error("microphone").message);
        }
        capture_writers::aggregate_output_processing_failures(failures)
    } else {
        writers_finalize_microphone_output_context(
            context.writer.as_mut(),
            context.first_error.take(),
        )
    };

    match finalize_result {
        Err(error) if is_nonfatal_microphone_finalize_error(&error) => {
            if context.boundary_trim_enabled {
                take_microphone_vad_speech_boundaries_for_context(context);
            }
            if let Some(path) = context.output_file.as_deref() {
                maybe_remove_microphone_output_file(path);
            }
            Ok(microphone_output_finalization_for_context(
                context,
                false,
                0,
                Some("no_audio_samples".to_string()),
            ))
        }
        Err(error) => Err(error),
        Ok(()) => {
            let resume_correction_ms = microphone_resume_stamp_correction_ms(context);
            finalize_microphone_vad_boundary_trim(context).map(|mut finalization| {
                finalization.start_offset_ms += resume_correction_ms;
                finalization
            })
        }
    }
}

#[cfg(target_os = "macos")]
fn microphone_output_context_for_output_url(
    output_url: &cidre::ns::Url,
    output_file: Option<String>,
    inactivity_tail_trim_seconds: u64,
    activity_threshold: f32,
    tail_activity_mode: MicrophoneInactivityTailTrimActivityMode,
) -> MicrophoneOutputContext {
    let boundary_trim_enabled = matches!(
        tail_activity_mode,
        MicrophoneInactivityTailTrimActivityMode::VadSpeech
    );
    let source_output_file = output_file.as_deref().map(|file| {
        if boundary_trim_enabled {
            microphone_vad_temp_source_file(file)
        } else {
            file.to_string()
        }
    });
    let source_output_url = source_output_file
        .as_deref()
        .map(|file| cidre::ns::Url::with_fs_path_str(file, false));
    MicrophoneOutputContext {
        writer: None,
        output_url: Some(
            source_output_url
                .as_ref()
                .map(|url| url.retained())
                .unwrap_or_else(|| output_url.retained()),
        ),
        source_output_file,
        output_file,
        first_error: None,
        format_state: MicFormatStabilityState::default(),
        pending_samples: VecDeque::new(),
        inactivity_tail_trim_seconds,
        activity_threshold: normalized_audio_activity_threshold(activity_threshold),
        tail_activity_mode,
        observed_vad_tail_speech_sequence: current_microphone_vad_tail_speech_sequence(),
        boundary_trim_enabled,
        boundary_trim_disabled: false,
        observed_boundary_trim_disabled_sequence:
            current_microphone_vad_boundary_trim_disabled_sequence(),
        first_speech_start_secs: None,
        last_speech_end_secs: None,
        detected_vad_speech: false,
        resumed_at_host_secs: None,
        resume_carried_secs: 0.0,
    }
}

#[cfg(target_os = "macos")]
fn microphone_vad_temp_source_file(output_file: &str) -> String {
    let path = std::path::Path::new(output_file);
    let parent = path.parent();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("microphone.m4a");
    let temp_name = format!(".{file_name}.vad-source.m4a");
    parent
        .map(|parent| parent.join(&temp_name))
        .unwrap_or_else(|| std::path::PathBuf::from(temp_name))
        .to_string_lossy()
        .to_string()
}

#[cfg(target_os = "macos")]
fn normalized_audio_activity_threshold(threshold: f32) -> f32 {
    if threshold.is_finite() {
        threshold.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(target_os = "macos")]
fn microphone_probe_only_context() -> MicrophoneOutputContext {
    MicrophoneOutputContext {
        writer: None,
        output_url: None,
        source_output_file: None,
        output_file: None,
        first_error: None,
        format_state: MicFormatStabilityState::default(),
        pending_samples: VecDeque::new(),
        inactivity_tail_trim_seconds: 0,
        activity_threshold: 0.0,
        tail_activity_mode: MicrophoneInactivityTailTrimActivityMode::PeakLevel,
        observed_vad_tail_speech_sequence: current_microphone_vad_tail_speech_sequence(),
        boundary_trim_enabled: false,
        boundary_trim_disabled: false,
        observed_boundary_trim_disabled_sequence:
            current_microphone_vad_boundary_trim_disabled_sequence(),
        first_speech_start_secs: None,
        last_speech_end_secs: None,
        detected_vad_speech: false,
        resumed_at_host_secs: None,
        resume_carried_secs: 0.0,
    }
}

#[cfg(target_os = "macos")]
const MICROPHONE_STREAM_OUTPUT_FAILURE_PREFIX: &str = "microphone stream output failed: ";
#[cfg(target_os = "macos")]
const MICROPHONE_WRITER_FAILURE_PREFIX: &str = "microphone writer failed: ";

#[cfg(target_os = "macos")]
fn maybe_remove_microphone_output_file(path: &str) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => capture_runtime::warn_log!(
            "[capture-microphone] failed to remove invalid microphone artifact {path}: {error}"
        ),
    }
}

#[cfg(target_os = "macos")]
fn is_nonfatal_microphone_finalize_error(error: &CaptureErrorResponse) -> bool {
    error.code == "capture_output_processing_failed"
        && capture_writers::single_output_processing_failure_detail(
            &error.message,
            &[
                MICROPHONE_STREAM_OUTPUT_FAILURE_PREFIX,
                MICROPHONE_WRITER_FAILURE_PREFIX,
            ],
        )
        .is_some_and(|detail| {
            capture_writers::is_no_audio_samples_error_message("microphone", detail)
                || detail
                    .strip_prefix(MICROPHONE_WRITER_FAILURE_PREFIX)
                    .is_some_and(|detail| {
                        capture_writers::is_no_audio_samples_error_message("microphone", detail)
                    })
        })
}

pub fn resolve_effective_microphone_device(
    devices: &[MicrophoneDevice],
    preference: &MicrophonePreference,
    disconnect_policy: MicrophoneDisconnectPolicy,
) -> Option<MicrophoneDevice> {
    let default_device = || devices.iter().find(|device| device.is_default).cloned();

    match preference.mode {
        MicrophonePreferenceMode::Default => default_device(),
        MicrophonePreferenceMode::SpecificDevice => {
            let configured_id = preference.device_id.as_deref()?;
            let selected = devices
                .iter()
                .find(|device| device.id == configured_id)
                .cloned();

            if selected.is_some() {
                return selected;
            }

            match disconnect_policy {
                MicrophoneDisconnectPolicy::FallbackToDefault => default_device(),
                MicrophoneDisconnectPolicy::WaitForSameDevice => None,
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub fn list_microphone_devices() -> Result<Vec<MicrophoneDevice>, CaptureErrorResponse> {
    let default_device_id = av::CaptureDevice::default_with_media(av::MediaType::audio())
        .map(|device| device.unique_id().to_string());

    let mut devices = Vec::new();
    for device in av::CaptureDevice::devices().iter() {
        if !device.has_media_type(av::MediaType::audio()) || !device.is_connected() {
            continue;
        }

        let id = device.unique_id().to_string();
        let name = device.localized_name().to_string();
        let is_default = default_device_id.as_deref() == Some(id.as_str());
        devices.push(MicrophoneDevice {
            id,
            name,
            is_default,
        });
    }

    Ok(devices)
}

#[cfg(not(target_os = "macos"))]
pub fn list_microphone_devices() -> Result<Vec<MicrophoneDevice>, CaptureErrorResponse> {
    Ok(Vec::new())
}

pub fn microphone_controller_state(
    preference: MicrophonePreference,
    disconnect_policy: MicrophoneDisconnectPolicy,
) -> Result<MicrophoneControllerState, CaptureErrorResponse> {
    let devices = list_microphone_devices()?;
    let effective_device =
        resolve_effective_microphone_device(&devices, &preference, disconnect_policy.clone());

    Ok(MicrophoneControllerState {
        devices,
        preference,
        disconnect_policy,
        effective_device,
    })
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct MicrophoneDeviceChangeNotifier {
    _connected_observer: cidre::ns::NotificationGuard,
    _disconnected_observer: cidre::ns::NotificationGuard,
    _default_input_listener: Option<DefaultInputDeviceListener>,
}

#[cfg(target_os = "macos")]
type DeviceChangeCallback = dyn Fn() + Send + Sync + 'static;

#[cfg(target_os = "macos")]
type AudioObjectId = u32;
#[cfg(target_os = "macos")]
type AudioObjectPropertySelector = u32;
#[cfg(target_os = "macos")]
type AudioObjectPropertyScope = u32;
#[cfg(target_os = "macos")]
type AudioObjectPropertyElement = u32;
#[cfg(target_os = "macos")]
type OsStatus = i32;

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct AudioObjectPropertyAddress {
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
    element: AudioObjectPropertyElement,
}

#[cfg(target_os = "macos")]
type AudioObjectPropertyListenerProc = extern "C-unwind" fn(
    AudioObjectId,
    u32,
    *const AudioObjectPropertyAddress,
    *mut c_void,
) -> OsStatus;

#[cfg(target_os = "macos")]
#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectAddPropertyListener(
        object_id: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        listener: AudioObjectPropertyListenerProc,
        client_data: *mut c_void,
    ) -> OsStatus;

    fn AudioObjectRemovePropertyListener(
        object_id: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        listener: AudioObjectPropertyListenerProc,
        client_data: *mut c_void,
    ) -> OsStatus;
}

#[cfg(target_os = "macos")]
const fn four_char_code(bytes: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*bytes)
}

#[cfg(target_os = "macos")]
const AUDIO_OBJECT_SYSTEM_OBJECT: AudioObjectId = 1;
#[cfg(target_os = "macos")]
const AUDIO_HARDWARE_PROPERTY_DEFAULT_INPUT_DEVICE: AudioObjectPropertySelector =
    four_char_code(b"dIn ");
#[cfg(target_os = "macos")]
const AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL: AudioObjectPropertyScope = four_char_code(b"glob");
#[cfg(target_os = "macos")]
const AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN: AudioObjectPropertyElement = 0;

#[cfg(target_os = "macos")]
const DEFAULT_INPUT_DEVICE_ADDRESS: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    selector: AUDIO_HARDWARE_PROPERTY_DEFAULT_INPUT_DEVICE,
    scope: AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
    element: AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
};

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct DefaultInputDeviceListener {
    callback_ptr_addr: usize,
}

#[cfg(target_os = "macos")]
extern "C-unwind" fn default_input_device_changed_listener(
    _obj_id: AudioObjectId,
    _number_addresses: u32,
    _addresses: *const AudioObjectPropertyAddress,
    callback_ptr: *mut c_void,
) -> OsStatus {
    let Some(callback) = (unsafe { (callback_ptr as *mut Arc<DeviceChangeCallback>).as_ref() })
    else {
        return 0;
    };

    let _ = ns::try_catch(|| {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            callback();
        }));
    });
    0
}

#[cfg(target_os = "macos")]
impl Drop for DefaultInputDeviceListener {
    fn drop(&mut self) {
        let _ = unsafe {
            AudioObjectRemovePropertyListener(
                AUDIO_OBJECT_SYSTEM_OBJECT,
                &DEFAULT_INPUT_DEVICE_ADDRESS,
                default_input_device_changed_listener,
                self.callback_ptr_addr as *mut c_void,
            )
        };

        unsafe {
            drop(Box::from_raw(
                self.callback_ptr_addr as *mut Arc<DeviceChangeCallback>,
            ));
        }
    }
}

#[cfg(target_os = "macos")]
trait IntoNotificationName<'a> {
    fn into_notification_name(self) -> &'a cidre::ns::NotificationName;
}

#[cfg(target_os = "macos")]
impl<'a> IntoNotificationName<'a> for &'a cidre::ns::NotificationName {
    fn into_notification_name(self) -> &'a cidre::ns::NotificationName {
        self
    }
}

#[cfg(target_os = "macos")]
impl<'a> IntoNotificationName<'a> for Option<&'a cidre::ns::NotificationName> {
    fn into_notification_name(self) -> &'a cidre::ns::NotificationName {
        self.expect("AVCaptureDevice notification unavailable")
    }
}

#[cfg(target_os = "macos")]
#[allow(unused_unsafe)]
pub fn start_microphone_device_change_notifier(
    callback: impl Fn() + Send + Sync + 'static,
) -> MicrophoneDeviceChangeNotifier {
    let mut center = cidre::ns::NotificationCenter::default();
    let callback: Arc<DeviceChangeCallback> = Arc::new(callback);
    let connected_notification = IntoNotificationName::into_notification_name(unsafe {
        av::capture::device::notifications::was_connected()
    });
    let disconnected_notification = IntoNotificationName::into_notification_name(unsafe {
        av::capture::device::notifications::was_disconnected()
    });

    let callback_connected = Arc::clone(&callback);
    let connected_observer =
        center.add_observer_guard(connected_notification, None, None, move |_notification| {
            callback_connected()
        });

    let callback_disconnected = Arc::clone(&callback);
    let disconnected_observer = center.add_observer_guard(
        disconnected_notification,
        None,
        None,
        move |_notification| callback_disconnected(),
    );

    let callback_ptr = Box::into_raw(Box::new(Arc::clone(&callback)));
    let default_input_listener = if unsafe {
        AudioObjectAddPropertyListener(
            AUDIO_OBJECT_SYSTEM_OBJECT,
            &DEFAULT_INPUT_DEVICE_ADDRESS,
            default_input_device_changed_listener,
            callback_ptr.cast::<c_void>(),
        )
    } == 0
    {
        Some(DefaultInputDeviceListener {
            callback_ptr_addr: callback_ptr as usize,
        })
    } else {
        unsafe {
            drop(Box::from_raw(callback_ptr));
        }
        None
    };

    MicrophoneDeviceChangeNotifier {
        _connected_observer: connected_observer,
        _disconnected_observer: disconnected_observer,
        _default_input_listener: default_input_listener,
    }
}

#[cfg(not(target_os = "macos"))]
#[derive(Debug, Default)]
pub struct MicrophoneDeviceChangeNotifier;

#[cfg(not(target_os = "macos"))]
pub fn start_microphone_device_change_notifier(
    _callback: impl Fn() + Send + Sync + 'static,
) -> MicrophoneDeviceChangeNotifier {
    MicrophoneDeviceChangeNotifier
}

#[cfg(target_os = "macos")]
fn resolve_capture_device_for_id(
    device_id: Option<&str>,
) -> Result<cidre::arc::R<av::CaptureDevice>, CaptureErrorResponse> {
    match device_id {
        Some(device_id) => {
            let ns_device_id = cidre::ns::String::with_str(device_id);
            av::CaptureDevice::with_unique_id(ns_device_id.as_ref()).ok_or_else(|| {
                CaptureErrorResponse {
                    code: "microphone_input_unavailable".to_string(),
                    message: "Failed to resolve requested microphone device".to_string(),
                }
            })
        }
        None => av::CaptureDevice::default_with_media(av::MediaType::audio()).ok_or_else(|| {
            CaptureErrorResponse {
                code: "microphone_input_unavailable".to_string(),
                message: "Failed to resolve microphone device".to_string(),
            }
        }),
    }
}

#[cfg(target_os = "macos")]
pub fn start_avfoundation_microphone_capture_session(
    output_url: &cidre::ns::Url,
) -> Result<AvFoundationMicrophoneCaptureSession, CaptureErrorResponse> {
    start_avfoundation_microphone_capture_session_with_output_file(
        output_url,
        None,
        None,
        0,
        0.0,
        MicrophoneInactivityTailTrimActivityMode::PeakLevel,
    )
}

#[cfg(target_os = "macos")]
pub fn start_avfoundation_microphone_capture_session_with_device_id(
    output_url: &cidre::ns::Url,
    device_id: Option<&str>,
) -> Result<AvFoundationMicrophoneCaptureSession, CaptureErrorResponse> {
    start_avfoundation_microphone_capture_session_with_output_file(
        output_url,
        None,
        device_id,
        0,
        0.0,
        MicrophoneInactivityTailTrimActivityMode::PeakLevel,
    )
}

#[cfg(target_os = "macos")]
pub fn start_avfoundation_microphone_capture_session_for_file_with_device_id_and_inactivity_tail_trim_seconds(
    output_file: &str,
    device_id: Option<&str>,
    tail_trim_seconds: u64,
    activity_threshold: f32,
) -> Result<AvFoundationMicrophoneCaptureSession, CaptureErrorResponse> {
    start_avfoundation_microphone_capture_session_for_file_with_device_id_and_inactivity_tail_trim_activity_mode(
        output_file,
        device_id,
        tail_trim_seconds,
        activity_threshold,
        MicrophoneInactivityTailTrimActivityMode::PeakLevel,
    )
}

#[cfg(target_os = "macos")]
pub fn start_avfoundation_microphone_capture_session_for_file_with_device_id_and_inactivity_tail_trim_activity_mode(
    output_file: &str,
    device_id: Option<&str>,
    tail_trim_seconds: u64,
    activity_threshold: f32,
    tail_activity_mode: MicrophoneInactivityTailTrimActivityMode,
) -> Result<AvFoundationMicrophoneCaptureSession, CaptureErrorResponse> {
    let output_url = cidre::ns::Url::with_fs_path_str(output_file, false);
    start_avfoundation_microphone_capture_session_with_output_file(
        &output_url,
        Some(output_file.to_string()),
        device_id,
        tail_trim_seconds,
        activity_threshold,
        tail_activity_mode,
    )
}

#[cfg(target_os = "macos")]
fn start_avfoundation_microphone_capture_session_with_output_file(
    output_url: &cidre::ns::Url,
    output_file: Option<String>,
    device_id: Option<&str>,
    tail_trim_seconds: u64,
    activity_threshold: f32,
    tail_activity_mode: MicrophoneInactivityTailTrimActivityMode,
) -> Result<AvFoundationMicrophoneCaptureSession, CaptureErrorResponse> {
    reset_last_microphone_activity_unix_ms();
    reset_microphone_vad_pcm_feed();
    reset_microphone_vad_tail_activity();

    let mut capture_session = av::CaptureSession::new();

    let mic_device = resolve_capture_device_for_id(device_id)?;

    let mic_input = av::CaptureDeviceInput::with_device(mic_device.as_ref()).map_err(|_| {
        CaptureErrorResponse {
            code: "microphone_input_unavailable".to_string(),
            message: "Failed to create microphone input".to_string(),
        }
    })?;

    let mut audio_output = av::capture::AudioDataOutput::new();
    let output_delegate = new_mic_output_delegate(microphone_output_context_for_output_url(
        output_url,
        output_file,
        tail_trim_seconds,
        activity_threshold,
        tail_activity_mode,
    ));
    let output_queue = dispatch::Queue::serial_with_ar_pool();
    audio_output.set_sample_buf_delegate(Some(output_delegate.as_ref()), Some(&output_queue));

    let can_add_input = capture_session.can_add_input(&mic_input);
    let can_add_output = capture_session.can_add_output(&audio_output);

    if !can_add_input {
        return Err(CaptureErrorResponse {
            code: "microphone_input_unavailable".to_string(),
            message: "Failed to add microphone input".to_string(),
        });
    }

    if !can_add_output {
        return Err(CaptureErrorResponse {
            code: "capture_output_unavailable".to_string(),
            message: "Failed to add microphone audio output".to_string(),
        });
    }

    capture_session.configure(|session| {
        session.add_input(&mic_input);
        session.add_output(&audio_output);
    });

    capture_session.start_running();

    Ok(AvFoundationMicrophoneCaptureSession {
        capture_session,
        _audio_output: audio_output,
        output_delegate,
        output_queue,
    })
}

#[cfg(target_os = "macos")]
pub fn start_avfoundation_microphone_capture_session_for_file(
    output_file: &str,
) -> Result<AvFoundationMicrophoneCaptureSession, CaptureErrorResponse> {
    start_avfoundation_microphone_capture_session_for_file_with_device_id(output_file, None)
}

#[cfg(target_os = "macos")]
pub fn start_avfoundation_microphone_capture_session_for_file_with_device_id(
    output_file: &str,
    device_id: Option<&str>,
) -> Result<AvFoundationMicrophoneCaptureSession, CaptureErrorResponse> {
    let output_url = cidre::ns::Url::with_fs_path_str(output_file, false);
    start_avfoundation_microphone_capture_session_with_output_file(
        &output_url,
        Some(output_file.to_string()),
        device_id,
        0,
        0.0,
        MicrophoneInactivityTailTrimActivityMode::PeakLevel,
    )
}

#[cfg(target_os = "macos")]
pub fn microphone_permission_state() -> CapturePermissionState {
    match av::CaptureDevice::authorization_status_for_media_type(av::MediaType::audio()) {
        Ok(av::AuthorizationStatus::Authorized) => CapturePermissionState::Granted,
        Ok(av::AuthorizationStatus::Denied | av::AuthorizationStatus::Restricted) => {
            CapturePermissionState::Denied
        }
        Ok(av::AuthorizationStatus::NotDetermined) => CapturePermissionState::NotDetermined,
        _ => CapturePermissionState::Unknown,
    }
}

#[cfg(target_os = "macos")]
pub fn ensure_microphone_permission() -> bool {
    match microphone_permission_state() {
        CapturePermissionState::Granted => return true,
        // `AssumedWorking`/`PossiblyBlocked` are system audio's inferred states
        // (ADR 0052) and unreachable here — the microphone can be *asked*
        // whether it was granted. Refusing on them keeps this total: a state we
        // cannot interpret must never be read as consent.
        CapturePermissionState::Denied
        | CapturePermissionState::Unsupported
        | CapturePermissionState::Unknown
        | CapturePermissionState::AssumedWorking
        | CapturePermissionState::PossiblyBlocked => return false,
        CapturePermissionState::NotDetermined => {}
    }

    let (tx, rx) = mpsc::channel::<bool>();
    let mut completion = cidre::blocks::SendBlock::new1(move |granted: bool| {
        let _ = tx.send(granted);
    });

    let request_result = av::CaptureDevice::request_access_for_media_type_ch(
        av::MediaType::audio(),
        &mut completion,
    );

    if request_result.is_err() {
        return matches!(
            microphone_permission_state(),
            CapturePermissionState::Granted
        );
    }

    if let Ok(granted) = rx.recv_timeout(Duration::from_secs(20)) {
        granted
    } else {
        matches!(
            microphone_permission_state(),
            CapturePermissionState::Granted
        )
    }
}

#[cfg(not(target_os = "macos"))]
pub fn microphone_permission_state() -> CapturePermissionState {
    CapturePermissionState::Unsupported
}

#[cfg(not(target_os = "macos"))]
pub fn ensure_microphone_permission() -> bool {
    false
}
