// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import {
  agoLabel,
  popoverCalls,
  popoverCounts,
  recentText,
  searchNotice,
  waysOut,
  weekDays,
  withRecent,
} from "./recall-states";
import { pillOptions } from "./filterSurfaces.svelte";

// Friday 2026-10-09, 16:40.
const NOW = new Date(2026, 9, 9, 16, 40);
const arc = { kind: "app", app: "Arc" };
const tueAfternoon = { kind: "date", value: "2026-10-06", label: "Tue Oct 6", part: "afternoon" };

describe("recent queries", () => {
  test("remember pills as their phrases", () => {
    expect(recentText("invoice ", [arc, { kind: "date", value: "last-week", label: "Last week", part: null }])).toBe(
      "invoice in Arc last week",
    );
  });

  test("newest first, deduped case-insensitively, capped at 8", () => {
    const list = Array.from({ length: 8 }, (_, i) => ({ text: `q${i}`, ask: false, at: i }));
    const next = withRecent(list, { text: "Q3", ask: true, at: 99 });
    expect(next.map((q) => q.text)).toEqual(["Q3", "q0", "q1", "q2", "q4", "q5", "q6", "q7"]);
    expect(withRecent(list, { text: "new", ask: false, at: 99 })).toHaveLength(8);
    expect(withRecent(list, { text: "  ", ask: false, at: 99 })).toEqual(list);
  });

  test("ago labels", () => {
    const at = (d, h, m) => new Date(2026, 9, d, h, m).getTime();
    expect(agoLabel(at(9, 16, 40), NOW)).toBe("just now");
    expect(agoLabel(at(9, 16, 28), NOW)).toBe("12m ago");
    expect(agoLabel(at(9, 13, 0), NOW)).toBe("3h ago");
    expect(agoLabel(at(8, 23, 0), NOW)).toBe("yesterday");
    expect(agoLabel(at(7, 9, 0), NOW)).toBe("Wed");
    expect(agoLabel(at(1, 9, 0), NOW)).toBe("Oct 1");
  });
});

describe("notice line", () => {
  const now = NOW.getTime();
  const status = (o = {}) => ({ queuedFrames: 0, oldestQueuedMs: null, semanticBuilding: false, semanticIndexedPercent: 100, ...o });

  test("indexing behind only once the backlog is a minute old", () => {
    expect(searchNotice(status({ queuedFrames: 3, oldestQueuedMs: now - 5_000 }), null, true, now)).toBeNull();
    expect(searchNotice(status({ queuedFrames: 38, oldestQueuedMs: now - 4 * 60_000 }), null, true, now)?.text).toBe(
      "Still reading your last 4 min — 38 frames queued. Newer moments may be missing.",
    );
  });

  test("meaning building shows below the threshold, off says keyword only", () => {
    expect(searchNotice(status({ semanticBuilding: true, semanticIndexedPercent: 99 }), null, true, now)).toBeNull();
    expect(searchNotice(status({ semanticBuilding: true, semanticIndexedPercent: 62 }), null, true, now)?.text).toContain("62%");
    const off = searchNotice(status({ semanticIndexedPercent: null }), null, true, now);
    expect(off?.settings).toBe(true);
  });

  test("most actionable first", () => {
    const flags = { semanticDegraded: false, untranscribed: 2 };
    expect(searchNotice(status({ semanticIndexedPercent: null }), flags, false, now)?.text).toBe(
      "2 recordings are still being transcribed — they’ll show up once they are.",
    );
    expect(searchNotice(status({ semanticIndexedPercent: null }), { ...flags, untranscribed: 0 }, false, now)?.icon).toBe("off");
    expect(searchNotice(status(), { semanticDegraded: true, untranscribed: 2 }, false, now)?.tone).toBe("warn");
  });
});

describe("pill popover counts", () => {
  const counts = (facet, total, buckets, capped = false) => ({ facet, total, capped, buckets: buckets.map(([key, count, label = key]) => ({ key, label, count })) });

  test("app options read buckets by name, Any app is the total", () => {
    const options = pillOptions(arc, ["Arc", "Zed", "Slack"]);
    const out = popoverCounts(arc, options, [
      { facet: "app", query: "", counts: counts("app", 9, [["company.thebrowser.Browser", 5, "Arc"], ["dev.zed.Zed", 4, "Zed"]]) },
    ], NOW);
    expect(out).toEqual({ "app:Arc": 5, "app:Zed": 4, "app:Slack": 0, "": 9 });
  });

  test("date options: parts of the same day, weeks from days, capped reads as a lower bound", () => {
    const options = pillOptions(tueAfternoon, []);
    const calls = popoverCalls("pricing", [arc, tueAfternoon], tueAfternoon);
    expect(calls).toEqual([
      { facet: "partOfDay", query: "pricing app:Arc date:2026-10-06 hours:12-18" },
      { facet: "day", query: "pricing app:Arc" },
    ]);
    const out = popoverCounts(tueAfternoon, options, [
      { ...calls[0], counts: counts("partOfDay", 6, [["afternoon", 4], ["morning", 2]]) },
      { ...calls[1], counts: counts("day", 50, [["2026-10-06", 6], ["2026-10-08", 3], ["2026-10-01", 7]], true) },
    ], NOW);
    expect(out).toEqual({
      "date:2026-10-06 hours:5-12": 2,
      "date:2026-10-06 hours:12-18": 4,
      "date:2026-10-06 hours:18-24": 0,
      "date:2026-10-06": 6,
      "date:this-week": "9+",
      "date:last-week": "7+",
      "": "50+",
    });
  });

  test("weeks run Monday to Sunday", () => {
    expect([...weekDays(NOW, 0)][0]).toBe("2026-10-05");
    expect([...weekDays(NOW, -1)][6]).toBe("2026-10-04");
  });
});

describe("no results ways out", () => {
  test("widen the day, drop each pill, then ask", () => {
    const out = waysOut("invoice", [arc, tueAfternoon], true);
    expect(out.map((w) => w.label)).toEqual([
      "All of Tue Oct 6, not just the afternoon",
      "Drop “Arc”",
      "Any time — not just on Tue Oct 6 afternoon",
      "Ask instead — I’ll look harder",
    ]);
    expect(out[0].call.query).toBe("invoice app:Arc date:2026-10-06");
    expect(out[1].call).toEqual({ facet: "app", query: "invoice date:2026-10-06 hours:12-18" });
  });

  test("no ask without words or a provider", () => {
    expect(waysOut("", [arc], true).map((w) => w.kind)).toEqual(["pills"]);
    expect(waysOut("invoice", [], false)).toEqual([]);
  });
});
