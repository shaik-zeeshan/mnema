export type PreviewDisplaySource = "exact" | "scrub" | "none";

export type PreviewSourceKind =
  | "original_frame"
  | "segment_frame_fallback"
  | "video_fallback"
  | "scrub_preview";

export function activeDisplayPreviewPathForFrame(
  frameId: number | null,
  exactPreviewCache: ReadonlyMap<number, string>,
  scrubPreviewCache: ReadonlyMap<number, string>,
): string | null {
  if (frameId == null) return null;
  const exactPreviewPath = exactPreviewCache.get(frameId) ?? null;
  const scrubPreviewPath = scrubPreviewCache.get(frameId) ?? null;
  return exactPreviewPath ?? scrubPreviewPath;
}

export function activeDisplayPreviewSourceForFrame(
  frameId: number | null,
  exactPreviewCache: ReadonlyMap<number, string>,
  scrubPreviewCache: ReadonlyMap<number, string>,
): PreviewDisplaySource {
  if (frameId == null) return "none";
  if (exactPreviewCache.has(frameId)) return "exact";
  if (scrubPreviewCache.has(frameId)) return "scrub";
  return "none";
}

export function scrubPreviewShouldPopulateExactCache(_sourceKind: PreviewSourceKind): boolean {
  // Exact cache entries should come from `get_frame_preview`; scrub batches are
  // temporary display placeholders even when backed by an existing frame file.
  return false;
}

export function timelineMovementShouldScheduleScrubPreview(
  indexDelta: number,
  _velocityPxPerMs: number,
  _fastScrubThresholdPxPerMs: number,
): boolean {
  return indexDelta > 0;
}

export function scrubPreviewResponseShouldApply(
  requestGeneration: number,
  currentGeneration: number,
): boolean {
  return requestGeneration === currentGeneration;
}

export function activeExactPreviewDelayMs(
  shouldScheduleScrubPreview: boolean,
  _hasScrubPreview: boolean,
  settleMs: number,
): number {
  return shouldScheduleScrubPreview ? settleMs : 0;
}
