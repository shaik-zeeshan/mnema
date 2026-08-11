// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
// Test 7 (gate predicates) and test 8 (re-entry) from PLAN.md.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  captureStorageBlockReason,
  storageNeedBytes,
  RESERVE_FLOOR_BYTES,
  type CaptureStorageGateInput,
} from "./gates";
import { resolveSetup, workListBytes } from "./resolve-setup";

const GB = 1024 * 1024 * 1024;
/** The default work-list (SPEC.md) and the default capture rate (2 s → 405 MB/day). */
const WORK_LIST = 1_115_434_189;
const INTERVAL_S = 2;

function input(overrides: Partial<CaptureStorageGateInput> = {}): CaptureStorageGateInput {
  return {
    probe: { exists: true, writable: true, freeBytes: 100 * GB },
    requiredBytes: WORK_LIST,
    captureIntervalSeconds: INTERVAL_S,
    customResolutionErrors: [],
    customBitrateErrors: [],
    ...overrides,
  };
}

describe("test 7 — capture & storage gate predicates", () => {
  test("a healthy path with room passes", () => {
    expect(captureStorageBlockReason(input())).toBeNull();
  });

  test("an unwritable path blocks", () => {
    const reason = captureStorageBlockReason(
      input({ probe: { exists: true, writable: false, freeBytes: 100 * GB } }),
    );
    expect(reason).toBe("That folder is not writable. Choose another folder.");
  });

  test("a missing path blocks", () => {
    const reason = captureStorageBlockReason(
      input({ probe: { exists: false, writable: false, freeBytes: 100 * GB } }),
    );
    expect(reason).toBe("That folder doesn't exist. Choose another folder.");
  });

  test("insufficient disk for the work-list blocks, naming both figures", () => {
    const reason = captureStorageBlockReason(
      input({ probe: { exists: true, writable: true, freeBytes: 412_000_000 } }),
    );
    expect(reason).toContain("Not enough room");
    expect(reason).toContain("free");
    expect(reason).toContain("needed");
  });

  test("free space exactly equal to reserve + downloads + a day passes", () => {
    const need = storageNeedBytes(WORK_LIST, INTERVAL_S);
    expect(
      captureStorageBlockReason(
        input({ probe: { exists: true, writable: true, freeBytes: need } }),
      ),
    ).toBeNull();
    expect(
      captureStorageBlockReason(
        input({ probe: { exists: true, writable: true, freeBytes: need - 1 } }),
      ),
    ).not.toBeNull();
  });

  test("a 1.3 GB volume blocks — the downloads alone do not clear the reserve", () => {
    const reason = captureStorageBlockReason(
      input({ probe: { exists: true, writable: true, freeBytes: 1.3e9 } }),
    );
    expect(reason).toContain("Not enough room for the downloads");
  });

  test("a volume that fits the models but not a day of capture blocks, with its own reason", () => {
    // Reserve + work-list + 100 MB: every download lands, and then there is
    // nowhere to record. 405 MB/day at the 2 s default does not fit.
    const freeBytes = RESERVE_FLOOR_BYTES + WORK_LIST + 100e6;
    const reason = captureStorageBlockReason(
      input({ probe: { exists: true, writable: true, freeBytes } }),
    );
    expect(reason).toContain("Not enough room to record a day of capture");
    expect(reason).not.toContain("for the downloads");

    // A slower capture rate is a real escape; dropping Semantic Search is not —
    // the same volume minus nomic's 548 MB still cannot hold a day.
    expect(
      captureStorageBlockReason(
        input({ probe: { exists: true, writable: true, freeBytes }, captureIntervalSeconds: 60 }),
      ),
    ).toBeNull();
  });

  test("the reserve mirrors the backend's RESERVE_FLOOR_BYTES", () => {
    const rust = readFileSync(
      new URL("../../../src-tauri/src/native_capture/disk_space.rs", import.meta.url),
      "utf8",
    );
    // Anchored, not `toContain`: a bare substring match still passes if the Rust
    // constant grows a factor (`... * 1024 * 2`), which would silently let the
    // onboarding gate under-reserve against what the capture pipeline enforces.
    expect(rust).toMatch(/RESERVE_FLOOR_BYTES: u64 = 1024 \* 1024 \* 1024;/);
    expect(RESERVE_FLOOR_BYTES).toBe(1024 * 1024 * 1024);
  });

  test("an unmeasured path or unreadable volume never blocks", () => {
    expect(captureStorageBlockReason(input({ probe: null }))).toBeNull();
    expect(
      captureStorageBlockReason(
        input({ probe: { exists: true, writable: true, freeBytes: null } }),
      ),
    ).toBeNull();
  });

  test("an out-of-range custom resolution or bitrate blocks", () => {
    expect(
      captureStorageBlockReason(input({ customResolutionErrors: ["Width must be 16–8192."] })),
    ).toBe("Width must be 16–8192.");
    expect(
      captureStorageBlockReason(input({ customBitrateErrors: ["Bitrate must be 1–40 Mbps."] })),
    ).toBe("Bitrate must be 1–40 Mbps.");
  });

  test("the storage gate's requirement grows with capture resolution", () => {
    const need720 = storageNeedBytes(0, 2, 1280 * 720);
    const need1080 = storageNeedBytes(0, 2, 1920 * 1080);
    expect(need1080).toBeGreaterThan(need720);
    // 720p is the anchor: omitting `videoPixels` prices exactly the 720p need.
    expect(storageNeedBytes(0, 2)).toBe(need720);

    // Free space strictly between the two needs: the SAME volume passes at
    // 720p and blocks at 1080p — the pixels reach the decision, not just the
    // leaf arithmetic.
    const freeBytes = Math.round((need720 + need1080) / 2);
    const probe = { exists: true, writable: true, freeBytes };
    expect(
      captureStorageBlockReason(
        input({ requiredBytes: 0, probe, videoPixels: 1280 * 720 }),
      ),
    ).toBeNull();
    expect(
      captureStorageBlockReason(
        input({ requiredBytes: 0, probe, videoPixels: 1920 * 1080 }),
      ),
    ).not.toBeNull();
  });

  test("nothing else gates — an empty work-list on a roomy disk still passes", () => {
    expect(
      captureStorageBlockReason(
        input({ requiredBytes: 0, probe: { exists: true, writable: true, freeBytes: 100 * GB } }),
      ),
    ).toBeNull();
  });
});

describe("test 8 — re-entry", () => {
  const permissions = { screen: true, microphone: true, systemAudio: true };
  const nothingInstalled = {
    speakerAnalysis: false,
    whisperBase: false,
    semanticSearch: false,
  };

  test("a deliberately disabled feature stays disabled after re-resolving", () => {
    const first = resolveSetup(permissions, nothingInstalled, null);
    expect(first.features.semanticSearch).toBe(true);

    // The user turns Semantic Search off, finishes, and re-enters onboarding.
    const saved = {
      features: { ...featuresOf(first), semanticSearch: false },
      excludedApps: ["com.apple.Passwords"],
    };
    const second = resolveSetup(permissions, nothingInstalled, saved);

    expect(second.features.semanticSearch).toBe(false);
    // …and its 550 MB download is not re-proposed.
    expect(second.workList.some((item) => item.subsystem === "semanticSearch")).toBe(false);
    expect(workListBytes(second.workList)).toBeLessThan(workListBytes(first.workList));
  });

  test("re-entry never re-seeds privacy-listed apps or re-ticks AI features", () => {
    const second = resolveSetup(permissions, nothingInstalled, {
      excludedApps: [],
      features: { aiFeatures: false },
    });
    expect(second.applyRecommendedExcludedApps).toBe(false);
    expect(second.excludedApps).toEqual([]);
    expect(second.features.aiFeatures).toBe(false);
  });

  test("a disabled audio source stays off and keeps its cascade off", () => {
    const second = resolveSetup(permissions, nothingInstalled, {
      features: { microphone: false, systemAudio: false },
    });
    expect(second.features.microphone).toBe(false);
    expect(second.features.transcription).toBe(false);
    expect(second.features.speakerSeparation).toBe(false);
    // Only Semantic Search (no audio dependency) is left to fetch.
    expect(second.workList.map((item) => item.subsystem)).toEqual(["semanticSearch"]);
  });
});

function featuresOf(resolved: ReturnType<typeof resolveSetup>) {
  const f = resolved.features;
  return {
    screen: f.screen,
    microphone: f.microphone,
    systemAudio: f.systemAudio,
    ocr: f.ocr,
    transcription: f.transcription,
    speakerSeparation: f.speakerSeparation,
    semanticSearch: f.semanticSearch,
    aiFeatures: f.aiFeatures,
    privacy: f.privacy,
  };
}

describe("regression — the gate's non-probe branches", () => {
  // `captureStorageBlockReason` returns the custom-range errors from OUTSIDE the
  // `if (probe)` block, so an unmeasurable volume must still not let an invalid
  // resolution through: those values serialize as `null` and break the backend
  // save at the Finale. Every other case in this file supplies a probe, so
  // moving that return inside the block would keep them all green.
  test("a custom-range error blocks even when the volume could not be measured", () => {
    expect(
      captureStorageBlockReason(
        input({ probe: null, customResolutionErrors: ["Width must be 16–8192 px."] }),
      ),
    ).toBe("Width must be 16–8192 px.");
    expect(
      captureStorageBlockReason(
        input({ probe: null, customBitrateErrors: ["Bitrate must be 1–40 Mbps."] }),
      ),
    ).toBe("Bitrate must be 1–40 Mbps.");
  });

  // A MEASURED zero is the most determined free-space reading there is (the
  // volume is full), but `formatBytes` renders every non-positive value with its
  // can't-determine sentinel — so the gate that fires on a full disk quoted
  // "unknown size free", the exact words the UNMEASURABLE case uses. The whole
  // discipline here (ADR 0040) is that measuring and failing to measure are
  // different things; the blocking copy has to say which one happened.
  test("a full volume states zero free space, not the can't-determine sentinel", () => {
    const reason = captureStorageBlockReason(
      input({ probe: { exists: true, writable: true, freeBytes: 0 } }),
    );
    expect(reason).not.toBeNull();
    expect(reason).not.toContain("unknown size");
    expect(reason).toContain("0 B free");
  });

  // Re-entry with every model already installed: `requiredBytes` is 0, so
  // `free < RESERVE_FLOOR_BYTES + 0` degenerates to "below the safety reserve".
  // Blaming downloads that do not exist names a term worth zero bytes.
  test("a nearly full volume with nothing to download names the reserve, not downloads", () => {
    const reason = captureStorageBlockReason(
      input({ requiredBytes: 0, probe: { exists: true, writable: true, freeBytes: 800e6 } }),
    );
    expect(reason).not.toBeNull();
    expect(reason).not.toContain("for the downloads");
  });
});
