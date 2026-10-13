<!--
  Stub until F2: the health rows as one flat list, so the window stays usable
  mid-build. F2 replaces it with the grouped Overview.
-->
<script lang="ts">
  import { formatClock } from "$lib/time-format";
  import HealthRow from "../HealthRow.svelte";
  import { getDebugController } from "../state/controller.svelte";
  import { FEATURE_ROWS, toneOf } from "../verdict";

  const controller = getDebugController();
  type Row = { feature: string; severity: string; reason: string; metric?: string | null; metricAtMs?: number | null };
  const rows = $derived(controller.health.entries as Row[]);
</script>

<section class="dv-sec">
  <div class="dv-list">
    {#each rows as row (row.feature)}
      {@const meta = FEATURE_ROWS[row.feature]}
      <HealthRow
        tone={toneOf(row.severity)}
        name={meta?.name ?? row.feature}
        why={row.reason}
        metric={row.metricAtMs != null ? formatClock(row.metricAtMs) : (row.metric ?? "")}
        onclick={() => controller.go(meta?.tab ?? "overview")}
      />
    {/each}
  </div>
</section>
