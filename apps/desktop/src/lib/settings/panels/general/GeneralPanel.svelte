<script lang="ts">
  import Appearance from "./Appearance.svelte";
  import Startup from "./Startup.svelte";
  import Shortcuts from "./Shortcuts.svelte";
</script>

<!-- The mounted tab panel for the `general` toolbar tab. Its groups lay out on
     the bento grid (settings-bento.css). -->
<div
  class="settings-panel"
  id="settings-panel-general"
  role="tabpanel"
  aria-labelledby="settings-tab-general"
>
  <Appearance />
  <Startup />
  <Shortcuts />
</div>
