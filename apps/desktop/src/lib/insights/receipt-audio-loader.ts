// receipt-audio-loader — the invoke-touching hydration behind the Activity
// Receipt's audio evidence (ADR 0049), split out (mirroring receipt-frames.ts)
// to keep ActivityReceipt.svelte under the 800-line ceiling: resolves the
// shared person profiles once, hydrates every audio segment in the span into an
// ordered `TurnView[]`, and fetches a clip's playable media on demand.
// Generation-guarded — a new activity drops any stale hydration.

import { invoke } from "@tauri-apps/api/core";
import { audioDataUrl, buildTurnViews, type TurnView } from "$lib/insights/receipt-audio";
import type {
  AudioSegmentDto,
  AudioSegmentMediaDto,
  PersonProfileDto,
  SpeakerTurnDto,
} from "$lib/types/app-infra";

export interface ReceiptAudioEvents {
  /** Shared people directory — resolved once, drives live name attribution. */
  onProfiles(profiles: PersonProfileDto[]): void;
  /** Span-wide diarized turn view models (the receipt's one audio surface). */
  onTurns?(turns: TurnView[]): void;
}

/** `invoke`-shaped IPC entry point, injectable so tests can stub Tauri. */
export type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

// Cap the per-segment turn fan-out. A multi-hour Activity spans dozens of 5-min
// audio segments (mic+system doubles it), and an unbounded `Promise.all` would
// open one `list_speaker_turns` IPC per segment at once against the 4-connection
// owner-reader pool that live capture/OCR also read through. The DB already
// serializes to 4 connections, so extra parallelism buys nothing past that — a
// small cap keeps the modal-open burst bounded without adding latency.
// ponytail: still N round-trips; a batched `list_speaker_turns_in_range`
// (WHERE audio_segment_id IN (...)) backend command would collapse it to one
// query — do that if span-hydration latency ever bites.
const TURN_HYDRATION_CONCURRENCY = 6;

/** Order- and cardinality-preserving bounded-concurrency map. */
async function mapBounded<T, R>(
  items: T[],
  limit: number,
  fn: (item: T) => Promise<R>,
): Promise<R[]> {
  const results = new Array<R>(items.length);
  let next = 0;
  async function worker(): Promise<void> {
    for (let i = next++; i < items.length; i = next++) {
      results[i] = await fn(items[i]);
    }
  }
  await Promise.all(
    Array.from({ length: Math.min(limit, items.length) }, () => worker()),
  );
  return results;
}

/** The cited-audio refs the receipt hands in (subset of ActivityEvidenceRef). */
export interface AudioRef {
  subjectId: number;
  capturedAtMs?: number | null;
  isHeadline: boolean;
}

export class ReceiptAudioLoader {
  #events: ReceiptAudioEvents;
  #invoke: InvokeFn;
  #gen = 0; // bumped per load / reset; stale async work drops

  constructor(events: ReceiptAudioEvents, invokeFn: InvokeFn = invoke) {
    this.#events = events;
    this.#invoke = invokeFn;
  }

  /** New activity: invalidate any in-flight hydration + media fetches. */
  reset(): void {
    this.#gen++;
  }

  /**
   * Span-wide hydration: every audio segment in `[startMs, endMs]` → all its
   * speaker turns → an ordered `TurnView[]`. Generation-guarded like `load()`.
   * Reports the shared profiles (for live name resolution) then the turns.
   */
  async loadSpan(
    startMs: number,
    endMs: number,
    citedRefs: { subjectId: number; isHeadline: boolean }[],
  ): Promise<void> {
    const gen = ++this.#gen;
    // Build the RFC3339 range up front so a NaN / out-of-range span (a corrupt
    // activity timestamp) degrades to the honest empty state instead of throwing
    // an unhandled RangeError — `new Date(x).toISOString()` throws synchronously
    // and would escape the invoke `.catch`. Mirrors loadStrip, whose identical
    // Date calls sit inside its try. The `.catch(() => [])` invoke fallbacks below
    // are the "render empty on failure" contract; keep the throw on the same path.
    let range: { capturedAtStart: string; capturedAtEnd: string };
    try {
      range = { capturedAtStart: new Date(startMs).toISOString(), capturedAtEnd: new Date(endMs).toISOString() };
    } catch {
      this.#events.onTurns?.([]);
      return;
    }
    const profiles = await this.#invoke<PersonProfileDto[]>("list_person_profiles").catch(
      () => [] as PersonProfileDto[],
    );
    if (gen !== this.#gen) return;
    this.#events.onProfiles(profiles);
    const segments = await this.#invoke<AudioSegmentDto[]>("list_audio_segments", {
      request: range,
    }).catch(() => [] as AudioSegmentDto[]);
    // Shed the per-segment turn fan-out for a superseded generation: without this
    // a stale run still opens up to TURN_HYDRATION_CONCURRENCY list_speaker_turns
    // IPC against the shared 4-connection reader pool before discarding them.
    if (gen !== this.#gen) return;
    const hydrated = await mapBounded(segments, TURN_HYDRATION_CONCURRENCY, async (segment) => {
      const turns = await this.#invoke<SpeakerTurnDto[]>("list_speaker_turns", {
        request: { audioSegmentId: segment.id },
      }).catch(() => [] as SpeakerTurnDto[]);
      return { segment, turns };
    });
    const turns = buildTurnViews(hydrated, citedRefs, profiles);
    if (gen !== this.#gen) return;
    this.#events.onTurns?.(turns);
  }

  /** Fetch a segment's playable media as a data: URL. Null on failure. */
  async fetchMediaSrc(audioSegmentId: number): Promise<string | null> {
    try {
      const media = await this.#invoke<AudioSegmentMediaDto>("get_audio_segment_media", {
        request: { audioSegmentId },
      });
      return audioDataUrl(media);
    } catch {
      return null;
    }
  }
}
