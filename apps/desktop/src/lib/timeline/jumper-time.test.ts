// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the svelte-check tsconfig.
import { describe, expect, test } from "bun:test";
import { dayHeat } from "./jumper-time";

describe("dayHeat", () => {
  test("no frames draws no dot; the busiest day is full; the rest scale by thirds", () => {
    expect(dayHeat(0, 900)).toBe(0);
    expect(dayHeat(1, 900)).toBe(1);
    expect(dayHeat(300, 900)).toBe(1);
    expect(dayHeat(301, 900)).toBe(2);
    expect(dayHeat(600, 900)).toBe(2);
    expect(dayHeat(900, 900)).toBe(3);
    expect(dayHeat(5, 0)).toBe(3);
  });
});
