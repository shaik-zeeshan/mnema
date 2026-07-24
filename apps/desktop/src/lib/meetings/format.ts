// Meetings surface presentation helpers — title inference, provenance, and
// wall-clock formatting shared by the list and detail views (Slice 5).
// The wire carries no meeting title (ADR 0057 mic-hold detection is
// calendar-free), so the title is derived from who was heard / where.
import type { MeetingSummary } from "./api";

/** True for diarization placeholder labels ("Speaker 3"), false for saved names. */
export function isUnknownSpeaker(name: string): boolean {
  return /^speaker \d+$/i.test(name);
}

function urlHost(url: string): string {
  try {
    return new URL(url).hostname;
  } catch {
    return url;
  }
}

/** Derived row title: named voices win, then a voice count, then the place. */
export function meetingTitle(m: MeetingSummary): string {
  const named = m.speakers.filter((s) => s !== "You" && !isUnknownSpeaker(s));
  if (named.length > 0) return `Meeting with ${named.join(", ")}`;
  const others = m.speakers.filter((s) => s !== "You").length;
  if (others > 0) return `Meeting with ${others} other${others === 1 ? "" : "s"}`;
  return `Meeting on ${m.meetingUrl ? urlHost(m.meetingUrl) : m.appDisplayName}`;
}

/** Short mono glyph for the app tile (mockup vocabulary: zm / ts / ft / ◈). */
export function appGlyph(m: MeetingSummary): string {
  if (m.meetingUrl) return "◈";
  const name = m.appDisplayName.toLowerCase();
  if (name.includes("zoom")) return "zm";
  if (name.includes("teams")) return "ts";
  if (name.includes("facetime")) return "ft";
  return name.replace(/[^a-z]/g, "").slice(0, 2) || "◉";
}

/** `detected via mic-hold · zoom.us` / `… · meet.google.com · Arc`. */
export function provenanceLabel(m: MeetingSummary): string {
  // Browser meetings carry the browser in the display name's parenthetical
  // ("Google Meet (Arc)"); the URL host is the truthful place.
  const browser = /\(([^)]+)\)$/.exec(m.appDisplayName)?.[1];
  const src = m.meetingUrl
    ? urlHost(m.meetingUrl) + (browser ? ` · ${browser}` : "")
    : m.bundleId;
  return `detected via mic-hold · ${src}`;
}

const timeFmt = new Intl.DateTimeFormat("en-US", {
  hour: "numeric",
  minute: "2-digit",
});

/** "10:00 – 10:47 AM" — the shared meridiem collapses onto the end time. */
export function timeRange(startMs: number, endMs: number): string {
  const s = timeFmt.format(new Date(startMs));
  const e = timeFmt.format(new Date(endMs));
  const sm = / (AM|PM)$/.exec(s);
  const em = / (AM|PM)$/.exec(e);
  const start = sm && em && sm[1] === em[1] ? s.slice(0, -3) : s;
  return `${start} – ${e}`;
}

/** "47m" / "1h 30m" from a millisecond span (floors at 1m). */
export function durationLabel(ms: number): string {
  const mins = Math.max(1, Math.round(ms / 60_000));
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  if (h === 0) return `${m}m`;
  return m === 0 ? `${h}h` : `${h}h ${m}m`;
}

/** Day-group heading: "Today" / "Yesterday" / weekday, plus "WED · JUL 23 2026". */
export function dayHeading(
  day: string,
  now: Date = new Date(),
): { label: string; sub: string } {
  const [y, mo, d] = day.split("-").map(Number);
  const date = new Date(y, mo - 1, d);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const diffDays = Math.round((today.getTime() - date.getTime()) / 86_400_000);
  const label =
    diffDays === 0
      ? "Today"
      : diffDays === 1
        ? "Yesterday"
        : diffDays > 1 && diffDays < 7
          ? date.toLocaleDateString("en-US", { weekday: "long" })
          : date.toLocaleDateString("en-US", { month: "short", day: "numeric" });
  const wd = date.toLocaleDateString("en-US", { weekday: "short" });
  const mon = date.toLocaleDateString("en-US", { month: "short" });
  return { label, sub: `${wd} · ${mon} ${d} ${y}`.toUpperCase() };
}

/** "3 meetings · 1h 30m" for a day group's rule line. */
export function dayTotals(meetings: MeetingSummary[]): string {
  const total = meetings.reduce((sum, m) => sum + (m.endMs - m.startMs), 0);
  const n = meetings.length;
  return `${n} meeting${n === 1 ? "" : "s"} · ${durationLabel(total)}`;
}
