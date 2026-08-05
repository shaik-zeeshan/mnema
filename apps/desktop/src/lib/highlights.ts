// Day-highlight DTOs — the frontend mirror of
// `crates/capture-types/src/highlights.rs`. These are the wire shapes of the
// `get_conversations` / `get_moments` Tauri commands, which are read-time
// queries (no conversations table, no moments table) feeding the Overview tiles
// and the timeline drawer header.
//
// Both commands take a half-open `[startMs, endMs)` window; all timestamps are
// unix milliseconds.

import type { ActivityFocus } from "./types/recording";

/** An Activity whose window overlapped recorded speech for at least two
 *  minutes — the read-time join of activities against speaker turns. */
export interface ConversationCluster {
  activityId: number;
  title: string;
  startedAtMs: number;
  endedAtMs: number;
  /** Total speech overlap inside the Activity window, summed across turns.
   *  Always >= the backend's two-minute threshold. */
  spokenMs: number;
  /** Distinct speaker clusters heard inside the window; 1 is valid. */
  speakerCount: number;
}

/** A headline frame of an Activity, ranked by focus band then Activity
 *  duration. `filePath` is an on-disk path — render it via `convertFileSrc`. */
export interface Moment {
  frameId: number;
  filePath: string;
  capturedAtMs: number;
  activityId: number;
  title: string;
  /** Effective focus band (a user correction wins); null when unlabelled. */
  focus: ActivityFocus | null;
  activityStartedAtMs: number;
  activityEndedAtMs: number;
  /** `activityEndedAtMs - activityStartedAtMs`. */
  durationMs: number;
}
