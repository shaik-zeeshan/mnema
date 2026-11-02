// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { expect, test } from "bun:test";
import { createAutosaveEngine } from "./autosave.svelte";
import { recordingAutosaveDebounceMs, settleRecDomainSaveError } from "./autosave-core";

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

// Start awaits this flush, then starts with the saved sources. A unit the flush
// could not save (backoff after a failed save, a privacy command in flight, all
// sources off) must say so, or Start records the old sources under the new
// switches.
test("flush reports not-flushed when the save leaves the unit dirty (backoff)", async () => {
  const engine = createAutosaveEngine({ privacyCommandInFlight: () => false });
  // saveRecordingDomain's backoff gate: returns without saving.
  engine.register({ ...unit("capture_sources", "edited", []), save: async () => {} });

  expect(await engine.flush("capture_sources")).toBe(false);
});

test("flush reports not-flushed while a privacy command holds saves", async () => {
  const saves: string[] = [];
  const engine = createAutosaveEngine({ privacyCommandInFlight: () => true });
  engine.register(unit("capture_sources", "edited", saves));

  expect(await engine.flush("capture_sources")).toBe(false);
  expect(saves).toEqual([]);
});

test("flush reports flushed for a clean unit and after a landed save", async () => {
  let baseline = "saved";
  const engine = createAutosaveEngine({ privacyCommandInFlight: () => false });
  engine.register({
    ...unit("capture_sources", "edited", []),
    baseline: () => baseline,
    save: async () => {
      baseline = "edited";
    },
  });

  expect(await engine.flush("capture_sources")).toBe(true); // saved now
  expect(await engine.flush("capture_sources")).toBe(true); // clean
});

// saveRecordingDomain's catch: a sources save refused because recording started
// under it reverts the switches to the saved sources, records no failure and
// arms no retry; any other rejection is a failure (which arms the backoff).
test("a capture_sources_locked rejection reverts to canonical without a failure or retry", () => {
  const drafts = { captureMicrophone: true };
  const canonical = { captureMicrophone: false };
  const calls: string[] = [];
  const on = {
    revertToCanonical: () => {
      Object.assign(drafts, canonical);
      calls.push("revert");
    },
    clearFailure: () => calls.push("clear"),
    noteFailure: () => calls.push("note"),
  };

  settleRecDomainSaveError({ code: "capture_sources_locked", message: "locked" }, on);
  expect(drafts).toEqual(canonical);
  expect(calls).toEqual(["revert", "clear"]);

  calls.length = 0;
  settleRecDomainSaveError(new Error("disk full"), on);
  expect(calls).toEqual(["note"]);
});
