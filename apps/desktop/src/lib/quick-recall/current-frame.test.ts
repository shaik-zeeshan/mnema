// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { FRAME_STALE_MS, frameAgePhrase } from "./current-frame";

describe("frameAgePhrase", () => {
  test("tenths of a second while the grab is fresh", () => {
    expect(frameAgePhrase(0)).toBe("0.0 s");
    expect(frameAgePhrase(430)).toBe("0.4 s");
    expect(frameAgePhrase(9_940)).toBe("9.9 s");
  });

  test("whole seconds up to a minute", () => {
    expect(frameAgePhrase(10_000)).toBe("10 s");
    expect(frameAgePhrase(FRAME_STALE_MS + 1_000)).toBe("46 s");
  });

  test("coarse minutes past a minute", () => {
    expect(frameAgePhrase(61_000)).toBe("1 min");
    expect(frameAgePhrase(9 * 60_000)).toBe("9 min");
  });

  test("a negative clock skew never renders a negative age", () => {
    expect(frameAgePhrase(-5_000)).toBe("0.0 s");
  });
});
