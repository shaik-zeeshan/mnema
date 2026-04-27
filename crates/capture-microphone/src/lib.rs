use capture_types::{
    CaptureErrorResponse, CapturePermissionState, MicrophoneControllerState, MicrophoneDevice,
    MicrophoneDisconnectPolicy, MicrophonePreference, MicrophonePreferenceMode,
};

#[cfg(target_os = "macos")]
use capture_writers::{
    append_audio_sample_to_writer, create_audio_asset_writer_for_sample_format,
    derive_audio_activity_level_from_sample_buf, derive_audio_sample_format_from_sample_buf,
    finalize_microphone_output_context as writers_finalize_microphone_output_context,
    AudioAssetWriterState, AudioSampleFormat,
};

#[cfg(target_os = "macos")]
use cidre::{av, core_audio as ca, dispatch, os};
#[cfg(target_os = "macos")]
use cidre::{ns, objc};
#[cfg(target_os = "macos")]
use std::collections::VecDeque;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
#[cfg(target_os = "macos")]
use std::sync::mpsc;
#[cfg(target_os = "macos")]
use std::sync::{Arc, OnceLock};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
static LAST_MICROPHONE_ACTIVITY_UNIX_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "macos")]
static LAST_MICROPHONE_ACTIVITY_MONOTONIC_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "macos")]
static LAST_MICROPHONE_ACTIVITY_LEVEL_BITS: AtomicU32 = AtomicU32::new(0);

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
    LAST_MICROPHONE_ACTIVITY_LEVEL_BITS.store(level.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    LAST_MICROPHONE_ACTIVITY_MONOTONIC_MS.store(now_monotonic_ms, Ordering::Relaxed);
    LAST_MICROPHONE_ACTIVITY_UNIX_MS.store(now_unix_ms, Ordering::Relaxed);
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
pub fn reset_last_microphone_activity_unix_ms() {
    LAST_MICROPHONE_ACTIVITY_UNIX_MS.store(0, Ordering::Relaxed);
    LAST_MICROPHONE_ACTIVITY_MONOTONIC_MS.store(0, Ordering::Relaxed);
    LAST_MICROPHONE_ACTIVITY_LEVEL_BITS.store(0, Ordering::Relaxed);
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

#[cfg(not(target_os = "macos"))]
pub fn microphone_activity_level() -> Option<f32> {
    None
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct MicrophoneOutputContext {
    writer: Option<AudioAssetWriterState>,
    output_url: cidre::arc::R<cidre::ns::Url>,
    output_file: Option<String>,
    first_error: Option<CaptureErrorResponse>,
    format_state: MicFormatStabilityState,
    logged_format_samples: u32,
    pending_samples: VecDeque<BufferedMicSample>,
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
        last_microphone_activity_unix_ms, microphone_activity_idle_ms, microphone_activity_level,
        microphone_output_callback_objc_exception_error, microphone_output_callback_panic_error,
        observe_microphone_format, reset_last_microphone_activity_unix_ms,
        resolve_microphone_finalize_format, resolve_microphone_live_format,
        store_microphone_activity, AudioSampleFormat, MicFormatStabilityState, OnceLock,
    };

    fn microphone_activity_state_test_guard() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        GUARD
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
fn flush_pending_microphone_samples(
    context: &mut MicrophoneOutputContext,
) -> Result<(), CaptureErrorResponse> {
    let selected_format = fallback_microphone_format(context);
    let Some(writer) = context.writer.as_mut() else {
        return Ok(());
    };

    while let Some(sample) = context.pending_samples.pop_front() {
        if selected_format.is_some() && Some(sample.format) != selected_format {
            continue;
        }

        append_audio_sample_to_writer(writer, sample.sample_buf.as_ref())?;
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
        append_audio_sample_to_writer, create_audio_asset_writer_for_sample_format,
        derive_audio_sample_format_from_sample_buf, flush_pending_microphone_samples,
        maybe_track_microphone_activity, microphone_output_callback_objc_exception_error,
        microphone_output_callback_panic_error, ns, objc, record_observed_audio_format,
        resolve_microphone_live_format, BufferedMicSample, MicrophoneOutputContext,
        MAX_PENDING_MIC_SAMPLES,
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
            let objc_result = ns::try_catch(|| {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let ctx = self.inner_mut();
                    if ctx.first_error.is_some() {
                        return;
                    }

                    maybe_track_microphone_activity(sample_buf);

                    if let Some(writer) = ctx.writer.as_mut() {
                        if let Err(error) = append_audio_sample_to_writer(writer, sample_buf) {
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
                            ctx.output_url.as_ref(),
                            "microphone",
                            stable_format,
                        ) {
                            Ok(writer) => ctx.writer = Some(writer),
                            Err(error) => {
                                ctx.first_error = Some(error);
                                return;
                            }
                        }
                    }

                    if let Err(error) = flush_pending_microphone_samples(ctx) {
                        ctx.first_error = Some(error);
                    }
                }));

                if let Err(payload) = result {
                    self.inner_mut().first_error =
                        Some(microphone_output_callback_panic_error(payload));
                }
            });

            if let Err(exception) = objc_result {
                self.inner_mut().first_error =
                    Some(microphone_output_callback_objc_exception_error(exception));
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
        self.capture_session.stop_running();
        synchronize_stream_output_queue(Some(self.output_queue.as_ref()));
        finalize_microphone_output_context(self.output_delegate.inner_mut())
    }

    pub fn rotate_output_file(&mut self, output_file: &str) -> Result<(), CaptureErrorResponse> {
        let output_url = cidre::ns::Url::with_fs_path_str(output_file, false);
        let next_context =
            microphone_output_context_for_output_url(&output_url, Some(output_file.to_string()));

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
        if let Some(format) = resolve_microphone_finalize_format(&context.format_state) {
            match create_audio_asset_writer_for_sample_format(
                context.output_url.as_ref(),
                "microphone",
                format,
            ) {
                Ok(writer) => {
                    context.writer = Some(writer);
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

    match writers_finalize_microphone_output_context(
        context.writer.as_mut(),
        context.first_error.take(),
    ) {
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
) -> MicrophoneOutputContext {
    MicrophoneOutputContext {
        writer: None,
        output_url: output_url.retained(),
        output_file,
        first_error: None,
        format_state: MicFormatStabilityState::default(),
        logged_format_samples: 0,
        pending_samples: VecDeque::new(),
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
#[derive(Debug)]
struct DefaultInputDeviceListener {
    callback_ptr_addr: usize,
}

#[cfg(target_os = "macos")]
extern "C-unwind" fn default_input_device_changed_listener(
    _obj_id: ca::Obj,
    _number_addresses: u32,
    _addresses: *const ca::PropAddr,
    callback_ptr: *mut Arc<DeviceChangeCallback>,
) -> os::Status {
    let Some(callback) = (unsafe { callback_ptr.as_ref() }) else {
        return os::Status::NO_ERR;
    };

    let _ = ns::try_catch(|| {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            callback();
        }));
    });
    os::Status::NO_ERR
}

#[cfg(target_os = "macos")]
impl Drop for DefaultInputDeviceListener {
    fn drop(&mut self) {
        let _ = ca::System::OBJ.remove_prop_listener(
            &ca::PropSelector::HW_DEFAULT_INPUT_DEVICE.global_addr(),
            default_input_device_changed_listener,
            self.callback_ptr_addr as *mut Arc<DeviceChangeCallback>,
        );

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
pub fn start_microphone_device_change_notifier(
    callback: impl Fn() + Send + Sync + 'static,
) -> MicrophoneDeviceChangeNotifier {
    let mut center = cidre::ns::NotificationCenter::default();
    let callback: Arc<DeviceChangeCallback> = Arc::new(callback);
    let connected_notification = IntoNotificationName::into_notification_name(
        av::capture::device::notifications::was_connected(),
    );
    let disconnected_notification = IntoNotificationName::into_notification_name(
        av::capture::device::notifications::was_disconnected(),
    );

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
    let default_input_listener = if ca::System::OBJ
        .add_prop_listener(
            &ca::PropSelector::HW_DEFAULT_INPUT_DEVICE.global_addr(),
            default_input_device_changed_listener,
            callback_ptr,
        )
        .is_ok()
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
    start_avfoundation_microphone_capture_session_with_output_file(output_url, None, None)
}

#[cfg(target_os = "macos")]
pub fn start_avfoundation_microphone_capture_session_with_device_id(
    output_url: &cidre::ns::Url,
    device_id: Option<&str>,
) -> Result<AvFoundationMicrophoneCaptureSession, CaptureErrorResponse> {
    start_avfoundation_microphone_capture_session_with_output_file(output_url, None, device_id)
}

#[cfg(target_os = "macos")]
fn start_avfoundation_microphone_capture_session_with_output_file(
    output_url: &cidre::ns::Url,
    output_file: Option<String>,
    device_id: Option<&str>,
) -> Result<AvFoundationMicrophoneCaptureSession, CaptureErrorResponse> {
    reset_last_microphone_activity_unix_ms();

    let mut capture_session = av::CaptureSession::new();

    let mic_device = resolve_capture_device_for_id(device_id)?;

    let mic_input = av::CaptureDeviceInput::with_device(mic_device.as_ref()).map_err(|_| {
        CaptureErrorResponse {
            code: "microphone_input_unavailable".to_string(),
            message: "Failed to create microphone input".to_string(),
        }
    })?;

    let mut audio_output = av::capture::AudioDataOutput::new();
    let output_delegate = MicAudioDataOutputDelegate::with(
        microphone_output_context_for_output_url(output_url, output_file),
    );
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
