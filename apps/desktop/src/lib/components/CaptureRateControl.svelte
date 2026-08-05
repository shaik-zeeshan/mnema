<script lang="ts">
  // ══ INSTRUMENT 1 of 6 — CAPTURE RATE ══════════════════════════════════════
  //
  // Direction 05 "Tactile Instruments". This setting earns an instrument
  // because it passes the rule: the physical quantity is *frames per second*,
  // and the consequence can be written as a fraction of something real —
  // gigabytes a day against the free space actually on this Mac.
  //
  // One anatomy, shared with the other five:
  //   header (name + live value) → WELL (the control, recessed) → gauge WELL
  //   (the consequence) → READOUT (the sentence, with its denominator).
  // The readout slot exists at rest and never reflows.
  //
  // G8 governs every number here. The scale, the notch and the sentence all
  // come from `get_system_facts` via `system-facts.ts`; nothing is invented. If
  // Mnema has not yet measured a full capture day, `projectedBytesPerDay`
  // returns null and this instrument renders its well and NO numbers — a
  // missing denominator is the designed outcome, not a zero and not a guess.
  //
  // The API is unchanged from the control this replaced: `value` is still
  // bindable wire-format fps, and only exact ladder values are ever written back.

  import {
    CAPTURE_INTERVAL_LADDER_S,
    captureIntervalPhrase,
    intervalSToFps,
    nearestLadderIndex,
  } from "./capture-rate";
  import { systemFacts } from "$lib/settings/state/system-facts.svelte";
  import { coarseRuntime, projectedBytesPerDay } from "$lib/settings/state/system-facts";
  import { formatBytes } from "$lib/settings/state/format";

  interface Props {
    // Wire-format fps (`screenFrameRate`); the control renders it as a
    // snapshot interval and only ever writes exact ladder values back.
    value: number;
    disabled?: boolean;
  }

  let { value = $bindable(), disabled = false }: Props = $props();

  $effect(() => {
    void systemFacts.ensureLoaded();
  });

  const facts = $derived(systemFacts.value);

  // The slider moves over ladder indexes. fps→index is a pure projection, so
  // external updates (settings reload) reposition the thumb without loops:
  // index→fps→index round-trips exactly for ladder values.
  const idx = $derived(nearestLadderIndex(value));
  const lastIdx = CAPTURE_INTERVAL_LADDER_S.length - 1;
  const intervalS = $derived(CAPTURE_INTERVAL_LADDER_S[idx]!);

  // The live value in the instrument header. The ladder is stored as intervals,
  // but the physical quantity the user is choosing is a rate, so the header
  // reads the rate and the well's ticks read the ladder.
  const fps = $derived(intervalSToFps(intervalS));
  const headline = $derived(
    fps >= 1
      ? { n: String(Math.round(fps * 10) / 10), unit: "fps" }
      : { n: String(Math.round(intervalS)), unit: "s / frame" },
  );

  // ── the gauge ────────────────────────────────────────────────────────────
  // Scale: 0 → what the FASTEST ladder stop would cost per day. That is a real
  // denominator (it is the most this control can spend), so the bar answers
  // "how much of what this setting could cost am I actually spending?".
  const perDay = $derived(projectedBytesPerDay(facts, fps));
  const maxPerDay = $derived(
    projectedBytesPerDay(facts, intervalSToFps(CAPTURE_INTERVAL_LADDER_S[0]!)),
  );
  const fillPct = $derived(
    perDay !== null && maxPerDay !== null && maxPerDay > 0
      ? Math.min(100, Math.max(1.5, (perDay / maxPerDay) * 100))
      : null,
  );
  // The notch is the 7-day measured average — what you are ACTUALLY spending
  // today, so moving the knob is legible as "more or less than I spend now".
  const notchPct = $derived(
    facts?.measuredBytesPerDay != null && maxPerDay !== null && maxPerDay > 0
      ? Math.min(100, (facts.measuredBytesPerDay / maxPerDay) * 100)
      : null,
  );
  const runtime = $derived(coarseRuntime(facts?.diskFreeBytes ?? null, perDay));

  // A tick is a scale mark, not a sentence: "10/s", "2s", "60s".
  function tickLabel(stop: number): string {
    return stop < 1 ? `${Math.round(1 / stop)}/s` : `${stop}s`;
  }

  function setIndex(nextIdx: number) {
    const interval = CAPTURE_INTERVAL_LADDER_S[nextIdx];
    if (interval !== undefined) value = intervalSToFps(interval);
  }
</script>

<div class="ti-instr ti-instr--bare capture-rate">
  <div class="ti-instr__hd">
    <span class="ti-instr__name">Capture rate</span>
    <span class="ti-instr__sub">what a snapshot costs you in disk</span>
    <span class="ti-instr__v">{headline.n}<em>{headline.unit}</em></span>
  </div>

  <!-- The WELL. A real <input type=range> lives invisibly on top, so the
       instrument keeps native keyboard stepping, drag and accessibility for
       free — the drawn track is decoration over a stock control, never a
       re-implementation of one. -->
  <div class="ti-well ti-rate">
    <input
      class="ti-rate__input"
      type="range"
      min="0"
      max={lastIdx}
      step="1"
      value={lastIdx - idx}
      {disabled}
      aria-label="Capture rate"
      aria-valuetext={captureIntervalPhrase(intervalS)}
      oninput={(e) => setIndex(lastIdx - Number(e.currentTarget.value))}
    />
    <span class="ti-rate__track">
      <span class="ti-rate__fill" style:width="{((lastIdx - idx) / lastIdx) * 100}%"></span>
      {#each CAPTURE_INTERVAL_LADDER_S as _stop, i (i)}
        <span class="ti-rate__notch" style:left="{(i / lastIdx) * 100}%"></span>
      {/each}
      <span class="ti-rate__knob" style:left="{((lastIdx - idx) / lastIdx) * 100}%"></span>
    </span>
  </div>

  <!-- The well carries a notch per ladder stop, but only every other stop is
       LABELLED (plus wherever the knob is) — eleven labels under a 600px well
       is a wall of numbers, and the point of the scale is orientation, not a
       full legend. -->
  <div class="ti-rate__ticks" aria-hidden="true">
    {#each CAPTURE_INTERVAL_LADDER_S as stop, i (stop)}
      {@const shown = lastIdx - i}
      <span class:is-on={i === idx}>{shown % 2 === 0 || i === idx ? tickLabel(stop) : ""}</span>
    {/each}
  </div>

  <!-- The consequence, as a second face. Rendered only when the machine can
       actually answer: G8 forbids a gauge with an invented denominator. -->
  {#if fillPct !== null}
    <div class="ti-well ti-gauge">
      <span class="ti-gauge__track">
        <span class="ti-gauge__seg ti-gauge__seg--a" style:width="{fillPct}%"></span>
        {#if notchPct !== null}
          <span class="ti-gauge__notch" style:left="{notchPct}%"></span>
        {/if}
      </span>
    </div>
    <div class="ti-gauge__scale">
      <span>0</span>
      {#if maxPerDay !== null}<span>{formatBytes(maxPerDay)}/day at the fastest rate</span>{/if}
    </div>
    <div class="ti-legend">
      <span><i></i>at this rate · {perDay === null ? "—" : formatBytes(perDay)} a day</span>
      {#if facts?.measuredBytesPerDay != null}
        <span><i class="n"></i>your 7-day average · {formatBytes(facts.measuredBytesPerDay)} a day</span>
      {/if}
    </div>
  {/if}

  <div class="ti-instr__out">
    {#if perDay === null}
      Mnema has not measured a full day of capture yet, so it will not guess what
      this rate costs. The figure appears once there is a real day to divide.
    {:else}
      ≈ <b>{formatBytes(perDay)}</b> a day{#if facts?.diskFreeBytes != null}
        · <b>{formatBytes(facts.diskFreeBytes)}</b> free on this Mac{/if}{#if runtime}
        — {runtime} before retention starts culling{/if}.
    {/if}
  </div>
</div>

<style>
  .capture-rate {
    width: 100%;
    min-width: 0;
  }

  /* The ladder has five stops and their phrases are wider than a bare number,
     so the tick row wraps its ends inward instead of overhanging the well. */
  .capture-rate :global(.ti-rate__ticks span:first-child) {
    text-align: left;
  }
  .capture-rate :global(.ti-rate__ticks span:last-child) {
    text-align: right;
  }
</style>
