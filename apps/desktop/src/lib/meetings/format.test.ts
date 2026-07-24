// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here (repo convention).
import { describe, expect, it } from "bun:test";
import {
  appGlyph,
  dayHeading,
  dayTotals,
  durationLabel,
  meetingTitle,
  provenanceLabel,
  timeRange,
} from "./format";

const base = {
  id: "meeting-1",
  appDisplayName: "Zoom",
  bundleId: "us.zoom.xos",
  startMs: 0,
  endMs: 60_000,
  state: "recap",
  speakers: [],
};

describe("meetingTitle", () => {
  it("uses named speakers, skipping You and Speaker N labels", () => {
    expect(
      meetingTitle({ ...base, speakers: ["Dev Patel", "You", "Speaker 3"] }),
    ).toBe("Meeting with Dev Patel");
  });
  it("counts unnamed voices when no names matched", () => {
    expect(meetingTitle({ ...base, speakers: ["Speaker 1", "You"] })).toBe(
      "Meeting with 1 other",
    );
  });
  it("falls back to the URL host, then the app name", () => {
    expect(
      meetingTitle({ ...base, meetingUrl: "https://meet.google.com/abc" }),
    ).toBe("Meeting on meet.google.com");
    expect(meetingTitle(base)).toBe("Meeting on Zoom");
  });
});

describe("provenanceLabel", () => {
  it("names the bundle for native apps", () => {
    expect(provenanceLabel(base)).toBe("detected via mic-hold · us.zoom.xos");
  });
  it("names host + browser for browser meetings", () => {
    expect(
      provenanceLabel({
        ...base,
        appDisplayName: "Google Meet (Arc)",
        meetingUrl: "https://meet.google.com/abc",
      }),
    ).toBe("detected via mic-hold · meet.google.com · Arc");
  });
});

describe("appGlyph", () => {
  it("maps known apps and browser meetings", () => {
    expect(appGlyph(base)).toBe("zm");
    expect(appGlyph({ ...base, appDisplayName: "Microsoft Teams" })).toBe("ts");
    expect(appGlyph({ ...base, meetingUrl: "https://x.test" })).toBe("◈");
  });
});

describe("time formatting", () => {
  it("collapses a shared meridiem", () => {
    const s = new Date(2026, 6, 23, 10, 0).getTime();
    const e = new Date(2026, 6, 23, 10, 47).getTime();
    expect(timeRange(s, e)).toBe("10:00 – 10:47 AM");
  });
  it("keeps both meridiems across noon", () => {
    const s = new Date(2026, 6, 23, 11, 40).getTime();
    const e = new Date(2026, 6, 23, 12, 10).getTime();
    expect(timeRange(s, e)).toBe("11:40 AM – 12:10 PM");
  });
  it("renders durations", () => {
    expect(durationLabel(47 * 60_000)).toBe("47m");
    expect(durationLabel(90 * 60_000)).toBe("1h 30m");
    expect(durationLabel(120 * 60_000)).toBe("2h");
  });
});

describe("dayHeading / dayTotals", () => {
  const now = new Date(2026, 6, 23, 15, 0);
  it("labels today, yesterday, weekday, then dates", () => {
    expect(dayHeading("2026-07-23", now).label).toBe("Today");
    expect(dayHeading("2026-07-22", now).label).toBe("Yesterday");
    expect(dayHeading("2026-07-20", now).label).toBe("Monday");
    expect(dayHeading("2026-07-10", now).label).toBe("Jul 10");
    expect(dayHeading("2026-07-23", now).sub).toBe("THU · JUL 23 2026");
  });
  it("sums a day's meetings", () => {
    expect(
      dayTotals([
        { ...base, startMs: 0, endMs: 47 * 60_000 },
        { ...base, startMs: 0, endMs: 43 * 60_000 },
      ]),
    ).toBe("2 meetings · 1h 30m");
  });
});
