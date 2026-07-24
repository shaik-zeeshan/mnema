// Meetings surface wire types + invoke wrappers (Warm Paper redesign, Slice 1)
// — the frontend mirror of `crates/capture-types/src/meetings.rs` (camelCase
// serde, no codegen; keep in sync with the Rust round-trip test).
import { invoke } from "@tauri-apps/api/core";

/** Resolved recap state of a detected meeting. */
export type MeetingState =
  | "recap"
  | "processing"
  | "skipped"
  | "transcript_only";

/** One detected meeting in the day-grouped list. */
export interface MeetingSummary {
  /** Stable meeting id (`meeting-<startMs>-<bundleId>`). */
  id: string;
  /** "Zoom", or "Google Meet (Arc)" for a browser meeting. */
  appDisplayName: string;
  /** Provenance: the mic-holding app's bundle id. */
  bundleId: string;
  /** The sighted meeting URL for a browser meeting (Rust skips when absent). */
  meetingUrl?: string;
  startMs: number;
  endMs: number;
  state: MeetingState;
  /** For `state === "recap"`: the trigger-run conversation to open. */
  conversationId?: string;
  /** For `state === "skipped"`: the honest reason, when known. */
  reason?: string;
  /** Diarized speaker names heard in the window (saved person names where
   *  matched, else "Speaker N" labels). */
  speakers: string[];
}

/** One local calendar day of meetings, newest day first. */
export interface MeetingDay {
  /** `YYYY-MM-DD` in the user's local time. */
  day: string;
  meetings: MeetingSummary[];
}

/** One speaker-labeled transcript turn, wall-clock ordered. */
export interface MeetingTurn {
  speaker: string;
  /** Absolute wall-clock unix-ms start of the turn. */
  startedAtMs: number;
  text: string;
}

/** The meeting detail: summary + the user's notes + the transcript turns +
 *  the recap checklist items the user has ticked (keyed by item text). */
export interface MeetingDetail {
  summary: MeetingSummary;
  notes: string | null;
  turns: MeetingTurn[];
  checklist: string[];
}

/** Recent detected meetings, newest first, grouped by local calendar day. */
export function listMeetings(offsetMinutes: number): Promise<MeetingDay[]> {
  return invoke<MeetingDay[]>("list_meetings", { offsetMinutes });
}

/** One meeting with notes and the speaker-turn transcript for its window. */
export function getMeeting(meetingId: string): Promise<MeetingDetail> {
  return invoke<MeetingDetail>("get_meeting", { meetingId });
}

/** Save (or clear, with `null`) the user's notes on a meeting. */
export function setMeetingNotes(
  meetingId: string,
  notes: string | null,
): Promise<void> {
  return invoke("set_meeting_notes", { meetingId, notes });
}

/** Persist which recap-checklist items are ticked (by item text); `[]` clears. */
export function setMeetingChecklist(
  meetingId: string,
  items: string[],
): Promise<void> {
  return invoke("set_meeting_checklist", { meetingId, items });
}
