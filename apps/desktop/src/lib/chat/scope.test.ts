// @ts-nocheck — run under `bun test`; bun:test types aren't in the svelte-check
// tsconfig, so skip static checking here (same as chat-format.test.ts).
import { describe, expect, test } from "bun:test";
import { defaultScope, scopeFromParams, scopeLabel, toWireScope } from "./scope";

const FRI = new Date(2026, 9, 9, 15); // Fri Oct 9 2026

describe("chat scope", () => {
  test("defaults to this week with About you on", () => {
    const s = defaultScope(FRI);
    expect(s).toEqual({ start: "2026-10-05", end: "2026-10-11", aboutYou: true });
    expect(scopeLabel(s, FRI)).toBe("this week");
  });

  test("labels presets in lower case, other spans by date", () => {
    expect(scopeLabel({ start: "2026-10-08", end: "2026-10-08", aboutYou: true }, FRI)).toBe("yesterday");
    expect(scopeLabel({ start: "2026-10-01", end: "2026-10-06", aboutYou: true }, FRI)).toBe("Oct 1 – 6");
  });

  test("wire scope spans local midnight to the end day's last ms", () => {
    const w = toWireScope({ start: "2026-10-05", end: "2026-10-11", aboutYou: false });
    expect(w.fromMs).toBe(new Date(2026, 9, 5).getTime());
    expect(w.toMs).toBe(new Date(2026, 9, 12).getTime() - 1);
    expect(w.aboutYou).toBe(false);
  });

  test("?from=&to= params", () => {
    expect(scopeFromParams("2026-10-05", "2026-10-11", false)).toEqual({ start: "2026-10-05", end: "2026-10-11", aboutYou: false });
    expect(scopeFromParams("2026-10-05", null, true)).toEqual({ start: "2026-10-05", end: "2026-10-05", aboutYou: true });
    expect(scopeFromParams("2026-10-11", "2026-10-05", true)).toBeNull();
    expect(scopeFromParams("oct 5", null, true)).toBeNull();
  });
});
