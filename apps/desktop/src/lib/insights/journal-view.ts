// Presentation helpers for DayTimeline (Journal, Slice 3). Pure + bun-testable
// so the river-merge / banding / reason-copy logic stays out of the .svelte
// component (keeps it under the 800-line ceiling) and has a runnable check.

import type { JournalCardSlot, JournalGap } from "./journal-day";

/** One row of the river, chronological — either an activity card or an away-gap. */
export type RiverRow =
  | { kind: "card"; slot: JournalCardSlot; atMs: number }
  | { kind: "gap"; gap: JournalGap; atMs: number };

export type BandLabel = "Morning" | "Afternoon" | "Evening";

/** A run of consecutive river rows that share a time-of-day band. */
export interface RiverBand {
  label: BandLabel;
  rows: RiverRow[];
}

/** Merge activity slots and away-gaps into one chronological river (oldest first). */
export function buildRiver(
  slots: JournalCardSlot[],
  gaps: JournalGap[],
): RiverRow[] {
  const rows: RiverRow[] = [
    ...slots.map(
      (slot): RiverRow => ({ kind: "card", slot, atMs: slot.activity.startedAtMs }),
    ),
    ...gaps.map((gap): RiverRow => ({ kind: "gap", gap, atMs: gap.startMs })),
  ];
  rows.sort((a, b) => a.atMs - b.atMs);
  return rows;
}

/**
 * Activities shorter than this render as compact one-line rows instead of full
 * cards. 5 minutes — deliberately the same magnitude as `AWAY_GAP_MIN_MS` and
 * the capture segment cap.
 */
export const SHORT_ACTIVITY_MAX_MS = 300_000;

/** True when the activity's duration is under `SHORT_ACTIVITY_MAX_MS`. */
export function isShortActivity(a: { startedAtMs: number; endedAtMs: number }): boolean {
  return a.endedAtMs - a.startedAtMs < SHORT_ACTIVITY_MAX_MS;
}

/** Local time-of-day band for a timestamp: <12 Morning, <17 Afternoon, else Evening. */
export function bandOf(ms: number): BandLabel {
  const hour = new Date(ms).getHours();
  if (hour < 12) return "Morning";
  if (hour < 17) return "Afternoon";
  return "Evening";
}

/** Group a chronological river into consecutive Morning/Afternoon/Evening bands. */
export function bandRiver(rows: RiverRow[]): RiverBand[] {
  const bands: RiverBand[] = [];
  for (const row of rows) {
    const label = bandOf(row.atMs);
    const last = bands[bands.length - 1];
    if (last && last.label === label) last.rows.push(row);
    else bands.push({ label, rows: [row] });
  }
  return bands;
}

/**
 * Human copy for the pending slot when the engine is unavailable. Codes mirror
 * `AiRuntimeStatus.reason` / `UserContextStatus.reason` (see
 * `settings/state/ai-runtime.svelte.ts`'s `aiRuntimeReasonLabel`), reshaped into
 * "Summaries are paused — <why>" sentences with a safe generic default.
 */
export function pendingReasonCopy(reason: string): string {
  if (reason.startsWith("no_provider_key:")) {
    return "Summaries are paused — no API key is saved for the engine's provider.";
  }
  if (reason.startsWith("provider_not_connected:")) {
    return "Summaries are paused — the engine's default provider isn't connected.";
  }
  switch (reason) {
    case "user_context_disabled":
      return "Summaries are paused — continuous derivation is turned off.";
    case "ai_runtime_disabled":
      return "Summaries are paused — AI features are turned off.";
    case "no_providers":
      return "Summaries are paused — no AI provider is connected yet.";
    case "no_default_model":
      return "Summaries are paused — no default model is chosen.";
    case "no_base_url":
      return "Summaries are paused — the local provider needs a base URL.";
    case "local_endpoint_unreachable":
      return "Summaries are paused — the local engine can't be reached.";
    default:
      return "Summaries are paused — the Reasoning Engine isn't available right now.";
  }
}
