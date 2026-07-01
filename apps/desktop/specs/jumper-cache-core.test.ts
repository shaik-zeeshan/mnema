import { describe, expect, test } from "bun:test";
import {
  JumperCacheCore,
  type FetchMonth,
} from "../src/lib/timeline/jumper-cache-core";
import type {
  FrameRangeRequest,
  FrameSummaryDto,
} from "../src/lib/types/app-infra";

// Characterization tests for the rune-free jumper cache core. The cache logic
// was verified correct during review; these lock in its observable behavior
// (per-month load, stale-while-revalidate, the mid-flight revalidation race
// from commit 0b518359, local→UTC month bounds, atomic month swap) so a future
// edit can't silently regress them. The adapter (`createJumperCache`) only adds
// `$state`-backed reactivity over this, so testing the core covers the logic.
//
// Assertions are derived from the SAME local-Date math the core uses, so they
// hold in any TZ without pinning `process.env.TZ`.

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** A fetcher that records requests and hands each call a deferred to drive. */
function deferredFetcher() {
  const calls: FrameRangeRequest[] = [];
  const pending: Deferred<FrameSummaryDto[]>[] = [];
  const fetchMonth: FetchMonth = (request) => {
    calls.push(request);
    const d = deferred<FrameSummaryDto[]>();
    pending.push(d);
    return d.promise;
  };
  return { fetchMonth, calls, pending };
}

/** A frame summary at a given LOCAL wall-clock moment (round-trips via ISO). */
function localFrame(
  id: number,
  y: number,
  m: number,
  d: number,
  h = 12,
  min = 0,
): FrameSummaryDto {
  return {
    id,
    capturedAt: new Date(y, m - 1, d, h, min, 0, 0).toISOString(),
  };
}

describe("JumperCacheCore.load — month fetch + caching", () => {
  test("loads a never-seen month once and groups frames by local date", async () => {
    const fetcher = deferredFetcher();
    const core = new JumperCacheCore(fetcher.fetchMonth);

    const p = core.load({ year: 2026, month: 6, day: 15 });
    // First load of an unseen month shows the spinner.
    expect(core.loading).toBe(true);
    fetcher.pending[0].resolve([
      localFrame(1, 2026, 6, 3, 9),
      localFrame(2, 2026, 6, 3, 11),
      localFrame(3, 2026, 6, 20, 8),
    ]);
    await p;

    expect(core.loading).toBe(false);
    expect(core.error).toBeNull();
    expect(core.monthLoaded({ year: 2026, month: 6 })).toBe(true);
    expect(core.daySummaries({ year: 2026, month: 6, day: 3 })?.map((s) => s.id)).toEqual([1, 2]);
    expect(core.daySummaries({ year: 2026, month: 6, day: 20 })?.map((s) => s.id)).toEqual([3]);
    expect(core.daySummaries({ year: 2026, month: 6, day: 4 })).toBeUndefined();
  });

  test("a loaded, non-stale month short-circuits (no second fetch)", async () => {
    const fetcher = deferredFetcher();
    const core = new JumperCacheCore(fetcher.fetchMonth);

    const p = core.load({ year: 2026, month: 6, day: 1 });
    fetcher.pending[0].resolve([localFrame(1, 2026, 6, 3)]);
    await p;

    await core.load({ year: 2026, month: 6, day: 28 }); // same month, already loaded
    expect(fetcher.calls.length).toBe(1);
  });

  test("concurrent loads of the same in-flight month dedupe to one fetch", async () => {
    const fetcher = deferredFetcher();
    const core = new JumperCacheCore(fetcher.fetchMonth);

    const p1 = core.load({ year: 2026, month: 6, day: 1 });
    const p2 = core.load({ year: 2026, month: 6, day: 2 }); // returns immediately (in flight)
    expect(fetcher.calls.length).toBe(1);
    fetcher.pending[0].resolve([localFrame(1, 2026, 6, 3)]);
    await Promise.all([p1, p2]);
    expect(fetcher.calls.length).toBe(1);
  });

  test("month bounds are local-month start..next-month start as UTC ISO (Dec→Jan rollover)", async () => {
    const fetcher = deferredFetcher();
    const core = new JumperCacheCore(fetcher.fetchMonth);

    const p = core.load({ year: 2025, month: 12, day: 10 });
    fetcher.pending[0].resolve([]);
    await p;

    // The end bound must roll into next January — `new Date(y, 12, 1)` overflows
    // to next-year January, the whole point of the `value.month` (not -1) end.
    expect(fetcher.calls[0]).toEqual({
      capturedAtStart: new Date(2025, 11, 1, 0, 0, 0, 0).toISOString(),
      capturedAtEnd: new Date(2026, 0, 1, 0, 0, 0, 0).toISOString(),
    });
  });

  test("frames within a day are sorted ascending by capture time", async () => {
    const fetcher = deferredFetcher();
    const core = new JumperCacheCore(fetcher.fetchMonth);

    const p = core.load({ year: 2026, month: 6, day: 1 });
    fetcher.pending[0].resolve([
      localFrame(3, 2026, 6, 3, 18),
      localFrame(1, 2026, 6, 3, 7),
      localFrame(2, 2026, 6, 3, 12),
    ]);
    await p;

    expect(core.daySummaries({ year: 2026, month: 6, day: 3 })?.map((s) => s.id)).toEqual([1, 2, 3]);
  });
});

describe("JumperCacheCore — stale-while-revalidate + atomic swap", () => {
  test("re-fetch atomically swaps a month's rows and leaves other months intact", async () => {
    const fetcher = deferredFetcher();
    const core = new JumperCacheCore(fetcher.fetchMonth);

    // Load June and May.
    let p = core.load({ year: 2026, month: 6, day: 1 });
    fetcher.pending[0].resolve([localFrame(1, 2026, 6, 3), localFrame(2, 2026, 6, 10)]);
    await p;
    p = core.load({ year: 2026, month: 5, day: 1 });
    fetcher.pending[1].resolve([localFrame(9, 2026, 5, 4)]);
    await p;

    // Invalidate June, then re-load: its old rows are dropped and replaced.
    core.invalidateMonthsForFrames([localFrame(0, 2026, 6, 25)]);
    p = core.load({ year: 2026, month: 6, day: 1 });
    fetcher.pending[2].resolve([localFrame(5, 2026, 6, 25)]);
    await p;

    expect(core.daySummaries({ year: 2026, month: 6, day: 3 })).toBeUndefined();
    expect(core.daySummaries({ year: 2026, month: 6, day: 10 })).toBeUndefined();
    expect(core.daySummaries({ year: 2026, month: 6, day: 25 })?.map((s) => s.id)).toEqual([5]);
    // May untouched by the June swap.
    expect(core.daySummaries({ year: 2026, month: 5, day: 4 })?.map((s) => s.id)).toEqual([9]);
  });

  test("a stale revalidation does NOT raise the spinner", async () => {
    const fetcher = deferredFetcher();
    const core = new JumperCacheCore(fetcher.fetchMonth);

    let p = core.load({ year: 2026, month: 6, day: 1 });
    fetcher.pending[0].resolve([localFrame(1, 2026, 6, 3)]);
    await p;

    core.invalidateAllLoadedMonths();
    p = core.load({ year: 2026, month: 6, day: 1 });
    expect(core.loading).toBe(false); // already have rows to show — revalidate quietly
    fetcher.pending[1].resolve([localFrame(2, 2026, 6, 4)]);
    await p;
  });

  test("REGRESSION (0b518359): frames arriving mid-flight keep the month stale so a fresh fetch re-runs and latest wins", async () => {
    const fetcher = deferredFetcher();
    const core = new JumperCacheCore(fetcher.fetchMonth);

    // First load is in flight...
    const p1 = core.load({ year: 2026, month: 6, day: 1 });
    expect(fetcher.calls.length).toBe(1);
    // ...and new frames for June land WHILE it's in flight (head poll / refresh).
    // This bumps June's epoch past the snapshot the in-flight load captured.
    core.invalidateMonthsForFrames([localFrame(0, 2026, 6, 28)]);
    // The in-flight response (which predates those frames) resolves.
    fetcher.pending[0].resolve([localFrame(1, 2026, 6, 3)]);
    await p1;

    // Because the epoch advanced mid-flight, June is kept STALE — so the next
    // load() must re-fetch rather than short-circuit on "loaded && !stale".
    // (Pre-0b518359 cleared stale on any success, so this second load would
    // short-circuit and `calls.length` would stay 1 — this asserts the fix.)
    const p2 = core.load({ year: 2026, month: 6, day: 1 });
    expect(fetcher.calls.length).toBe(2);
    // Latest fetch wins.
    fetcher.pending[1].resolve([localFrame(7, 2026, 6, 28)]);
    await p2;

    expect(core.daySummaries({ year: 2026, month: 6, day: 28 })?.map((s) => s.id)).toEqual([7]);
    expect(core.daySummaries({ year: 2026, month: 6, day: 3 })).toBeUndefined();
  });
});

describe("JumperCacheCore.load — error handling", () => {
  test("a rejected fetch surfaces a humanized error and leaves the month unloaded (retryable)", async () => {
    const fetcher = deferredFetcher();
    const core = new JumperCacheCore(fetcher.fetchMonth);

    const p = core.load({ year: 2026, month: 6, day: 1 });
    fetcher.pending[0].reject("backend exploded");
    await p;

    expect(core.error).toBe("Backend exploded");
    expect(core.loading).toBe(false);
    expect(core.monthLoaded({ year: 2026, month: 6 })).toBe(false);

    // Not loaded → a retry actually re-fetches.
    const p2 = core.load({ year: 2026, month: 6, day: 1 });
    expect(fetcher.calls.length).toBe(2);
    fetcher.pending[1].resolve([localFrame(1, 2026, 6, 3)]);
    await p2;
    expect(core.error).toBeNull();
    expect(core.monthLoaded({ year: 2026, month: 6 })).toBe(true);
  });

  test("clearError resets the error without touching loaded data", async () => {
    const fetcher = deferredFetcher();
    const core = new JumperCacheCore(fetcher.fetchMonth);
    const p = core.load({ year: 2026, month: 6, day: 1 });
    fetcher.pending[0].reject("boom");
    await p;
    expect(core.error).not.toBeNull();
    core.clearError();
    expect(core.error).toBeNull();
  });
});

describe("JumperCacheCore.isDateDisabled", () => {
  test("pre-load: never disabled (lets the user navigate into an unloaded month)", () => {
    const core = new JumperCacheCore(deferredFetcher().fetchMonth);
    expect(core.isDateDisabled({ year: 2026, month: 6, day: 3 })).toBe(false);
  });

  test("post-load: disabled iff the local date has no frames in the loaded month", async () => {
    const fetcher = deferredFetcher();
    const core = new JumperCacheCore(fetcher.fetchMonth);
    const p = core.load({ year: 2026, month: 6, day: 1 });
    fetcher.pending[0].resolve([localFrame(1, 2026, 6, 3)]);
    await p;

    expect(core.isDateDisabled({ year: 2026, month: 6, day: 3 })).toBe(false); // has a frame
    expect(core.isDateDisabled({ year: 2026, month: 6, day: 4 })).toBe(true); // loaded, empty
    expect(core.isDateDisabled({ year: 2026, month: 7, day: 1 })).toBe(false); // month not loaded
  });
});

describe("JumperCacheCore — invalidation guards + earliestKey", () => {
  test("invalidateMonthsForFrames is a no-op on empty / unparseable input", async () => {
    const fetcher = deferredFetcher();
    const core = new JumperCacheCore(fetcher.fetchMonth);
    const p = core.load({ year: 2026, month: 6, day: 1 });
    fetcher.pending[0].resolve([localFrame(1, 2026, 6, 3)]);
    await p;

    core.invalidateMonthsForFrames([]); // empty
    core.invalidateMonthsForFrames([{ capturedAt: "not-a-date" }]); // unparseable → skipped
    // Neither marked June stale, so a re-load short-circuits with no new fetch.
    await core.load({ year: 2026, month: 6, day: 1 });
    expect(fetcher.calls.length).toBe(1);
  });

  test("invalidateAllLoadedMonths is a no-op when nothing is loaded", () => {
    const core = new JumperCacheCore(deferredFetcher().fetchMonth);
    expect(() => core.invalidateAllLoadedMonths()).not.toThrow();
  });

  test("earliestKey is null when empty and the min loaded date otherwise", async () => {
    const fetcher = deferredFetcher();
    const core = new JumperCacheCore(fetcher.fetchMonth);
    expect(core.earliestKey()).toBeNull();

    let p = core.load({ year: 2026, month: 6, day: 1 });
    fetcher.pending[0].resolve([localFrame(1, 2026, 6, 10), localFrame(2, 2026, 6, 3)]);
    await p;
    p = core.load({ year: 2026, month: 5, day: 1 });
    fetcher.pending[1].resolve([localFrame(9, 2026, 5, 20)]);
    await p;

    expect(core.earliestKey()).toBe("2026-05-20");
  });
});

describe("JumperCacheCore — onMutate reactivity hook", () => {
  test("fires on load, invalidation and clearError so the rune adapter can re-render", async () => {
    const fetcher = deferredFetcher();
    let mutations = 0;
    const core = new JumperCacheCore(fetcher.fetchMonth, () => {
      mutations++;
    });

    const before = mutations;
    const p = core.load({ year: 2026, month: 6, day: 1 });
    expect(mutations).toBeGreaterThan(before); // spinner-on bump is synchronous
    fetcher.pending[0].resolve([localFrame(1, 2026, 6, 3)]);
    await p;
    const afterLoad = mutations;
    expect(afterLoad).toBeGreaterThan(before + 1); // settle bump too

    core.invalidateAllLoadedMonths();
    expect(mutations).toBe(afterLoad + 1);
    core.clearError();
    expect(mutations).toBe(afterLoad + 2);
  });
});
