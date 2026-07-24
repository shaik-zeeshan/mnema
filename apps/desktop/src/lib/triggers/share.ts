// Trigger JSON Share / Import (issue #184) — pure and dependency-free so it
// runs under `bun test` (the onboarding-privacy-sync pattern).
//
// The shareable form is the stored wire shape minus the machine-local fields
// (id, enabled): { version, name, condition, prompt, cooldownMinutes? }. It
// never carries provider/model config (docs/triggers/CONTEXT.md).
import type {
  ScheduleWeekday,
  TriggerCondition,
  TriggerDefinition,
  TriggerDraft,
} from "./api";

export const TRIGGER_JSON_VERSION = 1;

export function shareTriggerJson(trigger: TriggerDefinition): string {
  return JSON.stringify(
    {
      version: TRIGGER_JSON_VERSION,
      name: trigger.name,
      condition: trigger.condition,
      prompt: trigger.prompt,
      ...(trigger.cooldownMinutes !== undefined
        ? { cooldownMinutes: trigger.cooldownMinutes }
        : {}),
    },
    null,
    2,
  );
}

export type ParseTriggerResult =
  | { ok: true; draft: TriggerDraft }
  | { ok: false; error: string };

const WEEKDAYS: readonly ScheduleWeekday[] = [
  "monday",
  "tuesday",
  "wednesday",
  "thursday",
  "friday",
  "saturday",
  "sunday",
];

function fail(error: string): ParseTriggerResult {
  return { ok: false, error };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Optional minute fields must be whole non-negative numbers (Rust u32). */
function badMinutes(value: unknown): boolean {
  return (
    value !== undefined &&
    (typeof value !== "number" || !Number.isInteger(value) || value < 0)
  );
}

/**
 * Parse pasted Trigger JSON into a wizard prefill. Strict on purpose: every
 * rejection names the problem (shown inline next to Import), and the returned
 * draft is rebuilt field-by-field so unknown junk never reaches the wizard or
 * the backend.
 */
export function parseTriggerJson(text: string): ParseTriggerResult {
  if (!text.trim()) {
    return fail("Clipboard is empty — copy a shared Trigger JSON first.");
  }
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return fail("That isn't valid JSON — copy a shared Trigger JSON and try again.");
  }
  if (!isRecord(raw)) {
    return fail("That JSON isn't a trigger — expected an object with name, condition and prompt.");
  }
  // Absent version = 1 (matches the Rust serde default on TriggerDefinition).
  const version = raw.version ?? TRIGGER_JSON_VERSION;
  if (version !== TRIGGER_JSON_VERSION) {
    return fail(
      `This trigger uses format version ${String(version)}, but this build only understands version ${TRIGGER_JSON_VERSION} — it was likely shared from a newer Mnema.`,
    );
  }
  if (typeof raw.name !== "string" || !raw.name.trim()) {
    return fail('The trigger is missing a "name".');
  }
  if (typeof raw.prompt !== "string" || !raw.prompt.trim()) {
    return fail('The trigger is missing a "prompt".');
  }
  if (badMinutes(raw.cooldownMinutes)) {
    return fail('"cooldownMinutes" must be a whole number of minutes.');
  }
  if (!isRecord(raw.condition)) {
    return fail('The trigger is missing a "condition".');
  }
  const condition = parseCondition(raw.condition);
  if (typeof condition === "string") return fail(condition);
  return {
    ok: true,
    draft: {
      name: raw.name,
      condition,
      prompt: raw.prompt,
      ...(raw.cooldownMinutes !== undefined
        ? { cooldownMinutes: raw.cooldownMinutes as number }
        : {}),
    },
  };
}

/** A clean TriggerCondition, or a readable error message. */
function parseCondition(cond: Record<string, unknown>): TriggerCondition | string {
  switch (cond.type) {
    case "meeting_ends": {
      if (badMinutes(cond.minMeetingMinutes)) {
        return '"minMeetingMinutes" must be a whole number of minutes.';
      }
      return {
        type: "meeting_ends",
        ...(cond.minMeetingMinutes !== undefined
          ? { minMeetingMinutes: cond.minMeetingMinutes as number }
          : {}),
      };
    }
    case "app_opened": {
      if (typeof cond.bundleId !== "string" || !cond.bundleId.trim()) {
        return 'An app-opened trigger needs a "bundleId".';
      }
      if (typeof cond.appName !== "string" || !cond.appName.trim()) {
        return 'An app-opened trigger needs an "appName".';
      }
      if (badMinutes(cond.awayGapMinutes)) {
        return '"awayGapMinutes" must be a whole number of minutes.';
      }
      return {
        type: "app_opened",
        bundleId: cond.bundleId,
        appName: cond.appName,
        ...(cond.awayGapMinutes !== undefined
          ? { awayGapMinutes: cond.awayGapMinutes as number }
          : {}),
      };
    }
    case "schedule": {
      if (cond.cadence !== "daily" && cond.cadence !== "weekly") {
        return 'A schedule\'s "cadence" must be "daily" or "weekly".';
      }
      if (typeof cond.time !== "string" || !isValidTime(cond.time)) {
        return 'A schedule needs a "time" like "18:30".';
      }
      if (cond.cadence === "weekly") {
        const weekdays = parseWeekdaySet(cond);
        if (typeof weekdays === "string") return weekdays;
        return {
          type: "schedule",
          cadence: "weekly",
          time: cond.time,
          weekdays,
        };
      }
      return { type: "schedule", cadence: "daily", time: cond.time };
    }
    default:
      return `Unknown condition type ${JSON.stringify(cond.type ?? null)} — expected "meeting_ends", "app_opened" or "schedule".`;
  }
}

/** A weekly schedule's weekday set — the current `weekdays` array form, with
 *  the legacy single-`weekday` string (version-1 payloads shared before
 *  multi-day) still importing as a one-day set. Returns the clean set in
 *  canonical Mon→Sun order, or a readable error message. */
function parseWeekdaySet(cond: Record<string, unknown>): ScheduleWeekday[] | string {
  const invalid =
    'A weekly schedule needs a "weekdays" list of days (e.g. ["monday","friday"]).';
  if (cond.weekdays !== undefined) {
    if (
      !Array.isArray(cond.weekdays) ||
      cond.weekdays.length === 0 ||
      !cond.weekdays.every((day) => WEEKDAYS.includes(day as ScheduleWeekday))
    ) {
      return invalid;
    }
    const selected = cond.weekdays as ScheduleWeekday[];
    return WEEKDAYS.filter((day) => selected.includes(day));
  }
  // Legacy single-weekday form.
  if (WEEKDAYS.includes(cond.weekday as ScheduleWeekday)) {
    return [cond.weekday as ScheduleWeekday];
  }
  return invalid;
}

function isValidTime(hm: string): boolean {
  const match = /^(\d{1,2}):(\d{2})$/.exec(hm.trim());
  if (!match) return false;
  return Number(match[1]) <= 23 && Number(match[2]) <= 59;
}
