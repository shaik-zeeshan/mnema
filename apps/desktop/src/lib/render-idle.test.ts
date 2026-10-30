// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, mock, test } from "bun:test";
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

globalThis.window ??= {};
/** `document` listeners the module registered, keyed by event name. */
const docHandlers = new Map();
globalThis.document = {
  visibilityState: "visible",
  addEventListener(name, handler) {
    docHandlers.set(name, handler);
  },
};

// ONE init for the whole file: `initRenderIdle`'s `_initialized` latch is
// module-global, so every test drives the same registered handlers.
const mod = await import("./render-idle.svelte");
mod.initRenderIdle();
await Promise.resolve();

function emit(name) {
  const handler = emitters.get(name);
  if (!handler) throw new Error(`no listener registered for "${name}"`);
  handler({ event: name, id: 0, payload: null });
}

/** Flush pending microtasks. */
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

  test("whenRenderable parks while screens sleep and resolves on wake", async () => {
    emit("screens_did_sleep");
    let resolved = false;
    const parked = mod.whenRenderable().then(() => {
      resolved = true;
    });
    await drain();
    expect(resolved).toBe(false);

    emit("screens_did_wake");
    await parked;
    expect(resolved).toBe(true);
    // And while renderable it never parks.
    let immediate = false;
    void mod.whenRenderable().then(() => {
      immediate = true;
    });
    await drain();
    expect(immediate).toBe(true);
  });

  test("initRenderIdle arms the frame-preview render gate: a parked swap paints on wake", async () => {
    const { FramePreviewUrlHolder } = await import("$lib/frame-preview");
    const holder = new FramePreviewUrlHolder({
      convertFileSrcImpl: (path) => `asset://${path}`,
      fetchImpl: async () => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(4) }),
      createObjectUrlImpl: () => "blob:armed",
      revokeObjectUrlImpl: () => {},
    });

    emit("screens_did_sleep");
    let url = null;
    let settled = false;
    const pending = holder.swap("/frames/asleep.png").then((swapped) => {
      settled = true;
      url = swapped;
    });
    await drain();
    // The one assertion proving the halves connect: with no explicit gate
    // wiring here, the swap parked because `initRenderIdle` armed it.
    expect(settled).toBe(false);

    emit("screens_did_wake");
    await pending;
    expect(url).toBe("blob:armed");
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

