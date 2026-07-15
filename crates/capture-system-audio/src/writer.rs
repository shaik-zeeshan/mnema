//! Tap deliveries in, m4a segments out.
//!
//! **Writer format follows the tap** (ADR 0052): the tap's ASBD is
//! device-dependent (44.1 ↔ 48 kHz) and changes across rebuilds, so the pinned
//! `DEFAULT_AUDIO_WRITER_FORMAT` the ScreenCaptureKit path used is not an option
//! here. Instead this mirrors the microphone's proven pattern
//! (`MicFormatStabilityState`, `capture-microphone/src/lib.rs`): derive the
//! format from the buffers actually delivered, and create the writer lazily once
//! that format holds still. Nothing resamples — nothing downstream reads the
//! recorded rate.

use std::path::PathBuf;

use capture_types::CaptureErrorResponse;
use capture_writers::{
    append_audio_sample_to_writer, create_audio_asset_writer_for_sample_format,
    derive_audio_sample_format_from_sample_buf, finish_audio_asset_writer, no_audio_samples_error,
    AudioAssetWriterState, AudioSampleFormat,
};
use cidre::{arc, cat, cf, cm, ns, os};

use crate::LOG_PREFIX;

// cidre wraps neither `CMSampleBufferSetDataBufferFromAudioBufferList` nor a
// safe `CMSampleBufferCreate` for audio, so the one call is declared against the
// CoreMedia framework cidre already links (same shape as `tap.rs`'s CoreAudio
// externs).
#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C-unwind" {
    fn CMSampleBufferSetDataBufferFromAudioBufferList(
        sample_buf: &cm::SampleBuf,
        block_buf_structure_allocator: Option<&cf::Allocator>,
        block_buf_block_allocator: Option<&cf::Allocator>,
        flags: u32,
        buffer_list: *const cat::AudioBufList<MAX_TAP_AUDIO_BUFFERS>,
    ) -> os::Status;
}

/// `kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment`. The call copies
/// the IOProc's buffers either way — it has to, they only live for the duration
/// of the callback — and the copy is what lets us align it for the encoder.
const ASSURE_16_BYTE_ALIGNMENT: u32 = 1 << 0;

/// A stereo global tap delivers one interleaved buffer, or one per channel on a
/// non-interleaved device. Eight is slack; a delivery wider than this is dropped
/// rather than read past the end of the list we rebuild for CoreMedia.
const MAX_TAP_AUDIO_BUFFERS: usize = 8;

/// Mirrors the microphone's thresholds: a format is stable once the same one has
/// arrived three times running, with at least five deliveries seen.
const FORMAT_STABILITY_REQUIRED_CONSECUTIVE: u32 = 3;
const FORMAT_STABILITY_MIN_OBSERVED: u32 = 5;
const FORMAT_LOG_SAMPLE_LIMIT: u32 = 8;

const WRITER_LABEL: &str = "system audio";

/// The observed-format state machine, one per tap generation.
#[derive(Debug, Default, Clone, Copy)]
pub struct FormatStabilityState {
    observed_format_count: u32,
    candidate_format: Option<AudioSampleFormat>,
    candidate_format_streak: u32,
    stable_format: Option<AudioSampleFormat>,
}

/// A format only becomes stable once, per tap generation. A rebuild mints a
/// fresh state, which is exactly how a changed ASBD gets adopted rather than
/// rejected: the new generation has no previous format to disagree with.
pub fn observe_system_audio_format(state: &mut FormatStabilityState, format: AudioSampleFormat) {
    state.observed_format_count += 1;

    match state.candidate_format {
        Some(candidate) if candidate == format => state.candidate_format_streak += 1,
        _ => {
            state.candidate_format = Some(format);
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

/// The format a writer may be created with, or `None` while the tap is still
/// settling.
pub fn resolve_system_audio_writer_format(
    state: &FormatStabilityState,
) -> Option<AudioSampleFormat> {
    state.stable_format
}

/// One tap delivery, turned into something the shared writers accept.
pub struct SystemAudioSample {
    pub sample_buf: arc::R<cm::SampleBuf>,
    /// `Some` once the generation's format has settled; a writer may be created
    /// with it, and every segment after the first in this generation gets it
    /// immediately.
    pub writer_format: Option<AudioSampleFormat>,
}

/// Turns a tap generation's raw IOProc deliveries into `cm::SampleBuf`s, and
/// tracks the format they carry. Lives for the generation, so a rotation never
/// re-pays the settling cost — only a rebuild does.
pub struct SystemAudioSampleBuilder {
    asbd: cat::AudioStreamBasicDesc,
    format_desc: arc::R<cm::AudioFormatDesc>,
    format_state: FormatStabilityState,
    logged_format_samples: u32,
    /// Frame counter behind `sample_time`, for the timestamps that arrive
    /// without it marked valid.
    next_fallback_frame: i64,
}

impl SystemAudioSampleBuilder {
    pub fn new(asbd: cat::AudioStreamBasicDesc) -> Result<Self, CaptureErrorResponse> {
        let format_desc =
            cm::AudioFormatDesc::with_asbd(&asbd).map_err(|error| CaptureErrorResponse {
                code: "system_audio_tap_start_failed".to_string(),
                message: format!("describe tap format: {error:?}"),
            })?;

        Ok(Self {
            asbd,
            format_desc,
            format_state: FormatStabilityState::default(),
            logged_format_samples: 0,
            next_fallback_frame: 0,
        })
    }

    pub fn build(
        &mut self,
        time: &cat::AudioTimeStamp,
        buffers: &[cat::AudioBuf],
    ) -> Option<SystemAudioSample> {
        let sample_buf = self.sample_buf(time, buffers)?;

        let format = derive_audio_sample_format_from_sample_buf(&sample_buf)?;
        self.record_observed_format(format);

        Some(SystemAudioSample {
            sample_buf,
            writer_format: resolve_system_audio_writer_format(&self.format_state),
        })
    }

    fn sample_buf(
        &mut self,
        time: &cat::AudioTimeStamp,
        buffers: &[cat::AudioBuf],
    ) -> Option<arc::R<cm::SampleBuf>> {
        if buffers.is_empty() || buffers.len() > MAX_TAP_AUDIO_BUFFERS {
            return None;
        }
        if self.asbd.bytes_per_frame == 0 || self.asbd.sample_rate <= 0.0 {
            return None;
        }

        // Interleaved or not, `bytes_per_frame` is per buffer by CoreAudio's own
        // convention (a non-interleaved ASBD describes one channel's frame), so
        // one division covers both layouts.
        let frames = buffers[0].data_bytes_size as usize / self.asbd.bytes_per_frame as usize;
        if frames == 0 {
            return None;
        }

        let mut buffer_list = cat::AudioBufList::<MAX_TAP_AUDIO_BUFFERS> {
            number_buffers: buffers.len() as u32,
            buffers: [cat::AudioBuf {
                number_channels: 0,
                data_bytes_size: 0,
                data: std::ptr::null_mut(),
            }; MAX_TAP_AUDIO_BUFFERS],
        };
        buffer_list.buffers[..buffers.len()].copy_from_slice(buffers);

        let sample_rate = self.asbd.sample_rate as i32;
        let timing = cm::SampleTimingInfo {
            duration: cm::Time::new(1, sample_rate),
            pts: cm::Time::new(self.presentation_frame(time, frames), sample_rate),
            dts: cm::Time::invalid(),
        };

        let mut sample_buf = unsafe {
            let mut created = None;
            cm::SampleBuf::create_in(
                None,
                None,
                false,
                None,
                std::ptr::null(),
                Some(&self.format_desc),
                frames as cm::ItemCount,
                1,
                &timing,
                0,
                std::ptr::null(),
                &mut created,
            )
            .ok()?;
            created?
        };

        unsafe {
            CMSampleBufferSetDataBufferFromAudioBufferList(
                &sample_buf,
                None,
                None,
                ASSURE_16_BYTE_ALIGNMENT,
                &buffer_list,
            )
            .result()
            .ok()?;
        }
        sample_buf.set_data_ready();

        Some(sample_buf)
    }

    /// The tap's own sample clock, which does not start at zero — harmless, the
    /// asset writer starts its session at the first sample's PTS and a rebuild
    /// starts a new segment rather than splicing generations into one file.
    fn presentation_frame(&mut self, time: &cat::AudioTimeStamp, frames: usize) -> i64 {
        let sample_time_valid = time.flags.0 & cat::AudioTimeStampFlags::SAMPLE_TIME_VALID.0 != 0;
        let frame = if sample_time_valid {
            time.sample_time as i64
        } else {
            self.next_fallback_frame
        };
        self.next_fallback_frame = frame + frames as i64;
        frame
    }

    fn record_observed_format(&mut self, format: AudioSampleFormat) {
        let was_stable = self.format_state.stable_format.is_some();
        observe_system_audio_format(&mut self.format_state, format);

        if self.logged_format_samples < FORMAT_LOG_SAMPLE_LIMIT {
            self.logged_format_samples += 1;
            capture_runtime::debug_log!(
                "{LOG_PREFIX} sample_format_observed index={} sample_rate_hz={} channels={} bits_per_channel={} bytes_per_frame={} format_id={} format_flags={}",
                self.format_state.observed_format_count,
                format.sample_rate_hz,
                format.channels_per_frame,
                format.bits_per_channel,
                format.bytes_per_frame,
                format.format_id,
                format.format_flags,
            );
        }

        if !was_stable {
            if let Some(stable_format) = self.format_state.stable_format {
                capture_runtime::debug_log!(
                    "{LOG_PREFIX} sample_format_stabilized observed={} streak={} sample_rate_hz={} channels={}",
                    self.format_state.observed_format_count,
                    self.format_state.candidate_format_streak,
                    stable_format.sample_rate_hz,
                    stable_format.channels_per_frame,
                );
            }
        }
    }
}

/// The segment currently being written, or the absence of one: system audio
/// paused for inactivity keeps the tap (and so the watchdog) alive with no
/// output file.
pub struct SystemAudioOutputContext {
    output_file: Option<PathBuf>,
    output_url: Option<arc::R<ns::Url>>,
    writer: Option<AudioAssetWriterState>,
    first_error: Option<CaptureErrorResponse>,
}

/// What Slice 5 hands back to the `SegmentPlanner` when a segment closes.
#[derive(Debug)]
pub struct SystemAudioSegmentFinalization {
    pub output_file: PathBuf,
    /// `Err` for a segment that never got a usable sample — a paused-through
    /// segment or a tap that only ever delivered a format the writer refused.
    pub result: Result<(), CaptureErrorResponse>,
}

impl SystemAudioOutputContext {
    pub fn new(output_file: Option<PathBuf>) -> Self {
        let output_url = output_file
            .as_deref()
            .map(|path| ns::Url::with_fs_path_str(&path.to_string_lossy(), false));

        Self {
            output_file,
            output_url,
            writer: None,
            first_error: None,
        }
    }

    /// Appends one delivery, creating the writer on the first sample whose
    /// format has settled.
    pub fn append(&mut self, sample: &SystemAudioSample) {
        if self.first_error.is_some() {
            return;
        }
        let Some(output_url) = self.output_url.as_ref() else {
            return;
        };

        if self.writer.is_none() {
            // ponytail: the handful of deliveries before the format settles
            // (~50 ms, once per tap generation) are dropped rather than queued
            // the way the microphone queues its own. Add a bounded pending queue
            // if a segment head ever needs to be sample-accurate.
            let Some(writer_format) = sample.writer_format else {
                return;
            };
            match create_audio_asset_writer_for_sample_format(
                output_url,
                WRITER_LABEL,
                writer_format,
            ) {
                Ok(writer) => self.writer = Some(writer),
                Err(error) => {
                    self.first_error = Some(error);
                    return;
                }
            }
        }

        let writer = self
            .writer
            .as_mut()
            .expect("system audio writer exists once its format settled");
        if let Err(error) = append_audio_sample_to_writer(writer, &sample.sample_buf) {
            self.first_error = Some(error);
        }
    }

    /// Closes the segment. Never called on the IOProc's queue: finishing an
    /// asset writer blocks.
    pub fn finalize(mut self) -> Option<SystemAudioSegmentFinalization> {
        let output_file = self.output_file.take()?;

        let mut result = match self.writer.as_mut() {
            Some(writer) => finish_audio_asset_writer(writer),
            None => Err(no_audio_samples_error(WRITER_LABEL)),
        };
        if let Some(error) = self.first_error.take() {
            result = Err(error);
        }

        Some(SystemAudioSegmentFinalization {
            output_file,
            result,
        })
    }
}

/// Synthetic tap deliveries, for the tests that must not need a sound card.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    /// The shape a stereo global tap actually delivers: packed native-endian
    /// f32, interleaved.
    pub(crate) fn asbd(sample_rate: f64, channels: u32) -> cat::AudioStreamBasicDesc {
        cat::AudioStreamBasicDesc {
            sample_rate,
            format: cat::AudioFormat::LINEAR_PCM,
            format_flags: cat::AudioFormatFlags(
                cat::AudioFormatFlags::IS_FLOAT.0 | cat::AudioFormatFlags::IS_PACKED.0,
            ),
            bytes_per_packet: 4 * channels,
            frames_per_packet: 1,
            bytes_per_frame: 4 * channels,
            channels_per_frame: channels,
            bits_per_channel: 32,
            reserved: 0,
        }
    }

    pub(crate) fn timestamp(sample_time: f64) -> cat::AudioTimeStamp {
        let mut time = cat::AudioTimeStamp::invalid();
        time.sample_time = sample_time;
        time.flags = cat::AudioTimeStampFlags::SAMPLE_TIME_VALID;
        time
    }

    /// Borrows `pcm` exactly as an IOProc borrows its buffers: for the length of
    /// the call and no longer.
    pub(crate) fn buffers(pcm: &mut [f32], channels: u32) -> [cat::AudioBuf; 1] {
        [cat::AudioBuf {
            number_channels: channels,
            data_bytes_size: std::mem::size_of_val(pcm) as u32,
            data: pcm.as_mut_ptr().cast(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{asbd, buffers, timestamp};
    use super::*;

    fn format(sample_rate_hz: f64, channels: u32) -> AudioSampleFormat {
        AudioSampleFormat {
            sample_rate_hz,
            format_id: cat::AudioFormat::LINEAR_PCM.0,
            format_flags: 0,
            bytes_per_packet: 4 * channels,
            frames_per_packet: 1,
            bytes_per_frame: 4 * channels,
            channels_per_frame: channels,
            bits_per_channel: 32,
        }
    }

    fn deliver(
        builder: &mut SystemAudioSampleBuilder,
        sample_time: f64,
        pcm: &mut [f32],
        channels: u32,
    ) -> Option<SystemAudioSample> {
        builder.build(&timestamp(sample_time), &buffers(pcm, channels))
    }

    #[test]
    fn format_is_adopted_only_once_it_holds_still() {
        let mut state = FormatStabilityState::default();
        for _ in 0..4 {
            observe_system_audio_format(&mut state, format(48_000.0, 2));
            assert_eq!(resolve_system_audio_writer_format(&state), None);
        }

        observe_system_audio_format(&mut state, format(48_000.0, 2));
        assert_eq!(
            resolve_system_audio_writer_format(&state),
            Some(format(48_000.0, 2))
        );
    }

    #[test]
    fn a_flapping_format_never_settles() {
        let mut state = FormatStabilityState::default();
        for index in 0..20 {
            let rate = if index % 2 == 0 { 48_000.0 } else { 44_100.0 };
            observe_system_audio_format(&mut state, format(rate, 2));
        }
        assert_eq!(resolve_system_audio_writer_format(&state), None);
    }

    // The writer is created from the settled format and never re-derived: a
    // later disagreement is a rebuild's business, not the writer's.
    #[test]
    fn a_settled_format_is_not_replaced_in_place() {
        let mut state = FormatStabilityState::default();
        for _ in 0..5 {
            observe_system_audio_format(&mut state, format(44_100.0, 2));
        }
        for _ in 0..10 {
            observe_system_audio_format(&mut state, format(48_000.0, 2));
        }
        assert_eq!(
            resolve_system_audio_writer_format(&state),
            Some(format(44_100.0, 2))
        );
    }

    // A rebuild mints a fresh builder, which is how the device-dependent ASBD
    // (44.1 ↔ 48 kHz across a device switch) gets adopted rather than rejected.
    #[test]
    fn each_tap_generation_adopts_the_format_its_buffers_carry() {
        let mut pcm = vec![0.25_f32; 256];

        for (rate, channels) in [(44_100.0, 2_u32), (48_000.0, 2), (48_000.0, 1)] {
            let mut builder =
                SystemAudioSampleBuilder::new(asbd(rate, channels)).expect("builder starts");

            let mut settled = None;
            for index in 0..FORMAT_STABILITY_MIN_OBSERVED {
                let sample = deliver(&mut builder, index as f64 * 128.0, &mut pcm, channels)
                    .expect("delivery becomes a sample buffer");
                settled = sample.writer_format;
            }

            let settled = settled.expect("format settles within the observation window");
            assert_eq!(settled.sample_rate_hz, rate);
            assert_eq!(settled.channels_per_frame, channels);
        }
    }

    #[test]
    fn an_empty_delivery_is_not_a_sample() {
        let mut builder = SystemAudioSampleBuilder::new(asbd(48_000.0, 2)).expect("builder starts");
        let time = cat::AudioTimeStamp::invalid();
        assert!(builder.build(&time, &[]).is_none());
    }
}
