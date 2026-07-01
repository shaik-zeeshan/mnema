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
import type { FrameRangeRequest, FrameSummaryDto } from "$lib/types/app-infra";
import { JumperCacheCore, type CalendarFields } from "./jumper-cache-core";

// Re-exported so existing importers keep a single entry point for the cache.
export {
  pad2,
  dateKeyOf,
  monthKeyOf,
  type DateKey,
  type MonthKey,
} from "./jumper-cache-core";

export type JumperCache = ReturnType<typeof createJumperCache>;

export function createJumperCache() {
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
    daySummaries(d: CalendarFields) {
      version;
      return core.daySummaries(d);
    },
    monthLoaded(d: { year: number; month: number }) {
      version;
      return core.monthLoaded(d);
    },
    earliestKey() {
      version;
      return core.earliestKey();
    },
    load(value: { year: number; month: number }) {
      // Read `version` so the caller's $effect depends on cache mutations: an
      // invalidate* that marks the visible month stale bumps `version`, which
      // re-runs the picker's load effect and re-fetches (stale-while-revalidate).
      // Without this read the effect only re-runs on open/placeholder changes,
      // so a month invalidated mid-open (head poll / refresh) never re-loads.
      // core.load() short-circuits on loaded-&-fresh / in-flight, so the extra
      // re-runs from unrelated version bumps are cheap no-ops.
      version;
      return core.load(value);
    },
    isDateDisabled(d: CalendarFields) {
      version;
      return core.isDateDisabled(d);
    },
    invalidateMonthsForFrames(frames: { capturedAt: string }[]) {
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
