<!--
  Pipeline tab (developer.html `?state=pipeline`): the five-node flow strip,
  the four processor rows (each drills into ProcessorView) and Housekeeping.
-->
<script lang="ts">
  import { formatClock } from "$lib/time-format";
  import HealthRow from "../HealthRow.svelte";
  import ProcessorView from "./ProcessorView.svelte";
  import { parseJobTs } from "../format";
  import { getDebugController } from "../state/controller.svelte";
  import { appJobsLine, flowNodes, frameBatchLine, processorRows, retentionLine } from "../pipeline-view";

  const controller = getDebugController();
  const { nav, health, pipeline } = controller;

  $effect(() => controller.poll(pipeline.fetchStatus, 2000));
  $effect(() => controller.poll(pipeline.fetchHousekeeping, 30_000));

  const nodes = $derived(flowNodes(health.entries, pipeline.lane, pipeline.semanticIndex));
  const rows = $derived(processorRows(health.entries, pipeline.lane, pipeline.settings));
  const housekeeping = $derived([
    { label: "Retention cleanup", ...retentionLine(pipeline.retention) },
    { label: "Frame batches", ...frameBatchLine(pipeline.batches) },
    { label: "App jobs", ...appJobsLine(pipeline.appJobs) },
  ]);

  const clock = (at: string | null) => {
    const ms = parseJobTs(at);
    return ms == null ? "—" : formatClock(ms);
  };
</script>

{#if nav.processor != null}
  <ProcessorView />
{:else}
  <section class="dv-sec">
    <h3 class="dv-h">Flow<span class="num">failed = last 24 h</span></h3>
    <div class="dv-flow">
      {#each nodes as node (node.name)}
        <div class="dv-node" data-tone={node.tone}>
          <span class="dv-node__name">{node.name}</span>
          <span class="dv-node__big">{node.big}</span>
          <span class="dv-node__unit">{node.unit}</span>
          {#each node.lines as line, i (i)}
            <span class="dv-node__line" data-tone={line.tone}>{line.text}</span>
          {/each}
          {#if node.bar !== undefined}
            <span class="dv-node__bar"><i style:--v={node.bar}></i></span>
          {/if}
        </div>
      {/each}
    </div>
    {#if pipeline.error}<p class="dv-err" data-tone="danger">{pipeline.error}</p>{/if}
  </section>

  <section class="dv-sec">
    <h3 class="dv-h">Processors</h3>
    <div class="dv-list">
      {#each rows as row (row.key)}
        <HealthRow
          tone={row.tone}
          name={row.name}
          why={row.why}
          metric={row.metric}
          plain
          onclick={() => controller.openProcessor(row.key)}
        />
      {/each}
    </div>
  </section>

  <section class="dv-sec">
    <h3 class="dv-h">Housekeeping</h3>
    <dl class="mx-spec">
      {#each housekeeping as line (line.label)}
        <div><dt>{line.label}</dt><dd>{line.value}</dd><dd class="mx-spec__note">{clock(line.at)}</dd></div>
      {/each}
    </dl>
    {#if pipeline.housekeepingError}<p class="dv-err" data-tone="danger">{pipeline.housekeepingError}</p>{/if}
  </section>
{/if}

<style>
  .dv-flow { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 8px; }
  .dv-node { position: relative; display: grid; align-content: start; gap: 3px; min-width: 0; padding: 9px 9px 10px;
    border: 1px solid var(--app-border); border-radius: var(--r-md); background: var(--app-surface-subtle); }
  .dv-node + .dv-node::before { content: ""; position: absolute; left: -9px; top: 50%; width: 8px; height: 1px; background: var(--app-border-strong); }
  .dv-node:is([data-tone="warn"], [data-tone="danger"]) { border-color: color-mix(in srgb, var(--tone-bd) 75%, var(--app-border));
    background: color-mix(in srgb, var(--tone-bg) 55%, var(--app-surface)); }
  .dv-node__name { font: 500 var(--text-sm)/1.2 var(--font-sans); color: var(--app-text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .dv-node__big { margin-top: 5px; font: 400 20px/1 var(--font-mono); font-variant-numeric: tabular-nums; letter-spacing: -0.04em; color: var(--app-text-strong); }
  .dv-node__unit { font: 400 var(--text-xs)/1.2 var(--font-sans); color: var(--app-text-subtle); margin-bottom: 3px; }
  .dv-node__line { font: 450 var(--text-xs)/1.35 var(--font-mono); color: var(--app-text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .dv-node__line[data-tone] { color: var(--tone); }
  .dv-node__bar { height: 3px; margin-top: 6px; border-radius: 2px; background: var(--mx-wash-strong); overflow: hidden; }
  .dv-node__bar i { display: block; height: 100%; width: calc(var(--v, 0) * 100%); border-radius: inherit; background: var(--chart-grey-4); }
  .dv-node[data-tone] .dv-node__bar i { background: var(--tone); }
  .dv-err { margin: 0; font: 400 var(--text-sm)/1.4 var(--font-sans); color: var(--tone); }
</style>
