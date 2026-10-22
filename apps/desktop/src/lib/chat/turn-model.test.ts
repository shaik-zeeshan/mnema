// @ts-nocheck — run under `bun test`; bun:test types aren't in the svelte-check
// tsconfig, so skip static checking here (same as journal-day.test.ts).
import { expect, test } from "bun:test";
import { applyUpdate, hydrateTurn, makeTurn } from "./turn-model";

test("appendProse coalesces into the trailing prose block, else opens a new one", () => {
  const t = makeTurn(0, "q", "thinking");
  applyUpdate(t, { op: "appendProse", text: "Hello " });
  applyUpdate(t, { op: "appendProse", text: "world" });
  applyUpdate(t, { op: "openBlock", block: { kind: "bars", items: [] } });
  applyUpdate(t, { op: "appendProse", text: "After" });
  expect(t.blocks).toEqual([
    { kind: "prose", markdown: "Hello world" },
    { kind: "bars", items: [] },
    { kind: "prose", markdown: "After" },
  ]);
  t.startedAtMs = Date.now() - 1000;
  applyUpdate(t, { op: "done" });
  expect(t.phase).toBe("done");
  expect(t.elapsedMs).toBeGreaterThanOrEqual(1000);
});

test("hydrateTurn keeps the persisted error kind and engine; old turns read null", () => {
  const base = {
    turnIndex: 0, question: "q", answer: "", reasoning: null, blocks: [], toolActivities: [], sources: [],
    phase: "error", errorMessage: "Over quota", createdAtMs: 1, updatedAtMs: 2,
  };
  const t = hydrateTurn({ ...base, errorKind: "quota", provider: "anthropic-1", model: "claude-sonnet-4-5" });
  expect([t.errorKind, t.provider, t.model]).toEqual(["quota", "anthropic-1", "claude-sonnet-4-5"]);
  const old = hydrateTurn(base);
  expect([old.errorKind, old.provider, old.model]).toEqual([null, null, null]);
});
