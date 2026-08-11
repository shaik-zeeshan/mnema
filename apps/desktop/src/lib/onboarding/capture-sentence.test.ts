// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
//
// Slice 6. The first block is the mockup's own `console.assert` self-check
// (`docs/onboarding/mockups/input-components/parts/sentence.part.html`) ported
// to a real test: it pins the manifest sums, the 270 MB/day capture anchor and
// the ghost windows. The rest drives every failure state the sentence must
// render — the value it commits, never the geometry it draws it with.
import { describe, expect, test } from "bun:test";
import {
  DEFAULT_FOLDER_LABEL,
  RATE_GHOST_WIDTH,
  RETENTION_STOPS,
  dailyBytes,
  ghostWindow,
  humanDuration,
  planSegments,
  retentionIndex,
  sentencePath,
  sentenceVerdict,
  volumeLabel,
} from "./capture-sentence";
import {
  CAPTURE_INTERVAL_LADDER_S,
  DEFAULT_CAPTURE_INTERVAL_S,
} from "../components/capture-rate";
import { ANCHOR_INTERVAL_S, estimateDailyStorageMb } from "./disk-estimate";
import {
  RESERVE_FLOOR_BYTES,
  captureStorageBlockReason,
  storageNeedBytes,
} from "./gates";
import { formatBytes } from "../settings/state/format";

/** The default work-list (SPEC.md): speakrs + Whisper base + nomic. */
const SPEAKRS = 419_482_724;
const WHISPER_BASE = 147_951_465;
const NOMIC = 548_000_000;
const WORK_LIST = SPEAKRS + WHISPER_BASE + NOMIC;

const text = (segs) => segs.map((s) => s.text).join("");

function input(overrides = {}) {
  return {
    intervalSeconds: DEFAULT_CAPTURE_INTERVAL_S,
    retention: "never",
    path: "/Users/you/.mnema",
    probe: { exists: true, writable: true, freeBytes: 214e9 },
    probeState: "done",
    requiredBytes: WORK_LIST,
    semanticSearchOn: true,
    ...overrides,
  };
}

describe("the mockup's self-check, ported", () => {
  test("the capture ladder's default stop is 2 s", () => {
    expect(CAPTURE_INTERVAL_LADDER_S[3]).toBe(2);
    expect(DEFAULT_CAPTURE_INTERVAL_S).toBe(2);
  });

  test("`never` is first, is the default, and is never punished", () => {
    expect(RETENTION_STOPS[0].id).toBe("never");
    expect(RETENTION_STOPS[0].days).toBeNull();
    expect(retentionIndex("never")).toBe(0);
  });

  test("405 MB/day at the 2 s default, from the 270 MB/day 3 s anchor", () => {
    expect(dailyBytes(2)).toBe(405e6);
    expect(formatBytes(dailyBytes(2))).toBe("405.0 MB");
  });

  test("the anchor itself round-trips", () => {
    expect(dailyBytes(3)).toBe(270e6);
    expect(formatBytes(dailyBytes(3))).toBe("270.0 MB");
  });

  test("the default work list is 1,115,434,189 B", () => {
    expect(WORK_LIST).toBe(1_115_434_189);
  });

  test("the 30-day ceiling at the default rate is 12.2 GB", () => {
    expect(formatBytes(dailyBytes(2) * 30)).toBe("12.2 GB");
  });

  test("the rate window is always 5 wide, clamped at both ends", () => {
    expect(ghostWindow(0, 11, RATE_GHOST_WIDTH)).toEqual({ start: 0, end: 5 });
    expect(ghostWindow(10, 11, RATE_GHOST_WIDTH)).toEqual({ start: 6, end: 11 });
    expect(ghostWindow(3, 11, RATE_GHOST_WIDTH)).toEqual({ start: 1, end: 6 });
    for (let i = 0; i < CAPTURE_INTERVAL_LADDER_S.length; i++) {
      const win = ghostWindow(i, CAPTURE_INTERVAL_LADDER_S.length, RATE_GHOST_WIDTH);
      expect(win.end - win.start).toBe(RATE_GHOST_WIDTH);
      expect(i).toBeGreaterThanOrEqual(win.start);
      expect(i).toBeLessThan(win.end);
    }
  });

  test("retention prints all four stops at every position", () => {
    for (let i = 0; i < RETENTION_STOPS.length; i++) {
      expect(ghostWindow(i, RETENTION_STOPS.length, RETENTION_STOPS.length)).toEqual({
        start: 0,
        end: 4,
      });
    }
  });

  test("the Semantic Search escape does not rescue the SD card", () => {
    const sd = 0.9e9;
    // It fails today's download-only gate…
    expect(sd).toBeLessThan(WORK_LIST);
    // …and still has nowhere to record after dropping nomic.
    expect(sd - (SPEAKRS + WHISPER_BASE) - RESERVE_FLOOR_BYTES).toBeLessThan(
      dailyBytes(2),
    );
  });
});

describe("the plan line always reads rate · horizon", () => {
  test("`never` prints a first-year projection, not a hole", () => {
    const line = text(planSegments(2, "never"));
    expect(line).toContain("405.0 MB a day");
    expect(line).toContain("about 147.8 GB in the first year, and it keeps going");
  });

  test("a bounded window prints a steady ceiling in the same position", () => {
    const line = text(planSegments(2, "days_30"));
    expect(line).toContain("405.0 MB a day");
    expect(line).toContain("12.2 GB held, then it stops growing");
  });

  test("the horizon moves when the rate moves", () => {
    expect(text(planSegments(60, "days_30"))).toContain("405.0 MB held");
    expect(text(planSegments(60, "days_30"))).toContain("13.5 MB a day");
  });

  test("the sentence's plan prices the draft resolution", () => {
    // The daily figure the plan renders is the resolution-scaled estimate, not
    // the 720p anchor: at the anchor interval, a 1080p draft prints the 1080p
    // day and a 540p draft prints a strictly smaller one.
    const hi = sentenceVerdict(
      input({ intervalSeconds: ANCHOR_INTERVAL_S, videoPixels: 1920 * 1080 }),
    );
    expect(hi.plan[0].text).toBe(
      formatBytes(estimateDailyStorageMb(ANCHOR_INTERVAL_S, 1920 * 1080) * 1e6),
    );
    expect(text(hi.plan)).toContain(" a day");

    const lo = sentenceVerdict(
      input({ intervalSeconds: ANCHOR_INTERVAL_S, videoPixels: 960 * 540 }),
    );
    expect(lo.plan[0].text).toBe(
      formatBytes(estimateDailyStorageMb(ANCHOR_INTERVAL_S, 960 * 540) * 1e6),
    );
    expect(estimateDailyStorageMb(ANCHOR_INTERVAL_S, 960 * 540)).toBeLessThan(
      estimateDailyStorageMb(ANCHOR_INTERVAL_S, 1920 * 1080),
    );
    expect(lo.plan[0].text).not.toBe(hi.plan[0].text);
  });
});

describe("every failure state the sentence must render", () => {
  test("probing: the sentence trails off and says nothing yet", () => {
    const v = sentenceVerdict(input({ probe: null, probeState: "checking" }));
    expect(v.probing).toBe(true);
    expect(v.clause).toBeNull();
    expect(v.verdict).toEqual([]);
    expect(v.blocking).toBe(false);
    // The plan is still printed while the disk is being read.
    expect(text(v.plan)).toContain("405.0 MB a day");
  });

  test("probe failed: distinguishable from not-yet-probed, and never blocks", () => {
    const v = sentenceVerdict(input({ probe: null, probeState: "failed" }));
    expect(v.probing).toBe(false);
    expect(v.tone).toBe("warn");
    expect(text(v.verdict)).toContain("couldn't check that folder");
    expect(text(v.verdict)).toContain("doesn't block anything");
    expect(v.blocking).toBe(false);
    expect(v.repairs.map((r) => r.act)).toContain("recheck");
  });

  test("folder missing on a connected volume", () => {
    const v = sentenceVerdict(
      input({
        path: "/Volumes/Samsung T7/Mnema",
        probe: { exists: false, writable: false, freeBytes: 1.42e12 },
      }),
    );
    expect(v.clause).toBe("that folder isn't there yet.");
    expect(v.blocking).toBe(true);
    expect(text(v.verdict)).toBe(
      "1.4 TB free on Samsung T7 — only the folder is missing.",
    );
    expect(v.repairs.map((r) => r.act)).toEqual(["pick", "recheck"]);
  });

  test("read-only volume names the volume, and says the probe proved it", () => {
    const v = sentenceVerdict(
      input({
        path: "/Volumes/Time Machine/Mnema",
        probe: { exists: true, writable: false, freeBytes: 180e9 },
      }),
    );
    expect(v.clause).toBe("Time Machine is read-only.");
    expect(v.blocking).toBe(true);
    expect(text(v.verdict)).toContain("proven, not guessed");
  });

  test("free space unknown warns, breaks nothing, and NEVER blocks (ADR 0040)", () => {
    const v = sentenceVerdict(
      input({
        path: "/Volumes/Archive/Mnema",
        probe: { exists: true, writable: true, freeBytes: null },
      }),
    );
    expect(v.tone).toBe("warn");
    expect(v.clause).toBeNull();
    expect(v.blocking).toBe(false);
    expect(text(v.verdict)).toContain("Free space on Archive couldn't be read.");
    expect(text(v.verdict)).toContain("doesn't block anything");
  });

  test("the downloads don't fit", () => {
    const v = sentenceVerdict(
      input({
        path: "/Volumes/SD Card/Mnema",
        probe: { exists: true, writable: true, freeBytes: 0.9e9 },
      }),
    );
    expect(v.clause).toBe("there isn't room for the models yet.");
    expect(v.blocking).toBe(true);
    expect(text(v.verdict)).toContain("short before recording even starts");
    // Any total containing nomic reads "about".
    expect(text(v.verdict)).toContain("The downloads are about 1.1 GB");
    expect(v.repairs.map((r) => r.act)).toEqual(["nosemantic", "pick"]);
  });

  // A MEASURED zero is the most determined free-space reading there is, but
  // `formatBytes` renders every non-positive value as "unknown size" — its
  // can't-determine sentinel. So a genuinely full volume said the same words as
  // the volume nobody could read, and the missing-folder branch went further and
  // told the user "only the folder is missing" while quoting an unknown size.
  test("a full volume states zero free space, not the can't-determine sentinel", () => {
    const v = sentenceVerdict(
      input({ probe: { exists: true, writable: true, freeBytes: 0 } }),
    );
    expect(v.blocking).toBe(true);
    expect(text(v.verdict)).not.toContain("unknown size");
    expect(text(v.verdict)).toContain("0 B free");
  });

  test("a missing folder on a full volume still states zero, not unknown", () => {
    const v = sentenceVerdict(
      input({ probe: { exists: false, writable: false, freeBytes: 0 } }),
    );
    expect(v.clause).toBe("that folder isn't there yet.");
    expect(text(v.verdict)).not.toContain("unknown size");
    expect(text(v.verdict)).toContain("0 B free");
  });

  test("the downloads fit but a day of capture does not", () => {
    const free = 2.4e9;
    const v = sentenceVerdict(
      input({
        path: "/Volumes/USB Stick/Mnema",
        probe: { exists: true, writable: true, freeBytes: free },
      }),
    );
    // The shipped download-only check would have passed this volume.
    expect(free).toBeGreaterThan(RESERVE_FLOOR_BYTES + WORK_LIST);
    expect(free).toBeLessThan(storageNeedBytes(WORK_LIST, 2));
    expect(v.clause).toBe("there isn't room for a day of recording.");
    expect(v.blocking).toBe(true);
    expect(text(v.verdict)).toContain("The downloads fit; the capture does not.");
    expect(text(v.verdict)).toContain("one day costs 405.0 MB");
    expect(v.repairs.map((r) => r.act)).toEqual(["pick", "slower"]);
  });

  test("a disconnected volume: missing AND unmeasurable", () => {
    const v = sentenceVerdict(
      input({
        path: "/Volumes/Samsung T7/Mnema",
        probe: { exists: false, writable: false, freeBytes: null },
      }),
    );
    expect(v.clause).toBe("that drive isn't connected right now.");
    expect(v.blocking).toBe(true);
    expect(text(v.verdict)).toContain("won't record to a volume it can't see");
    expect(v.repairs[0].label).toBe(`Use ${DEFAULT_FOLDER_LABEL} instead`);
  });

  test("an unmeasurable path that is NOT under /Volumes is just missing", () => {
    const v = sentenceVerdict(
      input({
        path: "/Users/you/Desktop/Gone/Mnema",
        probe: { exists: false, writable: false, freeBytes: null },
      }),
    );
    expect(v.clause).toBe("that folder isn't there yet.");
    expect(text(v.verdict)).toContain("Free space unknown on your startup disk");
  });
});

describe("the healthy readings", () => {
  test("a roomy disk with `never` reports how long it lasts", () => {
    const v = sentenceVerdict(input());
    expect(v.tone).toBe("ok");
    expect(v.clause).toBeNull();
    expect(v.blocking).toBe(false);
    expect(text(v.verdict)).toContain("room for about 1.4 years at this rate");
    expect(v.repairs).toEqual([]);
  });

  test("a tight disk with `never` offers the two real escapes", () => {
    const v = sentenceVerdict(
      input({ probe: { exists: true, writable: true, freeBytes: 20e9 } }),
    );
    expect(v.tone).toBe("warn");
    expect(v.blocking).toBe(false);
    expect(v.repairs.map((r) => r.act)).toEqual(["keep30", "slower"]);
  });

  test("a bounded window that fits reports the spare room", () => {
    const v = sentenceVerdict(input({ retention: "days_30" }));
    expect(v.tone).toBe("ok");
    expect(text(v.verdict)).toContain("fits, with");
    expect(text(v.verdict)).toContain("stops growing after 30 days");
  });

  test("a bounded window that does not fit says when it fills", () => {
    const v = sentenceVerdict(
      input({ retention: "days_30", probe: { exists: true, writable: true, freeBytes: 8e9 } }),
    );
    expect(v.tone).toBe("warn");
    expect(v.blocking).toBe(false);
    expect(text(v.verdict)).toContain("Holding 30 days takes about 12.2 GB");
    expect(text(v.verdict)).toContain("It fills up after about");
    expect(v.repairs.map((r) => r.act)).toEqual(["slower", "tighten"]);
  });

  test("the verdict never blocks where the gate does not, and always where it does", () => {
    // The one invariant that keeps the printed consequence and the held
    // Continue from disagreeing: both read `storageNeedBytes`.
    for (const free of [0.5e9, 1.5e9, 2.2e9, 2.4e9, 2.6e9, 10e9, 214e9]) {
      const v = sentenceVerdict(
        input({ probe: { exists: true, writable: true, freeBytes: free } }),
      );
      expect(v.blocking).toBe(free < storageNeedBytes(WORK_LIST, 2));
    }
  });

  test("the same invariant holds at a non-720p capture resolution", () => {
    // The sentence now PRINTS a resolution-scaled day (`videoPixels`), so the
    // shortfall it tests must be scaled by the same figure the gate uses — else
    // a 1080p/original draft on a volume that clears the 720p need but not the
    // real one disables Continue while the panel says it fits.
    const pixels = 1920 * 1080;
    for (const free of [2.6e9, 2.8e9, 2.9e9, 3.5e9]) {
      const probe = { exists: true, writable: true, freeBytes: free };
      const v = sentenceVerdict(input({ videoPixels: pixels, probe }));
      const gate = captureStorageBlockReason({
        probe,
        requiredBytes: WORK_LIST,
        captureIntervalSeconds: 2,
        videoPixels: pixels,
        customResolutionErrors: [],
        customBitrateErrors: [],
      });
      expect({ free, blocking: v.blocking }).toEqual({
        free,
        blocking: gate !== null,
      });
    }
  });
});

describe("small pure helpers", () => {
  test("volumeLabel names the mount, else the startup disk", () => {
    expect(volumeLabel("/Volumes/Samsung T7/Mnema")).toBe("Samsung T7");
    expect(volumeLabel("/Users/you/.mnema")).toBe("your startup disk");
    expect(volumeLabel("")).toBe("your startup disk");
  });

  test("sentencePath is home-relative and drops the middle, not the end", () => {
    expect(sentencePath("/Users/you/.mnema")).toBe("~/.mnema");
    expect(sentencePath("/Volumes/T7/Mnema")).toBe("/Volumes/T7/Mnema");
    const long = sentencePath("/Users/you/Documents/Archive/Deep/Nested/Mnema");
    expect(long.startsWith("~/…/")).toBe(true);
    expect(long.endsWith("Mnema")).toBe(true);
    expect(long.length).toBeLessThanOrEqual(28);
  });

  test("humanDuration steps through the units", () => {
    expect(humanDuration(0)).toBe("no time at all");
    expect(humanDuration(0.4)).toBe("less than a day");
    expect(humanDuration(9)).toBe("9 days");
    expect(humanDuration(21)).toBe("3 weeks");
    expect(humanDuration(200)).toBe("7 months");
    expect(humanDuration(730)).toBe("2 years");
  });

  test("humanDuration is singular when the count is one", () => {
    // A tight volume prints this: "it fills up after about 1 days" was the bug.
    expect(humanDuration(1)).toBe("1 day");
    expect(humanDuration(1.4)).toBe("1 day");
    expect(humanDuration(1.6)).toBe("2 days");
    expect(humanDuration(365)).toBe("1 year");
    expect(humanDuration(400)).toBe("1.1 years");
    // Weeks and months can never round to 1 — the branch above them catches it.
    expect(humanDuration(14)).toBe("2 weeks");
    expect(humanDuration(60)).toBe("2 months");
  });
});
