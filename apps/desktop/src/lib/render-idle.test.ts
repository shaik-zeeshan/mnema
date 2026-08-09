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
mock.module("@tauri-apps/api/dpi", () => ({ PhysicalPosition: class {} }));
mock.module("@tauri-apps/api/window", () => ({
  availableMonitors: async () => [],
  getCurrentWindow: () => ({}),
}));

globalThis.window ??= {};
globalThis.document ??= { visibilityState: "visible", addEventListener() {} };

function emit(name) {
  const handler = emitters.get(name);
  if (!handler) throw new Error(`no listener registered for "${name}"`);
  handler({ event: name, id: 0, payload: null });
}

describe("render-idle", () => {
  test("a backend system wake un-gates rendering when screens_did_wake is lost", async () => {
    const mod = await import("./render-idle.svelte");
    mod.initRenderIdle();
    await Promise.resolve();

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
});
