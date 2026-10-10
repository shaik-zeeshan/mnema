<script lang="ts">
  import { renderIdle } from "$lib/render-idle.svelte";

  // "updated 1m ago" — the only sign of a quiet background refresh (OV-22).
  // `at` is stamped in each loader's success branch; null hides the stamp.
  // `inline` prefixes a "·" separator for use inside a sentence-y subtitle.
  let { at, inline = false }: { at: number | null; inline?: boolean } = $props();

  let now = $state(Date.now());
  $effect(() => {
    // Skips its tick while nothing can render ($lib/render-idle.svelte).
    const timer = setInterval(() => {
      if (!renderIdle()) now = Date.now();
    }, 30_000);
    return () => clearInterval(timer);
  });

  const label = $derived.by(() => {
    if (at == null) return "";
    const min = Math.floor(Math.max(0, now - at) / 60_000);
    if (min < 1) return "just now";
    if (min < 60) return `${min}m ago`;
    const hr = Math.floor(min / 60);
    return hr < 24 ? `${hr}h ago` : `${Math.floor(hr / 24)}d ago`;
  });
</script>

{#if at != null}<span class="updated-stamp">{inline ? "· " : ""}updated {label}</span>{/if}

<style>
  .updated-stamp {
    flex: 0 0 auto;
    white-space: nowrap;
  }
</style>
