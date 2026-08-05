//! The one-shot current-frame capture handed to the frontend and handed back on
//! send (round-4 decisions G1–G3).
//!
//! Hand-mirrored in `apps/desktop/src/lib/quick-recall/current-frame.ts` — there
//! is no codegen, so the round-trip test below plus `bun run check` are the only
//! guards against drift.
//!
//! The frontend never decides anything from this: it renders the chip
//! (`excludedAppNames`, `visionSupported`, `modelLabel`) and hands the whole
//! record back with the question. The backend re-derives pixels-vs-OCR-text at
//! send time.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentFrameCapture {
    /// Absolute path of the JPEG written at invoke time.
    pub image_path: String,
    pub captured_at_unix_ms: i64,
    /// The app owning the frontmost non-excluded window, if any.
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub window_title: Option<String>,
    /// Privacy-listed apps that were on screen and got blanked. Named in the
    /// chip ("1Password excluded") — never silently dropped.
    #[serde(default)]
    pub excluded_app_names: Vec<String>,
    /// Whether the model this turn will resolve to can read images. `false`
    /// drives the upfront chip disclosure AND the OCR-text send path.
    pub vision_supported: bool,
    /// The model id the disclosure names, e.g. "llama3.1 can't see images".
    #[serde(default)]
    pub model_label: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_frame_capture_round_trips_in_camel_case() {
        let capture = CurrentFrameCapture {
            image_path: "/tmp/mnema-current-frame.jpg".to_string(),
            captured_at_unix_ms: 1_717_000_123_456,
            app_name: Some("Safari".to_string()),
            window_title: Some("Docs".to_string()),
            excluded_app_names: vec!["1Password".to_string()],
            vision_supported: false,
            model_label: "llama3.1".to_string(),
        };

        let json = serde_json::to_value(&capture).expect("serialize");
        assert_eq!(json["imagePath"], "/tmp/mnema-current-frame.jpg");
        assert_eq!(json["capturedAtUnixMs"], 1_717_000_123_456i64);
        assert_eq!(json["appName"], "Safari");
        assert_eq!(json["windowTitle"], "Docs");
        assert_eq!(json["excludedAppNames"][0], "1Password");
        assert_eq!(json["visionSupported"], false);
        assert_eq!(json["modelLabel"], "llama3.1");

        let back: CurrentFrameCapture = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, capture);
    }

    #[test]
    fn optional_fields_default_when_absent() {
        let back: CurrentFrameCapture = serde_json::from_str(
            r#"{"imagePath":"/tmp/a.jpg","capturedAtUnixMs":1,"visionSupported":true}"#,
        )
        .expect("deserialize");
        assert_eq!(back.app_name, None);
        assert!(back.excluded_app_names.is_empty());
        assert!(back.model_label.is_empty());
    }
}
