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

/**
 * Minutes to ADD to UTC to reach local time (PST = -480, IST = 330).
 *
 * `getTimezoneOffset()` returns minutes BEHIND UTC (PST → +480), so it is
 * negated to get the conventional sign. Shared with the usage-charts hour
 * bucketing, which is wrong by the sub-hour part of the offset without it.
 */
export function localUtcOffsetMinutes(): number {
  return -new Date().getTimezoneOffset();
}

/** Snapshot the browser's current local offset + IANA zone. */
export function askAiClock(): AskAiClock {
  return {
    utcOffsetMinutes: localUtcOffsetMinutes(),
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
  };
}
