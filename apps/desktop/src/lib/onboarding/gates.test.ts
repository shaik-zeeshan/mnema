// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
// Test 7 (gate predicates) and test 8 (re-entry) from PLAN.md.
import { describe, expect, test } from "bun:test";
import { captureStorageBlockReason, type CaptureStorageGateInput } from "./gates";
import { resolveSetup, workListBytes } from "./resolve-setup";

const GB = 1024 * 1024 * 1024;

function input(overrides: Partial<CaptureStorageGateInput> = {}): CaptureStorageGateInput {
  return {
    probe: { exists: true, writable: true, freeBytes: 100 * GB },
    requiredBytes: 1_115_434_189,
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

  test("free space exactly equal to the work-list passes", () => {
    const requiredBytes = 1_000_000;
    expect(
      captureStorageBlockReason(
        input({ requiredBytes, probe: { exists: true, writable: true, freeBytes: requiredBytes } }),
      ),
    ).toBeNull();
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

  test("nothing else gates — an empty work-list on a full disk still passes", () => {
    expect(
      captureStorageBlockReason(
        input({ requiredBytes: 0, probe: { exists: true, writable: true, freeBytes: 0 } }),
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
