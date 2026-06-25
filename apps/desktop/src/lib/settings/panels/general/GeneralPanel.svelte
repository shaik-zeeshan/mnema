<script lang="ts">
  import Appearance from "./Appearance.svelte";
  import Startup from "./Startup.svelte";
  import Shortcuts from "./Shortcuts.svelte";
</script>

<div role="tabpanel" id="settings-panel-general" tabindex="0">
  <Appearance />
  <Startup />
  <Shortcuts />
</div>
