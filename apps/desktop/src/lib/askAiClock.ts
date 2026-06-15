// The user's local wall-clock context, passed into every `ask_ai_start` /
// `ask_ai_followup` turn so the agent can anchor relative dates ("yesterday",
// "this morning") and translate the user's local-time phrasing into the UTC
// windows the capture broker speaks. The frontend is the SOUND source for this:
// the Rust `time` crate is built without `local-offset`, and reading the local
// offset there would be unsound under Tauri's multithreading.
export interface AskAiClock {
  /** Minutes to ADD to UTC to reach local time (PST = -480, IST = 330). */
  utcOffsetMinutes: number;
  /** IANA zone name for display, e.g. "America/Los_Angeles". */
  timeZone: string;
}

/** Snapshot the browser's current local offset + IANA zone. */
export function askAiClock(): AskAiClock {
  // `getTimezoneOffset()` returns minutes BEHIND UTC (PST → +480), so negate it
  // to get the conventional "add to UTC" offset (PST → -480).
  return {
    utcOffsetMinutes: -new Date().getTimezoneOffset(),
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
  };
}
