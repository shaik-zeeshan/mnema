// @ts-nocheck — run under `bun test`; bun:test types aren't in the svelte-check
// tsconfig, so skip static checking here (same as turn-model.test.ts).
import { expect, test } from "bun:test";
import { regeneratePlan } from "./regenerate";

test("only a settled trailing turn re-runs; a local-only error skips the delete", () => {
  const done = { turnIndex: 2, phase: "done" };
  expect(regeneratePlan(done, 3, false)).toEqual({ deleteFirst: true });
  expect(regeneratePlan({ ...done, turnIndex: 1 }, 3, false)).toBeNull(); // earlier turn
  expect(regeneratePlan(done, 3, true)).toBeNull(); // busy
  expect(regeneratePlan({ ...done, phase: "streaming" }, 3, false)).toBeNull();
  expect(regeneratePlan({ ...done, phase: "thinking" }, 3, false)).toBeNull();
  expect(regeneratePlan({ ...done, stopping: true }, 3, false)).toBeNull(); // stop not confirmed
  expect(regeneratePlan({ ...done, phase: "error" }, 3, false)).toEqual({ deleteFirst: true });
  expect(regeneratePlan({ ...done, phase: "error", localOnly: true }, 3, false)).toEqual({ deleteFirst: false });
});
