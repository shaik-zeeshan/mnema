<script lang="ts">
  // ══ INSTRUMENT 4 of 6 — OCR DUTY CYCLE ════════════════════════════════════
  //
  // Direction 05 "Tactile Instruments". Reading the screen is the hottest thing
  // Mnema does, and this passes the instrument rule: the physical quantity is
  // *how much of each minute the OCR engine is allowed to run*, and the
  // consequence is a fraction of something real — the share of wall-clock time
  // spent working, against the backlog that share has to drain.
  //
  // Anatomy, shared with the other five: header (name + live value) → WELL
  // (both halves of the cycle) → READOUT (the consequence, with a denominator).
  //
  // ── Two things this instrument deliberately does NOT say ────────────────
  //  1. **No temperature.** G8 deleted every °C claim app-wide, and there is no
  //     thermal fact in `SystemFacts` to derive one from. The mockup printed
  //     "+3 °C sustained"; that is the exact pixel G8 amended.
  //  2. **No ETA, coarse or otherwise.** An ETA needs throughput, and nothing
  //     reachable from Settings measures OCR throughput — `get_system_facts`
  //     returns the backlog *count* and no run duration. Under G8 a fact that
  //     is null renders no number, so the backlog states its size and stops.
  //     (`semanticIndexPrice` is silent for exactly the same reason.)
  //
  // ── Why this instrument reads and never turns ───────────────────────────
  // The pacing is a pair of build-time constants in `ocr_budget.rs`, not a
  // setting, so there is no knob to draw — the same posture as Overview's two
  // read-only readouts. The bar is a face on a fixed cycle, not a control.

  import { systemFacts } from "$lib/settings/state/system-facts.svelte";
  import { backlogPhrase } from "$lib/settings/state/system-facts";

  interface Props {
    /** Dim the face when OCR is switched off — the cycle still exists, but it
        is not running, and a live-looking meter would be a lie. */
    idle?: boolean;
  }
  let { idle = false }: Props = $props();

  // Mirrors `OCR_ACTIVE_RECORDING_COOLDOWN_MULTIPLIER` / `OCR_CATCH_UP_COOLDOWN_MULTIPLIER`
  // in `apps/desktop/src-tauri/src/ocr_budget.rs`: the engine sleeps for a
  // multiple of each job's own run time, so the duty cycle is 1 / (1 + m)
  // whatever a single frame costs. 4.0 → 20 % while recording, 1.5 → 40 % when
  // capture is off or paused (paused is the state a user picks to cool the
  // machine down, so it drains harder). Not read over IPC on purpose: they are
  // compile-time constants, and the debug command that exposes the live pacing
  // ships a 256-event ring — far too much payload for a settings face.
  const RECORDING_COOLDOWN_MULTIPLIER = 4.0;
  const PAUSED_COOLDOWN_MULTIPLIER = 1.5;
  const dutyPercent = (multiplier: number) => Math.round(100 / (1 + multiplier));

  const recordingDuty = dutyPercent(RECORDING_COOLDOWN_MULTIPLIER);
  const pausedDuty = dutyPercent(PAUSED_COOLDOWN_MULTIPLIER);

  void systemFacts.ensureLoaded();
  const backlog = $derived(
    backlogPhrase(systemFacts.value?.ocrBacklog ?? null, "captured frame"),
  );
  // Zero is a real measurement and says "Nothing waiting." — but there is then
  // no queue to disclaim an ETA for.
  const queued = $derived((systemFacts.value?.ocrBacklog ?? 0) > 0);

  const bands = $derived([
    { label: "While recording", duty: recordingDuty },
    { label: "While paused", duty: pausedDuty },
  ]);
</script>

<div class="ti-instr ti-instr--bare duty-instr" class:is-idle={idle}>
  <div class="ti-instr__hd">
    <span class="ti-instr__name">Processing pace</span>
    <span class="ti-instr__sub">work and cooldown, drawn as one bar</span>
    <span class="ti-instr__v">{recordingDuty} / {pausedDuty}<em>% duty</em></span>
  </div>

  <!-- BOTH halves of the cycle. A duty cycle you can only see one end of is a
       lie: the cooldown is the larger half and the reason the fans stay off. -->
  <div class="ti-well ti-duty">
    {#each bands as band (band.label)}
      <div class="ti-duty__leg">
        <span class="t-label duty-instr__leg">{band.label}</span>
        <span class="ti-duty__bar">
          <!-- No grip: the grip is a drag handle, and this face does not turn.
               Drawing one on a fixed cycle would promise a control that the
               app does not have. -->
          <span class="ti-duty__work" style:width="{band.duty}%">{band.duty} % work</span>
          <span class="ti-duty__cool">{100 - band.duty} % cooldown</span>
        </span>
      </div>
    {/each}
  </div>

  <div class="ti-instr__out">
    <p>
      For every second spent reading the screen, Mnema waits
      <b>{RECORDING_COOLDOWN_MULTIPLIER}</b> while you record and
      <b>{PAUSED_COOLDOWN_MULTIPLIER}</b> once capture stops — so a paused Mac
      drains the queue about twice as fast as a recording one.
    </p>
    {#if backlog}
      <!-- The backlog is the denominator this pace has to answer for. No ETA
           beside it, coarse or otherwise: nothing measures the throughput that
           would turn a count into a time (G8). -->
      <p>
        {backlog}{#if queued} Mnema does not time the queue, so it will not
          guess when it empties.{/if}
      </p>
    {/if}
  </div>
</div>

<style>
  .duty-instr {
    width: 100%;
    min-width: 0;
  }

  /* OCR off: the cycle is still true, it is simply not running. */
  .duty-instr.is-idle {
    opacity: 0.45;
  }

  /* One fixed gutter so both bars start on the same x and the two work
     segments can be compared by eye — the whole point of drawing them
     together. */
  .duty-instr__leg {
    width: 96px;
    flex: 0 0 auto;
  }

  .duty-instr :global(.ti-duty__bar) {
    flex: 1 1 auto;
  }

  .duty-instr .ti-instr__out p {
    margin: 0;
  }

  .duty-instr .ti-instr__out p + p {
    margin-top: var(--s-4);
  }
</style>
