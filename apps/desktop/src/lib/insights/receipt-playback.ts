// Pure playback helpers for the Activity Receipt (ActivityReceipt.svelte): a
// bounded LRU for decoded frame previews, index stepping with clamped bounds,
// even filmstrip sampling, nearest-sample lookup, a speed→fps mapping, and a
// gap-based capture-segment count. No DOM, no Svelte — unit-testable in bun.

/**
 * Bounded insertion-order LRU. `get`/`set` mark a key most-recently-used;
 * setting past `capacity` evicts the least-recently-used key. `peek` reads
 * without reordering (used from Svelte deriveds so display never mutates order).
 * Values are opaque (preview DTOs, frame DTOs, …); eviction just drops the
 * reference and lets GC reclaim it.
 */
export class LruCache<V> {
  private map = new Map<number, V>();
  constructor(private capacity: number) {}

  peek(key: number): V | undefined {
    return this.map.get(key);
  }

  has(key: number): boolean {
    return this.map.has(key);
  }

  get(key: number): V | undefined {
    const v = this.map.get(key);
    if (v === undefined) return undefined;
    this.map.delete(key);
    this.map.set(key, v);
    return v;
  }

  set(key: number, value: V): void {
    if (this.map.has(key)) this.map.delete(key);
    this.map.set(key, value);
    while (this.map.size > this.capacity) {
      // Map preserves insertion order; the first key is the LRU.
      const lru = this.map.keys().next().value as number;
      this.map.delete(lru);
    }
  }

  get size(): number {
    return this.map.size;
  }

  keys(): number[] {
    return [...this.map.keys()];
  }
}

/** Clamp an index into `[0, count-1]` (0 when empty). */
export function clampIndex(i: number, count: number): number {
  if (count <= 0) return 0;
  return Math.max(0, Math.min(count - 1, i));
}

/** Step an index by `delta`, clamped to the strip bounds. */
export function stepIndex(current: number, delta: number, count: number): number {
  return clampIndex(current + delta, count);
}

/**
 * Evenly sample up to `samples` strip indices across `[0, count-1]`, inclusive
 * of both ends. Fewer frames than samples → one thumb per frame.
 */
export function sampleIndices(count: number, samples: number): number[] {
  if (count <= 0) return [];
  const n = Math.min(samples, count);
  if (n === 1) return [0];
  const out: number[] = [];
  for (let i = 0; i < n; i++) {
    out.push(Math.round((i * (count - 1)) / (n - 1)));
  }
  return out;
}

/** Array-position of the sample nearest `target` (−1 when empty). Ties take the earlier. */
export function nearestSampleIndex(samples: number[], target: number): number {
  let best = -1;
  let bestDist = Infinity;
  for (let p = 0; p < samples.length; p++) {
    const dist = Math.abs(samples[p] - target);
    if (dist < bestDist) {
      bestDist = dist;
      best = p;
    }
  }
  return best;
}

/** The initial poster index: the headline frame if it's in the strip, else the middle. */
export function initialPosterIndex(frameIds: number[], headlineFrameId: number | null): number {
  if (frameIds.length === 0) return 0;
  if (headlineFrameId != null) {
    const i = frameIds.indexOf(headlineFrameId);
    if (i >= 0) return i;
  }
  return Math.floor((frameIds.length - 1) / 2);
}

export const SPEEDS = [2, 8, 16] as const;
export type Speed = (typeof SPEEDS)[number];

/**
 * Playback speed → source-frames advanced per wall-second. Retained anchor
 * frames land at roughly capture cadence, so treating the "×" label as
 * frames-per-second gives a timelapse feel (2× slow, 8× default, 16× fast).
 * ponytail: naive constant cadence; exact fps isn't tested — tune the factor
 * here if playback feels off on a given capture rate.
 */
export function framesPerSecond(speed: Speed): number {
  return speed;
}

/**
 * Approximate the number of capture segments spanned by a set of frame
 * timestamps (ascending): a time gap beyond `gapMs` starts a new segment.
 * ponytail: heuristic — FrameSummaryDto carries no segment id; a segment is
 * capped at 5 min and frames within one are seconds apart, so 90s cleanly
 * separates them. Swap for a real count if the summary DTO grows a segment id.
 */
export function countCaptureSegments(sortedMs: number[], gapMs = 90_000): number {
  if (sortedMs.length === 0) return 0;
  let segments = 1;
  for (let i = 1; i < sortedMs.length; i++) {
    if (sortedMs[i] - sortedMs[i - 1] > gapMs) segments++;
  }
  return segments;
}
