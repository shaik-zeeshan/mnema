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
  signal?: AbortSignal,
): Promise<Uint8Array> {
  const fetchImpl = deps.fetchImpl ?? fetch;
  const assetUrl = framePreviewAssetUrl(filePath, deps);
  const response = await fetchImpl(assetUrl, signal ? { signal } : undefined);
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
  // The read behind the swap in flight, so the next swap can cancel it — see
  // `swap`.
  #inFlight: AbortController | null = null;

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
   *
   * The read it supersedes is CANCELLED, not just ignored: DetailPane swaps once
   * per selection (macOS key repeat holds ↓ at ~15-30/s) and the timeline stage
   * once per 80 ms of scrubbing, so leaving them running materialises a hero's
   * worth of bytes per dead frame in the JS heap and makes the webview's
   * scheme-handler queue serve them ahead of the frame the user stopped on. The
   * `<img src>` this replaced got that for free — the browser cancels the
   * outgoing load on every src change.
   */
  async swap(filePath: string, mimeType?: string | null): Promise<string | null> {
    const generation = ++this.#generation;
    this.#abortInFlight();
    const controller = typeof AbortController === "undefined" ? null : new AbortController();
    this.#inFlight = controller;
    let bytes: Uint8Array;
    try {
      bytes = await readFramePreviewBytes(filePath, this.#deps, controller?.signal);
    } catch (error) {
      // A swap that already lost the race reports "superseded", never the
      // failure: callers route a rejection to their no-hero path (DetailPane
      // `clear()`s the holder, the timeline stage runs its decode-retry loop),
      // which would revoke and blank the newer preview that painted fine.
      if (generation !== this.#generation) return null;
      throw error;
    } finally {
      if (this.#inFlight === controller) this.#inFlight = null;
    }
    if (generation !== this.#generation) return null;
    const create = this.#deps.createObjectUrlImpl ?? URL.createObjectURL;
    const url = create(new Blob([bytes], mimeType ? { type: mimeType } : undefined));
    if (this.#current) this.#retired.add(this.#current);
    this.#current = url;
    // Callers assign this URL from a `.then`, one microtask after the mint. A
    // `clear()` already queued ahead of that continuation — the pane's `$effect`
    // flush when the hero goes away, or its unmount teardown — would revoke `url`
    // before the caller ever paints it, and nothing re-assigns afterwards, so the
    // caller paints a dead blob URL for good (DetailPane gates its hero on
    // `heroUrl`, not `heroPath`, so it never re-runs to correct itself). Yield once
    // so an already-queued teardown lands first, then report it as superseded like
    // any other swap the holder has disowned. Nothing is stranded either way:
    // `clear` already revoked it, and a superseding `swap` retires it as usual.
    await Promise.resolve();
    if (generation !== this.#generation) return null;
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
    // ...and stop their reads: an unmounted pane's hero is bytes nobody wants.
    this.#abortInFlight();
  }

  #abortInFlight(): void {
    this.#inFlight?.abort();
    this.#inFlight = null;
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
 * How many preview reads a {@link FramePreviewUrlMap} keeps in flight — across
 * every `merge` running against it, not per call.
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
  // Frames a concurrent `merge` is already reading. Not in `#urls` yet, so without
  // this a second merge re-reads them: Quick Recall debounces at 250 ms and filters
  // candidates on `thumbnailCache`, which the first merge has not assigned yet — a
  // pause-then-type query reads every overlapping frame twice.
  #reading = new Set<number>();
  // Bumped by `clear()` so an in-flight `merge` stops minting into a map the
  // owner has already released.
  #generation = 0;
  // The map's ONE read pool — see `#schedule`.
  #queue: (() => Promise<void>)[] = [];
  #active = 0;
  // `clear()` is a RELEASE, not a reset: the owner is gone (component teardown,
  // discarded results) and will never call it again. Merges that start after it
  // must mint nothing — see `merge`. Reuse means a fresh map, not a cleared one.
  #released = false;

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
   *
   * `onMinted` fires as each URL lands, BEFORE the batch resolves. Callers should
   * use it to publish, or the grid stays glyph-only until the slowest read in the
   * batch returns: 24 reads through a 6-worker pool is four SEQUENTIAL asset round
   * trips, where the `asset://` code this replaced assigned the map synchronously
   * and painted after one.
   *
   * A released map (see `clear`) merges nothing: its owner's teardown has already
   * run, so every consumer of these loaders reads the IPC FIRST and merges when it
   * returns — the destroy lands in that gap, and a URL minted after it has nobody
   * left to revoke it.
   */
  async merge(
    entries: Iterable<{ frameId: number; preview: { filePath: string; mimeType: string } }>,
    onMinted?: (frameId: number, url: string) => void,
  ): Promise<Map<number, string>> {
    if (this.#released) return this.snapshot();
    // `get` (not `peek`) so an already-held frame is still marked
    // most-recently-used — a re-search over the same results must not make its
    // own thumbnails the next eviction candidates.
    const pending: { frameId: number; preview: { filePath: string; mimeType: string } }[] = [];
    const claimed = new Set<number>();
    for (const entry of entries) {
      if (this.#urls.get(entry.frameId) !== undefined) continue;
      if (this.#reading.has(entry.frameId)) continue; // a concurrent merge owns it
      if (claimed.has(entry.frameId)) continue; // a repeated id must not double-fetch
      claimed.add(entry.frameId);
      this.#reading.add(entry.frameId);
      pending.push(entry);
    }

    // `clear()` (unmount teardown, discarded results) routinely lands inside
    // this loop — it is the LAST call a component-scoped map ever gets, so a
    // URL minted after it is stranded for the life of the WebContent process,
    // which is the leak blob URLs exist to stop. The generation makes `clear`
    // abort the workers instead of racing them.
    const generation = this.#generation;
    const read = async (entry: {
      frameId: number;
      preview: { filePath: string; mimeType: string };
    }): Promise<void> => {
      if (generation !== this.#generation) return; // cleared before its turn
      const { frameId, preview } = entry;
      try {
        const bytes = await readFramePreviewBytes(preview.filePath, this.#deps);
        if (generation !== this.#generation) return; // cleared mid-read
        const create = this.#deps.createObjectUrlImpl ?? URL.createObjectURL;
        const url = create(new Blob([bytes], { type: preview.mimeType }));
        this.#urls.set(frameId, url);
        // Publish now, not at the end of the batch: the caller repaints this one
        // cell a round trip after the read that produced it.
        onMinted?.(frameId, url);
      } catch {
        // Best-effort: leave the frame out so a later pass can retry it.
      }
    };
    try {
      // Through the map's ONE pool, not a pool per call — see `#schedule`.
      await Promise.all(pending.map((entry) => this.#schedule(() => read(entry))));
    } finally {
      // Every exit path (resolved, aborted mid-batch, thrown) hands the claims back,
      // or a later merge would skip those frames forever.
      for (const frameId of claimed) this.#reading.delete(frameId);
    }
    return this.snapshot();
  }

  /**
   * Run `job` when the map's read pool has a free slot.
   *
   * The pool belongs to the MAP, not to one `merge` call: callers fan merges out
   * over the same map — `Chat.hydrateConversation` fires one per turn (`for
   * (const t of turns) void loadSourceThumbnails(t.sources)`), Quick Recall
   * overlaps a debounced search with the answer-source pass — so a per-call pool
   * multiplies {@link FRAME_PREVIEW_MERGE_CONCURRENCY} by the number of merges
   * in flight. A 20-turn thread opened 120 simultaneous asset reads, each
   * materialising its bytes on the main thread in one burst.
   */
  #schedule(job: () => Promise<void>): Promise<void> {
    return new Promise<void>((resolve) => {
      this.#queue.push(() => job().finally(resolve));
      this.#drain();
    });
  }

  #drain(): void {
    while (this.#active < FRAME_PREVIEW_MERGE_CONCURRENCY && this.#queue.length > 0) {
      const job = this.#queue.shift();
      if (!job) return;
      this.#active += 1;
      void job().finally(() => {
        this.#active -= 1;
        this.#drain();
      });
    }
  }

  /**
   * Mark a frame the caller is STILL SHOWING as most-recently-used, and report
   * whether it is held.
   *
   * Every caller filters the frames it already holds out of the request BEFORE
   * `merge` ever sees them, so the touch inside `merge` never fires on the real
   * path: without this the live set is ordered by FIRST MINT, and the frame that
   * matches search after search is the oldest one — evicted (and revoked) while
   * its own card is on screen, with nothing to re-request it for that view.
   */
  touch(frameId: number): boolean {
    return this.#urls.get(frameId) !== undefined;
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

  /**
   * Release the map: revoke everything and mint nothing ever again. Call it on
   * unmount, or when the results it belongs to are discarded.
   *
   * Permanent on purpose. The generation alone only aborts a merge that is
   * ALREADY running, and every consumer awaits `get_frame_scrub_previews` before
   * it merges — so the teardown routinely lands BEFORE the merge starts, and a
   * URL minted then is stranded for the life of the WebContent process. An owner
   * that needs the map again (a new activity's filmstrip) builds a fresh one.
   */
  clear(): void {
    // Bump first: an in-flight `merge` must not mint into a map that will
    // never be cleared again.
    this.#generation += 1;
    this.#released = true;
    this.#urls.clear();
  }
}
