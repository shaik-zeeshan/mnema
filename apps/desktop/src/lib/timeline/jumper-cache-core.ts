// ── Timeline Jumper — per-month summary cache (rune-free core) ────────────────
// The load-bearing logic powering the jump picker's calendar, extracted out of
// the Svelte-runes adapter (`jumper-cache.svelte.ts`) so it can be exercised
// under `bun test` (rune modules throw `$state is not defined` on import).
//
//   - Frame summaries (id + capturedAt) are loaded per visible calendar month
//     and grouped by LOCAL date. The calendar disables dates with no frames in
//     months already loaded.
//   - Stale-while-revalidate: months whose summaries are known out-of-date are
//     marked stale (NOT deleted) so the open picker keeps rendering its
//     disabled-date map until the replacement response lands — avoiding the
//     visible flicker that came from dropping a month's cache before its
//     replacement arrived.
//
// Pure data only — the "latest at or before X" resolution + focused-window load
// stay backend-owned (reached by the dashboard via `get_latest_frame_in_range`
// / `get_timeline_window_around_frame`).
//
// `JumperCacheCore` holds the Map/Set state and all methods with its network
// dependency injected (`fetchMonth`). `createJumperCache()` (in the sibling
// `.svelte.ts`) is the thin rune adapter: it wires `fetchMonth` to `invoke`
// and an `onMutate` callback that bumps a `$state` version so Svelte
// re-renders when this core mutates.
import { parseCapturedAt } from "$lib/format-time";
import { humanizeError } from "$lib/format-error";
import type { FrameRangeRequest, FrameSummaryDto } from "$lib/types/app-infra";

export type DateKey = string; // "YYYY-MM-DD" in local time
export type MonthKey = string; // "YYYY-MM" in local time

/** The minimal calendar shape both `DateValue` and plain test inputs satisfy. */
export interface CalendarFields {
  year: number;
  month: number;
  day: number;
}

export function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

export function dateKeyOf(d: CalendarFields): DateKey {
  return `${d.year}-${pad2(d.month)}-${pad2(d.day)}`;
}

export function monthKeyOf(d: { year: number; month: number }): MonthKey {
  return `${d.year}-${pad2(d.month)}`;
}

function localDateKeyFromTs(ts: string): DateKey {
  const d = parseCapturedAt(ts);
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

/** Fetches one calendar month's frame summaries (UTC ISO bounds). */
export type FetchMonth = (
  request: FrameRangeRequest,
) => Promise<FrameSummaryDto[]>;

export class JumperCacheCore {
  private summariesByDate = new Map<DateKey, FrameSummaryDto[]>();
  private loadedMonths = new Set<MonthKey>();
  private staleMonths = new Set<MonthKey>();
  // Per-month invalidation epoch. Bumped by every invalidation (even for a month
  // already stale) so a fetch in flight can detect that frames landed mid-flight
  // — `load()` captures the epoch before its first await and, on success, keeps
  // the month stale if the epoch advanced so a fresh fetch re-runs.
  private monthEpochs = new Map<MonthKey, number>();
  // In-flight month fetches. Dedupes concurrent revalidations triggered by the
  // picker effect, manual refresh, and head poll all racing the same month.
  private monthsInFlight = new Set<MonthKey>();

  /** Spinner gate — only set on a first load of a never-seen month. */
  loading = false;
  /** Month-load error (distinct from a commit/jump error). */
  error: string | null = null;

  constructor(
    private readonly fetchMonth: FetchMonth,
    /** Bumped after any state mutation so the rune adapter can re-render. */
    private readonly onMutate: () => void = () => {},
  ) {}

  async load(value: { year: number; month: number }): Promise<void> {
    const key = monthKeyOf(value);
    const isStale = this.staleMonths.has(key);
    // Already up-to-date and loaded — nothing to do.
    if (this.loadedMonths.has(key) && !isStale) return;
    // Another caller is already revalidating this month; let its response be
    // the one that swaps the data in.
    if (this.monthsInFlight.has(key)) return;
    this.monthsInFlight.add(key);
    // Snapshot the month's invalidation epoch before the first await. If frames
    // for this month arrive mid-flight, invalidateMonths* bumps the epoch; on
    // success we compare and keep the month stale so it revalidates (the
    // in-flight response predates those frames).
    const startEpoch = this.monthEpochs.get(key) ?? 0;
    // Only show the spinner when there's nothing to render yet. Stale
    // revalidations happen quietly.
    const isFirstLoad = !this.loadedMonths.has(key);
    if (isFirstLoad) {
      this.loading = true;
      this.onMutate();
    }
    try {
      // Local month bounds, converted to UTC ISO for the backend.
      const start = new Date(value.year, value.month - 1, 1, 0, 0, 0, 0);
      const end = new Date(value.year, value.month, 1, 0, 0, 0, 0);
      const req: FrameRangeRequest = {
        capturedAtStart: start.toISOString(),
        capturedAtEnd: end.toISOString(),
      };
      const summaries = await this.fetchMonth(req);
      // Atomically swap this month's rows: drop any prior entries whose local
      // date falls inside this month, then insert the fresh ones. One
      // assignment means the picker never observes an intermediate "month
      // exists in loadedMonths but has no rows" state.
      const next = new Map(this.summariesByDate);
      for (const k of Array.from(next.keys())) {
        if (k.startsWith(`${key}-`)) next.delete(k);
      }
      const touched = new Set<DateKey>();
      for (const s of summaries) {
        const k = localDateKeyFromTs(s.capturedAt);
        const arr = next.get(k);
        if (arr) arr.push(s);
        else next.set(k, [s]);
        touched.add(k);
      }
      // Ascending by capture time within each day so minute buckets resolve
      // their "latest in bucket" by simple last-write-wins. Sort ONLY the days
      // we just rebuilt for this month — every other month's arrays are already
      // sorted and untouched, so re-sorting all of them would burn O(N log N)
      // across the whole loaded history on each stale-while-revalidate tick
      // (the head poll revalidates the visible month every 1.5s while the
      // picker is open during active capture).
      for (const k of touched) {
        next.get(k)?.sort((a, b) => a.capturedAt.localeCompare(b.capturedAt));
      }
      this.summariesByDate = next;
      if (!this.loadedMonths.has(key)) {
        const nextMonths = new Set(this.loadedMonths);
        nextMonths.add(key);
        this.loadedMonths = nextMonths;
      }
      // Re-evaluate staleness against the epoch captured at fetch start. If it
      // advanced, frames landed while this fetch was in flight — keep the month
      // stale (so the load effect re-runs) so a fresh fetch runs once
      // monthsInFlight clears below. Otherwise clear the stale flag.
      const epochAdvanced = (this.monthEpochs.get(key) ?? 0) !== startEpoch;
      if (epochAdvanced) {
        this.staleMonths.add(key);
      } else if (this.staleMonths.has(key)) {
        this.staleMonths.delete(key);
      }
      this.error = null;
    } catch (err) {
      this.error = humanizeError(err);
    } finally {
      this.monthsInFlight.delete(key);
      if (isFirstLoad) this.loading = false;
      this.onMutate();
    }
  }

  isDateDisabled(d: CalendarFields): boolean {
    // Pre-load: don't disable so the user can navigate into a month before its
    // summaries arrive. Once a month is loaded, disable any local date not
    // present in the dataset.
    if (!this.loadedMonths.has(monthKeyOf(d))) return false;
    return !this.summariesByDate.has(dateKeyOf(d));
  }

  invalidateMonthsForFrames(frames: { capturedAt: string }[]): void {
    if (frames.length === 0) return;
    const affectedMonths = new Set<MonthKey>();
    for (const f of frames) {
      const d = parseCapturedAt(f.capturedAt);
      if (isNaN(d.getTime())) continue;
      affectedMonths.add(`${d.getFullYear()}-${pad2(d.getMonth() + 1)}`);
    }
    if (affectedMonths.size === 0) return;
    // Bump each affected month's epoch unconditionally — even one already stale
    // — so an in-flight fetch notices frames arrived mid-flight and re-runs.
    // Mark stale so the load effect picks the month up.
    for (const m of affectedMonths) {
      this.monthEpochs.set(m, (this.monthEpochs.get(m) ?? 0) + 1);
      this.staleMonths.add(m);
    }
    this.onMutate();
  }

  invalidateAllLoadedMonths(): void {
    if (this.loadedMonths.size === 0) return;
    // Same epoch bump as invalidateMonthsForFrames so a month being fetched
    // right now is re-fetched rather than left displaying pre-invalidation rows.
    for (const month of this.loadedMonths) {
      this.monthEpochs.set(month, (this.monthEpochs.get(month) ?? 0) + 1);
      this.staleMonths.add(month);
    }
    this.onMutate();
  }

  /** Summaries for a local date, or undefined if the day has no frames. */
  daySummaries(d: CalendarFields): FrameSummaryDto[] | undefined {
    return this.summariesByDate.get(dateKeyOf(d));
  }

  /** Whether the month containing `d` has been loaded at least once. */
  monthLoaded(d: { year: number; month: number }): boolean {
    return this.loadedMonths.has(monthKeyOf(d));
  }

  /** Earliest local date with frames across all loaded months ("YYYY-MM-DD"). */
  earliestKey(): DateKey | null {
    let min: DateKey | null = null;
    for (const [k, rows] of this.summariesByDate) {
      if (rows.length === 0) continue;
      if (min === null || k < min) min = k;
    }
    return min;
  }

  clearError(): void {
    this.error = null;
    this.onMutate();
  }
}
