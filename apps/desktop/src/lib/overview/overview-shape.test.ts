// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import { heroHours, openThreadSentence, todayRange, weekFromCoverage } from "./overview-shape";

describe("openThreadSentence", () => {
  it("returns the digest's own open-thread sentence (G11: no extraction)", () => {
    const narrative =
      "Mostly a licensing day. One open thread — Tom wants delivery ids in the webhook log. " +
      "The afternoon went to answer_view.rs.";
    expect(openThreadSentence(narrative)).toBe(
      "One open thread — Tom wants delivery ids in the webhook log.",
    );
  });

  it("is null when the digest never named one, so the tile can say so", () => {
    expect(openThreadSentence("A quiet day of reading.")).toBeNull();
    expect(openThreadSentence(null)).toBeNull();
    expect(openThreadSentence("")).toBeNull();
  });
});

describe("weekFromCoverage", () => {
  const now = new Date(2026, 7, 5, 14, 30); // Wed 5 Aug 2026, local

  it("is seven days ending today, with absent days as real zeroes", () => {
    const week = weekFromCoverage(
      [
        { day: "2026-08-05", coveredMs: 3_600_000, hours: [9] },
        { day: "2026-08-03", coveredMs: 7_200_000, hours: [10, 11] },
        { day: "2026-06-01", coveredMs: 999, hours: [1] }, // outside the window
      ],
      now,
    );
    expect(week).toHaveLength(7);
    expect(week[0].key).toBe("2026-07-30");
    expect(week[6].key).toBe("2026-08-05");
    expect(week[6].isToday).toBe(true);
    expect(week.filter((d) => d.isToday)).toHaveLength(1);
    expect(week[6].coveredMs).toBe(3_600_000);
    expect(week[4].coveredMs).toBe(7_200_000); // 2026-08-03
    expect(week[5].coveredMs).toBe(0); // absent day => zero bar
  });
});

describe("heroHours", () => {
  it("formats H:MM and stays empty below a minute (never prints 0:00)", () => {
    expect(heroHours(6 * 3_600_000 + 42 * 60_000)).toBe("6:42");
    expect(heroHours(9 * 60_000)).toBe("0:09");
    expect(heroHours(0)).toBe("");
    expect(heroHours(30_000)).toBe("");
  });
});

describe("todayRange", () => {
  it("is half-open [local midnight, now)", () => {
    const now = new Date(2026, 7, 5, 14, 30, 15);
    const { startMs, endMs } = todayRange(now);
    expect(new Date(startMs).getHours()).toBe(0);
    expect(new Date(startMs).getDate()).toBe(5);
    expect(endMs).toBe(now.getTime());
  });
});
