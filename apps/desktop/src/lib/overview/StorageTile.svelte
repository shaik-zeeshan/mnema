<script lang="ts">
  // Storage (1×1). Round-4 decision G8 rewrote this tile: every figure comes
  // from `get_system_facts`, the command built to answer "is this number real on
  // THIS machine", and each line renders only when its value is non-null.
  //
  // DEVIATIONS from the mockup, all forced by G8:
  //   · "270 MB today"  — no per-day byte total exists; the honest equivalent is
  //     the measured daily average, which SystemFacts actually carries.
  //   · "34.2 GB of history on disk" — nothing measures the capture directory's
  //     total size, so the line is dropped rather than guessed. Free space is
  //     real, and is what a retention decision actually turns on.
  //   · "90-day keep" — the retention ladder tops out at 30 days.
  import { captureControls } from "$lib/capture-controls.svelte";
  import { retentionLabel } from "$lib/components/retention";
  import type { SystemFacts } from "$lib/types/system-facts";
  import { bytesLabel } from "./overview-format";

  let { facts, loaded }: { facts: SystemFacts | null; loaded: boolean } = $props();

  const perDay = $derived(bytesLabel(facts?.measuredBytesPerDay));
  const perMonth = $derived(
    facts?.measuredBytesPerDay != null ? bytesLabel(facts.measuredBytesPerDay * 30) : null,
  );
  const free = $derived(bytesLabel(facts?.diskFreeBytes));
  const keep = $derived(
    captureControls.recordingSettings
      ? retentionLabel(captureControls.recordingSettings.retentionPolicy)
      : null,
  );
  const measured = $derived(facts?.measuredDays ?? 0);
</script>

<div class="tile tile--static">
  <div class="tile__h">
    <span class="t-label">Storage</span>
    {#if keep}<span class="tile__more">{keep}</span>{/if}
  </div>

  {#if perDay}
    <div class="trow"><span class="t-ui strong is-num">≈ {perDay} / day</span></div>
    <div class="trow">
      <span class="t-meta is-num">
        measured over {measured}
        {measured === 1 ? "day" : "days"} of capture
      </span>
    </div>
    {#if perMonth}
      <div class="trow projection">
        <span class="t-meta subtle is-num">≈ {perMonth} / month at that pace</span>
      </div>
    {/if}
  {:else if free}
    <div class="trow"><span class="t-ui strong is-num">{free} free</span></div>
    <div class="trow">
      <span class="t-meta">{loaded ? "no full day of capture measured yet" : "Reading disk…"}</span>
    </div>
  {:else}
    <div class="pay quiet"><span class="t-meta subtle">Disk figures unavailable</span></div>
  {/if}

  {#if perDay && free}
    <div class="trow"><span class="t-meta subtle is-num">{free} free on disk</span></div>
  {/if}
</div>

<style>
  .strong {
    font-weight: var(--w-semi);
    color: var(--app-text-strong);
  }
  .subtle {
    color: var(--app-text-subtle);
  }
  .quiet {
    display: flex;
    align-items: center;
  }
  /* First thing to go at the 800px floor — the mockup's stated drop order. */
  @media (max-width: 900px) {
    .projection {
      display: none;
    }
  }
</style>
