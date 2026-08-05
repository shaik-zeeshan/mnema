// Pure grid model + keyboard-nav math for the Quick Look 3-up result grid
// (redesign slice 7, frame 08). Screen and audio results share ONE grid,
// merged newest-first and grouped into temporal day sections; selection moves
// in 2D over the sectioned grid. Plain TS so it's bun-testable without Svelte.
import type {
  FrameSearchResultDto,
  AudioSearchResultDto,
} from "$lib/types/app-infra";
import { parseToolDate, isSameCalendarDay } from "./query-tokens";

export const GRID_COLS = 3;
// Hard DOM ceiling for rendered cells (PLAN.md: ≤50 cells in the DOM). The
// fetch limits (24 frames + 12 audio) sit below it today; the cap is the
// invariant, not the expectation.
export const GRID_CELL_CAP = 50;

export type GridItem =
  | { kind: "frame"; frame: FrameSearchResultDto }
  | { kind: "audio"; audio: AudioSearchResultDto };

export function itemStartAt(item: GridItem): string {
  return item.kind === "frame"
    ? item.frame.groupStartAt
    : item.audio.absoluteStartAt;
}

function itemTimeMs(item: GridItem): number {
  return parseToolDate(itemStartAt(item))?.getTime() ?? 0;
}

// Merge both result kinds into one newest-first list, capped for the DOM.
export function buildGridItems(
  frames: FrameSearchResultDto[],
  audio: AudioSearchResultDto[],
  cap: number = GRID_CELL_CAP,
): GridItem[] {
  const items: GridItem[] = [
    ...frames.map((frame): GridItem => ({ kind: "frame", frame })),
    ...audio.map((audio): GridItem => ({ kind: "audio", audio })),
  ];
  items.sort((a, b) => itemTimeMs(b) - itemTimeMs(a));
  return items.slice(0, cap);
}

// One temporal section: a --t-label day line over a 3-up grid slice.
export type GridSection = { label: string; start: number; count: number };

// Frame-08 day label: "Today — Monday, August 3" / "Yesterday — …" /
// "Friday, July 31" (year appended when it isn't the current year).
export function dayLabel(d: Date, now: Date): string {
  const sameYear = d.getFullYear() === now.getFullYear();
  const long = d.toLocaleDateString(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
    ...(sameYear ? {} : { year: "numeric" }),
  });
  if (isSameCalendarDay(d, now)) return `Today — ${long}`;
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1);
  if (isSameCalendarDay(d, yesterday)) return `Yesterday — ${long}`;
  return long;
}

// Group consecutive items (already sorted newest-first) by calendar day.
export function buildGridSections(
  items: GridItem[],
  now: Date = new Date(),
): GridSection[] {
  const sections: GridSection[] = [];
  let currentDay: Date | null = null;
  for (let i = 0; i < items.length; i++) {
    const day = parseToolDate(itemStartAt(items[i]));
    if (
      currentDay !== null &&
      day !== null &&
      isSameCalendarDay(day, currentDay) &&
      sections.length > 0
    ) {
      sections[sections.length - 1].count += 1;
      continue;
    }
    sections.push({
      label: day !== null ? dayLabel(day, now) : "Earlier",
      start: i,
      count: 1,
    });
    currentDay = day;
  }
  return sections;
}

export type GridMove = "up" | "down" | "left" | "right";

// Move a flattened selection index in 2D over sectioned 3-up grids.
// `sizes` are the per-section item counts in render order. Left/right walk the
// flattened order (crossing section boundaries naturally); up/down move by
// row within a section and hop to the adjacent section's nearest cell in the
// same column at the boundary. Out-of-moves clamp (the index stays).
export function moveInGrid(
  index: number,
  move: GridMove,
  sizes: number[],
  cols: number = GRID_COLS,
): number {
  const total = sizes.reduce((a, b) => a + b, 0);
  if (total === 0) return -1;
  if (index < 0) return 0;
  if (index >= total) return total - 1;

  if (move === "left") return Math.max(0, index - 1);
  if (move === "right") return Math.min(total - 1, index + 1);

  // Locate the section + local position.
  let start = 0;
  let s = 0;
  while (s < sizes.length && index >= start + sizes[s]) {
    start += sizes[s];
    s += 1;
  }
  const local = index - start;
  const col = local % cols;

  if (move === "down") {
    if (local + cols < sizes[s]) return start + local + cols;
    // Last row but the row to the right of us is ragged-short: clamp to the
    // section's last cell if we weren't already on the last row.
    const lastRowStart = Math.floor((sizes[s] - 1) / cols) * cols;
    if (local < lastRowStart) return start + sizes[s] - 1;
    // Hop to the next section's first row, same column.
    if (s + 1 < sizes.length) {
      return start + sizes[s] + Math.min(col, sizes[s + 1] - 1);
    }
    return index;
  }

  // up
  if (local - cols >= 0) return start + local - cols;
  if (s > 0) {
    const prevStart = start - sizes[s - 1];
    const prevLastRowStart = Math.floor((sizes[s - 1] - 1) / cols) * cols;
    return prevStart + Math.min(prevLastRowStart + col, sizes[s - 1] - 1);
  }
  return index;
}

// ── Cell caption formatting (frame-08 l2 line) ─────────────────────────────

// "14:32:08" — the cell's absolute time-of-day.
export function cellTime(iso: string): string {
  const d = parseToolDate(iso);
  if (d === null) return "—";
  const p = (n: number) => n.toString().padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

// "4m 06s" / "31m 40s" / "58s" — a result group's duration.
export function cellDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "—";
  const total = Math.round(ms / 1000);
  const m = Math.floor(total / 60);
  const s = total % 60;
  if (m === 0) return `${s}s`;
  return `${m}m ${s.toString().padStart(2, "0")}s`;
}
