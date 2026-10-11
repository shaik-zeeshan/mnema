<!-- The best moment (mockup `recall.html` `.rc-hero`): the top of the ranked
     list, either kind. A frame shows a crop zoomed onto its first match box at
     the preview's native pixels (outlines: solid = keyword, dashed = meaning;
     no boxes = the whole frame); audio shows the matched stretch of the
     waveform. Then title, time range, snippet, the keyword / by-meaning chip,
     and Open in Timeline (the one primary while selected), Quick Look, Open
     page. -->
<script lang="ts">
  import IconTimeline from "~icons/lucide/chart-no-axes-gantt";
  import IconEye from "~icons/lucide/eye";
  import IconExternal from "~icons/lucide/external-link";
  import IconMic from "~icons/lucide/mic";
  import IconSpeaker from "~icons/lucide/volume-2";
  import { formatRelativeTime, parseCapturedAt } from "$lib/format-time";
  import { parseSearchSnippet } from "$lib/search-snippet";
  import { quickRecallSearch as search } from "./searchStore.svelte";
  import { resultTile, markedSnippet } from "./SearchResultCard.svelte";
  import type { RankedResult } from "./result-sections";

  let {
    result,
    thumbnailUrl = null,
    selected = false,
    id = undefined,
    onselect,
    onlook,
  }: {
    result: RankedResult;
    thumbnailUrl?: string | null;
    selected?: boolean;
    id?: string;
    onselect: () => void;
    onlook?: () => void;
  } = $props();

  const frame = $derived(result.kind === "frame" ? result.frame : null);
  const audio = $derived(result.kind === "audio" ? result.audio : null);

  const hm = (d: Date) => d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  // m:ss, or h:mm:ss past the hour.
  function clock(ms: number): string {
    const t = Math.max(0, Math.round(ms / 1000));
    const ss = String(t % 60).padStart(2, "0");
    const m = Math.floor(t / 60);
    return m >= 60 ? `${Math.floor(m / 60)}:${String(m % 60).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
  }

  const startIso = $derived(frame ? frame.groupStartAt : audio!.absoluteStartAt);
  const start = $derived(parseCapturedAt(startIso));
  const end = $derived(parseCapturedAt(frame ? frame.groupEndAt : audio!.absoluteEndAt));
  const day = $derived(
    start.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" }),
  );
  const segmentMs = $derived(
    audio
      ? parseCapturedAt(audio.audioSegment.endedAt).getTime() -
          parseCapturedAt(audio.audioSegment.startedAt).getTime()
      : 0,
  );
  const span = $derived(
    frame
      ? `${Math.max(1, Math.round((end.getTime() - start.getTime()) / 60_000))} min on screen`
      : `${clock(segmentMs)} recording`,
  );
  const title = $derived(
    frame
      ? (frame.windowTitle ?? frame.appName ?? "Unknown app")
      : audio!.sourceKind === "microphone"
        ? "Microphone"
        : "System audio",
  );
  // Audio has no app or site; its source is already the title.
  const where = $derived(frame ? (frame.url?.split("/")[0] ?? frame.appName ?? "") : "");
  const meaning = $derived(frame ? frame.foundByMeaning : audio!.foundByMeaning);
  // The marked words, for the keyword chip's "“pricing page” on screen".
  const terms = $derived([
    ...new Set(
      parseSearchSnippet(frame ? frame.snippet : audio!.snippet)
        .filter((s) => s.marked)
        .map((s) => s.text.toLowerCase()),
    ),
  ]);
  const matchCount = $derived(frame ? frame.matchCount : audio!.matchCount);

  // ── Frame crop: center the first (best) match box at native pixels ──────
  let natural = $state<{ w: number; h: number } | null>(null);
  let cropW = $state(0);
  let cropH = $state(0);
  $effect(() => {
    void thumbnailUrl;
    natural = null;
  });
  const focus = $derived(frame?.matchBoxes[0] ?? null);
  const offset = $derived.by(() => {
    if (!natural || !focus || !cropW) return null;
    const clamp = (v: number, lo: number) => Math.min(0, Math.max(lo, v));
    // A box wider than the crop centers on its start, as the mockup does.
    const cx = focus.x * natural.w + Math.min(focus.w * natural.w, cropW - 40) / 2;
    const cy = (focus.y + focus.h / 2) * natural.h;
    return {
      x: clamp(cropW / 2 - cx, cropW - natural.w),
      y: clamp(cropH / 2 - cy, cropH - natural.h),
    };
  });

  // ── Audio crop: ±20 s around the matched span, decorative bars ──────────
  // ponytail: bar heights are seeded from the group key, not real peaks
  // (`get_audio_segment_waveform_peaks` exists; wire it if this must be true).
  const BARS = 44;
  const audioWindow = $derived.by(() => {
    if (!audio) return null;
    const from = Math.max(0, audio.spanStartMs - 20_000);
    const to = Math.max(from + 1, Math.min(segmentMs || audio.spanEndMs + 20_000, audio.spanEndMs + 20_000));
    let s = 0;
    for (const ch of audio.groupKey) s = (s * 31 + ch.charCodeAt(0)) >>> 0;
    s = (s % 2147483646) + 1;
    const bars = Array.from({ length: BARS }, (_, i) => {
      s = (s * 16807) % 2147483647;
      const at = from + ((i + 0.5) / BARS) * (to - from);
      return { v: 16 + (s % 85), on: at >= audio.spanStartMs && at <= audio.spanEndMs };
    });
    return { from, to, bars };
  });
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
  class="rc-hero"
  {id}
  role="option"
  aria-selected={selected}
  tabindex="-1"
  onclick={onselect}
>
  <div class="rc-crop" bind:clientWidth={cropW} bind:clientHeight={cropH}>
    {#if frame}
      {#if thumbnailUrl}
        <div
          class="rc-crop__frame"
          class:rc-crop__frame--zoomed={offset !== null}
          style:width={offset ? `${natural?.w}px` : null}
          style:height={offset ? `${natural?.h}px` : null}
          style:transform={offset ? `translate(${offset.x}px, ${offset.y}px)` : null}
        >
          <img
            src={thumbnailUrl}
            alt=""
            onload={(event) => {
              const img = event.currentTarget as HTMLImageElement;
              natural = { w: img.naturalWidth, h: img.naturalHeight };
            }}
          />
          {#if offset}
            {#each frame.matchBoxes as box, i (i)}
              <span
                class="rc-box"
                data-kind={box.kind}
                style:left="{box.x * 100}%"
                style:top="{box.y * 100}%"
                style:width="{box.w * 100}%"
                style:height="{box.h * 100}%"
              ></span>
            {/each}
          {/if}
        </div>
      {/if}
      <time>{hm(start)}</time>
    {:else if audio && audioWindow}
      <div class="rc-acrop" data-src={audio.sourceKind === "microphone" ? "mic" : "sys"}>
        <header>
          {#if audio.sourceKind === "microphone"}<IconMic width="12" height="12" aria-hidden="true" />{:else}<IconSpeaker width="12" height="12" aria-hidden="true" />{/if}
          {audio.sourceKind === "microphone" ? "microphone" : "system audio"} · {clock(segmentMs)}
        </header>
        <div class="rc-wave" aria-hidden="true">
          {#each audioWindow.bars as bar, i (i)}<i class:on={bar.on} style:--v={bar.v}></i>{/each}
        </div>
        <footer>
          <span>{clock(audioWindow.from)}</span><b>{clock(audio.spanStartMs)}</b><span
            >{clock(audioWindow.to)}</span
          >
        </footer>
      </div>
    {/if}
  </div>

  <div class="rc-hero__info">
    <div class="rc-hero__k">
      <span class="mx-kicker">best moment</span><span class="mono">{span}</span>
    </div>
    <h2 class="rc-hero__t">{@render resultTile(result)}<span>{title}</span></h2>
    <div class="rc-meta">
      <span><b>{day}</b> · {hm(start)}{hm(end) === hm(start) ? "" : `–${hm(end)}`}</span>
      <span>{formatRelativeTime(startIso)}</span>
      {#if where}<span>{where}</span>{/if}
    </div>
    <p class="rc-snip">{@render markedSnippet(result)}</p>
    <div class="rc-why">
      {#if meaning}
        <span class="mx-chip">by meaning</span><span>no exact words</span>
      {:else}
        <span class="mx-chip mx-chip--accent">keyword</span><span
          >{terms.map((t) => `“${t}”`).join(" + ")}{matchCount > 1 ? ` ×${matchCount}` : ""}
          {frame ? "on screen" : "in the transcript"}</span
        >
      {/if}
    </div>
    <div class="rc-acts">
      <button
        type="button"
        class="mx-btn mx-btn--sm"
        class:mx-btn--primary={selected}
        onclick={(event) => {
          event.stopPropagation();
          search.openResultAt(0);
        }}
        ><IconTimeline width="13" height="13" aria-hidden="true" />Open in Timeline<kbd>↵</kbd></button
      >
      <button
        type="button"
        class="mx-btn mx-btn--ghost mx-btn--sm"
        onclick={(event) => {
          event.stopPropagation();
          onselect();
          onlook?.();
        }}><IconEye width="13" height="13" aria-hidden="true" />Quick Look</button
      >
      {#if frame?.url}
        <button
          type="button"
          class="mx-btn mx-btn--ghost mx-btn--sm"
          onclick={(event) => {
            event.stopPropagation();
            void search.openCapturedFrameUrl(frame.thumbnailFrameId);
          }}
          ><IconExternal width="13" height="13" aria-hidden="true" />Open page<kbd>⌘↵</kbd></button
        >
      {/if}
    </div>
  </div>
</div>

<style>
  .rc-hero {
    display: grid;
    grid-template-columns: 268px minmax(0, 1fr);
    gap: 18px;
    padding: 10px;
    border-radius: var(--r-lg);
    cursor: default;
    transition: background-color var(--t-fast) var(--ease-quart);
  }
  .rc-hero[aria-selected="true"] {
    background: var(--mx-selected);
  }
  .rc-crop {
    position: relative;
    height: 168px;
    overflow: hidden;
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
  }
  .rc-crop::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: inherit;
    box-shadow: inset 0 0 0 1px var(--app-border);
    pointer-events: none;
  }
  /* Unzoomed (no boxes, or before the preview decodes): the whole frame. */
  .rc-crop__frame {
    position: absolute;
    inset: 0;
  }
  .rc-crop__frame img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .rc-crop__frame--zoomed {
    inset: auto;
    left: 0;
    top: 0;
  }
  .rc-box {
    position: absolute;
    border-radius: 3px;
    background: color-mix(in srgb, var(--app-accent) 12%, transparent);
    box-shadow: inset 0 0 0 1.5px var(--app-accent);
  }
  .rc-box[data-kind="meaning"] {
    background: color-mix(in srgb, var(--app-fg) 7%, transparent);
    box-shadow: none;
    border: 1px dashed var(--app-text-subtle);
  }
  .rc-crop time {
    position: absolute;
    z-index: 1;
    left: 8px;
    bottom: 8px;
    padding: 3px 6px;
    border-radius: var(--r-xs);
    background: var(--app-overlay-bg-strong);
    border: 1px solid var(--app-overlay-border);
    font: 500 var(--text-xs) / 1 var(--font-mono);
    color: var(--app-text-strong);
  }
  .rc-acrop {
    --s: var(--app-source-sysaudio);
    display: grid;
    grid-template-rows: auto 1fr auto;
    gap: 8px;
    height: 100%;
    padding: 12px 12px 10px;
    background: color-mix(in srgb, var(--s) 7%, var(--app-surface-subtle));
  }
  .rc-acrop[data-src="mic"] {
    --s: var(--app-source-mic);
  }
  .rc-acrop header {
    display: flex;
    align-items: center;
    gap: 6px;
    font: 500 var(--text-xs) / 1 var(--font-mono);
    color: var(--s);
    letter-spacing: 0.02em;
  }
  .rc-wave {
    display: flex;
    align-items: center;
    gap: 2px;
    min-height: 0;
  }
  .rc-wave i {
    flex: 1;
    height: calc(var(--v) * 1%);
    min-height: 2px;
    border-radius: 1px;
    background: color-mix(in srgb, var(--app-fg) 18%, transparent);
  }
  .rc-wave i.on {
    background: var(--app-accent);
  }
  .rc-acrop footer {
    display: flex;
    justify-content: space-between;
    font: 400 var(--text-xs) / 1 var(--font-mono);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }
  .rc-acrop footer b {
    font-weight: 500;
    color: var(--app-accent);
  }
  .rc-hero__info {
    min-width: 0;
    display: grid;
    align-content: start;
    gap: 6px;
    padding-top: 2px;
  }
  .rc-hero__k {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .rc-hero__k .mono {
    margin-left: auto;
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }
  .rc-hero__t {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    margin: 2px 0 0;
    font: 600 var(--text-lg) / 1.3 var(--font-sans);
    letter-spacing: -0.012em;
    color: var(--app-text-strong);
  }
  .rc-hero__t > span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rc-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 10px;
    font: 400 var(--text-sm) / 1.4 var(--font-mono);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }
  .rc-meta b {
    font-weight: 500;
    color: var(--app-text);
  }
  .rc-snip {
    margin: 2px 0 0;
    font: 400 var(--text-md) / 1.55 var(--font-sans);
    color: var(--app-text-muted);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .rc-why {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    font: 400 var(--text-sm) / 1.3 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .rc-why .mx-chip {
    height: 18px;
    padding: 0 6px;
  }
  .rc-acts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    margin-top: 6px;
  }
</style>
