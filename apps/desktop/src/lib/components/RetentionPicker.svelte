<script lang="ts">
  // ══ INSTRUMENT 2 of 6 — RETENTION LADDER ══════════════════════════════════
  //
  // Direction 05 "Tactile Instruments". This setting earns an instrument
  // because it passes the rule: the physical quantity is *how far back your
  // recording reaches*, and the consequence can be written as a fraction of
  // something real — the gigabytes that stay on disk at your measured capture
  // rate, and the days that get deleted.
  //
  // Anatomy: header (name + live value) → WELL (the ladder of stops) → the
  // survive/cull bar → READOUT (the sentence, with its denominator).
  //
  // THE AXIS IS TIME, and it runs the way time runs: old on the LEFT (the
  // hatched, danger-tinted band that gets culled), kept on the RIGHT. A
  // retention control that draws "kept" on the left is telling the user their
  // oldest recording is the safest one, which is the exact opposite of true.
  //
  // G8 governs the numbers: they come from `get_system_facts` via
  // `system-facts.ts`. With no measured capture day the sentence and the bar's
  // labels simply do not render — a missing denominator is the designed
  // outcome, never a zero and never a guess.
  //
  // The API is unchanged from the picker this replaced: same bindable
  // `RetentionPolicy`, same `onValueChange`, same radiogroup semantics and
  // roving-tabindex keyboard model.

  import { tick } from "svelte";
  import type { RetentionPolicy } from "$lib/types";
  import { retentionPresets, retentionToDays } from "./retention";
  import { systemFacts } from "$lib/settings/state/system-facts.svelte";
  import { coarseRuntime } from "$lib/settings/state/system-facts";
  import { formatBytes } from "$lib/settings/state/format";

  interface Props {
    /** The selected retention policy (bindable). */
    value: RetentionPolicy;
    /** Called whenever a different preset is chosen. */
    onValueChange?: (v: RetentionPolicy) => void;
    /** Disables the whole picker. */
    disabled?: boolean;
    /** Optional aria-label for the group container. */
    ariaLabel?: string;
  }

  let {
    value = $bindable(),
    onValueChange,
    disabled = false,
    ariaLabel = "Retention duration",
  }: Props = $props();

  const presets = retentionPresets();

  $effect(() => {
    void systemFacts.ensureLoaded();
  });
  const facts = $derived(systemFacts.value);

  const days = $derived(retentionToDays(value));
  const perDay = $derived(facts?.measuredBytesPerDay ?? null);
  const kept = $derived(days !== null && perDay !== null ? perDay * days : null);

  // How much of the bar survives. "Forever" culls nothing, so the whole bar is
  // the keep band; a bounded window shows the culled tail as a fixed 36% so the
  // hatching is always legible — the bar is a diagram of the rule, not a
  // to-scale chart of a history whose true length nothing measures.
  const cullPct = $derived(days === null ? 0 : 36);

  const runtime = $derived(coarseRuntime(facts?.diskFreeBytes ?? null, perDay));

  // Per-stop button refs, so keyboard nav can move DOM focus onto the newly
  // active stop (focus-follows-selection — the roving tabindex alone leaves
  // focus stranded on the now tabindex=-1 button).
  let stopEls = $state<(HTMLButtonElement | null)[]>([]);

  function select(next: RetentionPolicy) {
    if (disabled || next === value) return;
    value = next;
    onValueChange?.(next);
  }

  // Click handler: select, then pull DOM focus onto the clicked stop. The Tauri
  // WKWebView doesn't focus a <button> on click, so without this the roving
  // tabindex has no anchor and a follow-up arrow key does nothing.
  function selectByClick(index: number) {
    select(presets[index].value);
    stopEls[index]?.focus();
  }

  // After a keyboard selection, move focus to the new stop — but only when
  // focus is already inside this instrument, so we never steal focus on mount
  // or on a programmatic value change.
  function focusSelected(index: number) {
    const group = stopEls[index]?.closest(".retention-instr");
    if (group && group.contains(document.activeElement)) {
      stopEls[index]?.focus();
    }
  }

  function onKeydown(event: KeyboardEvent, index: number) {
    if (disabled) return;
    let nextIndex: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      nextIndex = (index + 1) % presets.length;
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      nextIndex = (index - 1 + presets.length) % presets.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = presets.length - 1;
    }
    if (nextIndex === null) return;
    const target = nextIndex;
    event.preventDefault();
    select(presets[target].value);
    // Selecting flips the roving tabindex to `target`; follow it with DOM focus
    // so the new stop is what the user is actually on. tick() lets the
    // tabindex/active classes update first.
    void tick().then(() => focusSelected(target));
  }
</script>

<div class="ti-instr ti-instr--bare retention-instr" class:is-disabled={disabled}>
  <div class="ti-instr__hd">
    <span class="ti-instr__name">Keep recordings for</span>
    <span class="ti-instr__sub">what survives, and what is culled</span>
    <span class="ti-instr__v">
      {#if days === null}∞<em>forever</em>{:else}{days}<em>days</em>{/if}
    </span>
  </div>

  <div class="ti-well ti-ladder" role="radiogroup" aria-label={ariaLabel}>
    <div class="ti-ladder__stops">
      {#each presets as preset, index (preset.value)}
        {@const active = value === preset.value}
        <button
          type="button"
          bind:this={stopEls[index]}
          class="ti-ladder__s"
          class:is-on={active}
          role="radio"
          aria-checked={active}
          aria-label={preset.label}
          tabindex={active || (!presets.some((p) => p.value === value) && index === 0) ? 0 : -1}
          {disabled}
          onclick={() => selectByClick(index)}
          onkeydown={(e) => onKeydown(e, index)}
        >
          {preset.label}
        </button>
      {/each}
    </div>
  </div>

  <!-- The rule, drawn. Old on the left gets culled; recent on the right is
       kept. Aria-hidden: the readout below says the same thing in words, and
       a screen reader does not need the diagram read out as two labels. -->
  <div class="ti-survive" aria-hidden="true">
    <div class="ti-survive__bar">
      {#if cullPct > 0}
        <span class="ti-survive__cull" style:flex="0 0 {cullPct}%">
          <span class="ti-survive__lbl">deleted</span>
        </span>
      {/if}
      <span class="ti-survive__keep">
        <span class="ti-survive__lbl">
          {#if days === null}everything kept{:else}last {days} days{/if}
        </span>
      </span>
    </div>
    <div class="ti-survive__scale">
      <span>oldest</span>
      <span>now</span>
    </div>
  </div>

  <div class="ti-instr__out">
    {#if perDay === null}
      Mnema has not measured a full day of capture yet, so it will not guess how
      much this window keeps. The figure appears once there is a real day to
      divide.
    {:else if kept !== null}
      Keeps about <b>{formatBytes(kept)}</b> on disk at your measured rate of
      <b>{formatBytes(perDay)}</b> a day. Anything older is deleted permanently —
      including its text and audio.
    {:else}
      Nothing is deleted{#if runtime} — at your measured rate of <b>{formatBytes(perDay)}</b>
        a day, the free space lasts {runtime}{/if}.
    {/if}
  </div>
</div>

<style>
  .retention-instr {
    width: 100%;
    min-width: 0;
  }

  .retention-instr.is-disabled {
    opacity: var(--opacity-disabled);
    pointer-events: none;
  }
</style>
