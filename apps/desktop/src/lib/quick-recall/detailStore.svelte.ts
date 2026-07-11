// Quick Recall detail-pane store — owns the lazy, per-result-cached fetches
// behind the detail pane (Slice 5). Fetches fire on selection change only:
//   frame → hero preview (`get_frame_scrub_previews`, 1280px — the row
//           thumbnails keep their separate 200px batch in searchStore) +
//           flat OCR text (`list_processing_results` for the representative
//           frame; search results carry NO OCR text and `FrameDto.ocrText`
//           is never emitted by Rust).
//   audio → transcript turns (`list_speaker_turns` for the segment).
// Everything degrades gracefully: any failed/missing piece resolves to
// null/empty and the pane still renders identity/badges/times.
import { invoke } from "@tauri-apps/api/core";
import { framePreviewAssetUrl } from "$lib/frame-preview";
import type {
  AudioSearchResultDto,
  FrameScrubPreviewsDto,
  FrameSearchResultDto,
  ProcessingResultDto,
  SpeakerTurnDto,
} from "$lib/types/app-infra";
import type { SelectedSearchResult } from "./searchStore.svelte";
import { detailCacheKey } from "./detail-view";

const HERO_MAX_PIXEL_SIZE = 1280;
// ponytail: FIFO cap, not LRU — 80 entries outlive any realistic session.
const CACHE_CAP = 80;

export type FrameDetailData = {
  kind: "frame";
  // Asset URL of the 1280px hero preview; null (preview missing for any
  // `missingReason`, or the fetch failed) renders the SVG glyph fallback.
  heroUrl: string | null;
  // Flat OCR `result_text` for the representative frame; null when no OCR
  // result exists (or the fetch failed) — the pane shows a quiet note.
  ocrText: string | null;
};

export type AudioDetailData = {
  kind: "audio";
  // All speaker turns for the segment; empty when none exist / fetch failed.
  turns: SpeakerTurnDto[];
};

export type DetailData = FrameDetailData | AudioDetailData;

export class DetailStore {
  // Key of the selection the pane is showing (set synchronously on load, so
  // in-flight responses for a superseded selection are discarded).
  key = $state<string | null>(null);
  // Fetched data for `key`; null while the fetch is in flight.
  data = $state<DetailData | null>(null);

  // Render cache, keyed by detailCacheKey. Deliberately a NON-reactive Map:
  // it is written from async loads only, never from the template.
  #cache = new Map<string, DetailData>();

  clear(): void {
    this.key = null;
    this.data = null;
  }

  async load(selected: SelectedSearchResult): Promise<void> {
    const key = detailCacheKey(selected);
    if (key === this.key) {
      return; // same selection (or its fetch is already in flight)
    }
    this.key = key;

    const hit = this.#cache.get(key);
    if (hit !== undefined) {
      this.data = hit;
      return;
    }

    this.data = null;
    const data =
      selected.kind === "frame"
        ? await loadFrameDetail(selected.frame)
        : await loadAudioDetail(selected.audio);

    // A full miss (every piece failed/absent) is NOT cached, so a transient
    // backend hiccup retries on the next visit instead of sticking all session.
    if (hasContent(data)) {
      this.#cache.set(key, data);
      if (this.#cache.size > CACHE_CAP) {
        const oldest = this.#cache.keys().next().value;
        if (oldest !== undefined) this.#cache.delete(oldest);
      }
    }

    if (this.key !== key) {
      return; // selection moved on while we fetched
    }
    this.data = data;
  }
}

function hasContent(data: DetailData): boolean {
  return data.kind === "frame"
    ? data.heroUrl !== null || data.ocrText !== null
    : data.turns.length > 0;
}

async function loadFrameDetail(
  frame: FrameSearchResultDto,
): Promise<FrameDetailData> {
  const [heroUrl, ocrText] = await Promise.all([
    fetchHeroUrl(frame.thumbnailFrameId),
    fetchOcrText(frame.representativeFrame.id),
  ]);
  return { kind: "frame", heroUrl, ocrText };
}

async function fetchHeroUrl(frameId: number): Promise<string | null> {
  try {
    const response = await invoke<FrameScrubPreviewsDto>(
      "get_frame_scrub_previews",
      { request: { frameIds: [frameId], maxPixelSize: HERO_MAX_PIXEL_SIZE } },
    );
    const entry =
      response.previews.find((preview) => preview.frameId === frameId) ??
      response.previews[0];
    // A `missingReason` entry has `preview: null` → glyph fallback via null.
    return entry?.preview
      ? framePreviewAssetUrl(entry.preview.filePath)
      : null;
  } catch {
    return null;
  }
}

async function fetchOcrText(frameId: number): Promise<string | null> {
  try {
    const results = await invoke<ProcessingResultDto[]>(
      "list_processing_results",
      { request: { subjectType: "frame", subjectId: frameId } },
    );
    const ocr = results
      .filter(
        (result) => result.processor === "ocr" && result.resultText !== null,
      )
      .sort((a, b) => b.id - a.id)[0];
    const text = ocr?.resultText?.trim();
    return text ? text : null;
  } catch {
    return null;
  }
}

async function loadAudioDetail(
  audio: AudioSearchResultDto,
): Promise<AudioDetailData> {
  try {
    const turns = await invoke<SpeakerTurnDto[]>("list_speaker_turns", {
      request: { audioSegmentId: audio.audioSegment.id },
    });
    return { kind: "audio", turns };
  } catch {
    return { kind: "audio", turns: [] };
  }
}

export const quickRecallDetail = new DetailStore();
