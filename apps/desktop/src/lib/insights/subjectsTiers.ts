// subjectsTiers.ts — pure tiering/grouping/trend helpers for the Subjects
// "Conviction view". No Svelte, no I/O — just data transforms so they can be
// unit-tested in isolation and reused across the Subjects sub-surfaces.
//
import type { ConfidenceSnapshot } from "$lib/types/recording";
import { interpolateConfidence } from "./subjectTimeline";

// Threshold constants — mirror crates/app-infra/src/user_context/confidence.rs.
// Keep in sync if engine policy changes.
export const DISPLAY_FLOOR = 0.15; // confidence.rs DISPLAY_FLOOR — faded boundary
export const INITIAL_BASE = 0.3; // confidence.rs INITIAL_BASE — "just taking shape" ceiling
export const STRONGLY_HELD = 0.68; // component-local: no engine constant for this boundary; tune here
export const SPARSE_LIMIT = 5; // below this many subjects, render one flat list (no tiers)
export const FORTNIGHT_MS = 14 * 86_400_000; // conviction-map tail: where the lead sat 14 days ago

export type Axis = "conviction" | "movement";
export type Trend = "up" | "steady" | "down";

/** Minimal shape the tiering helpers operate on (a SubjectRow satisfies this). */
export interface TierSubject {
  topConfidence: number; // highest-confidence conclusion's confidence
  faded: boolean; // all conclusions faded
  trend: Trend;
  lastMovedAtMs: number;
}

export interface Tier<T> {
  id: string;
  title: string;
  note: string;
  faded: boolean; // true only for the "Fading · kept for history" tier
  items: T[];
}

/** Below SPARSE_LIMIT subjects, the caller renders one flat list (no tiers). */
export function isSparse(count: number): boolean {
  return count < SPARSE_LIMIT;
}

export type ConvictionTier = "strong" | "forming" | "shaping" | "fading";

export function convictionTierId(s: TierSubject): ConvictionTier {
  if (s.faded) return "fading";
  if (s.topConfidence >= STRONGLY_HELD) return "strong";
  if (s.topConfidence >= INITIAL_BASE) return "forming";
  return "shaping";
}

export function movementTierId(
  s: TierSubject,
): "warming" | "steady" | "cooling" | "fading" {
  if (s.faded) return "fading";
  if (s.trend === "up") return "warming";
  if (s.trend === "down") return "cooling";
  return "steady";
}

/** Derive a subject's trend from its conclusions' real trajectory history
 *  (first vs last point averaged across measured conclusions); fall back to
 *  status when no history (all-faded => "down", else "steady"). */
export function deriveTrend(
  cs: { id: number; status: string }[],
  history: Map<number, number[]> | undefined,
): Trend {
  let delta = 0;
  let measured = 0;
  for (const c of cs) {
    const pts = history?.get(c.id);
    if (pts && pts.length >= 2) {
      delta += pts[pts.length - 1] - pts[0];
      measured += 1;
    }
  }
  if (measured > 0) {
    const avg = delta / measured;
    if (avg > 0.04) return "up";
    if (avg < -0.04) return "down";
    return "steady";
  }
  // No history available — infer from status.
  const allFaded = cs.every((c) => c.status === "faded");
  if (allFaded) return "down";
  return "steady";
}

// Tier metadata, in top→bottom display order per axis. The membership test for
// each tier is the corresponding *TierId function; we build these in order so
// the caller can hide empty ones without losing the ordering.
interface TierMeta {
  id: string;
  title: string;
  note: string;
  faded: boolean;
}

const FADING_META: TierMeta = {
  id: "fading",
  title: "Fading · kept for history",
  note: "every conclusion below the 15% floor",
  faded: true,
};

const CONVICTION_TIERS: TierMeta[] = [
  { id: "strong", title: "Strongly held", note: "lead ≥ 68%", faded: false },
  { id: "forming", title: "Forming", note: "30 – 68% · building support", faded: false },
  { id: "shaping", title: "Just taking shape", note: "under 30% · early", faded: false },
  FADING_META,
];

const MOVEMENT_TIERS: TierMeta[] = [
  { id: "warming", title: "Warming", note: "gaining support", faded: false },
  { id: "steady", title: "Steady", note: "holding", faded: false },
  { id: "cooling", title: "Cooling", note: "losing support", faded: false },
  FADING_META,
];

// ---- Slice 4: realtime refresh-pill helpers (pure, testable) ----------------

/** Compare displayed vs freshly-loaded subject lists (in display order).
 *  `changed` = set membership differs OR display order differs.
 *  `count`   = size of the symmetric difference (added + removed subjects) —
 *  i.e. |displayed \ staged| + |staged \ displayed|. A pure reorder (same set)
 *  reports `changed: true` with `count: 0`. */
export function subjectsDiff(
  displayed: string[],
  staged: string[],
): { changed: boolean; count: number } {
  const displayedSet = new Set(displayed);
  const stagedSet = new Set(staged);
  let added = 0;
  for (const s of stagedSet) if (!displayedSet.has(s)) added += 1;
  let removed = 0;
  for (const s of displayedSet) if (!stagedSet.has(s)) removed += 1;
  const count = added + removed;
  const membershipChanged = count > 0;
  const orderChanged =
    displayed.length !== staged.length ||
    displayed.some((s, i) => s !== staged[i]);
  return { changed: membershipChanged || orderChanged, count };
}

/** Decide what to do with a changed staged reload.
 *  - "ignore": nothing changed.
 *  - "apply":  safe to swap in silently (no row open AND the list is at the top).
 *  - "stage":  hold the new data behind the refresh pill (a row is open, or the
 *              user has scrolled — don't yank content out from under them). */
export function decideRefresh(opts: {
  changed: boolean;
  expanded: boolean;
  atTop: boolean;
}): "apply" | "stage" | "ignore" {
  if (!opts.changed) return "ignore";
  if (!opts.expanded && opts.atTop) return "apply";
  return "stage";
}

/** Tiny debounce: collapses calls within `delayMs` into one trailing
 *  invocation, called with the LAST args. `cancel()` clears a pending call.
 *  Built on setTimeout/clearTimeout so tests can drive it with fake timers. */
export function debounce<A extends unknown[]>(
  fn: (...a: A) => void,
  delayMs: number,
): ((...a: A) => void) & { cancel(): void } {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const wrapped = (...args: A): void => {
    if (timer !== undefined) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = undefined;
      fn(...args);
    }, delayMs);
  };
  wrapped.cancel = (): void => {
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
  };
  return wrapped;
}

/** Group rows into ORDERED tiers for the axis. Order top→bottom:
 *  conviction: strong, forming, shaping, fading
 *  movement:   warming, steady, cooling, fading
 *  Within conviction tiers sort by topConfidence DESC; within movement tiers
 *  sort by lastMovedAtMs DESC. Include ALL tiers (even empty) — the caller
 *  decides to hide empty ones. */
export function buildTiers<T extends TierSubject>(
  rows: T[],
  axis: Axis,
): Tier<T>[] {
  const metas = axis === "conviction" ? CONVICTION_TIERS : MOVEMENT_TIERS;
  const tierIdOf = (s: TierSubject) =>
    axis === "conviction" ? convictionTierId(s) : movementTierId(s);

  const buckets = new Map<string, T[]>();
  for (const meta of metas) buckets.set(meta.id, []);
  for (const row of rows) {
    const bucket = buckets.get(tierIdOf(row));
    if (bucket) bucket.push(row);
  }

  const sortItems = (items: T[]): T[] =>
    axis === "conviction"
      ? [...items].sort((a, b) => b.topConfidence - a.topConfidence)
      : [...items].sort((a, b) => b.lastMovedAtMs - a.lastMovedAtMs);

  return metas.map((meta) => ({
    id: meta.id,
    title: meta.title,
    note: meta.note,
    faded: meta.faded,
    items: sortItems(buckets.get(meta.id) ?? []),
  }));
}

// ---- Lede headline + conviction-map tail (pure, testable) -------------------

/** "A and B", or "A, B and 3 more" past two names. */
export function listNames(xs: string[]): string {
  return xs.length <= 2 ? xs.join(" and ") : `${xs.slice(0, 2).join(", ")} and ${xs.length - 2} more`;
}

/** Headline template over the tiers (no LLM pass): "A is strongly held. B is
 *  forming — and C is fading into history." `tail` is the de-emphasised fading
 *  clause; empty tiers drop out, and "just taking shape" speaks only when
 *  nothing is held or forming. */
export function subjectsHeadline(names: Record<ConvictionTier, string[]>): { lead: string; tail: string } {
  const clause = (xs: string[], what: string) =>
    xs.length ? `${listNames(xs)} ${xs.length > 1 ? "are" : "is"} ${what}` : "";
  let lead = [clause(names.strong, "strongly held"), clause(names.forming, "forming")].filter(Boolean);
  if (!lead.length) lead = [clause(names.shaping, "just taking shape")].filter(Boolean);
  const fading = clause(names.fading, "fading into history");
  if (!lead.length) return { lead: fading ? `${fading}.` : "", tail: "" };
  return fading
    ? { lead: lead.join(". "), tail: `— and ${fading}.` }
    : { lead: `${lead.join(". ")}.`, tail: "" };
}

/** The lead conclusion's confidence one fortnight ago, read off its real
 *  Confidence History; null when it formed inside the fortnight ("new"). */
export function fortnightAgo(
  lead: { formedAtMs: number; confidence: number },
  history: readonly ConfidenceSnapshot[],
  nowMs: number,
): number | null {
  const then = nowMs - FORTNIGHT_MS;
  if (lead.formedAtMs > then) return null;
  return interpolateConfidence(history, then, lead.confidence);
}

// ---- Paging + "Moving this fortnight" (pure, testable) ----------------------

export const MAP_PAGE = 12; // conviction-map rows (the rest are in the paged list)
export const MOVING_LIMIT = 8; // "Moving this fortnight" rows

// Numbered paging: one pager per list, "21–40 of 138", a fixed page height.
export const SUBJECT_PAGE_SIZES = [20, 50, 100] as const; // Subjects list (tiers are dividers inside it)
export const CONCLUSION_PAGE_SIZES = [10, 20, 50] as const; // a subject's conclusions
export const EVIDENCE_PAGE = 8; // a conclusion's evidence entries

export interface Page<T> {
  visible: T[];
  page: number; // 0-based, clamped into range
  pages: number; // ≥ 1
  from: number; // index of the first visible row
  to: number; // one past the last visible row
  total: number;
}

/** The rows on page `page` (0-based, clamped) of `size` rows each. */
export function pageSlice<T>(items: readonly T[], page: number, size: number): Page<T> {
  const total = items.length;
  const pages = Math.max(1, Math.ceil(total / size));
  const p = Math.min(Math.max(0, Math.floor(page) || 0), pages - 1);
  const from = p * size;
  const to = Math.min(total, from + size);
  return { visible: items.slice(from, to), page: p, pages, from, to, total };
}

/** "21–40 of 138" (en dash, 1-based). */
export function pageLabel(p: { from: number; to: number; total: number }): string {
  return p.total === 0 ? "0 of 0" : `${p.from + 1}–${p.to} of ${p.total}`;
}

/** Page numbers to draw (0-based); null = an ellipsis gap. Always the first,
 *  the last and the current ±1 — never more than 7 entries. */
export function pageNumbers(page: number, pages: number): (number | null)[] {
  if (pages <= 7) return Array.from({ length: pages }, (_, i) => i);
  const keep = new Set([0, pages - 1, page - 1, page, page + 1]);
  if (page <= 3) [1, 2, 3, 4].forEach((i) => keep.add(i));
  if (page >= pages - 4) [pages - 5, pages - 4, pages - 3, pages - 2].forEach((i) => keep.add(i));
  const sorted = [...keep].filter((i) => i >= 0 && i < pages).sort((a, b) => a - b);
  const out: (number | null)[] = [];
  sorted.forEach((n, i) => {
    if (i > 0 && n - sorted[i - 1] > 1) out.push(null);
    out.push(n);
  });
  return out;
}

interface Mover {
  topConfidence: number;
  then: number | null; // lead's confidence 14 days ago; null = formed this fortnight
  sinceMs: number;
}

/** Whole points the lead moved this fortnight; null = formed inside it. */
export function movePts(r: Mover): number | null {
  return r.then === null ? null : Math.round(r.topConfidence * 100) - Math.round(r.then * 100);
}

/** "Moving this fortnight": real ±pts movers by size of the move, then the
 *  subjects formed inside the fortnight (newest first) to fill; flat rows drop.
 *  Capped, since on a young dossier nearly every subject "formed" this fortnight. */
export function rankMovers<T extends Mover>(rows: readonly T[], limit = MOVING_LIMIT): T[] {
  const moved = rows.filter((r) => (movePts(r) ?? 0) !== 0);
  moved.sort((a, b) => Math.abs(movePts(b) ?? 0) - Math.abs(movePts(a) ?? 0));
  const formed = rows.filter((r) => r.then === null).sort((a, b) => b.sinceMs - a.sinceMs);
  return [...moved, ...formed].slice(0, limit);
}
