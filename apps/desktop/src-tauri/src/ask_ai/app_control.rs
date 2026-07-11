//! App-control tools for the Ask AI chat agent (Workstream A).
//!
//! Five unconditional tools let the user drive on-device recording from chat:
//! `capture_status`, `start_capture`, `stop_capture`, `pause_capture`,
//! `resume_capture`. Each takes NO params. `capture_status` returns the current
//! status; the other four perform the action and return the SAME status shape
//! (post-action) so the model always sees the resulting state.
//!
//! Each tool delegates to the existing lifecycle seams in
//! [`crate::native_capture`], which already emit `native-capture-session-changed`
//! and refresh the tray — so the tray flips for free (do NOT re-emit here). The
//! seams run under `spawn_blocking`: `stop` does a `block_on` internally and
//! would panic on the async runtime thread, and `current_native_capture_session`
//! takes a mutex, so all five are treated the same.
//!
//! Security invariant: the status JSON is built BY HAND from a handful of session
//! flags and NEVER serializes [`NativeCaptureSession`] directly — its
//! `output_files` carry local filesystem paths that must not reach a cloud model.

use capture_types::NativeCaptureSession;

/// Empty-object JSON Schema — every app-control tool takes no params.
fn empty_object_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

/// The five app-control tools described to the model. All unconditional — no
/// setting gates them.
pub(crate) fn app_control_tools() -> Vec<ai_engine::AgentTool> {
    vec![
        ai_engine::AgentTool {
            name: "capture_status".to_string(),
            description:
                "Report whether Mnema is currently recording the user's on-device screen/audio \
capture, whether it is paused (by the user or by inactivity) or suspended for low disk, and which \
sources (screen, microphone, system audio) are enabled. Takes no arguments."
                    .to_string(),
            parameters_schema: empty_object_schema(),
        },
        ai_engine::AgentTool {
            name: "start_capture".to_string(),
            description:
                "Start Mnema's on-device screen/audio recording, then return the resulting capture \
status. Use ONLY when the user explicitly asks to start/resume recording in this message. Takes \
no arguments."
                    .to_string(),
            parameters_schema: empty_object_schema(),
        },
        ai_engine::AgentTool {
            name: "stop_capture".to_string(),
            description:
                "Stop Mnema's on-device screen/audio recording, then return the resulting capture \
status. Use ONLY when the user explicitly asks to stop recording in this message. Takes no \
arguments."
                    .to_string(),
            parameters_schema: empty_object_schema(),
        },
        ai_engine::AgentTool {
            name: "pause_capture".to_string(),
            description:
                "Pause Mnema's on-device screen/audio recording (the session stays alive and can be \
resumed), then return the resulting capture status. Use ONLY when the user explicitly asks to \
pause recording in this message. Takes no arguments."
                    .to_string(),
            parameters_schema: empty_object_schema(),
        },
        ai_engine::AgentTool {
            name: "resume_capture".to_string(),
            description:
                "Resume Mnema's on-device screen/audio recording after a pause, then return the \
resulting capture status. Use ONLY when the user explicitly asks to resume recording in this \
message. Takes no arguments."
                    .to_string(),
            parameters_schema: empty_object_schema(),
        },
    ]
}

/// Whether `name` is one of the five app-control tools.
pub(crate) fn is_app_control_tool(name: &str) -> bool {
    matches!(
        name,
        "capture_status" | "start_capture" | "stop_capture" | "pause_capture" | "resume_capture"
    )
}

/// Run the lifecycle seam for `tool` synchronously and return the resulting
/// session. Seam errors are stringified into their readable message so the model
/// sees why an action failed. Runs on the blocking pool (see module docs).
fn run_control_seam(
    tool: &str,
    app_handle: &tauri::AppHandle,
) -> Result<NativeCaptureSession, String> {
    match tool {
        "capture_status" => Ok(crate::native_capture::current_native_capture_session(app_handle)),
        "start_capture" => crate::native_capture::start_native_capture_from_app_handle(
            "ask-ai-chat",
            app_handle,
        )
        .map(|response| response.session)
        .map_err(|error| error.message),
        "stop_capture" => crate::native_capture::stop_native_capture_from_app_handle(app_handle)
            .map(|response| response.session)
            .map_err(|error| error.message),
        "pause_capture" => crate::native_capture::pause_native_capture_from_app_handle(app_handle)
            .map(|response| response.session)
            .map_err(|error| error.message),
        "resume_capture" => {
            crate::native_capture::resume_native_capture_from_app_handle(app_handle)
                .map(|response| response.session)
                .map_err(|error| error.message)
        }
        other => Err(format!("unknown app control tool: {other}")),
    }
}

/// Dispatch an app-control tool: run its seam on the blocking pool, then return
/// the resulting capture status as a serialized JSON string (the executor returns
/// tool results to the model as a JSON string).
pub(crate) async fn execute_app_control_tool(
    app_handle: &tauri::AppHandle,
    tool: &str,
) -> Result<String, String> {
    let handle = app_handle.clone();
    let tool = tool.to_string();
    let session = tauri::async_runtime::spawn_blocking(move || run_control_seam(&tool, &handle))
        .await
        .map_err(|error| format!("app control task failed to join: {error}"))??;
    let status = session_status_json(&session);
    serde_json::to_string(&status)
        .map_err(|error| format!("failed to serialize capture status: {error}"))
}

/// Build the model-facing capture status JSON BY HAND from session flags.
///
/// `sources` defaults to all-false when `requested_sources` is `None`. NEVER
/// includes `output_files` — those are local filesystem paths (security
/// invariant; see module docs).
fn session_status_json(session: &NativeCaptureSession) -> serde_json::Value {
    let sources = session.requested_sources.as_ref();
    serde_json::json!({
        "isRunning": session.is_running,
        "isUserPaused": session.is_user_paused,
        "isInactivityPaused": session.is_inactivity_paused,
        "isLowDiskSuspended": session.is_low_disk_suspended,
        "sources": {
            "screen": sources.map(|s| s.screen).unwrap_or(false),
            "microphone": sources.map(|s| s.microphone).unwrap_or(false),
            "systemAudio": sources.map(|s| s.system_audio).unwrap_or(false),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::{CaptureOutputFiles, CaptureSources};

    fn session_with_output_files() -> NativeCaptureSession {
        NativeCaptureSession {
            is_running: true,
            is_inactivity_paused: false,
            is_user_paused: true,
            is_low_disk_suspended: false,
            requested_sources: Some(CaptureSources {
                screen: true,
                microphone: false,
                system_audio: true,
            }),
            // Deliberately populated so the "never leak local paths" assertion is
            // meaningful — the status JSON must still exclude it.
            output_files: Some(CaptureOutputFiles {
                screen_file: Some("/Users/somebody/.mnema/capture/screen.mov".to_string()),
                screen_files: vec![],
                microphone_file: None,
                microphone_files: vec![],
                system_audio_file: None,
                system_audio_files: vec![],
            }),
            source_sessions: None,
        }
    }

    #[test]
    fn status_json_has_expected_keys_and_maps_flags() {
        let status = session_status_json(&session_with_output_files());
        let obj = status.as_object().expect("status is an object");

        for key in [
            "isRunning",
            "isUserPaused",
            "isInactivityPaused",
            "isLowDiskSuspended",
            "sources",
        ] {
            assert!(obj.contains_key(key), "missing top-level key {key}");
        }

        assert_eq!(status["isRunning"], serde_json::json!(true));
        assert_eq!(status["isUserPaused"], serde_json::json!(true));
        assert_eq!(status["isInactivityPaused"], serde_json::json!(false));
        assert_eq!(status["isLowDiskSuspended"], serde_json::json!(false));

        let sources = status["sources"].as_object().expect("sources is an object");
        for key in ["screen", "microphone", "systemAudio"] {
            assert!(sources.contains_key(key), "missing sources key {key}");
        }
        assert_eq!(status["sources"]["screen"], serde_json::json!(true));
        assert_eq!(status["sources"]["microphone"], serde_json::json!(false));
        assert_eq!(status["sources"]["systemAudio"], serde_json::json!(true));
    }

    #[test]
    fn status_json_never_leaks_output_files() {
        let status = session_status_json(&session_with_output_files());
        let obj = status.as_object().expect("status is an object");
        assert!(!obj.contains_key("outputFiles"));
        assert!(!obj.contains_key("output_files"));
        assert!(!obj.contains_key("outputFile"));

        // Belt-and-suspenders: no local path leaks anywhere in the serialized form.
        let serialized = serde_json::to_string(&status).expect("serialize");
        assert!(!serialized.contains("outputFiles"));
        assert!(!serialized.contains("output_files"));
        assert!(!serialized.contains("outputFile"));
        assert!(!serialized.contains("screen.mov"));
    }

    #[test]
    fn status_json_sources_default_all_false_when_absent() {
        let session = NativeCaptureSession {
            is_running: false,
            is_inactivity_paused: false,
            is_user_paused: false,
            is_low_disk_suspended: false,
            requested_sources: None,
            output_files: None,
            source_sessions: None,
        };
        let status = session_status_json(&session);
        assert_eq!(status["sources"]["screen"], serde_json::json!(false));
        assert_eq!(status["sources"]["microphone"], serde_json::json!(false));
        assert_eq!(status["sources"]["systemAudio"], serde_json::json!(false));
    }

    #[test]
    fn is_app_control_tool_recognizes_the_five_and_rejects_data_tools() {
        for name in [
            "capture_status",
            "start_capture",
            "stop_capture",
            "pause_capture",
            "resume_capture",
        ] {
            assert!(is_app_control_tool(name), "{name} should be app control");
        }
        assert!(!is_app_control_tool("search"));
        assert!(!is_app_control_tool("reference_captures"));
    }

    #[test]
    fn app_control_tools_yields_exactly_the_five_named_tools() {
        let tools = app_control_tools();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "capture_status",
                "start_capture",
                "stop_capture",
                "pause_capture",
                "resume_capture",
            ]
        );
        for tool in &tools {
            assert!(
                !tool.description.trim().is_empty(),
                "{} has an empty description",
                tool.name
            );
            assert_eq!(
                tool.parameters_schema["type"],
                serde_json::json!("object"),
                "{} schema is not an object",
                tool.name
            );
        }
    }
}
