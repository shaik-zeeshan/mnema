// receipt-frames — the invoke-touching frame-fetch machinery behind the
// Activity Receipt (ActivityReceipt.svelte), split out to keep that component
// under the 800-line ceiling: a bounded preview prefetch pump around the
// playhead, the filmstrip thumbnail queue, and per-frame display metadata.
// Everything async is generation-guarded: `reset()` (new activity) invalidates
// all in-flight work, so stale results are dropped, never painted. The pure
// window math (`desiredWindow`) and `LruCache` live in receipt-playback.ts.

import { invoke } from "@tauri-apps/api/core";
import {
	FramePreviewUrlMap,
	readFramePreviewBytes,
	type FramePreviewUrlDependencies,
} from "$lib/frame-preview";
import { LruCache, desiredWindow } from "$lib/insights/receipt-playback";
import type {
	FrameDto,
	FramePreviewDto,
	FrameScrubPreviewsDto,
	GetFramePreviewRequest,
	GetFrameScrubPreviewsRequest,
} from "$lib/types/app-infra";

// ── Tuning knobs ─────────────────────────────────────────────────────────
const DECODE_CONCURRENCY = 2; // simultaneous get_frame_preview calls
const LOOKAHEAD = 6; // frames to prefetch ahead of the playhead
const BEHIND = 1; // frames to keep warm behind it
const PREVIEW_CACHE_CAP = 40; // LRU of decoded previews
const THUMB_BATCH_SIZE = 24; // filmstrip cells per get_frame_scrub_previews call
const META_CACHE_CAP = 40; // LRU of per-frame FrameDto meta

/** A cached playback frame: its DTO plus the object URL the viewer paints. */
type CachedPreview = { dto: FramePreviewDto; url: string };

export interface ReceiptFrameEvents {
	/** A preview landed — the display should re-read `peekPreview`. */
	onPreview(): void;
	/**
	 * A filmstrip thumbnail resolved to a renderable URL, or `null` when the URL
	 * previously handed out was revoked (LRU eviction / reset) — the cell must
	 * drop it and become re-requestable, not keep painting a dead blob URL.
	 */
	onThumb(frameId: number, url: string | null): void;
	/** Display metadata for the most recently requested frame. */
	onMeta(meta: FrameDto): void;
}

/** `invoke`-shaped IPC entry point, injectable so tests can stub Tauri. */
export type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

export class ReceiptFrameLoader {
	#events: ReceiptFrameEvents;
	#invoke: InvokeFn;
	#urlDeps: FramePreviewUrlDependencies;
	// Playback paints object URLs, never the frames' `asset://` URLs: WebKit
	// keeps a decoded IOSurface per asset URL it has ever loaded and only drops
	// them on an explicit purge, so a receipt played end to end used to strand
	// one full-size surface per frame for the life of the window. Minting the
	// URL here — at prefetch time, warm-decoded like before — keeps the instant
	// swap AND caps the live surfaces at the LRU's size, because eviction now
	// revokes.
	#previews = new LruCache<CachedPreview>(PREVIEW_CACHE_CAP, (cached) =>
		this.#revokeUrl(cached.url),
	);
	// Same reasoning as `#previews`, for the strip's thumbnails: they were the one
	// path here still painting `asset://`, so a long receipt stranded a decoded
	// surface per thumbnail on top of the full-size ones. Bounded + revoking.
	// Built in the constructor, not here: `#urlDeps` is only assigned there, and a
	// field initializer would capture it as `undefined`.
	#thumbUrls: FramePreviewUrlMap;
	#metaCache = new LruCache<FrameDto>(META_CACHE_CAP);
	#inFlight = new Set<number>();
	#failed = new Set<number>();
	#thumbQueue: number[] = [];
	#thumbInFlight = new Set<number>();
	#thumbDone = new Set<number>();
	#thumbPumpScheduled = false;
	#stripIds: number[] = [];
	#index = 0;
	#gen = 0; // bumped per reset; stale async work drops
	#metaToken = 0; // last-requested-meta wins
	#releasing = false; // inside reset/dispose: suppress per-cell retractions

	constructor(
		events: ReceiptFrameEvents,
		invokeFn: InvokeFn = invoke,
		urlDeps: FramePreviewUrlDependencies = {},
	) {
		this.#events = events;
		this.#invoke = invokeFn;
		this.#urlDeps = urlDeps;
		// The strip renders a cell per frame of the span and keeps whatever URL it
		// was handed, so a receipt longer than the map's cap evicts (and revokes)
		// URLs that are still assigned into the component's record. Retract them:
		// the cell drops the dead URL, and clearing `#thumbDone` lets it re-fetch
		// when it scrolls back into view.
		this.#thumbUrls = new FramePreviewUrlMap(urlDeps, undefined, (fid) => {
			if (this.#releasing) return; // reset/dispose: the strip drops the lot anyway
			this.#thumbDone.delete(fid);
			this.#events.onThumb(fid, null);
		});
	}

	/** New activity: drop caches/queues and invalidate all in-flight work. */
	reset(): void {
		this.#gen++;
		// `clear()`, not a fresh LruCache: the cache owns live object URLs and
		// replacing it would strand every one of them.
		this.#previews.clear();
		this.#releaseThumbUrls();
		this.#metaCache.clear();
		this.#inFlight.clear();
		this.#failed.clear();
		this.#thumbQueue.length = 0;
		// `#pumpThumbs` is one-batch-at-a-time, gated on this set. The batch that
		// belongs to the activity we just left is still in flight and its `finally` is
		// generation-guarded, so without this the NEW strip never pumps at all: every
		// newly-intersecting cell calls `requestThumb`, `#pumpThumbs` returns early on
		// the stale claim, and the filmstrip stays blank until the old IPC returns.
		this.#thumbInFlight.clear();
		this.#thumbDone.clear();
		this.#stripIds = [];
		this.#index = 0;
	}

	/** Read a decoded preview without touching LRU order (safe from deriveds). */
	peekPreview(frameId: number): FramePreviewDto | null {
		return this.#previews.peek(frameId)?.dto ?? null;
	}

	/** The object URL to paint for `frameId`, or null when it isn't cached. */
	peekPreviewUrl(frameId: number): string | null {
		return this.#previews.peek(frameId)?.url ?? null;
	}

	/** Unmount: invalidate in-flight work and revoke every live object URL. */
	dispose(): void {
		this.#gen++;
		this.#previews.clear();
		this.#releaseThumbUrls();
	}

	/** Wholesale thumbnail release — revoke without retracting cell by cell. */
	#releaseThumbUrls(): void {
		this.#releasing = true;
		try {
			this.#thumbUrls.clear();
		} finally {
			this.#releasing = false;
		}
	}

	// ── Bounded, cancellable preview prefetch ──────────────────────────────
	// Keeps ≤DECODE_CONCURRENCY get_frame_preview calls in flight, fetching the
	// lookahead window around the playhead. Out-of-window frames are simply
	// never scheduled, so a long activity or a scrub never thrashes the decoder.
	pump(stripIds: number[], index: number): void {
		this.#stripIds = stripIds;
		this.#index = index;
		if (stripIds.length === 0) return;
		const gen = this.#gen;
		for (const fid of desiredWindow(stripIds, index, LOOKAHEAD, BEHIND)) {
			if (this.#inFlight.size >= DECODE_CONCURRENCY) break;
			if (this.#previews.has(fid) || this.#inFlight.has(fid) || this.#failed.has(fid)) continue;
			this.#inFlight.add(fid);
			void this.#fetchPreview(fid, gen);
		}
	}

	async #fetchPreview(fid: number, gen: number): Promise<void> {
		try {
			const dto = await this.#invoke<FramePreviewDto | null>("get_frame_preview", {
				request: { frameId: fid } satisfies GetFramePreviewRequest,
			});
			if (gen !== this.#gen) return; // superseded — drop
			if (dto) {
				const url = await this.#mintPreviewUrl(dto);
				if (gen !== this.#gen) {
					this.#revokeUrl(url); // superseded mid-read — never cached, never painted
					return;
				}
				this.#previews.set(fid, { dto, url });
				// ponytail: warm the browser decode with a throwaway Image so the
				// <img> swap is instant (no CSS transition) — that's what sells the
				// "video" feel over raw frames.
				if (typeof Image !== "undefined") {
					const warm = new Image();
					warm.src = url;
				}
				this.#events.onPreview();
			} else {
				this.#failed.add(fid);
			}
		} catch {
			if (gen === this.#gen) this.#failed.add(fid);
		} finally {
			this.#inFlight.delete(fid);
			if (gen === this.#gen) this.pump(this.#stripIds, this.#index);
		}
	}

	// ── Filmstrip thumbnails ───────────────────────────────────────────────
	// Every visible cell requests its thumbnail once; one batch is in flight at
	// a time. Resolved URLs are handed to the component (outside the playback
	// LRU) so eviction during playback never blanks a loaded thumb.
	//
	// Thumbnails come from `get_frame_scrub_previews` — the 200px scrub preview,
	// same source as Quick Recall's result rows — NOT the playback preview. A
	// cell is ~70×44 CSS px, so painting the full-size preview there decoded a
	// surface ~30× the pixels the cell can show, and WebKit keeps one per URL
	// for the life of the window: a long receipt's strip alone ran to GBs.
	requestThumb(fid: number): void {
		if (!Number.isFinite(fid) || this.#thumbDone.has(fid)) return;
		if (this.#thumbInFlight.has(fid) || this.#thumbQueue.includes(fid)) return;
		this.#thumbQueue.push(fid);
		this.#scheduleThumbPump();
	}

	// One IntersectionObserver callback delivers each newly-visible cell as its
	// own `requestThumb`, so coalescing to a microtask turns a burst into one
	// batch instead of a round trip for the first cell and a batch for the rest.
	#scheduleThumbPump(): void {
		if (this.#thumbPumpScheduled) return;
		this.#thumbPumpScheduled = true;
		queueMicrotask(() => {
			this.#thumbPumpScheduled = false;
			this.#pumpThumbs();
		});
	}

	#pumpThumbs(): void {
		if (this.#thumbInFlight.size > 0) return; // one batch at a time
		const batch = this.#thumbQueue.splice(0, THUMB_BATCH_SIZE);
		if (batch.length === 0) return;
		for (const fid of batch) this.#thumbInFlight.add(fid);
		void this.#fetchThumbs(batch, this.#gen);
	}

	async #mintPreviewUrl(dto: FramePreviewDto): Promise<string> {
		const bytes = await readFramePreviewBytes(dto.filePath, this.#urlDeps);
		const create = this.#urlDeps.createObjectUrlImpl ?? URL.createObjectURL;
		return create(new Blob([bytes], { type: dto.mimeType }));
	}

	#revokeUrl(url: string): void {
		const revoke = this.#urlDeps.revokeObjectUrlImpl ?? URL.revokeObjectURL;
		revoke(url);
	}

	async #fetchThumbs(fids: number[], gen: number): Promise<void> {
		try {
			const response = await this.#invoke<FrameScrubPreviewsDto>(
				"get_frame_scrub_previews",
				// No `maxPixelSize`: the backend's default (200) is also its floor.
				{ request: { frameIds: fids } satisfies GetFrameScrubPreviewsRequest },
			);
			if (gen !== this.#gen) return;
			// A `missingReason` entry has no preview — leave it out of `#thumbDone`
			// so a later re-intersection retries it.
			// Painted per cell as each read lands, not after the whole batch: 24
			// thumbnails through the merge's 6-worker pool is four SEQUENTIAL asset
			// round trips, and waiting for the last one holds every cell on its
			// placeholder for the entire batch. The per-mint generation check is the
			// same one the batch used to do — a `reset` mid-batch already revoked
			// these URLs, so nothing may be handed to the strip after it.
			await this.#thumbUrls.merge(
				response.previews.flatMap((entry) =>
					entry.preview ? [{ frameId: entry.frameId, preview: entry.preview }] : [],
				),
				(frameId, url) => {
					if (gen !== this.#gen) return;
					this.#thumbDone.add(frameId);
					this.#events.onThumb(frameId, url);
				},
			);
		} catch {
			// cells keep their placeholders
		} finally {
			// A superseded batch owns none of this state any more — `reset` already
			// dropped its claims, and deleting them here would release ids the new
			// activity has in flight (and pump a second concurrent batch).
			if (gen === this.#gen) {
				for (const fid of fids) this.#thumbInFlight.delete(fid);
				this.#pumpThumbs();
			}
		}
	}

	// ── Display-only per-frame metadata (app / window / OCR-present) ──────
	// Token-guarded so a slow response never paints onto a newer frame.
	async loadMeta(fid: number): Promise<void> {
		// Claim the latest-meta token up front — even a cache hit must supersede
		// an older still-in-flight request, or that slow fetch would resolve and
		// paint its (now-stale) chips over the frame we just jumped to.
		const token = ++this.#metaToken;
		const cached = this.#metaCache.peek(fid);
		if (cached) {
			this.#events.onMeta(cached);
			return;
		}
		const gen = this.#gen;
		try {
			const dto = await this.#invoke<FrameDto | null>("get_frame", {
				request: { frameId: fid },
			});
			if (token !== this.#metaToken || gen !== this.#gen) return;
			if (dto) {
				this.#metaCache.set(fid, dto);
				this.#events.onMeta(dto);
			}
		} catch {
			// keep the last-shown chips; a transient failure shouldn't blank them
		}
	}
}
