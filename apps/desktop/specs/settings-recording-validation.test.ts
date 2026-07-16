import { describe, expect, test } from "bun:test";
import {
  parseCustomDimension,
  customResolutionErrors,
  customResolutionBlocked,
  customBitrateErrors,
  customBitrateBlocked,
  recValidationErrors,
  recSaveBlocked,
  recDomainSaveBlocked,
  type RecordingValidationState,
  type RecordingValidationGates,
} from "../src/lib/settings/state/recording-validation";

// A clean, valid baseline: screen on, save directory set, both resolution and
// bitrate in non-custom modes, no entangled raw text.
const state = (over: Partial<RecordingValidationState> = {}): RecordingValidationState => ({
  draftCaptureScreen: true,
  draftCaptureMicrophone: false,
  draftCaptureSystemAudio: false,
  draftSaveDirectory: "/tmp/captures",
  draftResolutionMode: "original",
  draftBitrateMode: "preset",
  customWidthRaw: "",
  customHeightRaw: "",
  draftCustomMbpsRaw: "",
  ...over,
});

const gates = (over: Partial<RecordingValidationGates> = {}): RecordingValidationGates => ({
  resolutionSupportPendingForNonOriginal: false,
  ...over,
});

describe("parseCustomDimension", () => {
  test("parses a plain integer string", () => {
    expect(parseCustomDimension("1920")).toBe(1920);
  });
  test("empty string -> null", () => {
    expect(parseCustomDimension("")).toBeNull();
  });
  test("non-numeric chars -> null", () => {
    expect(parseCustomDimension("12a")).toBeNull();
  });
  test("negative (leading -) -> null (regex rejects sign)", () => {
    expect(parseCustomDimension("-3")).toBeNull();
  });
  test("zero parses to 0", () => {
    expect(parseCustomDimension("0")).toBe(0);
  });
});

describe("customResolutionErrors", () => {
  test("non-custom mode -> no errors", () => {
    expect(customResolutionErrors(state({ draftResolutionMode: "original" }))).toEqual([]);
    expect(customResolutionErrors(state({ draftResolutionMode: "preset" }))).toEqual([]);
  });
  test("valid in-range custom resolution -> no errors", () => {
    const errs = customResolutionErrors(
      state({ draftResolutionMode: "custom", customWidthRaw: "1920", customHeightRaw: "1080" }),
    );
    expect(errs).toEqual([]);
  });
  test("width out of range (below 16) -> range error", () => {
    const errs = customResolutionErrors(
      state({ draftResolutionMode: "custom", customWidthRaw: "8", customHeightRaw: "1080" }),
    );
    expect(errs).toContain("Width must be between 16 and 8192.");
  });
  test("height out of range (above 8192) -> range error", () => {
    const errs = customResolutionErrors(
      state({ draftResolutionMode: "custom", customWidthRaw: "1920", customHeightRaw: "9000" }),
    );
    expect(errs).toContain("Height must be between 16 and 8192.");
  });
  test("non-integer width raw -> integer error", () => {
    const errs = customResolutionErrors(
      state({ draftResolutionMode: "custom", customWidthRaw: "12a", customHeightRaw: "1080" }),
    );
    expect(errs).toContain("Width must be an integer.");
  });
  test("non-integer height raw -> integer error", () => {
    const errs = customResolutionErrors(
      state({ draftResolutionMode: "custom", customWidthRaw: "1920", customHeightRaw: "10x" }),
    );
    expect(errs).toContain("Height must be an integer.");
  });
  test("empty width or height -> both-required error", () => {
    const errs = customResolutionErrors(
      state({ draftResolutionMode: "custom", customWidthRaw: "", customHeightRaw: "1080" }),
    );
    expect(errs).toContain("Both width and height are required for custom mode.");
  });
});

describe("customResolutionBlocked", () => {
  test("non-custom mode is never blocked", () => {
    expect(customResolutionBlocked(state({ draftResolutionMode: "original" }))).toBe(false);
  });
  test("custom mode with valid dims -> not blocked", () => {
    expect(
      customResolutionBlocked(
        state({ draftResolutionMode: "custom", customWidthRaw: "1920", customHeightRaw: "1080" }),
      ),
    ).toBe(false);
  });
  test("custom mode with errors -> blocked", () => {
    expect(
      customResolutionBlocked(
        state({ draftResolutionMode: "custom", customWidthRaw: "", customHeightRaw: "" }),
      ),
    ).toBe(true);
  });
});

describe("customBitrateErrors", () => {
  test("non-custom mode -> no errors", () => {
    expect(customBitrateErrors(state({ draftBitrateMode: "preset" }))).toEqual([]);
  });
  test("empty custom bitrate -> required error", () => {
    const errs = customBitrateErrors(state({ draftBitrateMode: "custom", draftCustomMbpsRaw: "" }));
    expect(errs).toContain("Custom bitrate is required (1–40 Mbps, whole number).");
  });
  test("non-numeric custom bitrate -> whole-number error", () => {
    const errs = customBitrateErrors(state({ draftBitrateMode: "custom", draftCustomMbpsRaw: "12.5" }));
    expect(errs).toContain("Bitrate must be a whole number of Mbps (e.g. 12).");
  });
  test("zero custom bitrate -> positive-whole-number error", () => {
    const errs = customBitrateErrors(state({ draftBitrateMode: "custom", draftCustomMbpsRaw: "0" }));
    expect(errs).toContain("Bitrate must be a positive whole number.");
  });
  test("over-max custom bitrate -> not-exceed error", () => {
    const errs = customBitrateErrors(state({ draftBitrateMode: "custom", draftCustomMbpsRaw: "41" }));
    expect(errs).toContain("Bitrate must not exceed 40 Mbps.");
  });
  test("valid in-range custom bitrate -> no errors", () => {
    expect(
      customBitrateErrors(state({ draftBitrateMode: "custom", draftCustomMbpsRaw: "12" })),
    ).toEqual([]);
  });
});

describe("customBitrateBlocked", () => {
  test("non-custom mode is never blocked", () => {
    expect(customBitrateBlocked(state({ draftBitrateMode: "preset" }))).toBe(false);
  });
  test("custom mode with valid mbps -> not blocked", () => {
    expect(
      customBitrateBlocked(state({ draftBitrateMode: "custom", draftCustomMbpsRaw: "12" })),
    ).toBe(false);
  });
  test("custom mode with empty mbps -> blocked", () => {
    expect(
      customBitrateBlocked(state({ draftBitrateMode: "custom", draftCustomMbpsRaw: "" })),
    ).toBe(true);
  });
});

describe("recValidationErrors", () => {
  test("clean state -> no errors", () => {
    expect(recValidationErrors(state(), gates())).toEqual([]);
  });
  test("no capture source -> error", () => {
    const errs = recValidationErrors(
      state({ draftCaptureScreen: false, draftCaptureMicrophone: false, draftCaptureSystemAudio: false }),
      gates(),
    );
    expect(errs).toContain(
      "At least one capture source (Screen, Microphone, or System Audio) must be enabled.",
    );
  });
  // ADR 0052: system audio is an independent capture family — audio-only
  // sessions (screen + mic off, system audio on) are allowed. The Settings
  // System Audio toggle was decoupled from Screen, so this state is reachable
  // and must not raise a validation error.
  test("audio-only (system audio without screen) -> no error", () => {
    const errs = recValidationErrors(
      state({ draftCaptureScreen: false, draftCaptureMicrophone: false, draftCaptureSystemAudio: true }),
      gates(),
    );
    expect(errs).toEqual([]);
  });
  test("resolution support pending -> error", () => {
    const errs = recValidationErrors(state(), gates({ resolutionSupportPendingForNonOriginal: true }));
    expect(errs).toContain(
      "Wait for capture support to load before saving preset/custom resolution.",
    );
  });
});

describe("recSaveBlocked", () => {
  test("empty save directory -> blocked", () => {
    expect(recSaveBlocked(state({ draftSaveDirectory: "" }), gates())).toBe(true);
  });
  test("clean state -> not blocked", () => {
    expect(recSaveBlocked(state(), gates())).toBe(false);
  });
  test("custom resolution blocked propagates -> blocked", () => {
    expect(
      recSaveBlocked(
        state({ draftResolutionMode: "custom", customWidthRaw: "", customHeightRaw: "" }),
        gates(),
      ),
    ).toBe(true);
  });
  test("custom bitrate blocked propagates -> blocked", () => {
    expect(
      recSaveBlocked(state({ draftBitrateMode: "custom", draftCustomMbpsRaw: "" }), gates()),
    ).toBe(true);
  });
});

describe("recDomainSaveBlocked", () => {
  test("capture_sources: no source -> blocked", () => {
    expect(
      recDomainSaveBlocked(
        "capture_sources",
        state({ draftCaptureScreen: false, draftCaptureMicrophone: false, draftCaptureSystemAudio: false }),
        gates(),
      ),
    ).toBe(true);
  });
  // ADR 0052: audio-only must persist — the capture_sources autosave must not
  // block system-audio-on + screen-off, or the toggle silently never saves.
  test("capture_sources: audio-only (system audio without screen) -> not blocked", () => {
    expect(
      recDomainSaveBlocked(
        "capture_sources",
        state({ draftCaptureScreen: false, draftCaptureMicrophone: false, draftCaptureSystemAudio: true }),
        gates(),
      ),
    ).toBe(false);
  });
  test("capture_sources: valid -> not blocked", () => {
    expect(recDomainSaveBlocked("capture_sources", state(), gates())).toBe(false);
  });
  test("video: custom resolution blocked -> blocked", () => {
    expect(
      recDomainSaveBlocked(
        "video",
        state({ draftResolutionMode: "custom", customWidthRaw: "", customHeightRaw: "" }),
        gates(),
      ),
    ).toBe(true);
  });
  test("video: resolution support pending -> blocked", () => {
    expect(
      recDomainSaveBlocked("video", state(), gates({ resolutionSupportPendingForNonOriginal: true })),
    ).toBe(true);
  });
  test("video: clean -> not blocked", () => {
    expect(recDomainSaveBlocked("video", state(), gates())).toBe(false);
  });
  test("storage: empty save directory -> blocked", () => {
    expect(recDomainSaveBlocked("storage", state({ draftSaveDirectory: "" }), gates())).toBe(true);
  });
  test("storage: directory set -> not blocked", () => {
    expect(recDomainSaveBlocked("storage", state(), gates())).toBe(false);
  });
  test("a benign domain -> never blocked", () => {
    expect(recDomainSaveBlocked("display", state({ draftSaveDirectory: "" }), gates())).toBe(false);
  });
});
