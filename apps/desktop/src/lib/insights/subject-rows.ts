// subject-rows.ts — the Subjects index's row projection. Pure, no Svelte, no
// I/O, `import type` only, so it is unit-testable under `bun test`.
//
// A "Subject" is NOT a backend row: it is a client-side group-by over
// `Conclusion.subject`. Everything a row may claim is derived here, and nothing
// it may not claim exists in this file — there is no rolled-up subject score,
// and no evidence count (the index never resolves evidence refs).
import type { Conclusion } from "$lib/types/recording";
import { deriveTrend, type TierSubject, type Trend } from "./subjectsTiers";

// Sparkline lines encode MAGNITUDE, not identity: the top conclusion draws in
// accent and the rest share one neutral grey, so a subject with many
// conclusions never fans out into a hard-to-decode rainbow of hues. A wholly
// faded subject keeps its plate but loses its colour — every line grey.
export const SPARK_LEAD = "--app-accent";
export const SPARK_REST = "--chart-grey-3";

export interface SubjectSpark {
  colorVar: string;
  faded: boolean;
  points: number[];
}

/** A row on the Subjects index. Satisfies `TierSubject`, so the tiering helpers
 *  group it directly without a separate projection. */
export interface SubjectRow extends TierSubject {
  subject: string;
  conclusions: Conclusion[];
  conclusionCount: number;
  /** Faded conclusions inside a still-active subject ("1 below floor"). */
  belowFloorCount: number;
  pinned: boolean;
  faded: boolean; // every conclusion faded
  headline: string; // the top (highest-confidence) conclusion's statement
  lastMovedAtMs: number; // newest updated/last-supported across conclusions
  trend: Trend;
  spark: SubjectSpark[];
  /** The TOP CONCLUSION's confidence — never a rolled-up subject score. */
  topConfidence: number;
}

export function groupSubjects(list: Conclusion[]): Map<string, Conclusion[]> {
  const groups = new Map<string, Conclusion[]>();
  for (const c of list) {
    const bucket = groups.get(c.subject);
    if (bucket) bucket.push(c);
    else groups.set(c.subject, [c]);
  }
  return groups;
}

/** One sparkline series per conclusion, confidence-desc order. Prefers the real
 *  history; falls back to a flat baseline at the current confidence. Points are
 *  evenly spaced BY INDEX — the backend stores a confidence history, not a time
 *  series, so the chart has no time axis. */
export function buildSpark(
  cs: Conclusion[],
  history: Map<number, number[]> | undefined,
  subjectFaded: boolean,
): SubjectSpark[] {
  return cs.map((c, i) => {
    const pts = history?.get(c.id);
    // A polyline needs >= 2 points to draw a visible segment. A single snapshot
    // (one history point, or none) would render an invisible line, so flatten
    // it into a 2-point baseline at that confidence.
    const points =
      pts && pts.length >= 2
        ? pts
        : pts && pts.length === 1
          ? [pts[0], pts[0]]
          : [c.confidence, c.confidence];
    return {
      colorVar: i === 0 && !subjectFaded ? SPARK_LEAD : SPARK_REST,
      faded: c.status === "faded",
      points,
    };
  });
}

/** Group conclusions into rows. `trajectories` maps subject → (conclusionId →
 *  oldest-first confidence points); a subject missing from it keeps flat
 *  baselines and a status-inferred trend. */
export function buildSubjectRows(
  conclusions: Conclusion[],
  trajectories: Map<string, Map<number, number[]>>,
): SubjectRow[] {
  const out: SubjectRow[] = [];
  for (const [subject, cs] of groupSubjects(conclusions)) {
    const history = trajectories.get(subject);
    const sorted = [...cs].sort((a, b) => b.confidence - a.confidence);
    const top = sorted[0];
    const faded = cs.every((c) => c.status === "faded");
    out.push({
      subject,
      conclusions: sorted,
      conclusionCount: cs.length,
      belowFloorCount: faded ? 0 : cs.filter((c) => c.status === "faded").length,
      pinned: cs.some((c) => c.pinned),
      faded,
      headline: top?.statement ?? subject,
      lastMovedAtMs: cs.reduce(
        (acc, c) => Math.max(acc, c.updatedAtMs, c.lastSupportedAtMs),
        0,
      ),
      trend: deriveTrend(cs, history),
      spark: buildSpark(sorted, history, faded),
      topConfidence: top?.confidence ?? 0,
    });
  }
  return out;
}

/** The ONE display ordering: active first by top confidence desc, faded sunk to
 *  the bottom, ties by name. Feeds the flat list, the tiers and the realtime
 *  diff alike, so "what the user sees" has a single definition. Returns a new
 *  array; does not mutate the input. */
export function sortDisplayRows<T extends { subject: string; faded: boolean; topConfidence: number }>(
  rows: T[],
): T[] {
  return [...rows].sort(
    (a, b) =>
      Number(a.faded) - Number(b.faded) ||
      b.topConfidence - a.topConfidence ||
      a.subject.localeCompare(b.subject),
  );
}

/** Subject names in display order for an arbitrary conclusions list — used to
 *  diff a staged reload against what is on screen, in the SAME order. */
export function displayedSubjectOrder(list: Conclusion[]): string[] {
  const summaries: { subject: string; faded: boolean; topConfidence: number }[] = [];
  for (const [subject, cs] of groupSubjects(list)) {
    const top = [...cs].sort((a, b) => b.confidence - a.confidence)[0];
    summaries.push({
      subject,
      faded: cs.every((c) => c.status === "faded"),
      topConfidence: top?.confidence ?? 0,
    });
  }
  return sortDisplayRows(summaries).map((s) => s.subject);
}

// ---- Row copy -------------------------------------------------------------

export const TREND_GLYPH: Record<Trend, string> = {
  up: "▲",
  steady: "–",
  down: "▼",
};

export function trendClass(t: Trend): string {
  return t === "up" ? "tr-up" : t === "down" ? "tr-dn" : "tr-st";
}
export function trendWord(t: Trend): string {
  return t === "up" ? "warming" : t === "down" ? "cooling" : "steady";
}

export function countLabel(n: number): string {
  return `${n} ${n === 1 ? "conclusion" : "conclusions"}`;
}

/** The row's meta line — a conclusion count, plus "faded" when the whole
 *  subject is below the floor. Built here rather than in the template so the
 *  separator's spacing can't be eaten by Svelte's whitespace trimming. */
export function metaLabel(r: { conclusionCount: number; faded: boolean }): string {
  const count = countLabel(r.conclusionCount);
  return r.faded ? `${count} · faded` : count;
}
