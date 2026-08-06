// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { conf, deltaLine, pct, shortDate } from "./format";

describe("the two registers", () => {
  test("row register is the 0–1 measurement, clamped", () => {
    expect(conf(0.857)).toBe("0.86");
    expect(conf(1.4)).toBe("1.00");
    expect(conf(Number.NaN)).toBe("0.00");
  });

  test("conclusion register is the whole percent", () => {
    expect(pct(0.857)).toBe(86);
    expect(pct(-1)).toBe(0);
  });
});

describe("deltaLine", () => {
  const at = Date.now() - 3 * 3600_000;

  test("a moved arc states both ends", () => {
    expect(
      deltaLine({ history: [0.42, 0.7, 0.86], confidence: 0.86, faded: false, lastSupportedAtMs: at }),
    ).toBe("0.42 → 0.86 · 3h ago");
  });

  test("a flat arc says steady rather than 0.74 → 0.74", () => {
    expect(
      deltaLine({ history: [0.74, 0.74], confidence: 0.74, faded: false, lastSupportedAtMs: at }),
    ).toBe("steady near 0.74 · 3h ago");
  });

  test("one snapshot has no arc to state", () => {
    expect(
      deltaLine({ history: [0.5], confidence: 0.5, faded: false, lastSupportedAtMs: at }),
    ).toBe("steady near 0.50 · 3h ago");
  });

  test("faded ends at the floor, never at a time", () => {
    expect(
      deltaLine({ history: [0.38, 0.12], confidence: 0.12, faded: true, lastSupportedAtMs: at }),
    ).toBe("0.38 → 0.12 · below floor");
  });
});

test("shortDate refuses to invent a date", () => {
  expect(shortDate(0)).toBe("—");
});
