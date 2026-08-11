// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
//
// The flow's `videoPixels` mapping, table-driven: `original` reads the
// display's BACKING pixels (CSS pixels × devicePixelRatio on each axis, so the
// ratio enters SQUARED), presets and custom sizes price their literal pixel
// counts, and a missing `window` prices the 1080p fallback. The pure halves are
// `nativeScreenPixels` (onboarding-flow) composed with `draftVideoPixels`
// (disk-estimate) — exactly how the flow's `$derived` wires them.
import { describe, expect, test } from "bun:test";
import { mock } from "bun:test";
import { compileModule } from "svelte/compiler";
import { draftVideoPixels } from "../src/lib/onboarding/disk-estimate";

// `nativeScreenPixels` lives in a rune module; compile it the way Vite does.
Bun.plugin({
  name: "svelte-rune-module",
  setup(build) {
    build.onLoad({ filter: /\.svelte\.ts$/ }, async (args) => {
      const source = await Bun.file(args.path).text();
      const js0 = new Bun.Transpiler({ loader: "ts" }).transformSync(source);
      const { js } = compileModule(js0, { filename: args.path, generate: "client" });
      return { contents: js.code, loader: "js" };
    });
  },
});

// Importing onboarding-flow pulls the whole controller graph; these stubs only
// exist so the import resolves. Export-name sets mirror the sibling specs'
// mocks of the same modules (bun fixes them process-wide).
mock.module("@tauri-apps/api/core", () => ({
  invoke: async () => null,
  convertFileSrc: (p: string) => `asset://${p}`,
}));
mock.module("@tauri-apps/api/event", () => ({
  listen: async () => () => {},
}));
mock.module("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "main" }),
  availableMonitors: async () => [],
}));
// Full export-name set (message/confirm/ask) — the settings-state graph this
// import pulls in uses all three, and bun fixes the set process-wide.
mock.module("@tauri-apps/plugin-dialog", () => ({
  message: async () => {},
  confirm: async () => true,
  ask: async () => false,
}));
mock.module("$app/navigation", () => ({ goto: async () => {} }));

const { nativeScreenPixels } = await import(
  "../src/routes/onboarding/onboarding-flow.svelte"
);

const draft = (over = {}) => ({
  resolutionMode: "preset",
  resolutionPreset: "720p",
  customWidth: null,
  customHeight: null,
  ...over,
});

/** Run `fn` with `globalThis.window` set (or absent, for `undefined`). */
function withWindow(value, fn) {
  const had = "window" in globalThis;
  const prior = globalThis.window;
  if (value === undefined) delete globalThis.window;
  else globalThis.window = value;
  try {
    fn();
  } finally {
    if (had) globalThis.window = prior;
    else delete globalThis.window;
  }
}

describe("the flow's videoPixels mapping", () => {
  test("original mode prices the display's backing pixels — DPR squared", () => {
    withWindow({ screen: { width: 1512, height: 982 }, devicePixelRatio: 2 }, () => {
      expect(nativeScreenPixels()).toBe(1512 * 982 * 4);
      expect(
        draftVideoPixels(draft({ resolutionMode: "original" }), nativeScreenPixels()),
      ).toBe(1512 * 982 * 4);
    });
  });

  test("a missing devicePixelRatio reads as 1", () => {
    withWindow({ screen: { width: 1920, height: 1080 } }, () => {
      expect(nativeScreenPixels()).toBe(1920 * 1080);
    });
  });

  test("the 720p preset prices 1280×720, whatever the display", () => {
    withWindow({ screen: { width: 1512, height: 982 }, devicePixelRatio: 2 }, () => {
      expect(draftVideoPixels(draft(), nativeScreenPixels())).toBe(1280 * 720);
    });
  });

  test("a custom 1600×900 prices its literal pixel count", () => {
    withWindow({ screen: { width: 1512, height: 982 }, devicePixelRatio: 2 }, () => {
      expect(
        draftVideoPixels(
          draft({ resolutionMode: "custom", customWidth: 1600, customHeight: 900 }),
          nativeScreenPixels(),
        ),
      ).toBe(1_440_000);
    });
  });

  test("no window at all falls back to 1080p for original mode", () => {
    withWindow(undefined, () => {
      // `nativeScreenPixels` reports "can't read"; `draftVideoPixels` prices
      // the 1080p stand-in.
      expect(nativeScreenPixels()).toBeNull();
      expect(
        draftVideoPixels(draft({ resolutionMode: "original" }), nativeScreenPixels()),
      ).toBe(1920 * 1080);
    });
  });
});
