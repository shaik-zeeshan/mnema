<script lang="ts">
  import PanelHeading from "$lib/settings/ui/PanelHeading.svelte";
  import { systemFacts } from "$lib/settings/state/system-facts.svelte";
  import { storageOverview } from "$lib/settings/state/system-facts";
  import Storage from "./Storage.svelte";
  import Access from "./Access.svelte";

  // Three rows, and two of them are the most consequential in the app — so the
  // head carries the two figures they act on. Both measured; null → no line.
  void systemFacts.ensureLoaded();
  const overview = $derived(storageOverview(systemFacts.value));
</script>

<div class="settings-panel" id="settings-panel-data">
  <PanelHeading title="Data" meta={overview} />
  <Storage />
  <Access />
</div>
