<!--
  Onboarding provider choice (issue #195, slice 9; redesign ON2).

  Skin of record: `docs/mockups/redesign/onboarding.html` (Change settings →
  Engines): three `mx-choice` tiles with download/memory meters. Real radios
  carry ←→ and form state natively.

  What it says, and why:
   · One engine is RECOMMENDED; the other two carry their price as a real delta
     (download from the manifest, memory from the measured table in
     `transcription-engines.ts`). "Use recommended" always walks it back.
   · Only measured axes are drawn: download and memory. Speed/accuracy are not
     benchmarked in this build — don't add an empty track back, add the
     benchmark. An estimated memory figure gets a dashed meter, an unmeasured
     one an empty track.
   · Deepgram is never selectable here (ADR 0047): cloud transcription is
     Settings-only behind a consent gate, filtered out of the list this gets;
     a quiet disclosure under the tiles and the screen footer say so.

  Motion: none.
-->
<script lang="ts">
  import type { Component } from "svelte";
  import {
    RECOMMENDED_ENGINE,
    buildEngines,
    downloadLabel,
    engineDelta,
    memoryLabel,
    trackWidth,
    type ProviderSource,
  } from "$lib/onboarding/transcription-engines";
  import IconChip from "~icons/lucide/cpu";
  import IconFeather from "~icons/lucide/feather";
  import IconWave from "~icons/lucide/audio-waveform";

  let {
    providers,
    value,
    onValueChange,
  }: {
    /** `transcriptionModelStatus?.providers ?? []` — Deepgram may be present; it is filtered out. */
    providers: readonly ProviderSource[];
    /** The draft transcription provider id. */
    value: string;
    onValueChange: (provider: string) => void;
  } = $props();

  const ICONS: Record<string, Component> = {
    apple_speech_on_device: IconFeather,
    local_whisper: IconWave,
    parakeet: IconChip,
  };

  const engines = $derived(buildEngines(providers));
  const maxBytes = $derived(Math.max(0, ...engines.map((e) => e.bytes ?? 0)));
  const maxRam = $derived(Math.max(0, ...engines.map((e) => e.ramBytes ?? 0)));
  const delta = $derived(engineDelta(engines, value));
</script>

<div class="ob-pane">
  <div class="ob-head">
    <h2 class="mx-h2">Your voice never leaves this Mac.</h2>
    <p class="ob-fine">
      Every engine below transcribes on this machine. Once an engine's model is on disk, it keeps
      working with the network off.
    </p>
  </div>

  <div class="ob-pane__head">
    <span class="mx-kicker">Turning speech into text</span>
    <span class="ob-fine">one is picked for you</span>
  </div>

  <div class="mx-choice ob-engines" role="radiogroup" aria-label="Transcription engine">
    {#each engines as engine (engine.id)}
      {@const Icon = ICONS[engine.id]}
      <label class="mx-choice__opt">
        <input
          type="radio"
          name="ob-engine"
          value={engine.id}
          checked={engine.id === value}
          onchange={() => onValueChange(engine.id)}
        />
        {#if Icon}<span class="mx-choice__icon" aria-hidden="true"><Icon /></span>{/if}
        <span class="mx-choice__title">
          {engine.name}
          {#if engine.id === RECOMMENDED_ENGINE}<span class="mx-choice__tag">recommended</span>{/if}
        </span>
        <span class="mx-choice__desc">{engine.line}</span>
        {#if engine.foot}<span class="ob-fine foot">{engine.foot}</span>{/if}
        <span class="ob-met">
          <span>download<b>{downloadLabel(engine)}</b></span>
          <i><b style:width="{trackWidth(engine.bytes, maxBytes)}%"></b></i>
        </span>
        <span class="ob-met">
          <span>memory<b>{memoryLabel(engine)}</b></span>
          <i class="est">
            <b style:width="{trackWidth(engine.ramBytes, maxRam)}%"></b>
          </i>
        </span>
      </label>
    {/each}
  </div>

  <div class="ob-acts">
    <span class="ob-fine" class:up={delta.up} aria-live="polite">{delta.text}</span>
    {#if value !== RECOMMENDED_ENGINE}
      <button class="mx-btn mx-btn--sm" type="button" onclick={() => onValueChange(RECOMMENDED_ENGINE)}>
        Use recommended
      </button>
    {/if}
  </div>

  <p class="ob-fine axn">
    Memory figures are measured on a running app, not published in any manifest — they always
    read “about”. The one ordering the manifest states is memory: Parakeet uses the most.
  </p>

  <details class="cloud">
    <summary>Where's the cloud option?</summary>
    <p>
      <b>Deepgram is real, and it is left out of setup on purpose.</b> Choosing it uploads your
      microphone <i>and</i> system-audio recordings to your own Deepgram account — so it is
      switched on in Settings → Transcription, behind a consent step, and never here.
    </p>
  </details>
</div>

<style>
  .ob-pane {
    display: grid;
    gap: var(--s-4);
  }
  .ob-pane__head {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
    margin-bottom: calc(var(--s-2) - var(--s-4));
  }
  .ob-engines .mx-choice__opt {
    gap: 6px;
  }
  .foot {
    font-size: var(--text-sm);
  }
  .ob-met {
    display: grid;
    gap: 4px;
    margin-top: 4px;
  }
  .ob-met span {
    display: flex;
    justify-content: space-between;
    font: 400 var(--text-sm) / 1 var(--font-mono);
    color: var(--app-text-subtle);
  }
  .ob-met span b {
    font-weight: 500;
    color: var(--app-text);
  }
  .ob-met i {
    display: block;
    height: 3px;
    border-radius: 2px;
    background: var(--app-surface-hover);
    overflow: hidden;
  }
  .ob-met i b {
    display: block;
    height: 100%;
    background: var(--chart-grey-4);
  }
  /* Memory is measured on a running app, never published: always "about". */
  .ob-met i.est b {
    background: repeating-linear-gradient(90deg, var(--chart-grey-4) 0 3px, transparent 3px 5px);
  }
  .ob-acts .ob-fine {
    margin-left: var(--s-2);
  }
  .ob-fine.up {
    color: var(--app-warn);
  }
  .axn {
    font-size: var(--text-sm);
    max-width: 62ch;
  }
  .cloud {
    font: 400 var(--text-sm) / 1.5 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .cloud summary {
    width: fit-content;
    cursor: pointer;
  }
  .cloud summary:hover {
    color: var(--app-text);
  }
  .cloud summary:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
    border-radius: var(--r-sm);
  }
  .cloud p {
    margin: var(--s-2) 0 0;
    max-width: 62ch;
  }
  .cloud b {
    font-weight: 600;
    color: var(--app-text);
  }
</style>
