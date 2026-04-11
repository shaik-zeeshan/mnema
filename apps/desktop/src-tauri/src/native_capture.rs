use capture_microphone as microphone_capture;
use capture_screen::StopScreenCaptureSessionArgs;
use capture_types::{
    CaptureErrorResponse, CaptureOutputFiles, CapturePermissionState, CapturePermissions,
    CapturePermissionsResponse, CaptureSources, CaptureSupportResponse, NativeCaptureSession,
    NativeCaptureSessionResponse, StartNativeCaptureRequest,
};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
fn maybe_remove_intermediate_file(file: &str, label: &str, failures: &mut Vec<String>) {
    match std::fs::remove_file(file) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            failures.push(format!(
                "failed to remove intermediate {label} recording file: {error}"
            ));
        }
    }
}

#[cfg(target_os = "macos")]
fn finalize_capture_outputs(
    output_files: Option<&CaptureOutputFiles>,
    recording_file: Option<&str>,
    microphone_recording_file: Option<&str>,
    system_audio_recording_file: Option<&str>,
    requested_sources: Option<&CaptureSources>,
) -> Result<(), CaptureErrorResponse> {
    let Some(output_files) = output_files else {
        return Ok(());
    };

    let mut failures: Vec<String> = Vec::new();

    if let Some(microphone_file) = output_files.microphone_file.as_deref() {
        let source_recording = microphone_recording_file.or(recording_file);

        if let Some(source_recording) = source_recording {
            if source_recording != microphone_file {
                if let Err(error) = capture_writers::convert_recording_audio_to_m4a(
                    source_recording,
                    microphone_file,
                ) {
                    failures.push(format!(
                        "microphone output conversion failed: {}",
                        error.message
                    ));
                }
            }
        } else {
            failures
                .push("microphone output conversion failed: missing source recording".to_string());
        }
    }

    if let Some(system_audio_file) = output_files.system_audio_file.as_deref() {
        if let Some(source_recording) = system_audio_recording_file {
            if source_recording != system_audio_file {
                if let Err(error) = capture_writers::convert_recording_audio_to_m4a(
                    source_recording,
                    system_audio_file,
                ) {
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

    if requested_sources.is_some_and(|sources| sources.system_audio) {
        if let Some(recording_file) = recording_file {
            if let Err(error) = capture_screen::strip_audio_from_recording_file(recording_file) {
                failures.push(format!(
                    "screen output video-only conversion failed: {}",
                    error.message
                ));
            }
        }
    }

    if let Some(microphone_recording_file) = microphone_recording_file {
        if output_files.microphone_file.as_deref() != Some(microphone_recording_file) {
            maybe_remove_intermediate_file(microphone_recording_file, "microphone", &mut failures);
        }
    }

    if let Some(system_audio_recording_file) = system_audio_recording_file {
        if output_files.system_audio_file.as_deref() != Some(system_audio_recording_file) {
            maybe_remove_intermediate_file(
                system_audio_recording_file,
                "system audio",
                &mut failures,
            );
        }
    }

    capture_writers::aggregate_output_processing_failures(failures)
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
    pub active_screen_session: Option<capture_screen::ActiveCaptureSession>,
    #[cfg(target_os = "macos")]
    pub active_microphone_session: Option<microphone_capture::AvFoundationMicrophoneCaptureSession>,
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
    let screen_support = capture_screen::support_for_current_platform();
    let microphone_supported = !matches!(
        microphone_capture::microphone_permission_state(),
        CapturePermissionState::Unsupported
    );

    CaptureSupportResponse {
        platform: screen_support.platform,
        native_capture_supported: screen_support.native_capture_supported,
        supported_sources: CaptureSources {
            screen: screen_support.screen,
            microphone: microphone_supported,
            system_audio: screen_support.system_audio,
        },
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
        runtime.active_screen_session = None;
        runtime.active_microphone_session = None;
    }
}

#[tauri::command]
pub fn get_capture_permissions(
    state: tauri::State<'_, NativeCaptureState>,
) -> CapturePermissionsResponse {
    let runtime = state.lock().expect("native capture state poisoned");
    CapturePermissionsResponse {
        permissions: CapturePermissions {
            screen: capture_screen::screen_permission_state(),
            microphone: microphone_capture::microphone_permission_state(),
            system_audio: capture_screen::system_audio_permission_state(),
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
        let screen_ok = capture_screen::ensure_screen_permission();
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
        let microphone_ok = microphone_capture::ensure_microphone_permission();
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
        let session_id = capture_screen::new_session_id()?;
        let mut output_files = CaptureOutputFiles {
            screen_file: None,
            microphone_file: None,
            system_audio_file: None,
        };

        let mut recording_file: Option<String> = None;
        let mut microphone_recording_file: Option<String> = None;
        let mut system_audio_recording_file: Option<String> = None;
        let mut active_screen_session: Option<capture_screen::ActiveCaptureSession> = None;
        let mut active_microphone_session: Option<
            microphone_capture::AvFoundationMicrophoneCaptureSession,
        > = None;

        if sources.screen || sources.system_audio {
            let screen_sources = capture_screen::ScreenCaptureSources {
                screen: sources.screen,
                system_audio: sources.system_audio,
            };
            let screen_capture =
                capture_screen::start_capture_session(&session_id, &screen_sources)?;
            output_files.screen_file = screen_capture.output_files.screen_file;
            output_files.system_audio_file = screen_capture.output_files.system_audio_file;
            recording_file = Some(screen_capture.recording_file);
            system_audio_recording_file = screen_capture.system_audio_recording_file;
            active_screen_session = Some(screen_capture.session);
        }

        if sources.microphone {
            let microphone_output_file =
                if let Some(existing_screen_file) = output_files.screen_file.as_deref() {
                    std::path::Path::new(existing_screen_file)
                        .parent()
                        .expect("screen output path should have parent")
                        .join("microphone.m4a")
                        .to_string_lossy()
                        .to_string()
                } else {
                    let session_dir = std::env::temp_dir()
                        .join("z-native-capture")
                        .join(&session_id);
                    std::fs::create_dir_all(&session_dir).map_err(|e| CaptureErrorResponse {
                        code: "io_error".to_string(),
                        message: format!("Failed to create capture session directory: {e}"),
                    })?;
                    session_dir
                        .join("microphone.m4a")
                        .to_string_lossy()
                        .to_string()
                };

            let mic_start =
                microphone_capture::start_avfoundation_microphone_capture_session_for_file(
                    &microphone_output_file,
                );

            match mic_start {
                Ok(session) => {
                    output_files.microphone_file = Some(microphone_output_file.clone());
                    microphone_recording_file = Some(microphone_output_file);
                    active_microphone_session = Some(session);
                }
                Err(error) => {
                    if let Err(rollback_error) =
                        capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                            active_session: &mut active_screen_session,
                        })
                    {
                        return Err(CaptureErrorResponse {
                            code: error.code,
                            message: format!(
                                "{}; additionally failed to rollback screen capture session: [{}] {}",
                                error.message, rollback_error.code, rollback_error.message
                            ),
                        });
                    }

                    return Err(error);
                }
            }
        }

        runtime.is_running = true;
        runtime.started_at_unix_ms = Some(started);
        runtime.session_id = Some(session_id);
        runtime.requested_sources = Some(sources);
        runtime.output_files = Some(output_files);
        runtime.recording_file = recording_file;
        runtime.microphone_recording_file = microphone_recording_file;
        runtime.system_audio_recording_file = system_audio_recording_file;
        runtime.active_screen_session = active_screen_session;
        runtime.active_microphone_session = active_microphone_session;
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
            let output_files = runtime.output_files.clone();
            let recording_file = runtime.recording_file.clone();
            let system_audio_recording_file = runtime.system_audio_recording_file.clone();
            let requested_sources = runtime.requested_sources.clone();

            let mut first_error: Option<CaptureErrorResponse> = None;

            if let Some(session) = runtime.active_microphone_session.as_mut() {
                if let Err(error) = session.stop() {
                    first_error = Some(error);
                }
                runtime.active_microphone_session = None;
            }

            if let Err(error) =
                capture_screen::stop_screen_capture_session(StopScreenCaptureSessionArgs {
                    active_session: &mut runtime.active_screen_session,
                })
            {
                if capture_screen::should_preserve_runtime_on_stop_error(&error) {
                    return Err(error);
                }

                if first_error.is_none() {
                    first_error = Some(error);
                }
            }

            if let Err(error) = finalize_capture_outputs(
                output_files.as_ref(),
                recording_file.as_deref(),
                runtime.microphone_recording_file.as_deref(),
                system_audio_recording_file.as_deref(),
                requested_sources.as_ref(),
            ) {
                if let Some(previous_error) = first_error.take() {
                    first_error = Some(CaptureErrorResponse {
                        code: previous_error.code,
                        message: format!(
                            "{}; additionally failed to finalize capture outputs: [{}] {}",
                            previous_error.message, error.code, error.message
                        ),
                    });
                } else {
                    first_error = Some(error);
                }
            }

            if let Some(error) = first_error {
                Err(error)
            } else {
                Ok(())
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok(())
        }
    };

    if let Err(error) = stop_result {
        if capture_screen::should_preserve_runtime_on_stop_error(&error) {
            return Err(error);
        }

        mark_runtime_session_stopped(&mut runtime);
        return Err(error);
    }

    mark_runtime_session_stopped(&mut runtime);
    let session = stopped_session_from_runtime(&runtime);

    Ok(NativeCaptureSessionResponse { session })
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
            active_screen_session: None,
            #[cfg(target_os = "macos")]
            active_microphone_session: None,
        };

        mark_runtime_session_stopped(&mut runtime);

        assert!(!runtime.is_running);
        assert_eq!(runtime.session_id, Some("session-1".to_string()));
        assert_eq!(runtime.started_at_unix_ms, Some(123));
        assert!(runtime.requested_sources.is_some());
        assert!(runtime.output_files.is_some());
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
            active_screen_session: None,
            #[cfg(target_os = "macos")]
            active_microphone_session: None,
        };

        let session = stopped_session_from_runtime(&runtime);

        assert!(!session.is_running);
        assert_eq!(session.session_id, Some("session-1".to_string()));
        assert_eq!(session.started_at_unix_ms, Some(123));
        assert!(session.requested_sources.as_ref().is_some_and(|sources| {
            sources.screen && sources.microphone && sources.system_audio
        }));
    }
}
