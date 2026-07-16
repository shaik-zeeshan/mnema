<script lang="ts">
  import Slider from "$lib/components/Slider.svelte";
  import {
    CAPTURE_INTERVAL_LADDER_S,
    captureIntervalPhrase,
    intervalSToFps,
    nearestLadderIndex,
    relativeStorageLabel,
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
  const snapshotsPerMinute = $derived(Math.round(60 / intervalS));

  // "One minute of recording" strip: a tick per snapshot. Above 80 ticks
  // individual marks blur together, so render a density band instead.
  const dotFractions = $derived(
    snapshotsPerMinute <= 80
      ? Array.from({ length: snapshotsPerMinute }, (_, i) => i / snapshotsPerMinute)
      : null,
  );

  // Log-scaled fill so the bar moves visibly across the whole 1/30×–20× range.
  const storageBarPct = $derived.by(() => {
    const ratio = 2 / intervalS;
    const pct = (Math.log10(ratio * 30) / Math.log10(600)) * 100;
    return Math.min(100, Math.max(4, pct));
  });

  // Sweep line replays the minute in 6 seconds. Plain rAF writing one $state
  // number; dot highlighting derives from it in the template.
  let sweepP = $state(0);
  let reducedMotion = $state(false);
  $effect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    reducedMotion = media.matches;
    if (media.matches || disabled) return;
    let raf = 0;
    const t0 = performance.now();
    const tick = (now: number) => {
      sweepP = ((now - t0) / 6000) % 1;
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });

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
  <div class="capture-rate__ticks" aria-hidden="true">
    <span>10/sec</span><span>1/sec</span><span>every 5s</span><span>every 15s</span><span>1/min</span>
  </div>

  <div class="capture-rate__strip-wrap">
    <div class="capture-rate__strip-label">
      <span class="capture-rate__meta">one minute of recording</span>
      <span class="capture-rate__meta">
        {snapshotsPerMinute} {snapshotsPerMinute === 1 ? "snapshot" : "snapshots"}
      </span>
    </div>
    <div class="capture-rate__strip">
      {#if dotFractions}
        {#each dotFractions as at (at)}
          <div
            class="capture-rate__dot"
            class:capture-rate__dot--hot={!reducedMotion && sweepP >= at && sweepP - at < 0.06}
            style:left="{2 + at * 96}%"
          ></div>
        {/each}
      {:else}
        <div class="capture-rate__band"></div>
      {/if}
      {#if !reducedMotion}
        <div class="capture-rate__sweep" style:left="{2 + sweepP * 96}%"></div>
      {/if}
    </div>
    <div class="capture-rate__storage-bar">
      <i style:width="{storageBarPct}%"></i>
    </div>
    <div class="capture-rate__kv">
      <span class="capture-rate__meta">relative storage &amp; CPU</span>
      <span class="capture-rate__meta">
        <b>{relativeStorageLabel(intervalS)}</b> vs default
      </span>
    </div>
  </div>
</div>

<style>
  .capture-rate {
    display: flex;
    flex-direction: column;
    width: 100%;
    min-width: 0;
  }

  .capture-rate__ticks {
    display: flex;
    justify-content: space-between;
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 10px;
    color: var(--app-text-subtle);
    margin-top: 2px;
    user-select: none;
  }

  .capture-rate__strip-wrap {
    margin-top: 14px;
  }

  .capture-rate__strip-label,
  .capture-rate__kv {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }

  .capture-rate__strip-label {
    margin-bottom: 6px;
  }

  .capture-rate__kv {
    margin-top: 8px;
  }

  .capture-rate__meta {
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 11px;
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }

  .capture-rate__meta b {
    color: var(--app-text-muted);
    font-weight: 600;
  }

  .capture-rate__strip {
    position: relative;
    height: 34px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface-raised);
    overflow: hidden;
  }

  .capture-rate__dot {
    position: absolute;
    top: 50%;
    width: 4px;
    height: 16px;
    border-radius: 2px;
    background: var(--app-accent-strong);
    transform: translate(-50%, -50%);
  }

  .capture-rate__dot--hot {
    background: var(--app-accent);
    box-shadow: 0 0 6px var(--app-accent-glow);
  }

  .capture-rate__band {
    position: absolute;
    top: 50%;
    left: 0;
    right: 0;
    height: 16px;
    transform: translateY(-50%);
    background: repeating-linear-gradient(
      90deg,
      var(--app-accent-strong) 0 2px,
      transparent 2px 5px
    );
    opacity: 0.8;
  }

  .capture-rate__sweep {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: color-mix(in srgb, var(--app-text-strong) 35%, transparent);
  }

  .capture-rate__storage-bar {
    height: 4px;
    border-radius: 999px;
    background: var(--app-border);
    margin-top: 8px;
    overflow: hidden;
  }

  .capture-rate__storage-bar i {
    display: block;
    height: 100%;
    background: linear-gradient(90deg, var(--app-accent-strong), var(--app-accent));
    border-radius: 999px;
    transition: width 0.15s ease;
  }

  @media (prefers-reduced-motion: reduce) {
    .capture-rate__storage-bar i {
      transition: none;
    }
  }
</style>
