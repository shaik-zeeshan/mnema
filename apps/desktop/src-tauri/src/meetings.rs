//! The Meetings surface commands (Warm Paper redesign, Slice 1).
//!
//! Thin adapters over the app-infra meetings ledger
//! (`app_infra::meetings::MeetingsStore`): `list_meetings` (day-grouped, with
//! resolved recap state + speaker chips), `get_meeting` (summary + notes + the
//! speaker-turn transcript for the meeting window), `set_meeting_notes`. Wire
//! shapes live in `capture-types/src/meetings.rs`, mirrored by
//! `apps/desktop/src/lib/meetings/api.ts`.

use std::sync::Arc;

use ::app_infra::meetings::{MeetingRecapState, ResolvedMeeting};
use ::app_infra::AudioSegment;
use capture_types::{MeetingDay, MeetingDetail, MeetingSummary, MeetingTurn};
use serde::Deserialize;

use crate::app_infra::AppInfraState;
use crate::triggers::meeting_worker::{ms_from_rfc3339, rfc3339_from_ms};

/// List cap — the surface is "recent meetings", not an archive browser.
const LIST_LIMIT: u32 = 100;

/// The unattributed speaker for a transcription-fallback run (no diarized
/// cluster exists to name), matching the receipt's synthetic "Voice".
const FALLBACK_SPEAKER: &str = "Voice";

/// A time-bounded text run within one segment (segment-relative ms): either a
/// diarized speaker turn or, as a fallback, a transcription segment/word run.
struct SegmentRun {
    start_ms: i64,
    end_ms: i64,
    text: String,
    speaker: String,
}

/// A transcription result's timed runs (`segments` preferred, `words` fallback)
/// — the same stored `structured_payload_json` shape `parseTranscriptionRuns`
/// reads on the frontend (camelCase `startMs`/`endMs`/`text`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TxRun {
    start_ms: i64,
    end_ms: i64,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TxPayload {
    #[serde(default)]
    segments: Vec<TxRun>,
    #[serde(default)]
    words: Vec<TxRun>,
}

/// Clip a segment's runs to the meeting window (dropping empty text and runs
/// wholly outside the window) and lift them to absolute wall-clock turns.
fn window_turns(
    segment_start_ms: i64,
    window_start_ms: i64,
    window_end_ms: i64,
    runs: Vec<SegmentRun>,
) -> Vec<MeetingTurn> {
    runs.into_iter()
        .filter_map(|run| {
            let text = run.text.trim().to_string();
            if text.is_empty() {
                return None;
            }
            let started_at_ms = segment_start_ms + run.start_ms;
            if started_at_ms > window_end_ms || segment_start_ms + run.end_ms < window_start_ms {
                return None;
            }
            Some(MeetingTurn {
                speaker: run.speaker,
                started_at_ms,
                text,
            })
        })
        .collect()
}

/// The segment's completed transcription as synthetic runs — the fallback for a
/// segment whose diarization produced no text (speakrs found nothing, or hasn't
/// run; `list_speaker_turns_for_audio_segment` INNER-JOINs clusters, so an
/// un-clustered segment is silently empty). Without it those minutes vanish
/// from the transcript. Mirrors receipt-audio-loader's transcription fallback.
async fn transcription_fallback_runs(
    infra: &AppInfraState,
    segment: &AudioSegment,
    speaker: &str,
) -> Vec<SegmentRun> {
    let subject = ::app_infra::ProcessingSubject::audio_segment(segment.id);
    let Some(result) = infra
        .list_processing_results_for_subject(&subject)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.processor == ::app_infra::AUDIO_TRANSCRIPTION_PROCESSOR)
        .max_by_key(|r| r.id)
    else {
        return Vec::new();
    };
    let timed = result
        .structured_payload_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<TxPayload>(json).ok())
        .map(|p| if !p.segments.is_empty() { p.segments } else { p.words })
        .unwrap_or_default();
    if !timed.is_empty() {
        return timed
            .into_iter()
            .map(|r| SegmentRun {
                start_ms: r.start_ms,
                end_ms: r.end_ms,
                text: r.text,
                speaker: speaker.to_string(),
            })
            .collect();
    }
    // No timed runs but plain text exists: one whole-segment run keeps it shown.
    match result.result_text.filter(|t| !t.trim().is_empty()) {
        Some(text) => {
            let duration_ms = ms_from_rfc3339(&segment.ended_at)
                .zip(ms_from_rfc3339(&segment.started_at))
                .map(|(end, start)| (end - start).max(0))
                .unwrap_or(0);
            vec![SegmentRun {
                start_ms: 0,
                end_ms: duration_ms,
                text,
                speaker: speaker.to_string(),
            }]
        }
        None => Vec::new(),
    }
}

fn state_fields(state: &MeetingRecapState) -> (&'static str, Option<String>, Option<String>) {
    match state {
        MeetingRecapState::RecapReady { conversation_id } => {
            ("recap", Some(conversation_id.clone()), None)
        }
        MeetingRecapState::Processing => ("processing", None, None),
        MeetingRecapState::Skipped { reason } => ("skipped", None, reason.clone()),
        MeetingRecapState::TranscriptOnly => ("transcript_only", None, None),
    }
}

async fn summary_for(infra: &AppInfraState, meeting: &ResolvedMeeting) -> MeetingSummary {
    let (state, conversation_id, reason) = state_fields(&meeting.state);
    // Best-effort: a failed speaker read degrades to no chips, never an error.
    let speakers = infra
        .meetings()
        .list_speaker_names_for_range(
            &rfc3339_from_ms(meeting.record.start_ms),
            &rfc3339_from_ms(meeting.record.end_ms),
        )
        .await
        .unwrap_or_default();
    MeetingSummary {
        id: meeting.record.id.clone(),
        app_display_name: meeting.record.app_display_name.clone(),
        bundle_id: meeting.record.bundle_id.clone(),
        meeting_url: meeting.record.meeting_url.clone(),
        start_ms: meeting.record.start_ms,
        end_ms: meeting.record.end_ms,
        state: state.to_string(),
        conversation_id,
        reason,
        speakers,
    }
}

/// Recent detected meetings, newest first, grouped by local calendar day.
/// `offset_minutes` is the frontend's UTC offset (the Ask AI clock convention).
#[tauri::command]
pub async fn list_meetings(
    state: tauri::State<'_, AppInfraState>,
    offset_minutes: i32,
) -> Result<Vec<MeetingDay>, String> {
    let infra = Arc::clone(&*state);
    let meetings = infra
        .meetings()
        .list_meetings(LIST_LIMIT)
        .await
        .map_err(|error| format!("failed to read the meetings ledger: {error}"))?;
    let grouped = ::app_infra::meetings::group_meetings_by_local_day(meetings, offset_minutes);
    let mut days = Vec::with_capacity(grouped.len());
    for (day, meetings) in grouped {
        let mut summaries = Vec::with_capacity(meetings.len());
        for meeting in &meetings {
            // ponytail: one speaker query per meeting, bounded by LIST_LIMIT;
            // batch into one range query if list latency ever matters.
            summaries.push(summary_for(&infra, meeting).await);
        }
        days.push(MeetingDay {
            day,
            meetings: summaries,
        });
    }
    Ok(days)
}

/// One meeting: summary + the user's notes + the transcript for the meeting
/// window. BOTH audio families, interleaved by wall-clock: the mic carries the
/// user's own side, the remote participants arrive on system audio. Both are
/// diarized (system audio reaches speaker analysis via speech-activity →
/// transcription → speaker_analysis), so every voice gets a speaker label.
#[tauri::command]
pub async fn get_meeting(
    state: tauri::State<'_, AppInfraState>,
    meeting_id: String,
) -> Result<MeetingDetail, String> {
    let infra = Arc::clone(&*state);
    let meeting = infra
        .meetings()
        .get_meeting(&meeting_id)
        .await
        .map_err(|error| format!("failed to read the meetings ledger: {error}"))?
        .ok_or_else(|| "this meeting no longer exists".to_string())?;
    let summary = summary_for(&infra, &meeting).await;

    // Saved person names win over diarization labels, matching the list chips.
    let person_names: std::collections::HashMap<i64, String> = infra
        .list_person_profiles()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|person| (person.id, person.display_name))
        .collect();

    let start_rfc3339 = rfc3339_from_ms(meeting.record.start_ms);
    let end_rfc3339 = rfc3339_from_ms(meeting.record.end_ms);
    // BOTH families: the mic is only the user's own side of a meeting — the
    // remote participants arrive on the system-audio stream. A mic-only
    // transcript shows ~one voice and silently drops everyone else.
    let segments = infra
        .list_audio_segments_overlapping_range(&start_rfc3339, &end_rfc3339, None, None)
        .await
        .map_err(|error| format!("failed to read audio segments: {error}"))?;

    let mut turns = Vec::new();
    for segment in &segments {
        let Some(segment_start_ms) = ms_from_rfc3339(&segment.started_at) else {
            continue;
        };
        // Both families diarize: system audio reaches speaker analysis through
        // speech-activity → transcription → speaker_analysis (processing/store),
        // so remote participants get real speaker labels too. Treat them alike.
        // Best-effort per segment: one flaky read never blanks the transcript.
        let diarized: Vec<SegmentRun> = infra
            .list_speaker_turns_for_audio_segment(segment.id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|turn| {
                let speaker = turn
                    .person_id
                    .and_then(|person_id| person_names.get(&person_id).cloned())
                    .unwrap_or(turn.speaker_label);
                SegmentRun {
                    start_ms: turn.start_ms as i64,
                    end_ms: turn.end_ms as i64,
                    text: turn.transcript_text.unwrap_or_default(),
                    speaker,
                }
            })
            .collect();
        // Fall back to the raw transcription when diarization yielded no text for
        // this segment — otherwise its minutes silently drop from the transcript.
        let runs = if diarized.iter().any(|r| !r.text.trim().is_empty()) {
            diarized
        } else {
            transcription_fallback_runs(&infra, segment, FALLBACK_SPEAKER).await
        };
        turns.extend(window_turns(
            segment_start_ms,
            meeting.record.start_ms,
            meeting.record.end_ms,
            runs,
        ));
    }
    // Interleave mic + system-audio turns into one chronological conversation.
    turns.sort_by_key(|turn| turn.started_at_ms);

    // The recap markdown owns the checklist item set; this is only which items
    // the user ticked. A malformed stored value degrades to "nothing ticked".
    let checklist = meeting
        .record
        .checklist_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
        .unwrap_or_default();

    Ok(MeetingDetail {
        summary,
        notes: meeting.record.notes.clone(),
        turns,
        checklist,
    })
}

/// Persist the recap-checklist items the user has ticked (keyed by item text);
/// an empty list clears the stored state.
#[tauri::command]
pub async fn set_meeting_checklist(
    state: tauri::State<'_, AppInfraState>,
    meeting_id: String,
    items: Vec<String>,
) -> Result<(), String> {
    let infra = Arc::clone(&*state);
    let json = if items.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&items).map_err(|e| format!("failed to encode checklist: {e}"))?)
    };
    infra
        .meetings()
        .set_checklist_json(&meeting_id, json.as_deref())
        .await
        .map_err(|error| format!("failed to save meeting checklist: {error}"))
}

/// Save (or clear, with `null`) the user's notes text on a meeting row.
#[tauri::command]
pub async fn set_meeting_notes(
    state: tauri::State<'_, AppInfraState>,
    meeting_id: String,
    notes: Option<String>,
) -> Result<(), String> {
    let infra = Arc::clone(&*state);
    infra
        .meetings()
        .set_notes(&meeting_id, notes.as_deref())
        .await
        .map_err(|error| format!("failed to save meeting notes: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(start_ms: i64, end_ms: i64, text: &str) -> SegmentRun {
        SegmentRun {
            start_ms,
            end_ms,
            text: text.to_string(),
            speaker: "S1".to_string(),
        }
    }

    #[test]
    fn window_turns_lifts_to_wall_clock_and_clips_the_window() {
        // Segment starts at 1000; window is [1500, 3000].
        let turns = window_turns(
            1000,
            1500,
            3000,
            vec![
                run(0, 400, "before window"),   // ends at 1400 < 1500 → dropped
                run(600, 900, "in window"),     // 1600..1900 → kept
                run(1000, 1200, "   "),         // empty text → dropped
                run(2500, 2800, "still in"),    // 3500..3800, starts 3500 > 3000 → dropped
            ],
        );
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].text, "in window");
        assert_eq!(turns[0].started_at_ms, 1600);
    }
}
