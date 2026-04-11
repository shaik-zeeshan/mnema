use capture_types::{
    CaptureErrorResponse, CaptureOutputFiles, CapturePermissionState, ScreenResolution,
    ScreenResolutionPreset,
};

#[cfg(target_os = "macos")]
use capture_writers::{
    append_audio_sample_to_writer, append_video_sample_to_writer, create_audio_asset_writer,
    create_video_asset_writer_for_sample_buf,
    finalize_stream_output_context as writers_finalize_stream_output_context,
    AudioAssetWriterState, VideoAssetWriterState,
};

#[cfg(target_os = "macos")]
use cidre::arc::Retain;
#[cfg(target_os = "macos")]
use cidre::dispatch;
#[cfg(target_os = "macos")]
use cidre::objc;
#[cfg(target_os = "macos")]
use std::collections::HashMap;
#[cfg(target_os = "macos")]
use std::ffi::CString;
#[cfg(target_os = "macos")]
use std::fmt::Display;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "macos")]
use std::sync::mpsc;
#[cfg(target_os = "macos")]
use std::sync::{Mutex, OnceLock};
#[cfg(target_os = "macos")]
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenCaptureSources {
    pub screen: bool,
    pub system_audio: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCaptureResolution {
    pub width: u32,
    pub height: u32,
}

fn even_dimension(value: u32) -> u32 {
    let at_least_two = value.max(2);
    if at_least_two % 2 == 0 {
        at_least_two
    } else {
        at_least_two + 1
    }
}

#[cfg(target_os = "macos")]
fn resolve_stream_resolution(
    requested: &ScreenResolution,
    display_width: u32,
    display_height: u32,
) -> ScreenCaptureResolution {
    let display_width = display_width.max(1);
    let display_height = display_height.max(1);

    let requested_height = match requested {
        ScreenResolution::Preset { preset } => match preset {
            ScreenResolutionPreset::Original => {
                return ScreenCaptureResolution {
                    width: display_width,
                    height: display_height,
                }
            }
            ScreenResolutionPreset::P1080 => 1080,
            ScreenResolutionPreset::P720 => 720,
            ScreenResolutionPreset::P540 => 540,
        },
        ScreenResolution::Custom { width, height } => {
            return ScreenCaptureResolution {
                width: even_dimension(*width),
                height: even_dimension(*height),
            };
        }
    };

    if requested_height >= display_height {
        return ScreenCaptureResolution {
            width: display_width,
            height: display_height,
        };
    }

    let width = ((display_width as f64) * (requested_height as f64) / (display_height as f64))
        .round() as u32;

    ScreenCaptureResolution {
        width: even_dimension(width),
        height: even_dimension(requested_height),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCaptureSupport {
    pub platform: String,
    pub native_capture_supported: bool,
    pub screen: bool,
    pub system_audio: bool,
}

fn output_files_for_session(
    session_dir: &Path,
    sources: &ScreenCaptureSources,
) -> CaptureOutputFiles {
    let screen_file = sources
        .screen
        .then_some(session_dir.join("screen.mov").to_string_lossy().to_string());
    let system_audio_file = sources.system_audio.then_some(
        session_dir
            .join("system-audio.m4a")
            .to_string_lossy()
            .to_string(),
    );

    CaptureOutputFiles {
        screen_file: screen_file.clone(),
        screen_files: screen_file.into_iter().collect(),
        microphone_file: None,
        microphone_files: Vec::new(),
        system_audio_file: system_audio_file.clone(),
        system_audio_files: system_audio_file.into_iter().collect(),
    }
}

#[cfg(target_os = "macos")]
static SCREEN_PERMISSION_REQUESTED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct StreamOutputContext {
    screen_video_output_file: Option<String>,
    screen_video_writer: Option<VideoAssetWriterState>,
    video_bitrate_bps: Option<u32>,
    system_audio_writer: Option<AudioAssetWriterState>,
    first_error: Option<CaptureErrorResponse>,
}

#[cfg(target_os = "macos")]
mod stream_output_delegate {
    #![allow(clippy::useless_transmute)]

    use super::{
        append_audio_sample_to_writer, append_video_sample_to_writer,
        create_video_asset_writer_for_sample_buf, objc, StreamOutputContext,
    };
    use cidre::ns;
    use cidre::sc::StreamOutput;

    cidre::define_obj_type!(
        pub(super) ScStreamOutputDelegate + cidre::sc::StreamOutputImpl,
        StreamOutputContext,
        ZScStreamOutputDelegate
    );

    impl cidre::sc::StreamOutput for ScStreamOutputDelegate {}

    #[cidre::objc::add_methods]
    impl cidre::sc::StreamOutputImpl for ScStreamOutputDelegate {
        extern "C" fn impl_stream_did_output_sample_buf(
            &mut self,
            _cmd: Option<&cidre::objc::Sel>,
            _stream: &cidre::sc::Stream,
            sample_buf: &mut cidre::cm::SampleBuf,
            kind: cidre::sc::OutputType,
        ) {
            let ctx = self.inner_mut();

            let append_result = match kind {
                cidre::sc::OutputType::Screen => {
                    if !super::should_append_screen_sample(sample_buf) {
                        return;
                    }

                    if ctx.screen_video_writer.is_none() {
                        let Some(output_file) = ctx.screen_video_output_file.as_deref() else {
                            return;
                        };
                        let output_url = ns::Url::with_fs_path_str(output_file, false);
                        match create_video_asset_writer_for_sample_buf(
                            &output_url,
                            "screen",
                            sample_buf,
                            ctx.video_bitrate_bps,
                        ) {
                            Ok(writer) => ctx.screen_video_writer = Some(writer),
                            Err(error) => {
                                if ctx.first_error.is_none() {
                                    ctx.first_error = Some(error);
                                }
                                return;
                            }
                        }
                    }

                    ctx.screen_video_writer
                        .as_mut()
                        .map(|writer| append_video_sample_to_writer(writer, sample_buf))
                }
                cidre::sc::OutputType::Audio => ctx
                    .system_audio_writer
                    .as_mut()
                    .map(|writer| append_audio_sample_to_writer(writer, sample_buf)),
                cidre::sc::OutputType::Mic => None,
            };

            if let Some(Err(error)) = append_result {
                if ctx.first_error.is_none() {
                    ctx.first_error = Some(error);
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
use stream_output_delegate::ScStreamOutputDelegate;

#[cfg(target_os = "macos")]
fn should_append_screen_sample(sample_buf: &cidre::cm::SampleBuf) -> bool {
    use cidre::{cf, cm, sc};

    let mut attachment_mode = cm::AttachMode::Propagate;
    let status_key = sc::FrameInfo::status().as_type_ref().try_as_string();
    let status_value = status_key
        .and_then(|key| sample_buf.attach(key, &mut attachment_mode))
        .and_then(cf::Type::try_as_number)
        .and_then(|status| status.to_i32());

    match status_value {
        Some(value) => value == sc::FrameStatus::Complete as i32,
        None => true,
    }
}

#[cfg(target_os = "macos")]
fn maybe_remove_screen_video_file(path: &str) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("failed to remove invalid screen video artifact {path}: {error}"),
    }
}

#[cfg(target_os = "macos")]
fn validate_screen_video_file(path: &str) -> Result<(), CaptureErrorResponse> {
    use cidre::{av, ns};

    let metadata = std::fs::metadata(path).map_err(|error| CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message: format!("Failed to inspect finalized screen recording: {error}"),
    })?;
    if metadata.len() == 0 {
        return Err(CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Finalized screen recording is empty".to_string(),
        });
    }

    let output_url = ns::Url::with_fs_path_str(path, false);
    let asset = av::UrlAsset::with_url(&output_url, None).ok_or_else(|| CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message: "Failed to open finalized screen recording for validation".to_string(),
    })?;

    let tracks = load_asset_tracks_with_timeout(
        asset.as_ref(),
        av::MediaType::video(),
        "capture_output_processing_failed",
        "Timed out while validating finalized screen recording video track",
    )?;

    if tracks.is_empty() {
        return Err(CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Finalized screen recording has no playable video track".to_string(),
        });
    }

    Ok(())
}

#[cfg(target_os = "macos")]
pub struct StartedCaptureSession {
    pub session: ActiveCaptureSession,
    pub recording_file: String,
    pub system_audio_recording_file: Option<String>,
    pub output_files: CaptureOutputFiles,
}

#[cfg(target_os = "macos")]
pub struct RotatedCaptureOutputs {
    pub recording_file: String,
    pub system_audio_recording_file: Option<String>,
    pub output_files: CaptureOutputFiles,
}

#[cfg(target_os = "macos")]
type StartCallbackMap = HashMap<usize, mpsc::Sender<()>>;
#[cfg(target_os = "macos")]
type FinishCallbackMap = HashMap<usize, mpsc::Sender<Result<(), CaptureErrorResponse>>>;

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct AvFoundationCaptureSession {
    capture_session: objc2::rc::Retained<objc2_av_foundation::AVCaptureSession>,
    movie_output: objc2::rc::Retained<objc2_av_foundation::AVCaptureMovieFileOutput>,
    _delegate: objc2::rc::Retained<objc2_foundation::NSObject>,
    delegate_key: usize,
    finish_rx: mpsc::Receiver<Result<(), CaptureErrorResponse>>,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct ScreenCaptureKitCaptureSession {
    stream: cidre::arc::R<cidre::sc::Stream>,
    stream_output_delegate: cidre::arc::R<ScStreamOutputDelegate>,
    stream_output_queue: cidre::arc::R<dispatch::Queue>,
    sources: ScreenCaptureSources,
    video_bitrate_bps: Option<u32>,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
enum CaptureBackendSession {
    AvFoundation(AvFoundationCaptureSession),
    ScreenCaptureKit(ScreenCaptureKitCaptureSession),
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct ActiveCaptureSession {
    backend: CaptureBackendSession,
}

#[cfg(target_os = "macos")]
unsafe impl Send for ActiveCaptureSession {}

#[cfg(target_os = "macos")]
impl ActiveCaptureSession {
    fn stop(&mut self) -> Result<(), CaptureErrorResponse> {
        match &mut self.backend {
            CaptureBackendSession::AvFoundation(session) => session.stop(),
            CaptureBackendSession::ScreenCaptureKit(session) => session.stop(),
        }
    }
}

#[cfg(target_os = "macos")]
impl AvFoundationCaptureSession {
    fn stop(&mut self) -> Result<(), CaptureErrorResponse> {
        let was_recording = unsafe { self.movie_output.isRecording() };
        let mut finalize_error: Option<CaptureErrorResponse> = None;

        if was_recording {
            unsafe { self.movie_output.stopRecording() };

            match self.finish_rx.recv_timeout(Duration::from_secs(15)) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => finalize_error = Some(error),
                Err(_) => {
                    let mut callbacks = delegate_finish_callbacks()
                        .lock()
                        .expect("delegate callback map poisoned");
                    callbacks.remove(&self.delegate_key);
                    finalize_error = Some(CaptureErrorResponse {
                        code: "capture_stop_incomplete".to_string(),
                        message: "Timed out waiting for native capture file finalization"
                            .to_string(),
                    });
                }
            }
        }

        unsafe { self.capture_session.stopRunning() };

        if let Some(error) = finalize_error {
            return Err(error);
        }

        Ok(())
    }
}

#[cfg(target_os = "macos")]
impl ScreenCaptureKitCaptureSession {
    fn is_stop_timeout_code(code: &str) -> bool {
        matches!(
            code,
            "capture_stop_incomplete" | "capture_start_rollback_incomplete"
        )
    }

    fn stop_stream(
        stream: &cidre::sc::Stream,
        timeout_code: &str,
    ) -> Result<(), CaptureErrorResponse> {
        let (tx, rx) = mpsc::channel::<Result<(), CaptureErrorResponse>>();
        let mut completion = cidre::blocks::ErrCh::new1(move |error| {
            let _ = tx.send(match error {
                Some(error) => Err(error_with_ns_error(
                    "capture_stop_failed",
                    "Failed to stop ScreenCaptureKit stream",
                    error,
                )),
                None => Ok(()),
            });
        });

        stream.stop_with_ch_block(Some(&mut completion));

        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(result) => result,
            Err(_) => Err(CaptureErrorResponse {
                code: timeout_code.to_string(),
                message: "Timed out waiting for ScreenCaptureKit stream stop".to_string(),
            }),
        }
    }

    fn stop(&mut self) -> Result<(), CaptureErrorResponse> {
        let mut stop_error: Option<CaptureErrorResponse> = None;

        let stream_stopped = match Self::stop_stream(&self.stream, "capture_stop_incomplete") {
            Ok(()) => true,
            Err(error) => {
                if Self::is_stop_timeout_code(error.code.as_str()) {
                    return Err(error);
                }

                if stop_error.is_none() {
                    stop_error = Some(error);
                }

                false
            }
        };

        if stream_stopped {
            synchronize_stream_output_queue(Some(self.stream_output_queue.as_ref()));
            if let Err(error) =
                finalize_stream_output_context(self.stream_output_delegate.inner_mut())
            {
                if stop_error.is_none() {
                    stop_error = Some(error);
                }
            }
        }

        if let Some(error) = stop_error {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn rotate_output_files(
        &mut self,
        segment_dir: &Path,
    ) -> Result<RotatedCaptureOutputs, CaptureErrorResponse> {
        let output_files = output_files_for_session(segment_dir, &self.sources);
        let recording_file = segment_dir.join("screen.mov").to_string_lossy().to_string();
        let system_audio_recording_file = self.sources.system_audio.then_some(
            segment_dir
                .join("system-audio.m4a")
                .to_string_lossy()
                .to_string(),
        );

        std::fs::create_dir_all(segment_dir).map_err(|e| CaptureErrorResponse {
            code: "io_error".to_string(),
            message: format!("Failed to create capture session directory: {e}"),
        })?;

        let next_context = stream_output_context_for_segment(
            &recording_file,
            system_audio_recording_file.as_deref(),
            &self.sources,
            self.video_bitrate_bps,
        )?;

        synchronize_stream_output_queue(Some(self.stream_output_queue.as_ref()));
        let mut previous_context =
            std::mem::replace(self.stream_output_delegate.inner_mut(), next_context);
        finalize_stream_output_context(&mut previous_context)?;

        Ok(RotatedCaptureOutputs {
            recording_file,
            system_audio_recording_file,
            output_files,
        })
    }
}

#[cfg(target_os = "macos")]
fn fmt_ns<T: Display + ?Sized>(value: &T) -> String {
    format!("{value}")
}

#[cfg(target_os = "macos")]
fn error_with_ns_error(code: &str, prefix: &str, error: &cidre::ns::Error) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: code.to_string(),
        message: format!(
            "{prefix}: {} (code: {})",
            fmt_ns(error.localized_desc().as_ref()),
            error.code(),
        ),
    }
}

#[cfg(target_os = "macos")]
pub fn new_session_id() -> Result<String, CaptureErrorResponse> {
    use cidre::ns;

    let uuid = ns::Uuid::new();
    let uuid_str = uuid.string();
    Ok(format!(
        "native-session-{}",
        fmt_ns(uuid_str.as_ref()).to_lowercase()
    ))
}

#[cfg(target_os = "macos")]
fn create_session_dir(session_dir: &Path) -> Result<(), CaptureErrorResponse> {
    std::fs::create_dir_all(session_dir).map_err(|e| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!("Failed to create capture session directory: {e}"),
    })
}

#[cfg(target_os = "macos")]
fn remove_session_dir(session_dir: &std::path::Path) -> Result<(), CaptureErrorResponse> {
    match std::fs::remove_dir_all(session_dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CaptureErrorResponse {
            code: "io_error".to_string(),
            message: format!(
                "Failed to remove capture session directory after startup rollback: {error}"
            ),
        }),
    }
}

#[cfg(target_os = "macos")]
fn finalize_startup_result<T>(
    start_result: Result<T, CaptureErrorResponse>,
    session_dir: &std::path::Path,
) -> Result<T, CaptureErrorResponse> {
    match start_result {
        Ok(started) => Ok(started),
        Err(start_error) => {
            if should_preserve_runtime_on_startup_error(&start_error) {
                return Err(start_error);
            }

            if let Err(cleanup_error) = remove_session_dir(session_dir) {
                return Err(CaptureErrorResponse {
                    code: start_error.code,
                    message: format!(
                        "{}; additionally failed startup cleanup: [{}] {}",
                        start_error.message, cleanup_error.code, cleanup_error.message
                    ),
                });
            }

            Err(start_error)
        }
    }
}

#[cfg(target_os = "macos")]
fn should_preserve_runtime_on_startup_error(error: &CaptureErrorResponse) -> bool {
    error.code == "capture_start_rollback_incomplete"
}

#[cfg(target_os = "macos")]
fn recording_delegate_class() -> &'static objc2::runtime::AnyClass {
    use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, ClassBuilder, Sel};
    use objc2::{sel, ClassType};
    use objc2_foundation::NSObject;

    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();

    CLASS.get_or_init(|| {
        extern "C-unwind" fn did_start_recording(
            this: *mut AnyObject,
            _cmd: Sel,
            _output: *mut AnyObject,
            _url: *mut AnyObject,
            _connections: *mut AnyObject,
        ) {
            let key = this as usize;
            if let Some(tx) = delegate_start_callbacks()
                .lock()
                .expect("delegate callback map poisoned")
                .remove(&key)
            {
                let _ = tx.send(());
            }
        }

        extern "C-unwind" fn did_finish_recording(
            this: *mut AnyObject,
            _cmd: Sel,
            _output: *mut AnyObject,
            _url: *mut AnyObject,
            _connections: *mut AnyObject,
            error: *mut AnyObject,
        ) {
            let key = this as usize;
            if let Some(tx) = delegate_finish_callbacks()
                .lock()
                .expect("delegate callback map poisoned")
                .remove(&key)
            {
                let result = if error.is_null() {
                    Ok(())
                } else {
                    Err(CaptureErrorResponse {
                        code: "capture_finalize_failed".to_string(),
                        message: "Native capture finalization failed".to_string(),
                    })
                };
                let _ = tx.send(result);
            }
        }

        let mut decl = ClassBuilder::new(
            &CString::new("ZNativeCaptureRecorderDelegate").expect("valid class name"),
            NSObject::class(),
        )
        .expect("failed to create recorder delegate class");

        if let Some(protocol) = AnyProtocol::get(
            &CString::new("AVCaptureFileOutputRecordingDelegate").expect("valid protocol name"),
        ) {
            decl.add_protocol(protocol);
        }

        unsafe {
            let did_start: extern "C-unwind" fn(
                *mut AnyObject,
                Sel,
                *mut AnyObject,
                *mut AnyObject,
                *mut AnyObject,
            ) = did_start_recording;
            decl.add_method(
                sel!(fileOutput:didStartRecordingToOutputFileAtURL:fromConnections:),
                did_start,
            );

            let did_finish: extern "C-unwind" fn(
                *mut AnyObject,
                Sel,
                *mut AnyObject,
                *mut AnyObject,
                *mut AnyObject,
                *mut AnyObject,
            ) = did_finish_recording;
            decl.add_method(
                sel!(fileOutput:didFinishRecordingToOutputFileAtURL:fromConnections:error:),
                did_finish,
            );
        }

        decl.register()
    })
}

#[cfg(target_os = "macos")]
fn delegate_finish_callbacks() -> &'static Mutex<FinishCallbackMap> {
    static CALLBACKS: OnceLock<Mutex<FinishCallbackMap>> = OnceLock::new();
    CALLBACKS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(target_os = "macos")]
fn delegate_start_callbacks() -> &'static Mutex<StartCallbackMap> {
    static CALLBACKS: OnceLock<Mutex<StartCallbackMap>> = OnceLock::new();
    CALLBACKS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(target_os = "macos")]
pub fn start_capture_session(
    session_dir: &Path,
    sources: &ScreenCaptureSources,
    screen_frame_rate: u32,
    screen_resolution: &ScreenResolution,
    video_bitrate_bps: Option<u32>,
) -> Result<StartedCaptureSession, CaptureErrorResponse> {
    if sources.screen && supports_screen_capture_kit_backend() {
        return start_screen_capture_kit_session(
            session_dir,
            sources,
            screen_frame_rate,
            screen_resolution,
            video_bitrate_bps,
        );
    }

    start_avfoundation_capture_session(session_dir, sources, screen_resolution, video_bitrate_bps)
}

#[cfg(target_os = "macos")]
fn start_avfoundation_capture_session(
    session_dir: &Path,
    sources: &ScreenCaptureSources,
    screen_resolution: &ScreenResolution,
    _video_bitrate_bps: Option<u32>,
) -> Result<StartedCaptureSession, CaptureErrorResponse> {
    use objc2_av_foundation::{
        AVCaptureInput, AVCaptureMovieFileOutput, AVCaptureOutput, AVCaptureScreenInput,
        AVCaptureSession,
    };
    use objc2_foundation::{NSObject, NSURL};

    create_session_dir(session_dir)?;

    if sources.screen
        && *screen_resolution
            != (ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            })
    {
        return Err(CaptureErrorResponse {
            code: "screen_resolution_unsupported".to_string(),
            message: "Selected screen resolution requires the ScreenCaptureKit backend (macOS 15+). On this backend, only the original display resolution is supported.".to_string(),
        });
    }

    let start_result = (|| {
        let output_file = session_dir.join("screen.mov");
        let output_file_str = output_file.to_string_lossy().to_string();

        let output_files = output_files_for_session(&session_dir, sources);

        let capture_session = unsafe { AVCaptureSession::new() };

        if sources.screen {
            let screen_input = unsafe { AVCaptureScreenInput::new() };
            let screen_input_ref: &AVCaptureInput =
                unsafe { &*(&*screen_input as *const _ as *const AVCaptureInput) };

            let can_add = unsafe { capture_session.canAddInput(screen_input_ref) };
            if can_add {
                unsafe { capture_session.addInput(screen_input_ref) };
            }

            if !can_add {
                return Err(CaptureErrorResponse {
                    code: "screen_input_unavailable".to_string(),
                    message: "Failed to add screen input".to_string(),
                });
            }
        }

        let movie_output = unsafe { AVCaptureMovieFileOutput::new() };

        let movie_output_ref: &AVCaptureOutput =
            unsafe { &*(&*movie_output as *const _ as *const AVCaptureOutput) };
        let can_add_output = unsafe { capture_session.canAddOutput(movie_output_ref) };
        if !can_add_output {
            return Err(CaptureErrorResponse {
                code: "capture_output_unavailable".to_string(),
                message: "Failed to add movie output".to_string(),
            });
        }

        unsafe { capture_session.addOutput(movie_output_ref) };

        let output_url =
            NSURL::from_file_path(&output_file).ok_or_else(|| CaptureErrorResponse {
                code: "capture_output_url_failed".to_string(),
                message: "Failed to create output URL for AVFoundation capture".to_string(),
            })?;

        let delegate_object: objc2::rc::Retained<NSObject> =
            unsafe { objc2::msg_send![recording_delegate_class(), new] };
        let delegate_key = (&*delegate_object as *const NSObject) as usize;
        let (start_tx, start_rx) = mpsc::channel::<()>();
        let (finish_tx, finish_rx) = mpsc::channel::<Result<(), CaptureErrorResponse>>();
        delegate_start_callbacks()
            .lock()
            .expect("delegate callback map poisoned")
            .insert(delegate_key, start_tx);
        delegate_finish_callbacks()
            .lock()
            .expect("delegate callback map poisoned")
            .insert(delegate_key, finish_tx);

        unsafe { capture_session.startRunning() };
        unsafe {
            let _: () = objc2::msg_send![
                &*movie_output,
                startRecordingToOutputFileURL: &*output_url,
                recordingDelegate: &*delegate_object
            ];
        }

        if start_rx.recv_timeout(Duration::from_secs(3)).is_err() {
            unsafe { movie_output.stopRecording() };
            unsafe { capture_session.stopRunning() };
            delegate_start_callbacks()
                .lock()
                .expect("delegate callback map poisoned")
                .remove(&delegate_key);
            delegate_finish_callbacks()
                .lock()
                .expect("delegate callback map poisoned")
                .remove(&delegate_key);
            return Err(CaptureErrorResponse {
                code: "capture_start_timeout".to_string(),
                message: "AVFoundation movie output did not transition to recording in time"
                    .to_string(),
            });
        }

        Ok(StartedCaptureSession {
            session: ActiveCaptureSession {
                backend: CaptureBackendSession::AvFoundation(AvFoundationCaptureSession {
                    capture_session,
                    movie_output,
                    _delegate: delegate_object,
                    delegate_key,
                    finish_rx,
                }),
            },
            recording_file: output_file_str,
            system_audio_recording_file: None,
            output_files,
        })
    })();

    finalize_startup_result(start_result, &session_dir)
}

#[cfg(target_os = "macos")]
fn start_screen_capture_kit_session(
    session_dir: &Path,
    sources: &ScreenCaptureSources,
    screen_frame_rate: u32,
    screen_resolution: &ScreenResolution,
    video_bitrate_bps: Option<u32>,
) -> Result<StartedCaptureSession, CaptureErrorResponse> {
    use cidre::{api, cm, ns, sc};

    if !api::version!(macos = 15.0) {
        return Err(CaptureErrorResponse {
            code: "screen_capture_kit_unsupported".to_string(),
            message: "ScreenCaptureKit recording requires macOS 15.0 or newer".to_string(),
        });
    }

    create_session_dir(session_dir)?;

    let start_result = (|| {
        let output_file = session_dir.join("screen.mov");
        let output_file_str = output_file.to_string_lossy().to_string();
        let system_audio_output_file = session_dir.join("system-audio.m4a");
        let system_audio_output_file_str = system_audio_output_file.to_string_lossy().to_string();

        let output_files = output_files_for_session(&session_dir, sources);

        let (content_tx, content_rx) =
            mpsc::channel::<Result<cidre::arc::R<sc::ShareableContent>, CaptureErrorResponse>>();
        sc::ShareableContent::current_with_ch(move |content, error| {
            let result = if let Some(content) = content {
                Ok(content.retained())
            } else if let Some(error) = error {
                Err(error_with_ns_error(
                    "capture_shareable_content_failed",
                    "Failed to query ScreenCaptureKit shareable content",
                    error,
                ))
            } else {
                Err(CaptureErrorResponse {
                    code: "capture_shareable_content_unavailable".to_string(),
                    message: "No ScreenCaptureKit shareable content available".to_string(),
                })
            };
            let _ = content_tx.send(result);
        });

        let content = match content_rx.recv_timeout(Duration::from_secs(20)) {
            Ok(result) => result?,
            Err(_) => {
                return Err(CaptureErrorResponse {
                    code: "capture_shareable_content_timeout".to_string(),
                    message: "Timed out while querying ScreenCaptureKit shareable content"
                        .to_string(),
                });
            }
        };

        let displays = content.displays();
        let display = displays.first().ok_or_else(|| CaptureErrorResponse {
            code: "capture_display_unavailable".to_string(),
            message: "No display available for ScreenCaptureKit capture".to_string(),
        })?;

        let excluded_windows = ns::Array::<sc::Window>::new();
        let filter = sc::ContentFilter::with_display_excluding_windows(display, &excluded_windows);

        let stream_resolution = resolve_stream_resolution(
            screen_resolution,
            display.width().max(1) as u32,
            display.height().max(1) as u32,
        );

        let mut screen_stream_cfg = sc::StreamCfg::new();
        screen_stream_cfg.set_width(stream_resolution.width as usize);
        screen_stream_cfg.set_height(stream_resolution.height as usize);
        screen_stream_cfg.set_minimum_frame_interval(cm::Time::new(1, screen_frame_rate as i32));
        screen_stream_cfg.set_shows_cursor(sources.screen);
        screen_stream_cfg.set_captures_audio(sources.system_audio);
        screen_stream_cfg.set_capture_mic(false);
        if sources.system_audio {
            screen_stream_cfg.set_sample_rate(48_000);
            screen_stream_cfg.set_channel_count(2);
        }

        let stream = cidre::sc::Stream::new(&filter, &screen_stream_cfg);
        let stream_output_queue = dispatch::Queue::serial_with_ar_pool();
        let stream_output_delegate =
            ScStreamOutputDelegate::with(stream_output_context_for_segment(
                &output_file_str,
                sources
                    .system_audio
                    .then_some(system_audio_output_file_str.as_str()),
                sources,
                video_bitrate_bps,
            )?);

        if sources.screen {
            stream
                .add_stream_output(
                    stream_output_delegate.as_ref(),
                    sc::OutputType::Screen,
                    Some(&stream_output_queue),
                )
                .map_err(|error| {
                    error_with_ns_error(
                        "capture_stream_output_attach_failed",
                        "Failed to attach ScreenCaptureKit screen output",
                        error,
                    )
                })?;
        }

        if sources.system_audio {
            stream
                .add_stream_output(
                    stream_output_delegate.as_ref(),
                    sc::OutputType::Audio,
                    Some(&stream_output_queue),
                )
                .map_err(|error| {
                    error_with_ns_error(
                        "capture_stream_output_attach_failed",
                        "Failed to attach ScreenCaptureKit system audio output",
                        error,
                    )
                })?;
        }

        start_screen_capture_kit_stream(&stream)?;

        Ok(StartedCaptureSession {
            session: ActiveCaptureSession {
                backend: CaptureBackendSession::ScreenCaptureKit(ScreenCaptureKitCaptureSession {
                    stream,
                    stream_output_delegate,
                    stream_output_queue,
                    sources: *sources,
                    video_bitrate_bps,
                }),
            },
            recording_file: output_file_str,
            system_audio_recording_file: sources
                .system_audio
                .then_some(system_audio_output_file_str),
            output_files,
        })
    })();

    finalize_startup_result(start_result, &session_dir)
}

#[cfg(target_os = "macos")]
fn stream_output_context_for_segment(
    recording_file: &str,
    system_audio_recording_file: Option<&str>,
    sources: &ScreenCaptureSources,
    video_bitrate_bps: Option<u32>,
) -> Result<StreamOutputContext, CaptureErrorResponse> {
    use cidre::ns;

    let screen_video_output_file = if sources.screen {
        Some(recording_file.to_string())
    } else {
        None
    };
    let screen_video_writer = None;

    let system_audio_writer = if sources.system_audio {
        let output_file = system_audio_recording_file.ok_or_else(|| CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Missing system audio output file while creating segment writer".to_string(),
        })?;
        let output_url = ns::Url::with_fs_path_str(output_file, false);
        Some(create_audio_asset_writer(&output_url, "system audio")?)
    } else {
        None
    };

    Ok(StreamOutputContext {
        screen_video_output_file,
        screen_video_writer,
        video_bitrate_bps,
        system_audio_writer,
        first_error: None,
    })
}

#[cfg(target_os = "macos")]
fn start_screen_capture_kit_stream(stream: &cidre::sc::Stream) -> Result<(), CaptureErrorResponse> {
    let (start_tx, start_rx) = mpsc::channel::<Result<(), CaptureErrorResponse>>();
    let mut completion = cidre::blocks::ErrCh::new1(move |error| {
        let _ = start_tx.send(match error {
            Some(error) => Err(error_with_ns_error(
                "capture_stream_start_failed",
                "Failed to start ScreenCaptureKit capture",
                error,
            )),
            None => Ok(()),
        });
    });

    stream.start_with_ch_block(Some(&mut completion));

    match start_rx.recv_timeout(Duration::from_secs(20)) {
        Ok(result) => result,
        Err(_) => Err(CaptureErrorResponse {
            code: "capture_stream_start_timeout".to_string(),
            message: "Timed out while starting ScreenCaptureKit stream capture".to_string(),
        }),
    }
}

#[cfg(target_os = "macos")]
fn synchronize_stream_output_queue(queue: Option<&dispatch::Queue>) {
    if let Some(queue) = queue {
        queue.sync(|| ());
    }
}

#[cfg(target_os = "macos")]
fn finalize_stream_output_context(
    context: &mut StreamOutputContext,
) -> Result<(), CaptureErrorResponse> {
    let screen_video_output_file = context.screen_video_output_file.as_deref();

    if context.screen_video_output_file.is_some() && context.screen_video_writer.is_none() {
        if let Some(path) = screen_video_output_file {
            maybe_remove_screen_video_file(path);
        }

        if let Some(error) = context.first_error.take() {
            return Err(error);
        }
        return Err(capture_writers::no_video_samples_error("screen"));
    }

    if let Err(error) = writers_finalize_stream_output_context(
        context.screen_video_writer.as_mut(),
        context.system_audio_writer.as_mut(),
        context.first_error.take(),
    ) {
        if let Some(path) = screen_video_output_file {
            maybe_remove_screen_video_file(path);
        }
        return Err(error);
    }

    if let Some(path) = screen_video_output_file {
        if let Err(error) = validate_screen_video_file(path) {
            maybe_remove_screen_video_file(path);
            return Err(error);
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
pub struct RotateScreenCaptureSessionArgs<'a> {
    pub active_session: &'a mut Option<ActiveCaptureSession>,
    pub segment_dir: &'a Path,
}

#[cfg(target_os = "macos")]
pub fn rotate_screen_capture_session(
    args: RotateScreenCaptureSessionArgs<'_>,
) -> Result<RotatedCaptureOutputs, CaptureErrorResponse> {
    let Some(session) = args.active_session.as_mut() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Missing active screen capture session for segment rotation".to_string(),
        });
    };

    match &mut session.backend {
        CaptureBackendSession::ScreenCaptureKit(session) => {
            session.rotate_output_files(args.segment_dir)
        }
        CaptureBackendSession::AvFoundation(_) => Err(CaptureErrorResponse {
            code: "capture_rotation_requires_restart".to_string(),
            message: "This capture backend requires full restart for segment rotation".to_string(),
        }),
    }
}

#[cfg(target_os = "macos")]
pub struct StopScreenCaptureSessionArgs<'a> {
    pub active_session: &'a mut Option<ActiveCaptureSession>,
}

#[cfg(target_os = "macos")]
pub fn stop_screen_capture_session(
    args: StopScreenCaptureSessionArgs<'_>,
) -> Result<(), CaptureErrorResponse> {
    let mut stop_error: Option<CaptureErrorResponse> = None;

    if let Some(session) = args.active_session.as_mut() {
        match session.stop() {
            Ok(()) => {
                *args.active_session = None;
            }
            Err(error)
                if ScreenCaptureKitCaptureSession::is_stop_timeout_code(error.code.as_str()) =>
            {
                return Err(error);
            }
            Err(error) => {
                stop_error = Some(error);
                *args.active_session = None;
            }
        }
    }

    if let Some(error) = stop_error {
        Err(error)
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[cfg(target_os = "macos")]
pub fn screen_permission_state() -> CapturePermissionState {
    if unsafe { CGPreflightScreenCaptureAccess() } {
        CapturePermissionState::Granted
    } else if SCREEN_PERMISSION_REQUESTED.load(Ordering::SeqCst) {
        CapturePermissionState::Denied
    } else {
        CapturePermissionState::Unknown
    }
}

#[cfg(target_os = "macos")]
pub fn ensure_screen_permission() -> bool {
    SCREEN_PERMISSION_REQUESTED.store(true, Ordering::SeqCst);
    unsafe { CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess() }
}

#[cfg(target_os = "macos")]
pub fn system_audio_permission_state() -> CapturePermissionState {
    if supports_system_audio_capture() {
        screen_permission_state()
    } else {
        CapturePermissionState::Unsupported
    }
}

#[cfg(target_os = "macos")]
pub fn supports_system_audio_capture() -> bool {
    cidre::api::version!(macos = 15.0)
}

#[cfg(target_os = "macos")]
pub fn should_preserve_runtime_on_stop_error(error: &CaptureErrorResponse) -> bool {
    ScreenCaptureKitCaptureSession::is_stop_timeout_code(error.code.as_str())
}

#[cfg(target_os = "macos")]
fn supports_screen_capture_kit_backend() -> bool {
    cidre::api::version!(macos = 15.0)
}

#[cfg(not(target_os = "macos"))]
pub struct ActiveCaptureSession;

#[cfg(not(target_os = "macos"))]
pub struct StartedCaptureSession {
    pub session: ActiveCaptureSession,
    pub recording_file: String,
    pub system_audio_recording_file: Option<String>,
    pub output_files: CaptureOutputFiles,
}

#[cfg(not(target_os = "macos"))]
pub struct RotatedCaptureOutputs {
    pub recording_file: String,
    pub system_audio_recording_file: Option<String>,
    pub output_files: CaptureOutputFiles,
}

#[cfg(not(target_os = "macos"))]
pub struct StopScreenCaptureSessionArgs<'a> {
    pub active_session: &'a mut Option<ActiveCaptureSession>,
}

#[cfg(not(target_os = "macos"))]
pub struct RotateScreenCaptureSessionArgs<'a> {
    pub active_session: &'a mut Option<ActiveCaptureSession>,
    pub segment_dir: &'a Path,
}

#[cfg(not(target_os = "macos"))]
pub fn new_session_id() -> Result<String, CaptureErrorResponse> {
    Err(CaptureErrorResponse {
        code: "unsupported_platform".to_string(),
        message: "Native capture is currently supported only on macOS".to_string(),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn start_capture_session(
    _session_dir: &Path,
    _sources: &ScreenCaptureSources,
    _screen_frame_rate: u32,
    _screen_resolution: &ScreenResolution,
    _video_bitrate_bps: Option<u32>,
) -> Result<StartedCaptureSession, CaptureErrorResponse> {
    Err(CaptureErrorResponse {
        code: "unsupported_platform".to_string(),
        message: "Native capture is currently supported only on macOS".to_string(),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn stop_screen_capture_session(
    _args: StopScreenCaptureSessionArgs<'_>,
) -> Result<(), CaptureErrorResponse> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn rotate_screen_capture_session(
    _args: RotateScreenCaptureSessionArgs<'_>,
) -> Result<RotatedCaptureOutputs, CaptureErrorResponse> {
    Err(CaptureErrorResponse {
        code: "unsupported_platform".to_string(),
        message: "Native capture is currently supported only on macOS".to_string(),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn screen_permission_state() -> CapturePermissionState {
    CapturePermissionState::Unsupported
}

#[cfg(not(target_os = "macos"))]
pub fn ensure_screen_permission() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn system_audio_permission_state() -> CapturePermissionState {
    CapturePermissionState::Unsupported
}

#[cfg(not(target_os = "macos"))]
pub fn supports_system_audio_capture() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn should_preserve_runtime_on_stop_error(_error: &CaptureErrorResponse) -> bool {
    false
}

pub fn support_for_current_platform() -> ScreenCaptureSupport {
    #[cfg(target_os = "macos")]
    {
        ScreenCaptureSupport {
            platform: "macos".to_string(),
            native_capture_supported: true,
            screen: true,
            system_audio: supports_system_audio_capture(),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        ScreenCaptureSupport {
            platform: std::env::consts::OS.to_string(),
            native_capture_supported: false,
            screen: false,
            system_audio: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_stream_resolution_scales_preset_with_display_aspect_ratio() {
        let resolved = resolve_stream_resolution(
            &ScreenResolution::Preset {
                preset: ScreenResolutionPreset::P720,
            },
            2560,
            1440,
        );

        assert_eq!(resolved.width, 1280);
        assert_eq!(resolved.height, 720);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_stream_resolution_clamps_preset_to_display_size() {
        let resolved = resolve_stream_resolution(
            &ScreenResolution::Preset {
                preset: ScreenResolutionPreset::P1080,
            },
            1366,
            768,
        );

        assert_eq!(resolved.width, 1366);
        assert_eq!(resolved.height, 768);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_stream_resolution_keeps_custom_dimensions() {
        let resolved = resolve_stream_resolution(
            &ScreenResolution::Custom {
                width: 1001,
                height: 777,
            },
            1920,
            1080,
        );

        assert_eq!(resolved.width, 1002);
        assert_eq!(resolved.height, 778);
    }
}

#[cfg(target_os = "macos")]
fn load_asset_tracks_with_timeout(
    asset: &cidre::av::UrlAsset,
    media_type: &cidre::av::MediaType,
    timeout_code: &str,
    timeout_message: &str,
) -> Result<cidre::arc::R<cidre::ns::Array<cidre::av::asset::Track>>, CaptureErrorResponse> {
    let (tx, rx) = mpsc::channel::<
        Result<cidre::arc::R<cidre::ns::Array<cidre::av::asset::Track>>, CaptureErrorResponse>,
    >();

    asset.load_tracks_with_media_type_block(media_type, move |tracks, error| {
        let result = if let Some(tracks) = tracks {
            Ok(tracks.retained())
        } else if let Some(error) = error {
            Err(error_with_ns_error(
                "capture_output_processing_failed",
                "Failed to query recording tracks",
                error,
            ))
        } else {
            Err(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to query recording tracks".to_string(),
            })
        };

        let _ = tx.send(result);
    });

    match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(result) => result,
        Err(_) => Err(CaptureErrorResponse {
            code: timeout_code.to_string(),
            message: timeout_message.to_string(),
        }),
    }
}

#[cfg(target_os = "macos")]
pub fn strip_audio_from_recording_file(recording_file: &str) -> Result<(), CaptureErrorResponse> {
    use cidre::{av, ns};

    let input_path = std::path::Path::new(recording_file);
    let temp_path = input_path.with_extension("video-only.mov");
    let _ = std::fs::remove_file(&temp_path);

    let input_url = ns::Url::with_fs_path_str(recording_file, false);
    let temp_url = ns::Url::with_fs_path_str(temp_path.to_string_lossy().as_ref(), false);

    let asset = av::UrlAsset::with_url(&input_url, None).ok_or_else(|| CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message: "Failed to open recording for video-only conversion".to_string(),
    })?;

    let video_tracks = load_asset_tracks_with_timeout(
        asset.as_ref(),
        av::MediaType::video(),
        "capture_output_processing_failed",
        "Timed out while loading recording video track",
    )?;
    let video_track = video_tracks.first().ok_or_else(|| CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message: "Recording has no video track to preserve".to_string(),
    })?;

    let mut reader =
        av::AssetReader::with_asset(asset.as_ref()).map_err(|_| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Failed to create asset reader for video-only conversion".to_string(),
        })?;

    let mut reader_output =
        av::AssetReaderTrackOutput::with_track(video_track, None).map_err(|_| {
            CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to create video track reader output".to_string(),
            }
        })?;
    reader_output.set_always_copies_sample_data(false);

    let reader_output_ref: &av::AssetReaderOutput =
        unsafe { &*(&*reader_output as *const _ as *const av::AssetReaderOutput) };
    if !reader.can_add_output(reader_output_ref) {
        return Err(CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Failed to add video track reader output".to_string(),
        });
    }
    reader
        .add_output(reader_output_ref)
        .map_err(|_| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Failed to attach video track reader output".to_string(),
        })?;

    let mut writer = av::AssetWriter::with_url_and_file_type(&temp_url, av::FileType::qt())
        .map_err(|_| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Failed to create video-only asset writer".to_string(),
        })?;
    let mut writer_input =
        av::AssetWriterInput::with_media_type_and_output_settings(av::MediaType::video(), None)
            .map_err(|_| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to create video-only writer input".to_string(),
            })?;
    writer_input.set_expects_media_data_in_real_time(false);

    if !writer.can_add_input(&writer_input) {
        return Err(CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Failed to add video-only writer input".to_string(),
        });
    }
    writer
        .add_input(&writer_input)
        .map_err(|_| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Failed to attach video-only writer input".to_string(),
        })?;

    let started_reading = reader.start_reading().map_err(|_| CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message: "Failed to start reading recording for video-only conversion".to_string(),
    })?;
    if !started_reading {
        if let Some(error) = reader.error() {
            return Err(error_with_ns_error(
                "capture_output_processing_failed",
                "Failed to start reading recording for video-only conversion",
                error.as_ref(),
            ));
        }

        return Err(CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Failed to start reading recording for video-only conversion".to_string(),
        });
    }

    if !writer.start_writing() {
        return Err(capture_writers::writer_error_response(
            &writer,
            "capture_output_processing_failed",
            "Failed to start writing video-only recording",
        ));
    }
    writer.start_session_at_src_time(cidre::cm::Time::zero());

    loop {
        let sample_buf = reader_output
            .next_sample_buf()
            .map_err(|_| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to read video sample during video-only conversion".to_string(),
            })?;

        let Some(sample_buf) = sample_buf else {
            break;
        };

        while !writer_input.is_ready_for_more_media_data() {
            std::thread::sleep(Duration::from_millis(1));
        }

        let appended = writer_input
            .append_sample_buf(sample_buf.as_ref())
            .map_err(|_| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to append video sample during video-only conversion".to_string(),
            })?;

        if !appended {
            return Err(capture_writers::writer_error_response(
                &writer,
                "capture_output_processing_failed",
                "Failed to append video sample during video-only conversion",
            ));
        }
    }

    writer_input.mark_as_finished();
    writer.finish_writing();

    let wait_deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        match writer.status() {
            cidre::av::asset::WriterStatus::Completed => break,
            cidre::av::asset::WriterStatus::Failed => {
                return Err(capture_writers::writer_error_response(
                    &writer,
                    "capture_output_processing_failed",
                    "Failed to finalize video-only recording",
                ));
            }
            status if std::time::Instant::now() >= wait_deadline => {
                return Err(CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: format!(
                        "Timed out while finalizing video-only recording (status: {:?})",
                        status
                    ),
                });
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }

    match reader.status() {
        cidre::av::asset::ReaderStatus::Completed => {}
        cidre::av::asset::ReaderStatus::Failed => {
            if let Some(error) = reader.error() {
                return Err(error_with_ns_error(
                    "capture_output_processing_failed",
                    "Video-only conversion reader failed",
                    error.as_ref(),
                ));
            }

            return Err(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Video-only conversion reader failed".to_string(),
            });
        }
        status => {
            return Err(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: format!(
                    "Video-only conversion reader ended unexpectedly (status: {:?})",
                    status
                ),
            });
        }
    }

    let video_only_asset =
        av::UrlAsset::with_url(&temp_url, None).ok_or_else(|| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Failed to verify video-only recording".to_string(),
        })?;
    let audio_tracks = load_asset_tracks_with_timeout(
        video_only_asset.as_ref(),
        av::MediaType::audio(),
        "capture_output_processing_failed",
        "Timed out while validating video-only recording audio tracks",
    )?;
    if !audio_tracks.is_empty() {
        return Err(CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Video-only conversion produced an unexpected audio track".to_string(),
        });
    }

    std::fs::rename(&temp_path, input_path).map_err(|error| CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message: format!("Failed to replace recording with video-only mov: {error}"),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn strip_audio_from_recording_file(_recording_file: &str) -> Result<(), CaptureErrorResponse> {
    Ok(())
}
