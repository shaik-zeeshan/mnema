// subjectsTiers.ts — pure tiering/grouping/trend helpers for the Subjects
// "Conviction view". No Svelte, no I/O — just data transforms so they can be
// unit-tested in isolation and reused across the Subjects sub-surfaces.
//
// Threshold constants — mirror crates/app-infra/src/user_context/confidence.rs.
// Keep in sync if engine policy changes.
export const DISPLAY_FLOOR = 0.15; // confidence.rs DISPLAY_FLOOR — faded boundary
export const INITIAL_BASE = 0.3; // confidence.rs INITIAL_BASE — "just taking shape" ceiling
export const STRONGLY_HELD = 0.68; // component-local: no engine constant for this boundary; tune here
export const SPARSE_LIMIT = 5; // below this many subjects, render one flat list (no tiers)

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

export function convictionTierId(
  s: TierSubject,
): "strong" | "forming" | "shaping" | "fading" {
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

export function tierFor(s: TierSubject, axis: Axis): string {
  return axis === "conviction" ? convictionTierId(s) : movementTierId(s);
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

/** Honest summary counts for the header line — no rolled-up score. `active`
 *  and `fading` partition the whole set by faded; warming/steady/cooling count
 *  trend WITHIN the active subjects only (faded subjects never count toward a
 *  movement bucket). Mirrors the mockup's renderSummary(). */
export function summaryCounts(
  rows: { faded: boolean; trend: Trend }[],
): {
  active: number;
  fading: number;
  warming: number;
  steady: number;
  cooling: number;
} {
  let active = 0;
  let fading = 0;
  let warming = 0;
  let steady = 0;
  let cooling = 0;
  for (const r of rows) {
    if (r.faded) {
      fading += 1;
      continue;
    }
    active += 1;
    if (r.trend === "up") warming += 1;
    else if (r.trend === "down") cooling += 1;
    else steady += 1;
  }
  return { active, fading, warming, steady, cooling };
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
  note: "below display floor",
  faded: true,
};

const CONVICTION_TIERS: TierMeta[] = [
  { id: "strong", title: "Strongly held", note: "held firmly", faded: false },
  { id: "forming", title: "Forming", note: "building support", faded: false },
  { id: "shaping", title: "Just taking shape", note: "early", faded: false },
  FADING_META,
];

const MOVEMENT_TIERS: TierMeta[] = [
  { id: "warming", title: "Warming", note: "▲ gaining support", faded: false },
  { id: "steady", title: "Steady", note: "holding", faded: false },
  { id: "cooling", title: "Cooling", note: "▼ losing support", faded: false },
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
