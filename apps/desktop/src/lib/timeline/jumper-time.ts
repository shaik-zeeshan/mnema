// ── Timeline Jumper — time-list pure helpers ─────────────────────────────────
// Rune-free logic for the jump picker's hourly time list, extracted out of
// `TimelineJumper.svelte` so the AM/PM label formatting, the per-hour bucket
// build (today caps at the current local hour; loaded-but-empty hours render
// disabled), and the local→Date commit ranges can be unit-tested under
// `bun test`. The component keeps only the reactive `$derived` plumbing that
// feeds these.
import { parseCapturedAt } from "$lib/format-time";
import type { FrameSummaryDto } from "$lib/types/app-infra";
import type { CalendarFields } from "./jumper-cache-core";

export type HourBucket = {
  label: string;
  hour: number;
  disabled: boolean;
  count: number;
};

/** "1:00 AM" … "12:00 PM" … "11:00 PM". 12-hour clock, noon/midnight = 12. */
export function formatHourLabel(hour: number): string {
  const period = hour < 12 ? "AM" : "PM";
  const display = hour % 12 === 0 ? 12 : hour % 12;
  return `${display}:00 ${period}`;
}

/**
 * Hourly buckets for the previewed local date.
 *   - Today caps at the current local hour; other dates render 0–23.
 *   - Counts come from the day's summaries (only once the month is loaded).
 *   - A loaded month with zero frames in an hour renders that hour disabled;
 *     pre-load (monthLoaded=false) nothing is disabled and counts are 0.
 */
export function buildHourBuckets(
  date: CalendarFields,
  now: Date,
  monthLoaded: boolean,
  daySummaries: FrameSummaryDto[] | undefined,
): HourBucket[] {
  const isToday =
    date.year === now.getFullYear() &&
    date.month === now.getMonth() + 1 &&
    date.day === now.getDate();
  const lastHour = isToday ? now.getHours() : 23;
  const counts = new Map<number, number>();
  if (monthLoaded && daySummaries) {
    for (const s of daySummaries) {
      const dt = parseCapturedAt(s.capturedAt);
      if (!isNaN(dt.getTime())) {
        const h = dt.getHours();
        counts.set(h, (counts.get(h) ?? 0) + 1);
      }
    }
  }
  const out: HourBucket[] = [];
  for (let h = 0; h <= lastHour; h++) {
    const count = counts.get(h) ?? 0;
    const disabled = monthLoaded && count === 0;
    out.push({ label: formatHourLabel(h), hour: h, disabled, count });
  }
  return out;
}

/** Full local-day range [00:00:00.000 .. 23:59:59.999] for "Latest of day". */
export function dayRange(d: CalendarFields): { start: Date; end: Date } {
  const start = new Date(d.year, d.month - 1, d.day, 0, 0, 0, 0);
  const end = new Date(d.year, d.month - 1, d.day, 23, 59, 59, 999);
  return { start, end };
}

/**
 * Range for "latest at or before the end of the picked hour": the day start
 * through [hh:59:59.999] of that hour (the backend treats the range as
 * inclusive, so we extend to the last millisecond of the hour).
 */
export function hourRange(
  d: CalendarFields,
  hour: number,
): { start: Date; end: Date } {
  const start = new Date(d.year, d.month - 1, d.day, 0, 0, 0, 0);
  const end = new Date(d.year, d.month - 1, d.day, hour, 59, 59, 999);
  return { start, end };
}
