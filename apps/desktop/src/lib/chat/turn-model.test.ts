// @ts-nocheck — run under `bun test`; bun:test types aren't in the svelte-check
// tsconfig, so skip static checking here (same as journal-day.test.ts).
import { expect, test } from "bun:test";
import { applyUpdate, makeTurn } from "./turn-model";

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
