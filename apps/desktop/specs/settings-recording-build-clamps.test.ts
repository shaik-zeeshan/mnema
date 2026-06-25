import { describe, expect, test } from "bun:test";
import {
  clampAskAiMaxToolCalls,
  clampTranscriptionIdleUnloadSeconds,
  clampTranscriptionChunkSeconds,
  clampOcrTesseractUpscaleFactor,
  ASK_AI_MAX_TOOL_CALL_LIMIT,
} from "../src/lib/settings/state/recording-build";

// clampAskAiMaxToolCalls: 0 (= no cap) passes through; any positive value is
// clamped to [1, 64]. floor() is applied first, so 0.5 -> 0 -> 0.
describe("clampAskAiMaxToolCalls [0,64]", () => {
  test("zero passes through (= no cap)", () => {
    expect(clampAskAiMaxToolCalls(0)).toBe(0);
  });
  test("negative clamps to 0 (no cap)", () => {
    expect(clampAskAiMaxToolCalls(-5)).toBe(0);
  });
  test("above max clamps down to 64", () => {
    expect(clampAskAiMaxToolCalls(65)).toBe(64);
    expect(clampAskAiMaxToolCalls(1000)).toBe(64);
  });
  test("in-range value passes through", () => {
    expect(clampAskAiMaxToolCalls(12)).toBe(12);
  });
  test("boundary values", () => {
    expect(clampAskAiMaxToolCalls(1)).toBe(1);
    expect(clampAskAiMaxToolCalls(64)).toBe(ASK_AI_MAX_TOOL_CALL_LIMIT);
  });
  test("fractional positive floors then clamps to >= 1", () => {
    expect(clampAskAiMaxToolCalls(0.5)).toBe(0);
    expect(clampAskAiMaxToolCalls(1.9)).toBe(1);
  });
});

// clampTranscriptionIdleUnloadSeconds: trunc then clamp to [0, 1800].
describe("clampTranscriptionIdleUnloadSeconds [0,1800]", () => {
  test("below min clamps up to 0", () => {
    expect(clampTranscriptionIdleUnloadSeconds(-1)).toBe(0);
    expect(clampTranscriptionIdleUnloadSeconds(-999)).toBe(0);
  });
  test("above max clamps down to 1800", () => {
    expect(clampTranscriptionIdleUnloadSeconds(1801)).toBe(1800);
    expect(clampTranscriptionIdleUnloadSeconds(100000)).toBe(1800);
  });
  test("in-range value passes through (truncated)", () => {
    expect(clampTranscriptionIdleUnloadSeconds(300)).toBe(300);
    expect(clampTranscriptionIdleUnloadSeconds(300.9)).toBe(300);
  });
  test("boundary values", () => {
    expect(clampTranscriptionIdleUnloadSeconds(0)).toBe(0);
    expect(clampTranscriptionIdleUnloadSeconds(1800)).toBe(1800);
  });
});

// clampTranscriptionChunkSeconds: trunc then clamp to [0, 300].
describe("clampTranscriptionChunkSeconds [0,300]", () => {
  test("below min clamps up to 0", () => {
    expect(clampTranscriptionChunkSeconds(-10)).toBe(0);
  });
  test("above max clamps down to 300", () => {
    expect(clampTranscriptionChunkSeconds(301)).toBe(300);
    expect(clampTranscriptionChunkSeconds(5000)).toBe(300);
  });
  test("in-range value passes through (truncated)", () => {
    expect(clampTranscriptionChunkSeconds(60)).toBe(60);
    expect(clampTranscriptionChunkSeconds(60.7)).toBe(60);
  });
  test("boundary values", () => {
    expect(clampTranscriptionChunkSeconds(0)).toBe(0);
    expect(clampTranscriptionChunkSeconds(300)).toBe(300);
  });
});

// clampOcrTesseractUpscaleFactor: trunc(Number(value)||1) then clamp to [1, 4].
describe("clampOcrTesseractUpscaleFactor [1,4]", () => {
  test("below min clamps up to 1", () => {
    expect(clampOcrTesseractUpscaleFactor(0)).toBe(1);
    expect(clampOcrTesseractUpscaleFactor(-3)).toBe(1);
  });
  test("above max clamps down to 4", () => {
    expect(clampOcrTesseractUpscaleFactor(5)).toBe(4);
    expect(clampOcrTesseractUpscaleFactor(99)).toBe(4);
  });
  test("in-range value passes through (truncated)", () => {
    expect(clampOcrTesseractUpscaleFactor(2)).toBe(2);
    expect(clampOcrTesseractUpscaleFactor(3.8)).toBe(3);
  });
  test("boundary values", () => {
    expect(clampOcrTesseractUpscaleFactor(1)).toBe(1);
    expect(clampOcrTesseractUpscaleFactor(4)).toBe(4);
  });
});
