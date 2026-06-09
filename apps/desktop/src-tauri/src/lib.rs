mod ai_runtime;
mod app_infra;
mod app_updates;
mod ask_ai;
mod audio_transcription_models;
mod broker_authorization_channel;
mod cli_access;
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

use std::{collections::VecDeque, path::Path, sync::Mutex};

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
const BROKER_AUTHORIZATION_REQUEST_FILE_NAME: &str = "broker-authorization-request.json";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BrokerOpenCaptureResultPayload {
    opaque_id: String,
    kind: String,
    frame_id: Option<i64>,
    audio_segment_id: Option<i64>,
    /// Audio Search Result Anchor: the match span start within the segment (ms)
    /// and the aligned frame id, so an audio handoff lands on the selected
    /// transcript match instead of the segment start. Absent for the broker-URL
    /// path, which only resolves a capture reference (kind/frame/segment id).
    #[serde(default)]
    span_start_ms: Option<i64>,
    #[serde(default)]
    aligned_frame_id: Option<i64>,
}

#[derive(Default)]
struct BrokerOpenCaptureResultState {
    pending: Mutex<VecDeque<BrokerOpenCaptureResultPayload>>,
}

#[tauri::command]
fn drain_pending_broker_open_capture_results(
    state: tauri::State<'_, BrokerOpenCaptureResultState>,
) -> Vec<BrokerOpenCaptureResultPayload> {
    let Ok(mut pending) = state.pending.lock() else {
        return Vec::new();
    };
    pending.drain(..).collect()
}

#[tauri::command]
fn open_capture_result_in_main_window(
    kind: String,
    frame_id: Option<i64>,
    audio_segment_id: Option<i64>,
    span_start_ms: Option<i64>,
    aligned_frame_id: Option<i64>,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, BrokerOpenCaptureResultState>,
) {
    let payload = BrokerOpenCaptureResultPayload {
        opaque_id: String::from("quick-recall"),
        kind,
        frame_id,
        audio_segment_id,
        span_start_ms,
        aligned_frame_id,
    };
    if let Ok(mut pending) = state.pending.lock() {
        pending.push_back(payload.clone());
    }
    let _ = windows::open_main_window(&app_handle);
    let _ = app_handle.emit(BROKER_OPEN_CAPTURE_RESULT_EVENT, payload);
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

fn broker_opaque_id_from_url(url: &url::Url) -> Option<String> {
    if url.scheme() != "mnema" {
        return None;
    }
    let mut segments = url.path_segments()?.collect::<Vec<_>>();
    if let Some(host) = url.host_str() {
        segments.insert(0, host);
    }
    match segments.as_slice() {
        ["open", opaque_id] | ["broker", "open", opaque_id] => Some((*opaque_id).to_string()),
        _ => return None,
    }
}

async fn broker_payload_from_url(
    config_dir: &Path,
    url: &url::Url,
) -> Option<BrokerOpenCaptureResultPayload> {
    let opaque_id = broker_opaque_id_from_url(url)?;
    let capture_ref = ::app_infra::brokered_access::authorize_active_opaque_capture_reference(
        config_dir, &opaque_id,
    )
    .await
    .ok()
    .flatten()?;
    Some(BrokerOpenCaptureResultPayload {
        opaque_id: capture_ref.opaque_id,
        frame_id: capture_ref.frame_id,
        audio_segment_id: capture_ref.audio_segment_id,
        kind: capture_ref.kind,
        // The broker-URL handoff resolves only a capture reference, so it carries
        // no search-result anchor; the audio receiver falls back to the segment start.
        span_start_ms: None,
        aligned_frame_id: None,
    })
}

fn enqueue_broker_open_result(app_handle: &tauri::AppHandle, url: &url::Url) {
    let Ok(config_dir) = app_handle.path().app_config_dir() else {
        return;
    };
    if broker_opaque_id_from_url(url).is_none() {
        return;
    }
    let url = url.clone();
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let Some(payload) = broker_payload_from_url(&config_dir, &url).await else {
            return;
        };
        if let Ok(mut pending) = app_handle
            .state::<BrokerOpenCaptureResultState>()
            .pending
            .lock()
        {
            pending.push_back(payload.clone());
        }
        let _ = windows::open_main_window(&app_handle);
        let _ = app_handle.emit(BROKER_OPEN_CAPTURE_RESULT_EVENT, payload);
    });
}

fn broker_authorization_request_path(app_handle: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app_handle
        .path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(BROKER_AUTHORIZATION_REQUEST_FILE_NAME))
}

fn drain_pending_broker_authorization_request_from_app(app_handle: &tauri::AppHandle) -> bool {
    let Some(path) = broker_authorization_request_path(app_handle) else {
        return false;
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return false;
    };
    let _ = std::fs::remove_file(&path);
    serde_json::from_str::<serde_json::Value>(&raw).is_ok()
}

fn notify_pending_broker_authorization_request(app_handle: &tauri::AppHandle) -> bool {
    let marker_drained = drain_pending_broker_authorization_request_from_app(app_handle);
    let has_pending_request =
        broker_authorization_channel::has_pending_cli_access_request(app_handle);
    if !should_open_pending_broker_authorization_request(marker_drained, has_pending_request) {
        return false;
    }
    let _ = windows::open_cli_access_request_window(app_handle);
    true
}

fn should_open_pending_broker_authorization_request(
    marker_drained: bool,
    has_pending_request: bool,
) -> bool {
    marker_drained && has_pending_request
}

fn should_notify_pending_broker_authorization_request(
    onboarding_complete: bool,
    already_handled: bool,
) -> bool {
    onboarding_complete && !already_handled
}

fn notify_pending_broker_authorization_request_if_onboarded(app_handle: &tauri::AppHandle) -> bool {
    should_notify_pending_broker_authorization_request(
        windows::is_onboarding_complete(app_handle),
        false,
    ) && notify_pending_broker_authorization_request(app_handle)
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

/// Startup work that runs after the main window is shown, off the window-open
/// critical path: app-infra maintenance + background workers (see
/// [`app_infra::run_deferred_startup_blocking`]) and, once onboarding is
/// complete, capture auto-start and the startup update check. Auto-start runs
/// only after the maintenance/repair passes complete so it preserves the
/// ordering the previous synchronous startup path guaranteed.
fn run_deferred_startup(app_handle: &tauri::AppHandle, onboarding_complete: bool) {
    app_infra::run_deferred_startup_blocking(app_handle);
    // If the user quit while the (now background) startup work was running,
    // graceful exit may have already stopped capture and be heading for a hard
    // process exit. Do not auto-start a NEW capture session or kick off the
    // update check on top of that teardown — that would record a segment
    // `complete_graceful_exit` then kills mid-write. The deferred maintenance
    // above already bails on this condition too (defense in depth).
    if windows::is_graceful_exit_in_progress(app_handle) {
        native_capture::debug_log::log_info(
            "graceful exit in progress; skipping capture auto-start and startup update check",
        );
        return;
    }
    if onboarding_complete {
        native_capture::maybe_auto_start_native_capture(app_handle);
        app_updates::start_startup_update_check(app_handle);
    }
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
        .manage(app_updates::AppUpdateSettingsState::default())
        .manage(app_updates::AppUpdateRuntimeState::default())
        .manage(audio_transcription_models::AudioTranscriptionModelDownloadState::default())
        .manage(speaker_analysis_models::SpeakerAnalysisModelDownloadState::default())
        .manage(ocr_models::OcrModelDownloadState::default())
        .manage(windows::OnboardingStateStore::default())
        .manage(windows::AppExitCoordinatorState::default())
        .manage(BrokerOpenCaptureResultState::default())
        .manage(broker_authorization_channel::BrokerAuthorizationChannelState::default())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if notify_pending_broker_authorization_request_if_onboarded(app) {
                return;
            }
            let _ = windows::open_main_window(app);
        }))
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
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            if window.label() == "cli-access-request"
                && matches!(event, tauri::WindowEvent::CloseRequested { .. })
            {
                broker_authorization_channel::cancel_pending_request(
                    &window.app_handle(),
                    "closed",
                );
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_infra::get_app_infra_status,
            app_updates::get_app_update_status,
            app_updates::check_for_app_update,
            app_updates::set_app_update_channel,
            app_updates::install_app_update,
            app_updates::restart_after_app_update,
            app_infra::preview_retention_cleanup,
            app_infra::run_retention_cleanup_now,
            app_infra::get_retention_cleanup_status,
            cli_access::list_cli_access_grants,
            cli_access::revoke_cli_access_grant,
            cli_access::revoke_cli_access_for_client,
            cli_access::list_cli_access_history,
            cli_access::get_cli_access_status,
            cli_access::install_cli,
            cli_access::reinstall_cli,
            ask_ai::get_pi_runtime_status,
            ask_ai::ask_ai_availability,
            ask_ai::ask_ai_list_models,
            ask_ai::ask_ai_start,
            ask_ai::ask_ai_followup,
            ask_ai::ask_ai_cancel,
            broker_authorization_channel::get_pending_cli_access_request,
            broker_authorization_channel::approve_pending_cli_access_request,
            broker_authorization_channel::cancel_pending_cli_access_request,
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
            app_infra::list_searchable_apps,
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
            native_capture::request_capture_permission,
            native_capture::open_capture_privacy_settings,
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
            native_capture::update_capture_source_settings,
            native_capture::update_capture_timing_settings,
            native_capture::update_video_settings,
            native_capture::update_storage_settings,
            native_capture::update_display_settings,
            native_capture::update_metadata_settings,
            native_capture::update_inactivity_settings,
            native_capture::update_processing_settings,
            native_capture::update_developer_settings,
            native_capture::update_access_settings,
            native_capture::update_ai_runtime_settings,
            native_capture::update_user_context_settings,
            ai_runtime::ai_runtime_set_provider_key,
            ai_runtime::ai_runtime_clear_provider_key,
            ai_runtime::ai_runtime_has_provider_key,
            ai_runtime::get_ai_runtime_status,
            ai_runtime::ai_runtime_test_connection,
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
            windows::focus_quick_recall_window,
            windows::quick_recall_suppress_blur_dismiss,
            windows::toggle_main_window_visibility_command,
            windows::get_onboarding_state,
            windows::complete_onboarding,
            keyboard_bindings::get_keyboard_bindings_settings,
            keyboard_bindings::update_keyboard_bindings_settings,
            drain_pending_broker_open_capture_results,
            open_capture_result_in_main_window,
        ])
        .setup(|app| {
            let app_handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    enqueue_broker_open_result(&app_handle, &url);
                }
            });
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                for url in urls {
                    enqueue_broker_open_result(app.handle(), &url);
                }
            }
            let _ = app.deep_link().register_all();
            windows::install_macos_terminate_handler(app.handle());
            native_capture::initialize_recording_settings_from_disk(app.handle());
            app_updates::initialize(app.handle());
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
            broker_authorization_channel::start(app.handle()).map_err(std::io::Error::other)?;
            native_capture::maybe_push_audio_transcription_unavailable_startup_warning(
                app.handle(),
            );
            native_capture::maybe_push_ocr_unavailable_startup_warning(app.handle());
            native_capture::start_microphone_device_change_notifier(app.handle().clone());
            native_capture::start_system_wake_notifier(app.handle().clone());
            native_capture::start_metadata_notifier(app.handle().clone());
            let onboarding_complete = windows::is_onboarding_complete(app.handle());
            let handled_startup_authorization_request =
                should_notify_pending_broker_authorization_request(onboarding_complete, false)
                    && notify_pending_broker_authorization_request(app.handle());
            if !handled_startup_authorization_request {
                let onboarding_state = app.state::<windows::OnboardingStateStore>();
                windows::open_startup_window(app.handle(), onboarding_state.inner())
                    .map_err(std::io::Error::other)?;
            }
            // The window is open and the database is ready; run the remaining
            // startup work (index maintenance, filesystem repair, background
            // workers) off the window-open critical path so it no longer delays
            // the first paint. Capture auto-start and the update check run only
            // after that maintenance completes, preserving the ordering the old
            // synchronous path guaranteed (notably: hidden-segment workspace
            // repair respects the live active-capture workspace set so it never
            // deletes a workspace a manually-started recording is already using).
            //
            // This thread is detached and never joined. That is safe: it is
            // best-effort and bails early once a graceful exit is requested (both
            // at its start and after maintenance), so it does not start NEW
            // capture or workers while the app is tearing down. Any maintenance
            // step it does not finish is re-run on the next launch — every step
            // commits through SQLite WAL, so a hard process exit mid-step is
            // crash-safe and idempotent to re-run. We therefore do not build a
            // join-with-timeout against the hard `complete_graceful_exit` path.
            let deferred_app_handle = app.handle().clone();
            if let Err(error) = std::thread::Builder::new()
                .name("mnema-deferred-startup".to_string())
                .spawn(move || {
                    run_deferred_startup(&deferred_app_handle, onboarding_complete)
                })
            {
                // Spawning a thread effectively never fails; if it does, run the
                // deferred startup inline as a last resort rather than leaving the
                // app without background workers or capture. This re-blocks first
                // paint with the full maintenance workload, which is why it is the
                // fallback and not the normal path. Logged through the app log sink
                // so a "no background workers" incident is captured in the packaged
                // app log rather than only on stderr.
                native_capture::debug_log::log_error(format!(
                    "failed to spawn deferred startup thread; running inline (re-blocks first paint): {error}"
                ));
                run_deferred_startup(app.handle(), onboarding_complete);
            }
            if should_notify_pending_broker_authorization_request(
                onboarding_complete,
                handled_startup_authorization_request,
            ) {
                notify_pending_broker_authorization_request(app.handle());
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
                if notify_pending_broker_authorization_request(app_handle) {
                    return;
                }
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
        broker_payload_from_url, exit_request_action_for_exit_request, is_app_log_target,
        should_forward_window_event, should_notify_pending_broker_authorization_request,
        should_open_pending_broker_authorization_request, ExitRequestAction,
    };

    #[test]
    fn broker_deep_link_rejects_unsigned_opaque_id() {
        let dir = tempfile::tempdir().expect("config dir should be created");
        let url = url::Url::parse("mnema://open/f1").expect("url should parse");

        let payload = tauri::async_runtime::block_on(broker_payload_from_url(dir.path(), &url));

        assert!(payload.is_none());
    }

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
    fn pending_broker_authorization_waits_for_onboarding() {
        assert!(!should_notify_pending_broker_authorization_request(
            false, false
        ));
    }

    #[test]
    fn pending_broker_authorization_is_not_handled_twice() {
        assert!(!should_notify_pending_broker_authorization_request(
            true, true
        ));
    }

    #[test]
    fn pending_broker_authorization_notifies_after_onboarding_once() {
        assert!(should_notify_pending_broker_authorization_request(
            true, false
        ));
    }

    #[test]
    fn pending_broker_authorization_marker_opens_only_for_real_pending_request() {
        assert!(should_open_pending_broker_authorization_request(true, true));
        assert!(!should_open_pending_broker_authorization_request(
            true, false
        ));
        assert!(!should_open_pending_broker_authorization_request(
            false, true
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
