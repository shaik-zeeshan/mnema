import { convertFileSrc } from "@tauri-apps/api/core";

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
   * route that to their existing preview-error path.
   */
  async swap(filePath: string, mimeType?: string | null): Promise<string | null> {
    const generation = ++this.#generation;
    const bytes = await readFramePreviewBytes(filePath, this.#deps);
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
