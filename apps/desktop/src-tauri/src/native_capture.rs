use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapturePermissionState {
    Granted,
    Denied,
    NotDetermined,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSources {
    pub screen: bool,
    pub microphone: bool,
    pub system_audio: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSupportResponse {
    pub platform: String,
    pub native_capture_supported: bool,
    pub supported_sources: CaptureSources,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePermissions {
    pub screen: CapturePermissionState,
    pub microphone: CapturePermissionState,
    pub system_audio: CapturePermissionState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOutputFiles {
    pub screen_file: Option<String>,
    pub microphone_file: Option<String>,
    pub system_audio_file: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCaptureSession {
    pub is_running: bool,
    pub session_id: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub requested_sources: Option<CaptureSources>,
    pub output_files: Option<CaptureOutputFiles>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePermissionsResponse {
    pub permissions: CapturePermissions,
    pub session: NativeCaptureSession,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartNativeCaptureRequest {
    pub capture_screen: bool,
    pub capture_microphone: bool,
    pub capture_system_audio: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCaptureSessionResponse {
    pub session: NativeCaptureSession,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureErrorResponse {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct NativeCaptureRuntime {
    pub is_running: bool,
    pub session_id: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub requested_sources: Option<CaptureSources>,
    pub output_files: Option<CaptureOutputFiles>,
    #[cfg(target_os = "macos")]
    pub recording_file: Option<String>,
    #[cfg(target_os = "macos")]
    pub microphone_recording_file: Option<String>,
    #[cfg(target_os = "macos")]
    pub system_audio_recording_file: Option<String>,
    #[cfg(target_os = "macos")]
    pub active_session: Option<platform::ActiveCaptureSession>,
}

pub type NativeCaptureState = Mutex<NativeCaptureRuntime>;

fn output_files_for_session(session_dir: &Path, sources: &CaptureSources) -> CaptureOutputFiles {
    CaptureOutputFiles {
        screen_file: sources
            .screen
            .then_some(session_dir.join("screen.mov").to_string_lossy().to_string()),
        microphone_file: sources.microphone.then_some(
            session_dir
                .join("microphone.m4a")
                .to_string_lossy()
                .to_string(),
        ),
        system_audio_file: sources.system_audio.then_some(
            session_dir
                .join("system-audio.m4a")
                .to_string_lossy()
                .to_string(),
        ),
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[tauri::command]
pub fn get_capture_support() -> CaptureSupportResponse {
    #[cfg(target_os = "macos")]
    {
        CaptureSupportResponse {
            platform: "macos".to_string(),
            native_capture_supported: true,
            supported_sources: CaptureSources {
                screen: true,
                microphone: true,
                system_audio: platform::supports_system_audio_capture(),
            },
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        CaptureSupportResponse {
            platform: std::env::consts::OS.to_string(),
            native_capture_supported: false,
            supported_sources: CaptureSources {
                screen: false,
                microphone: false,
                system_audio: false,
            },
        }
    }
}

fn session_from_runtime(runtime: &NativeCaptureRuntime) -> NativeCaptureSession {
    NativeCaptureSession {
        is_running: runtime.is_running,
        session_id: runtime.session_id.clone(),
        started_at_unix_ms: runtime.started_at_unix_ms,
        requested_sources: runtime.requested_sources.clone(),
        output_files: runtime.output_files.clone(),
    }
}

fn stopped_session_from_runtime(runtime: &NativeCaptureRuntime) -> NativeCaptureSession {
    NativeCaptureSession {
        is_running: false,
        session_id: runtime.session_id.clone(),
        started_at_unix_ms: runtime.started_at_unix_ms,
        requested_sources: runtime.requested_sources.clone(),
        output_files: runtime.output_files.clone(),
    }
}

fn validate_start_request(
    request: &StartNativeCaptureRequest,
    support: &CaptureSupportResponse,
) -> Result<CaptureSources, CaptureErrorResponse> {
    if !request.capture_screen && !request.capture_microphone && !request.capture_system_audio {
        return Err(CaptureErrorResponse {
            code: "invalid_request".to_string(),
            message: "At least one capture source must be enabled".to_string(),
        });
    }

    if !support.native_capture_supported {
        return Err(CaptureErrorResponse {
            code: "unsupported_platform".to_string(),
            message: "Native capture is currently supported only on macOS".to_string(),
        });
    }

    if request.capture_system_audio && !support.supported_sources.system_audio {
        return Err(CaptureErrorResponse {
            code: "system_audio_unsupported".to_string(),
            message: "System audio capture requires macOS 15.0 or newer".to_string(),
        });
    }

    if request.capture_system_audio && !request.capture_screen {
        return Err(CaptureErrorResponse {
            code: "system_audio_requires_screen".to_string(),
            message: "System audio-only capture is not supported; enable screen capture as well"
                .to_string(),
        });
    }

    Ok(CaptureSources {
        screen: request.capture_screen,
        microphone: request.capture_microphone,
        system_audio: request.capture_system_audio,
    })
}

fn mark_runtime_session_stopped(runtime: &mut NativeCaptureRuntime) {
    runtime.is_running = false;
    #[cfg(target_os = "macos")]
    {
        runtime.active_session = None;
    }
}

fn should_remove_intermediate_recording(sources: &CaptureSources) -> bool {
    !sources.screen && sources.microphone && !sources.system_audio
}

fn should_fallback_to_primary_recording_for_audio(sources: &CaptureSources) -> bool {
    !sources.screen && sources.microphone && !sources.system_audio
}

fn should_strip_screen_recording_audio(sources: &CaptureSources) -> bool {
    sources.screen && sources.system_audio
}

fn should_use_sample_buffer_microphone_only_path(sources: &CaptureSources) -> bool {
    !sources.screen && sources.microphone && !sources.system_audio
}

#[tauri::command]
pub fn get_capture_permissions(
    state: tauri::State<'_, NativeCaptureState>,
) -> CapturePermissionsResponse {
    let runtime = state.lock().expect("native capture state poisoned");
    CapturePermissionsResponse {
        permissions: CapturePermissions {
            screen: platform::screen_permission_state(),
            microphone: platform::microphone_permission_state(),
            system_audio: platform::system_audio_permission_state(),
        },
        session: session_from_runtime(&runtime),
    }
}

#[tauri::command]
pub fn start_native_capture(
    request: StartNativeCaptureRequest,
    state: tauri::State<'_, NativeCaptureState>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let support = get_capture_support();
    let sources = validate_start_request(&request, &support)?;

    let mut runtime = state.lock().expect("native capture state poisoned");
    if runtime.is_running {
        if runtime.requested_sources.as_ref() != Some(&sources) {
            return Err(CaptureErrorResponse {
                code: "capture_session_already_running".to_string(),
                message: "A native capture session is already running with different sources"
                    .to_string(),
            });
        }

        return Ok(NativeCaptureSessionResponse {
            session: session_from_runtime(&runtime),
        });
    }

    if request.capture_screen || request.capture_system_audio {
        let screen_ok = platform::ensure_screen_permission();
        if !screen_ok {
            return Err(CaptureErrorResponse {
                code: "screen_permission_denied".to_string(),
                message: if request.capture_system_audio {
                    "Screen capture permission is required for system audio capture"
                } else {
                    "Screen capture permission is required"
                }
                .to_string(),
            });
        }
    }

    if request.capture_microphone {
        let microphone_ok = platform::ensure_microphone_permission();
        if !microphone_ok {
            return Err(CaptureErrorResponse {
                code: "microphone_permission_denied".to_string(),
                message: "Microphone permission is required".to_string(),
            });
        }
    }

    #[cfg(target_os = "macos")]
    {
        let started = now_unix_ms();
        let session_id = platform::new_session_id()?;
        let capture = platform::start_capture_session(&session_id, &sources)?;

        runtime.is_running = true;
        runtime.started_at_unix_ms = Some(started);
        runtime.session_id = Some(session_id);
        runtime.requested_sources = Some(sources);
        runtime.output_files = Some(CaptureOutputFiles {
            screen_file: capture.output_files.screen_file,
            microphone_file: capture.output_files.microphone_file,
            system_audio_file: capture.output_files.system_audio_file,
        });
        runtime.recording_file = Some(capture.recording_file);
        runtime.microphone_recording_file = capture.microphone_recording_file;
        runtime.system_audio_recording_file = capture.system_audio_recording_file;
        runtime.active_session = Some(capture.session);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = sources;
        return Err(CaptureErrorResponse {
            code: "unsupported_platform".to_string(),
            message: "Native capture is currently supported only on macOS".to_string(),
        });
    }

    Ok(NativeCaptureSessionResponse {
        session: session_from_runtime(&runtime),
    })
}

#[tauri::command]
pub fn stop_native_capture(
    state: tauri::State<'_, NativeCaptureState>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let mut runtime = state.lock().expect("native capture state poisoned");

    let stop_result: Result<(), CaptureErrorResponse> = {
        #[cfg(target_os = "macos")]
        {
            platform::stop_capture_session(&mut runtime)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok(())
        }
    };

    if let Err(error) = stop_result {
        if platform::should_preserve_runtime_on_stop_error(&error) {
            return Err(error);
        }

        mark_runtime_session_stopped(&mut runtime);
        return Err(error);
    }

    mark_runtime_session_stopped(&mut runtime);
    let session = stopped_session_from_runtime(&runtime);

    Ok(NativeCaptureSessionResponse { session })
}

#[allow(clippy::useless_transmute)]
mod platform {
    use super::{
        output_files_for_session, should_fallback_to_primary_recording_for_audio,
        should_remove_intermediate_recording, should_strip_screen_recording_audio,
        should_use_sample_buffer_microphone_only_path, CaptureErrorResponse, CaptureOutputFiles,
        CapturePermissionState, CaptureSources, NativeCaptureRuntime,
    };
    #[cfg(target_os = "macos")]
    use cidre::arc::Retain;
    #[cfg(target_os = "macos")]
    use cidre::av::capture::AudioDataOutputSampleBufDelegate;
    #[cfg(target_os = "macos")]
    use cidre::dispatch;
    #[cfg(target_os = "macos")]
    use cidre::objc;
    #[cfg(target_os = "macos")]
    use cidre::sc::StreamOutput;
    #[cfg(target_os = "macos")]
    use std::collections::HashMap;
    #[cfg(target_os = "macos")]
    use std::ffi::CString;
    #[cfg(target_os = "macos")]
    use std::fmt::Display;
    use std::path::PathBuf;
    #[cfg(target_os = "macos")]
    use std::process::Command;
    #[cfg(target_os = "macos")]
    use std::sync::atomic::{AtomicBool, Ordering};
    #[cfg(target_os = "macos")]
    use std::sync::mpsc;
    #[cfg(target_os = "macos")]
    use std::sync::{Mutex, OnceLock};
    #[cfg(target_os = "macos")]
    use std::time::{Duration, Instant};

    #[cfg(target_os = "macos")]
    static SCREEN_PERMISSION_REQUESTED: AtomicBool = AtomicBool::new(false);

    #[cfg(target_os = "macos")]
    type ScreenCaptureKitRecordingStreamStart = (
        cidre::arc::R<cidre::sc::Stream>,
        cidre::arc::R<cidre::sc::RecordingOutput>,
        cidre::arc::R<ScRecordingOutputDelegate>,
    );

    #[cfg(target_os = "macos")]
    #[derive(Debug)]
    struct AudioAssetWriterState {
        writer: cidre::arc::R<cidre::av::AssetWriter>,
        input: cidre::arc::R<cidre::av::AssetWriterInput>,
        started: bool,
        appended_samples: u64,
        label: &'static str,
    }

    #[cfg(target_os = "macos")]
    #[derive(Debug)]
    struct StreamOutputContext {
        microphone_writer: Option<AudioAssetWriterState>,
        system_audio_writer: Option<AudioAssetWriterState>,
        first_error: Option<CaptureErrorResponse>,
    }

    #[cfg(target_os = "macos")]
    #[derive(Debug)]
    struct MicrophoneOutputContext {
        writer: AudioAssetWriterState,
        first_error: Option<CaptureErrorResponse>,
    }

    #[cfg(target_os = "macos")]
    cidre::define_obj_type!(
        ScStreamOutputDelegate + cidre::sc::StreamOutputImpl,
        StreamOutputContext,
        ZScStreamOutputDelegate
    );

    #[cfg(target_os = "macos")]
    impl cidre::sc::StreamOutput for ScStreamOutputDelegate {}

    #[cfg(target_os = "macos")]
    cidre::define_obj_type!(
        MicAudioDataOutputDelegate + cidre::av::capture::AudioDataOutputSampleBufDelegateImpl,
        MicrophoneOutputContext,
        ZMicAudioDataOutputDelegate
    );

    #[cfg(target_os = "macos")]
    impl cidre::av::capture::AudioDataOutputSampleBufDelegate for MicAudioDataOutputDelegate {}

    #[cfg(target_os = "macos")]
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

            let writer_state = match kind {
                cidre::sc::OutputType::Mic => ctx.microphone_writer.as_mut(),
                cidre::sc::OutputType::Audio => ctx.system_audio_writer.as_mut(),
                cidre::sc::OutputType::Screen => None,
            };

            let Some(writer_state) = writer_state else {
                return;
            };

            if let Err(error) = append_audio_sample_to_writer(writer_state, sample_buf) {
                if ctx.first_error.is_none() {
                    ctx.first_error = Some(error);
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
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

    #[cfg(target_os = "macos")]
    pub struct StartedCaptureSession {
        pub session: ActiveCaptureSession,
        pub recording_file: String,
        pub microphone_recording_file: Option<String>,
        pub system_audio_recording_file: Option<String>,
        pub output_files: CaptureOutputFiles,
    }

    #[cfg(target_os = "macos")]
    type FinishResult = Result<(), CaptureErrorResponse>;
    #[cfg(target_os = "macos")]
    type StartCallbackMap = HashMap<usize, mpsc::Sender<()>>;
    #[cfg(target_os = "macos")]
    type FinishCallbackMap = HashMap<usize, mpsc::Sender<FinishResult>>;

    #[cfg(target_os = "macos")]
    #[derive(Debug)]
    struct AvFoundationCaptureSession {
        capture_session: objc2::rc::Retained<objc2_av_foundation::AVCaptureSession>,
        movie_output: objc2::rc::Retained<objc2_av_foundation::AVCaptureMovieFileOutput>,
        _delegate: objc2::rc::Retained<objc2_foundation::NSObject>,
        delegate_key: usize,
        finish_rx: mpsc::Receiver<FinishResult>,
    }

    #[cfg(target_os = "macos")]
    #[derive(Debug)]
    struct AvFoundationMicrophoneCaptureSession {
        capture_session: cidre::arc::R<cidre::av::capture::Session>,
        _audio_output: cidre::arc::R<cidre::av::capture::AudioDataOutput>,
        output_delegate: cidre::arc::R<MicAudioDataOutputDelegate>,
        output_queue: cidre::arc::R<dispatch::Queue>,
    }

    #[cfg(target_os = "macos")]
    #[derive(Debug)]
    struct ScreenCaptureKitCaptureSession {
        stream: cidre::arc::R<cidre::sc::Stream>,
        _screen_recording_output: cidre::arc::R<cidre::sc::RecordingOutput>,
        _screen_delegate: cidre::arc::R<ScRecordingOutputDelegate>,
        microphone_session: Option<AvFoundationMicrophoneCaptureSession>,
        _stream_output_delegate: Option<cidre::arc::R<ScStreamOutputDelegate>>,
        _stream_output_queue: Option<cidre::arc::R<dispatch::Queue>>,
    }

    #[cfg(target_os = "macos")]
    #[derive(Debug)]
    enum CaptureBackendSession {
        AvFoundation(AvFoundationCaptureSession),
        AvFoundationMicrophone(AvFoundationMicrophoneCaptureSession),
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
                CaptureBackendSession::AvFoundationMicrophone(session) => session.stop(),
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
    impl AvFoundationMicrophoneCaptureSession {
        fn stop(&mut self) -> Result<(), CaptureErrorResponse> {
            self.capture_session.stop_running();
            synchronize_stream_output_queue(Some(self.output_queue.as_ref()));
            finalize_microphone_output_context(self.output_delegate.inner_mut())
        }
    }

    #[cfg(target_os = "macos")]
    cidre::define_obj_type!(
        ScRecordingOutputDelegate + cidre::sc::RecordingOutputDelegateImpl,
        (),
        ZScRecordingOutputDelegate
    );

    #[cfg(target_os = "macos")]
    impl cidre::sc::RecordingOutputDelegate for ScRecordingOutputDelegate {}

    #[cfg(target_os = "macos")]
    #[cidre::objc::add_methods]
    impl cidre::sc::RecordingOutputDelegateImpl for ScRecordingOutputDelegate {}

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
                synchronize_stream_output_queue(self._stream_output_queue.as_deref());

                if let Some(delegate) = self._stream_output_delegate.as_mut() {
                    if let Err(error) = finalize_stream_output_context(delegate.inner_mut()) {
                        if stop_error.is_none() {
                            stop_error = Some(error);
                        }
                    }
                }
            }

            if let Some(microphone_session) = self.microphone_session.as_mut() {
                if let Err(error) = microphone_session.stop() {
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
    }

    #[cfg(target_os = "macos")]
    fn fmt_ns<T: Display + ?Sized>(value: &T) -> String {
        format!("{value}")
    }

    #[cfg(target_os = "macos")]
    fn error_with_ns_error(
        code: &str,
        prefix: &str,
        error: &cidre::ns::Error,
    ) -> CaptureErrorResponse {
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
    fn capture_root() -> Result<PathBuf, CaptureErrorResponse> {
        let root = std::env::temp_dir().join("z-native-capture");
        std::fs::create_dir_all(&root).map_err(|e| CaptureErrorResponse {
            code: "io_error".to_string(),
            message: format!("Failed to create capture temp directory: {e}"),
        })?;
        Ok(root)
    }

    #[cfg(target_os = "macos")]
    fn create_session_dir(session_id: &str) -> Result<PathBuf, CaptureErrorResponse> {
        let root = capture_root()?;
        let session_dir = root.join(session_id);
        std::fs::create_dir(&session_dir).map_err(|e| CaptureErrorResponse {
            code: "io_error".to_string(),
            message: format!("Failed to create capture session directory: {e}"),
        })?;
        Ok(session_dir)
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
    fn combine_startup_error_with_rollback_error(
        start_error: CaptureErrorResponse,
        rollback_error: CaptureErrorResponse,
    ) -> CaptureErrorResponse {
        if rollback_error.code == "capture_start_rollback_incomplete" {
            return rollback_error;
        }

        CaptureErrorResponse {
            code: start_error.code,
            message: format!(
                "{}; additionally failed startup rollback: [{}] {}",
                start_error.message, rollback_error.code, rollback_error.message
            ),
        }
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

            // SAFETY: Method signatures match Objective-C delegate contract.
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
        session_id: &str,
        sources: &CaptureSources,
    ) -> Result<StartedCaptureSession, CaptureErrorResponse> {
        if sources.screen && sources.microphone && !supports_screen_capture_kit_backend() {
            return Err(CaptureErrorResponse {
                code: "separate_audio_unsupported".to_string(),
                message:
                    "Capturing screen and microphone as separate files requires macOS 15.0 or newer"
                        .to_string(),
            });
        }

        if should_use_sample_buffer_microphone_only_path(sources) {
            return start_avfoundation_microphone_only_capture_session(session_id, sources);
        }

        if sources.screen && supports_screen_capture_kit_backend() {
            return start_screen_capture_kit_session(session_id, sources);
        }

        start_avfoundation_capture_session(session_id, sources)
    }

    #[cfg(target_os = "macos")]
    fn start_avfoundation_capture_session(
        session_id: &str,
        sources: &CaptureSources,
    ) -> Result<StartedCaptureSession, CaptureErrorResponse> {
        use objc2_av_foundation::{
            AVCaptureDevice, AVCaptureDeviceInput, AVCaptureInput, AVCaptureMovieFileOutput,
            AVCaptureOutput, AVCaptureScreenInput, AVCaptureSession, AVMediaTypeAudio,
        };
        use objc2_foundation::{NSObject, NSURL};

        if sources.screen && sources.microphone {
            return Err(CaptureErrorResponse {
                code: "separate_audio_unsupported".to_string(),
                message:
                    "Capturing screen and microphone as separate files requires macOS 15.0 or newer"
                        .to_string(),
            });
        }

        let session_dir = create_session_dir(session_id)?;

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

            if sources.microphone {
                let media_type_audio =
                    unsafe { AVMediaTypeAudio }.ok_or_else(|| CaptureErrorResponse {
                        code: "microphone_input_unavailable".to_string(),
                        message: "Failed to resolve microphone device".to_string(),
                    })?;

                let mic_device =
                    unsafe { AVCaptureDevice::defaultDeviceWithMediaType(media_type_audio) }
                        .ok_or_else(|| CaptureErrorResponse {
                            code: "microphone_input_unavailable".to_string(),
                            message: "Failed to resolve microphone device".to_string(),
                        })?;

                let mic_input =
                    unsafe { AVCaptureDeviceInput::deviceInputWithDevice_error(&mic_device) }
                        .map_err(|_| CaptureErrorResponse {
                            code: "microphone_input_unavailable".to_string(),
                            message: "Failed to create microphone input".to_string(),
                        })?;

                let mic_input_ref: &AVCaptureInput =
                    unsafe { &*(&*mic_input as *const _ as *const AVCaptureInput) };
                let can_add = unsafe { capture_session.canAddInput(mic_input_ref) };
                if can_add {
                    unsafe { capture_session.addInput(mic_input_ref) };
                }

                if !can_add {
                    return Err(CaptureErrorResponse {
                        code: "microphone_input_unavailable".to_string(),
                        message: "Failed to add microphone input".to_string(),
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
            let (finish_tx, finish_rx) = mpsc::channel::<FinishResult>();
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
                microphone_recording_file: None,
                system_audio_recording_file: None,
                output_files,
            })
        })();

        finalize_startup_result(start_result, &session_dir)
    }

    #[cfg(target_os = "macos")]
    fn start_avfoundation_microphone_only_capture_session(
        session_id: &str,
        sources: &CaptureSources,
    ) -> Result<StartedCaptureSession, CaptureErrorResponse> {
        use cidre::ns;

        let session_dir = create_session_dir(session_id)?;

        let start_result = (|| {
            let output_files = output_files_for_session(&session_dir, sources);
            let mic_recording_file = session_dir.join("microphone.m4a");
            let mic_recording_file_str = mic_recording_file.to_string_lossy().to_string();
            let mic_recording_url = ns::Url::with_fs_path_str(&mic_recording_file_str, false);
            let recording_file = session_dir.join("screen.mov").to_string_lossy().to_string();

            let microphone_session =
                start_avfoundation_microphone_capture_session(&mic_recording_url)?;

            Ok(StartedCaptureSession {
                session: ActiveCaptureSession {
                    backend: CaptureBackendSession::AvFoundationMicrophone(microphone_session),
                },
                recording_file,
                microphone_recording_file: Some(mic_recording_file_str),
                system_audio_recording_file: None,
                output_files,
            })
        })();

        finalize_startup_result(start_result, &session_dir)
    }

    #[cfg(target_os = "macos")]
    fn start_avfoundation_microphone_capture_session(
        output_url: &cidre::ns::Url,
    ) -> Result<AvFoundationMicrophoneCaptureSession, CaptureErrorResponse> {
        use cidre::{av, dispatch};

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
    fn start_screen_capture_kit_session(
        session_id: &str,
        sources: &CaptureSources,
    ) -> Result<StartedCaptureSession, CaptureErrorResponse> {
        use cidre::{api, cm, ns, sc};

        if !api::version!(macos = 15.0) {
            return Err(CaptureErrorResponse {
                code: "screen_capture_kit_unsupported".to_string(),
                message: "ScreenCaptureKit recording requires macOS 15.0 or newer".to_string(),
            });
        }

        let session_dir = create_session_dir(session_id)?;

        let start_result = (|| {
            let output_file = session_dir.join("screen.mov");
            let output_file_str = output_file.to_string_lossy().to_string();
            let output_url = ns::Url::with_fs_path_str(&output_file_str, false);
            let mic_recording_file = session_dir.join("microphone.m4a");
            let mic_recording_file_str = mic_recording_file.to_string_lossy().to_string();
            let mic_recording_url = ns::Url::with_fs_path_str(&mic_recording_file_str, false);
            let system_audio_output_file = session_dir.join("system-audio.m4a");
            let system_audio_output_file_str =
                system_audio_output_file.to_string_lossy().to_string();
            let system_audio_output_url =
                ns::Url::with_fs_path_str(&system_audio_output_file_str, false);

            let output_files = output_files_for_session(&session_dir, sources);

            let (content_tx, content_rx) = mpsc::channel::<
                Result<cidre::arc::R<sc::ShareableContent>, CaptureErrorResponse>,
            >();
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
            let filter =
                sc::ContentFilter::with_display_excluding_windows(display, &excluded_windows);

            let mut screen_stream_cfg = sc::StreamCfg::new();
            screen_stream_cfg.set_width(display.width().max(1) as usize);
            screen_stream_cfg.set_height(display.height().max(1) as usize);
            screen_stream_cfg.set_minimum_frame_interval(cm::Time::new(1, 60));
            screen_stream_cfg.set_shows_cursor(sources.screen);
            screen_stream_cfg.set_captures_audio(sources.system_audio);
            screen_stream_cfg.set_capture_mic(false);
            if sources.system_audio {
                screen_stream_cfg.set_sample_rate(48_000);
                screen_stream_cfg.set_channel_count(2);
            }

            let (stream, recording_output, recording_delegate) =
                start_screen_capture_kit_recording_stream(
                    &filter,
                    &screen_stream_cfg,
                    &output_url,
                )?;

            let (mut stream_output_delegate, stream_output_queue) = if sources.system_audio {
                let system_audio_writer = if sources.system_audio {
                    Some(create_audio_asset_writer(
                        &system_audio_output_url,
                        "system audio",
                    )?)
                } else {
                    None
                };

                let delegate = ScStreamOutputDelegate::with(StreamOutputContext {
                    microphone_writer: None,
                    system_audio_writer,
                    first_error: None,
                });
                let queue = dispatch::Queue::serial_with_ar_pool();

                if sources.system_audio {
                    stream
                        .add_stream_output(delegate.as_ref(), sc::OutputType::Audio, Some(&queue))
                        .map_err(|error| {
                            error_with_ns_error(
                                "capture_stream_output_attach_failed",
                                "Failed to attach ScreenCaptureKit system audio output",
                                error,
                            )
                        })?;
                }

                (Some(delegate), Some(queue))
            } else {
                (None, None)
            };

            start_screen_capture_kit_stream(&stream)?;

            let microphone_session = if sources.microphone {
                match start_avfoundation_microphone_capture_session(&mic_recording_url) {
                    Ok(session) => Some(session),
                    Err(error) => {
                        let rollback_stop_result = ScreenCaptureKitCaptureSession::stop_stream(
                            &stream,
                            "capture_start_rollback_incomplete",
                        );
                        if rollback_stop_result.is_ok() {
                            synchronize_stream_output_queue(stream_output_queue.as_deref());
                            if let Some(delegate) = stream_output_delegate.as_mut() {
                                let _ = finalize_stream_output_context(delegate.inner_mut());
                            }
                        }

                        if let Err(rollback_error) = rollback_stop_result {
                            return Err(combine_startup_error_with_rollback_error(
                                error,
                                rollback_error,
                            ));
                        }

                        return Err(error);
                    }
                }
            } else {
                None
            };

            Ok(StartedCaptureSession {
                session: ActiveCaptureSession {
                    backend: CaptureBackendSession::ScreenCaptureKit(
                        ScreenCaptureKitCaptureSession {
                            stream,
                            _screen_recording_output: recording_output,
                            _screen_delegate: recording_delegate,
                            microphone_session,
                            _stream_output_delegate: stream_output_delegate,
                            _stream_output_queue: stream_output_queue,
                        },
                    ),
                },
                recording_file: output_file_str,
                microphone_recording_file: sources.microphone.then_some(mic_recording_file_str),
                system_audio_recording_file: sources
                    .system_audio
                    .then_some(system_audio_output_file_str),
                output_files,
            })
        })();

        finalize_startup_result(start_result, &session_dir)
    }

    #[cfg(target_os = "macos")]
    fn start_screen_capture_kit_recording_stream(
        filter: &cidre::sc::ContentFilter,
        stream_cfg: &cidre::sc::StreamCfg,
        output_url: &cidre::ns::Url,
    ) -> Result<ScreenCaptureKitRecordingStreamStart, CaptureErrorResponse> {
        let mut stream = cidre::sc::Stream::new(filter, stream_cfg);
        let mut recording_cfg = cidre::sc::RecordingOutputCfg::new();
        recording_cfg.set_output_url(output_url);

        let recording_delegate = ScRecordingOutputDelegate::new();
        let recording_output =
            cidre::sc::RecordingOutput::with_cfg(&recording_cfg, recording_delegate.as_ref());
        stream
            .add_recording_output(&recording_output)
            .map_err(|error| {
                error_with_ns_error(
                    "capture_recording_output_attach_failed",
                    "Failed to attach ScreenCaptureKit recording output",
                    error,
                )
            })?;

        Ok((stream, recording_output, recording_delegate))
    }

    #[cfg(target_os = "macos")]
    fn start_screen_capture_kit_stream(
        stream: &cidre::sc::Stream,
    ) -> Result<(), CaptureErrorResponse> {
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
    fn create_audio_asset_writer(
        output_url: &cidre::ns::Url,
        label: &'static str,
    ) -> Result<AudioAssetWriterState, CaptureErrorResponse> {
        use cidre::{av, cat, ns};

        let format_id = ns::Number::with_u32(cat::audio::Format::MPEG4_AAC.0);
        let sample_rate = ns::Number::with_i64(48_000);
        let channel_count = ns::Number::with_i64(2);

        let output_settings: cidre::arc::R<ns::Dictionary<ns::String, ns::Id>> =
            ns::Dictionary::with_keys_values(
                &[
                    av::audio::all_formats_keys::id(),
                    av::audio::all_formats_keys::sample_rate(),
                    av::audio::all_formats_keys::number_of_channels(),
                ],
                &[
                    format_id.as_id_ref(),
                    sample_rate.as_id_ref(),
                    channel_count.as_id_ref(),
                ],
            );

        let mut writer = av::AssetWriter::with_url_and_file_type(output_url, av::FileType::m4a())
            .map_err(|error| {
            error_with_ns_error(
                "capture_output_unavailable",
                "Failed to create audio asset writer",
                error,
            )
        })?;

        let mut input = av::AssetWriterInput::with_media_type_and_output_settings(
            av::MediaType::audio(),
            Some(output_settings.as_ref()),
        )
        .map_err(|_| CaptureErrorResponse {
            code: "capture_output_unavailable".to_string(),
            message: format!("Failed to create {label} asset writer input"),
        })?;
        input.set_expects_media_data_in_real_time(true);

        if !writer.can_add_input(&input) {
            return Err(CaptureErrorResponse {
                code: "capture_output_unavailable".to_string(),
                message: format!("Failed to add {label} asset writer input"),
            });
        }

        writer.add_input(&input).map_err(|_| CaptureErrorResponse {
            code: "capture_output_unavailable".to_string(),
            message: format!("Failed to attach {label} asset writer input"),
        })?;

        Ok(AudioAssetWriterState {
            writer,
            input,
            started: false,
            appended_samples: 0,
            label,
        })
    }

    #[cfg(target_os = "macos")]
    fn append_audio_sample_to_writer(
        writer_state: &mut AudioAssetWriterState,
        sample_buf: &cidre::cm::SampleBuf,
    ) -> Result<(), CaptureErrorResponse> {
        if !sample_buf.data_is_ready() {
            return Ok(());
        }

        if !writer_state.started {
            if !writer_state.writer.start_writing() {
                return Err(writer_error_response(
                    &writer_state.writer,
                    "capture_output_processing_failed",
                    &format!("Failed to start {} audio asset writer", writer_state.label),
                ));
            }

            writer_state
                .writer
                .start_session_at_src_time(sample_buf.pts());
            writer_state.started = true;
        }

        if !writer_state.input.is_ready_for_more_media_data() {
            return Ok(());
        }

        let appended = writer_state
            .input
            .append_sample_buf(sample_buf)
            .map_err(|_| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: format!(
                    "Failed to append {} audio sample to asset writer",
                    writer_state.label
                ),
            })?;

        if !appended {
            return Err(writer_error_response(
                &writer_state.writer,
                "capture_output_processing_failed",
                &format!(
                    "Failed to append {} audio sample to asset writer",
                    writer_state.label
                ),
            ));
        }

        writer_state.appended_samples += 1;

        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn no_audio_samples_error(label: &str) -> CaptureErrorResponse {
        CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: format!("No {label} audio samples were received; no output file was produced"),
        }
    }

    #[cfg(target_os = "macos")]
    fn writer_error_response(
        writer: &cidre::av::AssetWriter,
        code: &str,
        prefix: &str,
    ) -> CaptureErrorResponse {
        if let Some(error) = writer.error() {
            error_with_ns_error(code, prefix, error.as_ref())
        } else {
            CaptureErrorResponse {
                code: code.to_string(),
                message: prefix.to_string(),
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn finish_audio_asset_writer(
        writer_state: &mut AudioAssetWriterState,
    ) -> Result<(), CaptureErrorResponse> {
        if !writer_state.started || writer_state.appended_samples == 0 {
            return Err(no_audio_samples_error(writer_state.label));
        }

        writer_state.input.mark_as_finished();
        writer_state.writer.finish_writing();

        let wait_deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match writer_state.writer.status() {
                cidre::av::asset::WriterStatus::Completed => return Ok(()),
                cidre::av::asset::WriterStatus::Failed => {
                    return Err(writer_error_response(
                        &writer_state.writer,
                        "capture_output_processing_failed",
                        &format!(
                            "Failed to finalize {} audio asset writer",
                            writer_state.label
                        ),
                    ));
                }
                status if Instant::now() >= wait_deadline => {
                    return Err(CaptureErrorResponse {
                        code: "capture_output_processing_failed".to_string(),
                        message: format!(
                            "Timed out while finalizing {} audio asset writer (status: {:?})",
                            writer_state.label, status
                        ),
                    });
                }
                _ => std::thread::sleep(Duration::from_millis(10)),
            }
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
        let mut failures: Vec<String> = Vec::new();

        if let Some(error) = context.first_error.take() {
            failures.push(format!(
                "stream output failed: [{}] {}",
                error.code, error.message
            ));
        }

        if let Some(writer) = context.microphone_writer.as_mut() {
            if let Err(error) = finish_audio_asset_writer(writer) {
                failures.push(format!("microphone writer failed: {}", error.message));
            }
        }

        if let Some(writer) = context.system_audio_writer.as_mut() {
            if let Err(error) = finish_audio_asset_writer(writer) {
                failures.push(format!("system audio writer failed: {}", error.message));
            }
        }

        aggregate_output_processing_failures(failures)
    }

    #[cfg(target_os = "macos")]
    fn finalize_microphone_output_context(
        context: &mut MicrophoneOutputContext,
    ) -> Result<(), CaptureErrorResponse> {
        let mut failures: Vec<String> = Vec::new();

        if let Some(error) = context.first_error.take() {
            failures.push(format!(
                "microphone stream output failed: [{}] {}",
                error.code, error.message
            ));
        }

        if let Err(error) = finish_audio_asset_writer(&mut context.writer) {
            failures.push(format!("microphone writer failed: {}", error.message));
        }

        aggregate_output_processing_failures(failures)
    }

    #[cfg(target_os = "macos")]
    fn convert_recording_audio_to_m4a(
        recording_file: &str,
        output_file: &str,
    ) -> Result<(), CaptureErrorResponse> {
        let _ = std::fs::remove_file(output_file);

        let conversion = Command::new("/usr/bin/afconvert")
            .arg("-f")
            .arg("m4af")
            .arg("-d")
            .arg("aac")
            .arg("-o")
            .arg(output_file)
            .arg(recording_file)
            .output()
            .map_err(|error| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: format!("Failed to launch audio conversion: {error}"),
            })?;

        if !conversion.status.success() {
            let stderr = String::from_utf8_lossy(&conversion.stderr);
            return Err(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: format!(
                    "Failed to convert recording audio to m4a: {}",
                    stderr.trim()
                ),
            });
        }

        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn load_asset_tracks_with_timeout(
        asset: &cidre::av::UrlAsset,
        media_type: &cidre::av::MediaType,
        timeout_code: &str,
        timeout_message: &str,
    ) -> Result<cidre::arc::R<cidre::ns::Array<cidre::av::asset::Track>>, CaptureErrorResponse>
    {
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
    fn strip_audio_from_recording_file(recording_file: &str) -> Result<(), CaptureErrorResponse> {
        use cidre::{av, ns};

        let input_path = std::path::Path::new(recording_file);
        let temp_path = input_path.with_extension("video-only.mov");
        let _ = std::fs::remove_file(&temp_path);

        let input_url = ns::Url::with_fs_path_str(recording_file, false);
        let temp_url = ns::Url::with_fs_path_str(temp_path.to_string_lossy().as_ref(), false);

        let asset =
            av::UrlAsset::with_url(&input_url, None).ok_or_else(|| CaptureErrorResponse {
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
            return Err(writer_error_response(
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
                    message: "Failed to append video sample during video-only conversion"
                        .to_string(),
                })?;

            if !appended {
                return Err(writer_error_response(
                    &writer,
                    "capture_output_processing_failed",
                    "Failed to append video sample during video-only conversion",
                ));
            }
        }

        writer_input.mark_as_finished();
        writer.finish_writing();

        let wait_deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match writer.status() {
                cidre::av::asset::WriterStatus::Completed => break,
                cidre::av::asset::WriterStatus::Failed => {
                    return Err(writer_error_response(
                        &writer,
                        "capture_output_processing_failed",
                        "Failed to finalize video-only recording",
                    ));
                }
                status if Instant::now() >= wait_deadline => {
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

    #[cfg(target_os = "macos")]
    fn finalize_capture_output_files(
        runtime: &NativeCaptureRuntime,
    ) -> Result<(), CaptureErrorResponse> {
        let Some(output_files) = runtime.output_files.as_ref() else {
            return Ok(());
        };

        let mut failures: Vec<String> = Vec::new();
        let recording_file = runtime.recording_file.as_ref();
        let allow_recording_fallback = runtime
            .requested_sources
            .as_ref()
            .is_some_and(should_fallback_to_primary_recording_for_audio);

        if let Some(microphone_file) = output_files.microphone_file.as_ref() {
            let source_recording = runtime
                .microphone_recording_file
                .as_ref()
                .or_else(|| allow_recording_fallback.then_some(recording_file).flatten());

            if let Some(source_recording) = source_recording {
                if source_recording != microphone_file {
                    if let Err(error) =
                        convert_recording_audio_to_m4a(source_recording, microphone_file)
                    {
                        failures.push(format!(
                            "microphone output conversion failed: {}",
                            error.message
                        ));
                    }
                }
            } else {
                failures.push(
                    "microphone output conversion failed: missing source recording".to_string(),
                );
            }
        }

        if let Some(system_audio_file) = output_files.system_audio_file.as_ref() {
            let source_recording = runtime.system_audio_recording_file.as_ref();

            if let Some(source_recording) = source_recording {
                if source_recording != system_audio_file {
                    if let Err(error) =
                        convert_recording_audio_to_m4a(source_recording, system_audio_file)
                    {
                        failures.push(format!(
                            "system audio output conversion failed: {}",
                            error.message
                        ));
                    }
                }
            } else {
                failures.push(
                    "system audio output conversion failed: missing source recording".to_string(),
                );
            }
        }

        if runtime
            .requested_sources
            .as_ref()
            .is_some_and(should_strip_screen_recording_audio)
        {
            if let Some(recording_file) = recording_file {
                if let Err(error) = strip_audio_from_recording_file(recording_file) {
                    failures.push(format!(
                        "screen output video-only conversion failed: {}",
                        error.message
                    ));
                }
            }
        }

        if let Some(recording_file) = recording_file {
            if runtime
                .requested_sources
                .as_ref()
                .is_some_and(should_remove_intermediate_recording)
            {
                match std::fs::remove_file(recording_file) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        failures.push(format!(
                            "failed to remove intermediate recording file: {error}"
                        ));
                    }
                }
            }
        }

        if let Some(microphone_recording_file) = runtime.microphone_recording_file.as_ref() {
            if output_files.microphone_file.as_deref() != Some(microphone_recording_file) {
                match std::fs::remove_file(microphone_recording_file) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        failures.push(format!(
                            "failed to remove intermediate microphone recording file: {error}"
                        ));
                    }
                }
            }
        }

        if let Some(system_audio_recording_file) = runtime.system_audio_recording_file.as_ref() {
            if output_files.system_audio_file.as_deref() != Some(system_audio_recording_file) {
                match std::fs::remove_file(system_audio_recording_file) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        failures.push(format!(
                            "failed to remove intermediate system audio recording file: {error}"
                        ));
                    }
                }
            }
        }

        aggregate_output_processing_failures(failures)
    }

    #[cfg(target_os = "macos")]
    fn aggregate_output_processing_failures(
        failures: Vec<String>,
    ) -> Result<(), CaptureErrorResponse> {
        if failures.is_empty() {
            return Ok(());
        }

        Err(CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: format!(
                "Failed to finalize capture outputs: {}",
                failures.join("; ")
            ),
        })
    }

    #[cfg(target_os = "macos")]
    pub fn stop_capture_session(
        runtime: &mut NativeCaptureRuntime,
    ) -> Result<(), CaptureErrorResponse> {
        let mut stop_error: Option<CaptureErrorResponse> = None;

        if let Some(session) = runtime.active_session.as_mut() {
            match session.stop() {
                Ok(()) => {
                    runtime.active_session = None;
                }
                Err(error)
                    if ScreenCaptureKitCaptureSession::is_stop_timeout_code(
                        error.code.as_str(),
                    ) =>
                {
                    return Err(error);
                }
                Err(error) => {
                    stop_error = Some(error);
                    runtime.active_session = None;
                }
            }
        }

        let finalize_result = finalize_capture_output_files(runtime);

        match (stop_error, finalize_result) {
            (Some(stop_error), Err(finalize_error)) => Err(CaptureErrorResponse {
                code: stop_error.code,
                message: format!(
                    "{}; additionally failed to finalize capture outputs: [{}] {}",
                    stop_error.message, finalize_error.code, finalize_error.message
                ),
            }),
            (Some(stop_error), Ok(())) => Err(stop_error),
            (None, Err(finalize_error)) => Err(finalize_error),
            (None, Ok(())) => Ok(()),
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
        // SAFETY: CoreGraphics function is pure and available on supported macOS runtime.
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
        // SAFETY: CoreGraphics function prompts permission and returns current grant status.
        unsafe { CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess() }
    }

    #[cfg(target_os = "macos")]
    pub fn microphone_permission_state() -> CapturePermissionState {
        use cidre::av;

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
        use cidre::av;

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
    pub fn screen_permission_state() -> CapturePermissionState {
        CapturePermissionState::Unsupported
    }

    #[cfg(not(target_os = "macos"))]
    pub fn ensure_screen_permission() -> bool {
        false
    }

    #[cfg(not(target_os = "macos"))]
    pub fn microphone_permission_state() -> CapturePermissionState {
        CapturePermissionState::Unsupported
    }

    #[cfg(not(target_os = "macos"))]
    pub fn ensure_microphone_permission() -> bool {
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

    #[cfg(not(target_os = "macos"))]
    pub fn stop_capture_session(
        _runtime: &mut NativeCaptureRuntime,
    ) -> Result<(), CaptureErrorResponse> {
        Ok(())
    }

    #[cfg(all(test, target_os = "macos"))]
    mod tests {
        use super::*;
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };

        #[test]
        fn new_session_id_has_expected_prefix() {
            let session_id = new_session_id().expect("session id should be generated");
            assert!(session_id.starts_with("native-session-"));
            assert!(session_id.len() > "native-session-".len());
        }

        #[test]
        fn create_session_dir_creates_unique_dir() {
            let session_id = new_session_id().expect("session id should be generated");
            let path = create_session_dir(&session_id).expect("session dir should be created");
            assert!(path.exists());
            assert!(path.is_dir());

            std::fs::remove_dir_all(path).expect("cleanup should succeed");
        }

        #[test]
        fn remove_session_dir_removes_existing_directory() {
            let session_id = new_session_id().expect("session id should be generated");
            let path = create_session_dir(&session_id).expect("session dir should be created");

            remove_session_dir(path.as_path()).expect("session dir removal should succeed");

            assert!(!path.exists());
        }

        #[test]
        fn remove_session_dir_ignores_missing_directory() {
            let session_id = new_session_id().expect("session id should be generated");
            let path = capture_root()
                .expect("capture root should exist")
                .join(session_id);

            remove_session_dir(path.as_path()).expect("missing dir removal should be ignored");
        }

        #[test]
        fn finalize_startup_result_removes_session_dir_on_error() {
            let session_id = new_session_id().expect("session id should be generated");
            let path = create_session_dir(&session_id).expect("session dir should be created");

            let err = finalize_startup_result::<()>(
                Err(CaptureErrorResponse {
                    code: "capture_output_unavailable".to_string(),
                    message: "Failed to add movie output".to_string(),
                }),
                path.as_path(),
            )
            .expect_err("startup failure should be returned");

            assert_eq!(err.code, "capture_output_unavailable");
            assert!(!path.exists());
        }

        #[test]
        fn finalize_startup_result_preserves_session_dir_on_rollback_timeout() {
            let session_id = new_session_id().expect("session id should be generated");
            let path = create_session_dir(&session_id).expect("session dir should be created");

            let err = finalize_startup_result::<()>(
                Err(CaptureErrorResponse {
                    code: "capture_start_rollback_incomplete".to_string(),
                    message: "Timed out waiting for ScreenCaptureKit stream stop".to_string(),
                }),
                path.as_path(),
            )
            .expect_err("rollback timeout should be returned");

            assert_eq!(err.code, "capture_start_rollback_incomplete");
            assert!(path.exists());

            std::fs::remove_dir_all(path).expect("cleanup should succeed");
        }

        #[test]
        fn combine_startup_error_with_rollback_error_preserves_rollback_timeout_code() {
            let combined = combine_startup_error_with_rollback_error(
                CaptureErrorResponse {
                    code: "capture_microphone_start_failed".to_string(),
                    message: "Failed to start microphone capture".to_string(),
                },
                CaptureErrorResponse {
                    code: "capture_start_rollback_incomplete".to_string(),
                    message: "Timed out waiting for ScreenCaptureKit stream stop".to_string(),
                },
            );

            assert_eq!(combined.code, "capture_start_rollback_incomplete");
            assert_eq!(
                combined.message,
                "Timed out waiting for ScreenCaptureKit stream stop"
            );
        }

        #[test]
        fn combine_startup_error_with_rollback_error_keeps_startup_code_when_not_timeout() {
            let combined = combine_startup_error_with_rollback_error(
                CaptureErrorResponse {
                    code: "capture_microphone_start_failed".to_string(),
                    message: "Failed to start microphone capture".to_string(),
                },
                CaptureErrorResponse {
                    code: "capture_stop_failed".to_string(),
                    message: "Failed to stop ScreenCaptureKit stream".to_string(),
                },
            );

            assert_eq!(combined.code, "capture_microphone_start_failed");
            assert!(combined
                .message
                .contains("Failed to start microphone capture"));
            assert!(combined.message.contains("[capture_stop_failed]"));
        }

        #[test]
        fn aggregate_output_processing_failures_is_ok_when_empty() {
            let result = aggregate_output_processing_failures(Vec::new());
            assert!(result.is_ok());
        }

        #[test]
        fn aggregate_output_processing_failures_returns_all_failures() {
            let error = aggregate_output_processing_failures(vec![
                "microphone output conversion failed".to_string(),
                "failed to remove intermediate microphone recording file".to_string(),
            ])
            .expect_err("expected aggregated output processing failure");

            assert_eq!(error.code, "capture_output_processing_failed");
            assert!(error
                .message
                .contains("microphone output conversion failed"));
            assert!(error
                .message
                .contains("failed to remove intermediate microphone recording file"));
        }

        #[test]
        fn no_audio_samples_error_mentions_label() {
            let error = no_audio_samples_error("microphone");
            assert_eq!(error.code, "capture_output_processing_failed");
            assert!(error.message.contains("microphone"));
            assert!(error.message.contains("No"));
        }

        #[test]
        fn synchronize_stream_output_queue_waits_for_pending_callbacks() {
            let queue = dispatch::Queue::serial_with_ar_pool();
            let callback_ran = Arc::new(AtomicBool::new(false));
            let callback_ran_clone = Arc::clone(&callback_ran);

            queue.async_once(move || {
                callback_ran_clone.store(true, Ordering::SeqCst);
            });

            synchronize_stream_output_queue(Some(queue.as_ref()));

            assert!(callback_ran.load(Ordering::SeqCst));
        }

        #[test]
        fn screen_capture_kit_stop_timeout_codes_are_detected() {
            assert!(ScreenCaptureKitCaptureSession::is_stop_timeout_code(
                "capture_stop_incomplete"
            ));
            assert!(ScreenCaptureKitCaptureSession::is_stop_timeout_code(
                "capture_start_rollback_incomplete"
            ));
            assert!(!ScreenCaptureKitCaptureSession::is_stop_timeout_code(
                "capture_stop_failed"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn validate_start_request_rejects_system_audio_when_not_supported() {
        let request = StartNativeCaptureRequest {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: true,
        };
        let support = CaptureSupportResponse {
            platform: "macos".to_string(),
            native_capture_supported: true,
            supported_sources: CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            },
        };

        let error =
            validate_start_request(&request, &support).expect_err("must reject system audio");
        assert_eq!(error.code, "system_audio_unsupported");
    }

    #[test]
    fn validate_start_request_accepts_system_audio_when_supported() {
        let request = StartNativeCaptureRequest {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: true,
        };
        let support = CaptureSupportResponse {
            platform: "macos".to_string(),
            native_capture_supported: true,
            supported_sources: CaptureSources {
                screen: true,
                microphone: true,
                system_audio: true,
            },
        };

        let sources =
            validate_start_request(&request, &support).expect("must allow supported system audio");
        assert!(sources.system_audio);
    }

    #[test]
    fn validate_start_request_rejects_system_audio_only() {
        let request = StartNativeCaptureRequest {
            capture_screen: false,
            capture_microphone: false,
            capture_system_audio: true,
        };
        let support = CaptureSupportResponse {
            platform: "macos".to_string(),
            native_capture_supported: true,
            supported_sources: CaptureSources {
                screen: true,
                microphone: true,
                system_audio: true,
            },
        };

        let error = validate_start_request(&request, &support)
            .expect_err("must reject system-audio-only requests");
        assert_eq!(error.code, "system_audio_requires_screen");
    }

    #[test]
    fn validate_start_request_accepts_mic_and_system_audio_together() {
        let request = StartNativeCaptureRequest {
            capture_screen: true,
            capture_microphone: true,
            capture_system_audio: true,
        };
        let support = CaptureSupportResponse {
            platform: "macos".to_string(),
            native_capture_supported: true,
            supported_sources: CaptureSources {
                screen: true,
                microphone: true,
                system_audio: true,
            },
        };

        let sources = validate_start_request(&request, &support)
            .expect("must allow mixed microphone/system-audio capture");
        assert!(sources.microphone);
        assert!(sources.system_audio);
    }

    #[test]
    fn sample_buffer_microphone_only_path_is_selected_only_for_mic_only_capture() {
        assert!(should_use_sample_buffer_microphone_only_path(
            &CaptureSources {
                screen: false,
                microphone: true,
                system_audio: false,
            }
        ));

        assert!(!should_use_sample_buffer_microphone_only_path(
            &CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            }
        ));
        assert!(!should_use_sample_buffer_microphone_only_path(
            &CaptureSources {
                screen: false,
                microphone: true,
                system_audio: true,
            }
        ));
        assert!(!should_use_sample_buffer_microphone_only_path(
            &CaptureSources {
                screen: false,
                microphone: false,
                system_audio: false,
            }
        ));
    }

    #[test]
    fn mark_runtime_session_stopped_preserves_session_metadata() {
        let mut runtime = NativeCaptureRuntime {
            is_running: true,
            session_id: Some("session-1".to_string()),
            started_at_unix_ms: Some(123),
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            }),
            output_files: Some(CaptureOutputFiles {
                screen_file: Some("/tmp/screen.mov".to_string()),
                microphone_file: Some("/tmp/microphone.m4a".to_string()),
                system_audio_file: None,
            }),
            #[cfg(target_os = "macos")]
            recording_file: Some("/tmp/screen.mov".to_string()),
            #[cfg(target_os = "macos")]
            microphone_recording_file: Some("/tmp/microphone.mov".to_string()),
            #[cfg(target_os = "macos")]
            system_audio_recording_file: None,
            #[cfg(target_os = "macos")]
            active_session: None,
        };

        mark_runtime_session_stopped(&mut runtime);

        assert!(!runtime.is_running);
        assert_eq!(runtime.session_id, Some("session-1".to_string()));
        assert_eq!(runtime.started_at_unix_ms, Some(123));
        assert!(runtime.requested_sources.is_some());
        assert!(runtime.output_files.is_some());
        #[cfg(target_os = "macos")]
        {
            assert!(runtime.recording_file.is_some());
            assert!(runtime.microphone_recording_file.is_some());
            assert!(runtime.active_session.is_none());
        }
    }

    #[test]
    fn stopped_session_from_runtime_preserves_finalized_metadata() {
        let runtime = NativeCaptureRuntime {
            is_running: true,
            session_id: Some("session-1".to_string()),
            started_at_unix_ms: Some(123),
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: true,
                system_audio: true,
            }),
            output_files: Some(CaptureOutputFiles {
                screen_file: Some("/tmp/screen.mov".to_string()),
                microphone_file: Some("/tmp/microphone.m4a".to_string()),
                system_audio_file: Some("/tmp/system-audio.m4a".to_string()),
            }),
            #[cfg(target_os = "macos")]
            recording_file: None,
            #[cfg(target_os = "macos")]
            microphone_recording_file: None,
            #[cfg(target_os = "macos")]
            system_audio_recording_file: None,
            #[cfg(target_os = "macos")]
            active_session: None,
        };

        let session = stopped_session_from_runtime(&runtime);

        assert!(!session.is_running);
        assert_eq!(session.session_id, Some("session-1".to_string()));
        assert_eq!(session.started_at_unix_ms, Some(123));
        assert!(session.requested_sources.as_ref().is_some_and(|sources| {
            sources.screen && sources.microphone && sources.system_audio
        }));
        assert_eq!(
            session
                .output_files
                .as_ref()
                .and_then(|files| files.screen_file.as_deref()),
            Some("/tmp/screen.mov")
        );
        assert_eq!(
            session
                .output_files
                .as_ref()
                .and_then(|files| files.microphone_file.as_deref()),
            Some("/tmp/microphone.m4a")
        );
        assert_eq!(
            session
                .output_files
                .as_ref()
                .and_then(|files| files.system_audio_file.as_deref()),
            Some("/tmp/system-audio.m4a")
        );
    }

    #[test]
    fn output_files_for_session_uses_expected_filenames() {
        let sources = CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        };

        let output_files =
            output_files_for_session(Path::new("/tmp/native-capture-session"), &sources);

        assert_eq!(
            output_files.screen_file,
            Some("/tmp/native-capture-session/screen.mov".to_string())
        );
        assert_eq!(
            output_files.microphone_file,
            Some("/tmp/native-capture-session/microphone.m4a".to_string())
        );
        assert_eq!(
            output_files.system_audio_file,
            Some("/tmp/native-capture-session/system-audio.m4a".to_string())
        );
    }

    #[test]
    fn should_remove_intermediate_recording_only_for_microphone_only_capture() {
        assert!(should_remove_intermediate_recording(&CaptureSources {
            screen: false,
            microphone: true,
            system_audio: false,
        }));

        assert!(!should_remove_intermediate_recording(&CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }));

        assert!(!should_remove_intermediate_recording(&CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        }));
    }

    #[test]
    fn should_fallback_to_primary_recording_only_for_microphone_only_capture() {
        assert!(should_fallback_to_primary_recording_for_audio(
            &CaptureSources {
                screen: false,
                microphone: true,
                system_audio: false,
            }
        ));

        assert!(!should_fallback_to_primary_recording_for_audio(
            &CaptureSources {
                screen: true,
                microphone: true,
                system_audio: false,
            }
        ));

        assert!(!should_fallback_to_primary_recording_for_audio(
            &CaptureSources {
                screen: true,
                microphone: true,
                system_audio: true,
            }
        ));
    }

    #[test]
    fn should_strip_screen_recording_audio_only_when_screen_and_system_audio_requested() {
        assert!(should_strip_screen_recording_audio(&CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        }));

        assert!(!should_strip_screen_recording_audio(&CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }));

        assert!(!should_strip_screen_recording_audio(&CaptureSources {
            screen: false,
            microphone: false,
            system_audio: true,
        }));
    }

    #[test]
    fn preserve_runtime_on_stop_timeout_matches_platform_behavior() {
        let timeout_error = CaptureErrorResponse {
            code: "capture_stop_incomplete".to_string(),
            message: "Timed out waiting for ScreenCaptureKit stream stop".to_string(),
        };

        #[cfg(target_os = "macos")]
        assert!(platform::should_preserve_runtime_on_stop_error(
            &timeout_error
        ));

        #[cfg(not(target_os = "macos"))]
        assert!(!platform::should_preserve_runtime_on_stop_error(
            &timeout_error
        ));

        let non_timeout_error = CaptureErrorResponse {
            code: "capture_stop_failed".to_string(),
            message: "Failed to stop ScreenCaptureKit stream".to_string(),
        };

        assert!(!platform::should_preserve_runtime_on_stop_error(
            &non_timeout_error
        ));
    }
}
