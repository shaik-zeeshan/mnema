<script lang="ts">
  // The day's headline frames, as a full-width media strip (`.ss-tile--media`).
  // `Moment.filePath` is an on-disk path, so every thumbnail goes through
  // `convertFileSrc`; a frame the webview can't load falls back to its own empty
  // fill rather than a broken-image glyph.
  import { convertFileSrc } from "@tauri-apps/api/core";
  import type { Moment } from "$lib/highlights";
  import type { LoadState, Selection } from "./overview-data.svelte";
  import { formatClock, formatSpan } from "./overview-format";

  interface Props {
    moments: LoadState<Moment[]>;
    selectedKey: string | null;
    onselect: (selection: Selection) => void;
  }

  let { moments, selectedKey, onselect }: Props = $props();

  const rows = $derived(moments.status === "ok" ? moments.value : []);

  const quiet = $derived(
    moments.status === "failed"
      ? "Couldn't read this day's frames."
      : moments.status === "loading"
        ? null
        : rows.length === 0
          ? "No screen capture on this day."
          : null,
  );

  function toSelection(m: Moment): Selection {
    return {
      key: `moment:${m.frameId}`,
      source: "Moments",
      title: m.title,
      sections: [
        {
          label: "Frame",
          rows: [
            { k: "Captured", v: formatClock(m.capturedAtMs) ?? "", mono: true },
            { k: "Activity", v: m.title },
            { k: "Span", v: formatSpan(m.durationMs), mono: true },
            ...(m.focus ? [{ k: "Focus", v: m.focus }] : []),
          ],
        },
      ],
    };
  }
</script>

<section class="ss-tile ss-tile--media ss-tile--4">
  {#if quiet}
    <p class="quiet">{quiet}</p>
  {:else}
    <div class="strip">
      {#each rows as m (m.frameId)}
        <button
          type="button"
          class="shot"
          class:is-sel={selectedKey === `moment:${m.frameId}`}
          title={m.title}
          onclick={() => onselect(toSelection(m))}
        >
          <img src={convertFileSrc(m.filePath)} alt="" loading="lazy" />
          <span class="shot__t">{formatClock(m.capturedAtMs)}</span>
        </button>
      {/each}
    </div>
  {/if}
</section>

<style>
  .strip {
    display: flex;
    gap: var(--s-8);
    padding: var(--s-8);
    height: 100%;
    min-height: 0;
  }

  .shot {
    position: relative;
    flex: 1 1 auto;
    min-width: 0;
    padding: 0;
    border: 0;
    border-radius: var(--r-sm);
    overflow: hidden;
    background: var(--app-surface-subtle);
    box-shadow: 0 0 0 var(--hairline) var(--app-border);
    cursor: default;
  }

  .shot img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .shot.is-sel {
    box-shadow: 0 0 0 2px var(--app-accent);
  }

  .shot__t {
    position: absolute;
    left: 5px;
    bottom: 5px;
    padding: 1px 5px;
    border-radius: 3px;
    background: rgb(0 0 0 / 60%);
    color: #fff;
    font: var(--w-medium) var(--t-label) / 1.4 var(--app-font-mono);
  }

  .quiet {
    margin: 0;
    padding: var(--s-12);
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text-subtle);
  }
</style>
