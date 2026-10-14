<!--
  Mini content (developer.html `renderMini`): eight source/processor dots, then
  the worst row ("Name: why", or the verdict) and "N queued". The whole body is
  a button; its click bubbles to DevStrip's mini frame, which expands.
-->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { ProcessorPipelineStatus } from "$lib/types";
  import { getDebugController } from "./state/controller.svelte";
  import { MINI_DOTS, miniFooter, toneOf } from "./health-view";

  const controller = getDebugController();
  const entries = $derived(controller.health.entries);
  const dots = $derived(
    MINI_DOTS.map(([feature, label]) => {
      const severity = entries.find((e) => e.feature === feature)?.severity;
      return { feature, label, tone: severity ? toneOf(severity) : "off" };
    }),
  );
  const foot = $derived(miniFooter(entries, controller.health.error));

  let lanes = $state<ProcessorPipelineStatus[] | null>(null);
  $effect(() =>
    controller.poll(async () => {
      try {
        lanes = await invoke<ProcessorPipelineStatus[]>("get_processing_pipeline_status");
      } catch {
        // ponytail: keep the last count; the dots still say what's wrong.
      }
    }, 2000),
  );
  // `retrying` is already inside `queued`. The meaning backlog counts only if
  // the Pipeline tab already fetched it — no extra call here.
  const queued = $derived(
    lanes == null
      ? null
      : lanes.reduce((n, l) => n + l.queued, 0) + (controller.pipeline.semanticIndex?.backlogCount ?? 0),
  );
</script>

<button type="button" class="dv-mini__body" aria-label="Expand developer window">
  <span class="dv-mini__dots">
    {#each dots as dot (dot.feature)}
      <span class="dv-mini__dot" data-tone={dot.tone}><span class="dv-dot"></span>{dot.label}</span>
    {/each}
  </span>
  <span class="dv-mini__foot" data-tone={foot.tone}>
    <span class="dv-dot"></span>
    <span class="dv-mini__why">{foot.text}</span>
    {#if queued != null}<b class="num">{queued.toLocaleString()} queued</b>{/if}
  </span>
</button>

<style>
  .dv-mini__body { appearance: none; border: 0; background: none; font: inherit; color: inherit; cursor: pointer; text-align: left;
    min-width: 0; min-height: 0; display: grid; align-content: center; gap: 9px; padding: 0 12px; }
  .dv-mini__body:focus-visible { outline: 2px solid var(--app-accent); outline-offset: -2px; }
  .dv-mini__dots { display: grid; grid-template-columns: repeat(4, auto); justify-content: space-between; gap: 6px 10px; }
  .dv-mini__dot { display: flex; align-items: center; gap: 5px; min-width: 0; font: 500 var(--text-xs)/1 var(--font-sans); color: var(--app-text-muted); white-space: nowrap; }
  .dv-mini__dot .dv-dot { width: 6px; height: 6px; }
  .dv-mini__foot { display: flex; align-items: center; gap: 6px; padding-top: 7px; border-top: 1px solid var(--mx-hairline); min-width: 0; }
  .dv-mini__why { min-width: 0; flex: 1; font: 400 var(--text-sm)/1.2 var(--font-sans); color: var(--app-text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .dv-mini__foot .num { flex: none; font-size: var(--text-xs); color: var(--app-text-subtle); }
</style>
