// Shared category / thread helpers for the Insights surfaces. Extracted from
// Overview.svelte so a future modal component can reuse the same logic without
// duplication. Pure functions + constants only — no component state.

import type { Activity, ActivityCategory } from "$lib/types/recording";

export type { ActivityCategory };

// ── Category → colour token mapping (engine tier) ──────────────────────
export const CATEGORY_COLOR: Record<ActivityCategory, string> = {
  creating: "--cat-creating",
  communication: "--cat-communication",
  meetings: "--cat-meetings",
  research: "--cat-research",
  learning: "--cat-learning",
  organizing: "--cat-organizing",
  personal: "--cat-personal",
  entertainment: "--cat-entertainment",
};
// Stable legend ordering.
export const CATEGORY_ORDER: ActivityCategory[] = [
  "creating",
  "communication",
  "meetings",
  "research",
  "learning",
  "organizing",
  "personal",
  "entertainment",
];
export const UNCATEGORIZED_COLOR = "--chart-grey-3";

export function categoryLabel(c: ActivityCategory): string {
  return c.charAt(0).toUpperCase() + c.slice(1);
}

export function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

export function startOfHour(ms: number): number {
  const d = new Date(ms);
  d.setMinutes(0, 0, 0);
  return d.getTime();
}

// ── Humanisers ─────────────────────────────────────────────────────────
export function humanizeMs(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "0m";
  const totalMin = Math.round(ms / 60000);
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  if (h <= 0) return `${m}m`;
  if (m === 0) return `${h}h`;
  return `${h}h ${m}m`;
}
export function humanizeHours(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "0h";
  const h = ms / 3600000;
  if (h < 10) return `${(Math.round(h * 10) / 10).toString()}h`;
  return `${Math.round(h)}h`;
}

// ── Activity threads (#108 corrections) ───────────────────────────────
// The range's activities grouped by category — each thread is one line of
// "what you worked on" summarising sessions/time/days/focus, expandable to
// the raw activities (corrections sit behind a per-row "adjust"). Covers
// ALL of the range, not a newest-12 log slice.
export type ActivityFocus = "deep" | "mixed" | "distracted";
export interface ActivityThread {
  key: string; // category id, or "__uncat__"
  label: string;
  colorVar: string;
  totalMs: number;
  sessionCount: number;
  dayCount: number; // distinct local-calendar days touched
  dominantFocus: ActivityFocus | null; // most frequent non-null focus
  activities: Activity[]; // newest-first
}

export function buildActivityThreads(
  rangeActivities: Activity[],
): ActivityThread[] {
  const buckets = new Map<string, Activity[]>();
  for (const a of rangeActivities) {
    const key = a.category ?? "__uncat__";
    const list = buckets.get(key);
    if (list) list.push(a);
    else buckets.set(key, [a]);
  }
  const threads: ActivityThread[] = [];
  for (const [key, list] of buckets) {
    let totalMs = 0;
    const days = new Set<number>();
    const focusCounts = new Map<ActivityFocus, number>();
    for (const a of list) {
      totalMs += Math.max(0, a.endedAtMs - a.startedAtMs);
      days.add(startOfDay(a.startedAtMs));
      if (a.focus != null) {
        focusCounts.set(a.focus, (focusCounts.get(a.focus) ?? 0) + 1);
      }
    }
    // Dominant focus = most frequent non-null focus; ties resolve in
    // deep → mixed → distracted order so the readout stays stable.
    let dominantFocus: ActivityFocus | null = null;
    let dominantCount = 0;
    for (const f of ["deep", "mixed", "distracted"] as ActivityFocus[]) {
      const n = focusCounts.get(f) ?? 0;
      if (n > dominantCount) {
        dominantFocus = f;
        dominantCount = n;
      }
    }
    const uncat = key === "__uncat__";
    threads.push({
      key,
      label: uncat ? "Uncategorized" : categoryLabel(key as ActivityCategory),
      colorVar: uncat ? UNCATEGORIZED_COLOR : CATEGORY_COLOR[key as ActivityCategory],
      totalMs,
      sessionCount: list.length,
      dayCount: days.size,
      dominantFocus,
      activities: [...list].sort((a, b) => b.startedAtMs - a.startedAtMs),
    });
  }
  // Most time first; uncategorized ALWAYS sinks to the bottom — it's the
  // leftover bucket, not a body of work competing with named categories.
  threads.sort((a, b) => {
    const aUncat = a.key === "__uncat__" ? 1 : 0;
    const bUncat = b.key === "__uncat__" ? 1 : 0;
    return aUncat - bUncat || b.totalMs - a.totalMs;
  });
  return threads;
}

// One-line thread summary: "6 sessions · 4h 20m · across 3 days · mostly deep".
export function threadStats(
  t: ActivityThread,
  rangeMode: "day" | "week" | "month",
): string {
  const parts = [
    `${t.sessionCount} ${t.sessionCount === 1 ? "session" : "sessions"}`,
    humanizeMs(t.totalMs),
  ];
  // A Day range is trivially "across 1 day" — skip the noise.
  if (rangeMode !== "day") {
    parts.push(`across ${t.dayCount} ${t.dayCount === 1 ? "day" : "days"}`);
  }
  if (t.dominantFocus !== null) {
    // "Scattered" is the app's label for distracted.
    parts.push(
      t.dominantFocus === "distracted" ? "mostly scattered" : `mostly ${t.dominantFocus}`,
    );
  }
  return parts.join(" · ");
}

// Quiet read-only focus hint for non-editing rows ("Scattered" is the app's
// label for distracted). Category needs no hint — the thread already says it.
export function focusHint(focus: ActivityFocus): string {
  return focus === "distracted" ? "Scattered" : focus === "deep" ? "Deep" : "Mixed";
}

export const CATEGORY_OPTIONS: { value: ActivityCategory | ""; label: string }[] = [
  { value: "", label: "Uncategorized" },
  ...CATEGORY_ORDER.map((c) => ({
    value: c,
    label: categoryLabel(c),
  })),
];
export const FOCUS_OPTIONS: { value: string; label: string }[] = [
  { value: "", label: "—" },
  { value: "deep", label: "Deep" },
  { value: "mixed", label: "Mixed" },
  { value: "distracted", label: "Scattered" },
];
