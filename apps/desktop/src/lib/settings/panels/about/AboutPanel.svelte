<script lang="ts">
  import PanelHeading from "$lib/settings/ui/PanelHeading.svelte";
  import About from "./About.svelte";
  import License from "./License.svelte";
  import Developer from "./Developer.svelte";
</script>

<div class="settings-panel" id="settings-panel-about">
  <PanelHeading title="About" meta="Licence, version, updates, and the logs" />
  <License />
  <About />
  <Developer />
</div>
