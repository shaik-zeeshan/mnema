use super::{
    carry_probe_only_microphone_context, derive_audio_sample_format_from_sample_buf,
    feed_microphone_vad_pcm_samples, finalize_microphone_output_context,
    flush_pending_microphone_sample_queue, last_microphone_activity_unix_ms,
    microphone_activity_idle_ms, microphone_activity_level,
    microphone_output_callback_objc_exception_error, microphone_output_callback_panic_error,
    microphone_output_context_for_output_url, microphone_tail_activity_override,
    mono_pcm_samples_from_audio_buffers, observe_microphone_format,
    peek_microphone_activity_window_peak_level, push_bounded_pending_microphone_sample,
    record_microphone_vad_tail_speech, replace_microphone_output_context_for_rotation,
    reset_last_microphone_activity_unix_ms, reset_microphone_vad_tail_activity,
    resolve_microphone_finalize_format, resolve_microphone_live_format,
    resume_microphone_output_context, store_microphone_activity,
    take_microphone_activity_window_peak_level, transfer_pending_microphone_samples,
    AudioSampleAppendDisposition, AudioSampleFormat, BufferedMicSample, MicFormatStabilityState,
    MicrophoneInactivityTailTrimActivityMode, MicrophoneOutputContext, MicrophoneVadPcmFeedState,
    MicrophoneVadSpeechEvent, OnceLock, MAX_PENDING_MIC_SAMPLES,
    MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT, MICROPHONE_VAD_PCM_SAMPLE_RATE_HZ,
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
        source_output_file: None,
        output_file: None,
        first_error: None,
        format_state: MicFormatStabilityState::default(),
        pending_samples: std::collections::VecDeque::new(),
        inactivity_tail_trim_seconds: tail_seconds,
        activity_threshold: 0.0,
        tail_activity_mode: mode,
        observed_vad_tail_speech_sequence: 0,
        boundary_trim_enabled: false,
        boundary_trim_disabled: false,
        observed_boundary_trim_disabled_sequence: 0,
        first_speech_start_secs: None,
        last_speech_end_secs: None,
        detected_vad_speech: false,
        resumed_at_host_secs: None,
        resume_carried_secs: 0.0,
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
fn vad_speech_events_coalesce_first_and_last_media_boundaries() {
    let _guard = microphone_activity_state_test_guard();
    reset_microphone_vad_tail_activity();

    super::record_microphone_vad_speech_event(MicrophoneVadSpeechEvent {
        media_start_secs: Some(3.0),
        media_end_secs: Some(3.5),
    });
    super::record_microphone_vad_speech_event(MicrophoneVadSpeechEvent {
        media_start_secs: Some(1.25),
        media_end_secs: Some(1.75),
    });
    super::record_microphone_vad_speech_event(MicrophoneVadSpeechEvent {
        media_start_secs: Some(8.0),
        media_end_secs: Some(8.2),
    });

    let boundaries = super::current_microphone_vad_speech_boundaries();
    assert!(boundaries.detected_speech);
    assert_eq!(boundaries.first_start_secs, Some(1.25));
    assert_eq!(boundaries.last_end_secs, Some(8.2));
    assert!(!boundaries.invalid_timing);
}

#[test]
fn vad_boundary_trim_finalization_consumes_boundaries_per_output() {
    let _guard = microphone_activity_state_test_guard();
    reset_microphone_vad_tail_activity();

    let mut first = tail_activity_context(MicrophoneInactivityTailTrimActivityMode::VadSpeech, 10);
    first.boundary_trim_enabled = true;
    super::record_microphone_vad_speech_event(MicrophoneVadSpeechEvent {
        media_start_secs: Some(1.0),
        media_end_secs: Some(1.25),
    });

    let first_finalization = super::finalize_microphone_vad_boundary_trim(&mut first)
        .expect("boundary finalization should tolerate a missing source file");
    assert_eq!(first.first_speech_start_secs, Some(1.0));
    assert_eq!(first.last_speech_end_secs, Some(1.25));
    assert_eq!(
        first_finalization.discard_reason.as_deref(),
        Some("missing_source_file")
    );

    let mut second = tail_activity_context(MicrophoneInactivityTailTrimActivityMode::VadSpeech, 10);
    second.boundary_trim_enabled = true;
    super::record_microphone_vad_speech_event(MicrophoneVadSpeechEvent {
        media_start_secs: Some(8.0),
        media_end_secs: Some(8.25),
    });

    let second_finalization = super::finalize_microphone_vad_boundary_trim(&mut second)
        .expect("boundary finalization should tolerate a missing source file");
    assert_eq!(second.first_speech_start_secs, Some(8.0));
    assert_eq!(second.last_speech_end_secs, Some(8.25));
    assert_eq!(
        second_finalization.discard_reason.as_deref(),
        Some("missing_source_file")
    );
}

fn speech_boundaries(
    first_start_secs: f64,
    last_end_secs: f64,
) -> super::MicrophoneVadSpeechBoundaryState {
    super::MicrophoneVadSpeechBoundaryState {
        first_start_secs: Some(first_start_secs),
        last_end_secs: Some(last_end_secs),
        detected_speech: true,
        invalid_timing: false,
    }
}

#[test]
fn vad_trim_range_is_relative_to_the_writer_session_start() {
    // Host clock ~3 days after boot; speech 40-100 s into the file.
    let session_start = 259_200.0;
    assert_eq!(
        super::vad_trim::microphone_vad_trim_range_secs(
            &speech_boundaries(session_start + 40.0, session_start + 100.0),
            Some(session_start),
            0.0,
        ),
        Some((39.0, 101.0))
    );

    // Speech at the very start never trims before frame 0.
    assert_eq!(
        super::vad_trim::microphone_vad_trim_range_secs(
            &speech_boundaries(session_start, session_start + 5.0),
            Some(session_start),
            0.0,
        )
        .map(|(start, _)| start),
        Some(0.0)
    );

    // Speech carried over from before this file clamps to its first sample.
    assert_eq!(
        super::vad_trim::microphone_vad_trim_range_secs(
            &speech_boundaries(session_start - 3.0, session_start + 5.0),
            Some(session_start),
            0.0,
        ),
        Some((0.0, 6.0))
    );
}

#[test]
fn vad_trim_drops_speech_that_ended_before_the_file_started() {
    let session_start = 1_000.0;
    assert_eq!(
        super::vad_trim::microphone_vad_trim_range_secs(
            &speech_boundaries(session_start - 2.0, session_start - 0.5),
            Some(session_start),
            0.0,
        ),
        None,
        "never a trim to nothing; finalization discards such a file instead"
    );
}

#[test]
fn vad_trim_keeps_a_file_with_a_timeline_gap_whole() {
    // Buffers dropped mid-file (a Bluetooth profile flip) shift every later
    // sample earlier in the file than its host time says.
    let session_start = 1_000.0;
    let boundaries = speech_boundaries(session_start + 40.0, session_start + 100.0);
    assert_eq!(
        super::vad_trim::microphone_vad_trim_range_secs(&boundaries, Some(session_start), 0.0),
        Some((39.0, 101.0))
    );
    assert_eq!(
        super::vad_trim::microphone_vad_trim_range_secs(&boundaries, Some(session_start), 2.5),
        None
    );
}

#[test]
fn dropped_vad_frames_keep_the_output_whole() {
    let _guard = microphone_activity_state_test_guard();
    reset_microphone_vad_tail_activity();
    let mut context =
        tail_activity_context(MicrophoneInactivityTailTrimActivityMode::VadSpeech, 10);
    context.boundary_trim_enabled = true;

    // A capture tick stalled past the queue: the oldest frames are lost.
    let mut feed = MicrophoneVadPcmFeedState::default();
    let format = pcm_format(
        16_000.0,
        1,
        32,
        cidre::cat::AudioFormatFlags::IS_FLOAT | cidre::cat::AudioFormatFlags::IS_PACKED,
    );
    let frame = vec![0.0; MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT];
    for index in 0..=super::MAX_MICROPHONE_VAD_PCM_FRAMES as u64 {
        feed_microphone_vad_pcm_samples(&mut feed, format, index, Some(index as f64), &frame);
    }
    super::record_microphone_vad_speech_event(MicrophoneVadSpeechEvent {
        media_start_secs: Some(50.0),
        media_end_secs: Some(51.0),
    });

    let finalization = super::finalize_microphone_vad_boundary_trim(&mut context)
        .expect("boundary finalization should tolerate a missing source file");
    assert!(context.boundary_trim_disabled);
    // The untrimmed path, not the trim path's "missing_source_file".
    assert_eq!(finalization.discard_reason, None);
}

#[test]
fn a_slow_vad_drain_tick_keeps_boundary_trimming() {
    let _guard = microphone_activity_state_test_guard();
    reset_microphone_vad_tail_activity();
    let mut feed = MicrophoneVadPcmFeedState::default();
    let format = pcm_format(
        16_000.0,
        1,
        32,
        cidre::cat::AudioFormatFlags::IS_FLOAT | cidre::cat::AudioFormatFlags::IS_PACKED,
    );
    let frame = vec![0.0; MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT];

    // 3 s of 20 ms frames with no drain: one rotation tick that ran long.
    for index in 0..150 {
        feed_microphone_vad_pcm_samples(&mut feed, format, index, Some(index as f64), &frame);
    }

    assert_eq!(
        super::current_microphone_vad_boundary_trim_disabled_sequence(),
        0,
        "a slow tick must not leave the next file untrimmed"
    );
    assert_eq!(feed.output_frames.len(), 150);
}

#[test]
fn speech_that_ended_before_the_file_discards_it() {
    let _guard = microphone_activity_state_test_guard();
    reset_microphone_vad_tail_activity();
    let dir = std::env::temp_dir().join(format!("mnema-mic-stale-speech-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output_file = dir
        .join("microphone-stale-speech.m4a")
        .to_string_lossy()
        .to_string();
    let output_url = cidre::ns::Url::with_fs_path_str(&output_file, false);

    let resumed_at_host_secs = 1_000.0;
    let mut current = tail_activity_context(MicrophoneInactivityTailTrimActivityMode::PeakLevel, 0);
    let first_sample_secs = fill_ring_ending_at(&mut current, resumed_at_host_secs, 2_048);
    let next = microphone_output_context_for_output_url(
        &output_url,
        Some(output_file.clone()),
        0,
        0.0,
        MicrophoneInactivityTailTrimActivityMode::VadSpeech,
    );
    resume_microphone_output_context(&mut current, next, resumed_at_host_secs);
    let source_file = current
        .source_output_file
        .clone()
        .expect("VAD trimming writes to a temp source");
    // Booked just before the swap: the previous file's speech.
    super::record_microphone_vad_speech_event(MicrophoneVadSpeechEvent {
        media_start_secs: Some(first_sample_secs - 2.0),
        media_end_secs: Some(first_sample_secs - 0.5),
    });
    let finalization = finalize_microphone_output_context(&mut current);
    let output_kept = std::path::Path::new(&output_file).exists();
    let source_left = std::path::Path::new(&source_file).exists();
    let _ = std::fs::remove_dir_all(&dir);
    let finalization = finalization.expect("stale-speech microphone file should finalize");

    assert_eq!(
        finalization.discard_reason.as_deref(),
        Some("no_vad_speech")
    );
    assert!(!finalization.speech_detected);
    assert!(
        !output_kept,
        "a file with no speech of its own is discarded"
    );
    assert!(!source_left, "the temp source is removed too");
}

#[test]
fn vad_trim_without_a_session_start_keeps_the_full_file() {
    assert_eq!(
        super::vad_trim::microphone_vad_trim_range_secs(&speech_boundaries(3.0, 4.0), None, 0.0),
        None
    );
}

#[test]
fn resume_correction_ignores_a_non_host_clock_gap() {
    // PTS on some other clock: far outside anything the queue could hold.
    assert_eq!(
        super::vad_trim::resume_stamp_correction_ms(1_000.0, 1.365, 50_000.0),
        0
    );
    assert_eq!(
        super::vad_trim::resume_stamp_correction_ms(1_000.0, 1.365, 990.0),
        0
    );
    assert_eq!(
        super::vad_trim::resume_stamp_correction_ms(1_000.0, 1.365, f64::NAN),
        0
    );
    // Nothing carried: only device start latency is believable, which a
    // Bluetooth headset can stretch to seconds.
    assert_eq!(
        super::vad_trim::resume_stamp_correction_ms(1_000.0, 0.0, 1_000.25),
        250
    );
    assert_eq!(
        super::vad_trim::resume_stamp_correction_ms(1_000.0, 0.0, 1_003.0),
        3_000
    );
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
fn live_format_is_available_after_first_sample_to_preserve_startup_audio() {
    let mut state = MicFormatStabilityState::default();
    let fmt = format(32, 8);

    observe_microphone_format(&mut state, fmt);

    assert_eq!(resolve_microphone_live_format(&state), Some(fmt));
}

#[test]
fn transient_24_bit_then_32_bit_stabilizes_on_32_bit() {
    let mut state = MicFormatStabilityState::default();
    let fmt24 = format(24, 6);
    let fmt32 = format(32, 8);

    observe_microphone_format(&mut state, fmt24);
    assert_eq!(resolve_microphone_live_format(&state), Some(fmt24));
    observe_microphone_format(&mut state, fmt24);
    assert_eq!(resolve_microphone_live_format(&state), Some(fmt24));
    observe_microphone_format(&mut state, fmt32);
    assert_eq!(resolve_microphone_live_format(&state), Some(fmt32));
    observe_microphone_format(&mut state, fmt32);
    assert_eq!(resolve_microphone_live_format(&state), Some(fmt32));

    observe_microphone_format(&mut state, fmt32);
    assert_eq!(state.stable_format, Some(fmt32));
    assert_eq!(resolve_microphone_live_format(&state), Some(fmt32));
}

#[test]
fn finalize_format_falls_back_to_candidate_for_short_recording() {
    let mut state = MicFormatStabilityState::default();
    let fmt = format(32, 8);

    observe_microphone_format(&mut state, fmt);
    observe_microphone_format(&mut state, fmt);

    assert_eq!(resolve_microphone_live_format(&state), Some(fmt));
    assert_eq!(resolve_microphone_finalize_format(&state), Some(fmt));
}

#[test]
fn pending_microphone_samples_preserve_startup_sample_when_writer_defers() {
    let fmt = format(32, 8);
    let mut pending = std::collections::VecDeque::from(["hello", "one"]);
    let mut attempts = 0;

    flush_pending_microphone_sample_queue(
        &mut pending,
        Some(fmt),
        |_| fmt,
        |sample| {
            attempts += 1;
            if *sample == "hello" {
                Ok::<_, ()>(AudioSampleAppendDisposition::Deferred)
            } else {
                Ok(AudioSampleAppendDisposition::Appended)
            }
        },
    )
    .expect("flush should not fail");

    assert_eq!(attempts, 1);
    assert_eq!(pending, std::collections::VecDeque::from(["hello", "one"]));

    flush_pending_microphone_sample_queue(
        &mut pending,
        Some(fmt),
        |_| fmt,
        |_| Ok::<_, ()>(AudioSampleAppendDisposition::Appended),
    )
    .expect("flush should retry deferred sample");

    assert!(pending.is_empty());
}

#[test]
fn bounded_pending_microphone_samples_keep_recent_probe_audio() {
    let mut pending = std::collections::VecDeque::new();
    for index in 0..=MAX_PENDING_MIC_SAMPLES {
        push_bounded_pending_microphone_sample(&mut pending, index);
    }

    assert_eq!(pending.len(), MAX_PENDING_MIC_SAMPLES);
    assert_eq!(pending.front(), Some(&1));
    assert_eq!(pending.back(), Some(&MAX_PENDING_MIC_SAMPLES));
}

#[test]
fn transfer_pending_microphone_samples_keeps_recent_items() {
    let mut pending = std::collections::VecDeque::from(["hello".to_string(), "world".to_string()]);
    let mut resumed = std::collections::VecDeque::new();
    for index in 0..MAX_PENDING_MIC_SAMPLES {
        resumed.push_back(index.to_string());
    }

    transfer_pending_microphone_samples(&mut pending, &mut resumed);

    assert!(pending.is_empty());
    assert_eq!(resumed.len(), MAX_PENDING_MIC_SAMPLES);
    assert_eq!(resumed.front().map(String::as_str), Some("2"));
    assert_eq!(
        resumed.get(MAX_PENDING_MIC_SAMPLES - 2).map(String::as_str),
        Some("hello")
    );
    assert_eq!(resumed.back().map(String::as_str), Some("world"));
}

#[test]
fn carry_probe_only_microphone_context_preserves_format_state_for_resume() {
    let output_url = cidre::ns::Url::with_fs_path_str("/tmp/test-microphone-resume.m4a", false);
    let fmt = format(32, 8);
    let mut previous =
        tail_activity_context(MicrophoneInactivityTailTrimActivityMode::PeakLevel, 10);
    observe_microphone_format(&mut previous.format_state, fmt);

    let mut resumed = microphone_output_context_for_output_url(
        &output_url,
        Some("/tmp/test-microphone-resume.m4a".to_string()),
        10,
        0.0,
        MicrophoneInactivityTailTrimActivityMode::PeakLevel,
    );

    carry_probe_only_microphone_context(&mut previous, &mut resumed);

    assert_eq!(
        resolve_microphone_live_format(&resumed.format_state),
        Some(fmt)
    );
}

/// A packed mono f32 PCM buffer of silence whose PTS is `pts_secs`.
fn host_clock_pcm_sample(pts_secs: f64, frames: usize) -> BufferedMicSample {
    let asbd = cidre::cat::AudioStreamBasicDesc {
        sample_rate: 48_000.0,
        format: cidre::cat::AudioFormat::LINEAR_PCM,
        format_flags: cidre::cat::AudioFormatFlags::IS_FLOAT
            | cidre::cat::AudioFormatFlags::IS_PACKED,
        bytes_per_packet: 4,
        frames_per_packet: 1,
        bytes_per_frame: 4,
        channels_per_frame: 1,
        bits_per_channel: 32,
        reserved: 0,
    };
    let format_desc = cidre::cm::AudioFormatDesc::with_asbd(&asbd).unwrap();
    let mut block = cidre::cm::BlockBuf::with_mem_block(frames * 4).unwrap();
    block.as_mut_slice().unwrap().fill(0);
    let timing = cidre::cm::SampleTimingInfo {
        duration: cidre::cm::Time::new(1, 48_000),
        pts: cidre::cm::Time::with_secs(pts_secs, 48_000),
        dts: cidre::cm::Time::invalid(),
    };
    let sample_size = 4usize;
    // SAFETY: every pointer is to a live local that outlives the call:
    // `timing` and `sample_size` are single entries, matching the counts of
    // 1, and `block` holds `frames` packed 4-byte frames as `format_desc`
    // and `sample_size` describe. No make-data-ready callback, so its null
    // refcon is never read.
    let sample_buf = unsafe {
        let mut created = None;
        cidre::cm::SampleBuf::create_in(
            None,
            Some(&block),
            true,
            None,
            std::ptr::null(),
            Some(&format_desc),
            frames as cidre::cm::ItemCount,
            1,
            &timing,
            1,
            &sample_size,
            &mut created,
        )
        .unwrap();
        created.unwrap()
    };
    let format = derive_audio_sample_format_from_sample_buf(&sample_buf).unwrap();
    BufferedMicSample { sample_buf, format }
}

/// Fills a paused context's ring with `MAX_PENDING_MIC_SAMPLES` buffers of
/// `frames` each, the last one ending at `resumed_at_host_secs`; returns
/// the first buffer's PTS.
fn fill_ring_ending_at(
    context: &mut MicrophoneOutputContext,
    resumed_at_host_secs: f64,
    frames: usize,
) -> f64 {
    for index in 0..MAX_PENDING_MIC_SAMPLES {
        let frames_before_resume = (MAX_PENDING_MIC_SAMPLES - index) * frames;
        let sample = host_clock_pcm_sample(
            resumed_at_host_secs - frames_before_resume as f64 / 48_000.0,
            frames,
        );
        observe_microphone_format(&mut context.format_state, sample.format);
        push_bounded_pending_microphone_sample(&mut context.pending_samples, sample);
    }
    resumed_at_host_secs - (MAX_PENDING_MIC_SAMPLES * frames) as f64 / 48_000.0
}

#[test]
fn resumed_microphone_file_is_stamped_from_its_first_queued_sample() {
    let dir = std::env::temp_dir().join(format!("mnema-mic-resume-stamp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output_file = dir
        .join("microphone-resume.m4a")
        .to_string_lossy()
        .to_string();
    let output_url = cidre::ns::Url::with_fs_path_str(&output_file, false);

    // Paused for inactivity: the probe-only context holds the newest
    // MAX_PENDING_MIC_SAMPLES buffers, ending at the resume instant.
    let resumed_at_host_secs = 1_000.0;
    let frames = 512;
    let mut current = tail_activity_context(MicrophoneInactivityTailTrimActivityMode::PeakLevel, 0);
    fill_ring_ending_at(&mut current, resumed_at_host_secs, frames);

    let next = microphone_output_context_for_output_url(
        &output_url,
        Some(output_file.clone()),
        0,
        0.0,
        MicrophoneInactivityTailTrimActivityMode::PeakLevel,
    );
    resume_microphone_output_context(&mut current, next, resumed_at_host_secs);
    let finalization = finalize_microphone_output_context(&mut current);
    let _ = std::fs::remove_dir_all(&dir);
    let finalization = finalization.expect("resumed microphone file should finalize");

    assert_eq!(
        finalization.output_file.as_deref(),
        Some(output_file.as_str())
    );
    // 128 queued buffers x 512 frames at 48 kHz = 1.365 s before the stamp.
    assert_eq!(finalization.start_offset_ms, -1_365);
}

#[test]
fn stopping_the_device_drops_a_paused_ring_so_resume_carries_nothing_stale() {
    // Paused for inactivity: the probe-only ring holds pre-stop audio.
    let mut paused = tail_activity_context(MicrophoneInactivityTailTrimActivityMode::PeakLevel, 0);
    for index in 0..4 {
        let sample = host_clock_pcm_sample(100.0 + index as f64 * 0.01, 480);
        observe_microphone_format(&mut paused.format_state, sample.format);
        push_bounded_pending_microphone_sample(&mut paused.pending_samples, sample);
    }

    super::discard_probe_only_pending_samples(&mut paused);
    assert!(paused.pending_samples.is_empty());

    let output_url = cidre::ns::Url::with_fs_path_str("/tmp/mnema-mic-unused.m4a", false);
    let next = microphone_output_context_for_output_url(
        &output_url,
        Some("/tmp/mnema-mic-unused.m4a".to_string()),
        0,
        0.0,
        MicrophoneInactivityTailTrimActivityMode::PeakLevel,
    );
    resume_microphone_output_context(&mut paused, next, 200.0);
    assert!(paused.pending_samples.is_empty());
    assert_eq!(paused.resume_carried_secs, 0.0);
}

#[test]
fn stopping_the_device_keeps_a_live_files_unflushed_audio() {
    let output_url = cidre::ns::Url::with_fs_path_str("/tmp/mnema-mic-unused.m4a", false);
    let mut live = microphone_output_context_for_output_url(
        &output_url,
        Some("/tmp/mnema-mic-unused.m4a".to_string()),
        0,
        0.0,
        MicrophoneInactivityTailTrimActivityMode::PeakLevel,
    );
    push_bounded_pending_microphone_sample(
        &mut live.pending_samples,
        host_clock_pcm_sample(100.0, 480),
    );

    super::discard_probe_only_pending_samples(&mut live);

    assert_eq!(live.pending_samples.len(), 1);
}

#[test]
fn resume_correction_adds_to_the_vad_trim_offset() {
    let _guard = microphone_activity_state_test_guard();
    reset_microphone_vad_tail_activity();
    let dir = std::env::temp_dir().join(format!("mnema-mic-resume-trim-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output_file = dir
        .join("microphone-resume-trim.m4a")
        .to_string_lossy()
        .to_string();
    let output_url = cidre::ns::Url::with_fs_path_str(&output_file, false);

    // 2048-frame buffers fill the ring with 5.46 s, past the old 5 s cap.
    let resumed_at_host_secs = 1_000.0;
    let frames = 2_048;
    let mut current = tail_activity_context(MicrophoneInactivityTailTrimActivityMode::PeakLevel, 0);
    let first_sample_secs = fill_ring_ending_at(&mut current, resumed_at_host_secs, frames);

    let next = microphone_output_context_for_output_url(
        &output_url,
        Some(output_file.clone()),
        0,
        0.0,
        MicrophoneInactivityTailTrimActivityMode::VadSpeech,
    );
    resume_microphone_output_context(&mut current, next, resumed_at_host_secs);
    // Speech 3.0-3.5 s into the file: trimmed from 2.0 s with the padding.
    super::record_microphone_vad_speech_event(MicrophoneVadSpeechEvent {
        media_start_secs: Some(first_sample_secs + 3.0),
        media_end_secs: Some(first_sample_secs + 3.5),
    });
    let finalization = finalize_microphone_output_context(&mut current);
    let output_kept = std::path::Path::new(&output_file).is_file();
    let _ = std::fs::remove_dir_all(&dir);
    let finalization = finalization.expect("resumed microphone file should finalize");

    assert!(output_kept, "the trimmed file must be on disk");
    assert_eq!(
        finalization.output_file.as_deref(),
        Some(output_file.as_str())
    );
    // +2000 ms of trimmed lead-in, -5461 ms of queued audio before the stamp.
    assert_eq!(finalization.start_offset_ms, 2_000 - 5_461);
}

#[test]
fn whole_file_vad_speech_keeps_the_recording() {
    let _guard = microphone_activity_state_test_guard();
    reset_microphone_vad_tail_activity();
    let dir = std::env::temp_dir().join(format!("mnema-mic-whole-speech-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output_file = dir
        .join("microphone-whole-speech.m4a")
        .to_string_lossy()
        .to_string();
    let output_url = cidre::ns::Url::with_fs_path_str(&output_file, false);

    let resumed_at_host_secs = 1_000.0;
    let frames = 2_048;
    let mut current = tail_activity_context(MicrophoneInactivityTailTrimActivityMode::PeakLevel, 0);
    let first_sample_secs = fill_ring_ending_at(&mut current, resumed_at_host_secs, frames);

    let next = microphone_output_context_for_output_url(
        &output_url,
        Some(output_file.clone()),
        0,
        0.0,
        MicrophoneInactivityTailTrimActivityMode::VadSpeech,
    );
    resume_microphone_output_context(&mut current, next, resumed_at_host_secs);
    let source_file = current
        .source_output_file
        .clone()
        .expect("VAD trimming writes to a temp source");
    super::record_microphone_vad_speech_event(MicrophoneVadSpeechEvent {
        media_start_secs: Some(first_sample_secs),
        media_end_secs: Some(resumed_at_host_secs),
    });
    let finalization = finalize_microphone_output_context(&mut current);
    let output_kept = std::path::Path::new(&output_file).is_file();
    let source_left = std::path::Path::new(&source_file).exists();
    let _ = std::fs::remove_dir_all(&dir);
    let finalization = finalization.expect("speech-filled microphone file should finalize");

    assert!(output_kept, "a file that is all speech must be kept whole");
    assert!(!source_left, "the temp source must be moved into place");
    assert_eq!(
        finalization.output_file.as_deref(),
        Some(output_file.as_str())
    );
    // Nothing trimmed: only the -5461 ms of queued audio before the stamp.
    assert_eq!(finalization.start_offset_ms, -5_461);
}

#[test]
fn rotation_context_finalizes_previous_output_with_normal_tail_policy() {
    let current_url = cidre::ns::Url::with_fs_path_str("/tmp/test-microphone-current.m4a", false);
    let next_url = cidre::ns::Url::with_fs_path_str("/tmp/test-microphone-next.m4a", false);
    let mut current = microphone_output_context_for_output_url(
        &current_url,
        Some("/tmp/test-microphone-current.m4a".to_string()),
        10,
        0.08,
        MicrophoneInactivityTailTrimActivityMode::PeakLevel,
    );
    let next = microphone_output_context_for_output_url(
        &next_url,
        Some("/tmp/test-microphone-next.m4a".to_string()),
        10,
        0.08,
        MicrophoneInactivityTailTrimActivityMode::PeakLevel,
    );

    let previous = replace_microphone_output_context_for_rotation(&mut current, next);

    assert_eq!(
        previous.inactivity_tail_trim_seconds, 0,
        "normal segment rotation must flush buffered tail audio instead of discarding it as inactivity"
    );
    assert_eq!(current.inactivity_tail_trim_seconds, 10);
}

#[test]
fn delegates_created_on_many_threads_at_once_all_succeed() {
    // The race only exists before the class is registered, which any test
    // that ran earlier in this process may have done: race in a fresh one.
    const CHILD_ENV: &str = "CAPTURE_MICROPHONE_DELEGATE_RACE_CHILD";
    if std::env::var_os(CHILD_ENV).is_none() {
        let output = std::process::Command::new(
            std::env::current_exe().expect("current test binary should exist"),
        )
        .env(CHILD_ENV, "1")
        .args([
            "--exact",
            "tests::delegates_created_on_many_threads_at_once_all_succeed",
        ])
        .output()
        .expect("child test process should run");
        assert!(
            output.status.success(),
            "first-use delegate creation raced\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                drop(super::new_mic_output_delegate(
                    super::microphone_probe_only_context(),
                ));
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("delegate creation panicked");
    }
}

#[test]
fn context_swaps_on_the_output_queue_never_lose_or_duplicate_a_sample() {
    // Stands in for AVFoundation: sample callbacks land on the serial output
    // queue while this thread rotates and resumes the context under them.
    const ROUNDS: usize = 20;
    // Under MAX_PENDING_MIC_SAMPLES, so the ring bound never drops one.
    const SAMPLES_PER_ROUND: usize = 100;
    let queue = cidre::dispatch::Queue::serial_with_ar_pool();
    let mut delegate = super::new_mic_output_delegate(super::microphone_probe_only_context());
    // One pointer for every access below; `delegate` itself is not touched
    // again until it drops, so nothing reborrows it out from under this.
    let delegate_ptr = std::ptr::NonNull::from(&mut *delegate);
    let delegate_addr = delegate_ptr.as_ptr().expose_provenance();

    for round in 0..ROUNDS {
        let mut collected = 0;
        for index in 0..SAMPLES_PER_ROUND {
            queue.async_once(move || {
                // SAFETY: the delegate outlives every block (each round
                // drains the queue below), and blocks run one at a time on
                // the serial queue, exactly as AVFoundation's callback does.
                let delegate = unsafe {
                    &mut *std::ptr::with_exposed_provenance_mut::<
                        super::MicAudioDataOutputDelegate,
                    >(delegate_addr)
                };
                push_bounded_pending_microphone_sample(
                    &mut delegate.inner_mut().pending_samples,
                    host_clock_pcm_sample(index as f64, 16),
                );
            });
            if index % 3 != 0 {
                continue;
            }
            let previous = super::with_output_context_on_queue(&queue, delegate_ptr, |context| {
                let next = super::microphone_probe_only_context();
                if index % 2 == 0 {
                    replace_microphone_output_context_for_rotation(context, next)
                } else {
                    resume_microphone_output_context(context, next, 0.0)
                }
            });
            collected += previous.pending_samples.len();
        }
        collected += super::with_output_context_on_queue(&queue, delegate_ptr, |context| {
            std::mem::take(&mut context.pending_samples).len()
        });
        assert_eq!(collected, SAMPLES_PER_ROUND, "round {round}");
    }
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

    feed_microphone_vad_pcm_samples(&mut state, format, 42_000, Some(12.5), &samples);

    assert_eq!(state.output_frames.len(), 1);
    let frame = state.output_frames.pop_front().expect("frame should exist");
    assert_eq!(frame.sample_rate_hz, MICROPHONE_VAD_PCM_SAMPLE_RATE_HZ);
    assert_eq!(frame.captured_at_unix_ms, 42_000);
    assert_eq!(frame.media_start_secs, Some(12.5));
    assert_eq!(frame.media_end_secs, Some(12.52));
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

    feed_microphone_vad_pcm_samples(&mut state, fmt48, 1, None, &vec![0.25; 480]);
    assert_eq!(state.output_frames.len(), 0);
    assert_eq!(state.pending_source_samples.len(), 480);

    feed_microphone_vad_pcm_samples(&mut state, fmt16, 2, None, &vec![0.50; 320]);

    assert_eq!(state.pending_source_samples.len(), 0);
    assert_eq!(state.output_frames.len(), 1);
    assert_eq!(state.output_frames[0].samples, vec![16_384; 320]);
    assert_eq!(state.output_frames[0].normalized_peak_level, 0.50);
}

#[test]
fn vad_pcm_feed_is_bounded_to_recent_frames() {
    // Overflowing bumps the shared trim-disabled sequence other tests read.
    let _guard = microphone_activity_state_test_guard();
    let mut state = MicrophoneVadPcmFeedState::default();
    let format = pcm_format(
        16_000.0,
        1,
        32,
        cidre::cat::AudioFormatFlags::IS_FLOAT | cidre::cat::AudioFormatFlags::IS_PACKED,
    );
    let frame = vec![0.0; MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT];

    let cap = super::MAX_MICROPHONE_VAD_PCM_FRAMES as u64;
    for index in 0..cap + 24 {
        feed_microphone_vad_pcm_samples(&mut state, format, index, None, &frame);
    }

    assert_eq!(state.output_frames.len() as u64, cap);
    assert_eq!(state.output_frames[0].captured_at_unix_ms, 24);
    assert_eq!(
        state.output_frames.back().unwrap().captured_at_unix_ms,
        cap + 23
    );
}

#[test]
fn vad_pcm_decoder_downmixes_interleaved_signed_pcm() {
    let format = pcm_format(
        48_000.0,
        2,
        16,
        cidre::cat::AudioFormatFlags::IS_SIGNED_INTEGER | cidre::cat::AudioFormatFlags::IS_PACKED,
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
