use capture_types::{
    CaptureErrorResponse, CapturePermissionState, MicrophoneControllerState, MicrophoneDevice,
    MicrophoneDisconnectPolicy, MicrophonePreference, MicrophonePreferenceMode,
};

#[cfg(target_os = "macos")]
use capture_writers::{
    append_audio_sample_to_writer_with_activity_override,
    create_audio_asset_writer_for_sample_format, derive_audio_activity_level_from_sample_buf,
    derive_audio_sample_format_from_sample_buf, record_audio_writer_tail_activity,
    finalize_microphone_output_context as writers_finalize_microphone_output_context,
    finish_audio_asset_writer_discarding_inactivity_tail, set_audio_writer_activity_threshold,
    set_audio_writer_inactivity_tail_trim_seconds, AudioAssetWriterState, AudioSampleFormat,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophoneInactivityTailTrimActivityMode {
    PeakLevel,
    VadSpeech,
}

pub const MICROPHONE_VAD_PCM_SAMPLE_RATE_HZ: u32 = 16_000;
pub const MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT: usize = 320;
#[cfg(target_os = "macos")]
const MAX_MICROPHONE_VAD_PCM_FRAMES: usize = 96;

#[derive(Debug, Clone, PartialEq)]
pub struct MicrophoneVadPcmFrame {
    pub sample_rate_hz: u32,
    pub captured_at_unix_ms: u64,
    pub normalized_peak_level: f32,
    pub samples: Vec<i16>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
struct MicrophoneVadPcmFeedState {
    source_format: Option<MicrophoneVadSourceFormat>,
    pending_source_samples: Vec<f32>,
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
pub fn reset_microphone_vad_tail_activity() {
    MICROPHONE_VAD_TAIL_SPEECH_SEQUENCE.store(0, Ordering::Relaxed);
}

#[cfg(not(target_os = "macos"))]
pub fn reset_microphone_vad_tail_activity() {}

#[cfg(target_os = "macos")]
pub fn record_microphone_vad_tail_speech() {
    MICROPHONE_VAD_TAIL_SPEECH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
}

#[cfg(not(target_os = "macos"))]
pub fn record_microphone_vad_tail_speech() {}

#[cfg(target_os = "macos")]
fn current_microphone_vad_tail_speech_sequence() -> u64 {
    MICROPHONE_VAD_TAIL_SPEECH_SEQUENCE.load(Ordering::Relaxed)
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
        &samples,
    );
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
    output_file: Option<String>,
    first_error: Option<CaptureErrorResponse>,
    format_state: MicFormatStabilityState,
    logged_format_samples: u32,
    pending_samples: VecDeque<BufferedMicSample>,
    inactivity_tail_trim_seconds: u64,
    activity_threshold: f32,
    tail_activity_mode: MicrophoneInactivityTailTrimActivityMode,
    observed_vad_tail_speech_sequence: u64,
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
const FORMAT_LOG_SAMPLE_LIMIT: u32 = 8;
#[cfg(target_os = "macos")]
const MAX_PENDING_MIC_SAMPLES: usize = 64;

#[cfg(target_os = "macos")]
fn record_observed_audio_format(
    context: &mut MicrophoneOutputContext,
    sample_format: AudioSampleFormat,
) {
    let was_stable = context.format_state.stable_format.is_some();
    observe_microphone_format(&mut context.format_state, sample_format);

    if context.logged_format_samples < FORMAT_LOG_SAMPLE_LIMIT {
        context.logged_format_samples += 1;
        capture_runtime::debug_log!(
            "[capture-microphone] sample_format_observed index={} sample_rate_hz={} channels={} bits_per_channel={} bytes_per_frame={} format_id={} format_flags={}",
            context.format_state.observed_format_count,
            sample_format.sample_rate_hz,
            sample_format.channels_per_frame,
            sample_format.bits_per_channel,
            sample_format.bytes_per_frame,
            sample_format.format_id,
            sample_format.format_flags,
        );
    }

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
mod tests {
    use super::{
        feed_microphone_vad_pcm_samples, last_microphone_activity_unix_ms,
        microphone_activity_idle_ms, microphone_activity_level,
        microphone_output_callback_objc_exception_error, microphone_output_callback_panic_error,
        microphone_tail_activity_override, mono_pcm_samples_from_audio_buffers,
        observe_microphone_format, peek_microphone_activity_window_peak_level,
        record_microphone_vad_tail_speech, reset_last_microphone_activity_unix_ms,
        reset_microphone_vad_tail_activity, resolve_microphone_finalize_format,
        resolve_microphone_live_format, store_microphone_activity,
        take_microphone_activity_window_peak_level, AudioSampleFormat, MicFormatStabilityState,
        MicrophoneInactivityTailTrimActivityMode, MicrophoneOutputContext,
        MicrophoneVadPcmFeedState, OnceLock, MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT,
        MICROPHONE_VAD_PCM_SAMPLE_RATE_HZ,
    };

    fn microphone_activity_state_test_guard() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        GUARD
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn tail_activity_context(
        mode: MicrophoneInactivityTailTrimActivityMode,
        tail_seconds: u64,
    ) -> MicrophoneOutputContext {
        MicrophoneOutputContext {
            writer: None,
            output_url: None,
            output_file: None,
            first_error: None,
            format_state: MicFormatStabilityState::default(),
            logged_format_samples: 0,
            pending_samples: std::collections::VecDeque::new(),
            inactivity_tail_trim_seconds: tail_seconds,
            activity_threshold: 0.0,
            tail_activity_mode: mode,
            observed_vad_tail_speech_sequence: 0,
        }
    }

    #[test]
    fn vad_tail_activity_override_consumes_speech_events_once() {
        let _guard = microphone_activity_state_test_guard();
        reset_microphone_vad_tail_activity();
        let mut context =
            tail_activity_context(MicrophoneInactivityTailTrimActivityMode::VadSpeech, 10);

        assert_eq!(microphone_tail_activity_override(&mut context), Some(false));
        record_microphone_vad_tail_speech();
        assert_eq!(microphone_tail_activity_override(&mut context), Some(true));
        assert_eq!(microphone_tail_activity_override(&mut context), Some(false));
        record_microphone_vad_tail_speech();
        assert_eq!(microphone_tail_activity_override(&mut context), Some(true));
    }

    #[test]
    fn peak_level_tail_activity_mode_uses_writer_detector() {
        let mut context =
            tail_activity_context(MicrophoneInactivityTailTrimActivityMode::PeakLevel, 10);

        assert_eq!(microphone_tail_activity_override(&mut context), None);
    }

    fn format(bits_per_channel: u32, bytes_per_frame: u32) -> AudioSampleFormat {
        AudioSampleFormat {
            sample_rate_hz: 48_000.0,
            format_id: 1,
            format_flags: 2,
            bytes_per_packet: bytes_per_frame,
            frames_per_packet: 1,
            bytes_per_frame,
            channels_per_frame: 2,
            bits_per_channel,
        }
    }

    fn pcm_format(
        sample_rate_hz: f64,
        channels_per_frame: u32,
        bits_per_channel: u32,
        format_flags: cidre::cat::AudioFormatFlags,
    ) -> AudioSampleFormat {
        let bytes_per_sample = bits_per_channel.saturating_add(7) / 8;
        AudioSampleFormat {
            sample_rate_hz,
            format_id: cidre::cat::AudioFormat::LINEAR_PCM.0,
            format_flags: format_flags.0,
            bytes_per_packet: bytes_per_sample * channels_per_frame,
            frames_per_packet: 1,
            bytes_per_frame: bytes_per_sample * channels_per_frame,
            channels_per_frame,
            bits_per_channel,
        }
    }

    fn audio_buffer(bytes: &[u8], channel_count: u32) -> cidre::cat::AudioBuf {
        cidre::cat::AudioBuf {
            number_channels: channel_count,
            data_bytes_size: bytes.len() as u32,
            data: bytes.as_ptr() as *mut u8,
        }
    }

    #[test]
    fn live_format_waits_until_stable_threshold_met() {
        let mut state = MicFormatStabilityState::default();
        let fmt = format(32, 8);

        for _ in 0..4 {
            observe_microphone_format(&mut state, fmt);
            assert_eq!(resolve_microphone_live_format(&state), None);
        }

        observe_microphone_format(&mut state, fmt);
        assert_eq!(resolve_microphone_live_format(&state), Some(fmt));
    }

    #[test]
    fn transient_24_bit_then_32_bit_stabilizes_on_32_bit() {
        let mut state = MicFormatStabilityState::default();
        let fmt24 = format(24, 6);
        let fmt32 = format(32, 8);

        observe_microphone_format(&mut state, fmt24);
        observe_microphone_format(&mut state, fmt24);
        observe_microphone_format(&mut state, fmt32);
        observe_microphone_format(&mut state, fmt32);
        assert_eq!(resolve_microphone_live_format(&state), None);

        observe_microphone_format(&mut state, fmt32);
        assert_eq!(resolve_microphone_live_format(&state), Some(fmt32));
    }

    #[test]
    fn finalize_format_falls_back_to_candidate_for_short_recording() {
        let mut state = MicFormatStabilityState::default();
        let fmt = format(32, 8);

        observe_microphone_format(&mut state, fmt);
        observe_microphone_format(&mut state, fmt);

        assert_eq!(resolve_microphone_live_format(&state), None);
        assert_eq!(resolve_microphone_finalize_format(&state), Some(fmt));
    }

    #[test]
    fn microphone_activity_state_tracks_latest_level_and_reset() {
        let _guard = microphone_activity_state_test_guard();
        reset_last_microphone_activity_unix_ms();

        store_microphone_activity(0.75, 10_000, 20_000);

        assert_eq!(last_microphone_activity_unix_ms(), Some(20_000));
        assert_eq!(microphone_activity_level(), Some(0.75));
        assert_eq!(microphone_activity_idle_ms(), Some(0));

        reset_last_microphone_activity_unix_ms();

        assert_eq!(last_microphone_activity_unix_ms(), None);
        assert_eq!(microphone_activity_level(), None);
        assert_eq!(microphone_activity_idle_ms(), None);
    }

    #[test]
    fn microphone_activity_window_peak_tracks_max_until_taken() {
        let _guard = microphone_activity_state_test_guard();
        reset_last_microphone_activity_unix_ms();

        store_microphone_activity(0.05, 10_000, 20_000);
        store_microphone_activity(0.80, 10_010, 20_010);
        store_microphone_activity(0.10, 10_020, 20_020);

        assert_eq!(take_microphone_activity_window_peak_level(), Some(0.80));
        assert_eq!(take_microphone_activity_window_peak_level(), None);
        assert_eq!(microphone_activity_level(), Some(0.10));

        reset_last_microphone_activity_unix_ms();
    }

    #[test]
    fn microphone_activity_window_peak_peek_preserves_value_until_taken() {
        let _guard = microphone_activity_state_test_guard();
        reset_last_microphone_activity_unix_ms();

        store_microphone_activity(0.05, 10_000, 20_000);
        store_microphone_activity(0.80, 10_010, 20_010);

        assert_eq!(peek_microphone_activity_window_peak_level(), Some(0.80));
        assert_eq!(peek_microphone_activity_window_peak_level(), Some(0.80));
        assert_eq!(take_microphone_activity_window_peak_level(), Some(0.80));
        assert_eq!(peek_microphone_activity_window_peak_level(), None);

        reset_last_microphone_activity_unix_ms();
    }

    #[test]
    fn vad_pcm_feed_chunks_48khz_mono_into_16khz_frames() {
        let mut state = MicrophoneVadPcmFeedState::default();
        let format = pcm_format(
            48_000.0,
            1,
            32,
            cidre::cat::AudioFormatFlags::IS_FLOAT | cidre::cat::AudioFormatFlags::IS_PACKED,
        );
        let samples = (0..960)
            .map(|index| index as f32 / 960.0)
            .collect::<Vec<_>>();

        feed_microphone_vad_pcm_samples(&mut state, format, 42_000, &samples);

        assert_eq!(state.output_frames.len(), 1);
        let frame = state.output_frames.pop_front().expect("frame should exist");
        assert_eq!(frame.sample_rate_hz, MICROPHONE_VAD_PCM_SAMPLE_RATE_HZ);
        assert_eq!(frame.captured_at_unix_ms, 42_000);
        assert_eq!(frame.samples.len(), MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT);
        assert_eq!(frame.samples[0], 0);
        assert_eq!(frame.samples[1], 102);
        assert!((frame.normalized_peak_level - (957.0 / 960.0)).abs() < 0.000_001);
    }

    #[test]
    fn vad_pcm_feed_resets_pending_samples_on_format_change() {
        let mut state = MicrophoneVadPcmFeedState::default();
        let fmt48 = pcm_format(
            48_000.0,
            1,
            32,
            cidre::cat::AudioFormatFlags::IS_FLOAT | cidre::cat::AudioFormatFlags::IS_PACKED,
        );
        let fmt16 = pcm_format(
            16_000.0,
            1,
            32,
            cidre::cat::AudioFormatFlags::IS_FLOAT | cidre::cat::AudioFormatFlags::IS_PACKED,
        );

        feed_microphone_vad_pcm_samples(&mut state, fmt48, 1, &vec![0.25; 480]);
        assert_eq!(state.output_frames.len(), 0);
        assert_eq!(state.pending_source_samples.len(), 480);

        feed_microphone_vad_pcm_samples(&mut state, fmt16, 2, &vec![0.50; 320]);

        assert_eq!(state.pending_source_samples.len(), 0);
        assert_eq!(state.output_frames.len(), 1);
        assert_eq!(state.output_frames[0].samples, vec![16_384; 320]);
        assert_eq!(state.output_frames[0].normalized_peak_level, 0.50);
    }

    #[test]
    fn vad_pcm_feed_is_bounded_to_recent_frames() {
        let mut state = MicrophoneVadPcmFeedState::default();
        let format = pcm_format(
            16_000.0,
            1,
            32,
            cidre::cat::AudioFormatFlags::IS_FLOAT | cidre::cat::AudioFormatFlags::IS_PACKED,
        );
        let frame = vec![0.0; MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT];

        for index in 0..120 {
            feed_microphone_vad_pcm_samples(&mut state, format, index, &frame);
        }

        assert_eq!(state.output_frames.len(), 96);
        assert_eq!(state.output_frames[0].captured_at_unix_ms, 24);
        assert_eq!(state.output_frames.back().unwrap().captured_at_unix_ms, 119);
    }

    #[test]
    fn vad_pcm_decoder_downmixes_interleaved_signed_pcm() {
        let format = pcm_format(
            48_000.0,
            2,
            16,
            cidre::cat::AudioFormatFlags::IS_SIGNED_INTEGER
                | cidre::cat::AudioFormatFlags::IS_PACKED,
        );
        let bytes = [
            16_384_i16.to_ne_bytes(),
            0_i16.to_ne_bytes(),
            0_i16.to_ne_bytes(),
            (-16_384_i16).to_ne_bytes(),
        ]
        .concat();
        let buffer = audio_buffer(&bytes, 2);

        let samples = mono_pcm_samples_from_audio_buffers(&[buffer], format).unwrap();

        assert_eq!(samples.len(), 2);
        assert!((samples[0] - 0.25).abs() < 0.000_1);
        assert!((samples[1] + 0.25).abs() < 0.000_1);
    }

    #[test]
    fn vad_pcm_decoder_downmixes_planar_float_pcm() {
        let format = pcm_format(
            48_000.0,
            2,
            32,
            cidre::cat::AudioFormatFlags::IS_FLOAT | cidre::cat::AudioFormatFlags::IS_PACKED,
        );
        let left = [0.75_f32.to_ne_bytes(), 0.25_f32.to_ne_bytes()].concat();
        let right = [0.25_f32.to_ne_bytes(), (-0.25_f32).to_ne_bytes()].concat();
        let left_buffer = audio_buffer(&left, 1);
        let right_buffer = audio_buffer(&right, 1);

        let samples =
            mono_pcm_samples_from_audio_buffers(&[left_buffer, right_buffer], format).unwrap();

        assert_eq!(samples, vec![0.5, 0.0]);
    }

    #[test]
    fn vad_pcm_decoder_rejects_non_native_or_non_pcm_formats() {
        let big_endian_format = pcm_format(
            48_000.0,
            1,
            16,
            cidre::cat::AudioFormatFlags::IS_SIGNED_INTEGER
                | cidre::cat::AudioFormatFlags::IS_PACKED
                | cidre::cat::AudioFormatFlags::IS_BIG_ENDIAN,
        );
        let zero = 0_i16.to_ne_bytes();
        let buffer = audio_buffer(&zero, 1);
        assert_eq!(
            mono_pcm_samples_from_audio_buffers(&[buffer], big_endian_format),
            None
        );

        let mut aac_format = big_endian_format;
        aac_format.format_id = cidre::cat::AudioFormat::MPEG4_AAC.0;
        assert_eq!(
            mono_pcm_samples_from_audio_buffers(&[buffer], aac_format),
            None
        );
    }

    fn microphone_callback_error_from_panic<F>(panic_fn: F) -> capture_types::CaptureErrorResponse
    where
        F: FnOnce(),
    {
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(panic_fn))
            .expect_err("panic should be caught");
        microphone_output_callback_panic_error(payload)
    }

    #[test]
    fn microphone_callback_panic_error_formats_static_str_payload() {
        let error = microphone_callback_error_from_panic(|| panic!("mic boom"));
        assert_eq!(error.code, "capture_output_processing_failed");
        assert_eq!(
            error.message,
            "Microphone output callback panicked: mic boom"
        );
    }

    #[test]
    fn microphone_callback_panic_error_formats_string_payload() {
        let error = microphone_callback_error_from_panic(|| {
            std::panic::panic_any(String::from("owned mic boom"));
        });
        assert_eq!(error.code, "capture_output_processing_failed");
        assert_eq!(
            error.message,
            "Microphone output callback panicked: owned mic boom"
        );
    }

    #[test]
    fn microphone_callback_panic_error_handles_non_string_payloads() {
        let error = microphone_callback_error_from_panic(|| {
            std::panic::panic_any(42_u8);
        });
        assert_eq!(error.code, "capture_output_processing_failed");
        assert_eq!(
            error.message,
            "Microphone output callback panicked with a non-string payload"
        );
    }

    #[test]
    fn microphone_callback_objc_exception_error_contains_expected_wording() {
        let reason = cidre::ns::str!(c"test mic reason");
        let exception =
            cidre::ns::try_catch(|| cidre::ns::Exception::raise(reason)).expect_err("should catch");
        let error = microphone_output_callback_objc_exception_error(exception);
        assert_eq!(error.code, "capture_output_processing_failed");
        assert!(
            error
                .message
                .contains("Microphone output callback ObjC exception"),
            "unexpected message: {}",
            error.message
        );
        assert!(
            error.message.contains("test mic reason"),
            "unexpected message: {}",
            error.message
        );
    }
}

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
    state.stable_format
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
        state.source_format = Some(source_format);
    }

    state.pending_source_samples.extend_from_slice(samples);

    let input_frame_count = vad_input_frame_count_for_output_frame(source_format.sample_rate_hz);
    while state.pending_source_samples.len() >= input_frame_count {
        let output_samples = resample_microphone_vad_frame(
            &state.pending_source_samples[..input_frame_count],
            source_format.sample_rate_hz,
        );
        state.pending_source_samples.drain(..input_frame_count);

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
            samples,
        });

        while state.output_frames.len() > MAX_MICROPHONE_VAD_PCM_FRAMES {
            let _ = state.output_frames.pop_front();
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
) -> Result<(), CaptureErrorResponse> {
    let activity_override = microphone_tail_activity_override(context);
    let Some(writer) = context.writer.as_mut() else {
        return Ok(());
    };

    let activity_override = if activity_override == Some(true) && record_audio_writer_tail_activity(writer)? {
        Some(false)
    } else {
        activity_override
    };

    append_audio_sample_to_writer_with_activity_override(writer, sample_buf, activity_override)
}

#[cfg(target_os = "macos")]
fn flush_pending_microphone_samples(
    context: &mut MicrophoneOutputContext,
) -> Result<(), CaptureErrorResponse> {
    let selected_format = fallback_microphone_format(context);
    if context.writer.is_none() {
        return Ok(());
    }

    while let Some(sample) = context.pending_samples.pop_front() {
        if selected_format.is_some() && Some(sample.format) != selected_format {
            continue;
        }

        append_microphone_sample_to_writer(context, sample.sample_buf.as_ref())?;
    }

    Ok(())
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

    capture_runtime::debug_log!(
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

    capture_runtime::debug_log!(
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
        append_microphone_sample_to_writer, create_audio_asset_writer_for_sample_format,
        derive_audio_sample_format_from_sample_buf, flush_pending_microphone_samples,
        maybe_feed_microphone_vad_pcm, maybe_track_microphone_activity,
        microphone_output_callback_objc_exception_error, microphone_output_callback_panic_error,
        ns, objc, record_observed_audio_format, resolve_microphone_live_format, BufferedMicSample,
        MicrophoneOutputContext, MAX_PENDING_MIC_SAMPLES,
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

                    if ctx.output_url.is_none() {
                        return;
                    }

                    if ctx.writer.is_some() {
                        if let Err(error) = append_microphone_sample_to_writer(ctx, sample_buf) {
                            ctx.first_error = Some(error);
                        }
                        return;
                    }

                    let Some(sample_format) =
                        derive_audio_sample_format_from_sample_buf(sample_buf)
                    else {
                        return;
                    };

                    record_observed_audio_format(ctx, sample_format);

                    ctx.pending_samples.push_back(BufferedMicSample {
                        sample_buf: sample_buf.retained(),
                        format: sample_format,
                    });
                    while ctx.pending_samples.len() > MAX_PENDING_MIC_SAMPLES {
                        let _ = ctx.pending_samples.pop_front();
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
    }

    pub fn stop_for_inactivity(
        &mut self,
        tail_trim_seconds: u64,
        activity_threshold: f32,
    ) -> Result<(), CaptureErrorResponse> {
        self.stop_with_inactivity_tail_trim_seconds(tail_trim_seconds, activity_threshold)
    }

    pub fn stop_for_inactivity_with_tail_activity_mode(
        &mut self,
        tail_trim_seconds: u64,
        activity_threshold: f32,
        tail_activity_mode: MicrophoneInactivityTailTrimActivityMode,
    ) -> Result<(), CaptureErrorResponse> {
        self.stop_with_inactivity_tail_trim_seconds_and_activity_mode(
            tail_trim_seconds,
            activity_threshold,
            tail_activity_mode,
        )
    }

    fn stop_with_inactivity_tail_trim_seconds(
        &mut self,
        tail_trim_seconds: u64,
        activity_threshold: f32,
    ) -> Result<(), CaptureErrorResponse> {
        self.stop_with_inactivity_tail_trim_seconds_and_activity_mode(
            tail_trim_seconds,
            activity_threshold,
            MicrophoneInactivityTailTrimActivityMode::PeakLevel,
        )
    }

    fn stop_with_inactivity_tail_trim_seconds_and_activity_mode(
        &mut self,
        tail_trim_seconds: u64,
        activity_threshold: f32,
        tail_activity_mode: MicrophoneInactivityTailTrimActivityMode,
    ) -> Result<(), CaptureErrorResponse> {
        self.capture_session.stop_running();
        synchronize_stream_output_queue(Some(self.output_queue.as_ref()));
        let context = self.output_delegate.inner_mut();
        context.inactivity_tail_trim_seconds = tail_trim_seconds;
        context.activity_threshold = normalized_audio_activity_threshold(activity_threshold);
        context.tail_activity_mode = tail_activity_mode;
        finalize_microphone_output_context(self.output_delegate.inner_mut())
    }

    pub fn rotate_output_file(&mut self, output_file: &str) -> Result<(), CaptureErrorResponse> {
        let output_url = cidre::ns::Url::with_fs_path_str(output_file, false);

        synchronize_stream_output_queue(Some(self.output_queue.as_ref()));
        let current_context = self.output_delegate.inner_mut();
        let next_context = microphone_output_context_for_output_url(
            &output_url,
            Some(output_file.to_string()),
            current_context.inactivity_tail_trim_seconds,
            current_context.activity_threshold,
            current_context.tail_activity_mode,
        );
        let mut previous_context = std::mem::replace(current_context, next_context);
        finalize_microphone_output_context(&mut previous_context)?;

        Ok(())
    }

    pub fn pause_output_file(&mut self) -> Result<(), CaptureErrorResponse> {
        self.pause_output_file_for_inactivity(0, 0.0)
    }

    pub fn pause_output_file_for_inactivity(
        &mut self,
        tail_trim_seconds: u64,
        activity_threshold: f32,
    ) -> Result<(), CaptureErrorResponse> {
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
    ) -> Result<(), CaptureErrorResponse> {
        synchronize_stream_output_queue(Some(self.output_queue.as_ref()));
        let mut previous_context = std::mem::replace(
            self.output_delegate.inner_mut(),
            microphone_probe_only_context(),
        );
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
        let output_url = cidre::ns::Url::with_fs_path_str(output_file, false);
        let next_context = microphone_output_context_for_output_url(
            &output_url,
            Some(output_file.to_string()),
            tail_trim_seconds,
            activity_threshold,
            tail_activity_mode,
        );

        synchronize_stream_output_queue(Some(self.output_queue.as_ref()));
        let mut previous_context =
            std::mem::replace(self.output_delegate.inner_mut(), next_context);
        finalize_microphone_output_context(&mut previous_context)?;

        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn synchronize_stream_output_queue(queue: Option<&dispatch::Queue>) {
    if let Some(queue) = queue {
        queue.sync(|| ());
    }
}

#[cfg(target_os = "macos")]
fn finalize_microphone_output_context(
    context: &mut MicrophoneOutputContext,
) -> Result<(), CaptureErrorResponse> {
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
        ) && microphone_tail_activity_override(context) == Some(true);

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
            if let Some(path) = context.output_file.as_deref() {
                maybe_remove_microphone_output_file(path);
            }
            Ok(())
        }
        result => result,
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
    MicrophoneOutputContext {
        writer: None,
        output_url: Some(output_url.retained()),
        output_file,
        first_error: None,
        format_state: MicFormatStabilityState::default(),
        logged_format_samples: 0,
        pending_samples: VecDeque::new(),
        inactivity_tail_trim_seconds,
        activity_threshold: normalized_audio_activity_threshold(activity_threshold),
        tail_activity_mode,
        observed_vad_tail_speech_sequence: current_microphone_vad_tail_speech_sequence(),
    }
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
        output_file: None,
        first_error: None,
        format_state: MicFormatStabilityState::default(),
        logged_format_samples: 0,
        pending_samples: VecDeque::new(),
        inactivity_tail_trim_seconds: 0,
        activity_threshold: 0.0,
        tail_activity_mode: MicrophoneInactivityTailTrimActivityMode::PeakLevel,
        observed_vad_tail_speech_sequence: current_microphone_vad_tail_speech_sequence(),
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
        Err(error) => capture_runtime::debug_log!(
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
    let output_delegate =
        MicAudioDataOutputDelegate::with(microphone_output_context_for_output_url(
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
        CapturePermissionState::Denied
        | CapturePermissionState::Unsupported
        | CapturePermissionState::Unknown => return false,
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
