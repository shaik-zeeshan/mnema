// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
//
// `loadThumbnails`' generation guard, both halves:
//
//  1. The final merge result must NOT be assigned to `thumbnailCache` when the
//     search generation moved on mid-merge. The incremental `onMinted` publish
//     was guarded, but the merge's RETURN value was assigned after the await
//     with no re-check — so a superseded search's thumbnails clobbered the
//     displayed cache of the search that replaced it.
//  2. Thumbnails publish INCREMENTALLY: the cache gains entries while the
//     merge is still running, not only when the whole batch resolves.
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
globalThis.URL.createObjectURL = () => `blob:${++minted}`;
globalThis.URL.revokeObjectURL = () => {};

// Deferred fetches: every preview read parks until the test releases it, so
// the test controls exactly when the merge is mid-flight.
let waiters: (() => void)[] = [];
globalThis.fetch = (async () => {
  await new Promise<void>((resolve) => waiters.push(resolve));
  return { ok: true, arrayBuffer: async () => new ArrayBuffer(4) };
}) as unknown as typeof fetch;

async function hops(n = 30) {
  for (let i = 0; i < n; i++) await Promise.resolve();
}

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

test("a superseded search's thumbnails never reach the displayed cache", async () => {
  const store = new SearchStore();
  waiters = [];

  // Search A starts its thumbnail load; the previews IPC resolves, the merge's
  // byte reads are in flight.
  const loadA = store.loadThumbnails([row(1), row(2)], store.searchGeneration);
  await hops();
  expect(waiters.length).toBe(2);

  // Search B supersedes A while A's merge is still reading.
  store.searchGeneration += 1;

  // A's previews resolve anyway.
  for (const release of waiters.splice(0)) release();
  await loadA;

  // The displayed cache must contain nothing from A — neither via the
  // incremental publish nor via the merge's final return value.
  expect(store.thumbnailCache.has(1)).toBe(false);
  expect(store.thumbnailCache.has(2)).toBe(false);
  expect(store.thumbnailCache.size).toBe(0);
});

test("thumbnails publish incrementally, before the whole merge resolves", async () => {
  const store = new SearchStore();
  waiters = [];

  let settled = false;
  const load = store
    .loadThumbnails([row(11), row(12), row(13)], store.searchGeneration)
    .then(() => {
      settled = true;
    });
  await hops();
  expect(waiters.length).toBe(3);

  // Release only the first read: its thumbnail must reach the cache while the
  // other two are still in flight.
  waiters.shift()!();
  await hops();
  expect(settled).toBe(false);
  expect(store.thumbnailCache.size).toBe(1);

  for (const release of waiters.splice(0)) release();
  await load;
  expect(store.thumbnailCache.size).toBe(3);
});
