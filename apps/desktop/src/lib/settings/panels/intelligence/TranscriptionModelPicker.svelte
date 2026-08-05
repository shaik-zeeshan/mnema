<script lang="ts">
  // ══ INSTRUMENT 6 of 6 — MODEL PICKER ══════════════════════════════════════
  //
  // Direction 05 "Tactile Instruments". This passes the instrument rule where a
  // dropdown never could: the physical quantity is *how much of this Mac a
  // model claims*, and the consequence is a fraction of something real — the
  // model's weights against the RAM actually installed here. "large-v3" means
  // nothing; "3.1 GB of 16 GB while you are recording" means everything.
  //
  // Anatomy: header (name + live value) → WELL (the rows) → READOUT (the
  // sentence, with its denominator). The rows ARE the control, so the well is
  // the row stack rather than a track.
  //
  // G8 governs every number: sizes come from the model manifest the backend
  // reports (`download.byteSize`), the denominators come from
  // `get_system_facts`, and a model with nothing local to weigh — a cloud
  // provider, an OS-managed model — renders NO footprint rather than a zero.
  // Nothing here is the mockup's fiction; the mockup's own figures (620 MB,
  // 1.6 GB, 3.1 GB) never appear as literals.
  //
  // Behaviour is unchanged from the Combobox this replaces: the same
  // `__os_managed__` sentinel goes out on select, and picking a model is still
  // the only thing selection does.

  import type { AudioTranscriptionModelStatus } from "$lib/types";
  import { transcriptionStatusLabel } from "$lib/settings/state/models-format";
  import { formatBytes } from "$lib/settings/state/format";
  import { systemFacts } from "$lib/settings/state/system-facts.svelte";
  import { modelFit } from "./model-fit";

  interface Props {
    models: AudioTranscriptionModelStatus[];
    /** `rec.draftTranscriptionModelId` — null means the OS-managed sentinel. */
    selectedModelId: string | null;
    /** Receives the same wire value the Combobox sent: id, or `__os_managed__`. */
    onselect: (value: string) => void;
    disabled?: boolean;
  }

  let { models, selectedModelId, onselect, disabled = false }: Props = $props();

  void systemFacts.ensureLoaded();
  const facts = $derived(systemFacts.value);

  const rowValue = (model: AudioTranscriptionModelStatus) => model.modelId ?? "__os_managed__";

  // Same resolution the controller's `selectedTranscriptionModel` uses, so the
  // highlighted row is always the row whose status block is shown below.
  const activeIndex = $derived(
    Math.max(0, models.findIndex((model) => model.modelId === selectedModelId)),
  );

  // Cloud is a provider property, not a category (ADR 0047): Deepgram claims no
  // memory here, so it gets no footprint bar — it gets the fact that matters
  // about it instead. The consent gate lives on the provider switch and is
  // untouched by this control.
  const isCloud = (model: AudioTranscriptionModelStatus) => model.provider === "deepgram";

  const fitOf = (model: AudioTranscriptionModelStatus) =>
    isCloud(model)
      ? null
      : modelFit(model.download?.byteSize ?? null, facts?.totalRamBytes, facts?.diskFreeBytes);

  function select(index: number) {
    const model = models[index];
    if (disabled || !model) return;
    onselect(rowValue(model));
  }

  function onKeydown(event: KeyboardEvent, index: number) {
    let next: number | null = null;
    if (event.key === "ArrowDown" || event.key === "ArrowRight") next = index + 1;
    else if (event.key === "ArrowUp" || event.key === "ArrowLeft") next = index - 1;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = models.length - 1;
    if (next === null) return;
    event.preventDefault();
    const target = Math.min(Math.max(next, 0), models.length - 1);
    select(target);
    document.getElementById(`transcription-model-${target}`)?.focus();
  }

  const activeModel = $derived(models[activeIndex] ?? null);
  const activeFit = $derived(activeModel ? fitOf(activeModel) : null);
  const activeSize = $derived(activeModel?.download?.byteSize ?? null);
</script>

<div class="ti-instr ti-instr--bare model-instr">
  <div class="ti-instr__hd">
    <!-- The header names the live selection, not the control: an instrument
         outranks a row by carrying a value, and the row's own label ("Model")
         already says what this is. -->
    <span class="ti-instr__name">{activeModel?.displayName ?? "No model"}</span>
    <span class="ti-instr__sub">what it claims of this Mac</span>
    {#if activeSize !== null && facts?.totalRamBytes != null}
      <span class="ti-instr__v">
        {formatBytes(activeSize)}<em>of {formatBytes(facts.totalRamBytes)}</em>
      </span>
    {/if}
  </div>

  <div class="ti-well model-instr__well" role="radiogroup" aria-label="Transcription model">
    {#each models as model, i (rowValue(model))}
      {@const fit = fitOf(model)}
      <button
        type="button"
        id="transcription-model-{i}"
        class="ti-mrow"
        class:is-sel={i === activeIndex}
        role="radio"
        aria-checked={i === activeIndex}
        {disabled}
        tabindex={i === activeIndex ? 0 : -1}
        onclick={() => select(i)}
        onkeydown={(e) => onKeydown(e, i)}
      >
        <span class="ti-grow__txt">
          <span class="ti-grow__lbl">{model.displayName}</span>
          <span class="ti-grow__sub">
            {isCloud(model) ? "Cloud" : "On-device"}
            {#if model.download} · {formatBytes(model.download.byteSize)} on disk{/if}
            · {transcriptionStatusLabel(model)}
          </span>
        </span>

        <!-- The footprint bar. Drawn only when this Mac can answer the
             question: no RAM figure, no bar (G8). -->
        {#if fit?.ramPercent !== null && fit?.ramPercent !== undefined && facts?.totalRamBytes != null && model.download}
          <span class="ti-ramfoot">
            <span class="ti-ramfoot__t">
              <i
                class:warn={fit.tone === "warn"}
                class:dan={fit.tone === "danger"}
                style:width="{Math.max(2, fit.ramPercent)}%"
              ></i>
            </span>
            <span class="ti-mrow__num model-instr__num">
              {formatBytes(model.download.byteSize)} of {formatBytes(facts.totalRamBytes)}
            </span>
          </span>
        {/if}

        <!-- The verdict, not the number, is this control's real output. It is
             also the only semantic colour on the page. -->
        {#if isCloud(model)}
          <span class="ti-chip ti-chip--info">uploads your audio</span>
        {:else if fit?.verdict}
          <span
            class="ti-chip"
            class:ti-chip--acc={fit.tone === "ok"}
            class:ti-chip--warn={fit.tone === "warn"}
            class:ti-chip--danger={fit.tone === "danger"}
          >{fit.verdict}</span>
        {/if}
      </button>
    {/each}
  </div>

  <div class="ti-instr__out">
    {#if activeModel && isCloud(activeModel)}
      Deepgram runs in your own cloud account, so it claims nothing on this Mac —
      the cost is that your microphone and system audio leave it.
    {:else if activeFit?.ramPercent == null || activeSize === null}
      Mnema cannot read this Mac's memory, so it will not guess what this model
      costs to run. The figure appears when there is a real limit to divide by.
    {:else}
      Weights alone claim <b>{formatBytes(activeSize)}</b> of this Mac's
      <b>{formatBytes(facts?.totalRamBytes ?? 0)}</b> while a job runs — on top of
      whatever capture is already holding. Peak use during a run is higher; Mnema
      does not measure it, so this is the floor, not the ceiling.
    {/if}
  </div>
</div>

<style>
  .model-instr {
    width: 100%;
    min-width: 0;
  }

  /* The rows ARE the control, so the recess wraps the whole stack. Its own
     radius has to clip the selected row's accent fill, or the fill squares off
     the well's corners. */
  .model-instr__well {
    overflow: hidden;
  }

  .model-instr__num {
    display: block;
    margin-top: 3px;
    white-space: nowrap;
  }

  /* macOS reports a "16 GB" Mac as 17.2 GB, so the fraction is wider than the
     shared 112px footprint slot; widen it here rather than in the shared skin,
     which the capture-side instruments also use. */
  .model-instr :global(.ti-ramfoot) {
    width: 152px;
  }

  .model-instr :global(.ti-mrow:disabled) {
    opacity: var(--opacity-disabled);
  }

  .model-instr :global(.ti-mrow:focus-visible) {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--app-accent-glow);
  }
</style>
