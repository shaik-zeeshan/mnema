// The drawer's audio element and everything that drives it: play/pause, the two
// seek paths, and bounded-sample playback. Split out of `AudioDrawer.svelte`
// because it is the one part of the drawer that is pure transport — it knows
// about an HTMLAudioElement and nothing about speakers, turns, or the reader.
//
// Every method that writes `element.currentTime` or starts playback ends sample
// mode first; that invariant is the reason this lives in one place.

/** "Play 8s of each": how long each bounded sample runs before the next. */
export const SAMPLE_SECONDS = 8;

export class AudioTransport {
  /** Bound with `bind:this` in the template. */
  element = $state<HTMLAudioElement | null>(null);
  isPlaying = $state(false);
  currentTime = $state(0);
  duration = $state(0);
  /** True while the user drags the scrubber — `timeupdate` must not fight them. */
  scrubbing = $state(false);
  hasSeeked = $state(false);

  /** Bounded-sample playback: stop at this time, then move to the next sample. */
  #stopAt: number | null = null;
  #queue: number[] = [];
  #onMediaError: () => void;

  constructor(onMediaError: () => void) {
    this.#onMediaError = onMediaError;
  }

  get currentMs(): number {
    return Math.round(this.currentTime * 1000);
  }

  /** Whether the user has actually engaged with playback, as opposed to just
   *  having the drawer open at 0:00 — what decides if a paragraph reads as
   *  "current" or the reader stays unhighlighted. */
  get engaged(): boolean {
    return this.isPlaying || this.currentTime > 0 || this.hasSeeked;
  }

  /** New segment: the element is remounted, so every derived reading resets. */
  reset = (): void => {
    this.isPlaying = false;
    this.currentTime = 0;
    this.duration = 0;
    this.scrubbing = false;
    this.hasSeeked = false;
    this.#stopAt = null;
    this.#queue = [];
  };

  /** Any explicit user transport action ends bounded-sample mode: from here on
   *  playback is theirs, and a live stop point would otherwise pause them or yank
   *  them back to the merge candidate on the next `timeupdate`. EVERY path that
   *  writes `element.currentTime` or starts playback must call this. */
  endSamplePreview = (): void => {
    this.#stopAt = null;
    this.#queue = [];
  };

  /** `play()` rejects with AbortError when the request is INTERRUPTED rather than
   *  failed: a `pause()` landing before playback starts (a fast second click, a
   *  repeated play/pause shortcut), or the element being torn down because the user
   *  switched segments mid-playback (`audioSrc` goes null and `{#key segmentId}`
   *  remounts, and removing a media element from the document runs the internal
   *  pause steps). The media-error path raises the page's STICKY "could not decode
   *  this segment" banner, cleared only by a segment change — so routing an
   *  interruption there paints a permanent false error over healthy audio, on the
   *  segment the user just opened. */
  #onPlayRejected = (err: unknown): void => {
    if ((err as { name?: string } | null)?.name === "AbortError") return;
    this.#onMediaError();
  };

  togglePlayPause = (): void => {
    const el = this.element;
    if (!el) return;
    this.endSamplePreview();
    if (el.paused) void el.play().catch(this.#onPlayRejected);
    else el.pause();
  };

  onTimeUpdate = (): void => {
    const el = this.element;
    if (!el) return;
    if (this.#stopAt != null && el.currentTime >= this.#stopAt) {
      const next = this.#queue.shift();
      if (next == null) {
        this.#stopAt = null;
        el.pause();
      } else {
        this.#stopAt = next / 1000 + SAMPLE_SECONDS;
        el.currentTime = next / 1000;
      }
    }
    if (this.scrubbing) return;
    this.currentTime = el.currentTime;
  };

  onLoadedMetadata = (): void => {
    const el = this.element;
    if (!el) return;
    this.duration = Number.isFinite(el.duration) ? el.duration : 0;
  };

  onEnded = (): void => {
    this.isPlaying = false;
    this.currentTime = this.element?.duration ?? this.currentTime;
  };

  seekToMs = (startMs: number): void => {
    const el = this.element;
    if (!el) return;
    this.endSamplePreview();
    const cap = Number.isFinite(this.duration) && this.duration > 0 ? this.duration : Infinity;
    const next = Math.max(0, Math.min(cap, startMs / 1000));
    if (!Number.isFinite(next)) return;
    el.currentTime = next;
    this.currentTime = next;
    this.hasSeeked = true;
  };

  seekBySeconds = (delta: number): void => {
    const el = this.element;
    if (!el) return;
    this.endSamplePreview();
    const cap =
      Number.isFinite(this.duration) && this.duration > 0
        ? this.duration
        : Number.isFinite(el.duration) && el.duration > 0
          ? el.duration
          : Infinity;
    const next = Math.max(0, Math.min(cap, el.currentTime + delta));
    if (!Number.isFinite(next)) return;
    el.currentTime = next;
    this.currentTime = next;
    this.hasSeeked = true;
  };

  /** The scrubber's commit: seconds straight off the range input. */
  seekToSeconds = (seconds: number): void => {
    const el = this.element;
    if (!el || !Number.isFinite(seconds)) return;
    this.endSamplePreview();
    el.currentTime = seconds;
    this.currentTime = seconds;
    this.hasSeeked = true;
  };

  /** Play a bounded sample of each head in turn — this cluster's first turn, then
   *  the merge candidate's. Bounded in `onTimeUpdate`, so no second element. */
  playSamples = (headsMs: number[]): void => {
    const el = this.element;
    if (!el || headsMs.length === 0) return;
    this.#queue = headsMs.slice(1);
    el.currentTime = headsMs[0] / 1000;
    this.currentTime = headsMs[0] / 1000;
    this.#stopAt = headsMs[0] / 1000 + SAMPLE_SECONDS;
    void el.play().catch(this.#onPlayRejected);
  };
}
