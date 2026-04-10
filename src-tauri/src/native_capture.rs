use serde::{Deserialize, Serialize};
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
    pub combined_capture_file: Option<String>,
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
    pub active_session: Option<platform::ActiveCaptureSession>,
}

pub type NativeCaptureState = Mutex<NativeCaptureRuntime>;

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
            combined_capture_file: Some(capture.output_file),
        });
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

    #[cfg(target_os = "macos")]
    {
        platform::stop_capture_session(&mut runtime)?;
    }

    runtime.is_running = false;
    runtime.session_id = None;
    runtime.started_at_unix_ms = None;
    runtime.requested_sources = None;

    Ok(NativeCaptureSessionResponse {
        session: session_from_runtime(&runtime),
    })
}

mod platform {
    use super::{
        CaptureErrorResponse, CapturePermissionState, CaptureSources, NativeCaptureRuntime,
    };
    #[cfg(target_os = "macos")]
    use cidre::objc;
    #[cfg(target_os = "macos")]
    use std::collections::HashMap;
    #[cfg(target_os = "macos")]
    use std::ffi::CString;
    #[cfg(target_os = "macos")]
    use std::fmt::Display;
    use std::path::PathBuf;
    #[cfg(target_os = "macos")]
    use std::sync::atomic::{AtomicBool, Ordering};
    #[cfg(target_os = "macos")]
    use std::sync::mpsc;
    #[cfg(target_os = "macos")]
    use std::sync::{Mutex, OnceLock};
    #[cfg(target_os = "macos")]
    use std::time::Duration;

    #[cfg(target_os = "macos")]
    static SCREEN_PERMISSION_REQUESTED: AtomicBool = AtomicBool::new(false);

    #[cfg(target_os = "macos")]
    pub struct StartedCaptureSession {
        pub session: ActiveCaptureSession,
        pub output_file: String,
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
    struct ScreenCaptureKitCaptureSession {
        stream: cidre::arc::R<cidre::sc::Stream>,
        _recording_output: cidre::arc::R<cidre::sc::RecordingOutput>,
        _delegate: cidre::arc::R<ScRecordingOutputDelegate>,
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
        fn stop(&mut self) -> Result<(), CaptureErrorResponse> {
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

            self.stream.stop_with_ch_block(Some(&mut completion));

            match rx.recv_timeout(Duration::from_secs(20)) {
                Ok(result) => result,
                Err(_) => Err(CaptureErrorResponse {
                    code: "capture_stop_incomplete".to_string(),
                    message: "Timed out waiting for ScreenCaptureKit stream stop".to_string(),
                }),
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

        let session_dir = create_session_dir(session_id)?;

        let output_file = session_dir.join("capture.mov");
        let output_file_str = output_file.to_string_lossy().to_string();

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
                unsafe { AVCaptureDeviceInput::deviceInputWithDevice_error(&mic_device) }.map_err(
                    |_| CaptureErrorResponse {
                        code: "microphone_input_unavailable".to_string(),
                        message: "Failed to create microphone input".to_string(),
                    },
                )?;

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
            output_file: output_file_str,
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

        let output_file = session_dir.join("capture.mov");
        let output_file_str = output_file.to_string_lossy().to_string();
        let output_url = ns::Url::with_fs_path_str(&output_file_str, false);

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

        let mut stream_cfg = sc::StreamCfg::new();
        stream_cfg.set_width(display.width().max(1) as usize);
        stream_cfg.set_height(display.height().max(1) as usize);
        stream_cfg.set_minimum_frame_interval(cm::Time::new(1, 60));
        stream_cfg.set_shows_cursor(sources.screen);
        stream_cfg.set_captures_audio(sources.system_audio);
        stream_cfg.set_excludes_current_process_audio(!sources.system_audio);
        if sources.system_audio || sources.microphone {
            stream_cfg.set_sample_rate(48_000);
            stream_cfg.set_channel_count(2);
        }
        if sources.microphone {
            stream_cfg.set_capture_mic(true);
        }

        let mut stream = sc::Stream::new(&filter, &stream_cfg);
        let mut recording_cfg = sc::RecordingOutputCfg::new();
        recording_cfg.set_output_url(&output_url);

        let recording_delegate = ScRecordingOutputDelegate::new();
        let recording_output =
            sc::RecordingOutput::with_cfg(&recording_cfg, recording_delegate.as_ref());
        stream
            .add_recording_output(&recording_output)
            .map_err(|error| {
                error_with_ns_error(
                    "capture_recording_output_attach_failed",
                    "Failed to attach ScreenCaptureKit recording output",
                    error,
                )
            })?;

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
            Ok(result) => result?,
            Err(_) => {
                return Err(CaptureErrorResponse {
                    code: "capture_stream_start_timeout".to_string(),
                    message: "Timed out while starting ScreenCaptureKit stream capture".to_string(),
                });
            }
        }

        Ok(StartedCaptureSession {
            session: ActiveCaptureSession {
                backend: CaptureBackendSession::ScreenCaptureKit(ScreenCaptureKitCaptureSession {
                    stream,
                    _recording_output: recording_output,
                    _delegate: recording_delegate,
                }),
            },
            output_file: output_file_str,
        })
    }

    #[cfg(target_os = "macos")]
    pub fn stop_capture_session(
        runtime: &mut NativeCaptureRuntime,
    ) -> Result<(), CaptureErrorResponse> {
        if let Some(session) = runtime.active_session.as_mut() {
            session.stop()?;
            runtime.active_session = None;
        }

        Ok(())
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
    pub fn stop_capture_session(
        _runtime: &mut NativeCaptureRuntime,
    ) -> Result<(), CaptureErrorResponse> {
        Ok(())
    }

    #[cfg(all(test, target_os = "macos"))]
    mod tests {
        use super::*;

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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
