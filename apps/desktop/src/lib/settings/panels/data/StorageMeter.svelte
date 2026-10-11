<script lang="ts">
  // Settings › Storage usage meter (settings.html #s-storage): total used, free
  // space, per-day rate, and a five-part bar split screen / mic / system audio /
  // index / models. `get_storage_usage` walks the disk, so it is cached backend
  // side; this panel forces a fresh walk on open and after each cleanup.
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import IconAlert from "~icons/lucide/triangle-alert";
  import { humanizeError } from "$lib/format-error";
  import { formatMeasuredBytes } from "$lib/onboarding/gates";
  import { formatBytes } from "$lib/settings/state/format";
  import type { StorageUsage } from "$lib/types/app-infra";
  import { meterParts, perDayBytes, storageTotal } from "./storage-meter";

  let usage = $state<StorageUsage | null>(null);
  let error = $state<string | null>(null);

  async function load(): Promise<void> {
    error = null;
    try {
      usage = await invoke<StorageUsage>("get_storage_usage", { refresh: true });
    } catch (err) {
      error = humanizeError(err);
    }
  }

  // On open, and again after anything deletes capture (retention — automatic or
  // "Run cleanup now" — and Delete Recent Capture both emit this).
  onMount(() => {
    void load();
    const off = listen<{ reason: string }>("timeline_data_changed", (e) => {
      if (e.payload.reason === "retention" || e.payload.reason === "delete_recent_capture") void load();
    });
    return () => void off.then((fn) => fn());
  });

  // "38.4 GB" → value + unit, as the mockup sets the unit in <small>.
  const split = (bytes: number) => {
    const [value, unit] = formatMeasuredBytes(bytes).split(" ");
    return { value, unit };
  };
  const total = $derived(usage ? split(storageTotal(usage)) : null);
  const parts = $derived(usage ? meterParts(usage) : []);
  const perDay = $derived(usage ? perDayBytes(usage) : null);
</script>

<div class="st-meter" aria-busy={!usage && !error}>
  {#if usage && total}
    <div class="st-meter__top">
      <span class="mx-stat__value num">{total.value}<small>{total.unit}</small></span>
      <span class="mx-label"
        >used{#if usage.freeBytes !== null}{" · "}<span class="num">{formatMeasuredBytes(usage.freeBytes)}</span>
          free{/if}{#if perDay !== null}{" · "}<span class="num">~{formatBytes(perDay)}</span> a day{/if}</span
      >
    </div>
    <div class="st-meter__bar" role="img" aria-label="Storage breakdown">
      {#each parts as p (p.label)}
        {#if p.pct > 0}<i style:width="{p.pct}%" style:--c={p.color}></i>{/if}
      {/each}
    </div>
    <ul class="st-meter__legend">
      {#each parts as p (p.label)}
        <li style:--c={p.color}>{p.label} <b>{formatMeasuredBytes(p.bytes)}</b></li>
      {/each}
    </ul>
  {:else if error}
    <span class="mx-inline" data-tone="danger" role="alert"
      ><IconAlert width="13" height="13" aria-hidden="true" />Couldn’t measure storage: {error}<button
        type="button"
        class="mx-btn mx-btn--ghost mx-btn--sm"
        onclick={load}>Retry</button
      ></span
    >
  {:else}
    <div class="st-meter__top">
      <span class="mx-skel" style="width:96px;height:28px"></span>
      <span class="mx-skel mx-skel--line" style="width:220px"></span>
    </div>
    <div class="st-meter__bar"></div>
    <div class="st-meter__legend">
      {#each [64, 52, 64, 58, 64] as w, i (i)}<span class="mx-skel mx-skel--line" style:width="{w}px"></span>{/each}
    </div>
  {/if}
</div>

<style>
  .st-meter {
    display: grid;
    gap: var(--s-3);
    padding: 14px 0;
    border-bottom: 1px solid var(--mx-hairline);
  }
  .st-meter__top {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
  }
  .st-meter__bar {
    display: flex;
    gap: 2px;
    height: 10px;
    border-radius: var(--r-xs);
    overflow: hidden;
    background: var(--app-surface-hover);
  }
  .st-meter__bar i {
    display: block;
    height: 100%;
    background: var(--c);
  }
  .st-meter__legend {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2) var(--s-5);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .st-meter__legend li {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font: 400 var(--text-base)/1 var(--font-sans);
    color: var(--app-text-muted);
  }
  .st-meter__legend li::before {
    content: "";
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--c);
  }
  .st-meter__legend b {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    font-weight: 500;
    color: var(--app-text-strong);
  }
</style>
