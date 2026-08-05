<script lang="ts">
  // What capture costs, in numbers this machine actually produced (G8).
  //
  // Dropped from the mockup for want of a real source: "270 MB today" (nothing
  // measures a single day's bytes), "34.2 GB of history" (no total-media-bytes
  // read exists), and "of 90 GB budget" (there is no storage budget setting).
  //
  // What is real: `measuredBytesPerDay` (measured over complete capture days),
  // the retention window from settings, and `diskFreeBytes`. The bar is the
  // projected steady-state footprint of the retention window as a fraction of
  // that footprint plus the free space it has to live in — a fraction of
  // something real. "Keep everything" has no projection, so it draws no bar.
  import Tile from "./Tile.svelte";
  import { retentionDays } from "./format";
  import { formatBytes } from "$lib/settings/state/format";
  import type { Cell } from "./data";
  import type { RecordingSettings } from "$lib/types/recording";
  import type { SystemFacts } from "$lib/types/system-facts";

  interface Props {
    facts: Cell<SystemFacts | null>;
    settings: Cell<RecordingSettings | null>;
    loaded: boolean;
    open: () => void;
  }

  let { facts, settings, loaded, open }: Props = $props();

  const perDay = $derived(facts.data?.measuredBytesPerDay ?? null);
  const free = $derived(facts.data?.diskFreeBytes ?? null);
  const keepDays = $derived(
    settings.data ? retentionDays(settings.data.retentionPolicy) : null,
  );
  const projected = $derived(perDay !== null && keepDays !== null ? perDay * keepDays : null);
  const fillPct = $derived(
    projected !== null && free !== null && projected + free > 0
      ? Math.min(100, (projected / (projected + free)) * 100)
      : null,
  );
</script>

<Tile id="storage" title="Storage" kbd="⌃S" {open} openLabel="Open storage settings">
  {#if facts.error}
    <p class="tile-empty t-meta">Storage unavailable — {facts.error}</p>
  {:else if perDay !== null}
    <div class="tile-row">
      <span class="t-ui strong is-mono is-num">{formatBytes(perDay)}</span>
      <span class="t-meta">a day, measured</span>
    </div>
  {:else if loaded}
    <div class="tile-row">
      <span class="t-meta">No complete capture day measured yet.</span>
    </div>
  {/if}

  <div class="tile-row">
    <span class="t-meta">
      {#if keepDays !== null}Keep {keepDays} days{:else if settings.data}Keep everything{/if}
    </span>
  </div>

  {#if fillPct !== null}
    <div class="ladder__bar storage__bar">
      <i style="width:{fillPct.toFixed(1)}%"></i>
    </div>
    <div class="tile-row">
      <span class="t-meta is-mono is-num storage__foot">
        ≈{formatBytes(projected ?? 0)} kept · {formatBytes(free ?? 0)} free
      </span>
    </div>
  {:else if free !== null}
    <div class="tile-row storage__foot-row">
      <span class="t-meta is-mono is-num storage__foot">{formatBytes(free)} free</span>
    </div>
  {/if}
</Tile>
