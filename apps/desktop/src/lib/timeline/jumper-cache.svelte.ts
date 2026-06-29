// ── Timeline Jumper — per-month summary cache (rune adapter) ──────────────────
// Thin Svelte-runes wrapper over the rune-free `JumperCacheCore`. All the
// load-bearing logic (per-month load, stale-while-revalidate, the mid-flight
// revalidation race, local→UTC month bounds) lives in `jumper-cache-core.ts`
// so it can be unit-tested under `bun test`; this adapter only wires the
// network dependency to `invoke` and exposes reactive getters.
//
// Reactivity: the core mutates plain Maps/Sets in place, so it calls an
// injected `onMutate` after every change. We bump a `$state` `version` there;
// each getter/method reads `version` first, establishing the reactive
// dependency that re-renders the calendar when the core changes.
import { invoke } from "@tauri-apps/api/core";
import type { DateValue } from "@internationalized/date";
import type { FrameRangeRequest, FrameSummaryDto } from "$lib/types/app-infra";
import { JumperCacheCore } from "./jumper-cache-core";

// Re-exported so existing importers keep a single entry point for the cache.
export {
  pad2,
  dateKeyOf,
  monthKeyOf,
  type DateKey,
  type MonthKey,
} from "./jumper-cache-core";

export interface JumperCache {
  /** Spinner gate — only set on a first load of a never-seen month. */
  readonly loading: boolean;
  /** Month-load error (distinct from a commit/jump error). */
  readonly error: string | null;
  /** Summaries for a local date, or undefined if the day has no frames. */
  daySummaries(d: { year: number; month: number; day: number }): FrameSummaryDto[] | undefined;
  /** Whether the month containing `d` has been loaded at least once. */
  monthLoaded(d: { year: number; month: number }): boolean;
  /** Earliest local date with frames across all loaded months ("YYYY-MM-DD"). */
  earliestKey(): string | null;
  load(value: DateValue): Promise<void>;
  isDateDisabled(d: DateValue): boolean;
  invalidateMonthsForFrames(frames: { capturedAt: string }[]): void;
  invalidateAllLoadedMonths(): void;
  clearError(): void;
}

export function createJumperCache(): JumperCache {
  let version = $state(0);
  const core = new JumperCacheCore(
    (request: FrameRangeRequest) =>
      invoke<FrameSummaryDto[]>("list_frame_summaries_in_range", { request }),
    () => {
      version++;
    },
  );

  return {
    get loading() {
      version;
      return core.loading;
    },
    get error() {
      version;
      return core.error;
    },
    daySummaries(d) {
      version;
      return core.daySummaries(d);
    },
    monthLoaded(d) {
      version;
      return core.monthLoaded(d);
    },
    earliestKey() {
      version;
      return core.earliestKey();
    },
    load(value) {
      return core.load(value);
    },
    isDateDisabled(d) {
      version;
      return core.isDateDisabled(d);
    },
    invalidateMonthsForFrames(frames) {
      core.invalidateMonthsForFrames(frames);
    },
    invalidateAllLoadedMonths() {
      core.invalidateAllLoadedMonths();
    },
    clearError() {
      core.clearError();
    },
  };
}
