<!--
  Overview (developer.html `renderOverview`): the 12 health rows grouped under
  "Needs a look", "Right now" and "Running fine", ranked within each group.
  Every row drills into the tab that explains it. No polls of its own — the
  rows come from the shell's health poll.
-->
<script lang="ts">
  import HealthRow from "../HealthRow.svelte";
  import { getDebugController } from "../state/controller.svelte";
  import { group, type RowTarget } from "../health-view";

  const controller = getDebugController();
  const groups = $derived(group(controller.health.entries));

  function open(target: RowTarget) {
    if ("processor" in target) controller.openProcessor(target.processor);
    else controller.go(target.tab);
  }
</script>

<div class="dv-sec dv-overview">
  {#each groups as g (g.title)}
    <section class="dv-sec">
      <h3 class="dv-h">{g.title}<span class="num">{g.rows.length}</span></h3>
      <div class="dv-list">
        {#each g.rows as row (row.feature)}
          <HealthRow tone={row.tone} name={row.name} why={row.why} metric={row.metric} onclick={() => open(row.target)} />
        {/each}
      </div>
    </section>
  {/each}
</div>

<style>
  .dv-overview { gap: 18px; }
</style>
