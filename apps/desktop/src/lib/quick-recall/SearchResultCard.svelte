<!-- One ranked Quick Recall row (mockup `recall.html` `.rc-row`): app or
     source tile | title + marked snippet | "meaning" tag | time | Quick Look
     eye (on hover / selection). DOM focus stays on the field; the list is
     driven via aria-activedescendant, so the row itself is a plain option. -->
<script module lang="ts">
  import IconMic from "~icons/lucide/mic";
  import IconSpeaker from "~icons/lucide/volume-2";
  import { parseSearchSnippet } from "$lib/search-snippet";
  import { appIcons } from "./app-icons.svelte";
  import type { RankedResult } from "./result-sections";

  export { resultTile, markedSnippet };
</script>

<script lang="ts">
  import IconEye from "~icons/lucide/eye";
  import { parseCapturedAt } from "$lib/format-time";
  import { momentTime } from "./result-sections";

  let {
    result,
    selected = false,
    id = undefined,
    onselect,
    onlook,
  }: {
    result: RankedResult;
    selected?: boolean;
    id?: string;
    onselect: () => void;
    onlook?: () => void;
  } = $props();

  const title = $derived(
    result.kind === "frame"
      ? (result.frame.windowTitle ?? result.frame.appName ?? "Unknown app")
      : result.audio.sourceKind === "microphone"
        ? "Microphone"
        : "System audio",
  );
  const meaning = $derived(
    result.kind === "frame" ? result.frame.foundByMeaning : result.audio.foundByMeaning,
  );
  const at = $derived(
    parseCapturedAt(
      result.kind === "frame" ? result.frame.groupStartAt : result.audio.absoluteStartAt,
    ),
  );
</script>

<!-- The app icon (letter fallback) for a frame; the source glyph for audio. -->
{#snippet resultTile(result: RankedResult)}
  {#if result.kind === "frame"}
    {@const src = appIcons.src(result.frame.appBundleId ?? result.frame.appName)}
    {#if src}<img class="rc-app rc-app--img" {src} alt="" />{:else}<i class="rc-app"
        >{(result.frame.appName ?? "?").charAt(0).toUpperCase()}</i
      >{/if}
  {:else if result.audio.sourceKind === "microphone"}
    <i class="rc-app" data-src="mic"><IconMic width="11" height="11" aria-hidden="true" /></i>
  {:else}
    <i class="rc-app" data-src="sys"><IconSpeaker width="11" height="11" aria-hidden="true" /></i>
  {/if}
{/snippet}

<!-- The backend snippet with its <mark> runs; audio reads as a quote. -->
{#snippet markedSnippet(result: RankedResult)}
  {@const text = result.kind === "frame" ? result.frame.snippet : result.audio.snippet}
  {#if result.kind === "audio"}“{/if}{#each parseSearchSnippet(text) as seg, i (i)}{#if seg.marked}<mark
        >{seg.text}</mark
      >{:else}{seg.text}{/if}{/each}{#if result.kind === "audio"}”{/if}
{/snippet}

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="rc-row" {id} role="option" aria-selected={selected} tabindex="-1" onclick={onselect}>
  {@render resultTile(result)}
  <span class="rc-row__t">{title}<span>{@render markedSnippet(result)}</span></span>
  {#if meaning}<span class="rc-row__k">meaning</span>{:else}<span></span>{/if}
  <time>{momentTime(at)}</time>
  <button
    type="button"
    class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm rc-peek"
    tabindex="-1"
    aria-label="Quick Look"
    onclick={(event) => {
      event.stopPropagation();
      onselect();
      onlook?.();
    }}><IconEye width="14" height="14" aria-hidden="true" /></button
  >
</div>

<style>
  .rc-row {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) auto auto 26px;
    gap: 10px;
    align-items: center;
    height: 34px;
    padding: 0 4px 0 10px;
    border-radius: var(--r-md);
    cursor: default;
    color: var(--app-text);
    transition: background-color var(--t-fast) var(--ease-quart);
  }
  .rc-row:hover {
    background: var(--mx-wash);
  }
  .rc-row[aria-selected="true"] {
    background: var(--mx-selected);
  }
  .rc-row__t {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 500 var(--text-md) / 1 var(--font-sans);
    color: var(--app-text-strong);
  }
  .rc-row__t > span {
    margin-left: 8px;
    font-weight: 400;
    color: var(--app-text-muted);
  }
  .rc-row__k {
    font: 400 var(--text-xs) / 1 var(--font-mono);
    color: var(--app-text-subtle);
  }
  .rc-row time {
    min-width: 64px;
    text-align: right;
    font: 450 var(--text-sm) / 1 var(--font-mono);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }
  .rc-row[aria-selected="true"] time {
    color: var(--app-text);
  }
  /* The row's Quick Look affordance: shows on hover / selection. */
  .rc-peek {
    opacity: 0;
    transition: opacity var(--t-fast) var(--ease-quart);
  }
  .rc-row:hover .rc-peek,
  .rc-row[aria-selected="true"] .rc-peek,
  .rc-peek:focus-visible {
    opacity: 1;
  }
  .rc-app {
    --c: var(--app-text-muted);
    flex: none;
    display: inline-grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 5px;
    font: 700 10px / 1 var(--font-sans);
    font-style: normal;
    color: var(--c);
    background: color-mix(in srgb, var(--c) 18%, var(--app-surface));
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--c) 30%, transparent);
  }
  .rc-app[data-src="mic"] {
    --c: var(--app-source-mic);
  }
  .rc-app[data-src="sys"] {
    --c: var(--app-source-sysaudio);
  }
  .rc-app--img {
    object-fit: contain;
    background: none;
    box-shadow: none;
  }
</style>
