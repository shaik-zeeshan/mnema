// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  backlogPhrase,
  captureRateConsequence,
  coarseRuntime,
  modelFitVerdict,
  modelFootprint,
  projectedBytesPerDay,
  retentionConsequence,
  retentionFootprint,
  semanticCoverage,
  semanticIndexPrice,
} from "./system-facts";

const GB = 1_000_000_000;

const facts = (overrides = {}) => ({
  capturePath: "/Users/x/.mnema",
  diskFreeBytes: 200 * GB,
  totalRamBytes: 17_179_869_184,
  measuredBytesPerDay: 10 * GB,
  measuredDays: 7,
  screenFrameRate: 0.5,
  ocrBacklog: 0,
  transcriptionBacklog: 0,
  semanticVectorCount: 0,
  semanticPendingCount: 0,
  semanticVectorBytes: 768,
  databaseBytes: 5 * GB,
  ...overrides,
});

describe("projectedBytesPerDay", () => {
  it("scales the measured rate linearly with the frame rate", () => {
    expect(projectedBytesPerDay(facts(), 0.5)).toBe(10 * GB);
    expect(projectedBytesPerDay(facts(), 1)).toBe(20 * GB);
    expect(projectedBytesPerDay(facts(), 0.1)).toBeCloseTo(2 * GB, -3);
  });

  it("refuses to project without both a measurement and its baseline (G8)", () => {
    expect(projectedBytesPerDay(null, 1)).toBeNull();
    expect(projectedBytesPerDay(facts({ measuredBytesPerDay: null }), 1)).toBeNull();
    expect(projectedBytesPerDay(facts({ screenFrameRate: null }), 1)).toBeNull();
    expect(projectedBytesPerDay(facts({ screenFrameRate: 0 }), 1)).toBeNull();
    expect(projectedBytesPerDay(facts(), 0)).toBeNull();
    expect(projectedBytesPerDay(facts(), undefined)).toBeNull();
  });
});

describe("coarseRuntime", () => {
  it("is coarse by construction — never a minute-precise ETA (G8)", () => {
    expect(coarseRuntime(5 * GB, 10 * GB)).toBe("under a day");
    expect(coarseRuntime(15 * GB, 10 * GB)).toBe("about a day");
    expect(coarseRuntime(50 * GB, 10 * GB)).toBe("about 5 days");
    expect(coarseRuntime(200 * GB, 10 * GB)).toBe("about 3 weeks");
    expect(coarseRuntime(1000 * GB, 10 * GB)).toBe("about 3 months");
    expect(coarseRuntime(10_000 * GB, 10 * GB)).toBe("over a year");
  });

  it("says nothing when either side is unmeasurable", () => {
    expect(coarseRuntime(null, 10 * GB)).toBeNull();
    expect(coarseRuntime(200 * GB, null)).toBeNull();
    expect(coarseRuntime(200 * GB, 0)).toBeNull();
  });
});

describe("captureRateConsequence", () => {
  it("states the daily cost and how long the disk lasts", () => {
    expect(captureRateConsequence(facts(), 1)).toBe(
      "About 20.0 GB a day at this rate — about 10 days of free space left. Measured over your last 7 days of capture.",
    );
  });

  it("drops the runtime clause when free space is unmeasurable", () => {
    expect(captureRateConsequence(facts({ diskFreeBytes: null }), 0.5)).toBe(
      "About 10.0 GB a day at this rate — measured over your last 7 days of capture.",
    );
  });

  it("singularizes a one-day measurement", () => {
    const line = captureRateConsequence(facts({ measuredDays: 1 }), 0.5);
    expect(line).toContain("your last 1 day of capture");
  });

  it("says nothing before a complete capture day exists", () => {
    expect(
      captureRateConsequence(facts({ measuredBytesPerDay: null, measuredDays: 0 }), 0.5),
    ).toBeNull();
  });
});

describe("retentionConsequence", () => {
  it("turns a window into bytes kept", () => {
    expect(retentionConsequence(facts(), "days_7")).toBe(
      "Keeps about 70.0 GB on disk at your measured rate.",
    );
    expect(retentionConsequence(facts(), "days_30")).toBe(
      "Keeps about 300.0 GB on disk at your measured rate.",
    );
  });

  it("gives Forever a runtime instead of a ceiling", () => {
    expect(retentionConsequence(facts(), "never")).toBe(
      "Nothing is deleted — at your measured rate the free space lasts about 3 weeks.",
    );
  });

  it("says nothing without a measured rate", () => {
    expect(retentionConsequence(facts({ measuredBytesPerDay: null }), "days_7")).toBeNull();
  });
});

describe("modelFootprint", () => {
  it("prices a download against the two real machine limits", () => {
    // speakrs, from the corrected registry (419_482_724, not the mockups' 31 MB).
    expect(modelFootprint(facts(), 419_482_724)).toBe(
      "419.5 MB to download · 200.0 GB free on this disk · 17.2 GB RAM on this Mac",
    );
  });

  it("omits whichever limit is unmeasurable", () => {
    expect(modelFootprint(facts({ totalRamBytes: null }), 419_482_724)).toBe(
      "419.5 MB to download · 200.0 GB free on this disk",
    );
    expect(modelFootprint(facts({ diskFreeBytes: null, totalRamBytes: null }), 419_482_724)).toBeNull();
    expect(modelFootprint(null, 419_482_724)).toBeNull();
  });

  it("says nothing for an OS-managed (zero-byte) model", () => {
    expect(modelFootprint(facts(), 0)).toBeNull();
    expect(modelFootprint(facts(), null)).toBeNull();
  });
});

describe("backlogPhrase", () => {
  it("reports a measured zero, but stays silent on an unreadable queue", () => {
    expect(backlogPhrase(0, "frame")).toBe("Nothing waiting.");
    expect(backlogPhrase(null, "frame")).toBeNull();
  });

  it("pluralizes and groups", () => {
    expect(backlogPhrase(1, "frame")).toBe("1 frame waiting.");
    expect(backlogPhrase(4102, "frame")).toBe("4,102 frames waiting.");
  });
});

describe("semanticIndexPrice", () => {
  it("prices the index off the real pending-anchor count and the schema's vector width", () => {
    expect(semanticIndexPrice(facts({ semanticPendingCount: 2_000_000 }))).toBe(
      "Indexing what you have captured so far adds about 1.5 GB to the database (2,000,000 captures still to index).",
    );
  });

  it("says nothing when there is nothing to index, or nothing to count", () => {
    expect(semanticIndexPrice(facts())).toBeNull();
    expect(semanticIndexPrice(facts({ semanticPendingCount: null }))).toBeNull();
  });
});

describe("semanticCoverage", () => {
  it("states the fraction over both real counts summed", () => {
    const c = semanticCoverage(facts({ semanticVectorCount: 1200, semanticPendingCount: 3800 }));
    expect(c).toEqual({
      indexed: 1200,
      total: 5000,
      percent: 24,
      phrase: "1,200 of 5,000 captures indexed (24%) — 3,800 still to go.",
    });
  });

  it("floors, so a nearly-done index never reads as finished", () => {
    expect(semanticCoverage(facts({ semanticVectorCount: 999, semanticPendingCount: 1 }))?.percent).toBe(99);
  });

  it("says done only when nothing is pending", () => {
    const c = semanticCoverage(facts({ semanticVectorCount: 5000, semanticPendingCount: 0 }));
    expect(c?.percent).toBe(100);
    expect(c?.phrase).toBe("Indexed — all 5,000 captures have a search vector.");
  });

  it("draws nothing without real counts, or with nothing indexable", () => {
    expect(semanticCoverage(null)).toBeNull();
    expect(semanticCoverage(facts({ semanticVectorCount: null }))).toBeNull();
    expect(semanticCoverage(facts({ semanticPendingCount: null }))).toBeNull();
    expect(semanticCoverage(facts())).toBeNull();
  });
});

describe("modelFitVerdict", () => {
  // 16 GiB of RAM: a quarter is ~4.3 GB, a half ~8.6 GB.
  it("grades the download against this machine's memory", () => {
    expect(modelFitVerdict(facts(), 1 * GB)?.tone).toBe("ok");
    expect(modelFitVerdict(facts(), 6 * GB)?.tone).toBe("warn");
    expect(modelFitVerdict(facts(), 12 * GB)?.tone).toBe("bad");
  });

  it("free disk bites first — an undownloadable model is a fact, not a judgement", () => {
    const v = modelFitVerdict(facts({ diskFreeBytes: 2 * GB }), 3 * GB);
    expect(v?.tone).toBe("bad");
    expect(v?.label).toContain("free");
  });

  it("claims nothing without a real denominator (G8)", () => {
    expect(modelFitVerdict(facts(), null)).toBeNull();
    expect(modelFitVerdict(facts(), 0)).toBeNull();
    expect(modelFitVerdict(null, 1 * GB)).toBeNull();
    expect(modelFitVerdict(facts({ totalRamBytes: null, diskFreeBytes: null }), 1 * GB)).toBeNull();
  });
});

describe("retentionFootprint", () => {
  it("places the marker on the kept-vs-free axis", () => {
    // 10 GB/day over 30 days = 300 GB kept, against 200 GB free -> 300/500.
    const a = retentionFootprint(facts(), "days_30");
    expect(a?.percent).toBe(60);
    expect(a?.marker).toContain("you are here");
  });

  it("draws no axis for a window with no ceiling, or with nothing measured", () => {
    expect(retentionFootprint(facts(), "never")).toBeNull();
    expect(retentionFootprint(facts({ measuredBytesPerDay: null }), "days_30")).toBeNull();
    expect(retentionFootprint(facts({ diskFreeBytes: null }), "days_30")).toBeNull();
    expect(retentionFootprint(null, "days_30")).toBeNull();
  });
});
