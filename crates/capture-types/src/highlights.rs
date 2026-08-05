//! Day-highlight DTOs — the read-time "what stood out" surfaces that feed the
//! Overview tiles and the timeline drawer header.
//!
//! Two shapes, both derived at read time from data the pipeline already wrote:
//!
//! - [`ConversationCluster`]: an Activity whose window overlaps recorded speech
//!   for long enough to call it a conversation, with the number of distinct
//!   speakers heard inside it.
//! - [`Moment`]: a frame the Activity derivation flagged as its headline,
//!   ranked by the Activity's focus band and then its duration.
//!
//! The queries live in `crates/app-infra/src/highlights.rs`; the thin Tauri
//! adapter lives in `apps/desktop/src-tauri/src/highlights.rs`. Conventions
//! match the rest of `capture-types`: `camelCase` serde, `i64` unix millis,
//! `Eq` because no field is a float.

use serde::{Deserialize, Serialize};

use crate::user_context::FocusLevel;

/// An Activity that overlapped recorded speech long enough to count as a
/// conversation. There is no conversations table: this is a read-time join of
/// `user_context_activities` against `speaker_turns` (via `audio_segments` for
/// the absolute clock — turn offsets are segment-relative).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationCluster {
    pub activity_id: i64,
    pub title: String,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
    /// Total overlap between the Activity window and speaker turns, summed
    /// across turns. Always at least the minimum-overlap threshold the store
    /// applied, so a stray one-line utterance during focused work is not a
    /// "conversation".
    pub spoken_ms: i64,
    /// Distinct speaker clusters heard inside the Activity window. `1` is a
    /// perfectly valid conversation (a call where only the user was diarized,
    /// or a monologue).
    pub speaker_count: i64,
}

/// A headline frame of an Activity — the picture that stands for that stretch
/// of the day. Ordered by the Activity's focus band first, then by how long the
/// Activity lasted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Moment {
    pub frame_id: i64,
    /// On-disk path of the captured frame; the frontend renders it through
    /// `convertFileSrc`.
    pub file_path: String,
    pub captured_at_ms: i64,
    pub activity_id: i64,
    pub title: String,
    /// Effective focus band (a user correction wins over the engine label);
    /// `None` when the Activity was never labelled.
    pub focus: Option<FocusLevel>,
    pub activity_started_at_ms: i64,
    pub activity_ended_at_ms: i64,
    /// `activity_ended_at_ms - activity_started_at_ms`, precomputed because it
    /// is also the secondary sort key.
    pub duration_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn highlights_round_trip_and_camel_case_keys() {
        let cluster = ConversationCluster {
            activity_id: 7,
            title: "Standup".to_string(),
            started_at_ms: 1_000,
            ended_at_ms: 400_000,
            spoken_ms: 180_000,
            speaker_count: 3,
        };
        let value = serde_json::to_value(&cluster).unwrap();
        assert_eq!(
            value,
            json!({
                "activityId": 7,
                "title": "Standup",
                "startedAtMs": 1_000,
                "endedAtMs": 400_000,
                "spokenMs": 180_000,
                "speakerCount": 3,
            })
        );
        assert_eq!(
            serde_json::from_value::<ConversationCluster>(value).unwrap(),
            cluster
        );

        let moment = Moment {
            frame_id: 42,
            file_path: "/frames/a.jpg".to_string(),
            captured_at_ms: 2_000,
            activity_id: 7,
            title: "Standup".to_string(),
            focus: Some(FocusLevel::Deep),
            activity_started_at_ms: 1_000,
            activity_ended_at_ms: 400_000,
            duration_ms: 399_000,
        };
        let value = serde_json::to_value(&moment).unwrap();
        assert_eq!(
            value,
            json!({
                "frameId": 42,
                "filePath": "/frames/a.jpg",
                "capturedAtMs": 2_000,
                "activityId": 7,
                "title": "Standup",
                "focus": "deep",
                "activityStartedAtMs": 1_000,
                "activityEndedAtMs": 400_000,
                "durationMs": 399_000,
            })
        );
        assert_eq!(serde_json::from_value::<Moment>(value).unwrap(), moment);
    }
}
