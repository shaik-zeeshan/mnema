<script lang="ts">
  // Overview's 30px contextual tool strip — the direction's second fixed piece.
  // 22px controls (`--h-sm`), hairline separators, and only controls that belong
  // to THIS surface.
  //
  // What it carries: the day the bento is about, the day's one-line summary, and
  // the inspector toggle. The mockup also drew a Day/Week/Month segmented and a
  // "Re-run digest" button — both are omitted, see the notes at each seam below.
  import IconPanel from "~icons/lucide/panel-right";
  import IconPrev from "~icons/lucide/chevron-left";
  import IconNext from "~icons/lucide/chevron-right";
  import IconUpDown from "~icons/lucide/chevrons-up-down";
  import { formatDayShort, dayKeyOf } from "./overview-format";

  interface Props {
    dayKey: string;
    /** The day's summary, already assembled from real counts (or null). */
    summary: string | null;
    inspectorOpen: boolean;
    /** False below 1000px — the toggle is hidden, not broken. */
    inspectorAvailable: boolean;
    onday: (dayKey: string) => void;
    onstep: (days: number) => void;
    ontoggleinspector: () => void;
  }

  let {
    dayKey,
    summary,
    inspectorOpen,
    inspectorAvailable,
    onday,
    onstep,
    ontoggleinspector,
  }: Props = $props();

  const todayKey = dayKeyOf(new Date());
  const isToday = $derived(dayKey === todayKey);
  // A day in the future has nothing to show; the step forward stops at today.
  const canStepForward = $derived(dayKey < todayKey);
</script>

<div class="ss-tstrip">
  <div class="ss-tstrip__g">
    <button
      type="button"
      class="btn btn--sm btn--icon btn--ghost"
      aria-label="Previous day"
      onclick={() => onstep(-1)}
    ><IconPrev /></button>
    <!-- Native date input: the OS already ships a calendar, a keyboard path and
         a locale-correct format. A bespoke popover would be the same control
         with bugs. Styled as the kit's `.ss-pop`. -->
    <label class="ss-pop pop" aria-label="Day shown">
      <span class="pop__t">{formatDayShort(dayKey)}</span>
      <!-- The accent chevron badge is this direction's signature control mark. -->
      <span class="ss-pop__b" aria-hidden="true"><IconUpDown /></span>
      <input type="date" value={dayKey} max={todayKey} onchange={(e) => onday(e.currentTarget.value)} />
    </label>
    <button
      type="button"
      class="btn btn--sm btn--icon btn--ghost"
      aria-label="Next day"
      disabled={!canStepForward}
      onclick={() => onstep(1)}
    ><IconNext /></button>
    {#if !isToday}
      <button type="button" class="btn btn--sm btn--ghost" onclick={() => onday(todayKey)}>Today</button>
    {/if}
  </div>

  {#if summary}
    <div class="ss-tstrip__sep"></div>
    <div class="ss-tstrip__g"><span class="t-meta">{summary}</span></div>
  {/if}

  <span class="ss-tstrip__spacer"></span>

  {#if inspectorAvailable}
    <div class="ss-tstrip__g">
      <button
        type="button"
        class="btn btn--sm btn--icon"
        class:is-on={inspectorOpen}
        aria-pressed={inspectorOpen}
        aria-label="Toggle inspector"
        onclick={ontoggleinspector}
      ><IconPanel /></button>
    </div>
  {/if}
</div>

<style>
  .pop {
    position: relative;
    gap: var(--s-6);
    padding: 0 var(--s-8);
    cursor: default;
  }

  .pop__t {
    font-variant-numeric: tabular-nums;
  }

  .pop :global(.ss-pop__b svg) {
    width: 9px;
    height: 9px;
    stroke-width: 2.4;
  }

  /* The native picker overlays the pill: its own indicator stays the click
     target, the text underneath is what you read. */
  .pop input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    opacity: 0;
    border: 0;
    padding: 0;
    cursor: default;
  }

  .btn.is-on {
    background: var(--app-surface-active);
  }
</style>
