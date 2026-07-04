// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
// Latest-wins regression for ReceiptFrameLoader.loadMeta. The invoke stub is
// injected via the constructor seam — deliberately NOT mock.module: bun mocks
// persist per-process and specs/ files already register `@tauri-apps/api/core`
// mocks whose export-name set would poison this file's imports in a full run.
import { describe, expect, test } from "bun:test";

import { ReceiptFrameLoader } from "./receipt-frames";
import type { InvokeFn } from "./receipt-frames";

// Control invoke resolution order so we can simulate an out-of-order meta fetch.
let resolveFrame1: (v: unknown) => void = () => {};
const pendingFrame1 = new Promise((res) => {
	resolveFrame1 = res;
});
const meta1 = { id: 1, appName: "App-One", windowTitle: "Frame 1", ocrText: "" };
const meta2 = { id: 2, appName: "App-Two", windowTitle: "Frame 2", ocrText: "" };

const stubInvoke = ((cmd: string, args?: Record<string, unknown>) => {
	const fid = (args?.request as { frameId?: number } | undefined)?.frameId;
	if (cmd === "get_frame") {
		if (fid === 1) return pendingFrame1; // slow — resolves later, on demand
		if (fid === 2) return Promise.resolve(meta2); // fast
	}
	return Promise.resolve(null);
}) as InvokeFn;

describe("ReceiptFrameLoader.loadMeta latest-wins", () => {
	test("a cached newer frame's meta is not clobbered by a slow older fetch", async () => {
		const seen: number[] = [];
		const loader = new ReceiptFrameLoader(
			{
				onPreview: () => {},
				onThumb: () => {},
				onMeta: (m) => seen.push(m.id),
			},
			stubInvoke,
		);

		// 1) Load frame 2 first so its meta is cached.
		await loader.loadMeta(2); // resolves fast → cache meta2, onMeta(2)

		// 2) Step to frame 1 (uncached) — its fetch is in flight (pendingFrame1).
		const p1 = loader.loadMeta(1);

		// 3) Step back to the CURRENT frame 2 (cache hit) — should paint meta2 now
		//    AND invalidate the still-in-flight frame-1 request.
		await loader.loadMeta(2);

		// 4) The stale frame-1 fetch finally resolves — it must be dropped.
		resolveFrame1(meta1);
		await p1;

		// The last meta painted must be the current frame (2), never the stale 1.
		expect(seen[seen.length - 1]).toBe(2);
	});
});
