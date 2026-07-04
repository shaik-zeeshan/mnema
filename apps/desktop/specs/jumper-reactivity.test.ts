// Reactivity regression: the jump picker's load $effect must RE-RUN when the
// cache marks the visible month stale (head poll / manual refresh mid-open), so
// stale-while-revalidate actually re-fetches — not just re-renders the
// disabled-date map. TimelineJumper.svelte L186-189 calls `cache.load(...)`;
// jumper-cache.svelte.ts `load()` must read the reactive `version` so the
// effect depends on it. If it doesn't, invalidate* bumps `version` but the
// effect never re-runs and the month is never re-fetched.
//
// Runes can't run under bun test, so specs/_reactivity/build.mjs precompiles
// the REAL adapter + a driver (mirroring the component effect verbatim) with
// Svelte's compiler under node; we drive it here with flushSync.
import { describe, test, expect, mock, beforeAll } from "bun:test";
import { spawnSync } from "child_process";
import { flushSync } from "svelte";
import { resolve } from "path";

const here = resolve(import.meta.dir, "_reactivity");

let invokeCalls = 0;
mock.module("@tauri-apps/api/core", () => ({
  // Resolve immediately with an empty month — enough to mark the month loaded.
  invoke: async () => {
    invokeCalls++;
    return [];
  },
  // bun module mocks fix the export-name set process-wide; later test files
  // transitively import convertFileSrc (frame-preview), so it must exist here.
  convertFileSrc: (p: string) => p,
}));

async function settle() {
  // Let load()'s single await (the mocked invoke) resolve, then flush effects.
  await new Promise((r) => setTimeout(r, 0));
  flushSync();
  await new Promise((r) => setTimeout(r, 0));
}

let makeDriver: () => {
  setOpen: (v: boolean) => void;
  invalidate: (f: { capturedAt: string }[]) => void;
  stop: () => void;
};

beforeAll(async () => {
  const built = spawnSync("node", [resolve(here, "build.mjs")], {
    encoding: "utf8",
  });
  if (built.status !== 0) {
    throw new Error(`rune precompile failed: ${built.stderr || built.stdout}`);
  }
  ({ makeDriver } = await import(resolve(here, "gen/driver.js")));
});

describe("jump picker load effect — stale-while-revalidate re-trigger", () => {
  test("invalidating the visible month re-runs load() and re-fetches", async () => {
    invokeCalls = 0;
    const driver = makeDriver();

    // Open the picker → the load effect runs and fetches the month once.
    driver.setOpen(true);
    flushSync();
    await settle();
    expect(invokeCalls).toBe(1);

    // Head poll delivers new frames for the SAME (visible) month while the
    // picker sits open. This marks the month stale + bumps the cache version.
    driver.invalidate([
      { capturedAt: new Date(2026, 5, 15, 10, 0, 0, 0).toISOString() },
    ]);
    flushSync();
    await settle();

    // The effect must have re-run and re-fetched the now-stale month.
    expect(invokeCalls).toBe(2);
    driver.stop();
  });
});
