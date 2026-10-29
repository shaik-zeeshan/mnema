// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { expect, test } from "bun:test";
import { createAutosaveEngine } from "./autosave.svelte";
import { recordingAutosaveDebounceMs } from "./autosave-core";

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

test("flush waits out a save already in flight, then saves an edit made during it", async () => {
  const saves: string[] = [];
  let current = "edited";
  let baseline = "saved";
  let saving = false;
  let release: () => void = () => {};
  const engine = createAutosaveEngine({ privacyCommandInFlight: () => false });
  engine.register({
    key: "capture_sources",
    debounceMs: 0,
    snapshot: () => current,
    baseline: () => baseline,
    blocked: () => false,
    saving: () => saving,
    save: async () => {
      const sent = current;
      saving = true;
      await new Promise<void>((resolve) => (release = resolve));
      saves.push(sent);
      baseline = sent;
      saving = false;
    },
  });
  engine.tick();
  await new Promise((resolve) => setTimeout(resolve, 0)); // the debounce fires
  current = "edited again"; // a second toggle while the first save runs

  let flushed = false;
  const flushing = engine.flush("capture_sources").then(() => (flushed = true));
  await Promise.resolve();
  expect(flushed).toBe(false); // Start must not run while the save is in flight

  release();
  await new Promise((resolve) => setTimeout(resolve, 0));
  release();
  await flushing;
  expect(saves).toEqual(["edited", "edited again"]);
});

test("source switches save without a debounce; other recording domains keep it", () => {
  expect(recordingAutosaveDebounceMs("capture_sources")).toBe(0);
  expect(recordingAutosaveDebounceMs("video")).toBe(450);
});
