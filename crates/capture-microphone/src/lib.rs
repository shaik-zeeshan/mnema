use capture_types::{CaptureErrorResponse, CapturePermissionState};

#[cfg(target_os = "macos")]
use capture_writers::{
    append_audio_sample_to_writer, create_audio_asset_writer,
    finalize_microphone_output_context as writers_finalize_microphone_output_context,
    AudioAssetWriterState,
};

#[cfg(target_os = "macos")]
use cidre::objc;
#[cfg(target_os = "macos")]
use cidre::{av, dispatch};
#[cfg(target_os = "macos")]
use std::sync::mpsc;
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct MicrophoneOutputContext {
    writer: AudioAssetWriterState,
    first_error: Option<CaptureErrorResponse>,
}

#[cfg(target_os = "macos")]
mod microphone_delegate {
    #![allow(clippy::useless_transmute)]

    use super::{append_audio_sample_to_writer, objc, MicrophoneOutputContext};
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
            let ctx = self.inner_mut();
            if ctx.first_error.is_some() {
                return;
            }

            if let Err(error) = append_audio_sample_to_writer(&mut ctx.writer, sample_buf) {
                ctx.first_error = Some(error);
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
    writers_finalize_microphone_output_context(&mut context.writer, context.first_error.take())
}

#[cfg(target_os = "macos")]
pub fn start_avfoundation_microphone_capture_session(
    output_url: &cidre::ns::Url,
) -> Result<AvFoundationMicrophoneCaptureSession, CaptureErrorResponse> {
    let mut capture_session = av::CaptureSession::new();

    let mic_device =
        av::CaptureDevice::default_with_media(av::MediaType::audio()).ok_or_else(|| {
            CaptureErrorResponse {
                code: "microphone_input_unavailable".to_string(),
                message: "Failed to resolve microphone device".to_string(),
            }
        })?;

    let mic_input = av::CaptureDeviceInput::with_device(mic_device.as_ref()).map_err(|_| {
        CaptureErrorResponse {
            code: "microphone_input_unavailable".to_string(),
            message: "Failed to create microphone input".to_string(),
        }
    })?;

    let mut audio_output = av::capture::AudioDataOutput::new();
    let writer = create_audio_asset_writer(output_url, "microphone")?;
    let output_delegate = MicAudioDataOutputDelegate::with(MicrophoneOutputContext {
        writer,
        first_error: None,
    });
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
    let output_url = cidre::ns::Url::with_fs_path_str(output_file, false);
    start_avfoundation_microphone_capture_session(&output_url)
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
