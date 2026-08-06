// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import type { DayCoverage } from "$lib/types/app-infra";
import type { Conclusion } from "$lib/types/recording";
import {
  bytesLabel,
  capturedLabel,
  clock,
  confidenceDots,
  elapsedClock,
  heroHours,
  localDayWindow,
  minutesLabel,
  subjectRows,
  weekBars,
  weekTotalMs,
} from "./overview-format";

function conclusion(over: Partial<Conclusion>): Conclusion {
  return {
    id: 1,
    subject: "Mnema licensing",
    statement: "s",
    confidence: 0.5,
    status: "active",
    pinned: false,
    formedAtMs: 0,
    lastSupportedAtMs: 0,
    updatedAtMs: 0,
    evidence: [],
    ...over,
  } as Conclusion;
}

describe("G8 — an unmeasured quantity renders no number", () => {
  it("returns null instead of a zero", () => {
    expect(bytesLabel(null)).toBeNull();
    expect(bytesLabel(undefined)).toBeNull();
    expect(heroHours(null)).toBeNull();
    expect(capturedLabel(null)).toBeNull();
    // Under a minute is not "0:00 hours today" — it is no hero at all.
    expect(heroHours(30_000)).toBeNull();
    expect(capturedLabel(59_999)).toBeNull();
  });

  it("formats real quantities", () => {
    expect(bytesLabel(0)).toBe("0 B");
    expect(bytesLabel(270 * 1024 * 1024)).toBe("270 MB");
    expect(bytesLabel(36_722_925_568)).toBe("34.2 GB");
    expect(heroHours(6 * 3_600_000 + 42 * 60_000)).toBe("6:42");
    expect(capturedLabel(6 * 3_600_000 + 42 * 60_000)).toBe("6h 42m");
    expect(capturedLabel(47 * 60_000)).toBe("47m");
  });
});

describe("clocks", () => {
  it("counts capture elapsed as H:MM:SS and never negative", () => {
    expect(elapsedClock(0, 8047_000)).toBe("2:14:07");
    expect(elapsedClock(1000, 0)).toBe("0:00:00");
  });

  it("renders a local wall clock", () => {
    const d = new Date(2026, 7, 3, 13, 2);
    expect(clock(d.getTime())).toBe("13:02");
  });

  it("rounds a conversation to minutes, floor one", () => {
    expect(minutesLabel(38 * 60_000)).toBe("38 min");
    expect(minutesLabel(20_000)).toBe("1 min");
    expect(minutesLabel(62 * 60_000)).toBe("1h 02m");
  });
});

describe("localDayWindow", () => {
  it("is half-open across exactly one local day", () => {
    const { startMs, endMs } = localDayWindow(new Date(2026, 7, 3, 14, 40));
    expect(new Date(startMs).getHours()).toBe(0);
    expect(endMs - startMs).toBe(86_400_000);
  });
});

describe("weekBars", () => {
  const today = new Date(2026, 7, 3, 14, 0);
  const days: DayCoverage[] = [
    { day: "2026-08-03", coveredMs: 4 * 3_600_000, hours: [9, 10] },
    { day: "2026-07-31", coveredMs: 8 * 3_600_000, hours: [9] },
  ];

  it("returns seven days oldest-first, ending on today", () => {
    const bars = weekBars(days, today);
    expect(bars).toHaveLength(7);
    expect(bars[6].key).toBe("2026-08-03");
    expect(bars[6].isToday).toBe(true);
    expect(bars.filter((b) => b.isToday)).toHaveLength(1);
  });

  it("draws an absent day as an honest zero, and scales against the peak", () => {
    const bars = weekBars(days, today);
    expect(bars[6].fraction).toBeCloseTo(0.5);
    expect(bars.find((b) => b.key === "2026-07-31")?.fraction).toBe(1);
    expect(bars.find((b) => b.key === "2026-08-01")?.ms).toBe(0);
    expect(weekTotalMs(bars)).toBe(12 * 3_600_000);
  });

  it("keeps every bar at zero when the week holds no capture", () => {
    const bars = weekBars([], today);
    expect(bars.every((b) => b.ms === 0 && b.fraction === 0)).toBe(true);
  });
});

describe("subjectRows", () => {
  it("keeps one row per subject, most recently supported first", () => {
    const rows = subjectRows(
      [
        conclusion({ id: 1, subject: "Licensing", confidence: 0.4, lastSupportedAtMs: 10 }),
        conclusion({ id: 2, subject: "licensing", confidence: 0.9, lastSupportedAtMs: 10, statement: "top" }),
        conclusion({ id: 3, subject: "Deepgram", confidence: 0.5, lastSupportedAtMs: 99 }),
      ],
      5,
    );
    // NOCASE-deduped to one row; the winning belief's own casing labels it.
    expect(rows.map((r) => r.subject)).toEqual(["Deepgram", "licensing"]);
    expect(rows[1].statement).toBe("top");
  });

  it("lets a pinned belief win its subject's row regardless of confidence", () => {
    const rows = subjectRows([
      conclusion({ id: 1, confidence: 0.9, statement: "loud" }),
      conclusion({ id: 2, confidence: 0.2, statement: "pinned", pinned: true }),
    ]);
    expect(rows[0].statement).toBe("pinned");
  });

  it("is empty for no conclusions", () => {
    expect(subjectRows([])).toEqual([]);
  });
});

describe("confidenceDots", () => {
  it("never fabricates conviction, and never hides a held belief", () => {
    expect(confidenceDots(0)).toBe(0);
    expect(confidenceDots(-1)).toBe(0);
    expect(confidenceDots(0.02)).toBe(1);
    expect(confidenceDots(0.8)).toBe(4);
    expect(confidenceDots(2)).toBe(5);
  });
});
