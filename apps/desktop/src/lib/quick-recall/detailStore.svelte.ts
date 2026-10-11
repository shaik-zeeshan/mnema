// Quick Look store — the preview that opens under the bar on Space, a row's
// eye, the best moment's Quick Look button, or (QR5) an answer citation.
// Owns what is open, the scrub position, the lazily fetched data behind it,
// and the hand-off. QuickLook.svelte renders it; search-keys.ts drives it.
//   screen → `list_frame_summaries_in_range` (match ± 90 s, nearest frame per
//            15 s step, `quick-look.ts`), `get_frame_scrub_previews` for the
//            strip, `list_processing_results` for each frame's text, and the
//            hit's `matchBoxes` / `get_frame_match_boxes` for the outlines.
//   audio  → `list_speaker_turns`; the strip is the transcript turns.
// The stage's full-size preview is QuickLook.svelte's (one blob URL alive).
import { invoke } from "@tauri-apps/api/core";
import { FramePreviewUrlMap } from "$lib/frame-preview";
import { openCapturedUrl } from "$lib/open-captured-url";
import { closeCurrentWindow } from "$lib/surface-windows";
import { parseCapturedAt } from "$lib/format-time";
import { parseSearchSnippet } from "$lib/search-snippet";
import type {
  FrameScrubPreviewsDto,
  FrameSummaryDto,
  MatchBox,
  ProcessingResultDto,
  ScrubPreviewMissingReason,
  SpeakerTurnDto,
} from "$lib/types/app-infra";
import { quickRecallSearch as search } from "./searchStore.svelte";
import type { RankedResult } from "./result-sections";
import { matchTurnIndex } from "./detail-view";
import { pickStrip, RANGE, STEP_MS, type StripFrame } from "./quick-look";

export type LookTurn = Pick<SpeakerTurnDto, "startMs" | "endMs" | "speakerLabel"> & {
  text: string;
};

const keyOf = (r: RankedResult) => (r.kind === "frame" ? r.frame.groupKey : r.audio.groupKey);
const plainSnippet = (snippet: string) =>
  parseSearchSnippet(snippet)
    .map((s) => s.text)
    .join("");

export class QuickLookStore {
  // What is open; null = closed (the list shows).
  result = $state<RankedResult | null>(null);
  // Screen: step offset from the match (−6…6, ×15 s). Audio: turn index.
  k = $state(0);
  // Arrowed into the list (↓/↑): Space looks instead of typing a space.
  nav = $state(false);

  // Screen data. `strip[k + RANGE]`; `ocr`/`boxes` keyed by frame id
  // (undefined = still loading); `gone` = frames whose pictures are missing.
  strip = $state<(StripFrame | null)[]>([]);
  thumbs = $state<Record<number, string>>({});
  gone = $state<Record<number, ScrubPreviewMissingReason>>({});
  ocr = $state<Record<number, string | null>>({});
  boxes = $state<Record<number, MatchBox[]>>({});
  // Audio data: the spoken turns (null while loading).
  turns = $state<LookTurn[] | null>(null);
  copied = $state(false);

  #onClose: (() => void) | undefined;
  #gen = 0;
  #urls: FramePreviewUrlMap | null = null;
  #copiedTimer: ReturnType<typeof setTimeout> | undefined;

  isScreen = $derived(this.result?.kind === "frame");
  // The frame on the stage: ids for text/boxes/hand-off; `previewId` for pixels
  // (the hit's thumbnail frame at the match, as the best moment paints).
  frame = $derived(this.strip[this.k + RANGE] ?? null);
  previewId = $derived(this.frame ? this.pixelId(this.frame) : null);
  turn = $derived(this.turns?.[this.k] ?? null);
  // Where the selection sits in the visible list (↑/↓, "N of M").
  index = $derived(
    this.result ? search.visibleResults.findIndex((r) => keyOf(r) === keyOf(this.result!)) : -1,
  );

  // The frame whose pixels to paint for a strip frame.
  pixelId(f: StripFrame): number {
    const hit = this.result?.kind === "frame" ? this.result.frame : null;
    return hit && f.id === hit.representativeFrame.id ? hit.thumbnailFrameId : f.id;
  }

  // Open on `result`. `onClose` runs when it closes (QR5: restore the citation).
  show(result: RankedResult, onClose?: () => void): void {
    this.#onClose = onClose;
    this.#load(result);
  }

  close(): void {
    if (!this.result) return;
    this.result = null;
    this.#gen += 1;
    this.#urls?.clear();
    this.#urls = null;
    const done = this.#onClose;
    this.#onClose = undefined;
    done?.();
  }

  // ←/→: ±15 s (screen) or the next transcript line (audio).
  scrub(delta: number): void {
    const [lo, hi] = this.isScreen ? [-RANGE, RANGE] : [0, (this.turns?.length ?? 1) - 1];
    this.goTo(Math.max(lo, Math.min(hi, this.k + delta)));
  }

  goTo(k: number): void {
    this.k = k;
    if (this.isScreen && this.frame) void this.#loadFrame(this.frame.id, this.#gen);
  }

  // ↑/↓: the neighbouring result in the list, selected as you go.
  step(delta: number): void {
    const j = this.index + delta;
    const next = search.visibleResults[j];
    if (this.index < 0 || !next) return;
    search.selectResultAt(j);
    this.#load(next);
  }

  // ↵: the scrubbed frame (audio: the scrubbed turn's start) in Timeline.
  async openHere(): Promise<void> {
    const r = this.result;
    if (!r) return;
    if (r.kind === "frame") {
      await this.#handOff({ kind: "frame", frameId: this.frame?.id ?? r.frame.representativeFrame.id });
    } else {
      await this.#handOff({
        kind: "audio",
        audioSegmentId: r.audio.audioSegment.id,
        spanStartMs: this.turn?.startMs ?? r.audio.spanStartMs,
        alignedFrameId: r.audio.alignedFrame?.id ?? null,
      });
    }
  }

  // "On screen then": the frame aligned with an audio hit.
  async openAligned(): Promise<void> {
    const id = this.result?.kind === "audio" ? this.result.audio.alignedFrame?.id : undefined;
    if (id !== undefined) await this.#handOff({ kind: "frame", frameId: id });
  }

  // ⌘↵: the captured page behind the frame on the stage.
  openPage(): void {
    if (this.isScreen && this.previewId !== null) void search.openCapturedFrameUrl(this.previewId);
  }

  // ⌘C: the stage frame's text, or the scrubbed line.
  async copy(): Promise<void> {
    const text = this.isScreen
      ? this.frame && this.ocr[this.frame.id]
      : this.turn && this.turn.text;
    if (text) await this.#write(text);
  }

  // ⌘C in the list: the result's OCR text or its matched line.
  async copyResult(r: RankedResult): Promise<void> {
    if (r.kind === "frame") {
      const text = await fetchOcrText(r.frame.representativeFrame.id);
      await this.#write(text ?? plainSnippet(r.frame.snippet));
    } else {
      const turns = await fetchTurns(r.audio.audioSegment.id);
      const i = matchTurnIndex(turns, r.audio.spanStartMs);
      await this.#write(turns[i]?.text ?? plainSnippet(r.audio.snippet));
    }
  }

  async #write(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      this.copied = true;
      clearTimeout(this.#copiedTimer);
      this.#copiedTimer = setTimeout(() => (this.copied = false), 1400);
    } catch {
      // A rejected clipboard write leaves the label as it was.
    }
  }

  async #handOff(target: {
    kind: "frame" | "audio";
    frameId?: number;
    audioSegmentId?: number;
    spanStartMs?: number;
    alignedFrameId?: number | null;
  }): Promise<void> {
    try {
      await invoke("open_capture_result_in_main_window", {
        frameId: null,
        audioSegmentId: null,
        ...target,
        query: search.resultsQuery,
      });
    } catch (err) {
      await search.surfaceResultHandoffFailure(err);
      return;
    }
    await closeCurrentWindow();
  }

  #load(result: RankedResult): void {
    const gen = ++this.#gen;
    this.#urls?.clear();
    this.#urls = new FramePreviewUrlMap();
    search.filters.selectPill(null);
    this.nav = true;
    this.result = result;
    this.k = 0;
    this.strip = [];
    this.thumbs = {};
    this.gone = {};
    this.ocr = {};
    this.boxes = {};
    this.turns = null;
    if (result.kind === "frame") void this.#loadScreen(result.frame, gen);
    else void this.#loadAudio(result.audio, gen);
  }

  async #loadScreen(
    hit: Extract<RankedResult, { kind: "frame" }>["frame"],
    gen: number,
  ): Promise<void> {
    const repId = hit.representativeFrame.id;
    const ms = parseCapturedAt(hit.representativeFrame.capturedAt).getTime();
    this.boxes[repId] = hit.matchBoxes;
    this.strip = pickStrip([], { id: repId, ms });
    void this.#loadFrame(repId, gen);
    let frames: StripFrame[] = [];
    try {
      const summaries = await invoke<FrameSummaryDto[]>("list_frame_summaries_in_range", {
        request: {
          capturedAtStart: new Date(ms - RANGE * STEP_MS).toISOString(),
          capturedAtEnd: new Date(ms + RANGE * STEP_MS).toISOString(),
        },
      });
      frames = summaries
        .map((s) => ({ id: s.id, ms: parseCapturedAt(s.capturedAt).getTime() }))
        .filter((f) => Number.isFinite(f.ms));
    } catch {
      // No neighbours: the strip shows the match alone.
    }
    if (gen !== this.#gen) return;
    this.strip = pickStrip(frames, { id: repId, ms });
    const ids = new Set(this.strip.flatMap((f) => (f ? [this.pixelId(f)] : [])));
    await this.#loadThumbs([...ids], gen);
  }

  async #loadAudio(
    hit: Extract<RankedResult, { kind: "audio" }>["audio"],
    gen: number,
  ): Promise<void> {
    if (hit.alignedFrame) void this.#loadThumbs([hit.alignedFrame.id], gen);
    let turns = await fetchTurns(hit.audioSegment.id);
    if (gen !== this.#gen) return;
    // No diarized turns yet: the matched span alone, quoted from the snippet.
    if (turns.length === 0) {
      turns = [{ startMs: hit.spanStartMs, endMs: hit.spanEndMs, speakerLabel: "", text: plainSnippet(hit.snippet) }];
    }
    this.turns = turns;
    this.k = Math.max(0, matchTurnIndex(turns, hit.spanStartMs));
  }

  async #loadThumbs(ids: number[], gen: number): Promise<void> {
    const urls = this.#urls;
    try {
      const response = await invoke<FrameScrubPreviewsDto>("get_frame_scrub_previews", {
        request: { frameIds: ids },
      });
      if (gen !== this.#gen || !urls) return;
      for (const p of response.previews) if (p.missingReason) this.gone[p.frameId] = p.missingReason;
      await urls.merge(
        response.previews.flatMap((p) => (p.preview ? [{ frameId: p.frameId, preview: p.preview }] : [])),
        (frameId, url) => {
          if (gen === this.#gen) this.thumbs[frameId] = url;
        },
      );
    } catch {
      // Best-effort: the cells keep their empty frame.
    }
  }

  async #loadFrame(id: number, gen: number): Promise<void> {
    const hit = this.result?.kind === "frame" ? this.result.frame : null;
    if (!hit) return;
    const wantBoxes = this.boxes[id] === undefined;
    const [text, boxes] = await Promise.all([
      this.ocr[id] === undefined ? fetchOcrText(id) : this.ocr[id],
      wantBoxes
        ? invoke<MatchBox[]>("get_frame_match_boxes", {
            request: { frameId: id, query: search.resultsQuery, byMeaning: hit.foundByMeaning },
          }).catch(() => [])
        : this.boxes[id],
    ]);
    if (gen !== this.#gen) return;
    this.ocr[id] = text;
    this.boxes[id] = boxes;
  }
}

async function fetchOcrText(frameId: number): Promise<string | null> {
  try {
    const results = await invoke<ProcessingResultDto[]>("list_processing_results", {
      request: { subjectType: "frame", subjectId: frameId },
    });
    const ocr = results
      .filter((r) => r.processor === "ocr" && r.resultText !== null)
      .sort((a, b) => b.id - a.id)[0];
    return ocr?.resultText?.trim() || null;
  } catch {
    return null;
  }
}

async function fetchTurns(audioSegmentId: number): Promise<LookTurn[]> {
  try {
    const turns = await invoke<SpeakerTurnDto[]>("list_speaker_turns", {
      request: { audioSegmentId },
    });
    return turns
      .filter((t) => (t.transcriptText ?? "").trim())
      .map((t) => ({ startMs: t.startMs, endMs: t.endMs, speakerLabel: t.speakerLabel, text: t.transcriptText!.trim() }));
  } catch {
    return [];
  }
}

export const quickLook = new QuickLookStore();

// The key row while Quick Look is open (null = closed). Field's footer renders it.
export function lookKeys(): [string, string][] | null {
  const r = quickLook.result;
  if (!r) return null;
  // A citation the list doesn't hold has no neighbours to step to.
  const steps: [string, string][] = quickLook.index >= 0 ? [["↑↓", "results"]] : [];
  if (r.kind === "audio") {
    return [["←→", "lines"], ...steps, ["↵", "open at this line"], ["⌘C", "copy quote"], ["space", "back"]];
  }
  const gone = quickLook.previewId !== null && quickLook.gone[quickLook.previewId] !== undefined;
  return [
    ["←→", "±15 s"],
    ...steps,
    ["↵", gone ? "open in Timeline" : "open this frame"],
    ...(r.frame.url ? [["⌘↵", "page"] as [string, string]] : []),
    ["⌘C", "copy text"],
    ["space", "back"],
  ];
}
