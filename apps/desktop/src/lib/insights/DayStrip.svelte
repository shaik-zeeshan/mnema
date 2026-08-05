<script lang="ts">
  // The Overview bento's important-moments strip (frame 04): the day's
  // activities' headline frames as real, naked media in a padding-0 tile.
  // Clicking a frame hands off to the Main Timeline at that instant via the
  // same broker command Quick Recall results use.
  import { framePreviewAssetUrl } from "$lib/frame-preview";
  import { clockHM } from "$lib/insights/overview-format";
  import type { DayMoment } from "$lib/types/recording";

  let {
    moments,
    onOpen,
  }: {
    moments: DayMoment[];
    onOpen: (moment: DayMoment) => void;
  } = $props();

  // A frame whose media file aged out simply never fires `onload` — the glyph
  // placeholder stays (the Quick Recall cell pattern; there is no error path).
  let loaded = $state<Record<number, boolean>>({});
</script>

<div class="strip">
  {#each moments as moment (moment.frameId)}
    <button
      type="button"
      class="strip__cell"
      title={moment.activityTitle}
      aria-label={`${moment.activityTitle} — open in Timeline`}
      onclick={() => onOpen(moment)}
    >
      <span class="strip__glyph" aria-hidden="true">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
          <rect x="3" y="5" width="18" height="14" rx="2" />
          <path d="m3 15 4.5-4.5L12 15l3.5-3.5L21 17" />
        </svg>
      </span>
      <img
        class="strip__img"
        class:strip__img--loaded={loaded[moment.frameId]}
        src={framePreviewAssetUrl(moment.framePath)}
        alt=""
        loading="lazy"
        draggable="false"
        onload={() => (loaded[moment.frameId] = true)}
      />
      <span class="strip__t">{clockHM(moment.capturedAtMs)}</span>
    </button>
  {/each}
</div>

<style>
  .strip {
    display: flex;
    height: var(--strip-h, 148px);
    gap: 2px;
    background: #0b0b10;
  }

  .strip__cell {
    flex: 1 1 0;
    min-width: 0;
    position: relative;
    overflow: hidden;
    display: block;
    padding: 0;
    border: 0;
    background: transparent;
    cursor: pointer;
  }

  /* One scrim so the caption survives any frame (frame 04 / KNOWN GAP 2). */
  .strip__cell::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(to top, rgba(6, 6, 10, 0.72), rgba(6, 6, 10, 0) 42%);
    pointer-events: none;
  }

  .strip__glyph {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: rgba(255, 255, 255, 0.18);
  }

  .strip__glyph svg {
    width: 28px;
    height: 28px;
  }

  .strip__img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0;
    transition: opacity var(--dur-regular) var(--ease);
  }

  .strip__img--loaded {
    opacity: 1;
  }

  .strip__t {
    position: absolute;
    left: var(--s-8);
    bottom: var(--s-6);
    z-index: 2;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: 0.02em;
    color: #f2f2f5;
    font-variant-numeric: tabular-nums;
  }

  .strip__cell:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--app-accent);
  }
</style>
