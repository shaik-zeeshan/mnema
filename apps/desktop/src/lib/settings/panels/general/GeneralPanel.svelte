<script lang="ts">
  import PanelHeading from "$lib/settings/ui/PanelHeading.svelte";
  import Appearance from "./Appearance.svelte";
  import Startup from "./Startup.svelte";
  import Shortcuts from "./Shortcuts.svelte";
</script>

<div class="settings-panel" id="settings-panel-general">
  <PanelHeading title="General" meta="Appearance, startup, and the keys that reach Mnema" />
  <Appearance />
  <Startup />
  <Shortcuts />
</div>
