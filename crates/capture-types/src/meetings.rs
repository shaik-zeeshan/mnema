//! Meetings surface DTOs (Warm Paper redesign, Slice 1) — the wire shapes of
//! the `list_meetings` / `get_meeting` / `set_meeting_notes` Tauri commands.
//!
//! Hand-mirrored in `apps/desktop/src/lib/meetings/api.ts` (no codegen — keep
//! the serde round-trip test and `bun run check` green). Conventions match the
//! rest of the crate: camelCase serde, `i64` unix-ms timestamps.

use serde::{Deserialize, Serialize};

/// One detected meeting in the day-grouped list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    /// Stable meeting id (`meeting-<start_ms>-<bundle_id>`).
    pub id: String,
    /// "Zoom", or "Google Meet (Arc)" for a browser meeting.
    pub app_display_name: String,
    /// Provenance: the mic-holding app's bundle id.
    pub bundle_id: String,
    /// The sighted meeting URL for a browser meeting; absent for app holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting_url: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    /// `"recap"` | `"processing"` | `"skipped"` | `"transcript_only"`.
    pub state: String,
    /// For `state == "recap"`: the trigger-run conversation to open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// For `state == "skipped"`: the honest reason, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Diarized speaker display names heard in the window (saved person names
    /// where matched, else "Speaker N" labels).
    pub speakers: Vec<String>,
}

/// One local calendar day of meetings, newest day first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDay {
    /// `YYYY-MM-DD` in the user's local time.
    pub day: String,
    pub meetings: Vec<MeetingSummary>,
}

/// One speaker-labeled transcript turn, wall-clock ordered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingTurn {
    pub speaker: String,
    /// Absolute wall-clock unix-ms start of the turn.
    pub started_at_ms: i64,
    pub text: String,
}

/// The meeting detail: summary + the user's notes + the transcript turns +
/// the checked action-items of the recap checklist.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetail {
    pub summary: MeetingSummary,
    pub notes: Option<String>,
    pub turns: Vec<MeetingTurn>,
    /// The recap checklist items the user has ticked (keyed by item text). The
    /// recap markdown owns the item set; this is only which are done.
    #[serde(default)]
    pub checklist: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_wire_shapes_round_trip_in_camel_case() {
        let detail = MeetingDetail {
            summary: MeetingSummary {
                id: "meeting-1000-us.zoom.xos".to_string(),
                app_display_name: "Zoom".to_string(),
                bundle_id: "us.zoom.xos".to_string(),
                meeting_url: Some("https://zoom.us/j/123".to_string()),
                start_ms: 1_000,
                end_ms: 2_000,
                state: "recap".to_string(),
                conversation_id: Some("trigger-recap-2000".to_string()),
                reason: None,
                speakers: vec!["Sarah".to_string(), "Speaker 2".to_string()],
            },
            notes: Some("follow up with Dev".to_string()),
            turns: vec![MeetingTurn {
                speaker: "Sarah".to_string(),
                started_at_ms: 1_100,
                text: "Let's start.".to_string(),
            }],
            checklist: vec!["Send the report".to_string()],
        };
        let value = serde_json::to_value(&detail).unwrap();
        assert_eq!(value["summary"]["appDisplayName"], "Zoom");
        assert_eq!(value["checklist"][0], "Send the report");
        assert_eq!(value["summary"]["meetingUrl"], "https://zoom.us/j/123");
        assert_eq!(value["summary"]["conversationId"], "trigger-recap-2000");
        // Absent optionals skip the key entirely.
        assert!(value["summary"].get("reason").is_none());
        assert_eq!(value["turns"][0]["startedAtMs"], 1_100);
        let round_tripped: MeetingDetail = serde_json::from_value(value).unwrap();
        assert_eq!(round_tripped, detail);

        let day = MeetingDay {
            day: "2026-07-23".to_string(),
            meetings: vec![round_tripped.summary],
        };
        let value = serde_json::to_value(&day).unwrap();
        let round_tripped: MeetingDay = serde_json::from_value(value).unwrap();
        assert_eq!(round_tripped, day);
    }
}
