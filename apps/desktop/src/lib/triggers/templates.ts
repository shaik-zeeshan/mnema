// Trigger template gallery (PLAN.md Slice 6; DESIGN.md "Triggers —
// gallery-first creation"; cards mined from
// docs/mockups/unified-shell/app-match/triggers.html Frame 2).
//
// Static in-app list — no online gallery. Every template is a real
// `TriggerCondition` wire value plus a prompt prefill; picking one lands the
// wizard on Review with a removable "from template" chip.
import {
  DEFAULT_AWAY_GAP_MINUTES,
  DEFAULT_MIN_MEETING_MINUTES,
  STARTERS,
  WEEKDAY_ORDER,
  type ConditionType,
  type ScheduleWeekday,
  type TriggerCondition,
} from "./api";

/** The wizard-state fields a template prefills — pure; the wizard applies it. */
export interface TemplatePrefill {
  cond: ConditionType;
  name: string;
  prompt: string;
  appBundleId?: string;
  appName?: string;
  awayGap?: number;
  minLen?: number;
  time?: string;
  schedDays?: ScheduleWeekday[];
}

export function templatePrefill(tpl: TriggerTemplate): TemplatePrefill {
  const c = tpl.condition;
  const base = { cond: c.type, name: tpl.name, prompt: tpl.prompt };
  if (c.type === "app_opened") {
    return {
      ...base,
      appBundleId: c.bundleId,
      appName: c.appName,
      awayGap: c.awayGapMinutes ?? DEFAULT_AWAY_GAP_MINUTES,
    };
  }
  if (c.type === "schedule") {
    return {
      ...base,
      time: c.time,
      schedDays: c.cadence === "daily" ? [...WEEKDAY_ORDER] : [...(c.weekdays ?? [])],
    };
  }
  return { ...base, minLen: c.minMeetingMinutes ?? DEFAULT_MIN_MEETING_MINUTES };
}

export interface TriggerTemplate {
  id: string;
  /** Prefilled trigger name (Review's Name field). */
  name: string;
  /** Card display title where it differs from the saved name. */
  title?: string;
  /** Card body copy. */
  blurb: string;
  /** The muted one-line condition descriptor under the card. */
  condLine: string;
  condition: TriggerCondition;
  prompt: string;
}

export const TRIGGER_TEMPLATES: readonly TriggerTemplate[] = [
  {
    id: "meeting-recap",
    name: "Meeting Recap",
    blurb:
      "When a meeting ends, write a speaker-labeled recap with decisions, action items, and feedback for you.",
    condLine: "meeting ends · any conferencing app",
    condition: { type: "meeting_ends" },
    prompt: STARTERS.meeting_ends,
  },
  {
    id: "daily-digest",
    name: "Daily Digest",
    blurb:
      "Every weekday morning, summarize yesterday — the main threads of work, who you talked to, loose ends.",
    condLine: "weekdays · 8:00 AM",
    condition: {
      type: "schedule",
      cadence: "weekly",
      time: "08:00",
      weekdays: ["monday", "tuesday", "wednesday", "thursday", "friday"],
    },
    prompt:
      "Write my morning digest of yesterday.\n\nSummarize the main threads of work from yesterday: what moved, what stalled, and who I talked to. List loose ends — anything I started, promised, or was asked for that isn't finished.\n\nClose with the three things most worth picking up first today. Plain prose, short.",
  },
  {
    id: "figma-brief",
    name: "Figma Brief",
    title: "“When I open Figma, brief me”",
    blurb:
      "The moment Figma starts a fresh session, recall what you were designing and what feedback is pending.",
    condLine: "app opened · Figma · 30 min away",
    condition: { type: "app_opened", bundleId: "com.figma.Desktop", appName: "Figma" },
    prompt:
      "Brief me on my design work.\n\nSummarize what I was doing the last time I had Figma open — the files and frames I touched, where I left off, and any feedback or comments that arrived since. Note decisions from meetings or messages elsewhere that change what I should design.\n\nEnd with a one-line suggestion for what to pick up first. Plain prose, short.",
  },
  {
    id: "weekly-review",
    name: "Weekly Review",
    blurb:
      "Friday afternoon, look back over the week: shipped work, meetings, commitments, and what slipped.",
    condLine: "Fridays · 4:30 PM",
    condition: { type: "schedule", cadence: "weekly", time: "16:30", weekdays: ["friday"] },
    prompt:
      "Write my weekly review.\n\nLook back over the week: the work that shipped, the meetings that mattered, and the commitments I made. Note what slipped or went quiet, and any threads at risk of being forgotten.\n\nClose with a short plan for next week — the three most important things. Plain prose.",
  },
  {
    id: "one-on-one-prep",
    name: "1:1 Prep",
    blurb:
      "After any call, surface what each person committed to — so open items are ready before the next 1:1.",
    condLine: "meeting ends · any conferencing app",
    condition: { type: "meeting_ends" },
    prompt:
      "Prep me for my next 1:1 with each person on the call that just ended.\n\nFor every participant, list what they committed to, what I committed to them, and any open questions between us. Note anything unresolved that deserves a follow-up.\n\nKeep it short — a few bullets per person, ready to paste into my next 1:1 agenda.",
  },
  {
    id: "editor-catchup",
    name: "Editor Catch-up",
    blurb:
      "When you come back to Xcode after a long break, list where you left off and what changed elsewhere since.",
    condLine: "app opened · Xcode · 45 min away",
    condition: {
      type: "app_opened",
      bundleId: "com.apple.dt.Xcode",
      appName: "Xcode",
      awayGapMinutes: 45,
    },
    prompt:
      "Catch me up on where I left off in Xcode.\n\nSummarize what I was working on last session — the files I touched, the problem I was in the middle of, and anything I said I would do next. Note anything that changed elsewhere since (reviews, messages, decisions) that affects this work.\n\nEnd with a one-line suggestion for what to pick up first. Plain prose, short.",
  },
];
