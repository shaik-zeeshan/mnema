// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { afterAll, describe, expect, mock, test } from "bun:test";
import { compileModule } from "svelte/compiler";

// `.svelte.ts` rune modules aren't plain TS: compile them the way Vite does.
Bun.plugin({
  name: "svelte-rune-module",
  setup(build) {
    build.onLoad({ filter: /\.svelte\.ts$/ }, async (args) => {
      const source = await Bun.file(args.path).text();
      // Strip TS types first: `compileModule` only parses JS.
      const js0 = new Bun.Transpiler({ loader: "ts" }).transformSync(source);
      const { js } = compileModule(js0, { filename: args.path, generate: "client" });
      return { contents: js.code, loader: "js" };
    });
  },
});

/** Backend events the module subscribed to, keyed by event name. */
const emitters = new Map();
mock.module("@tauri-apps/api/event", () => ({
  listen: async (name, handler) => {
    emitters.set(name, handler);
    return () => emitters.delete(name);
  },
}));
mock.module("@tauri-apps/api/dpi", () => ({
  // Captures {x,y} so the setPosition assertion can read what was passed.
  PhysicalPosition: class {
    constructor(x, y) {
      this.x = x;
      this.y = y;
    }
  },
}));

// Mutable harness state the window mock reads per test.
let monitors = [];
let winGeometry = { pos: { x: 0, y: 0 }, size: { width: 800, height: 600 } };
const setPositionCalls = [];
const fakeWin = {
  outerPosition: async () => winGeometry.pos,
  outerSize: async () => winGeometry.size,
  setPosition: async (position) => {
    setPositionCalls.push(position);
  },
};
mock.module("@tauri-apps/api/window", () => ({
  availableMonitors: async () => monitors,
  getCurrentWindow: () => fakeWin,
}));

globalThis.window ??= {};
/** `document` listeners the module registered, keyed by event name. */
const docHandlers = new Map();
globalThis.document = {
  visibilityState: "visible",
  addEventListener(name, handler) {
    docHandlers.set(name, handler);
  },
};

// Capture timers so the clamp's 500 ms debounce is driven manually — no
// wall-clock waits. Installed BEFORE init; restored after the file.
const realSetTimeout = globalThis.setTimeout;
const realClearTimeout = globalThis.clearTimeout;
let nextTimerId = 1;
/** id → {fn, ms}. `clearTimeout` deletes, so only live timers remain. */
const pendingTimers = new Map();
globalThis.setTimeout = (fn, ms) => {
  const id = nextTimerId++;
  pendingTimers.set(id, { fn, ms });
  return id;
};
globalThis.clearTimeout = (id) => {
  pendingTimers.delete(id);
};
afterAll(() => {
  globalThis.setTimeout = realSetTimeout;
  globalThis.clearTimeout = realClearTimeout;
});

// ONE init for the whole file: `initRenderIdle`'s `_initialized` latch is
// module-global, so every test drives the same registered handlers.
const mod = await import("./render-idle.svelte");
const { clampTarget } = await import("$lib/render-idle-clamp");
mod.initRenderIdle({ clampWindow: true });
await Promise.resolve();

function emit(name) {
  const handler = emitters.get(name);
  if (!handler) throw new Error(`no listener registered for "${name}"`);
  handler({ event: name, id: 0, payload: null });
}

/** Flush the microtask chain behind `clampWindowOntoScreen`'s awaits. */
async function drain() {
  for (let i = 0; i < 20; i++) await Promise.resolve();
}

describe("render-idle", () => {
  test("a backend system wake un-gates rendering when screens_did_wake is lost", () => {
    emit("screens_did_sleep");
    expect(mod.renderIdle()).toBe(true);

    // `screens_did_wake` never arrives (it is a single unreliable
    // notification). `system_did_wake` is the backend's redundant wake
    // signal — emitted from the NSWorkspaceDidWake observer *and* the
    // display-reconfiguration recovery path — and `+page.svelte` already
    // treats it as the primary wake trigger. It must clear the gate,
    // otherwise every render-idle-gated updater stays frozen for the rest
    // of the process lifetime.
    const before = mod.renderResumeTick();
    emit("system_did_wake");
    expect(mod.renderIdle()).toBe(false);
    expect(mod.renderResumeTick()).toBe(before + 1);
  });

  test("the gate closes while the document is hidden and reopens when it becomes visible", () => {
    globalThis.document.visibilityState = "hidden";
    // Screens are awake — the document term alone must close the gate.
    expect(mod.renderIdle()).toBe(true);

    const before = mod.renderResumeTick();
    globalThis.document.visibilityState = "visible";
    const onVisibility = docHandlers.get("visibilitychange");
    expect(onVisibility).toBeDefined();
    onVisibility();
    expect(mod.renderIdle()).toBe(false);
    expect(mod.renderResumeTick()).toBe(before + 1);
  });

  test("screens_did_wake clears the sleep gate", () => {
    emit("screens_did_sleep");
    expect(mod.renderIdle()).toBe(true);

    // The visibilitychange handler is conditional on the document being
    // visible — a firing while hidden must NOT bump the resume tick.
    globalThis.document.visibilityState = "hidden";
    const before = mod.renderResumeTick();
    docHandlers.get("visibilitychange")();
    expect(mod.renderResumeTick()).toBe(before);
    globalThis.document.visibilityState = "visible";

    emit("screens_did_wake");
    expect(mod.renderIdle()).toBe(false);
    expect(mod.renderResumeTick()).toBe(before + 1);
  });
});

describe("offscreen clamp", () => {
  function resetClampHarness() {
    setPositionCalls.length = 0;
    pendingTimers.clear();
  }

  /** Fire the single pending debounce timer and let the async clamp settle. */
  async function runPendingClamp() {
    expect(pendingTimers.size).toBe(1);
    const [timer] = pendingTimers.values();
    pendingTimers.clear();
    timer.fn();
    await drain();
  }

  test("a display reconfiguration that parks the window offscreen moves it back", async () => {
    resetClampHarness();
    winGeometry = { pos: { x: 3000, y: 200 }, size: { width: 800, height: 600 } };
    monitors = [{ position: { x: 0, y: 0 }, size: { width: 1920, height: 1080 } }];

    emit("display_configuration_changed");
    await runPendingClamp();

    // The one assertion proving the halves connect: the position handed to
    // `win.setPosition` is exactly what the pure geometry returns.
    const expected = clampTarget(monitors, winGeometry.pos, winGeometry.size);
    expect(expected).not.toBeNull();
    expect(setPositionCalls).toHaveLength(1);
    expect({ x: setPositionCalls[0].x, y: setPositionCalls[0].y }).toEqual(expected);
  });

  test("several reconfiguration events in one burst clamp once", async () => {
    resetClampHarness();
    winGeometry = { pos: { x: 3000, y: 200 }, size: { width: 800, height: 600 } };
    monitors = [{ position: { x: 0, y: 0 }, size: { width: 1920, height: 1080 } }];

    // One CG reconfiguration fires the callback several times; the debounce
    // must collapse them to a single pending timer, hence a single move.
    emit("display_configuration_changed");
    emit("display_configuration_changed");
    emit("display_configuration_changed");
    await runPendingClamp();

    expect(setPositionCalls).toHaveLength(1);
  });

  test("a window still meaningfully visible is not moved", async () => {
    resetClampHarness();
    winGeometry = { pos: { x: 100, y: 100 }, size: { width: 800, height: 600 } };
    monitors = [{ position: { x: 0, y: 0 }, size: { width: 1920, height: 1080 } }];
    expect(clampTarget(monitors, winGeometry.pos, winGeometry.size)).toBeNull();

    emit("display_configuration_changed");
    await runPendingClamp();

    expect(setPositionCalls).toHaveLength(0);
  });
});
