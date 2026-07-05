// receipt-audio-loader — the invoke-touching hydration behind the Activity
// Receipt's audio evidence (ADR 0049), split out (mirroring receipt-frames.ts)
// to keep ActivityReceipt.svelte under the 800-line ceiling: resolves the
// shared person profiles once, hydrates each cited audio segment (source kind
// + speaker turns → an AudioCitation), and fetches a clip's playable media on
// demand. Generation-guarded — a new activity drops any stale hydration.

import { invoke } from "@tauri-apps/api/core";
import { audioDataUrl, buildCitation, type AudioCitation } from "$lib/insights/receipt-audio";
import type {
  AudioSegmentDto,
  AudioSegmentMediaDto,
  PersonProfileDto,
  SpeakerTurnDto,
} from "$lib/types/app-infra";

export interface ReceiptAudioEvents {
  /** Shared people directory — resolved once, drives live name attribution. */
  onProfiles(profiles: PersonProfileDto[]): void;
  /** The cited audio segments, hydrated and sorted by start (ascending). */
  onCitations(citations: AudioCitation[]): void;
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
   * Hydrate shared profiles + each cited segment's source kind and turns.
   * ponytail: fetches every cited segment then reports once (not incrementally)
   * — cited audio is bounded to a handful per Activity; add streaming reporting
   * only if an Activity ever cites enough segments to make the wait visible.
   */
  async load(refs: AudioRef[]): Promise<void> {
    const gen = ++this.#gen;
    if (refs.length === 0) {
      this.#events.onCitations([]);
      return;
    }
    const profiles = await this.#invoke<PersonProfileDto[]>("list_person_profiles").catch(
      () => [] as PersonProfileDto[],
    );
    if (gen !== this.#gen) return;
    this.#events.onProfiles(profiles);
    const citations = await Promise.all(
      refs.map(async (ref) => {
        const [segment, turns] = await Promise.all([
          this.#invoke<AudioSegmentDto | null>("get_audio_segment", {
            request: { audioSegmentId: ref.subjectId },
          }).catch(() => null),
          this.#invoke<SpeakerTurnDto[]>("list_speaker_turns", {
            request: { audioSegmentId: ref.subjectId },
          }).catch(() => [] as SpeakerTurnDto[]),
        ]);
        return buildCitation(ref, segment, turns);
      }),
    );
    if (gen !== this.#gen) return;
    // Ascending by segment start so ticks, ordinals, and the roster all align.
    citations.sort((a, b) => (a.capturedAtMs ?? 0) - (b.capturedAtMs ?? 0));
    this.#events.onCitations(citations);
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
