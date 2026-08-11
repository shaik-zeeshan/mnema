// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
//
// Quick Recall's thumbnails are blob URLs owned by a bounded LRU
// (`FramePreviewUrlMap`, cap FRAME_PREVIEW_URL_CAP = 256) whose eviction
// REVOKES. `merge` documents the policy that keeps that safe:
//
//   "`get` (not `peek`) so an already-held frame is still marked
//    most-recently-used — a re-search over the same results must not make its
//    own thumbnails the next eviction candidates."
//
// `loadThumbnails` filters ids it already holds out of the request BEFORE the
// merge, so `merge` never sees them and the touch never happens. The live set
// is therefore ordered by FIRST MINT, and the frame that keeps coming back in
// every search is the OLDEST — so it is evicted (and revoked) while its card is
// on screen, and nothing re-requests it for that result view.
import { beforeAll, expect, mock, test } from "bun:test";
import { compileModule } from "svelte/compiler";

// `.svelte.ts` rune modules aren't plain TS: compile them the way Vite does.
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

/** Scrub previews the backend "returns" for whatever ids are asked for. */
mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd !== "get_frame_scrub_previews") return null;
    const ids = (args?.request as { frameIds: number[] }).frameIds;
    return {
      previews: ids.map((frameId) => ({
        frameId,
        preview: { filePath: `/scrub/${frameId}.jpg`, mimeType: "image/jpeg" },
        missingReason: null,
      })),
    };
  },
  // bun module mocks fix the export-name set process-wide; keep parity with the
  // other specs' mock of this module.
  convertFileSrc: (p: string) => `asset://${p}`,
}));
mock.module("@tauri-apps/plugin-dialog", () => ({ message: async () => {} }));
mock.module("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "quick-recall" }),
  availableMonitors: async () => [],
}));
mock.module("$app/navigation", () => ({ goto: async () => {} }));

let minted = 0;
const revoked = new Set<string>();
globalThis.URL.createObjectURL = () => `blob:${++minted}`;
globalThis.URL.revokeObjectURL = (url: string) => {
  revoked.add(url);
};
globalThis.fetch = (async () => ({
  ok: true,
  arrayBuffer: async () => new ArrayBuffer(4),
})) as unknown as typeof fetch;

let SearchStore: new () => {
  thumbnailCache: Map<number, string>;
  searchGeneration: number;
  loadThumbnails(results: unknown[], generation: number): Promise<void>;
};

beforeAll(async () => {
  ({ SearchStore } = await import("../src/lib/quick-recall/searchStore.svelte"));
});

/** A frame result row: only `thumbnailFrameId` matters to the loader. */
const row = (id: number) => ({ thumbnailFrameId: id });

test("a result that keeps matching keeps its thumbnail across a long session", async () => {
  const store = new SearchStore();

  // Frame 1 is the same hit every time (the app the user searches for over and
  // over); the other 23 rows of each search are new frames. 12 searches mint
  // 24 + 11×23 = 277 URLs, past the 256 cap.
  for (let search = 0; search < 12; search++) {
    const fresh = Array.from({ length: 23 }, (_, i) => 1000 + search * 23 + i);
    await store.loadThumbnails([row(1), ...fresh.map(row)], store.searchGeneration);
  }

  // Sanity: the cap DID bite (otherwise this test proves nothing).
  expect(minted).toBeGreaterThan(256);
  expect(revoked.size).toBeGreaterThan(0);

  const painted = store.thumbnailCache.get(1);
  expect(painted).toBeDefined(); // still on screen → must still have a URL
  expect(revoked.has(painted as string)).toBe(false); // …and a live one
});
