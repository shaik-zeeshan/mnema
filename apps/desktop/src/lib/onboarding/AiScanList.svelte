<!--
  The onboarding AI step's local scan results (split out of AiSetup.svelte):
  one row per probed localhost port, "Use this" on a live one, and the honest
  empty state when nothing answers.
-->
<script lang="ts">
  import IconSearch from "~icons/lucide/search";
  import { plural } from "$lib/onboarding/ai-readiness";
  import type { AiProviderKind } from "$lib/types";

  type ScanRow = {
    port: number;
    kind: AiProviderKind;
    name: string;
    state: string;
    latencyMs: number | null;
    models: string[] | null;
  };

  let {
    rows,
    scanning,
    connected,
    onUse,
  }: {
    rows: ScanRow[];
    scanning: boolean;
    connected: (port: number) => boolean;
    onUse: (kind: AiProviderKind, baseUrl: string) => void;
  } = $props();
</script>

<div class="ob-scan">
  {#each rows as row (row.port)}
    <div class:dead={row.state === "dead"}>
      <span class="ob-lamp" class:live={row.state === "live"}></span>
      <span>localhost:{row.port} · {row.name}</span>
      <span>{#if row.models}<span class="mx-chip" class:mx-chip--accent={row.models.length > 0}
            class:mx-chip--warn={row.models.length === 0}>{plural(row.models.length, "model")}</span
          >{/if}</span>
      <span>
        {row.latencyMs != null ? `${row.latencyMs} ms` : row.state === "trying" ? "…" : "no answer"}
      </span>
      <span>
        {#if row.models}
          {#if connected(row.port)}
            <span class="mx-chip">connected</span>
          {:else}
            <button class="mx-btn mx-btn--sm" type="button" onclick={() => onUse(row.kind, `http://localhost:${row.port}`)}>
              Use this
            </button>
          {/if}
        {/if}
      </span>
    </div>
  {/each}
</div>
{#if !scanning && !rows.some((row) => row.models !== null)}
  <div class="mx-empty mx-empty--compact" data-tone="warn">
    <span class="mx-empty__glyph"><IconSearch width="14" height="14" /></span>
    <b class="mx-empty__title">Nothing is listening on the usual ports</b>
    <p class="mx-empty__text">
      That is an answer, not an error. Start Ollama and scan again, point at an engine on your
      network below, or pick a cloud account above.
    </p>
  </div>
{/if}

<style>
  .ob-scan {
    display: grid;
    border: 1px solid var(--app-border);
    border-radius: var(--r-md);
    background: var(--app-surface);
  }
  .ob-scan > div {
    display: grid;
    grid-template-columns: 8px minmax(0, 1fr) auto 80px 96px;
    align-items: center;
    gap: var(--s-3);
    height: 40px;
    padding: 0 var(--s-3);
    font: 400 var(--text-base) / 1 var(--font-mono);
    color: var(--app-text);
  }
  .ob-scan > div + div {
    border-top: 1px solid var(--mx-hairline);
  }
  .ob-scan > div.dead {
    color: var(--app-text-subtle);
  }
  .ob-scan > div > span:last-child {
    justify-self: end;
  }
  .ob-lamp {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--chart-grey-2);
  }
  .ob-lamp.live {
    background: var(--app-accent);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }
</style>
