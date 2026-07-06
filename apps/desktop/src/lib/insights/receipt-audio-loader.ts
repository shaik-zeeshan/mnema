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
    const profiles = await this.#invoke<PersonProfileDto[]>("list_person_profiles").catch(
      () => [] as PersonProfileDto[],
    );
    if (gen !== this.#gen) return;
    this.#events.onProfiles(profiles);
    const segments = await this.#invoke<AudioSegmentDto[]>("list_audio_segments", {
      request: {
        capturedAtStart: new Date(startMs).toISOString(),
        capturedAtEnd: new Date(endMs).toISOString(),
      },
    }).catch(() => [] as AudioSegmentDto[]);
    const hydrated = await Promise.all(
      segments.map(async (segment) => {
        const turns = await this.#invoke<SpeakerTurnDto[]>("list_speaker_turns", {
          request: { audioSegmentId: segment.id },
        }).catch(() => [] as SpeakerTurnDto[]);
        return { segment, turns };
      }),
    );
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
