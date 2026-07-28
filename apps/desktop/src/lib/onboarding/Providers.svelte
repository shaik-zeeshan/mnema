<!--
  Onboarding provider choice (issue #195, slice 9).

  Ported from `docs/onboarding/mockups/input-components/parts/providers.part.html`
  — that mockup is the design of record; behaviour and copy come from it.

  What it changes about the shipping surface:
   · One engine is RECOMMENDED and says why; the other two carry their price as
     a real delta (download from the manifest, memory from the measured table in
     `transcription-engines.ts`). "Use recommended" always walks it back.
   · Speed and accuracy are drawn as EMPTY hatched tracks. Nobody has
     benchmarked these three against each other in this build, so an ordering
     here would be the most load-bearing lie on the screen. Do not fill them.
   · Deepgram is named in a disclosure instead of shown as a disabled radio.
     ADR 0047 is unchanged: cloud transcription is Settings-only behind a
     consent gate, and it is still filtered out of the list this component gets.
   · OCR is a resolved read-only line, not a choice. Apple Vision has no
     download, no extra memory, and no axis on which Tesseract wins in this
     build. The one defensible difference — Tesseract ships English-only — is
     stated, and the choice itself stays in Settings.

  Motion: none ambient. Track widths transition only when the user picks.
-->
<script lang="ts">
  import { tick } from "svelte";
  import { formatBytes } from "$lib/settings/state/format";
  import {
    RECOMMENDED_ENGINE,
    buildEngines,
    downloadLabel,
    engineDelta,
    memoryLabel,
    ocrResolvedLine,
    trackWidth,
    type ProviderSource,
  } from "$lib/onboarding/transcription-engines";

  let {
    providers,
    value,
    onValueChange,
    ocrProviders,
    ocrProvider,
  }: {
    /** `transcriptionModelStatus?.providers ?? []` — Deepgram may be present; it is filtered out. */
    providers: readonly ProviderSource[];
    /** The draft transcription provider id. */
    value: string;
    onValueChange: (provider: string) => void;
    /** `ocrModelStatus?.providers ?? []`. */
    ocrProviders: readonly ProviderSource[];
    /** The draft OCR provider id — read only, this component never changes it. */
    ocrProvider: string;
  } = $props();

  const engines = $derived(buildEngines(providers));
  const maxBytes = $derived(Math.max(0, ...engines.map((e) => e.bytes ?? 0)));
  const maxRam = $derived(Math.max(0, ...engines.map((e) => e.ramBytes ?? 0)));
  const delta = $derived(engineDelta(engines, value));
  const ocr = $derived(ocrResolvedLine(ocrProviders, ocrProvider));

  let cards = $state<HTMLDivElement | null>(null);

  // Arrow keys move the selection, as a radio group does. Selection lives with
  // the caller, so the focus follows the value on the next tick.
  async function step(direction: number): Promise<void> {
    const at = engines.findIndex((e) => e.id === value);
    const next = engines[(Math.max(at, 0) + direction + engines.length) % engines.length];
    if (!next) return;
    onValueChange(next.id);
    await tick();
    cards?.querySelector<HTMLButtonElement>(`[data-v="${next.id}"]`)?.focus();
  }

  function onKeydown(event: KeyboardEvent): void {
    const direction = { ArrowLeft: -1, ArrowUp: -1, ArrowRight: 1, ArrowDown: 1 }[event.key];
    if (!direction) return;
    event.preventDefault();
    void step(direction);
  }
</script>

<div class="pv">
  <p class="claim">Your voice never leaves this Mac.</p>
  <p class="claim-sub">
    Every engine below transcribes on this machine. Once an engine's model is on disk, it
    keeps working with the network off.
  </p>

  <div class="pv-head">
    <span class="t">Turning speech into text</span>
    <span class="ob-fine">one is picked for you</span>
  </div>

  <div
    class="cards"
    bind:this={cards}
    role="radiogroup"
    aria-label="Transcription engine"
    tabindex={-1}
    onkeydown={onKeydown}
  >
    {#each engines as engine (engine.id)}
      {@const on = engine.id === value}
      <button
        class="card"
        type="button"
        role="radio"
        data-v={engine.id}
        aria-checked={on}
        tabindex={on ? 0 : -1}
        onclick={() => onValueChange(engine.id)}
      >
        <span class="cn">
          {engine.name}
          {#if engine.id === RECOMMENDED_ENGINE}<span class="badge">recommended</span>{/if}
        </span>
        <span class="cr">{engine.line}</span>
        {#if engine.foot}<span class="cf">{engine.foot}</span>{/if}

        <span class="met">
          <span class="ml"><span>download</span><span>{downloadLabel(engine)}</span></span>
          <i class="trk"><b style:width="{trackWidth(engine.bytes, maxBytes)}%"></b></i>
        </span>
        <span class="met">
          <span class="ml"><span>memory</span><span>{memoryLabel(engine)}</span></span>
          <!-- An estimated figure never gets a solid bar; an unmeasured one gets no bar. -->
          <i class="trk" class:est={engine.ramInferred} class:none={engine.ramBytes === null}>
            <b style:width="{trackWidth(engine.ramBytes, maxRam)}%"></b>
          </i>
        </span>
      </button>
    {/each}
  </div>

  <div class="delta">
    <span class="dt" class:up={delta.up} aria-live="polite">{delta.text}</span>
    {#if value !== RECOMMENDED_ENGINE}
      <button class="ob-btn sm" type="button" onclick={() => onValueChange(RECOMMENDED_ENGINE)}>
        Use recommended
      </button>
    {/if}
  </div>

  <div class="axes">
    <span class="ax">
      <span class="ml"><span>speed</span><span>not ranked</span></span>
      <i class="trk none"></i>
    </span>
    <span class="ax">
      <span class="ml"><span>accuracy</span><span>not ranked</span></span>
      <i class="trk none"></i>
    </span>
    <p class="axn">
      <b>Not ranked.</b> These engines have never been benchmarked against each other in this
      build, so the two tracks stay empty instead of inventing a winner. The one ordering the
      manifest does state is memory: Parakeet uses the most. Memory figures are measured on a
      running app, not published in any manifest — they always read “about”.
    </p>
  </div>

  <details class="cloud">
    <summary>Where's the cloud option?</summary>
    <p>
      <b>Deepgram is real, and it is left out of setup on purpose.</b> Choosing it uploads your
      microphone <i>and</i> system-audio recordings to your own Deepgram account — so it is
      switched on in Settings → Transcription, behind a consent step, and never here.
    </p>
  </details>

  <p class="resolved">
    <span class="rk">Reading on-screen text</span>
    <span class="rv">{ocr.value}</span>
    <span class="rn">{ocr.note}</span>
  </p>
</div>

<style>
  .claim {
    margin: 0;
    font-size: var(--text-lg);
    line-height: 1.35;
    color: var(--app-text-strong);
    letter-spacing: -0.01em;
  }
  .claim-sub {
    margin: 6px 0 0;
    font-size: var(--text-sm);
    line-height: 1.6;
    color: var(--app-text-muted);
    max-width: 56ch;
  }

  .pv-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
    margin: 18px 0 8px;
    padding-top: 14px;
    border-top: 1px solid var(--app-border);
  }
  .pv-head .t {
    font-size: var(--text-md);
    color: var(--app-text-strong);
  }

  .cards {
    display: grid;
    gap: 8px;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 7px;
    text-align: left;
    font: inherit;
    cursor: pointer;
    border: 1px solid var(--app-border-strong);
    border-radius: 10px;
    background: var(--app-surface-subtle);
    padding: 10px 11px;
    color: var(--app-text-muted);
    transition:
      background 0.15s,
      border-color 0.15s;
  }
  .card:hover[aria-checked="false"] {
    background: var(--app-surface-hover);
  }
  .card[aria-checked="true"] {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }
  .card:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .card .cn {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 6px;
    font-size: var(--text-md);
    color: var(--app-text-strong);
  }
  .card[aria-checked="true"] .cn {
    color: var(--app-accent);
  }
  .badge {
    font-size: var(--text-xs);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-accent);
    white-space: nowrap;
  }
  .card .cr {
    font-size: var(--text-sm);
    line-height: 1.5;
    color: var(--app-text-subtle);
    flex: 1;
  }
  .card .cf {
    font-size: var(--text-xs);
    line-height: 1.5;
    color: var(--app-text-subtle);
  }

  .met {
    display: block;
  }
  .met + .met {
    margin-top: 5px;
  }
  .ml {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }
  .card[aria-checked="true"] .ml {
    color: var(--app-text-muted);
  }
  .axes .ml {
    color: var(--app-text-faint);
  }
  .trk {
    display: block;
    height: 4px;
    margin-top: 3px;
    border-radius: 999px;
    background: var(--app-border);
    overflow: hidden;
  }
  .trk b {
    display: block;
    height: 100%;
    border-radius: 999px;
    background: var(--app-accent-strong);
    transition: width 0.18s ease;
  }
  .card[aria-checked="true"] .trk b {
    background: var(--app-accent);
  }
  /* An estimated figure never gets a solid bar. */
  .trk.est {
    background: transparent;
    box-shadow: inset 0 0 0 1px var(--app-border);
  }
  .trk.est b {
    background: repeating-linear-gradient(
      90deg,
      var(--app-accent-strong) 0 3px,
      transparent 3px 6px
    );
  }
  .card[aria-checked="true"] .trk.est b {
    background: repeating-linear-gradient(90deg, var(--app-accent) 0 3px, transparent 3px 6px);
  }
  /* Nothing measured this — a track nobody has filled, not a broken bar. */
  .trk.none {
    background: repeating-linear-gradient(45deg, var(--app-border) 0 2px, transparent 2px 5px);
    box-shadow: inset 0 0 0 1px var(--app-border);
  }

  .delta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-top: 10px;
    min-height: 26px;
  }
  .delta .dt {
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }
  .delta .dt.up {
    color: var(--app-warn);
  }

  .axes {
    margin-top: 12px;
    padding-top: 10px;
    border-top: 1px dashed var(--app-border);
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px 12px;
  }
  .ax {
    display: block;
  }
  .axn {
    grid-column: 1 / -1;
    margin: 4px 0 0;
    font-size: var(--text-xs);
    line-height: 1.65;
    color: var(--app-text-subtle);
  }
  .axn b {
    color: var(--app-text-muted);
    font-weight: 600;
  }

  .cloud {
    margin-top: 12px;
    border: 1px dashed var(--app-border-strong);
    border-radius: 8px;
    background: var(--app-surface-subtle);
  }
  .cloud summary {
    padding: 8px 11px;
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--app-text-muted);
  }
  .cloud summary:hover {
    color: var(--app-text);
  }
  .cloud summary:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
    border-radius: 8px;
  }
  .cloud p {
    margin: 0;
    padding: 0 11px 10px;
    font-size: var(--text-sm);
    line-height: 1.6;
    color: var(--app-text-subtle);
    max-width: 62ch;
  }
  .cloud b {
    color: var(--app-text);
    font-weight: 600;
  }

  .resolved {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px 10px;
    margin: 14px 0 0;
    padding-top: 12px;
    border-top: 1px solid var(--app-border);
  }
  .resolved .rk {
    font-size: var(--text-sm);
    color: var(--app-text-muted);
  }
  .resolved .rv {
    font-size: var(--text-sm);
    color: var(--app-text-strong);
  }
  .resolved .rn {
    flex-basis: 100%;
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
  }

  @media (prefers-reduced-motion: reduce) {
    .card,
    .trk b {
      transition: none;
    }
  }
</style>
