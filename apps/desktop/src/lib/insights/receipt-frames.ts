// receipt-frames — the invoke-touching frame-fetch machinery behind the
// Activity Receipt (ActivityReceipt.svelte), split out to keep that component
// under the 800-line ceiling: a bounded preview prefetch pump around the
// playhead, the filmstrip thumbnail queue, and per-frame display metadata.
// Everything async is generation-guarded: `reset()` (new activity) invalidates
// all in-flight work, so stale results are dropped, never painted. The pure
// window math (`desiredWindow`) and `LruCache` live in receipt-playback.ts.

import { invoke } from "@tauri-apps/api/core";
import { framePreviewAssetUrl } from "$lib/frame-preview";
import { LruCache, desiredWindow } from "$lib/insights/receipt-playback";
import type {
	FrameDto,
	FramePreviewDto,
	GetFramePreviewRequest,
} from "$lib/types/app-infra";

// ── Tuning knobs ─────────────────────────────────────────────────────────
const DECODE_CONCURRENCY = 2; // simultaneous get_frame_preview calls
const LOOKAHEAD = 6; // frames to prefetch ahead of the playhead
const BEHIND = 1; // frames to keep warm behind it
const PREVIEW_CACHE_CAP = 40; // LRU of decoded previews
const META_CACHE_CAP = 40; // LRU of per-frame FrameDto meta

export interface ReceiptFrameEvents {
	/** A preview landed — the display should re-read `peekPreview`. */
	onPreview(): void;
	/** A filmstrip thumbnail resolved to a renderable URL. */
	onThumb(frameId: number, url: string): void;
	/** Display metadata for the most recently requested frame. */
	onMeta(meta: FrameDto): void;
}

/** `invoke`-shaped IPC entry point, injectable so tests can stub Tauri. */
export type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

export class ReceiptFrameLoader {
	#events: ReceiptFrameEvents;
	#invoke: InvokeFn;
	#previews = new LruCache<FramePreviewDto>(PREVIEW_CACHE_CAP);
	#metaCache = new LruCache<FrameDto>(META_CACHE_CAP);
	#inFlight = new Set<number>();
	#failed = new Set<number>();
	#thumbQueue: number[] = [];
	#thumbInFlight = new Set<number>();
	#thumbDone = new Set<number>();
	#stripIds: number[] = [];
	#index = 0;
	#gen = 0; // bumped per reset; stale async work drops
	#metaToken = 0; // last-requested-meta wins

	constructor(events: ReceiptFrameEvents, invokeFn: InvokeFn = invoke) {
		this.#events = events;
		this.#invoke = invokeFn;
	}

	/** New activity: drop caches/queues and invalidate all in-flight work. */
	reset(): void {
		this.#gen++;
		this.#previews = new LruCache(PREVIEW_CACHE_CAP);
		this.#metaCache = new LruCache(META_CACHE_CAP);
		this.#inFlight.clear();
		this.#failed.clear();
		this.#thumbQueue.length = 0;
		this.#thumbDone.clear();
		this.#stripIds = [];
		this.#index = 0;
	}

	/** Read a decoded preview without touching LRU order (safe from deriveds). */
	peekPreview(frameId: number): FramePreviewDto | null {
		return this.#previews.peek(frameId) ?? null;
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
				this.#previews.set(fid, dto);
				// ponytail: warm the browser decode with a throwaway Image so the
				// <img> swap is instant (no CSS transition) — that's what sells the
				// "video" feel over raw frames.
				const warm = new Image();
				warm.src = framePreviewAssetUrl(dto.filePath);
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
	// Every visible cell requests its preview once; a DECODE_CONCURRENCY pump
	// keeps decode load bounded. Resolved URLs are handed to the component
	// (outside the playback LRU) so eviction during playback never blanks a
	// loaded thumb.
	requestThumb(fid: number): void {
		if (!Number.isFinite(fid) || this.#thumbDone.has(fid)) return;
		if (this.#thumbInFlight.has(fid) || this.#thumbQueue.includes(fid)) return;
		this.#thumbQueue.push(fid);
		this.#pumpThumbs();
	}

	#pumpThumbs(): void {
		while (this.#thumbInFlight.size < DECODE_CONCURRENCY) {
			const fid = this.#thumbQueue.shift();
			if (fid === undefined) return;
			this.#thumbInFlight.add(fid);
			void this.#fetchThumb(fid, this.#gen);
		}
	}

	async #fetchThumb(fid: number, gen: number): Promise<void> {
		try {
			const dto =
				this.#previews.peek(fid) ??
				(await this.#invoke<FramePreviewDto | null>("get_frame_preview", {
					request: { frameId: fid } satisfies GetFramePreviewRequest,
				}));
			if (gen === this.#gen && dto) {
				this.#thumbDone.add(fid);
				this.#events.onThumb(fid, framePreviewAssetUrl(dto.filePath));
			}
		} catch {
			// cell keeps its placeholder
		} finally {
			this.#thumbInFlight.delete(fid);
			this.#pumpThumbs();
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
