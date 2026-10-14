// The one place wall-clock times are formatted. Every clock time in the app
// ("10:20" / "10:20 AM") follows the Time format setting: `h12`, `h24`, or
// `system`, which follows the Mac's own "24-hour time" toggle. The Mac's cycle
// is resolved natively (`system_uses_24_hour_time`) because WKWebView's `Intl`
// ignores that override; until it answers (or off macOS) `Intl` stands in.
//
// Durations, relative ages, dates and video/transcript offsets are NOT clock
// times and stay with their own formatters.
//
// Plain `.ts` (not `.svelte.ts`) so the many plain helper modules that format
// clocks stay importable from `bun test`; `createSubscriber` still makes every
// read reactive inside components/effects, so a setting change re-renders.

import { invoke } from "@tauri-apps/api/core";
import { createSubscriber } from "svelte/reactivity";
import type { TimeFormatSetting } from "$lib/types";

let pref: TimeFormatSetting = "system";
let system24: boolean | null = null;
let notify = (): void => {};
const track = createSubscriber((update) => {
  notify = update;
  return () => {
    notify = () => {};
  };
});

function intl24(): boolean {
  try {
    const hc = new Intl.DateTimeFormat(undefined, { hour: "numeric" }).resolvedOptions().hourCycle;
    return hc === "h23" || hc === "h24";
  } catch {
    return false;
  }
}

export function setTimeFormat(next: TimeFormatSetting | undefined | null): void {
  const value = next ?? "system";
  if (value === pref) return;
  pref = value;
  notify();
}

let resolving: Promise<void> | null = null;
/** Ask the OS once per window for its hour cycle (macOS; `null` elsewhere). */
export function resolveSystemHourCycle(): Promise<void> {
  resolving ??= invoke<boolean | null>("system_uses_24_hour_time")
    .then((value) => {
      if (value !== system24) {
        system24 = value;
        notify();
      }
    })
    .catch(() => {});
  return resolving;
}

/** Whether clock times currently render 24-hour. Reactive. */
export function uses24h(): boolean {
  track();
  if (pref !== "system") return pref === "h24";
  return system24 ?? intl24();
}

/** For `toLocale*String` calls that mix a clock with date parts. */
export function hourCycle(): "h23" | "h12" {
  return uses24h() ? "h23" : "h12";
}

const pad = (n: number) => String(n).padStart(2, "0");

interface ClockOptions {
  seconds?: boolean;
  /** Override the setting (tests, or a caller that already resolved it). */
  h24?: boolean;
}

function toDate(at: Date | number): Date {
  return typeof at === "number" ? new Date(at) : at;
}

/** "14:05" / "2:05 PM" (+ ":09" with `seconds`). "" for an invalid time. */
export function formatClock(at: Date | number, opts: ClockOptions = {}): string {
  const d = toDate(at);
  if (isNaN(d.getTime())) return "";
  const h = d.getHours();
  const tail = `${pad(d.getMinutes())}${opts.seconds ? `:${pad(d.getSeconds())}` : ""}`;
  if (opts.h24 ?? uses24h()) return `${pad(h)}:${tail}`;
  return `${h % 12 || 12}:${tail} ${h < 12 ? "AM" : "PM"}`;
}

/** "09:20–10:50" / "9:20–10:50 AM" (shared AM/PM said once) / "11:30 AM–1:05 PM". */
export function formatClockRange(
  start: Date | number,
  end: Date | number,
  opts: ClockOptions & { sep?: string } = {},
): string {
  const sep = opts.sep ?? "–";
  const a = formatClock(start, opts);
  const b = formatClock(end, opts);
  const meridiem = / [AP]M$/;
  const ma = a.match(meridiem)?.[0];
  if (ma && ma === b.match(meridiem)?.[0]) return `${a.slice(0, -ma.length)}${sep}${b}`;
  return `${a}${sep}${b}`;
}

/** Hour-only axis/list label: "08" / "8 AM"; `hour` is 0–24 (24 = midnight). */
export function formatHour(hour: number, opts: { h24?: boolean } = {}): string {
  const h = ((hour % 24) + 24) % 24;
  if (opts.h24 ?? uses24h()) return pad(h);
  return `${h % 12 || 12} ${h < 12 ? "AM" : "PM"}`;
}

// "14:05" / "9:20" with nothing that marks it as something else: not glued to a
// word, digit, `.`, `:` or `/` ("v1:31", "3/1:31", "1:31:05"), minutes 00–59,
// not already followed by AM/PM.
const CLOCK_24 = /(?<![\w.:/])(\d{1,2}):([0-5]\d)(?![\d:]|\s*[AaPp]\.?[Mm](?!\w))/g;
// "2 PM" / "2:05 pm" / "2 p.m." (a trailing period stays — it may end the sentence).
const CLOCK_12 = /(?<![\w.:/])(\d{1,2})(?::([0-5]\d))?\s*([AaPp])\.?[Mm](?!\w)/g;

/**
 * Re-spell the clock times inside engine-written prose (digests are written
 * with the 24-hour labels Mnema feeds the model, and the model sometimes says
 * "2 PM") to the Time format setting. Applied at render, so stored text and a
 * setting change both follow without regenerating.
 *
 * ponytail: a regex, not a parser. Its ceiling: a bare "H:MM" in prose that is
 * not a time — a verse ("Genesis 1:31"), a ratio ("16:10"), a duration ("a
 * 1:31 clip") — reads as one and gets AM/PM in 12-hour mode; and in 24-hour
 * mode the first half of "9–11 AM" stays bare. Glued forms ("v1:31",
 * "1:31:05") and impossible times ("10:75", "25:00", "13 PM") are left alone.
 */
export function localizeClockTimes(text: string, h24: boolean = uses24h()): string {
  const clock = (h: number, m: number) => formatClock(new Date(2000, 0, 1, h, m), { h24 });
  if (!h24) {
    return text.replace(CLOCK_24, (all, h: string, m: string) => (+h > 23 ? all : clock(+h, +m)));
  }
  return text.replace(CLOCK_12, (all, h: string, m: string | undefined, ap: string) => {
    if (+h < 1 || +h > 12) return all;
    return clock((+h % 12) + (/p/i.test(ap) ? 12 : 0), +(m ?? 0));
  });
}
