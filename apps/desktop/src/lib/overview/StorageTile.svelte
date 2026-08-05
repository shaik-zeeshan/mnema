<script lang="ts">
  // What capture costs, in the figures this machine can actually measure.
  //
  // Two mockup numbers are deliberately absent (round-4 decision **G8**):
  //  * "270 MB today" — nothing measures bytes written *today*; `SystemFacts`
  //    carries a measured multi-day average, which is what is quoted here, named
  //    for what it is.
  //  * the 34% fill gauge — a fraction needs a denominator, and there is no
  //    total-disk figure (only free space). A bar drawn against an invented
  //    total is exactly what G8 forbids, so there is no bar.
  //
  // The semantic coverage meter is gated on semantic search being enabled
  // (**G10**; the shipping default is off, so it usually renders nothing).
  import { captureControls } from "$lib/capture-controls.svelte";
  import { formatBytes } from "$lib/settings/state/format";
  import { systemFacts } from "$lib/settings/state/system-facts.svelte";
  import { semanticCoverage } from "$lib/settings/state/system-facts";
  import { retentionPresets } from "$lib/components/retention";
  import TileShell from "./TileShell.svelte";

  // Idempotent — the status strip already warmed this singleton.
  $effect(() => {
    void systemFacts.ensureLoaded();
  });

  const facts = $derived(systemFacts.value);
  const settings = $derived(captureControls.recordingSettings);

  const perDay = $derived(facts?.measuredBytesPerDay ?? null);
  const perMonth = $derived(perDay === null ? null : perDay * 30);
  const measuredFor = $derived(
    facts === null || perDay === null
      ? null
      : `measured over ${facts.measuredDays} ${facts.measuredDays === 1 ? "day" : "days"}`,
  );

  const retentionLabel = $derived(
    settings ? (retentionPresets().find((p) => p.value === settings.retentionPolicy)?.label ?? null) : null,
  );

  const semantic = $derived(settings?.semanticSearch.enabled ? semanticCoverage(facts) : null);

  const hasAnything = $derived(
    perDay !== null || facts?.diskFreeBytes != null || retentionLabel !== null,
  );
</script>

<TileShell
  label="Storage"
  quiet={hasAnything ? null : "No storage figures measured yet."}
>
  {#if perDay !== null}
    <div class="ss-trow">
      <span class="t-ui strong is-num">{formatBytes(perDay)}</span>
      <span class="t-meta">a day</span>
    </div>
    {#if perMonth !== null}
      <div class="ss-trow">
        <span class="t-meta is-mono is-num">~ {formatBytes(perMonth)} /month</span>
      </div>
    {/if}
  {/if}

  {#if retentionLabel}
    <div class="ss-trow">
      <span class="t-meta">
        keep {retentionLabel.toLowerCase()}{#if facts?.diskFreeBytes != null}{" · "}{formatBytes(facts.diskFreeBytes)} free{/if}
      </span>
    </div>
  {:else if facts?.diskFreeBytes != null}
    <div class="ss-trow"><span class="t-meta">{formatBytes(facts.diskFreeBytes)} free</span></div>
  {/if}

  <!-- `retentionConsequence` is a settings-row sentence — too long for a tile,
       and the retention line above already states the window. The tile keeps the
       shorter honest clause: how many days the average was measured over. -->
  {#if measuredFor}
    <div class="ss-trow"><span class="t-meta sub">{measuredFor}</span></div>
  {/if}

  {#if semantic}
    <div class="meter">
      <span class="meter__b"><i style="width:{semantic.percent}%"></i></span>
      <span class="t-meta is-mono is-num">{semantic.percent}%</span>
    </div>
  {/if}
</TileShell>

<style>
  .strong {
    color: var(--app-text-strong);
  }

  .sub {
    color: var(--app-text-subtle);
  }

  .meter {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    margin-top: 2px;
  }

  .meter__b {
    position: relative;
    height: 6px;
    border-radius: 3px;
    flex: 1 1 auto;
    overflow: hidden;
    background: var(--app-surface-hover);
  }

  .meter__b i {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--app-accent);
  }
</style>
