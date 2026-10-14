// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the svelte-check tsconfig.
import { describe, expect, it } from "bun:test";
import { transcriptModelLabel } from "./transcript-model-label";

describe("transcriptModelLabel", () => {
  it("shortens model ids for the drawer strip and keeps the full id in the tip", () => {
    expect(transcriptModelLabel("parakeet", "parakeet-tdt-0.6b-v3-onnx")).toEqual({
      label: "parakeet v3",
      tip: "Parakeet · parakeet-tdt-0.6b-v3-onnx",
    });
    expect(transcriptModelLabel("parakeet", "parakeet-tdt-0.6b-v3-onnx-int8").label).toBe("parakeet v3 int8");
    expect(transcriptModelLabel("deepgram", "nova-3").label).toBe("deepgram nova-3");
    expect(transcriptModelLabel("local_whisper", "base").label).toBe("whisper base");
  });

  it("falls back to the provider alone when there is no model id", () => {
    expect(transcriptModelLabel("apple_speech_on_device", null)).toEqual({
      label: "apple speech",
      tip: "Apple Speech (on-device)",
    });
    expect(transcriptModelLabel("parakeet", null).label).toBe("parakeet");
    expect(transcriptModelLabel("future_thing", "m1").label).toBe("future_thing m1");
  });
});
