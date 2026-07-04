// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  defaultTranscriptionModelIdForProvider,
  shouldConfirmDeepgramSwitch,
} from "./models-format";

describe("defaultTranscriptionModelIdForProvider", () => {
  it("returns the bundled default per provider (deepgram → nova-3, else null)", () => {
    expect(defaultTranscriptionModelIdForProvider("deepgram")).toBe("nova-3");
    expect(defaultTranscriptionModelIdForProvider("local_whisper")).toBe("base");
    expect(defaultTranscriptionModelIdForProvider("parakeet")).toBe(
      "parakeet-tdt-0.6b-v3-onnx-int8",
    );
    expect(defaultTranscriptionModelIdForProvider("apple_speech_on_device")).toBeNull();
    expect(defaultTranscriptionModelIdForProvider("something_unknown")).toBeNull();
  });
});

describe("shouldConfirmDeepgramSwitch", () => {
  it("prompts only when switching to deepgram from a non-deepgram provider", () => {
    expect(shouldConfirmDeepgramSwitch("deepgram", "local_whisper")).toBe(true);
    expect(shouldConfirmDeepgramSwitch("deepgram", "apple_speech_on_device")).toBe(true);
    expect(shouldConfirmDeepgramSwitch("deepgram", "deepgram")).toBe(false);
    expect(shouldConfirmDeepgramSwitch("local_whisper", "deepgram")).toBe(false);
    expect(shouldConfirmDeepgramSwitch("parakeet", "local_whisper")).toBe(false);
  });
});
