<script lang="ts">
  // One answer's sources row (chat.html › sources): compact frame chips with a
  // thumbnail, audio chips, and a host chip for a frame with a captured page.
  // A frame peeks in place (FrameDetailModal), an audio clip plays in the
  // Activity Receipt; the raw-Timeline hand-off is the fallback.
  import { invoke } from "@tauri-apps/api/core";
  import { message } from "@tauri-apps/plugin-dialog";
  import { tip } from "$lib/components/tooltip";
  import { FramePreviewUrlMap } from "$lib/frame-preview";
  import { openCapturedUrl } from "$lib/open-captured-url";
  import { humanizeError } from "$lib/format-error";
  import { parseCapturedAt } from "$lib/format-time";
  import FrameDetailModal from "$lib/components/FrameDetailModal.svelte";
  import ActivityReceipt from "$lib/insights/ActivityReceipt.svelte";
  import type { AskAiSource } from "$lib/insights/conversation";
  import type { FrameScrubPreviewsDto } from "$lib/types/app-infra";
  import type { Activity } from "$lib/types/recording";

  let { sources }: { sources: AskAiSource[] } = $props();

  const frames = $derived(sources.filter((s) => s.kind === "frame"));
  const audio = $derived(sources.filter((s) => s.kind === "audio"));

  // Bounded blob-URL cache for the thumbnails, revoked on teardown (a blob URL
  // outlives the component that minted it).
  const urls = new FramePreviewUrlMap();
  let thumbs = $state(new Map<number, string>());
  $effect(() => () => urls.clear());
  $effect(() => {
    const ids = [...new Set(frames.map((s) => s.frameId).filter((id): id is number => id != null))].filter(
      (id) => !urls.touch(id),
    );
    if (ids.length === 0) return;
    void invoke<FrameScrubPreviewsDto>("get_frame_scrub_previews", { request: { frameIds: ids } })
      .then((res) =>
        urls.merge(
          res.previews.flatMap((e) => (e.preview ? [{ frameId: e.frameId, preview: e.preview }] : [])),
          () => (thumbs = urls.snapshot()),
        ),
      )
      .then((map) => (thumbs = map))
      .catch(() => {}); // best-effort: the chip keeps its placeholder
  });

  const when = (iso: string) => {
    const d = parseCapturedAt(iso);
    return isNaN(d.getTime()) ? iso : d.toLocaleString(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" });
  };

  let peek = $state<AskAiSource | null>(null);
  let receipt = $state<Activity | null>(null);
  let opening = $state<number | null>(null);

  async function openInTimeline(source: AskAiSource): Promise<void> {
    try {
      await invoke("open_capture_result_in_main_window", {
        kind: source.kind,
        frameId: source.frameId,
        audioSegmentId: source.audioSegmentId,
        spanStartMs: source.spanStartMs ?? null,
        alignedFrameId: source.alignedFrameId ?? null,
      });
    } catch (error) {
      await message(humanizeError(error), { title: "Couldn't open in timeline", kind: "error" });
    }
  }

  function select(source: AskAiSource): void {
    if (source.kind === "frame") {
      if (source.frameId == null) void openInTimeline(source);
      else peek = source;
      return;
    }
    const startedAtMs = Date.parse(source.startedAt);
    const endedAtMs = Date.parse(source.endedAt);
    if (source.audioSegmentId == null || !Number.isFinite(startedAtMs) || !Number.isFinite(endedAtMs) || endedAtMs <= startedAtMs) {
      void openInTimeline(source);
      return;
    }
    receipt = {
      id: source.audioSegmentId,
      title: source.sourceKind === "system" ? "System audio" : source.sourceKind === "microphone" ? "Microphone audio" : "Spoken audio",
      summary: "",
      category: null,
      startedAtMs,
      endedAtMs,
      createdAtMs: startedAtMs,
      evidence: [{ subjectType: "audio_segment", subjectId: source.audioSegmentId, capturedAtMs: startedAtMs, isHeadline: true }],
    };
  }

  async function openUrl(source: AskAiSource): Promise<void> {
    if (source.frameId == null || opening === source.frameId) return;
    opening = source.frameId;
    try {
      await openCapturedUrl(source.frameId);
    } finally {
      opening = null;
    }
  }
</script>

<div class="ch-srcrow">
  <span class="mx-label">sources</span>
  {#each frames as s, i (`f-${s.frameId}-${s.startedAt}-${i}`)}
    {@const thumb = s.frameId != null ? thumbs.get(s.frameId) : undefined}
    <button type="button" class="ch-frame" use:tip={s.windowTitle || "Peek this frame"} onclick={() => select(s)}>
      <span class="ch-frame__thumb">
        {#if thumb}<img src={thumb} alt="" />{:else}<b style="--w:80%"></b><b style="--w:55%"></b><b style="--w:70%"></b>{/if}
      </span>
      <span>{s.appName ?? "Unknown app"}</span>
      <small>{when(s.startedAt)}</small>
    </button>
    {#if s.url}
      <button type="button" class="mx-chip" disabled={opening === s.frameId} use:tip={s.url} onclick={() => void openUrl(s)}
        >{s.url.split("/")[0]} ↗</button
      >
    {/if}
  {/each}
  {#each audio as s, i (`a-${s.audioSegmentId}-${s.startedAt}-${i}`)}
    {@const mic = s.sourceKind === "microphone"}
    <button type="button" class="mx-chip mx-chip--{mic ? 'mic' : 'sysaudio'}" use:tip={"Play this clip"} onclick={() => select(s)}>
      {mic ? "mic" : "system"} · {s.appName ? `${s.appName} · ` : ""}{when(s.startedAt)}
    </button>
  {/each}
</div>

<FrameDetailModal
  open={peek !== null}
  frameId={peek?.frameId ?? null}
  appName={peek?.appName ?? null}
  windowTitle={peek?.windowTitle ?? null}
  capturedAt={peek?.startedAt ?? null}
  onClose={() => (peek = null)}
  onOpenInTimeline={peek ? () => peek && void openInTimeline(peek) : undefined}
/>
{#if receipt}
  <ActivityReceipt activity={receipt} onClose={() => (receipt = null)} />
{/if}

<style>
  .ch-srcrow { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; margin: var(--s-2) 0 0; }
  .ch-srcrow > .mx-label { margin-right: 4px; font-size: var(--text-xs); }
  .ch-srcrow .mx-chip { cursor: pointer; transition: border-color var(--t-fast); }
  .ch-srcrow .mx-chip:hover { border-color: var(--app-border-hover); }
  .ch-frame {
    display: grid;
    grid-template-columns: 44px auto;
    gap: 3px 8px;
    align-items: center;
    padding: 4px 9px 4px 4px;
    border-radius: var(--r-md);
    border: 1px solid var(--app-border);
    background: var(--app-surface);
    color: var(--app-text);
    cursor: pointer;
    text-align: left;
    transition: border-color var(--t-fast) var(--ease-quart), transform var(--t-med) var(--ease-expo);
  }
  .ch-frame:hover { border-color: var(--app-border-hover); transform: translateY(-1px); }
  .ch-frame:focus-visible { outline: none; box-shadow: var(--app-ring); }
  .ch-frame__thumb {
    grid-row: span 2;
    height: 28px;
    overflow: hidden;
    border-radius: 4px;
    padding: 4px;
    display: grid;
    align-content: start;
    gap: 3px;
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
  }
  .ch-frame__thumb:has(img) { padding: 0; }
  .ch-frame__thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .ch-frame__thumb b { display: block; height: 2px; border-radius: 1px; background: color-mix(in srgb, var(--app-fg) 18%, transparent); width: var(--w); }
  .ch-frame > span:not(.ch-frame__thumb) { font: 500 var(--text-xs)/1 var(--font-sans); color: var(--app-text-strong); }
  .ch-frame small { font: 400 9.5px/1 var(--font-mono); color: var(--app-text-subtle); }
</style>
