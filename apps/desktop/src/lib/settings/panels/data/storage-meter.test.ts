// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import { meterParts, perDayBytes, storageTotal } from "./storage-meter";

const usage = {
  screenBytes: 600,
  microphoneBytes: 200,
  systemAudioBytes: 100,
  indexBytes: 50,
  modelsBytes: 50,
  captureDays: 3,
  freeBytes: null,
};

describe("storage meter", () => {
  it("splits the total into five parts that sum to 100%", () => {
    expect(storageTotal(usage)).toBe(1000);
    const parts = meterParts(usage);
    expect(parts.map((p) => p.label)).toEqual(["screen", "mic", "system", "index", "models"]);
    expect(parts.map((p) => p.pct)).toEqual([60, 20, 10, 5, 5]);
  });

  it("an empty disk is all zero shares, not NaN", () => {
    const empty = { ...usage, screenBytes: 0, microphoneBytes: 0, systemAudioBytes: 0, indexBytes: 0, modelsBytes: 0, captureDays: 0 };
    expect(meterParts(empty).every((p) => p.pct === 0)).toBe(true);
    expect(perDayBytes(empty)).toBeNull();
  });

  it("per-day rate counts capture only, not index or models", () => {
    expect(perDayBytes(usage)).toBe(300);
  });
});
