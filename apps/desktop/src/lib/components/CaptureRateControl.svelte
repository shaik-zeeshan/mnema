<script lang="ts">
  // Custom input 1 of 5 — the frame-rate slider.
  //
  // It earns its weight by the one rule: the stored value and the value you
  // care about are different units. The setting is `screenFrameRate` (fps); the
  // question is "how much disk a day". So the control shows the ladder it snaps
  // to, and the caller prints the measured GB/day beneath it
  // (`captureRateConsequence`, computed from this machine's own capture days).
  //
  // Everything the old control drew ON TOP of that — the replayed one-minute
  // filmstrip, the rAF sweep line, the "relative storage vs default" bar — was
  // a second, weaker answer to the same question, in made-up units. Deleted.

  import Slider from "$lib/components/Slider.svelte";
  import {
    CAPTURE_INTERVAL_LADDER_S,
    captureIntervalPhrase,
    intervalSToFps,
    nearestLadderIndex,
  } from "./capture-rate";

  interface Props {
    // Wire-format fps (`screenFrameRate`); the control renders it as a
    // snapshot interval and only ever writes exact ladder values back.
    value: number;
    disabled?: boolean;
  }

  let { value = $bindable(), disabled = false }: Props = $props();

  // The slider moves over ladder indexes. fps→index is a pure projection, so
  // external updates (settings reload) reposition the thumb without loops:
  // index→fps→index round-trips exactly for ladder values.
  const idx = $derived(nearestLadderIndex(value));
  const intervalS = $derived(CAPTURE_INTERVAL_LADDER_S[idx]!);

  // Five anchor stops off the 11-stop ladder — enough to read the axis without
  // crowding it. Each carries its ladder index so the one you are standing on
  // can light up.
  const TICKS: { index: number; label: string }[] = [
    { index: 0, label: "10/sec" },
    { index: 2, label: "1/sec" },
    { index: 5, label: "every 5s" },
    { index: 7, label: "every 15s" },
    { index: 10, label: "1/min" },
  ];

  function handleIndexChange(nextIdx: number) {
    const interval = CAPTURE_INTERVAL_LADDER_S[nextIdx];
    if (interval !== undefined) value = intervalSToFps(interval);
  }
</script>

<div class="capture-rate">
  <Slider
    value={idx}
    onValueChange={handleIndexChange}
    min={0}
    max={CAPTURE_INTERVAL_LADDER_S.length - 1}
    step={1}
    {disabled}
    label="Take a snapshot"
    formatValue={(i) => captureIntervalPhrase(CAPTURE_INTERVAL_LADDER_S[i] ?? intervalS)}
  />
  <div class="inst-ticks" aria-hidden="true">
    {#each TICKS as tick (tick.index)}
      <span class:is-on={idx === tick.index}>{tick.label}</span>
    {/each}
  </div>
</div>

<style>
  .capture-rate {
    display: flex;
    flex-direction: column;
    width: 100%;
    min-width: 0;
  }

  /* The ladder under the track: mono, tabular, faint — except the stop you are
     standing on. */
  .inst-ticks {
    display: flex;
    justify-content: space-between;
    margin-top: var(--s-6);
    user-select: none;
  }

  .inst-ticks span {
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: var(--t-label);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-faint);
  }

  .inst-ticks span.is-on {
    color: var(--app-text-strong);
  }
</style>
