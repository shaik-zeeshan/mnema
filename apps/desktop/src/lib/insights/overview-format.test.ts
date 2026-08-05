// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  conversationDurationLabel,
  coverageHero,
  coverageLabel,
  firstSentence,
  momentsShownCount,
  monthlyPaceLabel,
  retentionLabel,
  speakersLabel,
  subjectRows,
} from "./overview-format";

function conv(startedAtMs, displayEndedAtMs) {
  return {
    activityId: 1,
    title: "t",
    startedAtMs,
    endedAtMs: displayEndedAtMs,
    displayEndedAtMs,
    speakerCount: 2,
    speechMs: 0,
  };
}

// The Conversations row's machine part: minutes under an hour, hours over,
// from the DISPLAY end (spill-extended) — never the raw activity end.
describe("conversationDurationLabel", () => {
  it("reads minutes under an hour", () => {
    expect(conversationDurationLabel(conv(0, 38 * 60_000))).toBe("38 min");
  });
  it("reads hours + minutes over an hour, hours alone on the boundary", () => {
    expect(conversationDurationLabel(conv(0, 64 * 60_000))).toBe("1h 4m");
    expect(conversationDurationLabel(conv(0, 120 * 60_000))).toBe("2h");
  });
  it("floors a degenerate range at one minute", () => {
    expect(conversationDurationLabel(conv(1_000, 0))).toBe("1 min");
  });
});

describe("speakersLabel", () => {
  it("singular/plural", () => {
    expect(speakersLabel(1)).toBe("1 speaker");
    expect(speakersLabel(5)).toBe("5 speakers");
  });
});

// The header meta and the one --t-display hero share the same coverage ms.
describe("coverage labels", () => {
  it("coverageLabel: minutes, hours, hours+minutes", () => {
    expect(coverageLabel(0)).toBe("0m");
    expect(coverageLabel(42 * 60_000)).toBe("42m");
    expect(coverageLabel(6 * 3_600_000 + 42 * 60_000)).toBe("6h 42m");
    expect(coverageLabel(2 * 3_600_000)).toBe("2h");
  });
  it("coverageHero: h:mm with zero-padded minutes", () => {
    expect(coverageHero(6 * 3_600_000 + 42 * 60_000)).toBe("6:42");
    expect(coverageHero(5 * 60_000)).toBe("0:05");
    expect(coverageHero(-1)).toBe("0:00");
  });
});

describe("monthlyPaceLabel", () => {
  it("projects today's bytes to a month in GB/MB", () => {
    expect(monthlyPaceLabel(270e6)).toBe("≈ 8.1 GB / month at today's pace");
    expect(monthlyPaceLabel(10e6)).toBe("≈ 300 MB / month at today's pace");
  });
  it("null when nothing captured (line is omitted, not zeroed)", () => {
    expect(monthlyPaceLabel(0)).toBeNull();
  });
});

describe("retentionLabel", () => {
  it("maps every policy", () => {
    expect(retentionLabel("days_7")).toBe("keep 7 days");
    expect(retentionLabel("days_30")).toBe("keep 30 days");
    expect(retentionLabel("never")).toBe("kept forever");
  });
});

// The 800×600 ladder truncates the digest to its first sentence.
describe("firstSentence", () => {
  it("cuts at the first sentence boundary", () => {
    expect(firstSentence("A licensing day. Then more.")).toBe("A licensing day.");
    expect(firstSentence("Really? Yes.")).toBe("Really?");
  });
  it("does not cut inside an abbreviation-like token (no following space)", () => {
    expect(firstSentence("Worked on answer_view.rs all day. Then rest.")).toBe(
      "Worked on answer_view.rs all day.",
    );
  });
  it("returns the whole text when no terminator exists", () => {
    expect(firstSentence("no terminator here")).toBe("no terminator here");
  });
});

describe("subjectRows", () => {
  const c = (subject, statement, confidence, lastSupportedAtMs, status = "visible") => ({
    id: 1,
    subject,
    statement,
    confidence,
    status,
    pinned: false,
    formedAtMs: 0,
    lastSupportedAtMs,
    updatedAtMs: 0,
    evidence: [],
  });
  it("groups by subject, keeps the highest-confidence statement, orders by freshness", () => {
    const rows = subjectRows([
      c("Licensing", "old take", 0.4, 10),
      c("Licensing", "strong take", 0.9, 5),
      c("Deepgram", "reading docs", 0.4, 20),
      c("Faded", "gone", 0.9, 99, "faded"),
    ]);
    expect(rows.map((r) => r.subject)).toEqual(["Deepgram", "Licensing"]);
    expect(rows[1].statement).toBe("strong take");
    expect(rows[1].dots).toBe(5);
    expect(rows[0].dots).toBe(2);
  });
  it("clamps dots into 1..5", () => {
    expect(subjectRows([c("S", "s", 0.01, 0)])[0].dots).toBe(1);
    expect(subjectRows([c("S", "s", 1.4, 0)])[0].dots).toBe(5);
  });
});

// Drop ladder: the moments strip narrows 5 → 3 and widens to 6 (frame 04).
describe("momentsShownCount", () => {
  it("3 narrow / 5 default / 6 wide", () => {
    expect(momentsShownCount(true, false)).toBe(3);
    expect(momentsShownCount(false, false)).toBe(5);
    expect(momentsShownCount(false, true)).toBe(6);
  });
});
