mod ai_runtime;
mod app_infra;
mod app_updates;
mod ask_ai;
mod audio_transcription_models;
mod broker_authorization_channel;
mod cli_access;
mod conversation;
mod debug_health;
mod debug_pipeline;
mod debug_status;
mod deep_links;
mod general_app_log;
mod keyboard_bindings;
mod licensing;
mod managed_storage_layout;
mod native_capture;
mod ocr_models;
mod one_time_prompts;
mod privacy_redaction_sources;
mod semantic_search_models;
mod sensitive_capture_recommendations;
mod speaker_analysis_models;
mod status_bar;
mod tauri_host;
mod transcription_deepgram;
mod usage_charts;
mod user_context;
mod voice_enrollment;
mod webview_cache;
mod windows;


use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use app_core::debug_log::{
    APP_LOG_FILE_NAME, APP_LOG_KEPT_FILES, APP_LOG_MAX_FILE_BYTES, APP_LOG_TARGET_PREFIXES,
};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};

const ALREADY_RUNNING_MESSAGE: &str =
    "Mnema is already running. Close the existing Mnema window before opening it again.";
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
    let context = tauri::generate_context!();
    let (host, core) = tauri_host::start_core(&context.config().identifier);
    app_core::startup::manage_default_states(&core);
    // The states the Tauri commands also take as `State`: the same `Arc`s.
    fn shared<T: Clone + Send + Sync + 'static>(core: &app_core::AppCore) -> T {
        (*core.state::<T>()).clone()
    }
    let native_capture_state =
        shared::<app_core::native_capture::lifecycle::NativeCaptureState>(&core);
    let app_notifications =
        shared::<app_core::native_capture::app_notifications::AppNotificationsState>(&core);
    let microphone_preferences =
        shared::<app_core::native_capture::microphone::MicrophoneControllerPreferencesState>(&core);
    let recording_settings = shared::<app_core::recording_settings::RecordingSettingsState>(&core);
    let semantic_worker_health = shared::<app_core::debug_status::SemanticWorkerHealthState>(&core);
    tauri::Builder::default()
        .manage(core)
        .manage(native_capture_state)
        .manage(microphone_preferences)
        .manage(recording_settings)
        .manage(status_bar::StatusBarState::default())
        .manage(keyboard_bindings::KeyboardBindingsState::default())
        .manage(app_notifications)
        .manage(app_updates::AppUpdateRuntimeState::default())
        .manage(windows::PendingOpenSettingsState::default())
        .manage(deep_links::BrokerOpenCaptureResultState::default())
        .manage(deep_links::InsightsOpenConversationState::default())
        .manage(deep_links::LicenseDeepLinkState::default())
        .manage(app_core::semantic_search_query::SemanticQueryEmbedderState::new())
        .manage(semantic_worker_health)
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = windows::open_main_window(app);
        }))
        .plugin(
            APP_LOG_TARGET_PREFIXES
                .iter()
                .fold(
                    tauri_plugin_log::Builder::new()
                        .level(tauri_plugin_log::log::LevelFilter::Info),
                    |builder, target| {
                        builder.level_for(*target, tauri_plugin_log::log::LevelFilter::Debug)
                    },
                )
                // The plugin default (40 KB, delete on rotate) kept ~5 minutes of
                // history; this keeps days, in at most 5 × 5 MB files.
                .max_file_size(APP_LOG_MAX_FILE_BYTES)
                .rotation_strategy(RotationStrategy::KeepSome(APP_LOG_KEPT_FILES))
                .filter(|metadata| {
                    is_app_log_target(metadata.target())
                        && app_core::debug_log::app_log_record_allowed(metadata.level())
                })
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
            // A user-dismissed approval window is a non-verdict, not a denial:
            // answer the waiting CLI `denied`/`closed` (exit 14, retryable) so
            // it never blocks forever on a window that is gone. This reads
            // `CloseRequested` and only `CloseRequested` on purpose — the app's
            // own teardown after a verdict uses `destroy()`, which emits
            // `Destroyed` instead, so it cannot land here and cancel whatever
            // request has since taken the pending slot.
            if window.label() == "cli-access-request"
                && matches!(event, tauri::WindowEvent::CloseRequested { .. })
            {
                app_core::broker_authorization_channel::cancel_pending_request(
                    &window.app_handle().state::<std::sync::Arc<app_core::AppCore>>(),
                    "closed",
                );
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_infra::get_app_infra_status,
            debug_pipeline::get_processing_pipeline_status,
            debug_pipeline::list_processing_jobs_by_processor,
            debug_health::get_debug_health,
            ask_ai::get_ask_ai_last_turn_usage,
            app_infra::get_storage_location,
            debug_status::get_semantic_index_status,
            debug_status::list_frame_batches,
            debug_status::list_user_context_derivation_runs,
            debug_status::tail_app_log,
            app_updates::get_app_update_status,
            app_updates::check_for_app_update,
            app_updates::set_app_update_channel,
            app_updates::install_app_update,
            app_updates::restart_after_app_update,
            licensing::get_license_status,
            licensing::start_trial,
            licensing::activate_license,
            licensing::refresh_license_now,
            licensing::reset_license_devices,
            licensing::get_license_devices,
            app_infra::retention::preview_retention_cleanup,
            app_infra::retention::run_retention_cleanup_now,
            app_infra::get_retention_cleanup_status,
            cli_access::list_cli_access_grants,
            cli_access::block_cli_access_client,
            cli_access::unblock_cli_access_client,
            cli_access::list_cli_access_history,
            cli_access::get_cli_access_status,
            cli_access::install_cli,
            ask_ai::ask_ai_availability,
            ask_ai::ask_ai_start,
            ask_ai::ask_ai_followup,
            ask_ai::ask_ai_cancel,
            ask_ai::ask_ai_snapshot,
            ask_ai::mcp_commands::mcp_warm_connectors,
            ask_ai::mcp_commands::mcp_list_server_tools,
            ask_ai::mcp_commands::mcp_oauth_begin,
            ask_ai::mcp_commands::mcp_oauth_disconnect,
            ask_ai::mcp_commands::mcp_oauth_statuses,
            ask_ai::mcp_commands::mcp_check_node,
            broker_authorization_channel::get_pending_cli_access_request,
            broker_authorization_channel::approve_pending_cli_access_request,
            broker_authorization_channel::block_pending_cli_access_request,
            broker_authorization_channel::cancel_pending_cli_access_request,
            app_infra::retention::delete_recent_capture,
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
            semantic_search_models::get_semantic_search_model_status,
            semantic_search_models::list_semantic_search_supported_models,
            semantic_search_models::start_semantic_search_model_download,
            semantic_search_models::cancel_semantic_search_model_download,
            semantic_search_models::select_semantic_search_model,
            semantic_search_models::delete_semantic_search_model,
            native_capture::update_semantic_search_settings,
            speaker_analysis_models::get_speaker_analysis_model_status,
            speaker_analysis_models::start_speaker_analysis_model_download,
            speaker_analysis_models::cancel_speaker_analysis_model_download,
            speaker_analysis_models::delete_speaker_analysis_model,
            ocr_models::get_ocr_model_status,
            ocr_models::start_ocr_model_download,
            ocr_models::cancel_ocr_model_download,
            ocr_models::delete_unused_ocr_models,
            get_third_party_notices,
            app_infra::list_app_jobs,
            app_infra::get_app_job,
            app_infra::debug_insert_frame_and_enqueue_processing_job,
            app_infra::debug_insert_frame_and_enqueue_ocr,
            app_infra::reprocess_captured_frame_ocr,
            app_infra::reprocess_audio_segment_transcription,
            app_infra::reprocess_audio_segment_speaker_analysis,
            app_infra::reprocess_system_audio_speech_activity,
            app_infra::list_frames,
            app_infra::list_frame_summaries_in_range,
            app_infra::get_latest_frame_in_range,
            app_infra::list_audio_segments,
            app_infra::get_audio_segment,
            app_infra::get_audio_segment_media,
            app_infra::get_audio_segment_waveform_peaks,
            app_infra::get_frame,
            app_infra::open_captured_url,
            app_infra::get_earliest_earlier_equivalent_frame,
            app_infra::get_nearest_earlier_equivalent_frame,
            app_infra::get_timeline_window_around_frame,
            app_infra::search_capture,
            app_infra::list_searchable_apps,
            app_infra::quick_recall::get_frame_match_boxes,
            app_infra::quick_recall::get_quick_recall_recent_queries,
            app_infra::quick_recall::set_quick_recall_recent_queries,
            app_infra::quick_recall::count_search_facet,
            app_infra::quick_recall::get_quick_recall_index_status,
            app_infra::get_frame_preview,
            app_infra::get_frame_scrub_previews,
            app_infra::get_scrub_preview_availability,
            app_infra::get_scrub_preview_cache_status,
            app_infra::clear_scrub_preview_cache,
            app_infra::cancel_active_frame_preview_video_requests,
            app_infra::list_processing_jobs,
            app_infra::get_processing_job,
            app_infra::retry_processing_job_now,
            app_infra::get_processing_result,
            app_infra::list_processing_results,
            app_infra::list_speaker_turns,
            app_infra::list_person_profiles,
            app_infra::create_person_profile,
            app_infra::delete_person_profile,
            voice_enrollment::enroll_account_owner_voice,
            voice_enrollment::get_account_owner_person_id,
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
            native_capture::get_system_audio_access_hint,
            native_capture::get_browser_url_accessibility_status,
            native_capture::request_browser_url_accessibility,
            native_capture::open_browser_url_accessibility_settings,
            native_capture::get_idle_debug,
            native_capture::get_capture_health,
            native_capture::get_app_notifications,
            native_capture::clear_app_notification,
            native_capture::clear_app_notifications,
            native_capture::snooze_not_recording_reminder,
            native_capture::list_privacy_app_candidates,
            native_capture::resolve_app_icons,
            native_capture::check_browser_url_support,
            native_capture::get_recording_settings,
            system_uses_24_hour_time,
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
            ai_runtime::ai_runtime_provider_key_ending,
            ai_runtime::ai_runtime_chatgpt_begin_login,
            ai_runtime::ai_runtime_chatgpt_cancel_login,
            ai_runtime::mcp_set_server_secret,
            ai_runtime::mcp_clear_server_secret,
            ai_runtime::mcp_has_server_secret,
            transcription_deepgram::transcription_set_deepgram_key,
            transcription_deepgram::transcription_has_deepgram_key,
            transcription_deepgram::transcription_clear_deepgram_key,
            transcription_deepgram::transcription_deepgram_auth_status,
            transcription_deepgram::transcription_test_deepgram,
            ai_runtime::get_ai_runtime_status,
            ai_runtime::ai_runtime_test_connection,
            ai_runtime::ai_runtime_list_models,
            ai_runtime::verify_ai_provider,
            user_context::commands::get_user_context_status,
            user_context::commands::list_user_context_activities,
            user_context::commands::list_user_context_activity_meta,
            user_context::commands::list_failed_derivation_windows,
            user_context::commands::list_user_context_conclusions,
            user_context::commands::get_user_context_subject,
            user_context::commands::get_user_context_digest,
            user_context::commands::regenerate_user_context_digest,
            user_context::commands::user_context_run_derivation_now,
            user_context::commands::user_context_dismiss_conclusion,
            user_context::commands::user_context_list_dismissed,
            user_context::commands::user_context_restore_dismissed,
            user_context::commands::user_context_set_pinned,
            user_context::commands::user_context_correct_activity_category,
            user_context::commands::user_context_correct_activity_focus,
            user_context::commands::list_user_context_authored,
            user_context::commands::user_context_add_authored,
            user_context::commands::user_context_update_authored,
            user_context::commands::user_context_delete_authored,
            user_context::commands::wipe_user_context,
            user_context::commands::user_context_stamp_local_offset,
            conversation::list_conversations,
            conversation::get_conversation,
            conversation::search_conversations,
            conversation::set_conversation_engine,
            conversation::set_conversation_title,
            conversation::set_conversation_pinned,
            conversation::delete_conversation,
            conversation::delete_last_turn,
            usage_charts::get_usage_charts,
            usage_charts::capture_presence,
            usage_charts::get_capture_hours,
            privacy_redaction_sources::add_privacy_excluded_app,
            privacy_redaction_sources::set_privacy_excluded_app_enabled,
            privacy_redaction_sources::set_privacy_filter_system_audio,
            privacy_redaction_sources::remove_privacy_excluded_app,
            native_capture::get_microphone_controller_state,
            native_capture::update_microphone_controller,
            native_capture::get_microphone_activity_level,
            native_capture::start_system_audio_level_probe,
            native_capture::get_system_audio_probe_level,
            native_capture::record_bounded_microphone_clip,
            native_capture::start_native_capture,
            native_capture::pause_native_capture,
            native_capture::resume_native_capture,
            native_capture::stop_native_capture,
            windows::focus_main_and_open_settings,
            windows::drain_pending_open_settings,
            windows::request_app_relaunch,
            windows::open_debug_window,
            windows::close_current_window,
            windows::permission_helper::show_permission_helper,
            windows::focus_quick_recall_window,
            windows::quick_recall_frame::set_quick_recall_frame,
            windows::quick_recall_suppress_blur_dismiss,
            windows::summon_quick_recall_window_command,
            windows::take_quick_recall_query,
            windows::toggle_main_window_visibility_command,
            windows::get_onboarding_state,
            windows::complete_onboarding,
            managed_storage_layout::probe_storage_path,
            managed_storage_layout::get_storage_usage,
            keyboard_bindings::get_keyboard_bindings_settings,
            keyboard_bindings::update_keyboard_bindings_settings,
            deep_links::drain_pending_broker_open_capture_results,
            deep_links::open_capture_result_in_main_window,
            deep_links::drain_pending_insights_open_conversations,
            deep_links::take_pending_license_deep_link,
            deep_links::has_pending_insights_open_conversations,
            deep_links::open_conversation_in_chat,
        ])
        .setup(move |app| {
            host.attach(app.handle().clone());
            let app_handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    deep_links::dispatch_deep_link(&app_handle, &url);
                }
            });
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                for url in urls {
                    deep_links::dispatch_deep_link(app.handle(), &url);
                }
            }
            let _ = app.deep_link().register_all();
            windows::install_macos_terminate_handler(app.handle());
            let core = app
                .state::<std::sync::Arc<app_core::AppCore>>()
                .inner()
                .clone();
            let started = core.start(
                |_| {
                    status_bar::initialize(app.handle())?;
                    keyboard_bindings::initialize(app.handle());
                    Ok(())
                },
                |core| {
                    use app_core::native_capture::os_notifiers as os;
                    os::start_system_wake_notifier(core, true);
                    os::start_display_reconfiguration_notifier(core);
                    os::start_metadata_notifier(core);
                },
            );
            let report = match started {
                Ok(report) => report,
                Err(app_core::StartError::AlreadyRunning) => {
                    app.dialog()
                        .message(ALREADY_RUNNING_MESSAGE)
                        .kind(MessageDialogKind::Warning)
                        .title("Mnema is already running")
                        .blocking_show();
                    app.handle().exit(0);
                    return Ok(());
                }
                Err(app_core::StartError::Failed(error)) => return Err(error),
            };
            app_infra::manage_shared_states(app).map_err(std::io::Error::other)?;
            let onboarding_state = core.state::<app_core::onboarding_state::OnboardingStateStore>();
            windows::open_startup_window(app.handle(), &onboarding_state)
                .map_err(std::io::Error::other)?;
            core.spawn_deferred_startup(&report);
            status_bar::refresh(app.handle());
            Ok(())
        })
        .build(context)
        .expect("error while building tauri application")
        .run(|app_handle, event| match event {
            tauri::RunEvent::ExitRequested { code, api, .. } => {
                match exit_request_action_for_exit_request(
                    code,
                    app_core::exit::is_graceful_exit_in_progress(
                        &app_handle.state::<std::sync::Arc<app_core::AppCore>>(),
                    ),
                    windows::is_final_graceful_exit_ready(app_handle),
                ) {
                    ExitRequestAction::StartGracefulExit => {
                        api.prevent_exit();
                        app_handle
                            .state::<std::sync::Arc<app_core::AppCore>>()
                            .shutdown(app_core::ExitKind::Quit);
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

#[tauri::command]
fn get_third_party_notices() -> app_core::third_party_notices::ThirdPartyNotices {
    app_core::third_party_notices::collect_third_party_notices()
}

#[tauri::command]
fn system_uses_24_hour_time() -> Option<bool> {
    app_core::clock_format::system_uses_24_hour_time()
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
