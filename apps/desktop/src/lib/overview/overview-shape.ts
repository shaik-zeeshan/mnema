// ── Overview bento — the pure shaping half (rune-free, IO-free) ─────────────
// Everything here is date math, string scanning and formatting, so `bun test`
// can exercise it. Only type imports touch `$lib`; the one value import is
// relative, because `bun test` has no `$lib` alias.
//
// `overview-data.ts` is the other half: the Tauri reads that feed these.

import { dayKeyOfDate, formatCapturedHours, indexCoverage } from "../timeline/jumper-coverage";
import type { DayCoverage } from "$lib/types/app-infra";

export interface WeekDay {
  key: string;
  /** Two-letter weekday initial ("Mo") — the sparkline's only label. */
  label: string;
  coveredMs: number;
  isToday: boolean;
}

/** Half-open `[localMidnight, now)` — the window every "today" read shares. */
export function todayRange(now: Date = new Date()): { startMs: number; endMs: number } {
  const start = new Date(now);
  start.setHours(0, 0, 0, 0);
  return { startMs: start.getTime(), endMs: now.getTime() };
}

/**
 * The one open-thread sentence the daily digest already wrote (round-4 decision
 * G11). Returns null when the digest never mentioned one — the tile then says so
 * rather than inventing a thread. Deliberately a sentence scan, not an
 * extraction: v1 has no entity, no table and no second LLM call.
 */
export function openThreadSentence(narrative: string | null | undefined): string | null {
  if (!narrative) return null;
  const sentences = narrative.split(/(?<=[.!?])\s+/);
  const hit = sentences.find((s) => /open thread|still open|unresolved|left open/i.test(s));
  return hit?.trim() || null;
}

/**
 * The last seven local days ending today. A day the backend omits is a real
 * zero — `list_day_coverage` leaves empty days out, and "no recording" is
 * exactly a zero bar.
 */
export function weekFromCoverage(days: DayCoverage[], now: Date = new Date()): WeekDay[] {
  const index = indexCoverage(days);
  const todayKey = dayKeyOfDate(now);
  const out: WeekDay[] = [];
  for (let back = 6; back >= 0; back -= 1) {
    const date = new Date(now);
    date.setHours(0, 0, 0, 0);
    date.setDate(date.getDate() - back);
    const key = dayKeyOfDate(date);
    out.push({
      key,
      label: date.toLocaleDateString(undefined, { weekday: "short" }).slice(0, 2),
      coveredMs: index.get(key)?.coveredMs ?? 0,
      isToday: key === todayKey,
    });
  }
  return out;
}

/** `6:42` — the page's single display-size number. Empty below one minute, so
 *  the tile can fall back to its empty state instead of printing `0:00`. */
export function heroHours(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  if (minutes <= 0) return "";
  return `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, "0")}`;
}

/** `13:02` local, from unix millis. */
export function clockAt(ms: number): string {
  // A tile whose timestamp never arrived says nothing rather than "Invalid
  // Date" — the empty string collapses the caller's `{#if}` instead.
  if (!Number.isFinite(ms)) return "";
  return new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

/** `38m` / `1h 12m` — a conversation's spoken length, same formatter the jump
 *  menu uses so two surfaces never disagree about what "6h 42m" means. */
export function spokenLabel(ms: number): string {
  return formatCapturedHours(ms);
}

// ── Subjects tile (page 09's door) ──────────────────────────────────────────
// A "Subject" is not a backend noun — it is a client-side group-by over
// `Conclusion.subject`, the same read Subjects itself renders. The tile may
// honestly show a subject's name, its top conclusion's confidence and the
// active/fading split; it must NOT show an evidence count (the index never
// loads one).

export interface SubjectTileRow {
  name: string;
  /** The subject's top conclusion's confidence — literally the number the
   *  Subjects index shows on the same row. */
  topConfidence: number;
  fading: boolean;
}

export interface SubjectsTileSummary {
  /** Active subjects, top conviction first. */
  rows: SubjectTileRow[];
  activeCount: number;
  fadingCount: number;
}

export function subjectsTile(conclusions: { subject: string; confidence: number; status: string }[]): SubjectsTileSummary {
  const bySubject = new Map<string, { top: number; anyVisible: boolean }>();
  for (const c of conclusions) {
    if (c.status === "dismissed") continue;
    const entry = bySubject.get(c.subject) ?? { top: 0, anyVisible: false };
    entry.top = Math.max(entry.top, c.confidence);
    entry.anyVisible ||= c.status === "visible";
    bySubject.set(c.subject, entry);
  }
  const all = [...bySubject.entries()].map(([name, entry]) => ({
    name,
    topConfidence: entry.top,
    fading: !entry.anyVisible,
  }));
  const active = all.filter((s) => !s.fading).sort((a, b) => b.topConfidence - a.topConfidence);
  return {
    rows: active,
    activeCount: active.length,
    fadingCount: all.length - active.length,
  };
}
