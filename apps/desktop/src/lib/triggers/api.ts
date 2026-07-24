// Triggers management surface — wire types, invoke wrappers, starters, and
// pure formatting helpers (issue #182).
//
// The wire shapes hand-mirror `apps/desktop/src-tauri/src/triggers.rs`
// (TriggerDefinition / TriggerCondition / TriggerStatus, camelCase serde) — no
// codegen, keep them in sync with the Rust wire-pin tests.
import { invoke } from "@tauri-apps/api/core";

export type ScheduleWeekday =
  | "monday"
  | "tuesday"
  | "wednesday"
  | "thursday"
  | "friday"
  | "saturday"
  | "sunday";

export type ConditionType = "meeting_ends" | "app_opened" | "schedule";

export type TriggerCondition =
  | {
      type: "schedule";
      cadence: "daily" | "weekly";
      /** Local time-of-day, "HH:MM". */
      time: string;
      /** The selected weekday SET for `weekly` — fires on each selected day.
       *  (The legacy single-`weekday` form is import-compat only, folded into
       *  this set by the backend and by `parseTriggerJson`.) */
      weekdays?: ScheduleWeekday[];
    }
  | { type: "meeting_ends"; minMeetingMinutes?: number }
  | {
      type: "app_opened";
      bundleId: string;
      appName: string;
      awayGapMinutes?: number;
    };

export interface TriggerDefinition {
  id: string;
  name: string;
  condition: TriggerCondition;
  prompt: string;
  enabled: boolean;
  cooldownMinutes?: number;
  version: number;
}

export interface TriggerLastFiring {
  firedAtMs: number;
  outcome: "completed" | "skipped" | "failed";
  reason?: string;
  conversationId?: string;
}

export interface TriggerStatus {
  id: string;
  name: string;
  enabled: boolean;
  needsProvider: boolean;
  /** Running / Readiness-Wait (the sixth lifecycle state): the UTC-ms instant
   *  the in-flight firing started, present only while one is running. */
  runningSinceMs?: number;
  lastFiring?: TriggerLastFiring;
}

/** The wizard's create payload (`TriggerDraft` in Rust). */
export interface TriggerDraft {
  name: string;
  condition: TriggerCondition;
  prompt: string;
  cooldownMinutes?: number;
}

// ── Advanced Options defaults (docs/triggers/CONTEXT.md) ────────────────────
export const DEFAULT_MIN_MEETING_MINUTES = 5;
export const DEFAULT_AWAY_GAP_MINUTES = 30;
export const DEFAULT_COOLDOWN_MINUTES = 10;

// ── Condition sections (list grouping + wizard cards) ───────────────────────
// Condition ICONS live in `./condition-icons` (lucide components; this module
// stays icon-free so the pure share/import logic runs under plain `bun test`).
export interface ConditionSection {
  cond: ConditionType;
  title: string;
  blurb: string;
  addLabel: string;
}

export const CONDITION_SECTIONS: readonly ConditionSection[] = [
  {
    cond: "meeting_ends",
    title: "When a meeting ends",
    blurb:
      "A conferencing app held your mic for at least 5 minutes, then released it and stayed quiet ~2 minutes.",
    addLabel: "add a meeting-ends trigger",
  },
  {
    cond: "app_opened",
    title: "When an app opens",
    blurb:
      "A chosen app comes to the front after 30+ minutes away — a fresh session, not window switching.",
    addLabel: "add an app-opened trigger",
  },
  {
    cond: "schedule",
    title: "On a schedule",
    blurb: "At a fixed local time on the days you pick.",
    addLabel: "add a scheduled trigger",
  },
];

export const CONDITION_LABEL: Record<ConditionType, string> = {
  meeting_ends: "Meeting Ends",
  app_opened: "App Opened",
  schedule: "Schedule",
};

// ── Starter templates (the wizard's per-condition prompt starters) ──────────
export const STARTERS: Record<ConditionType, string> = {
  meeting_ends:
    "Write a recap of the meeting that just ended.\n\nOpen with a one-paragraph summary of what the meeting was about and where it landed. Then give a speaker-labeled rundown: for each person, the main points they raised and anything they committed to. Call out decisions that were made, and list open questions separately.\n\nUnder an \"Action items\" heading, list what I need to follow up on as a markdown checklist — one `- [ ] ` line per item, each starting with the verb.\n\nClose with short feedback for me: where I was clear, where I rambled, and anything I said I would follow up on. Plain prose otherwise.",
  app_opened:
    "Catch me up on this app.\n\nSummarize what I was doing the last time I had it open — the files, boards or documents I touched, where I left off, and anything I said I would do next. Note anything relevant that happened elsewhere since (messages, decisions) that changes what I should do here.\n\nEnd with a one-line suggestion for what to pick up first. Plain prose, short.",
  schedule:
    "Write an end-of-day review.\n\nSummarize what I worked on today across apps: the main threads of work and where each one stands. Note anything I started but did not finish, and any commitments I made in meetings or messages.\n\nClose with a short plan for tomorrow — three items at most. Plain prose, no fluff.",
};

/** The Advanced steppers visible for a condition (wizard Review step). */
export function advRows(
  cond: ConditionType,
): { key: "minlen" | "awaygap" | "cooldown"; label: string; min: number; max: number; step: number }[] {
  return [
    {
      key: "minlen" as const,
      label: "Minimum meeting length",
      min: 1,
      max: 30,
      step: 1,
      visible: cond === "meeting_ends",
    },
    {
      key: "awaygap" as const,
      label: "Away gap",
      min: 5,
      max: 120,
      step: 5,
      visible: cond === "app_opened",
    },
    { key: "cooldown" as const, label: "Cooldown", min: 0, max: 120, step: 5, visible: true },
  ].filter((row) => row.visible);
}

// ── Formatting helpers ──────────────────────────────────────────────────────

const WEEKDAY_LABEL: Record<ScheduleWeekday, string> = {
  monday: "Mondays",
  tuesday: "Tuesdays",
  wednesday: "Wednesdays",
  thursday: "Thursdays",
  friday: "Fridays",
  saturday: "Saturdays",
  sunday: "Sundays",
};

export const WEEKDAY_ORDER: readonly ScheduleWeekday[] = [
  "monday",
  "tuesday",
  "wednesday",
  "thursday",
  "friday",
  "saturday",
  "sunday",
];

const WEEKDAY_SHORT: Record<ScheduleWeekday, string> = {
  monday: "Mon",
  tuesday: "Tue",
  wednesday: "Wed",
  thursday: "Thu",
  friday: "Fri",
  saturday: "Sat",
  sunday: "Sun",
};

/** Human phrase for a weekly weekday SET (mockup: "Runs weekdays at 6:00 PM"):
 *  "every day" / "weekdays" / "weekends" / "Fridays" / "Mon, Wed, Fri". */
export function weekdaySetLabel(weekdays: readonly ScheduleWeekday[]): string {
  // Canonical Mon→Sun order, deduplicated, whatever order the set arrived in.
  const days = WEEKDAY_ORDER.filter((day) => weekdays.includes(day));
  if (days.length === 0) return "weekly";
  if (days.length === 7) return "every day";
  if (days.length === 1) return WEEKDAY_LABEL[days[0]];
  const weekend = days.filter((day) => day === "saturday" || day === "sunday");
  if (days.length === 5 && weekend.length === 0) return "weekdays";
  if (days.length === 2 && weekend.length === 2) return "weekends";
  return days.map((day) => WEEKDAY_SHORT[day]).join(", ");
}

/** "18:00" → "6:00 PM". Malformed input comes back unchanged. */
export function fmtTime(hm: string): string {
  const match = /^(\d{1,2}):(\d{2})$/.exec(hm.trim());
  if (!match) return hm;
  const h = Number(match[1]);
  if (h > 23) return hm;
  const ap = h >= 12 ? "PM" : "AM";
  return `${h % 12 || 12}:${match[2]} ${ap}`;
}

/** "every day at 6:00 PM" / "weekdays at 6:00 PM" / "Fridays at 9:00 AM". */
export function scheduleLabel(condition: Extract<TriggerCondition, { type: "schedule" }>): string {
  const days =
    condition.cadence === "daily" ? "every day" : weekdaySetLabel(condition.weekdays ?? []);
  return `${days} at ${fmtTime(condition.time)}`;
}

/** The muted condition detail beside a row name (mockup: "— Figma, after 30 min away"). */
export function conditionDetail(condition: TriggerCondition): string {
  if (condition.type === "app_opened") {
    return `— ${condition.appName}, after ${condition.awayGapMinutes ?? DEFAULT_AWAY_GAP_MINUTES} min away`;
  }
  if (condition.type === "schedule") return `— ${scheduleLabel(condition)}`;
  return "";
}

/** "today 2:14 PM" / "yesterday 9:31 AM" / "Jul 17, 4:02 PM". */
export function fmtWhen(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  const date = new Date(ms);
  const time = date.toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
  const dayStart = new Date();
  dayStart.setHours(0, 0, 0, 0);
  if (ms >= dayStart.getTime()) return `today ${time}`;
  if (ms >= dayStart.getTime() - 86_400_000) return `yesterday ${time}`;
  const day = date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  return `${day}, ${time}`;
}

// ── Trigger JSON (Share / Import) ───────────────────────────────────────────
// Pure share/import logic lives in `./share` (dependency-free, bun-tested).

// ── Invoke wrappers ─────────────────────────────────────────────────────────

export function listTriggers(): Promise<TriggerDefinition[]> {
  return invoke<TriggerDefinition[]>("list_triggers");
}

export function listTriggersStatus(): Promise<TriggerStatus[]> {
  return invoke<TriggerStatus[]>("list_triggers_status");
}

/** The per-trigger runs ledger (DESIGN.md Screen 2): recent firings, ALL
 *  outcomes, newest first, capped backend-side at 50. */
export function listTriggerFirings(triggerId: string): Promise<TriggerLastFiring[]> {
  return invoke<TriggerLastFiring[]>("list_trigger_firings", { triggerId });
}

export function createTrigger(draft: TriggerDraft): Promise<TriggerDefinition> {
  return invoke<TriggerDefinition>("create_trigger", { draft });
}

export function updateTrigger(trigger: TriggerDefinition): Promise<TriggerDefinition> {
  return invoke<TriggerDefinition>("update_trigger", { trigger });
}

export function deleteTrigger(triggerId: string): Promise<void> {
  return invoke<void>("delete_trigger", { triggerId });
}

export function triggersProviderReady(): Promise<boolean> {
  return invoke<boolean>("triggers_provider_ready");
}

/**
 * Run Again (docs/triggers/CONTEXT.md): retry a FAILED firing — a fresh sealed
 * turn re-running the persisted question in the same conversation. Resolves as
 * soon as the retry is started; the outcome lands as a new ledger row.
 */
export function runTriggerAgain(triggerId: string, conversationId: string): Promise<void> {
  return invoke<void>("run_trigger_again", {
    triggerId,
    conversationId,
    offsetMinutes: -new Date().getTimezoneOffset(),
  });
}
