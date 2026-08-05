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

// ── Preview object-URL lifecycle ────────────────────────────────────────
// Playback paints object URLs (see the #previews comment in receipt-frames):
// every URL the loader mints must be revoked exactly once, or the receipt
// strands a full-size decoded surface per frame played.
function urlHarness(
	previewsFor: (fid: number) => unknown = () => defaultPreview,
	fetchImpl: typeof fetch = (async () => ({
		ok: true,
		arrayBuffer: async () => new ArrayBuffer(4),
	})) as unknown as typeof fetch,
) {
	const live = new Set<string>();
	const revoked: string[] = [];
	let minted = 0;
	const loader = new ReceiptFrameLoader(
		{ onPreview: () => {}, onThumb: () => {}, onMeta: () => {} },
		((cmd: string, args?: Record<string, unknown>) => {
			const fid = (args?.request as { frameId?: number } | undefined)?.frameId;
			if (cmd === "get_frame_preview") return Promise.resolve(previewsFor(fid));
			return Promise.resolve(null);
		}) as InvokeFn,
		{
			convertFileSrcImpl: (filePath: string) => `asset://${filePath}`,
			fetchImpl,
			createObjectUrlImpl: () => {
				const url = `blob:${++minted}`;
				live.add(url);
				return url;
			},
			revokeObjectUrlImpl: (url: string) => {
				revoked.push(url);
				live.delete(url);
			},
		},
	);
	return { loader, live, revoked };
}

const defaultPreview = {
	mimeType: "image/png",
	filePath: "/previews/frame.png",
	sourceKind: "originalFrame",
	hasSecretRedactions: false,
	secretRedactionCount: 0,
};

/** Let the loader's prefetch chain (fetch → mint → pump) settle. */
async function settle(): Promise<void> {
	for (let i = 0; i < 40; i++) await Promise.resolve();
}

describe("ReceiptFrameLoader preview object URLs", () => {
	test("closing the receipt revokes every URL it minted", async () => {
		const { loader, live, revoked } = urlHarness();

		loader.pump([1, 2, 3], 0);
		await settle();
		expect(live.size).toBeGreaterThan(0);
		expect(loader.peekPreviewUrl(1)).toBe("blob:1");

		loader.dispose();
		expect(live.size).toBe(0);
		expect(new Set(revoked).size).toBe(revoked.length); // revoked once each
	});

	test("a new activity releases the previous one's URLs", async () => {
		const { loader, live } = urlHarness();

		loader.pump([1, 2], 0);
		await settle();
		const before = live.size;
		expect(before).toBeGreaterThan(0);

		loader.reset();
		expect(live.size).toBe(0);

		loader.pump([3, 4], 0);
		await settle();
		expect(live.size).toBe(before); // bounded, not accumulated
	});

	test("a preview whose bytes land after a reset is revoked, not cached", async () => {
		let release: (() => void) | null = null;
		const stalled = new Promise<void>((resolve) => {
			release = resolve;
		});
		// Stall the byte read so the reset lands after the URL is minted but
		// before it could be cached — the superseded-mid-read branch.
		const { loader, live, revoked } = urlHarness(undefined, (async () => {
			await stalled;
			return { ok: true, arrayBuffer: async () => new ArrayBuffer(4) };
		}) as unknown as typeof fetch);

		loader.pump([1], 0);
		await settle();
		expect(live.size).toBe(0); // still reading bytes

		loader.reset();
		release?.();
		await settle();

		expect(loader.peekPreviewUrl(1)).toBeNull();
		expect(revoked).toEqual(["blob:1"]);
		expect(live.size).toBe(0);
	});
});

// ── Filmstrip thumbnails ────────────────────────────────────────────────
describe("ReceiptFrameLoader filmstrip thumbnails", () => {
	function thumbHarness(previewFor: (fid: number) => unknown) {
		const calls: number[][] = [];
		const thumbs: Array<[number, string]> = [];
		const revoked: string[] = [];
		let minted = 0;
		let resolveBatch: ((v: unknown) => void) | null = null;
		const loader = new ReceiptFrameLoader(
			{
				onPreview: () => {},
				onThumb: (fid, url) => thumbs.push([fid, url]),
				onMeta: () => {},
			},
			((cmd: string, args?: Record<string, unknown>) => {
				if (cmd !== "get_frame_scrub_previews") return Promise.resolve(null);
				const ids = (args?.request as { frameIds: number[] }).frameIds;
				calls.push(ids);
				return new Promise((resolve) => {
					resolveBatch = () =>
						resolve({
							previews: ids.map((fid) => ({
								frameId: fid,
								preview: previewFor(fid),
								missingReason: previewFor(fid) ? null : "source_missing",
							})),
						});
				});
			}) as InvokeFn,
			{
				// Thumbnails are painted as blob URLs, not `asset://` — the loader
				// fetches the bytes so it owns (and can revoke) the decoded surface.
				convertFileSrcImpl: (filePath: string) => `asset://${filePath}`,
				fetchImpl: async () => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(4) }),
				createObjectUrlImpl: () => `blob:thumb-${++minted}`,
				revokeObjectUrlImpl: (url: string) => revoked.push(url),
			},
		);
		return { loader, calls, thumbs, revoked, flush: () => resolveBatch?.(null) };
	}

	const scrubPreview = (fid: number) => ({
		mimeType: "image/jpeg",
		filePath: `/scrub/${fid}-200.jpg`,
		sourceKind: "generatedScrub",
		hasSecretRedactions: false,
		secretRedactionCount: 0,
	});

	test("cells batch into one scrub-preview call, not one full preview each", async () => {
		const { loader, calls, thumbs, flush } = thumbHarness(scrubPreview);

		// One IntersectionObserver callback → one request per cell → one batch.
		for (const fid of [1, 2, 3]) loader.requestThumb(fid);
		await settle();
		expect(calls).toEqual([[1, 2, 3]]); // one round trip, not three

		// Cells that scroll in while a batch is in flight wait for the next one.
		loader.requestThumb(4);
		await settle();
		expect(calls.length).toBe(1);

		flush();
		await settle();
		expect(thumbs).toEqual([
			[1, "blob:thumb-1"],
			[2, "blob:thumb-2"],
			[3, "blob:thumb-3"],
		]);
		expect(calls[1]).toEqual([4]);
	});

	test("a resolved cell is never re-requested; a missing one retries", async () => {
		const { loader, calls, flush } = thumbHarness((fid) =>
			fid === 2 ? null : scrubPreview(fid),
		);

		loader.requestThumb(1);
		loader.requestThumb(2);
		await settle();
		flush();
		await settle();
		expect(calls).toEqual([[1, 2]]);

		loader.requestThumb(1); // resolved → no new call
		await settle();
		expect(calls.length).toBe(1);

		loader.requestThumb(2); // had no preview → retried
		await settle();
		expect(calls[1]).toEqual([2]);
	});

	test("a new activity revokes the strip's thumbnail URLs", async () => {
		// The strip was the last path here still painting `asset://`, which WebKit
		// keeps a decoded surface for forever. Now the loader owns them, so moving
		// to another activity must actually hand them back.
		const { loader, thumbs, revoked, flush } = thumbHarness(scrubPreview);

		for (const fid of [1, 2]) loader.requestThumb(fid);
		await settle();
		flush();
		await settle();
		expect(thumbs.map(([, url]) => url)).toEqual(["blob:thumb-1", "blob:thumb-2"]);
		expect(revoked).toEqual([]);

		loader.reset();
		expect(revoked).toEqual(["blob:thumb-1", "blob:thumb-2"]);
	});
});
