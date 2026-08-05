//! Current-frame ask: "what's on screen right now" (round-4 decisions G1–G3).
//!
//! Two seams live here:
//!
//! - [`capture_current_frame`] — the Tauri command Quick Access invokes when it
//!   collapses to the bar. Takes ONE live ScreenCaptureKit screenshot through a
//!   filter that excludes Mnema's own windows and the privacy-listed apps, and
//!   reports the excluded apps that were on screen so the chip can name them.
//!   It also resolves the vision-capability dimension up front, because the
//!   disclosure has to be on the chip *before* the user types (G2).
//! - [`plan_frame_context`] — the send-time decision: attach pixels, or send the
//!   screen's OCR text plus window metadata. `ask_ai` calls it after resolving
//!   the engine, so the model that actually answers is the one the decision was
//!   made against.
//!
//! The screenshot is written to one fixed slot in the app cache dir. One Quick
//! Access window means one live capture, so a re-grab overwrites the previous
//! shot and nothing needs cleaning up.
//!
//! ponytail: single-slot file; give it a per-capture name only if a second
//! surface ever captures concurrently.

use capture_types::CurrentFrameCapture;
use tauri::Manager;

const CURRENT_FRAME_FILE_NAME: &str = "current-frame.jpg";

/// How the frame reaches the model this turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameContextPlan {
    /// Prepended to the question. Always names the app/window/time and any
    /// blanked apps — that context is useful to a vision model too.
    pub prompt_block: String,
    /// Send the JPEG's pixels.
    pub attach_image: bool,
    /// OCR the JPEG and append the text to `prompt_block` (the non-vision path).
    pub needs_ocr: bool,
}

/// Decide how a captured frame reaches the model.
///
/// `vision_supported` comes from `ai_runtime::resolve_engine_config_with_vision`
/// — the resolver's vision dimension — so the fallback is chosen against the
/// model that will actually answer, not against a stale flag from capture time.
/// The feature is never withheld: a non-vision model gets the screen's text.
pub(crate) fn plan_frame_context(
    frame: &CurrentFrameCapture,
    vision_supported: bool,
) -> FrameContextPlan {
    let mut prompt_block =
        String::from("Screen context: the user is asking about what is on their screen right now");
    if let Some(app) = frame
        .app_name
        .as_deref()
        .map(str::trim)
        .filter(|app| !app.is_empty())
    {
        prompt_block.push_str(&format!(" in {app}"));
        if let Some(title) = frame
            .window_title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
        {
            prompt_block.push_str(&format!(" (window: \"{title}\")"));
        }
    }
    prompt_block.push_str(".\n");

    if !frame.excluded_app_names.is_empty() {
        // Told to the MODEL as well as to the user: an answer that reasons about
        // a blank region should know why it is blank, and must not guess at what
        // was hidden.
        prompt_block.push_str(&format!(
            "These apps were blanked out of the screenshot for privacy and you cannot see them: {}.\n",
            frame.excluded_app_names.join(", ")
        ));
    }

    FrameContextPlan {
        prompt_block,
        attach_image: vision_supported,
        needs_ocr: !vision_supported,
    }
}

/// Append OCR'd screen text to the context block (the non-vision send path).
pub(crate) fn append_frame_text(prompt_block: &mut String, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        prompt_block
            .push_str("No readable text was found on the screen; say so rather than guessing.\n");
        return;
    }
    prompt_block.push_str("Text read from the screen:\n");
    prompt_block.push_str(text);
    prompt_block.push('\n');
}

/// OCR one screenshot with Apple Vision — no model download, no job queue, no
/// DB row. Blocking; callers run it off the async runtime's poll thread.
pub(crate) fn ocr_current_frame(image_path: &std::path::Path) -> Result<String, String> {
    use app_infra::{AppleVisionProvider, OcrProvider, OcrRequest};

    tauri::async_runtime::block_on(
        AppleVisionProvider::new().recognize(OcrRequest::new(image_path, "apple_vision")),
    )
    .map(|output| output.text)
    .map_err(|error| error.to_string())
}

/// Read a captured frame back as base64 JPEG for the vision send path.
pub(crate) fn read_frame_base64(image_path: &std::path::Path) -> Result<String, String> {
    use base64::Engine as _;

    std::fs::read(image_path)
        .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
        .map_err(|error| format!("Failed to read the captured frame: {error}"))
}

/// Take the live screenshot and describe it for the context chip.
///
/// Capture is implicit (G3): collapsing the Quick Access window IS the gesture,
/// so this is invoked by the collapse, not by an "attach screen?" prompt.
#[tauri::command]
pub async fn capture_current_frame(
    app_handle: tauri::AppHandle,
) -> Result<CurrentFrameCapture, String> {
    let settings = crate::native_capture::current_recording_settings_from_app_handle(&app_handle);

    // The SAME exclude-list source as the recording content filter and the
    // system-audio tap: the privacy settings' enabled excluded-app entries, run
    // through the one evaluator. Never a second list.
    let excluded_bundle_ids = capture_metadata::evaluate_privacy(
        &settings.privacy,
        &capture_metadata::MetadataContext::default(),
    )
    .excluded_bundle_ids;

    // The vision dimension is resolved HERE, not at send time, because the
    // disclosure has to be on the chip before the user types (G2). Send re-runs
    // the resolver, so a model change between collapse and send still lands on
    // the right path.
    let (config, vision_supported) = crate::ai_runtime::resolve_engine_config_with_vision(
        &settings.ai_runtime,
        None,
        settings.access.ask_ai_model.as_deref(),
    )
    .map_err(|reason| format!("Ask AI is not configured yet ({reason})"))?;
    let model_label = engine_model_label(&config);

    let output_path = app_handle
        .path()
        .app_cache_dir()
        .map_err(|error| format!("No app cache directory: {error}"))?
        .join(CURRENT_FRAME_FILE_NAME);
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to prepare the capture directory: {error}"))?;
    }

    let own_pid = std::process::id() as i32;
    let own_bundle_id = app_handle.config().identifier.clone();
    let capture_path = output_path.clone();
    let plan = tauri::async_runtime::spawn_blocking(move || {
        capture_screen::current_frame::capture_current_frame_jpeg(
            &excluded_bundle_ids,
            own_pid,
            &own_bundle_id,
            &capture_path,
        )
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.message)?;

    Ok(CurrentFrameCapture {
        image_path: output_path.to_string_lossy().to_string(),
        captured_at_unix_ms: now_unix_ms(),
        app_name: plan.app_name,
        window_title: plan.window_title,
        excluded_app_names: plan.excluded_app_names,
        vision_supported,
        model_label,
    })
}

fn engine_model_label(config: &ai_engine::EngineConfig) -> String {
    match config {
        ai_engine::EngineConfig::Cloud { model, .. }
        | ai_engine::EngineConfig::Local { model, .. } => model.clone(),
    }
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> CurrentFrameCapture {
        CurrentFrameCapture {
            image_path: "/tmp/current-frame.jpg".to_string(),
            captured_at_unix_ms: 1_717_000_123_456,
            app_name: Some("Safari".to_string()),
            window_title: Some("Quarterly plan".to_string()),
            excluded_app_names: Vec::new(),
            vision_supported: true,
            model_label: "claude-sonnet-4-5".to_string(),
        }
    }

    #[test]
    fn vision_models_get_pixels_and_no_ocr() {
        let plan = plan_frame_context(&frame(), true);
        assert!(plan.attach_image);
        assert!(!plan.needs_ocr);
        assert!(plan.prompt_block.contains("Safari"));
        assert!(plan.prompt_block.contains("Quarterly plan"));
    }

    #[test]
    fn non_vision_models_fall_back_to_screen_text() {
        let plan = plan_frame_context(&frame(), false);
        assert!(
            !plan.attach_image,
            "pixels must not be sent to a text-only model"
        );
        assert!(plan.needs_ocr, "the feature stays available via OCR text");
        // Window metadata rides along on the fallback path too (G2).
        assert!(plan.prompt_block.contains("Safari"));
        assert!(plan.prompt_block.contains("Quarterly plan"));
    }

    #[test]
    fn blanked_apps_are_named_to_the_model_on_both_paths() {
        let mut frame = frame();
        frame.excluded_app_names = vec!["1Password".to_string(), "Signal".to_string()];

        for vision in [true, false] {
            let plan = plan_frame_context(&frame, vision);
            assert!(plan.prompt_block.contains("1Password, Signal"));
            assert!(plan.prompt_block.contains("cannot see them"));
        }
    }

    #[test]
    fn a_frame_with_no_window_metadata_still_produces_a_context_block() {
        let mut frame = frame();
        frame.app_name = None;
        frame.window_title = Some("   ".to_string());

        let plan = plan_frame_context(&frame, false);
        assert!(plan.prompt_block.starts_with("Screen context:"));
        assert!(!plan.prompt_block.contains("(window:"));
    }

    #[test]
    fn empty_ocr_text_says_so_instead_of_sending_a_blank_block() {
        let mut block = String::new();
        append_frame_text(&mut block, "   ");
        assert!(block.contains("No readable text"));

        let mut block = String::new();
        append_frame_text(&mut block, "  Quarterly plan\nRevenue up 4%  ");
        assert!(block.contains("Text read from the screen:"));
        assert!(block.contains("Revenue up 4%"));
    }
}
