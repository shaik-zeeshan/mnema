// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { expect, test } from "bun:test";
import { createAutosaveEngine } from "./autosave.svelte";

function unit(key: string, current: string, saves: string[]) {
  return {
    key,
    debounceMs: 10_000,
    snapshot: () => current,
    baseline: () => "saved",
    blocked: () => false,
    saving: () => false,
    save: async () => {
      saves.push(key);
    },
  };
}

test("flush saves a debouncing unit now and drops its timer", async () => {
  const saves: string[] = [];
  const engine = createAutosaveEngine({ privacyCommandInFlight: () => false });
  engine.register(unit("capture_sources", "edited", saves));
  engine.register(unit("video", "edited", saves));
  engine.tick();

  await engine.flush("capture_sources");

  expect(saves).toEqual(["capture_sources"]);
  expect(engine.hasPendingTimer("capture_sources")).toBe(false);
  expect(engine.hasPendingTimer("video")).toBe(true);
  engine.cancelAll();
});

test("flush skips a clean unit", async () => {
  const saves: string[] = [];
  const engine = createAutosaveEngine({ privacyCommandInFlight: () => false });
  engine.register(unit("capture_sources", "saved", saves));

  await engine.flush("capture_sources");

  expect(saves).toEqual([]);
});
