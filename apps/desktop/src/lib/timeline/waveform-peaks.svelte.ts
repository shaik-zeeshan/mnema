// Real amplitude peaks for the drawer's scrubber, computed in Rust from the
// segment's own audio (Mnema stores no waveform data, and a fake waveform is
// worse than none because users navigate by its shape).
//
// ponytail: one fetch per segment, no cache, no retry. The command returns an
// EMPTY array on any failure, and empty means the plain scrub bar with no bars
// and no error message — playback surfaces the real problem. Cache the floats on
// the audio_segments row if the decode ever feels slow.

import { invoke } from "@tauri-apps/api/core";
import type { GetAudioSegmentWaveformPeaksRequest } from "$lib/types/app-infra";

export const WAVEFORM_BUCKETS = 150;

/** Call from a component's init; `segmentId` is read reactively. */
export function waveformPeaks(
  segmentId: () => number,
  bucketCount: number = WAVEFORM_BUCKETS,
): { readonly value: number[] } {
  let peaks = $state<number[]>([]);
  // Depend on the ID VALUE, not on whatever object the caller read it off. The
  // timeline replaces its audio-segment rows wholesale on every
  // `refreshAudioSegments()` (~1.5s head poll while capturing), so a closure that
  // reads `row.id` re-fires on identity alone — and each re-fire is a full
  // AVFoundation decode of a segment up to the 5-minute cap on the blocking pool.
  const id = $derived.by(segmentId);

  $effect(() => {
    const audioSegmentId = id;
    peaks = [];
    let cancelled = false;
    void invoke<number[]>("get_audio_segment_waveform_peaks", {
      request: { audioSegmentId, bucketCount } satisfies GetAudioSegmentWaveformPeaksRequest,
    })
      .then((result) => {
        if (!cancelled && Array.isArray(result)) peaks = result;
      })
      .catch(() => {
        /* empty peaks: the plain scrubber is the fallback */
      });
    return () => {
      cancelled = true;
    };
  });

  return {
    get value() {
      return peaks;
    },
  };
}
