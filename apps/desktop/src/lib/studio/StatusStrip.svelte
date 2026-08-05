<script lang="ts">
  // The fourth fixed piece of the Studio Shell: a 24px strip welded to the
  // bottom window edge. Because it is structural — outside every scroll region,
  // pinned to the window frame — capture state, disk cost and save state cannot
  // clip. That is the whole trade the direction pays 24px for.
  //
  // Every number here is a measured fact or it is absent (DECISIONS.md **G8**):
  // no projected pace until a capture day has actually been measured, no
  // temperature, no minute-precise ETA. `systemFacts` returns `null` fields when
  // the backend could not measure, and each group below renders nothing rather
  // than a zero or a placeholder.

  import { captureControls } from "$lib/capture-controls.svelte";
  import { formatBytes } from "$lib/settings/state/format";
  import { systemFacts } from "$lib/settings/state/system-facts.svelte";
  import { statusSave } from "./status-strip.svelte";

  // One load per app run; the figures move on the scale of a capture day.
  $effect(() => {
    void systemFacts.ensureLoaded();
  });

  const facts = $derived(systemFacts.value);

  const captureState = $derived(
    captureControls.isLowDiskSuspended
      ? { label: "Low disk", tone: "warn" as const }
      : captureControls.paused
        ? { label: "Paused", tone: "warn" as const }
        : captureControls.isCapturing
          ? { label: "Recording", tone: "live" as const }
          : { label: "Not recording", tone: "off" as const },
  );

  const rate = $derived(facts?.screenFrameRate ?? null);

  // The measured average, named for what it is. `measuredDays` is part of the
  // claim: a one-day average is not a week's.
  const perDay = $derived(facts?.measuredBytesPerDay ?? null);

  // Monthly pace is the measured average × 30 — a projection off a real
  // measurement, which G8 allows; without the measurement it is a guess, which
  // it does not.
  const perMonth = $derived(perDay === null ? null : perDay * 30);

  // One queue figure, summed across the two backlogs that actually exist.
  const queued = $derived(
    facts === null || (facts.ocrBacklog === null && facts.transcriptionBacklog === null)
      ? null
      : (facts.ocrBacklog ?? 0) + (facts.transcriptionBacklog ?? 0),
  );

  const save = $derived(statusSave.value);
</script>

<footer class="ss-sstrip" aria-label="Capture status">
  <span class="ss-sstrip__g">
    <span class="dot dot--{captureState.tone}" aria-hidden="true"></span>
    <b>{captureState.label}</b>
  </span>

  {#if rate !== null}
    <span class="ss-sstrip__dot" aria-hidden="true"></span>
    <span class="ss-sstrip__g"><span class="ss-num">{rate}</span>&nbsp;fps</span>
  {/if}

  {#if perDay !== null}
    <span class="ss-sstrip__dot" aria-hidden="true"></span>
    <span class="ss-sstrip__g"><span class="ss-num">{formatBytes(perDay)}</span>&nbsp;/day</span>
    <span class="ss-sstrip__dot" aria-hidden="true"></span>
    <span class="ss-sstrip__g">~<span class="ss-num">{formatBytes(perMonth ?? 0)}</span>&nbsp;/month</span>
  {/if}

  {#if queued !== null && queued > 0}
    <span class="ss-sstrip__dot" aria-hidden="true"></span>
    <span class="ss-sstrip__g"><span class="ss-num">{queued}</span>&nbsp;queued</span>
  {/if}

  {#if facts?.diskFreeBytes != null}
    <span class="ss-sstrip__dot" aria-hidden="true"></span>
    <span class="ss-sstrip__g ss-sstrip__g--shrink">
      <span class="ss-num">{formatBytes(facts.diskFreeBytes)}</span>&nbsp;free
    </span>
  {/if}

  <span class="ss-sstrip__spacer"></span>

  {#if save}
    <span
      class="ss-save"
      class:ss-save--busy={save.tone === "busy"}
      class:ss-save--bad={save.tone === "bad"}
      role="status"
      aria-live="polite"
    >
      {save.label}
    </span>
  {/if}
</footer>

<style>
  /* The capture dot is the strip's only colour at rest. */
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-text-faint);
    flex: 0 0 auto;
  }
  .dot--live {
    background: var(--app-record);
    box-shadow: 0 0 0 2.5px color-mix(in srgb, var(--app-record) 22%, transparent);
  }
  .dot--warn {
    background: var(--app-warn);
  }
</style>
