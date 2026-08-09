import { convertFileSrc } from "@tauri-apps/api/core";

import { LruCache } from "$lib/insights/receipt-playback";

type FramePreviewFetchDependencies = {
  convertFileSrcImpl?: (filePath: string) => string;
  fetchImpl?: typeof fetch;
};

export type FramePreviewUrlDependencies = FramePreviewFetchDependencies & {
  createObjectUrlImpl?: (blob: Blob) => string;
  revokeObjectUrlImpl?: (url: string) => void;
};

export function framePreviewAssetUrl(
  filePath: string,
  deps: Pick<FramePreviewFetchDependencies, "convertFileSrcImpl"> = {},
): string {
  const convert = deps.convertFileSrcImpl ?? convertFileSrc;
  return convert(filePath);
}

export async function readFramePreviewBytes(
  filePath: string,
  deps: FramePreviewFetchDependencies = {},
): Promise<Uint8Array> {
  const fetchImpl = deps.fetchImpl ?? fetch;
  const assetUrl = framePreviewAssetUrl(filePath, deps);
  const response = await fetchImpl(assetUrl);
  if (!response.ok) {
    throw new Error(`frame preview fetch failed: ${response.status} ${response.statusText}`.trim());
  }
  return new Uint8Array(await response.arrayBuffer());
}

/**
 * Owns the one object URL the frame stage paints.
 *
 * Painting `asset://` URLs directly leaks GPU memory: WebKit keeps one decoded
 * IOSurface per URL it has ever loaded and only drops them on an explicit purge
 * (`webview_cache.rs`) or system memory pressure — which macOS answers by
 * swapping. An afternoon of scrubbing mints a URL per frame, so the WebContent
 * process parks a surface per frame forever (measured 1.7 GB of IOAccelerator
 * memory, and the blur-purge only reclaimed ~10 MB of it).
 *
 * Fetching the bytes ourselves and painting a blob URL puts the lifetime back
 * under our control: `swap` retires the URL it replaces, and `settle` revokes
 * the retired ones once the replacement has actually painted — so exactly one
 * full-size preview is decoded at a time, whatever the scrub length.
 */
export class FramePreviewUrlHolder {
  #deps: FramePreviewUrlDependencies;
  #current: string | null = null;
  // Superseded URLs still painted by the DOM. Revoking one before its
  // replacement loads would blank the stage mid-scrub, so they wait for
  // `settle`. More than one only piles up when swaps outrun paints.
  #retired = new Set<string>();
  #generation = 0;

  constructor(deps: FramePreviewUrlDependencies = {}) {
    this.#deps = deps;
  }

  get current(): string | null {
    return this.#current;
  }

  /**
   * Fetch `filePath` and become its object URL. Returns the URL to paint, or
   * `null` when a newer `swap` superseded this one mid-fetch (nothing was
   * created, so there is nothing to revoke). A failed fetch throws — callers
   * route that to their existing preview-error path — but only while this swap
   * is still the current one; a superseded failure reports `null` like any
   * other superseded swap.
   */
  async swap(filePath: string, mimeType?: string | null): Promise<string | null> {
    const generation = ++this.#generation;
    let bytes: Uint8Array;
    try {
      bytes = await readFramePreviewBytes(filePath, this.#deps);
    } catch (error) {
      // A swap that already lost the race reports "superseded", never the
      // failure: callers route a rejection to their no-hero path (DetailPane
      // `clear()`s the holder, the timeline stage runs its decode-retry loop),
      // which would revoke and blank the newer preview that painted fine.
      if (generation !== this.#generation) return null;
      throw error;
    }
    if (generation !== this.#generation) return null;
    const create = this.#deps.createObjectUrlImpl ?? URL.createObjectURL;
    const url = create(new Blob([bytes], mimeType ? { type: mimeType } : undefined));
    if (this.#current) this.#retired.add(this.#current);
    this.#current = url;
    return url;
  }

  /** The replacement painted — drop every URL it replaced. */
  settle(): void {
    for (const url of this.#retired) this.#revoke(url);
    this.#retired.clear();
  }

  /** Nothing to show any more (no frame, or unmount) — revoke everything. */
  clear(): void {
    this.settle();
    if (this.#current) this.#revoke(this.#current);
    this.#current = null;
    // Invalidate in-flight swaps so a late one cannot resurrect a URL.
    this.#generation += 1;
  }

  #revoke(url: string): void {
    const revoke = this.#deps.revokeObjectUrlImpl ?? URL.revokeObjectURL;
    revoke(url);
  }
}

/**
 * How many thumbnail URLs stay live at once.
 *
 * Thumbnails come back from `get_frame_scrub_previews` at the backend's default
 * 200 px, so a decoded one is roughly 160 KB — this cap bounds a grid to ~40 MB
 * of decoded surface however long the session runs. Comfortably above a full
 * screen of result cards, so scrolling back up never re-fetches.
 */
export const FRAME_PREVIEW_URL_CAP = 256;

/**
 * How many preview reads a single {@link FramePreviewUrlMap.merge} keeps in
 * flight.
 *
 * A cold merge is all-or-nothing — the caller assigns the snapshot to reactive
 * state only once the whole batch resolves — and the batches are big: Quick
 * Recall merges `FRAME_FETCH_LIMIT` (24) result thumbnails per search, the
 * filmstrip merges `THUMB_BATCH_SIZE` (24) cells per pump. Awaiting them one at
 * a time stacks 24 asset-protocol round trips before the first thumbnail
 * paints, where the `asset://` code this replaced assigned the map
 * synchronously and let WebKit fetch them in parallel.
 *
 * Bounded rather than `Promise.all`: an unbounded fan-out would hand the whole
 * batch to the asset-protocol handler at once on every keystroke-settle.
 * ponytail: 6 mirrors a browser's per-origin connection budget; raise it only
 * with a measurement.
 */
export const FRAME_PREVIEW_MERGE_CONCURRENCY = 6;

/**
 * The frame-id → preview-URL map behind every thumbnail grid (Quick Recall
 * results, Chat/Subject receipts, the scrub strip).
 *
 * Same lifetime problem as {@link FramePreviewUrlHolder}, one-per-frame instead
 * of one-per-stage: painting `asset://` URLs parks a decoded IOSurface in the
 * WebContent process for every URL it has ever loaded, and those are never
 * revocable because the URL is a stable function of the file path — an afternoon
 * of searching keeps every thumbnail the user has scrolled past. Measured on a
 * 10h session: 271 MB of graphics memory across 680 regions in one WebContent
 * process, none of it reclaimable.
 *
 * Blob URLs put the lifetime back under our control, and the bounded LRU decides
 * when: eviction past {@link FRAME_PREVIEW_URL_CAP} (and `clear`) revokes, so the
 * live set is capped rather than merely revocable.
 *
 * ponytail: reuses `LruCache`'s `onEvict` rather than tracking URLs here — that
 * hook exists for exactly this ("values holding a resource GC cannot reclaim").
 */
export class FramePreviewUrlMap {
  #deps: FramePreviewUrlDependencies;
  #urls: LruCache<string>;
  // Bumped by `clear()` so an in-flight `merge` stops minting into a map the
  // owner has already released.
  #generation = 0;

  /**
   * `onDropped` fires for each frame whose URL was just revoked (eviction,
   * `clear`). Callers that hand URLs out one at a time — rather than
   * re-assigning the whole {@link snapshot} — MUST use it to retract the dead
   * URL, or they keep painting it forever.
   */
  constructor(
    deps: FramePreviewUrlDependencies = {},
    capacity = FRAME_PREVIEW_URL_CAP,
    onDropped?: (frameId: number) => void,
  ) {
    this.#deps = deps;
    this.#urls = new LruCache<string>(capacity, (url, frameId) => {
      const revoke = this.#deps.revokeObjectUrlImpl ?? URL.revokeObjectURL;
      revoke(url);
      onDropped?.(frameId);
    });
  }

  /**
   * Mint a URL for each preview not already held, then return the full live map
   * for the caller to assign to reactive state.
   *
   * A frame already in the map is only touched (marked most-recently-used), never
   * re-fetched — repeated searches over the same results cost nothing. A fetch
   * failure is swallowed per frame: thumbnails are best-effort and the card falls
   * back to its glyph, exactly as before.
   */
  async merge(
    entries: Iterable<{ frameId: number; preview: { filePath: string; mimeType: string } }>,
  ): Promise<Map<number, string>> {
    // `get` (not `peek`) so an already-held frame is still marked
    // most-recently-used — a re-search over the same results must not make its
    // own thumbnails the next eviction candidates.
    const pending: { frameId: number; preview: { filePath: string; mimeType: string } }[] = [];
    const claimed = new Set<number>();
    for (const entry of entries) {
      if (this.#urls.get(entry.frameId) !== undefined) continue;
      if (claimed.has(entry.frameId)) continue; // a repeated id must not double-fetch
      claimed.add(entry.frameId);
      pending.push(entry);
    }

    // `clear()` (unmount teardown, discarded results) routinely lands inside
    // this loop — it is the LAST call a component-scoped map ever gets, so a
    // URL minted after it is stranded for the life of the WebContent process,
    // which is the leak blob URLs exist to stop. The generation makes `clear`
    // abort the workers instead of racing them.
    const generation = this.#generation;
    let next = 0;
    const worker = async (): Promise<void> => {
      while (next < pending.length) {
        if (generation !== this.#generation) return;
        const { frameId, preview } = pending[next++];
        try {
          const bytes = await readFramePreviewBytes(preview.filePath, this.#deps);
          if (generation !== this.#generation) return; // cleared mid-read
          const create = this.#deps.createObjectUrlImpl ?? URL.createObjectURL;
          this.#urls.set(frameId, create(new Blob([bytes], { type: preview.mimeType })));
        } catch {
          // Best-effort: leave the frame out so a later pass can retry it.
        }
      }
    };
    await Promise.all(
      Array.from({ length: Math.min(FRAME_PREVIEW_MERGE_CONCURRENCY, pending.length) }, worker),
    );
    return this.snapshot();
  }

  /** The live frame-id → URL pairs. Reads without disturbing LRU order. */
  snapshot(): Map<number, string> {
    const out = new Map<number, string>();
    for (const frameId of this.#urls.keys()) {
      const url = this.#urls.peek(frameId);
      if (url !== undefined) out.set(frameId, url);
    }
    return out;
  }

  /** Revoke everything — call on unmount, or when results are discarded. */
  clear(): void {
    // Bump first: an in-flight `merge` must not mint into a map that will
    // never be cleared again.
    this.#generation += 1;
    this.#urls.clear();
  }
}
