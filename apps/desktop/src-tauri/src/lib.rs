mod app_infra;
mod audio_transcription_models;
mod general_app_log;
mod keyboard_bindings;
mod managed_storage_layout;
mod native_capture;
mod ocr_budget;
mod ocr_models;
mod one_time_prompts;
mod privacy_redaction_sources;
mod sensitive_capture_recommendations;
mod speaker_analysis_models;
mod speaker_analysis_runtime;
mod status_bar;
mod windows;

use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_log::{Target, TargetKind, WEBVIEW_TARGET};

pub(crate) const APP_LOG_FILE_NAME: &str = "rust";

const APP_LOG_TARGET_PREFIXES: &[&str] = &[
    "mnema",
    "mnema_lib",
    "app_infra",
    "audio_transcription",
    "ocr",
    "capture_runtime",
    "capture_screen",
    "capture_microphone",
    "capture_writers",
    "capture_types",
    WEBVIEW_TARGET,
];
const ALREADY_RUNNING_MESSAGE: &str =
    "Mnema is already running. Close the existing Mnema window before opening it again.";
const BROKER_OPEN_CAPTURE_RESULT_EVENT: &str = "broker_open_capture_result";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BrokerOpenCaptureResultPayload {
    opaque_id: String,
    kind: String,
    frame_id: Option<i64>,
    audio_segment_id: Option<i64>,
}

fn is_app_log_target(target: &str) -> bool {
    APP_LOG_TARGET_PREFIXES.iter().any(|prefix| {
        target == *prefix
            || target
                .strip_prefix(*prefix)
                .is_some_and(|suffix| suffix.starts_with("::"))
    })
}

fn should_forward_window_event(event: &tauri::WindowEvent, webview_window_found: bool) -> bool {
    matches!(event, tauri::WindowEvent::Destroyed) || webview_window_found
}

fn broker_payload_from_url(url: &url::Url) -> Option<BrokerOpenCaptureResultPayload> {
    if url.scheme() != "mnema" {
        return None;
    }
    let mut segments = url.path_segments()?.collect::<Vec<_>>();
    if let Some(host) = url.host_str() {
        segments.insert(0, host);
    }
    let opaque_id = match segments.as_slice() {
        ["open", opaque_id] | ["broker", "open", opaque_id] => (*opaque_id).to_string(),
        _ => return None,
    };
    let (kind, id) = app_infra::decode_broker_opaque_id(&opaque_id)?;
    Some(BrokerOpenCaptureResultPayload {
        opaque_id,
        frame_id: (kind == "frame").then_some(id),
        audio_segment_id: (kind == "audio").then_some(id),
        kind,
    })
}

fn emit_broker_open_result(app_handle: &tauri::AppHandle, url: &url::Url) {
    let Some(payload) = broker_payload_from_url(url) else {
        return;
    };
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let _ = windows::open_main_window(&app_handle);
        let _ = app_handle.emit(BROKER_OPEN_CAPTURE_RESULT_EVENT, payload);
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitRequestAction {
    StartGracefulExit,
    PreventExit,
    AllowExit,
}

fn exit_request_action_for_exit_request(
    code: Option<i32>,
    graceful_exit_in_progress: bool,
    final_graceful_exit_ready: bool,
) -> ExitRequestAction {
    if final_graceful_exit_ready && code == Some(0) {
        return ExitRequestAction::AllowExit;
    }

    if graceful_exit_in_progress {
        if code.is_none() || code == Some(0) {
            return ExitRequestAction::PreventExit;
        }

        return ExitRequestAction::AllowExit;
    }

    if code.is_none() || code == Some(0) {
        return ExitRequestAction::StartGracefulExit;
    }

    ExitRequestAction::AllowExit
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(native_capture::NativeCaptureState::default())
        .manage(native_capture::MicrophoneControllerPreferencesState::default())
        .manage(native_capture::MicrophoneDeviceChangeNotifierState::default())
        .manage(native_capture::SystemWakeNotifierState::default())
        .manage(native_capture::MetadataNotifierState::default())
        .manage(native_capture::PrivacyFilterRefreshState::default())
        .manage(native_capture::RecordingSettingsState::default())
        .manage(one_time_prompts::OneTimePromptStateStore::default())
        .manage(native_capture::CaptureMetadataState::default())
        .manage(status_bar::StatusBarState::default())
        .manage(keyboard_bindings::KeyboardBindingsState::default())
        .manage(native_capture::AppNotificationsState::default())
        .manage(audio_transcription_models::AudioTranscriptionModelDownloadState::default())
        .manage(speaker_analysis_models::SpeakerAnalysisModelDownloadState::default())
        .manage(ocr_models::OcrModelDownloadState::default())
        .manage(windows::OnboardingStateStore::default())
        .manage(windows::AppExitCoordinatorState::default())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .level_for("capture_runtime", tauri_plugin_log::log::LevelFilter::Debug)
                .level_for("mnema_lib", tauri_plugin_log::log::LevelFilter::Debug)
                .filter(|metadata| is_app_log_target(metadata.target()))
                .targets([
                    Target::new(TargetKind::Stderr),
                    Target::new(TargetKind::LogDir {
                        file_name: Some(APP_LOG_FILE_NAME.to_string()),
                    }),
                ])
                .build(),
        )
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(keyboard_bindings::handle_global_shortcut)
                .build(),
        )
        .on_window_event(|window, event| {
            let webview_window = window.get_webview_window(window.label());
            if !should_forward_window_event(event, webview_window.is_some()) {
                return;
            }

            windows::handle_window_event(
                &window.app_handle(),
                window.label(),
                event,
                webview_window.as_ref(),
            );
        })
        .invoke_handler(tauri::generate_handler![
            app_infra::get_app_infra_status,
            app_infra::preview_retention_cleanup,
            app_infra::run_retention_cleanup_now,
            app_infra::get_retention_cleanup_status,
            app_infra::list_broker_grants,
            app_infra::create_broker_grant,
            app_infra::revoke_broker_grant,
            app_infra::get_mnema_cli_status,
            app_infra::install_mnema_cli,
            app_infra::delete_recent_capture,
            one_time_prompts::get_one_time_prompt_state,
            one_time_prompts::mark_one_time_prompt_shown,
            one_time_prompts::dismiss_one_time_prompt,
            one_time_prompts::complete_one_time_prompt,
            sensitive_capture_recommendations::get_sensitive_capture_recommendations,
            audio_transcription_models::get_audio_transcription_model_status,
            audio_transcription_models::start_audio_transcription_model_download,
            audio_transcription_models::cancel_audio_transcription_model_download,
            audio_transcription_models::delete_unused_audio_transcription_models,
            audio_transcription_models::request_apple_speech_recognition_permission,
            audio_transcription_models::open_apple_speech_recognition_privacy_settings,
            speaker_analysis_models::get_speaker_analysis_model_status,
            speaker_analysis_models::start_speaker_analysis_model_download,
            speaker_analysis_models::cancel_speaker_analysis_model_download,
            speaker_analysis_models::delete_speaker_analysis_model,
            ocr_models::get_ocr_model_status,
            ocr_models::start_ocr_model_download,
            ocr_models::cancel_ocr_model_download,
            ocr_models::delete_unused_ocr_models,
            app_infra::submit_debug_cpu_job,
            app_infra::list_app_jobs,
            app_infra::get_app_job,
            app_infra::debug_insert_frame_and_enqueue_processing_job,
            app_infra::debug_insert_frame_and_enqueue_ocr,
            app_infra::reprocess_captured_frame_ocr,
            app_infra::reprocess_audio_segment_transcription,
            app_infra::reprocess_audio_segment_speaker_analysis,
            app_infra::reprocess_system_audio_speech_activity,
            app_infra::classify_hidden_segment_workspace,
            app_infra::list_frames,
            app_infra::list_frame_summaries_in_range,
            app_infra::get_latest_frame_in_range,
            app_infra::list_audio_segments,
            app_infra::get_audio_segment,
            app_infra::get_audio_segment_media,
            app_infra::get_frame,
            app_infra::get_earliest_earlier_equivalent_frame,
            app_infra::get_nearest_earlier_equivalent_frame,
            app_infra::get_timeline_window_around_frame,
            app_infra::search_capture,
            app_infra::frame_preview::get_frame_preview,
            app_infra::frame_preview::get_frame_scrub_previews,
            app_infra::frame_preview::get_scrub_preview_availability,
            app_infra::frame_preview::get_scrub_preview_cache_status,
            app_infra::frame_preview::clear_scrub_preview_cache,
            app_infra::frame_preview::cancel_active_frame_preview_video_requests,
            app_infra::list_processing_jobs,
            app_infra::get_processing_job,
            app_infra::get_processing_result,
            ocr_budget::get_ocr_budget_debug,
            app_infra::list_processing_results,
            app_infra::list_speaker_turns,
            app_infra::list_person_profiles,
            app_infra::create_person_profile,
            app_infra::delete_person_profile,
            app_infra::list_speaker_clusters,
            app_infra::name_speaker_cluster,
            app_infra::link_speaker_cluster_to_person,
            app_infra::unlink_speaker_cluster_from_person,
            app_infra::confirm_speaker_recognition_suggestion,
            app_infra::reject_speaker_recognition_suggestion,
            app_infra::merge_speaker_clusters,
            app_infra::move_speaker_turn_to_cluster,
            general_app_log::get_general_app_log_status,
            general_app_log::open_general_app_log,
            general_app_log::delete_general_app_log,
            native_capture::get_capture_support,
            native_capture::get_capture_permissions,
            native_capture::get_idle_debug,
            native_capture::get_app_notifications,
            native_capture::clear_app_notification,
            native_capture::clear_app_notifications,
            native_capture::list_privacy_app_candidates,
            native_capture::resolve_app_icons,
            native_capture::check_browser_url_support,
            native_capture::get_capture_privacy_debug,
            native_capture::get_recording_settings,
            native_capture::update_recording_settings,
            privacy_redaction_sources::add_privacy_excluded_app,
            privacy_redaction_sources::set_privacy_excluded_app_enabled,
            privacy_redaction_sources::remove_privacy_excluded_app,
            native_capture::get_native_capture_debug_log_status,
            native_capture::delete_native_capture_debug_log,
            native_capture::get_microphone_controller_state,
            native_capture::update_microphone_controller,
            native_capture::start_native_capture,
            native_capture::pause_native_capture,
            native_capture::resume_native_capture,
            native_capture::stop_native_capture,
            windows::open_settings_window,
            windows::open_settings_window_to_tab,
            windows::open_debug_window,
            windows::close_current_window,
            windows::toggle_main_window_visibility_command,
            windows::get_onboarding_state,
            windows::complete_onboarding,
            keyboard_bindings::get_keyboard_bindings_settings,
            keyboard_bindings::update_keyboard_bindings_settings,
        ])
        .setup(|app| {
            let app_handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    emit_broker_open_result(&app_handle, &url);
                }
            });
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                for url in urls {
                    emit_broker_open_result(app.handle(), &url);
                }
            }
            let _ = app.deep_link().register_all();
            windows::install_macos_terminate_handler(app.handle());
            native_capture::initialize_recording_settings_from_disk(app.handle());
            one_time_prompts::initialize(app.handle());
            status_bar::initialize(app.handle())?;
            keyboard_bindings::initialize(app.handle());
            native_capture::install_panic_hook();
            if let Err(error) = app_infra::initialize(app) {
                match error {
                    app_infra::AppInfraInitializeError::AlreadyRunning => {
                        app.dialog()
                            .message(ALREADY_RUNNING_MESSAGE)
                            .kind(MessageDialogKind::Warning)
                            .title("Mnema is already running")
                            .blocking_show();
                        app.handle().exit(0);
                        return Ok(());
                    }
                    app_infra::AppInfraInitializeError::Other(message) => {
                        return Err(std::io::Error::other(message).into());
                    }
                }
            }
            native_capture::maybe_push_audio_transcription_unavailable_startup_warning(
                app.handle(),
            );
            native_capture::maybe_push_ocr_unavailable_startup_warning(app.handle());
            native_capture::start_microphone_device_change_notifier(app.handle().clone());
            native_capture::start_system_wake_notifier(app.handle().clone());
            native_capture::start_metadata_notifier(app.handle().clone());
            let onboarding_state = app.state::<windows::OnboardingStateStore>();
            let onboarding_complete =
                windows::open_startup_window(app.handle(), onboarding_state.inner())
                    .map_err(std::io::Error::other)?;
            if onboarding_complete {
                native_capture::maybe_auto_start_native_capture(app.handle());
            }
            status_bar::refresh(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| match event {
            tauri::RunEvent::ExitRequested { code, api, .. } => {
                match exit_request_action_for_exit_request(
                    code,
                    windows::is_graceful_exit_in_progress(app_handle),
                    windows::is_final_graceful_exit_ready(app_handle),
                ) {
                    ExitRequestAction::StartGracefulExit => {
                        api.prevent_exit();
                        windows::request_graceful_exit(app_handle);
                    }
                    ExitRequestAction::PreventExit => {
                        api.prevent_exit();
                    }
                    ExitRequestAction::AllowExit => {}
                }
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } => {
                let _ = windows::open_main_window(app_handle);
            }
            _ => {}
        });
}

pub fn maybe_run_speaker_analysis_helper_and_exit() {
    speaker_analysis_runtime::maybe_run_subprocess_helper_and_exit();
}

#[cfg(test)]
mod tests {
    use super::{
        exit_request_action_for_exit_request, is_app_log_target, should_forward_window_event,
        ExitRequestAction,
    };

    #[test]
    fn app_log_filter_keeps_only_our_targets() {
        assert!(is_app_log_target("mnema_lib::native_capture"));
        assert!(is_app_log_target("capture_runtime"));
        assert!(is_app_log_target("app_infra::processing::runtime"));
        assert!(is_app_log_target(tauri_plugin_log::WEBVIEW_TARGET));

        assert!(!is_app_log_target("ort::logging"));
        assert!(!is_app_log_target("tauri"));
        assert!(!is_app_log_target("sqlx::query"));
        assert!(!is_app_log_target("capture_runtime_extra"));
    }

    #[test]
    fn destroyed_events_are_forwarded_even_when_manager_lookup_fails() {
        assert!(should_forward_window_event(
            &tauri::WindowEvent::Destroyed,
            false,
        ));
    }

    #[test]
    fn non_destroyed_events_without_a_resolved_webview_window_are_ignored() {
        assert!(!should_forward_window_event(
            &tauri::WindowEvent::Focused(true),
            false,
        ));
    }

    #[test]
    fn user_exit_requests_start_graceful_exit() {
        assert_eq!(
            exit_request_action_for_exit_request(None, false, false),
            ExitRequestAction::StartGracefulExit
        );
    }

    #[test]
    fn zero_exit_code_requests_start_graceful_exit_when_not_already_exiting() {
        assert_eq!(
            exit_request_action_for_exit_request(Some(0), false, false),
            ExitRequestAction::StartGracefulExit
        );
    }

    #[test]
    fn final_zero_exit_code_request_is_allowed_after_graceful_exit_is_ready() {
        assert_eq!(
            exit_request_action_for_exit_request(Some(0), true, true),
            ExitRequestAction::AllowExit
        );
    }

    #[test]
    fn repeated_user_exit_request_is_prevented_while_graceful_exit_is_running() {
        assert_eq!(
            exit_request_action_for_exit_request(None, true, false),
            ExitRequestAction::PreventExit
        );
    }

    #[test]
    fn repeated_zero_exit_request_is_prevented_while_graceful_exit_is_running() {
        assert_eq!(
            exit_request_action_for_exit_request(Some(0), true, false),
            ExitRequestAction::PreventExit
        );
    }

    #[test]
    fn restart_exit_code_is_not_rewritten_as_a_normal_graceful_quit() {
        assert_eq!(
            exit_request_action_for_exit_request(Some(tauri::RESTART_EXIT_CODE), false, false),
            ExitRequestAction::AllowExit
        );
    }

    #[test]
    fn restart_exit_code_is_not_blocked_while_graceful_exit_is_running() {
        assert_eq!(
            exit_request_action_for_exit_request(Some(tauri::RESTART_EXIT_CODE), true, false),
            ExitRequestAction::AllowExit
        );
    }

    #[test]
    fn user_exit_request_is_still_prevented_after_final_exit_is_ready() {
        assert_eq!(
            exit_request_action_for_exit_request(None, true, true),
            ExitRequestAction::PreventExit
        );
    }
}
