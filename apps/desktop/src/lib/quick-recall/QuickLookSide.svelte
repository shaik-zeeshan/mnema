<!-- Quick Look's side panel (recall.html `.rc-look__side`): what the stage
     shows, why it matched, its text (screen) or transcript (audio), and the
     actions — Open this frame / at this line (↵), Copy (⌘C), Page (⌘↵).
     Module snippet `marked` paints highlight segments with the kit mark. -->
<script module lang="ts">
  import type { HighlightSegment } from "./context-highlight";

  export { marked };
</script>

<script lang="ts">
  import IconTimeline from "~icons/lucide/chart-no-axes-gantt";
  import IconCopy from "~icons/lucide/copy";
  import IconExternal from "~icons/lucide/external-link";
  import { parseCapturedAt } from "$lib/format-time";
  import type { ScrubPreviewMissingReason } from "$lib/types/app-infra";
  import { quickLook as look } from "./detailStore.svelte";
  import { quickRecallSearch as search } from "./searchStore.svelte";
  import { resultTile } from "./SearchResultCard.svelte";
  import { highlightSegments, residualTerms } from "./context-highlight";
  import { formatTurnClock, segmentDurationMs } from "./detail-view";
  import { clock } from "./quick-look";

  let { gone, audioTitle }: { gone: ScrubPreviewMissingReason | undefined; audioTitle: string } =
    $props();

  const r = $derived(look.result!);
  const frame = $derived(r.kind === "frame" ? r.frame : null);
  const audio = $derived(r.kind === "audio" ? r.audio : null);
  const terms = $derived(residualTerms(search.residualQuery));
  const meaning = $derived(frame ? frame.foundByMeaning : audio!.foundByMeaning);
  const frameMs = $derived(look.frame?.ms ?? 0);
  const turns = $derived(look.turns ?? []);
  const segmentMs = $derived(audio ? segmentDurationMs(audio.audioSegment) : 0);
  const lines = $derived.by(() => {
    const text = look.frame ? look.ocr[look.frame.id] : undefined;
    return (text ?? "")
      .split("\n")
      .filter((l) => l.trim())
      .map((l) => highlightSegments(l, terms));
  });
  const nHit = $derived(lines.reduce((n, segs) => n + segs.filter((s) => s.marked).length, 0));

  const hms = (d: Date) =>
    d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
  const hm = (d: Date) => d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false });
  const day = (d: Date) => d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });

  // Keep the scrubbed transcript line in view.
  let linesEl = $state<HTMLElement | null>(null);
  $effect(() => {
    void look.k;
    linesEl?.querySelector("[data-cur]")?.scrollIntoView({ block: "nearest" });
  });
</script>

{#snippet marked(segs: HighlightSegment[])}{#each segs as seg, i (i)}{#if seg.marked}<mark>{seg.text}</mark>{:else}{seg.text}{/if}{/each}{/snippet}

<aside class="rc-look__side">
  {#if frame}
    <div class="rc-side__k">
      {@render resultTile(r)}<span>{frame.appName ?? "Unknown app"}</span><span class="mx-chip mx-chip--screen"
        >screen</span
      >
    </div>
    <h3 class="rc-side__t">{frame.windowTitle ?? frame.appName ?? "Untitled"}</h3>
    <div class="rc-side__when">
      <span>{day(new Date(frameMs))} · {hms(new Date(frameMs))}</span>
      <small>{frame.url ?? `${Math.max(1, Math.round((parseCapturedAt(frame.groupEndAt).getTime() - parseCapturedAt(frame.groupStartAt).getTime()) / 60_000))} min on screen`}</small>
    </div>
    <div class="rc-why">
      {#if meaning}
        <span class="mx-chip">by meaning</span><span>no exact words</span>
      {:else}
        <span class="mx-chip mx-chip--accent">keyword</span><span>{nHit} on this frame</span>
      {/if}
    </div>
    <div class="rc-side__txt">
      <header><span class="mx-label">Text on this frame</span><small>{lines.length} lines</small></header>
      <div class="rc-side__lines">
        {#each lines as segs, i (i)}
          <p data-m={segs.some((s) => s.marked) || undefined}>{@render marked(segs)}</p>
        {:else}
          <p>{look.frame && look.ocr[look.frame.id] === undefined ? "Reading…" : "No text was read off this frame."}</p>
        {/each}
      </div>
    </div>
    <div class="rc-side__acts">
      <button type="button" class="mx-btn mx-btn--primary mx-btn--sm" onclick={() => void look.openHere()}
        ><IconTimeline width="13" height="13" aria-hidden="true" />{gone ? "Open in Timeline" : "Open this frame"}<kbd
          >↵</kbd
        ></button
      >
      <div class="rc-side__row">
        <button type="button" class="mx-btn mx-btn--sm" onclick={() => void look.copy()}
          ><IconCopy width="12" height="12" aria-hidden="true" />{look.copied ? "Copied" : "Copy text"}<kbd>⌘C</kbd></button
        >
        {#if frame.url}
          <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => look.openPage()}
            ><IconExternal width="12" height="12" aria-hidden="true" />Page<kbd>⌘↵</kbd></button
          >
        {/if}
      </div>
    </div>
  {:else if audio}
    {@const src = audio.sourceKind === "microphone"}
    <div class="rc-side__k">
      {@render resultTile(r)}<span>{src ? "Microphone" : "System audio"}</span><span
        class="mx-chip {src ? 'mx-chip--mic' : 'mx-chip--sysaudio'}">{src ? "mic" : "system"}</span
      >
    </div>
    <h3 class="rc-side__t">{audioTitle}</h3>
    <div class="rc-side__when">
      <span
        >{day(parseCapturedAt(audio.audioSegment.startedAt))} · {formatTurnClock(
          audio.audioSegment.startedAt,
          look.turn?.startMs ?? audio.spanStartMs,
        )}</span
      >
      <small>{clock(segmentMs)} recording</small>
    </div>
    {#if audio.alignedFrame}
      {@const af = audio.alignedFrame}
      <button type="button" class="rc-side__also" onclick={() => void look.openAligned()}>
        <i>{#if look.thumbs[af.id]}<img src={look.thumbs[af.id]} alt="" />{/if}</i><b>On screen then</b><small
          >{af.appName ?? "Screen"} · {hm(parseCapturedAt(af.capturedAt))}</small
        >
      </button>
    {:else}
      <div class="rc-why">
        <span class="mx-chip {meaning ? '' : 'mx-chip--accent'}">{meaning ? "by meaning" : "keyword"}</span><span
          >in the transcript</span
        >
      </div>
    {/if}
    <div class="rc-side__txt">
      <header><span class="mx-label">Transcript</span><small>{turns.length} lines</small></header>
      <div class="rc-side__lines" bind:this={linesEl}>
        {#each turns as t, j (j)}
          <div class="turn" data-cur={j === look.k || undefined}>
            <time>{clock(t.startMs)}</time><span
              >{#if t.speakerLabel}<b>{t.speakerLabel}</b>{/if}
              {@render marked(highlightSegments(t.text, terms))}</span
            >
          </div>
        {:else}
          <p>Reading…</p>
        {/each}
      </div>
    </div>
    <div class="rc-side__acts">
      <button type="button" class="mx-btn mx-btn--primary mx-btn--sm" onclick={() => void look.openHere()}
        ><IconTimeline width="13" height="13" aria-hidden="true" />Open at this line<kbd>↵</kbd></button
      >
      <div class="rc-side__row">
        <button type="button" class="mx-btn mx-btn--sm" onclick={() => void look.copy()}
          ><IconCopy width="12" height="12" aria-hidden="true" />{look.copied ? "Copied" : "Copy quote"}<kbd
            >⌘C</kbd
          ></button
        >
      </div>
    </div>
  {/if}
</aside>

<style>
  .rc-look__side {
    min-width: 0;
    min-height: 0;
    display: grid;
    grid-template-rows: auto auto auto auto minmax(0, 1fr) auto;
    gap: 10px;
    padding: 14px 14px 12px;
    border-left: 1px solid var(--mx-hairline);
  }
  .rc-side__k {
    display: flex;
    align-items: center;
    gap: 8px;
    font: 500 var(--text-base) / 1 var(--font-sans);
    color: var(--app-text-muted);
  }
  .rc-side__k .mx-chip {
    margin-left: auto;
  }
  .rc-side__t {
    margin: 0;
    font: 600 var(--text-md) / 1.3 var(--font-sans);
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
    overflow-wrap: anywhere;
  }
  .rc-side__when {
    display: grid;
    gap: 5px;
    font: 450 var(--text-sm) / 1.2 var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text);
  }
  .rc-side__when small {
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
  .rc-side__txt {
    min-height: 0;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    gap: 6px;
  }
  .rc-side__txt header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }
  .rc-side__txt header small {
    font: 400 var(--text-xs) / 1 var(--font-mono);
    color: var(--app-text-subtle);
  }
  .rc-side__lines {
    min-height: 0;
    overflow: auto;
    padding: 8px 10px;
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
    border: 1px solid var(--mx-hairline);
    font: 400 var(--text-base) / 1.55 var(--font-sans);
    color: var(--app-text-muted);
  }
  .rc-side__lines p {
    margin: 0 0 2px;
    overflow-wrap: anywhere;
  }
  .rc-side__lines p[data-m] {
    color: var(--app-text-strong);
  }
  .turn {
    display: grid;
    grid-template-columns: 38px minmax(0, 1fr);
    gap: 6px;
    margin: 0 -6px 2px;
    padding: 3px 6px;
    border-radius: var(--r-sm);
  }
  .turn time {
    font: 450 var(--text-xs) / 1.9 var(--font-mono);
    color: var(--app-text-subtle);
  }
  .turn b {
    font-weight: 600;
    color: var(--app-text);
  }
  .turn[data-cur] {
    background: var(--mx-selected);
    color: var(--app-text);
  }
  .rc-side__also {
    display: grid;
    grid-template-columns: 64px minmax(0, 1fr);
    gap: 2px 10px;
    align-items: center;
    padding: 5px;
    margin: 0 -5px;
    border: 0;
    border-radius: var(--r-md);
    background: none;
    color: inherit;
    text-align: left;
    cursor: pointer;
    font: inherit;
    transition: background-color var(--t-fast) var(--ease-quart);
  }
  .rc-side__also:hover {
    background: var(--mx-wash);
  }
  .rc-side__also > i {
    grid-row: span 2;
    width: 64px;
    height: 40px;
    overflow: hidden;
    border-radius: 4px;
    box-shadow: 0 0 0 1px var(--app-border);
    background: var(--app-surface-subtle);
  }
  .rc-side__also img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: top;
  }
  .rc-side__also b {
    font: 500 var(--text-base) / 1.2 var(--font-sans);
    color: var(--app-text-strong);
  }
  .rc-side__also small {
    font: 400 var(--text-xs) / 1.2 var(--font-mono);
    color: var(--app-text-subtle);
  }
  .rc-side__acts {
    display: grid;
    gap: 6px;
  }
  .rc-side__acts .mx-btn {
    width: 100%;
  }
  .rc-side__acts .mx-btn kbd {
    margin-left: auto;
  }
  .rc-side__row {
    display: flex;
    gap: 6px;
  }
  .rc-side__row .mx-btn {
    flex: 1;
    min-width: 0;
  }
  .rc-look__side {
    animation: mx-rise var(--t-slow) var(--ease-expo) 120ms both;
  }
  @media (prefers-reduced-motion: reduce) {
    .rc-look__side {
      animation: none;
    }
  }
</style>
