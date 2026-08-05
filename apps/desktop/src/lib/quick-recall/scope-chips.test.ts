// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import {
  sourceScopeOf,
  dateScopeOf,
  setSourceScope,
  setDateScope,
} from "./scope-chips";

describe("scope chips", () => {
  test("reads the scope out of query tokens", () => {
    expect(sourceScopeOf("webhook")).toBe("all");
    expect(sourceScopeOf("webhook source:screen")).toBe("screen");
    expect(sourceScopeOf("webhook source:mic source:system")).toBe("audio");
    expect(sourceScopeOf("webhook source:mic")).toBe("audio");
    expect(dateScopeOf("webhook date:today")).toBe("today");
    expect(dateScopeOf("webhook after:7d")).toBe("week");
    expect(dateScopeOf("webhook after:2024-05-01")).toBe("custom");
  });

  test("toggling rewrites only its own operator family", () => {
    const q = setSourceScope("webhook date:today", "screen");
    expect(q.trim()).toBe("webhook date:today source:screen");
    expect(setSourceScope(q, "all").trim()).toBe("webhook date:today");
    expect(setDateScope(q, "week").trim()).toBe(
      "webhook source:screen after:7d",
    );
  });

  test("audio scope writes both audio sources", () => {
    expect(setSourceScope("webhook", "audio").trim()).toBe(
      "webhook source:mic source:system",
    );
  });
});
